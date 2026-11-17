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
    let failure = run(
        worker,
        &Request {
            path: bad,
            pdfium: None,
            office: None,
            projection: None,
            readback: None,
        },
        Duration::from_secs(10),
    )
    .unwrap_err();
    assert!(vz_core::failures::is_content(&failure));
    assert_eq!(
        run(
            worker,
            &Request {
                path: good,
                pdfium: None,
                office: None,
                projection: None,
                readback: None,
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
fn office_time_and_cancel_bounds() {
    use std::time::Instant;
    let request = Request {
        path: "timeout".into(),
        pdfium: None,
        projection: None,
        readback: None,
        office: Some(vz_core::worker::OfficeJob {
            name: "fixture.docx".into(),
            output: "unused".into(),
            edits: Default::default(),
            readback: false,
            sanitized_input: false,
            chart_sources: Default::default(),
        }),
    };
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"));
    for readback in [false, true] {
        let mut request = Request {
            path: request.path.clone(),
            pdfium: None,
            projection: None,
            readback: None,
            office: Some(vz_core::worker::OfficeJob {
                name: "fixture.docx".into(),
                output: "unused".into(),
                edits: Default::default(),
                readback,
                sanitized_input: false,
                chart_sources: Default::default(),
            }),
        };
        let start = Instant::now();
        let err = run(worker, &request, Duration::from_millis(100)).unwrap_err();
        assert!(err.to_string().contains("timeout"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let start = Instant::now();
        let err =
            vz_core::worker::run_cancellable(worker, &request, Duration::from_secs(120), || {
                if start.elapsed() > Duration::from_millis(50) {
                    Err(std::io::Error::other("cancel").into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(err.to_string().contains("cancel"));
        assert!(start.elapsed() < Duration::from_secs(2));
        request.path = "ok".into();
        assert!(run(worker, &request, Duration::from_secs(5)).is_ok());
    }
}
#[test]
fn crash_probe_exits_without_an_os_exception() {
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"))
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    vz_core::worker::write_frame(
        child.stdin.take().unwrap(),
        &Request {
            path: "crash".into(),
            pdfium: None,
            office: None,
            projection: None,
            readback: None,
        },
    )
    .unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(86));
}

#[test]
fn timeout_and_crash() {
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"));
    let request = |path: &str| Request {
        path: path.into(),
        pdfium: None,
        office: None,
        projection: None,
        readback: None,
    };
    for (path, expected, timeout) in [
        ("timeout", "timeout", Duration::from_millis(500)),
        ("crash", "crashed", Duration::from_secs(5)),
    ] {
        let failure = run(worker, &request(path), timeout).unwrap_err();
        assert!(failure.to_string().contains(expected));
        assert!(vz_core::failures::is_content(&failure));
        vz_core::index::Progress {
            completed: 1,
            failed: 1,
            total: 2,
            infrastructure_failed: usize::from(!vz_core::failures::is_content(&failure)),
            ..Default::default()
        }
        .ensure_publishable()
        .unwrap();
    }
    assert!(run(worker, &request("ok"), Duration::from_secs(5)).is_ok());
}
#[cfg(target_os = "macos")]
#[test]
fn macos_worker_denies_outbound_network() {
    use std::process::{Command, Stdio};
    let listener = std::net::TcpListener::bind((
        "127.0.0.1",
        std::env::var("KD_BRIDGE_PORT_BASE")
            .ok()
            .map(|p| p.parse::<u16>().expect("reserved test port"))
            .unwrap_or(0),
    ))
    .unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"));
    let request = Request {
        path: format!("network:{port}").into(),
        pdfium: None,
        office: None,
        projection: None,
        readback: None,
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
        office: None,
        projection: None,
        readback: None,
    };
    let error = run(
        Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe")),
        &request,
        Duration::from_secs(30),
    )
    .unwrap_err();
    assert!(error.to_string().contains("memory"), "{error:#}");
    assert!(vz_core::failures::is_content(&error));
}
#[cfg(target_os = "macos")]
#[test]
fn office_memory_cap_has_named_refusal() {
    let request = Request {
        path: "memory".into(),
        pdfium: None,
        projection: None,
        readback: None,
        office: Some(vz_core::worker::OfficeJob {
            name: "capacity.xlsx".into(),
            output: "unused".into(),
            edits: Default::default(),
            readback: false,
            sanitized_input: false,
            chart_sources: Default::default(),
        }),
    };
    let error = run(
        Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe")),
        &request,
        Duration::from_secs(30),
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("OFFICE_MEMORY_LIMIT_EXCEEDED")
            && message.contains("1073741824")
            && message.contains("capacity.xlsx")
            && message.contains("last observed"),
        "{message}"
    );
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
            office: None,
            projection: None,
            readback: None,
        },
        Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page, 1);
    assert_eq!(pages[0].needs_ocr, cfg!(windows));
}

#[test]
fn pdfium_io_and_spawn_failure_remain_infrastructure_through_transport() {
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"));
    let request = Request {
        path: "pdfium-io".into(),
        pdfium: None,
        office: None,
        projection: None,
        readback: None,
    };
    let failure = run(worker, &request, Duration::from_secs(5)).unwrap_err();
    assert!(!vz_core::failures::is_content(&failure));
    assert!(failure.to_string().contains("IoError"));
    let failure = run(
        Path::new("missing-worker-binary"),
        &request,
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(!vz_core::failures::is_content(&failure));
}

#[test]
fn projection_worker_time_and_cancel_bounds() {
    use std::time::Instant;
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"));
    for readback in [false, true] {
        let request = Request {
            path: "timeout".into(),
            pdfium: None,
            office: None,
            projection: (!readback).then(|| vz_core::worker::ProjectionJob {
                kind: "docx".into(),
                output: "unused".into(),
            }),
            readback: readback.then(|| vz_core::worker::ReadbackJob {
                name: "chunks.parquet".into(),
                output: "unused".into(),
                identity_terms: vec![],
            }),
        };
        let start = Instant::now();
        assert!(run(worker, &request, Duration::from_millis(100))
            .unwrap_err()
            .to_string()
            .contains("timeout"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let start = Instant::now();
        let error =
            vz_core::worker::run_cancellable(worker, &request, Duration::from_secs(120), || {
                if start.elapsed() > Duration::from_millis(50) {
                    Err(std::io::Error::other("projection cancel").into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(error.to_string().contains("projection cancel"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn projection_memory_cap_has_named_refusal() {
    let request = Request {
        path: "memory".into(),
        pdfium: None,
        office: None,
        readback: None,
        projection: Some(vz_core::worker::ProjectionJob {
            kind: "capacity.xlsx".into(),
            output: "unused".into(),
        }),
    };
    let error = run(
        Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe")),
        &request,
        Duration::from_secs(30),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("PROJECTION_MEMORY_LIMIT_EXCEEDED")
            && error.contains("1073741824")
            && error.contains("last observed"),
        "{error}"
    );
}

#[test]
fn projection_pdf_and_html_memory_profiles_are_distinct() {
    for (kind, expected) in [
        ("pdf", vz_core::worker::MEMORY_BYTES),
        ("html", vz_core::worker::OFFICE_MEMORY_BYTES),
        ("htm", vz_core::worker::OFFICE_MEMORY_BYTES),
    ] {
        let request = Request {
            path: "unused".into(),
            pdfium: None,
            office: None,
            readback: None,
            projection: Some(vz_core::worker::ProjectionJob {
                kind: kind.into(),
                output: "unused".into(),
            }),
        };
        assert_eq!(request.memory_bytes(), expected);
    }
}

#[test]
fn projection_html_resource_network_trap() {
    let port = std::env::var("KD_BRIDGE_PORT_BASE")
        .ok()
        .map(|p| p.parse::<u16>().unwrap() + 1)
        .unwrap_or(0);
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/trap",
        listener.local_addr().unwrap().port()
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trap.html");
    std::fs::write(&path, format!(r#"<link rel="stylesheet" href="{url}"><style>@import url({url});</style><script src="{url}">DROP</script><iframe src="{url}">DROP</iframe><img src="{url}"><p style="background:url({url})">INERT_BODY</p>"#)).unwrap();
    let request = Request {
        path,
        pdfium: None,
        office: None,
        readback: None,
        projection: Some(vz_core::worker::ProjectionJob {
            kind: "html".into(),
            output: dir.path().into(),
        }),
    };
    run(
        Path::new(env!("CARGO_BIN_EXE_vz-parser-worker")),
        &request,
        Duration::from_secs(10),
    )
    .unwrap();
    let p: vz_core::office::projection::Projection =
        serde_json::from_slice(&std::fs::read(dir.path().join("projection.json")).unwrap())
            .unwrap();
    assert_eq!(p.nodes.len(), 1);
    assert_eq!(p.nodes[0].text, "INERT_BODY");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "sale extraction must not fetch resources"
    );
}

#[test]
fn projection_pdf_resource_network_trap() {
    let port = std::env::var("KD_BRIDGE_PORT_BASE")
        .ok()
        .map(|p| p.parse::<u16>().unwrap() + 2)
        .unwrap_or(0);
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://127.0.0.1:{}/trap",
        listener.local_addr().unwrap().port()
    );
    // A minimal classic PDF with URI action, remote GoTo, external stream and
    // file specification. None is a supported sale-text resource fetch.
    let objects = [
        format!("<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /URI /URI ({url}) >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /Annots [<< /Subtype /Link /Rect [0 0 10 10] /A << /S /URI /URI ({url}) >> >>] /AA << /O << /S /GoToR /F ({url}) >> >> >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        { let content="BT /F1 12 Tf 30 700 Td (PDF_NETWORK_TRAP) Tj ET";format!("<< /Length {} >>\nstream\n{content}\nendstream",content.len()) },
        format!("<< /Type /Filespec /FS /URL /F ({url}) >>"),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trap.pdf");
    std::fs::write(&path, bytes).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::env::var_os("VZ_TEST_PDFIUM")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            root.join(".assets").join(if cfg!(windows) {
                "pdfium/bin/pdfium.dll"
            } else {
                "pdfium/lib/libpdfium.dylib"
            })
        });
    run(
        Path::new(env!("CARGO_BIN_EXE_vz-parser-worker")),
        &Request {
            path,
            pdfium: Some(library.canonicalize().unwrap()),
            office: None,
            readback: None,
            projection: Some(vz_core::worker::ProjectionJob {
                kind: "pdf".into(),
                output: dir.path().into(),
            }),
        },
        Duration::from_secs(30),
    )
    .unwrap();
    let p: vz_core::office::projection::Projection =
        serde_json::from_slice(&std::fs::read(dir.path().join("projection.json")).unwrap())
            .unwrap();
    assert!(p.nodes.iter().any(|n| n.text.contains("PDF_NETWORK_TRAP")));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "PDF projection fetched a remote resource"
    );
}

// Runs on macOS and Windows CI. The fixture hook exists only in the test probe;
// the shipping parser never accepts resource-probe bytes as PDF input.
#[test]
fn projection_resource_exhaustion_is_a_named_seller_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let worker = Path::new(env!("CARGO_BIN_EXE_vz-worker-test-probe"));
    for (bytes, expected, timeout) in [
        (
            b"VZ_RESOURCE_PROBE_RSS\n".as_slice(),
            "PROJECTION_MEMORY_LIMIT_EXCEEDED",
            Duration::from_secs(10),
        ),
        (
            b"VZ_RESOURCE_PROBE_DEADLINE\n".as_slice(),
            "PROJECTION_RESOURCE_LIMIT",
            Duration::from_millis(750),
        ),
    ] {
        let path = dir.path().join("resource-exhaustion.pdf");
        let receipt = dir.path().join("resource-probe-started");
        if receipt.exists() {
            std::fs::remove_file(&receipt).unwrap();
        }
        std::fs::write(&path, bytes).unwrap();
        let request = Request {
            path,
            pdfium: None,
            office: None,
            readback: None,
            projection: Some(vz_core::worker::ProjectionJob {
                kind: "pdf".into(),
                output: dir.path().into(),
            }),
        };
        let start = std::time::Instant::now();
        let failure =
            vz_core::worker::run_with_watchdog_limit(worker, &request, timeout, 128 * 1024 * 1024)
                .unwrap_err();
        let reason = format!("{failure:#}");
        assert!(reason.contains(expected), "{reason}");
        assert!(!reason.contains("crashed"), "{reason}");
        assert!(vz_core::failures::is_content(&failure));
        assert_eq!(
            std::fs::read(receipt).unwrap(),
            bytes,
            "test-only bypass must run before supervision refuses"
        );
        assert!(!dir.path().join("projection.json").exists());
        assert!(start.elapsed() < Duration::from_secs(12));
    }
    assert!(run(
        worker,
        &Request {
            path: "ok".into(),
            pdfium: None,
            office: None,
            projection: None,
            readback: None
        },
        Duration::from_secs(5)
    )
    .is_ok());
}
