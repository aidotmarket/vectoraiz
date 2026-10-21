use std::{path::Path, time::Duration};
use vz_core::worker::{run, Request};
#[test]
fn failed_file_does_not_prevent_next_worker() {
    let dir = std::env::temp_dir().join(format!("vz-worker-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.doc");
    std::fs::write(&bad, "bad").unwrap();
    let good = dir.join("good.txt");
    std::fs::write(&good, "working").unwrap();
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-parser-worker"));
    assert!(run(
        worker,
        &Request {
            path: bad,
            pdfium: None
        },
        Duration::from_secs(10)
    )
    .is_err());
    assert_eq!(
        run(
            worker,
            &Request {
                path: good,
                pdfium: None
            },
            Duration::from_secs(10)
        )
        .unwrap()[0]
            .text,
        "working"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn timeout_and_crash() {
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"));
    let request = |path: &str| Request {
        path: path.into(),
        pdfium: None,
    };
    assert!(run(worker, &request("timeout"), Duration::from_millis(500))
        .unwrap_err()
        .to_string()
        .contains("timeout"));
    assert!(run(worker, &request("crash"), Duration::from_secs(5)).is_err());
    assert!(run(worker, &request("ok"), Duration::from_secs(5)).is_ok());
}
#[cfg(target_os = "macos")]
#[test]
fn macos_worker_denies_outbound_network() {
    use std::process::{Command, Stdio};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"));
    let request = Request {
        path: format!("network:{port}").into(),
        pdfium: None,
    };
    let mut baseline = Command::new(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    vz_core::worker::write_frame(baseline.stdin.take().unwrap(), &request).unwrap();
    let response: vz_core::worker::Response =
        vz_core::worker::read_frame(baseline.stdout.take().unwrap()).unwrap();
    assert!(baseline.wait().unwrap().success());
    assert!(
        response.error.is_none(),
        "baseline listener must be reachable"
    );
    assert!(
        run(worker, &request, Duration::from_secs(5)).is_err(),
        "sandboxed worker must fail connection"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_memory_budget_kills_worker() {
    let request = Request {
        path: "memory".into(),
        pdfium: None,
    };
    let error = run(
        Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe")),
        &request,
        Duration::from_secs(30),
    )
    .unwrap_err();
    assert!(error.to_string().contains("memory"), "{error:#}");
}
