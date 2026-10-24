use crate::models;

// Keep the checked Update, including its version and signature, until consent.
#[derive(Default)]
struct PendingUpdate(tokio::sync::Mutex<Option<tauri_plugin_updater::Update>>);

#[tauri::command]
async fn update_check<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    pending: tauri::State<'_, PendingUpdate>,
    startup: Option<bool>,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_updater::UpdaterExt;
    if startup.unwrap_or(false)
        && std::env::args().any(|arg| {
            matches!(
                arg.as_str(),
                "--smoke-test"
                    | "--pipeline-smoke-test"
                    | "--update-test"
                    | "--single-instance-test"
            )
        })
    {
        return Ok(serde_json::json!({"available": false}));
    }
    let mut stored = pending.0.lock().await;
    *stored = None;
    let update = app
        .updater_builder()
        .restart_after_install(true)
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    let result = match &update {
        Some(update) => serde_json::json!({"available": true, "version": update.version}),
        None => serde_json::json!({"available": false}),
    };
    *stored = update;
    Ok(result)
}

#[tauri::command]
async fn update_install<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    pending: tauri::State<'_, PendingUpdate>,
    progress: tauri::ipc::Channel<serde_json::Value>,
) -> Result<(), serde_json::Value> {
    // Serialize checks and installs so another check cannot replace this consented version.
    let stored = pending.0.lock().await;
    let update = stored.as_ref().ok_or_else(
        || serde_json::json!({"detail": "Check for updates again", "checkAgain": true}),
    )?;
    let mut downloaded = 0;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk;
                let _ = progress.send(serde_json::json!({
                    "stage": "downloading", "downloaded": downloaded, "total": total
                }));
            },
            || {},
        )
        .await
        .map_err(|error| {
            // Download includes signature verification and has not replaced anything.
            serde_json::json!({"detail": error.to_string(), "currentVersionUnchanged": true})
        })?;
    let _ = progress.send(serde_json::json!({"stage": "installing"}));
    update.install(bytes).map_err(|error| {
        // A macOS replacement may already have happened, including PreviousAppNotRestored.
        // Do not claim that the installed app is unchanged for an install-stage error.
        serde_json::json!({"detail": error.to_string(), "currentVersionUnchanged": false})
    })?;
    #[cfg(target_os = "macos")]
    app.restart();
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(())
    }
}

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
            if matches!(checkpoint.as_str(), "guard-before" | "guard-after") {
                static BEFORE: std::sync::Mutex<Option<serde_json::Value>> =
                    std::sync::Mutex::new(None);
                let root = data_root(&_app).expect("data path").join("phase0-index");
                let snapshot = models::index_snapshot(&root).expect("seeded index snapshot");
                let mut before = BEFORE.lock().unwrap();
                if checkpoint == "guard-before" {
                    *before = Some(snapshot);
                } else {
                    assert_eq!(
                        *before,
                        Some(snapshot),
                        "Invalid Folder changed document rows or generation files"
                    );
                }
            }
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
            assert!(!resources.join("assets/bge-small-f16.gguf").exists());
            assert!(!resources
                .join("assets/qwen2.5-0.5b-instruct-q4_k_m.gguf")
                .exists());
            assert!(vz_core::worker::executable()
                .expect("worker path")
                .is_file());
            let data = data_root(&_app).expect("data directory");
            std::fs::create_dir_all(&data).expect("create persistent data folder");
            std::fs::write(
                data.join("install-self-test.json"),
                serde_json::to_vec(&result).unwrap(),
            )
            .unwrap();
            let mut installed_result = serde_json::to_value(&result).unwrap();
            installed_result["version"] =
                serde_json::json!(_app.package_info().version.to_string());
            std::fs::write(report, serde_json::to_vec(&installed_result).unwrap())
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

