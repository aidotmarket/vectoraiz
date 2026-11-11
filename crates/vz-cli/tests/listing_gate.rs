use std::process::Command;

#[test]
fn listing_commands_fail_before_loading_models_or_touching_destinations() {
    for args in [
        vec!["pack", "unused-output"],
        vec![
            "review",
            "--store",
            "unused-store",
            "--current",
            "unreadable-reference",
        ],
        vec![
            "review",
            "--store",
            "unused-store",
            "--current",
            "unreadable-reference",
            "--accept",
        ],
        vec![
            "upload",
            "unused-pack",
            "--endpoint",
            "http://127.0.0.1:1",
            "--bucket",
            "unused",
        ],
        vec![
            "export",
            "unused-pack",
            "--pack-id",
            "unused",
            "--folder",
            "unused-source",
            "--staging",
            "unused-stage",
        ],
        vec![
            "export",
            "unused-pack",
            "--pack-id",
            "unused",
            "--sftp",
            "user@127.0.0.1:1:/source",
            "--sftp-staging",
            "/stage",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_vectoraiz-cli"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains("Listing preview is not available") || error.contains("Contact IT"),
            "{error}"
        );
        assert!(!error.contains("model"), "{error}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn local_self_test_is_not_blocked_by_listing_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_vectoraiz-cli"))
        .arg("self-test")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "OK");
}
