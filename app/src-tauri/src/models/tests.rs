use super::*;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "vz-model-test-{}-{}",
            std::process::id(),
            STAGE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Server {
    child: Child,
    root: Temp,
    url: String,
}
impl Server {
    fn new(plan: Value) -> Self {
        let root = Temp::new();
        fs::write(root.0.join("plan.json"), plan.to_string()).unwrap();
        let python = if cfg!(windows) { "python" } else { "python3" };
        let mut child = Command::new(python)
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/models-test-server.py"))
            .arg(&root.0)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        assert!(
            !port.trim().is_empty(),
            "TLS fixture failed to start (Python/OpenSSL required)"
        );
        Self {
            child,
            root,
            url: format!("https://127.0.0.1:{}/initial", port.trim()),
        }
    }
    fn client(&self) -> Client {
        client_builder()
            .no_proxy()
            .add_root_certificate(
                reqwest::Certificate::from_pem(&fs::read(self.root.0.join("cert.pem")).unwrap())
                    .unwrap(),
            )
            .build()
            .unwrap()
    }
    fn pin(&self) -> Pin {
        pin(&self.url)
    }
    fn requests(&self) -> Value {
        serde_json::from_slice(&fs::read(self.root.0.join("requests.json")).unwrap()).unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn pin(url: &str) -> Pin {
    Pin {
        file: "test.gguf".into(),
        size: 100,
        sha256: vz_core::pipeline::sha256(&[b'x'; 100]),
        metadata: json!({"file":"test.gguf", "size":100, "url":url, "name":"test", "license":"MIT", "source":"test"}),
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}
async fn acquire(server: &Server, dir: &Path) -> Result<()> {
    let ops = Operations::default();
    let _guard = ops.acquire().unwrap();
    stage_verify_promote(
        dir,
        &server.pin(),
        Source::Download(&server.client()),
        ops.start(),
        &mut |_| {},
    )
    .await
}

#[test]
fn exact_40_99_range_appends_including_through_a_redirect() {
    for redirect in [false, true] {
        let mut plan = vec![];
        if redirect {
            plan.push(json!({"status":302,"length":0,"headers":{"Location":"/target"}}));
        }
        plan.push(json!({"status":206,"length":60,"headers":{"Content-Range":"bytes 40-99/100"}}));
        let server = Server::new(json!(plan));
        let dir = Temp::new();
        fs::write(dir.0.join("test.gguf.partial"), [b'x'; 40]).unwrap();
        runtime().block_on(acquire(&server, &dir.0)).unwrap();
        assert_eq!(fs::read(dir.0.join("test.gguf")).unwrap(), [b'x'; 100]);
        let requests = server.requests();
        assert_eq!(requests[0]["range"], "bytes=40-");
        if redirect {
            assert_eq!(requests[1]["range"], "bytes=40-");
            assert_eq!(requests[1]["path"], "/target");
        }
        assert!(!dir.0.join("test.gguf.partial").exists());
    }
}
#[test]
fn range_200_restarts_and_416_or_mismatched_range_restarts_once() {
    for response in [
        json!({"status":200}),
        json!({"status":416,"length":0}),
        json!({"status":206,"length":60,"headers":{"Content-Range":"bytes 40-100/100"}}),
    ] {
        let server = Server::new(json!([response, {"status":200}]));
        let dir = Temp::new();
        fs::write(dir.0.join("test.gguf.partial"), [b'y'; 40]).unwrap();
        runtime().block_on(acquire(&server, &dir.0)).unwrap();
        assert!(verified(&dir.0, &server.pin()));
        let requests = server.requests();
        if requests.as_array().unwrap().len() > 1 {
            assert!(requests[1]["range"].is_null());
        }
    }
    let server = Server::new(json!([{"status":416,"length":0}]));
    let dir = Temp::new();
    let error = runtime().block_on(acquire(&server, &dir.0)).unwrap_err();
    assert!(error.to_string().contains("after restart"));
    assert_eq!(server.requests().as_array().unwrap().len(), 2);
    assert!(!dir.0.join("test.gguf.partial").exists());
}
#[test]
fn size_hash_and_oversize_failures_delete_staging_and_preserve_installed_bytes() {
    for response in [
        json!({"length":101}),
        json!({"length":99}),
        json!({"byte":"y"}),
    ] {
        let server = Server::new(json!([response]));
        let dir = Temp::new();
        let installed = dir.0.join("test.gguf");
        fs::write(&installed, [b'x'; 100]).unwrap();
        assert!(runtime().block_on(acquire(&server, &dir.0)).is_err());
        assert_eq!(fs::read(&installed).unwrap(), [b'x'; 100]);
        assert!(!dir.0.join("test.gguf.partial").exists());
    }
}
#[test]
fn transport_failure_and_three_retries_preserve_installed_model() {
    let server = Server::new(json!([{"status":503,"length":0}]));
    let dir = Temp::new();
    fs::write(dir.0.join("test.gguf"), [b'x'; 100]).unwrap();
    assert!(runtime().block_on(acquire(&server, &dir.0)).is_err());
    assert_eq!(server.requests().as_array().unwrap().len(), 4);
    assert_eq!(fs::read(dir.0.join("test.gguf")).unwrap(), [b'x'; 100]);
}
#[test]
fn retry_after_redirect_returns_to_pinned_initial_url() {
    let server = Server::new(json!([
        {"status":302,"length":0,"headers":{"Location":"/signed-old-target"}},
        {"status":503,"length":0}, {"status":200}
    ]));
    let dir = Temp::new();
    runtime().block_on(acquire(&server, &dir.0)).unwrap();
    assert_eq!(server.requests()[2]["path"], "/initial");
}
#[test]
fn https_only_refuses_http_redirect_and_caps_chain_at_five() {
    for location in ["http://127.0.0.1:{port}/plain", "/again"] {
        let server =
            Server::new(json!([{"status":302,"length":0,"headers":{"Location":location}}]));
        let dir = Temp::new();
        let error = runtime().block_on(acquire(&server, &dir.0)).unwrap_err();
        assert!(
            format!("{error:#}").contains("redirect") || format!("{error:#}").contains("HTTPS")
        );
        let count = server.requests().as_array().unwrap().len();
        assert_eq!(count, if location.starts_with("http:") { 1 } else { 6 });
    }
}
#[test]
fn stalled_body_times_out() {
    let server = Server::new(json!([{"body_delay":1}]));
    let dir = Temp::new();
    let client = client_builder()
        .no_proxy()
        .add_root_certificate(
            reqwest::Certificate::from_pem(&fs::read(server.root.0.join("cert.pem")).unwrap())
                .unwrap(),
        )
        .read_timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let ops = Operations::default();
    let result = runtime().block_on(stage_verify_promote(
        &dir.0,
        &server.pin(),
        Source::Download(&client),
        ops.start(),
        &mut |_| {},
    ));
    assert!(format!("{:#}", result.unwrap_err()).contains("timed out"));
    assert_eq!(server.requests().as_array().unwrap().len(), 4);
}
#[test]
fn cancel_keeps_partial_and_refused_overlap_cannot_touch_its_staging() {
    let server = Server::new(json!([{"body_delay":2,"prefix":40}]));
    let dir = Temp::new();
    let ops = Operations::default();
    let guard = ops.acquire().unwrap();
    let cancel = ops.start();
    let mut progress = |value: Value| {
        if value["downloaded"] == 40 {
            // Every command uses this same try_lock. These are download/download,
            // download/import and download/pipeline refusals while bytes are staged.
            for _ in 0..5 {
                assert_eq!(ops.acquire().unwrap_err(), BUSY);
            }
            ops.cancel();
        }
    };
    let result = runtime().block_on(stage_verify_promote(
        &dir.0,
        &server.pin(),
        Source::Download(&server.client()),
        cancel,
        &mut progress,
    ));
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(
        fs::read(dir.0.join("test.gguf.partial")).unwrap(),
        [b'x'; 40]
    );
    assert!(!dir.0.join("test.gguf").exists());
    drop(guard);
    assert!(ops.acquire().is_ok());
}
#[test]
fn shared_lock_refuses_pipeline_pipeline_and_pipeline_mutation_without_waiting() {
    let ops = Operations::default();
    let guard = ops.acquire().unwrap();
    for _ in 0..5 {
        assert_eq!(ops.acquire().unwrap_err(), BUSY);
    }
    drop(guard);
    assert!(ops.acquire().is_ok());
}
#[test]
fn imports_use_verified_staging_and_failures_leave_valid_model_byte_identical() {
    let source = Temp::new();
    let installed = Temp::new();
    let pin = pin("https://unused.invalid");
    fs::write(installed.0.join(&pin.file), [b'x'; 100]).unwrap();
    for body in [vec![b'y'; 100], vec![b'x'; 99], vec![b'x'; 101]] {
        fs::write(source.0.join(&pin.file), body).unwrap();
        let ops = Operations::default();
        let report = runtime()
            .block_on(import(
                &installed.0,
                std::slice::from_ref(&pin),
                &source.0,
                ops.start(),
            ))
            .unwrap();
        assert!(report[0]["result"].as_str().unwrap().contains("mismatch"));
        assert_eq!(fs::read(installed.0.join(&pin.file)).unwrap(), [b'x'; 100]);
        assert_eq!(fs::read_dir(&installed.0).unwrap().count(), 1);
    }
    fs::remove_file(source.0.join(&pin.file)).unwrap();
    let ops = Operations::default();
    let result = runtime()
        .block_on(import(
            &installed.0,
            std::slice::from_ref(&pin),
            &source.0,
            ops.start(),
        ))
        .unwrap();
    assert_ne!(result[0]["result"], "imported");
    fs::write(source.0.join(&pin.file), [b'x'; 100]).unwrap();
    assert_eq!(
        runtime()
            .block_on(import(
                &installed.0,
                std::slice::from_ref(&pin),
                &source.0,
                ops.start()
            ))
            .unwrap()[0]["result"],
        "imported"
    );
    assert!(runtime()
        .block_on(import(
            &installed.0,
            &[pin],
            Path::new("relative"),
            ops.start()
        ))
        .is_err());
}
#[test]
fn failed_promotion_removes_only_owned_stage() {
    let dir = Temp::new();
    let source = Temp::new();
    let pin = pin("https://unused.invalid");
    fs::create_dir(dir.0.join(&pin.file)).unwrap(); // rename cannot replace a directory
    fs::write(dir.0.join("foreign-stage"), "untouched").unwrap();
    fs::write(source.0.join(&pin.file), [b'x'; 100]).unwrap();
    let ops = Operations::default();
    assert!(runtime()
        .block_on(stage_verify_promote(
            &dir.0,
            &pin,
            Source::Import(regular(&source.0.join(&pin.file), false).unwrap()),
            ops.start(),
            &mut |_| {}
        ))
        .is_err());
    assert_eq!(fs::read(dir.0.join("foreign-stage")).unwrap(), b"untouched");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[cfg(windows)]
#[test]
fn windows_sharing_failure_preserves_valid_installed_model_for_both_sources() {
    use std::os::windows::fs::OpenOptionsExt;
    let server = Server::new(json!([{}]));
    let dir = Temp::new();
    let source = Temp::new();
    let pin = server.pin();
    fs::write(dir.0.join(&pin.file), [b'x'; 100]).unwrap();
    fs::write(source.0.join(&pin.file), [b'x'; 100]).unwrap();
    // Permit reads but deny delete/rename, as another application can on Windows.
    let _held = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(dir.0.join(&pin.file))
        .unwrap();
    let client = server.client();
    let ops = Operations::default();
    for source in [
        Source::Download(&client),
        Source::Import(regular(&source.0.join(&pin.file), false).unwrap()),
    ] {
        let error = runtime()
            .block_on(stage_verify_promote(
                &dir.0,
                &pin,
                source,
                ops.start(),
                &mut |_| {},
            ))
            .unwrap_err();
        assert!(error.to_string().contains("could not promote"));
        assert_eq!(fs::read(dir.0.join(&pin.file)).unwrap(), [b'x'; 100]);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
}
#[test]
fn import_links_and_non_regular_files_are_refused() {
    let dir = Temp::new();
    let target = dir.0.join("target");
    fs::write(&target, [b'x'; 100]).unwrap();
    let link = dir.0.join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    {
        // Directory junctions require no symlink privilege and carry the same
        // FILE_ATTRIBUTE_REPARSE_POINT that cloud placeholders carry.
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&dir.0)
            .status()
            .unwrap();
        assert!(status.success());
    }
    assert!(format!("{:#}", regular(&link, false).unwrap_err()).contains("not a regular file"));
    assert!(regular(&dir.0, false)
        .unwrap_err()
        .to_string()
        .contains("not a regular file"));
}

#[cfg(windows)]
#[test]
fn non_symlink_file_reparse_point_reports_not_a_regular_file() {
    use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            handle: *mut std::ffi::c_void,
            code: u32,
            input: *const std::ffi::c_void,
            input_size: u32,
            output: *mut std::ffi::c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }
    let dir = Temp::new();
    let path = dir.0.join("placeholder.gguf");
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    // A user-defined GUID reparse tag produces a real file reparse point without
    // requiring a cloud provider or symlink privilege. Like OneDrive's tag, it
    // is not a symlink: only the FILE_ATTRIBUTE_REPARSE_POINT check catches it.
    #[repr(C)]
    struct Reparse {
        tag: u32,
        length: u16,
        reserved: u16,
        guid: [u8; 16],
    }
    let reparse = Reparse {
        tag: 0x42,
        length: 0,
        reserved: 0,
        guid: [0x42; 16],
    };
    let mut returned = 0;
    // SAFETY: valid file handle, correctly sized input, optional outputs null.
    let result = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x000900a4,
            (&reparse as *const Reparse).cast(),
            std::mem::size_of::<Reparse>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(
        result,
        0,
        "FSCTL_SET_REPARSE_POINT: {}",
        std::io::Error::last_os_error()
    );
    drop(file);
    let metadata = fs::symlink_metadata(&path).unwrap();
    assert!(metadata.is_file());
    assert!(!metadata.file_type().is_symlink());
    assert_ne!(metadata.file_attributes() & 0x400, 0);
    assert!(regular(&path, false)
        .unwrap_err()
        .to_string()
        .contains("not a regular file"));
}
#[test]
fn status_checks_size_and_hash_and_space_counts_only_remaining_bytes() {
    let dir = Temp::new();
    let mut pin = pin("https://unused.invalid");
    pin.metadata["sha256"] = json!(pin.sha256);
    assert_eq!(status(&dir.0, &[pin.clone()])["ready"], false);
    fs::write(dir.0.join("test.gguf.partial"), [b'x'; 40]).unwrap();
    assert_eq!(space_needed(&dir.0, &[pin.clone()], false).unwrap(), 60);
    assert_eq!(space_needed(&dir.0, &[pin.clone()], true).unwrap(), 100);
    assert!(check_space(&dir.0, u64::MAX)
        .unwrap_err()
        .to_string()
        .contains("bytes needed"));
    fs::write(dir.0.join(&pin.file), [b'x'; 100]).unwrap();
    assert_eq!(status(&dir.0, &[pin.clone()])["ready"], true);
    fs::write(dir.0.join(&pin.file), [b'y'; 100]).unwrap();
    assert!(!verified(&dir.0, &pin));
    fs::write(dir.0.join(&pin.file), [b'x'; 99]).unwrap();
    assert!(!verified(&dir.0, &pin));
}

#[test]
fn bundled_pins_are_sized_and_download_does_not_fetch_valid_models() {
    let pins =
        pins(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spike/fixtures/models.json"))
            .unwrap();
    assert_eq!(
        pins.iter().map(|p| p.size).collect::<Vec<_>>(),
        [67308128, 491400032]
    );
    let dir = Temp::new();
    let pin = pin("https://unused.invalid");
    fs::write(dir.0.join(&pin.file), [b'x'; 100]).unwrap();
    let ops = Operations::default();
    runtime()
        .block_on(download(&dir.0, &[pin], ops.start(), &mut |_| {
            panic!("No download of valid models")
        }))
        .unwrap();
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn invalid_folder_guard_preserves_seeded_rows_generation_files_and_search() {
    use vz_core::{
        pipeline::{Chunk, Document},
        storage::State,
    };
    let dir = Temp::new();
    let runtime = runtime();
    let mut state = State::open(&dir.0).unwrap();
    let doc = Document {
        doc_id: "doc".into(),
        path: "source.txt".into(),
        sha256: "hash".into(),
        bytes: 10,
        pages: 1,
        kind: "txt".into(),
        needs_ocr_pages: vec![],
        chunks: vec![Chunk {
            chunk_id: "doc-0".into(),
            doc_id: "doc".into(),
            text: "orbital archive retention is thirty days".into(),
            page: 1,
            heading_path: vec![],
            token_count: 8,
            embedding: vec![1.0, 0.0, 0.0, 0.0],
        }],
    };
    let generation = runtime.block_on(state.stage(&doc)).unwrap();
    state.commit(&doc, "1", &generation).unwrap();
    let search = runtime
        .block_on(state.search("orbital archive", &[1.0, 0.0, 0.0, 0.0], 1))
        .unwrap()[0]
        .chunk
        .text
        .clone();
    for (folder, message) in [
        ("", "Choose a folder to index"),
        ("   ", "Choose a folder to index"),
        (dir.0.join("missing").to_str().unwrap(), "Folder not found"),
    ] {
        let before = index_snapshot(&dir.0).unwrap();
        assert_eq!(validate_folder(folder).unwrap_err(), message);
        assert_eq!(index_snapshot(&dir.0).unwrap(), before);
        assert_eq!(state.documents().unwrap()[0].doc_id, "doc");
        assert_eq!(
            runtime
                .block_on(state.search("orbital archive", &[1.0, 0.0, 0.0, 0.0], 1))
                .unwrap()[0]
                .chunk
                .text,
            search
        );
    }
}