// The pinned single-instance plugin sends WM_COPYDATA and exits during setup.
// Hand foreground rights to its existing receiver BEFORE that signal. A launch
// from Explorer can grant rights that the background first instance lacks.
#[cfg(windows)]
fn allow_single_instance_foreground(identifier: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId,
    };
    let wide = |value: String| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    // These names are the Windows receiver names in pinned plugin 2.4.0
    // (without its optional semver feature).
    let class = wide(format!("{identifier}-sic"));
    let title = wide(format!("{identifier}-siw"));
    // SAFETY: both strings are NUL-terminated and live through FindWindowW;
    // GetWindowThreadProcessId writes into a valid local u32.
    unsafe {
        let receiver = FindWindowW(class.as_ptr(), title.as_ptr());
        if !receiver.is_null() {
            let mut pid = 0;
            GetWindowThreadProcessId(receiver, &mut pid);
            if pid != 0 {
                let granted = AllowSetForegroundWindow(pid) != 0;
                let error = if granted {
                    None
                } else {
                    Some(std::io::Error::last_os_error().to_string())
                };
                #[cfg(feature = "smoke-test")]
                if std::env::args().any(|a| a == "--single-instance-test") {
                    if let Some(path) = std::env::var_os("VZ_SINGLE_INSTANCE_HANDOFF") {
                        std::fs::write(path, serde_json::json!({"pid": std::process::id(), "target_pid": pid, "granted": granted, "error": error}).to_string()).expect("write foreground handoff report");
                    }
                }
                if let Some(error) = error {
                    eprintln!("Single-instance foreground permission refused: {error}");
                }
            }
        }
    }
}

fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let mut focus_request_accepted = window.set_focus().is_ok();
                // Restoration is asynchronous on macOS. Tao skips set_focus
                // while still minimized, so retry after the restore animation.
                std::thread::spawn(move || {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    loop {
                        let minimized = window.is_minimized().unwrap_or(true);
                        let focused = window.is_focused().unwrap_or(false);
                        let visible = window.is_visible().unwrap_or(false);
                        if (!minimized && visible && focused && focus_request_accepted)
                            || std::time::Instant::now() >= deadline
                        {
                            #[cfg(feature = "smoke-test")]
                            if std::env::args().any(|a| a == "--single-instance-test") {
                                let report = serde_json::json!({"pid": std::process::id(), "minimized": minimized, "visible": visible, "focused": focused, "focus_requested": true, "focus_request_accepted": focus_request_accepted});
                                std::fs::write(std::env::var_os("VZ_SINGLE_INSTANCE_REPORT").expect("test report"), report.to_string()).expect("write focus report");
                            }
                            break;
                        }
                        if !minimized {
                            focus_request_accepted |= window.set_focus().is_ok();
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                });
            }
        }))
        .manage(models::Operations::default())
        .manage(PendingUpdate::default())
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(feature = "smoke-test")]
    let builder = builder.on_page_load(|webview, payload| {
        if payload.event() == tauri::webview::PageLoadEvent::Finished
            && std::env::args().any(|a| a == "--single-instance-test") {
            let window = webview.window();
            if std::env::var_os("VZ_SINGLE_INSTANCE_MINIMIZE").is_some() { window.minimize().expect("minimize test window"); }
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                std::fs::write(std::env::var_os("VZ_SINGLE_INSTANCE_READY").expect("test ready path"),
                    serde_json::json!({"pid":std::process::id(),"minimized":window.is_minimized().expect("minimized state")}).to_string()).expect("ready report");
            });
        }
        if smoke_test_requested() && payload.event() == tauri::webview::PageLoadEvent::Finished {
            if std::env::args().any(|arg|arg == "--pipeline-smoke-test") {
                let fixture = std::env::var("VZ_PIPELINE_FIXTURE").unwrap_or_else(|_| {
                    std::env::current_dir().expect("cwd").join("spike/fixtures/documents").to_string_lossy().into_owned()
                });
                let config = serde_json::json!({"fixture": fixture, "acquisition": std::env::var_os("VZ_ACQUISITION_SMOKE").is_some()});
                webview.eval(format!("window.__VZ_SMOKE__ = {config};\n{}", include_str!("../../../scripts/pipeline-smoke.js"))).expect("pipeline smoke clicks");
            } else { webview.eval(
                "const timer = setInterval(() => { const button = document.querySelector('button'); if (button) { clearInterval(timer); button.click(); } }, 50);"
            ).expect("start smoke-test button click"); }
        }
    });
    builder.invoke_handler(tauri::generate_handler![
        self_test,
        pipeline_index,
        pipeline_search,
        pipeline_ask,
        update_check,
        update_install,
        models_status,
        models_download,
        models_cancel,
        models_import
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
    #[cfg(windows)]
    allow_single_instance_foreground(&context.config().identifier);
    configure(tauri::Builder::default().setup(|app| {
        #[cfg(feature = "smoke-test")]
        if std::env::args().any(|arg| arg == "--update-test") {
            use tauri_plugin_updater::UpdaterExt;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = async {
                    let update = handle
                        .updater_builder()
                        .restart_after_install(false)
                        .build()?
                        .check()
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("No update available"))?;
                    let expected = std::env::var("VZ_UPDATE_TO_VERSION")?;
                    anyhow::ensure!(update.version == expected, "Unexpected update version");
                    update.download_and_install(|_, _| {}, || {}).await?;
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                if let Err(error) = result {
                    eprintln!("Update test failed: {error:#}");
                    handle.exit(1);
                } else {
                    handle.exit(0);
                }
            });
        }
        #[cfg(not(feature = "smoke-test"))]
        let _ = app;
        Ok(())
    }))
    .build(context)
    .expect("failed to build vectorAIz")
    .run(|app, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            use tauri::Manager;
            app.state::<models::Operations>().cancel();
        }
    });
}

