//! Test-only process used to inject platform-independent crashes and hangs.
fn main() {
    let request: vz_core::worker::Request =
        vz_core::worker::read_frame(std::io::stdin().lock()).unwrap();
    let path = request.path.to_str().unwrap();
    if let Some(port) = path.strip_prefix("network:") {
        let error = std::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .err()
            .map(|e| e.to_string());
        vz_core::worker::write_frame(
            std::io::stdout().lock(),
            &vz_core::worker::Response {
                pages: vec![],
                error,
                content_failure: false,
            },
        )
        .unwrap();
        return;
    }
    // Deliberately bypass the PDF guard in this separate test executable.
    // Resource probes use file bytes so the same supervisor/request boundary is
    // exercised without executing an actual hostile PDF or exhausting the OS.
    if let Ok(bytes) = std::fs::read(&request.path) {
        if matches!(
            bytes.as_slice(),
            b"VZ_RESOURCE_PROBE_RSS\n" | b"VZ_RESOURCE_PROBE_DEADLINE\n"
        ) {
            let job = request
                .projection
                .as_ref()
                .expect("projection resource probe");
            std::fs::write(job.output.join("resource-probe-started"), &bytes).unwrap();
        }
        match bytes.as_slice() {
            b"VZ_RESOURCE_PROBE_RSS\n" => {
                let mut buffers = Vec::new();
                for _ in 0..16 {
                    let mut bytes = Vec::new();
                    if let Err(error) = bytes.try_reserve_exact(16 * 1024 * 1024) {
                        let _ = vz_core::worker::write_frame(
                            std::io::stdout().lock(),
                            &vz_core::worker::Response {
                                pages: vec![],
                                error: Some(format!("TEST_PROBE_ALLOCATION_DENIED: {error}")),
                                content_failure: false,
                            },
                        );
                        return;
                    }
                    bytes.resize(16 * 1024 * 1024, 0u8);
                    for byte in bytes.iter_mut().step_by(4096) {
                        *byte = 42;
                    }
                    buffers.push(bytes);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                std::hint::black_box(&buffers);
                std::thread::sleep(std::time::Duration::from_secs(30));
                return;
            }
            b"VZ_RESOURCE_PROBE_DEADLINE\n" => {
                std::thread::sleep(std::time::Duration::from_secs(30));
                return;
            }
            _ => (),
        }
    }
    match path {
        "pdfium-io" => {
            let error = vz_core::failures::parser(
                pdfium_render::prelude::PdfiumError::IoError(std::io::Error::from(
                    std::io::ErrorKind::Other,
                ))
                .into(),
            );
            vz_core::worker::write_frame(
                std::io::stdout().lock(),
                &vz_core::worker::Response {
                    pages: vec![],
                    error: Some(format!("{error:#}")),
                    content_failure: vz_core::failures::is_content(&error),
                },
            )
            .unwrap();
        }
        "memory" => {
            let mut buffers = Vec::new();
            loop {
                let mut bytes = vec![0_u8; 16 * 1024 * 1024];
                for byte in bytes.iter_mut().step_by(4096) {
                    *byte = 42;
                }
                buffers.push(bytes);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        // Any unsuccessful exit without a response is classified as crashed.
        // A dedicated code exercises that path without OS crash reports/dialogs.
        "crash" => std::process::exit(86),
        "timeout" => std::thread::sleep(std::time::Duration::from_secs(30)),
        _ => {
            vz_core::worker::write_frame(
                std::io::stdout().lock(),
                &vz_core::worker::Response {
                    pages: vec![],
                    error: None,
                    content_failure: false,
                },
            )
            .unwrap();
        }
    }
}
