// Generate Tauri's required placeholder icon as a build input only.
// No image asset or installer is published by the scaffold.
fn main() {
    let png: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 32, 0, 0, 0, 32, 8,
        6, 0, 0, 0, 115, 122, 122, 244, 0, 0, 0, 47, 73, 68, 65, 84, 120, 156, 237, 206, 33, 1, 0,
        0, 8, 3, 48, 146, 146, 154, 20, 148, 128, 24, 55, 19, 243, 171, 222, 185, 164, 18, 16, 16,
        16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 72, 7, 30, 20, 135, 108, 181, 23, 26, 90,
        52, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let generated = std::path::Path::new("gen");
    std::fs::create_dir_all(generated).expect("create generated assets");
    std::fs::write(generated.join("icon.png"), png).expect("write build icon");
    let mut ico = vec![0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 32, 0];
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22_u32.to_le_bytes());
    ico.extend_from_slice(png);
    std::fs::write(generated.join("icon.ico"), ico).expect("write Windows build icon");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(
                tauri_build::WindowsAttributes::new().window_icon_path("gen/icon.ico"),
            )
            .app_manifest(tauri_build::AppManifest::new().commands(&[
                "self_test",
                "models_status",
                "models_download",
                "models_cancel",
                "models_import",
                "pipeline_index",
                "pipeline_search",
                "pipeline_ask",
                "update_check",
                "update_install",
            ])),
    )
    .expect("failed to build Tauri app");
}
