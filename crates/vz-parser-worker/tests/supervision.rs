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

#[test]
fn pages_without_text_use_platform_ocr_policy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = root.join(".assets").join(if cfg!(windows) {
        "pdfium/bin/pdfium.dll"
    } else {
        "pdfium/lib/libpdfium.dylib"
    });
    assert!(library.exists(), "fetch pinned PDFium before cargo test");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blank.pdf");
    // A valid page with no text objects exercises render-to-Vision on macOS,
    // and the explicit needs-OCR marker on Windows. No image is committed.
    let objects = [
        "<</Type/Catalog/Pages 2 0 R>>",
        "<</Type/Pages/Kids[3 0 R]/Count 1>>",
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Resources<<>>/Contents 4 0 R>>",
        "<</Length 0>>\nstream\n\nendstream",
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<</Size 5/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n"
    ));
    std::fs::write(&path, pdf).unwrap();
    let pages = run(
        Path::new(env!("CARGO_BIN_EXE_vz-parser-worker")),
        &Request {
            path,
            pdfium: Some(library.canonicalize().unwrap()),
        },
        Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page, 1);
    assert_eq!(pages[0].needs_ocr, cfg!(windows));
}
