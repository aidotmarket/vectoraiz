#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
fn self_test<R: tauri::Runtime>(_app: tauri::AppHandle<R>) -> vz_core::SelfTestResult {
    let result = vz_core::self_test();
    #[cfg(feature = "smoke-test")]
    if smoke_test_requested() {
        assert_eq!(result.status, "OK");
        println!("VECTORAIZ_SMOKE_TEST_OK");
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
            webview.eval(
                "const timer = setInterval(() => { const button = document.querySelector('button'); if (button) { clearInterval(timer); button.click(); } }, 50);"
            ).expect("start smoke-test button click");
        }
    });
    builder.invoke_handler(tauri::generate_handler![self_test])
}

#[cfg(feature = "smoke-test")]
fn smoke_test_requested() -> bool {
    std::env::args().any(|arg| arg == "--smoke-test")
}

#[cfg(not(test))]
fn main() {
    configure(tauri::Builder::default())
        .run(tauri::generate_context!())
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