fn installed_assets<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let assets = model_dir(app)?;
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    let relative = if cfg!(windows) {
        "pdfium/bin/pdfium.dll"
    } else {
        "pdfium/lib/libpdfium.dylib"
    };
    let library = assets.join(relative);
    crate::bundled_library::refresh(&resources.join("assets").join(relative), &library)
        .map_err(|e| e.to_string())?;
    Ok(assets)
}

fn pipeline_paths<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    use tauri::Manager;
    let state = data_root(app)?.join("phase0-index");
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    if resources.join("models.json").is_file() {
        return Ok((state, installed_assets(app)?, resources.join("models.json")));
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let assets = model_dir(app)?;
    let relative = if cfg!(windows) {
        "pdfium/bin/pdfium.dll"
    } else {
        "pdfium/lib/libpdfium.dylib"
    };
    crate::bundled_library::refresh(&cwd.join(".assets").join(relative), &assets.join(relative))
        .map_err(|e| e.to_string())?;
    Ok((state, assets, cwd.join("spike/fixtures/models.json")))
}

fn pipeline_cpu_requested() -> bool {
    std::env::args().any(|arg| arg == "--cpu")
}

#[tauri::command]
async fn pipeline_index<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    folder: String,
    progress: tauri::ipc::Channel<vz_core::index::Progress>,
) -> Result<serde_json::Value, String> {
    let guard = operations.acquire()?;
    models::validate_folder(&folder)?;
    let (root, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let _guard = guard;
        let run = || -> anyhow::Result<serde_json::Value> {
            let engines = vz_core::engine::Engines::load(&assets, &pins, pipeline_cpu_requested())?;
            #[cfg(feature = "smoke-test")]
            record_loaded_models(&assets, &pins)?;
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
    operations: tauri::State<'_, models::Operations>,
    query: String,
) -> Result<serde_json::Value, String> {
    let guard = operations.acquire()?;
    let (root, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let _guard = guard;
        let run = || -> anyhow::Result<serde_json::Value> {
            let engines = vz_core::engine::Engines::load(&assets, &pins, pipeline_cpu_requested())?;
            #[cfg(feature = "smoke-test")]
            record_loaded_models(&assets, &pins)?;
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
    operations: tauri::State<'_, models::Operations>,
    question: String,
    tokens: tauri::ipc::Channel<String>,
) -> Result<serde_json::Value, String> {
    let guard = operations.acquire()?;
    let (root, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let _guard = guard;
        let run = || -> anyhow::Result<serde_json::Value> {
            let engines = vz_core::engine::Engines::load(&assets, &pins, pipeline_cpu_requested())?;
            #[cfg(feature = "smoke-test")]
            record_loaded_models(&assets, &pins)?;
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

fn data_root<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    #[cfg(feature = "smoke-test")]
    if smoke_test_requested() || std::env::args().any(|a| a == "--single-instance-test") {
        if let Some(root) = std::env::var_os("VZ_SMOKE_DATA_ROOT") {
            let root = std::path::PathBuf::from(root);
            if !root.is_absolute() {
                return Err("Smoke data root must be absolute".into());
            }
            return Ok(root);
        }
    }
    Ok(app
        .path()
        .home_dir()
        .map_err(|e| e.to_string())?
        .join("vectorAIz"))
}
fn model_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<std::path::PathBuf, String> {
    Ok(data_root(app)?.join("models"))
}
fn model_pins<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<Vec<models::Pin>, String> {
    use tauri::Manager;
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    let path = if resources.join("models.json").is_file() {
        resources.join("models.json")
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join("spike/fixtures/models.json")
    };
    models::pins(&path).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn models_status<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<serde_json::Value, String> {
    let dir = model_dir(&app)?;
    let pins = model_pins(&app)?;
    tauri::async_runtime::spawn_blocking(move || models::status(&dir, &pins))
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn models_download<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    progress: tauri::ipc::Channel<serde_json::Value>,
) -> Result<(), String> {
    let _guard = operations.acquire()?;
    let cancel = operations.start();
    #[cfg(feature = "smoke-test")]
    let mut last_attempt = None;
    models::download(
        &model_dir(&app)?,
        &model_pins(&app)?,
        cancel,
        &mut |value: serde_json::Value| {
            #[cfg(feature = "smoke-test")]
            if smoke_test_requested() {
                let attempt = (
                    value["file"].as_str().unwrap_or_default().to_owned(),
                    value["attempt"].as_u64(),
                );
                if last_attempt.as_ref() != Some(&attempt) {
                    println!("VECTORAIZ_MODEL_TRANSFER: {value}");
                    last_attempt = Some(attempt);
                }
            }
            let _ = progress.send(value);
        },
    )
    .await
    .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
fn models_cancel(operations: tauri::State<'_, models::Operations>) {
    operations.cancel();
}
#[tauri::command]
async fn models_import<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    folder: String,
) -> Result<Vec<serde_json::Value>, String> {
    let _guard = operations.acquire()?;
    let cancel = operations.start();
    models::import(
        &model_dir(&app)?,
        &model_pins(&app)?,
        std::path::Path::new(&folder),
        cancel,
    )
    .await
    .map_err(|e| format!("{e:#}"))
}
#[cfg(feature = "smoke-test")]
fn record_loaded_models(assets: &std::path::Path, pins: &std::path::Path) -> anyhow::Result<()> {
    if let Some(report) = std::env::var_os("VZ_LOADED_MODELS_REPORT") {
        let pins = models::pins(pins)?;
        anyhow::ensure!(
            pins.iter()
                .all(|p| models::regular(&assets.join(&p.file), false)
                    .and_then(|file| Ok(file.metadata()?.len() == p.size))
                    .unwrap_or(false)),
            "Loaded model sizes must match acquisition pins"
        );
        // Engines::load just rehashed these exact paths. The install harness
        // independently checks the acquired bytes against this report after exit.
        let files: Vec<_> = pins
            .iter()
            .map(|p| {
                serde_json::json!({
                    "path": assets.join(&p.file), "size": p.size, "sha256": p.sha256
                })
            })
            .collect();
        std::fs::write(report, serde_json::to_vec(&files)?)?;
    }
    Ok(())
}
