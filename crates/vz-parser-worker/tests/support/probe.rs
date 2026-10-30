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
    match path {
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
        "crash" => std::process::abort(),
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
