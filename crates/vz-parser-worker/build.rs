fn main() {
    println!("cargo:rerun-if-changed=src/office-bootstrap.c");
    println!("cargo:rerun-if-changed=src/office-fonts.c");
    println!("cargo:rerun-if-changed=src/office-font-files.inc");
    println!("cargo:rerun-if-changed=../../tests/office/fixtures/hostile-worker.c");
    if std::env::var_os("CARGO_FEATURE_OFFICE_FEASIBILITY").is_none()
        || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
    {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    for argv in [
        vec![
            "xcrun".into(),
            "clang".into(),
            "-c".into(),
            "-Wno-deprecated-declarations".into(),
            "src/office-fonts.c".into(),
            "-o".into(),
            out.join("fonts.o").into_os_string(),
        ],
        vec![
            "xcrun".into(),
            "clang".into(),
            "-c".into(),
            "-fblocks".into(),
            "-Wno-deprecated-declarations".into(),
            "src/office-bootstrap.c".into(),
            "-o".into(),
            out.join("bootstrap.o").into_os_string(),
        ],
        vec![
            "ar".into(),
            "rcs".into(),
            out.join("liboffice_bootstrap.a").into_os_string(),
            out.join("bootstrap.o").into_os_string(),
            out.join("fonts.o").into_os_string(),
        ],
    ] {
        assert!(std::process::Command::new("rtk")
            .arg("proxy")
            .args(argv)
            .status()
            .unwrap()
            .success());
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=office_bootstrap");
    println!("cargo:rustc-link-lib=sandbox");
    println!("cargo:rustc-link-lib=framework=CoreServices");
}
