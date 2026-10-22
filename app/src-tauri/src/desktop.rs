#[tauri::command]
fn self_test<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    checkpoint: Option<String>,
) -> vz_core::SelfTestResult {
    #[cfg(not(feature = "smoke-test"))]
    let _ = checkpoint;
    let result = vz_core::self_test();
    #[cfg(feature = "smoke-test")]
    if smoke_test_requested() {
        if let Some(checkpoint) = checkpoint {
            println!("VECTORAIZ_SMOKE_STAGE: {checkpoint}");
            if checkpoint.starts_with("ERROR:") {
                _app.exit(1);
            }
            return result;
        }
        assert_eq!(result.status, "OK");
        if let Some(report) = std::env::var_os("VZ_INSTALLED_SELF_TEST_REPORT") {
            use tauri::Manager;
            let resources = _app.path().resource_dir().expect("installed resources");
            assert!(resources.join("THIRD_PARTY_NOTICES").is_file());
            assert!(resources.join("models.json").is_file());
            assert!(resources.join("assets/pdfium").is_dir());
            assert!(vz_core::worker::executable()
                .expect("worker path")
                .is_file());
            let data = _app
                .path()
                .home_dir()
                .expect("home directory")
                .join("vectorAIz");
            std::fs::create_dir_all(&data).expect("create persistent data folder");
            std::fs::write(
                data.join("install-self-test.json"),
                serde_json::to_vec(&result).unwrap(),
            )
            .unwrap();
            std::fs::write(report, serde_json::to_vec(&result).unwrap())
                .expect("installed self-test report");
        }
        println!("VECTORAIZ_SMOKE_TEST_OK");
        if std::env::args().any(|arg| arg == "--pipeline-smoke-test") {
            println!("VECTORAIZ_PIPELINE_SMOKE_TEST_OK");
        }
        // Let the IPC response render before closing the smoke-test process.
        let app = _app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(1));
            app.exit(0);
        });
    }
    result
}

fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    // Rotation freeze: no updater can run with the deliberately invalid public key.
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("Tauri configuration");
    let builder = if config["plugins"]["updater"]["pubkey"]
        .as_str()
        .expect("updater public key")
        .starts_with("PLACEHOLDER-")
    {
        builder
    } else {
        builder.plugin(tauri_plugin_updater::Builder::new().build())
    };
    #[cfg(feature = "smoke-test")]
    let builder = builder.on_page_load(|webview, payload| {
        if smoke_test_requested() && payload.event() == tauri::webview::PageLoadEvent::Finished {
            if std::env::args().any(|arg|arg == "--pipeline-smoke-test") {
                webview.eval(include_str!("../../../scripts/pipeline-smoke.js")).expect("pipeline smoke clicks");
            } else { webview.eval(
                "const timer = setInterval(() => { const button = document.querySelector('button'); if (button) { clearInterval(timer); button.click(); } }, 50);"
            ).expect("start smoke-test button click"); }
        }
    });
    builder.invoke_handler(tauri::generate_handler![
        self_test,
        pipeline_index,
        pipeline_search,
        pipeline_ask
    ])
}

#[cfg(feature = "smoke-test")]
fn smoke_test_requested() -> bool {
    std::env::args().any(|arg| arg == "--smoke-test" || arg == "--pipeline-smoke-test")
}

pub(crate) fn run() {
    let context = tauri::generate_context!();
    #[cfg(feature = "smoke-test")]
    let mut context = context;
    #[cfg(feature = "smoke-test")]
    if smoke_test_requested() {
        // Hosted/occluded WebViews must keep driving the real controls. Keep
        // this test-only setting out of normal desktop battery behavior.
        for window in &mut context.config_mut().app.windows {
            window.background_throttling =
                Some(tauri::utils::config::BackgroundThrottlingPolicy::Disabled);
        }
    }
    configure(tauri::Builder::default())
        .run(context)
        .expect("failed to run vectorAIz");
}

fn pipeline_paths<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    use tauri::Manager;
    let state = app
        .path()
        .home_dir()
        .map_err(|e| e.to_string())?
        .join("vectorAIz")
        .join("phase0-index");
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    if resources.join("models.json").is_file() {
        return Ok((
            state,
            resources.join("assets"),
            resources.join("models.json"),
        ));
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    Ok((
        state,
        cwd.join(".assets"),
        cwd.join("spike/fixtures/models.json"),
    ))
}

fn pipeline_cpu_requested() -> bool {
    std::env::args().any(|arg| arg == "--cpu")
}

#[tauri::command]
async fn pipeline_index<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder: String,
    progress: tauri::ipc::Channel<vz_core::index::Progress>,
) -> Result<serde_json::Value, String> {
    let (root, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let run = || -> anyhow::Result<serde_json::Value> {
            let engines = vz_core::engine::Engines::load(&assets, &pins, pipeline_cpu_requested())?;
            let mut state = vz_core::storage::State::open(&root)?;
            let result = tokio::runtime::Runtime::new()?.block_on(vz_core::index::index(
                &mut state,
                std::path::Path::new(&folder),
                &engines,
                &assets,
                |value| {
                    let _ = progress.send(value.clone());
                },
            ))?;
            Ok(serde_json::to_value(result)?)
        };
        run().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn pipeline_search<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    query: String,
) -> Result<serde_json::Value, String> {
    let (root, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let run = || -> anyhow::Result<serde_json::Value> {
            let engines = vz_core::engine::Engines::load(&assets, &pins, pipeline_cpu_requested())?;
            let (state, _) = vz_core::storage::State::open_index(&root, &engines.recipe)?;
            let hits = tokio::runtime::Runtime::new()?.block_on(state.search(
                &query,
                &engines.vector(&query, true)?,
                5,
            ))?;
            Ok(serde_json::Value::Array(hits.into_iter().map(|hit| serde_json::json!({
                "chunk":{"chunk_id":hit.chunk.chunk_id,"doc_id":hit.chunk.doc_id,"text":hit.chunk.text,"page":hit.chunk.page},"score":hit.score
            })).collect()))
        };
        run().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn pipeline_ask<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    question: String,
    tokens: tauri::ipc::Channel<String>,
) -> Result<serde_json::Value, String> {
    let (root, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let run = || -> anyhow::Result<serde_json::Value> {
            let engines = vz_core::engine::Engines::load(&assets, &pins, pipeline_cpu_requested())?;
            let (state, _) = vz_core::storage::State::open_index(&root, &engines.recipe)?;
            let hits = tokio::runtime::Runtime::new()?.block_on(state.search(
                &question,
                &engines.vector(&question, true)?,
                3,
            ))?;
            let answer = vz_core::index::ask(&engines, &question, &hits, |token| {
                let _ = tokens.send(token.to_owned());
            })?;
            Ok(serde_json::to_value(answer)?)
        };
        run().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}
