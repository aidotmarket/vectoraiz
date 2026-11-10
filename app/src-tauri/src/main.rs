#![cfg_attr(not(test), windows_subsystem = "windows")]

#[cfg(all(feature = "ui-test", not(debug_assertions)))]
compile_error!("ui-test is restricted to debug builds");

// Keep inference/native backend startup out of the permission-test harness.
// On Windows the desktop-linked executable can block even on `--list`, before
// libtest enters a test. The real desktop IPC path is exercised by native-smoke.
#[cfg(any(test, feature = "smoke-test"))]
mod a9_test_hook;
mod bundled_library;
#[cfg_attr(test, allow(dead_code))]
mod cloud;
mod desktop_pipeline;
mod local_time;
#[cfg_attr(test, allow(dead_code))]
mod mcp_apps;
#[cfg_attr(test, allow(dead_code))]
mod mcp_channel;
#[cfg(test)]
mod mcp_tests;
#[cfg_attr(test, allow(dead_code))]
mod mcp_tools;
#[cfg_attr(test, allow(dead_code))]
mod models;
mod notices;
#[cfg_attr(test, allow(dead_code))]
mod settings;

#[cfg(not(test))]
mod desktop;

#[cfg(not(test))]
fn main() {
    desktop::run();
}

#[cfg(test)]
mod tests {
    #[test]
    fn ipc_permissions_allow_only_pipeline_commands_in_the_local_main_window() {
        // Resolve the production, build-generated ACL directly. Even MockRuntime
        // window construction and get_ipc_response can wait on runtime callbacks;
        // permission checks need neither a window nor an unbounded channel receive.
        let mut context: tauri::Context<tauri::test::MockRuntime> = tauri::generate_context!();
        let authority = context.runtime_authority_mut();
        let local = tauri::ipc::Origin::Local;
        let remote = tauri::ipc::Origin::Remote {
            url: "https://example.com".parse().unwrap(),
        };
        for command in [
            "open_product_page",
            "third_party_notices",
            "self_test",
            "pipeline_index",
            "pipeline_search",
            "pipeline_ask",
            "cloud_status",
            "app_status",
            "app_disclosure",
            "app_connect_claude",
            "app_connect_other",
            "app_confirm",
            "app_test_connection",
            "app_disconnect",
            "app_path_set",
            "app_access_retry",
            "app_show_extension",
            "api_key_disclosure",
            "api_key_confirm",
            "api_key_set",
            "api_key_stop",
            "api_key_cleanup_retry",
            "answer_path_set",
            "models_status",
            "models_download",
            "models_cancel",
            "models_import",
            "answer_add",
            "answer_switch",
            "answer_turn_off",
            "delete_unused_model",
        ] {
            assert!(
                authority
                    .resolve_access(command, "main", "main", &local)
                    .is_some(),
                "{command} must be allowed in the local main window"
            );
            assert!(
                authority
                    .resolve_access(command, "other", "other", &local)
                    .is_none(),
                "{command} must be denied in other windows"
            );
            assert!(
                authority
                    .resolve_access(command, "main", "main", &remote)
                    .is_none(),
                "{command} must be denied for remote origins"
            );
        }
        for command in [
            "plugin:app|version",
            "plugin:fs|read_file",
            "unknown_command",
        ] {
            assert!(
                authority
                    .resolve_access(command, "main", "main", &local)
                    .is_none(),
                "{command} must not acquire a permission"
            );
        }
        assert_eq!(vz_core::self_test().status, "OK");
    }
}
