// Test the production capability policy without linking inference backends.
#[cfg(test)]
mod tests {
    #[test]
    fn ipc_permissions_allow_only_pipeline_commands_in_the_local_main_window() {
        // Resolve the production, build-generated ACL directly. Even MockRuntime
        // window construction and get_ipc_response can wait on runtime callbacks;
        // permission checks need neither a window nor an unbounded channel receive.
        let mut context: tauri::Context<tauri::test::MockRuntime> =
            tauri::generate_context!("../../app/src-tauri/tauri.conf.json");
        let authority = context.runtime_authority_mut();
        let local = tauri::ipc::Origin::Local;
        let remote = tauri::ipc::Origin::Remote {
            url: "https://example.com".parse().unwrap(),
        };
        for command in [
            "self_test",
            "pipeline_index",
            "pipeline_search",
            "pipeline_ask",
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
    }
}
