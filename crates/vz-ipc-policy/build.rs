// Use the app's exact command manifest, capabilities, config and build inputs.
mod desktop_build {
    include!("../../app/src-tauri/build.rs");
    pub fn build() {
        main();
    }
}

fn main() {
    let app = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri");
    for source in ["build.rs", "tauri.conf.json", "capabilities", "permissions"] {
        println!("cargo:rerun-if-changed={}", app.join(source).display());
    }
    std::env::set_current_dir(app).expect("app capability source directory");
    desktop_build::build();
}
