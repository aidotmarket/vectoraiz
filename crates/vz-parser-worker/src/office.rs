//! Isolated real ingest/render experiment, never used by the product.
#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use anyhow::{ensure, Context};
    use pdfium_render::prelude::*;
    use std::ffi::CString;
    unsafe extern "C" {
        fn vz_office_reserve_baseline() -> i32;
        fn vz_office_supply_fds(input: i32, scratch: i32, assets: i32);
        fn vz_office_socket_checks(close_fd: i32) -> i32;
        fn vz_office_bootstrap(profile: *const std::ffi::c_char) -> i32;
        fn vz_office_probe_main(argc: i32, argv: *mut *mut std::ffi::c_char) -> i32;
        fn vz_office_memory_probe(
            kind: *const std::ffi::c_char,
            scratch: *const std::ffi::c_char,
        ) -> i32;
    }
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() >= 4,
        "profile, role, pinned PDFium path required"
    );
    let profile = CString::new(std::fs::read_to_string(&args[1])?)?;
    let asset = std::fs::File::open(&args[3])?;
    // Load required runtime assets before baseline, without accepting input.
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&args[3])?);
    if args[2] == "memory" && args.get(4).is_some_and(|s| s == "reserved-dirty") {
        ensure!(
            unsafe { vz_office_reserve_baseline() } == 0,
            "baseline reservation"
        );
    }
    use std::os::fd::AsRawFd;
    let real_role = matches!(
        args[2].as_str(),
        "ingest" | "render" | "control-ingest" | "control-render"
    );
    let mut input = if matches!(args[2].as_str(), "probe" | "control-probe")
        && args.get(4).is_some_and(|p| p == "snapshot")
    {
        Some(std::fs::File::open(args.get(5).context("probe snapshot")?)?)
    } else if real_role {
        Some(std::fs::File::open(
            args.get(4).context("exact supplied input")?,
        )?)
    } else {
        None
    };
    let scratch = if let Some(path) = args.get(if real_role || args[2] == "memory" {
        5
    } else if args.get(4).is_some_and(|p| p == "scratch_create") {
        6
    } else {
        99
    }) {
        Some(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)?,
        )
    } else {
        None
    };
    let closing = std::fs::File::open("/dev/null")?;
    unsafe {
        vz_office_supply_fds(
            input
                .as_ref()
                .map_or(closing.as_raw_fd(), |f| f.as_raw_fd()),
            scratch.as_ref().map_or(-1, |f| f.as_raw_fd()),
            asset.as_raw_fd(),
        );
    }
    // Independent positive controls never receive untrusted request bytes.
    if args[2] != "control-probe" && !args[2].starts_with("control-") {
        let status = unsafe { vz_office_bootstrap(profile.as_ptr()) };
        ensure!(status == 0, "bootstrap refused: {status}");
    }
    if args[2] == "socket-checks" || args[2] == "control-sockets" {
        std::process::exit(unsafe { vz_office_socket_checks(closing.as_raw_fd()) });
    }
    if matches!(args[2].as_str(), "probe" | "control-probe") {
        let strings: Vec<_> = args[4..]
            .iter()
            .map(|a| CString::new(a.as_str()))
            .collect::<Result<_, _>>()?;
        ensure!(strings.len() == 4, "probe arguments");
        let program = CString::new("office-worker")?;
        let mut pointers = vec![program.as_ptr() as *mut _];
        pointers.extend(strings.iter().map(|s| s.as_ptr() as *mut _));
        pointers.push(std::ptr::null_mut());
        std::process::exit(unsafe { vz_office_probe_main(5, pointers.as_mut_ptr()) });
    }
    if args[2] == "memory" {
        let kind = CString::new(args.get(4).context("memory kind")?.as_str())?;
        let scratch = CString::new(args.get(5).context("scratch")?.as_str())?;
        std::process::exit(unsafe { vz_office_memory_probe(kind.as_ptr(), scratch.as_ptr()) });
    }
    ensure!(real_role, "unknown role");
    vz_core::worker::write_frame(
        std::io::stdout().lock(),
        &serde_json::json!({"ready":true,"role":args[2]}),
    )?;
    let request: vz_core::worker::Request = vz_core::worker::read_frame(std::io::stdin().lock())?;
    ensure!(
        request.path == std::path::Path::new(&args[4]),
        "input identity mismatch"
    );
    ensure!(
        request.pdfium.as_deref() == Some(std::path::Path::new(&args[3])),
        "asset mismatch"
    );
    use std::io::Read;
    let mut bytes = Vec::new();
    input
        .take()
        .context("supplied descriptor")?
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 32 * 1024 * 1024, "input budget");
    let mut output = Vec::new();
    if request.path.extension().is_some_and(|e| e == "pdf") {
        let document = pdfium.load_pdf_from_byte_slice(&bytes, None)?;
        for i in 0..document.pages().len() {
            let page = document.pages().get(i)?;
            let text = page.text()?.all();
            // Full page raster, including scanned pages, supplied to parent OCR.
            let image = page
                .render_with_config(&PdfRenderConfig::new().set_target_width(1600))?
                .as_image();
            ensure!(
                image.width() <= 4096 && image.height() <= 4096,
                "decoded image budget"
            );
            let mut png = std::io::Cursor::new(Vec::new());
            image.write_to(&mut png, image::ImageFormat::Png)?;
            output.push(serde_json::json!({"page":i+1,"text":text,"ocr_required":text.trim().is_empty(),"width":image.width(),"height":image.height(),"png":png.into_inner()}));
            ensure!(
                serde_json::to_vec(&output)?.len() <= vz_core::worker::MAX_FRAME,
                "output budget"
            );
        }
    } else {
        let pages = vz_core::parsers::office_bytes::parse(
            request
                .path
                .extension()
                .and_then(|e| e.to_str())
                .context("extension")?,
            &bytes,
        )?;
        for page in pages {
            // Real, escaped full-text rendering. Images/style completeness is
            // explicitly unqualified; never treat parser text as original view.
            let html = format!(
                "<article><pre>{}</pre></article>",
                page.text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            );
            output.push(serde_json::json!({"page":page.page,"text":page.text,"html":html,"images_complete":false}));
        }
    }
    vz_core::worker::write_frame(
        std::io::stdout().lock(),
        &serde_json::json!({"pages":output}),
    )?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("NOT RUN: S1934 native worker is macOS-only");
    std::process::exit(78);
}
