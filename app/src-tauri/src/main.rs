#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

#[cfg(not(test))]
fn main() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_allows_only_the_self_test_in_the_main_window() {
        let app = configure(tauri::test::mock_builder())
            .build(tauri::generate_context!())
            .expect("mock app");
        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("main window");
        let other = tauri::WebviewWindowBuilder::new(&app, "other", Default::default())
            .build()
            .expect("unprivileged window");
        let request = |command: &str| tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .expect("local URL"),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        tauri::test::assert_ipc_response(&main, request("self_test"), Ok(vz_core::self_test()));
        assert!(tauri::test::get_ipc_response(&other, request("self_test")).is_err());
        assert!(tauri::test::get_ipc_response(&main, request("plugin:app|version")).is_err());
    }
}

fn pipeline_paths<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    use tauri::Manager;
    let state = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("phase0-index");
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
            let state = vz_core::storage::State::open(&root)?;
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
            let state = vz_core::storage::State::open(&root)?;
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
