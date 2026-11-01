use crate::{desktop_pipeline, models, settings};

// The overlap smoke test must not depend on Search being slower than its UI poll.
#[cfg(feature = "smoke-test")]
static HOLD_NEXT_SEARCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "smoke-test")]
static RELEASE_SEARCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[cfg(feature = "smoke-test")]
static HOLD_NEXT_TRANSFER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "smoke-test")]
static RELEASE_TRANSFER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[cfg(feature = "smoke-test")]
static HOLD_NEXT_INDEX: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "smoke-test")]
static RELEASE_INDEX: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

// Keep the checked Update, including its version and signature, until consent.
#[derive(Default)]
struct PendingUpdate(tokio::sync::Mutex<Option<tauri_plugin_updater::Update>>);

#[tauri::command]
async fn update_check<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    pending: tauri::State<'_, PendingUpdate>,
    startup: Option<bool>,
) -> Result<serde_json::Value, String> {
    #[cfg(all(feature = "ui-test", debug_assertions))]
    if startup.unwrap_or(false) {
        return Ok(serde_json::json!({"available": false}));
    }
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
            if checkpoint == "hold-next-search" {
                HOLD_NEXT_SEARCH.store(true, std::sync::atomic::Ordering::SeqCst);
            } else if checkpoint == "release-search" {
                RELEASE_SEARCH.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if checkpoint == "hold-next-index" {
                HOLD_NEXT_INDEX.store(true, std::sync::atomic::Ordering::SeqCst);
            } else if checkpoint == "release-index" {
                RELEASE_INDEX.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if checkpoint == "hold-next-transfer" {
                HOLD_NEXT_TRANSFER.store(true, std::sync::atomic::Ordering::SeqCst);
            } else if checkpoint == "release-transfer" {
                RELEASE_TRANSFER.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if matches!(checkpoint.as_str(), "guard-before" | "guard-after") {
                static BEFORE: std::sync::Mutex<Option<serde_json::Value>> =
                    std::sync::Mutex::new(None);
                let root = vz_core::active_index::resolve(&data_root(&_app).expect("data path"));
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

// Tao's parent-window focus cache is cleared when WebView2 takes keyboard
// focus. Check the actual foreground queue, including its focused child, rather
// than treating that normal child focus as a failed single-instance activation.
#[cfg(windows)]
fn single_instance_windows_focus<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> serde_json::Value {
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GetForegroundWindow, GetGUIThreadInfo, IsChild, GUITHREADINFO},
    };
    let hwnd: HWND = match window.hwnd() {
        Ok(hwnd) => hwnd.0,
        Err(error) => return serde_json::json!({"focused": false, "error": error.to_string()}),
    };
    // SAFETY: GUITHREADINFO is zero-initialized with the required cbSize and a
    // valid writable pointer. HWNDs are only passed back to Win32 for queries.
    unsafe {
        let foreground = GetForegroundWindow();
        let mut gui: GUITHREADINFO = std::mem::zeroed();
        gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        let queried = GetGUIThreadInfo(0, &mut gui) != 0; // current foreground queue
        let focus_in_window = queried
            && !gui.hwndFocus.is_null()
            && (gui.hwndFocus == hwnd || IsChild(hwnd, gui.hwndFocus) != 0);
        let foreground_after = GetForegroundWindow();
        serde_json::json!({
            "focused": foreground == hwnd && foreground_after == hwnd && focus_in_window,
            "window_hwnd": hwnd as usize,
            "foreground_hwnd": foreground as usize,
            "foreground_after_hwnd": foreground_after as usize,
            "gui_query_ok": queried,
            "active_hwnd": gui.hwndActive as usize,
            "keyboard_focus_hwnd": gui.hwndFocus as usize,
            "keyboard_focus_in_window": focus_in_window,
            "framework_focused": window.is_focused().unwrap_or(false),
        })
    }
}

fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    #[cfg(all(feature = "ui-test", debug_assertions))]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init()).plugin(
        tauri::plugin::Builder::<R>::new("ui-test-observer")
            .js_init_script(include_str!("../ui-test/observe.js").to_string())
            .build(),
    );
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
                        #[cfg(windows)]
                        let native_focus = single_instance_windows_focus(&window);
                        #[cfg(windows)]
                        let focused = native_focus["focused"].as_bool().unwrap_or(false);
                        #[cfg(not(windows))]
                        let focused = window.is_focused().unwrap_or(false);
                        #[cfg(all(feature = "smoke-test", not(windows)))]
                        let native_focus = serde_json::Value::Null;
                        let visible = window.is_visible().unwrap_or(false);
                        if (!minimized && visible && focused && focus_request_accepted)
                            || std::time::Instant::now() >= deadline
                        {
                            #[cfg(feature = "smoke-test")]
                            if std::env::args().any(|a| a == "--single-instance-test") {
                                let report = serde_json::json!({"pid": std::process::id(), "minimized": minimized, "visible": visible, "focused": focused, "focus_requested": true, "focus_request_accepted": focus_request_accepted, "native_focus": native_focus});
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
                let config = serde_json::json!({"fixture": fixture, "acquisition": std::env::var_os("VZ_ACQUISITION_SMOKE").is_some(), "migration": std::env::var("VZ_MIGRATION_SMOKE").ok()});
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
        models_import,
        answer_add,
        answer_switch,
        answer_turn_off,
        delete_unused_model
    ])
}

#[cfg(feature = "smoke-test")]
fn smoke_test_requested() -> bool {
    std::env::args().any(|arg| arg == "--smoke-test" || arg == "--pipeline-smoke-test")
}

pub(crate) fn run() {
    let context = tauri::generate_context!();
    #[cfg(all(feature = "ui-test", debug_assertions))]
    let context = {
        let mut context = context;
        context
            .runtime_authority_mut()
            .add_capability(include_str!("../ui-test/capability.json"))
            .expect("UI test capability");
        context
    };
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
            app.state::<models::Operations>().cancel_transfer();
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
    let state = vz_core::active_index::resolve(&data_root(app)?);
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    let selected = model_dir(app)?.join("models.selected.json");
    std::fs::create_dir_all(model_dir(app)?).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(&selected_model_pins(app)?).map_err(|e| e.to_string())?;
    if std::fs::read(&selected).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&selected, bytes).map_err(|e| e.to_string())?;
    }
    if resources.join("models.json").is_file() {
        return Ok((state, installed_assets(app)?, selected));
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
    Ok((state, assets, selected))
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
    let _timer = vz_core::engine::PhaseTimer::new("pipeline_index_total");
    models::validate_folder(&folder)?;
    let guard = operations.acquire()?;
    let cancel = operations.start();
    #[cfg(feature = "smoke-test")]
    let hold =
        smoke_test_requested() && HOLD_NEXT_INDEX.swap(false, std::sync::atomic::Ordering::SeqCst);
    #[cfg(feature = "smoke-test")]
    if hold {
        RELEASE_INDEX.store(false, std::sync::atomic::Ordering::SeqCst);
        tauri::Manager::get_webview_window(&app, "main")
            .expect("main window")
            .eval("window.__VZ_SMOKE__.indexLocked = true")
            .map_err(|e| e.to_string())?;
    }
    let data = data_root(&app)?;
    let (_, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let mut cache = guard;
        #[cfg(feature = "smoke-test")]
        if hold {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !RELEASE_INDEX.load(std::sync::atomic::Ordering::SeqCst) && !*cancel.borrow() {
                if std::time::Instant::now() >= deadline {
                    return Err("Smoke index release timed out".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        let mut run = || -> anyhow::Result<serde_json::Value> {
            let engines = cache.get_or_load(false, || {
                vz_core::engine::Engines::load_embedding(&assets, &pins, pipeline_cpu_requested())
            })?;
            #[cfg(feature = "smoke-test")]
            record_loaded_models(&assets, &pins, engines.chat_loaded())?;
            let result =
                tokio::runtime::Runtime::new()?.block_on(vz_core::active_index::reindex(
                    &data,
                    std::path::Path::new(&folder),
                    engines,
                    &assets,
                    |value| {
                        let _ = progress.send(value.clone());
                    },
                    || *cancel.borrow(),
                ))?;
            Ok(serde_json::to_value(result)?)
        };
        run().map_err(|e| {
            let detail = format!("{e:#}");
            if detail.starts_with("Re-index did not finish") {
                detail
            } else {
                format!("Re-index did not finish: {detail}")
            }
        })
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
    let _timer = vz_core::engine::PhaseTimer::new("pipeline_search_total");
    let guard = operations.acquire()?;
    desktop_pipeline::require_index(&vz_core::active_index::resolve(&data_root(&app)?))?;
    #[cfg(feature = "smoke-test")]
    let hold =
        smoke_test_requested() && HOLD_NEXT_SEARCH.swap(false, std::sync::atomic::Ordering::SeqCst);
    #[cfg(feature = "smoke-test")]
    if hold {
        RELEASE_SEARCH.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(feature = "smoke-test")]
    if smoke_test_requested() {
        tauri::Manager::get_webview_window(&app, "main")
            .expect("main window")
            .eval("window.__VZ_SMOKE__.searchLocked = true")
            .map_err(|e| e.to_string())?;
    }
    let (root, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let mut cache = guard;
        #[cfg(feature = "smoke-test")]
        if hold {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !RELEASE_SEARCH.load(std::sync::atomic::Ordering::SeqCst) {
                if std::time::Instant::now() >= deadline {
                    return Err("Smoke Search release timed out".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        let mut run = || -> anyhow::Result<serde_json::Value> {
            let engines = cache.get_or_load(false, || {
                vz_core::engine::Engines::load_embedding(&assets, &pins, pipeline_cpu_requested())
            })?;
            #[cfg(feature = "smoke-test")]
            record_loaded_models(&assets, &pins, engines.chat_loaded())?;
            let (state, _) = vz_core::storage::State::open_index(&root, &engines.recipe)?;
            let hits = tokio::runtime::Runtime::new()?.block_on(state.search(
                &query,
                &engines.vector(&query, true)?,
                5,
            ))?;
            let names = desktop_pipeline::file_names(&state);
            Ok(serde_json::Value::Array(hits.into_iter().map(|hit| serde_json::json!({
                "file_name": names.get(&hit.chunk.doc_id),
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
    let _timer = vz_core::engine::PhaseTimer::new("pipeline_ask_total");
    let guard = operations.acquire()?;
    let (manifest, settings) = manifest_selection(&app)?;
    let answers = manifest.answers().map_err(|e| e.to_string())?;
    let Some(pin) = answers
        .iter()
        .find(|pin| Some(pin.sha256.as_str()) == settings.selected())
    else {
        return Ok(serde_json::json!({"kind":"no_answer"}));
    };
    match answer_state(&app, &operations, pin).as_str() {
        "checking" => return Ok(serde_json::json!({"kind":"checking"})),
        "ready" => {}
        state => return Ok(serde_json::json!({"kind":"failed","state":state})),
    }

    desktop_pipeline::require_index(&vz_core::active_index::resolve(&data_root(&app)?))?;
    let (root, assets, pins) = pipeline_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let mut cache = guard;
        let mut run = || -> anyhow::Result<serde_json::Value> {
            let engines = cache.get_or_load(true, || {
                vz_core::engine::Engines::load_embedding(&assets, &pins, pipeline_cpu_requested())
            })?;
            #[cfg(feature = "smoke-test")]
            record_loaded_models(&assets, &pins, engines.chat_loaded())?;
            let (state, _) = vz_core::storage::State::open_index(&root, &engines.recipe)?;
            let hits = tokio::runtime::Runtime::new()?.block_on(state.search(
                &question,
                &engines.vector(&question, true)?,
                3,
            ))?;
            let answer = vz_core::index::ask(engines, &question, &hits, |token| {
                let _ = tokens.send(token.to_owned());
            })?;
            let names = desktop_pipeline::file_names(&state);
            let mut value = serde_json::to_value(answer)?;
            value["kind"] = serde_json::json!("answer");
            if let Some(citations) = value["citations"].as_array_mut() {
                for citation in citations {
                    citation["file_name"] = citation["doc_id"]
                        .as_str()
                        .and_then(|id| names.get(id))
                        .map_or(serde_json::Value::Null, |name| serde_json::json!(name));
                }
            }
            Ok(value)
        };
        run().map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn data_root<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<std::path::PathBuf, String> {
    #[cfg(all(feature = "ui-test", debug_assertions))]
    {
        let _ = app;
        let root = std::path::PathBuf::from(
            std::env::var_os("VZ_UI_TEST_DATA_ROOT").ok_or("UI test data root is required")?,
        );
        if !root.is_absolute() {
            return Err("UI test data root must be absolute".into());
        }
        Ok(root)
    }
    #[cfg(not(all(feature = "ui-test", debug_assertions)))]
    {
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
}
fn model_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<std::path::PathBuf, String> {
    Ok(data_root(app)?.join("models"))
}
fn model_pin_path<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    #[cfg(all(feature = "smoke-test", debug_assertions))]
    if smoke_test_requested() {
        if let Some(path) = std::env::var_os("VZ_SMOKE_MODELS") {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() {
                return Err("Smoke model pins must be absolute".into());
            }
            return Ok(path);
        }
    }
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    Ok(if resources.join("models.json").is_file() {
        resources.join("models.json")
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join("spike/fixtures/models.json")
    })
}
fn manifest_selection<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(models::Manifest, settings::Settings), String> {
    let manifest = models::Manifest::read(&model_pin_path(app)?).map_err(|e| format!("{e:#}"))?;
    let settings = settings::migrate(
        &model_dir(app)?,
        &manifest.embedding,
        &manifest.answers().map_err(|e| e.to_string())?,
        models::total_ram_bytes().map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{e:#}"))?;
    Ok((manifest, settings))
}
fn selected_model_pins<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<serde_json::Value, String> {
    let (manifest, settings) = manifest_selection(app)?;
    manifest
        .selected(settings.selected())
        .map_err(|e| e.to_string())
}

fn answer_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    operations: &models::Operations,
    pin: &models::Pin,
) -> String {
    let Ok(dir) = model_dir(app) else {
        return "failed".into();
    };
    let path = dir.join(&pin.file);
    let Ok(stamp) = vz_core::verification::stamp(&path) else {
        return "missing".into();
    };
    if vz_core::verification::recorded(&path, &pin.sha256, pin.size) {
        return "ready".into();
    }
    let mut checks = operations.checks.lock().unwrap();
    if let Some((checked_stamp, state)) = checks.get(&pin.sha256) {
        if *checked_stamp == stamp {
            return state.clone();
        }
    }
    checks.insert(pin.sha256.clone(), (stamp.clone(), "checking".into()));
    drop(checks);
    let checks = operations.checks.clone();
    let pin = pin.clone();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Emitter;
        let verified = models::verified(&dir, &pin);
        let mut checks = checks.lock().unwrap();
        if vz_core::verification::stamp(&path).ok().as_ref() == Some(&stamp)
            && checks
                .get(&pin.sha256)
                .is_some_and(|(current, _)| *current == stamp)
        {
            checks.insert(
                pin.sha256,
                (stamp, if verified { "ready" } else { "failed" }.into()),
            );
        }
        drop(checks);
        let _ = app.emit("models_changed", ());
    });
    "checking".into()
}

#[tauri::command]
async fn models_status<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
) -> Result<serde_json::Value, String> {
    let dir = model_dir(&app)?;
    let (manifest, settings) = manifest_selection(&app)?;
    let selected = manifest
        .selected(settings.selected())
        .map_err(|e| e.to_string())?;
    let root = vz_core::active_index::resolve(&data_root(&app)?);
    let search = manifest.embedding.clone();
    let search_dir = dir.clone();
    let verified =
        tauri::async_runtime::spawn_blocking(move || models::verified(&search_dir, &search))
            .await
            .map_err(|e| e.to_string())?;
    let answers = manifest.answers().map_err(|e| e.to_string())?;
    let active = answers
        .iter()
        .find(|pin| Some(pin.sha256.as_str()) == settings.selected());
    let answer = match active {
        Some(pin) => {
            serde_json::json!({"state":answer_state(&app, &operations, pin),"pin":pin.metadata})
        }
        None => serde_json::json!({"state":"none"}),
    };
    let files: Vec<_> = std::iter::once(&manifest.embedding)
        .chain(answers.iter())
        .map(|pin| {
            let mut value = pin.metadata.clone();
            value["verified"] = serde_json::json!(vz_core::verification::recorded(
                &dir.join(&pin.file),
                &pin.sha256,
                pin.size
            ));
            value["installed_bytes"] =
                serde_json::json!(models::regular(&dir.join(&pin.file), false)
                    .and_then(|file| Ok(file.metadata()?.len()))
                    .unwrap_or(0));
            value["partial_bytes"] = serde_json::json!(models::regular(
                &dir.join(format!("{}.partial", pin.file)),
                false
            )
            .and_then(|file| Ok(file.metadata()?.len()))
            .unwrap_or(0));
            value
        })
        .collect();
    let ram = models::total_ram_bytes().map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({"ready":verified,"search":{"pin":manifest.embedding.metadata,"verified":verified},"answer":answer,
        "target":operations.target.read().unwrap().clone(),"offer":manifest.offer(ram),"ram_bytes":ram,"files":files,
        "last_indexed_folder":vz_core::active_index::last_indexed_folder(&root),
        "needs_reindex":vz_core::active_index::needs_pointer_repair(&data_root(&app)?) || models::needs_reindex(&root,&selected).map_err(|e| e.to_string())?}),
    )
}

#[allow(clippy::too_many_arguments)]
async fn acquire_pin<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    operations: &models::Operations,
    pin: &models::Pin,
    cancel: tokio::sync::watch::Receiver<bool>,
    source: Option<std::fs::File>,
    progress: &mut (impl FnMut(serde_json::Value) + Send),
    protects_in_use: bool,
) -> Result<(), String> {
    use tauri::Emitter;
    let dir = model_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut settings = settings::Settings::read(&dir).map_err(|e| e.to_string())?;
    if !settings.initialised() {
        settings.select(None);
        settings.save(&dir).map_err(|e| e.to_string())?;
    }
    #[cfg(feature = "smoke-test")]
    if smoke_test_requested() && HOLD_NEXT_TRANSFER.swap(false, std::sync::atomic::Ordering::SeqCst)
    {
        operations.set_target(pin, "downloading", None);
        RELEASE_TRANSFER.store(false, std::sync::atomic::Ordering::SeqCst);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !RELEASE_TRANSFER.load(std::sync::atomic::Ordering::SeqCst) {
            if *cancel.borrow() {
                return Err("Model acquisition cancelled; partial download kept for resume".into());
            }
            if std::time::Instant::now() >= deadline {
                return Err("Smoke transfer release timed out".into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    }
    operations.set_target(pin, "checking", None);
    let check_dir = dir.clone();
    let check_pin = pin.clone();
    let verified =
        tauri::async_runtime::spawn_blocking(move || models::verified(&check_dir, &check_pin))
            .await
            .map_err(|e| e.to_string())?;
    if !verified {
        let needed = if source.is_some() {
            pin.size.saturating_add(pin.size.div_ceil(10))
        } else {
            models::space_needed(&dir, std::slice::from_ref(pin), false)
                .map_err(|e| e.to_string())?
        };
        if let Err(error) = models::check_space(&dir, needed) {
            operations.set_target(pin, "failed", None);
            return Err(error.to_string());
        }
        operations.set_target(pin, "downloading", None);
        let client = models::client().map_err(|e| e.to_string())?;
        let source = match source {
            Some(file) => models::Source::Import(file),
            None => models::Source::Download(&client),
        };
        let result = models::stage_verify_promote_guarded(
            &dir,
            pin,
            source,
            cancel.clone(),
            &mut |value| {
                let state = value["stage"].as_str().unwrap_or("downloading");
                operations.set_target(pin, state, Some(value.clone()));
                #[cfg(feature = "smoke-test")]
                if smoke_test_requested()
                    && (value["stage"].is_string() || value["downloaded"] == 0)
                {
                    println!("VECTORAIZ_MODEL_TRANSFER: {value}");
                }
                progress(value);
                let _ = app.emit("models_changed", ());
            },
            if protects_in_use {
                Some(operations.promotion_lock())
            } else {
                None
            },
        )
        .await;
        if let Err(error) = result {
            operations.set_target(
                pin,
                if *cancel.borrow() { "paused" } else { "failed" },
                None,
            );
            let _ = app.emit("models_changed", ());
            return Err(format!("{error:#}"));
        }
    }
    if *cancel.borrow() {
        operations.set_target(pin, "paused", None);
        return Err("Model acquisition cancelled; partial download kept for resume".into());
    }
    operations.set_target(pin, "verified", None);
    let _ = app.emit("models_changed", ());
    Ok(())
}

#[tauri::command]
async fn models_download<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    progress: tauri::ipc::Channel<serde_json::Value>,
) -> Result<(), String> {
    use tauri::Emitter;
    let _transfer = operations.acquire_transfer()?;
    let cancel = operations.start_transfer();
    let (manifest, _) = manifest_selection(&app)?;
    acquire_pin(
        &app,
        &operations,
        &manifest.embedding,
        cancel,
        None,
        &mut |value| {
            let _ = progress.send(value);
        },
        true,
    )
    .await?;
    *operations.target.write().unwrap() = None;
    let _ = app.emit("models_changed", ());
    Ok(())
}

async fn answer_acquire<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    tier_index: usize,
    switch: bool,
) -> Result<(), String> {
    use tauri::Emitter;
    let _transfer = operations.acquire_transfer()?;
    let cancel = operations.start_transfer();
    let (manifest, settings) = manifest_selection(&app)?;
    let tier = manifest
        .offer(models::total_ram_bytes().map_err(|e| e.to_string())?)
        .into_iter()
        .find(|tier| tier["tier_index"] == tier_index)
        .ok_or("Answer tier unavailable for this computer")?;
    let pin = manifest
        .answers()
        .map_err(|e| e.to_string())?
        .get(tier_index)
        .cloned()
        .ok_or("Unknown answer tier")?;
    let _ = tier;
    if !switch && settings.selected() == Some(pin.sha256.as_str()) {
        return Ok(());
    }
    acquire_pin(
        &app,
        &operations,
        &pin,
        cancel.clone(),
        None,
        &mut |_| {},
        settings.selected() == Some(pin.sha256.as_str()),
    )
    .await?;
    if switch
        || settings::Settings::read(&model_dir(&app)?)
            .map_err(|e| e.to_string())?
            .selected()
            .is_none()
    {
        let cache = operations.wait_inference().await;
        if *cancel.borrow() {
            operations.set_target(&pin, "paused", None);
            return Err("Model acquisition cancelled; partial download kept for resume".into());
        }
        let dir = model_dir(&app)?;
        let mut settings = settings::Settings::read(&dir).map_err(|e| e.to_string())?;
        if switch || settings.selected().is_none() {
            settings.select(Some(&pin.sha256));
            settings.save(&dir).map_err(|e| e.to_string())?;
            cache
                .set_chat(Some(pin.metadata))
                .map_err(|e| e.to_string())?;
        }
    }
    *operations.target.write().unwrap() = None;
    let _ = app.emit("models_changed", ());
    Ok(())
}
#[tauri::command]
async fn answer_add<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    tier_index: usize,
) -> Result<(), String> {
    answer_acquire(app, operations, tier_index, false).await
}
#[tauri::command]
async fn answer_switch<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    tier_index: usize,
) -> Result<(), String> {
    answer_acquire(app, operations, tier_index, true).await
}
#[tauri::command]
async fn answer_turn_off<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
) -> Result<(), String> {
    use tauri::Emitter;
    let cache = operations.wait_inference().await;
    let dir = model_dir(&app)?;
    let mut settings = settings::Settings::read(&dir).map_err(|e| e.to_string())?;
    settings.select(None);
    settings.save(&dir).map_err(|e| e.to_string())?;
    cache.set_chat(None).map_err(|e| e.to_string())?;
    let _ = app.emit("models_changed", ());
    Ok(())
}
#[tauri::command]
fn models_cancel(
    operations: tauri::State<'_, models::Operations>,
    kind: String,
) -> Result<(), String> {
    match kind.as_str() {
        "transfer" => operations.cancel_transfer(),
        "pipeline" => operations.cancel(),
        _ => return Err("Unknown operation kind".into()),
    }
    Ok(())
}
#[tauri::command]
async fn models_import<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    folder: String,
) -> Result<Vec<serde_json::Value>, String> {
    use tauri::Emitter;
    models::validate_import_folder(std::path::Path::new(&folder)).map_err(|e| e.to_string())?;
    let _transfer = operations.acquire_transfer()?;
    let cancel = operations.start_transfer();
    let (manifest, settings) = manifest_selection(&app)?;
    let mut pins = manifest.answers().map_err(|e| e.to_string())?;
    pins.insert(0, manifest.embedding.clone());
    let mut results = Vec::new();
    for pin in pins {
        if *cancel.borrow() {
            break;
        }
        let check_dir = model_dir(&app)?;
        let check_pin = pin.clone();
        if tauri::async_runtime::spawn_blocking(move || models::verified(&check_dir, &check_pin))
            .await
            .map_err(|e| e.to_string())?
        {
            results.push(serde_json::json!({"file":pin.file,"result":"already installed"}));
            continue;
        }
        let result = match models::regular(&std::path::Path::new(&folder).join(&pin.file), false) {
            Ok(file) => {
                if vz_core::verification::recorded(
                    &model_dir(&app)?.join(&pin.file),
                    &pin.sha256,
                    pin.size,
                ) {
                    Ok("already installed")
                } else {
                    acquire_pin(
                        &app,
                        &operations,
                        &pin,
                        cancel.clone(),
                        Some(file),
                        &mut |_| {},
                        pin.sha256 == manifest.embedding.sha256
                            || Some(pin.sha256.as_str()) == settings.selected(),
                    )
                    .await
                    .map(|()| "imported")
                }
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok("missing")
            }
            Err(error) => Err(format!("{error:#}")),
        };
        results.push(serde_json::json!({"file":pin.file,"result":match result {Ok(message)=>message.into(),Err(error)=>error}}));
    }
    *operations.target.write().unwrap() = None;
    let _ = app.emit("models_changed", ());
    Ok(results)
}
#[tauri::command]
async fn delete_unused_model<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    operations: tauri::State<'_, models::Operations>,
    sha256: String,
) -> Result<u64, String> {
    use tauri::Emitter;
    let _transfer = operations
        .acquire_transfer()
        .map_err(|_| "A download is running".to_owned())?;
    let mut cancel = operations.start_transfer();
    let _cache = tokio::select! { cache = operations.wait_inference() => cache, _ = cancel.changed() => return Err("Delete cancelled".into()) };
    let (manifest, settings) = manifest_selection(&app)?;
    if sha256 == manifest.embedding.sha256 {
        return Err("The search model cannot be deleted".into());
    }
    if Some(sha256.as_str()) == settings.selected() {
        return Err("Turn off or switch the answer model before deleting it".into());
    }
    let pin = manifest
        .answers()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|pin| pin.sha256 == sha256)
        .ok_or("Unknown model")?;
    let dir = model_dir(&app)?;
    let mut freed = 0;
    for path in [
        dir.join(&pin.file),
        dir.join(format!("{}.partial", pin.file)),
    ] {
        match models::regular(&path, false) {
            Ok(file) => {
                let size = file.metadata().map_err(|e| e.to_string())?.len();
                drop(file);
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                freed += size;
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(format!("{error:#}")),
        }
    }
    vz_core::verification::forget(&dir.join(&pin.file)).map_err(|e| e.to_string())?;
    operations.checks.lock().unwrap().remove(&sha256);
    let _ = app.emit("models_changed", ());
    Ok(freed)
}
#[cfg(feature = "smoke-test")]
fn record_loaded_models(
    assets: &std::path::Path,
    pins: &std::path::Path,
    chat_loaded: bool,
) -> anyhow::Result<()> {
    if let Some(report) = std::env::var_os("VZ_LOADED_MODELS_REPORT") {
        let mut pins = models::pins(pins)?;
        if !chat_loaded {
            pins.truncate(1); // Embedding only until Ask first loads chat.
        }
        anyhow::ensure!(
            pins.iter()
                .all(|p| models::regular(&assets.join(&p.file), false)
                    .and_then(|file| Ok(file.metadata()?.len() == p.size))
                    .unwrap_or(false)),
            "Loaded model sizes must match acquisition pins"
        );
        // These paths were hash-verified when this cache generation loaded.
        // Mutation drops the cache before any file can be replaced. Report only
        // actually loaded models, including chat retained after an earlier Ask.
        // The install harness independently hashes the files after exit.
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
