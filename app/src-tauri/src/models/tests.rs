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
    fn client_builder(&self) -> reqwest::ClientBuilder {
        // Trust only this disposable fixture CA. Merging it with system roots
        // invokes Apple's platform trust evaluator and can stall loopback TLS
        // on hosted runners. Keep certificate and hostname verification enabled.
        client_builder()
            .no_proxy()
            .tls_certs_only([reqwest::Certificate::from_pem(
                &fs::read(self.root.0.join("ca.pem")).unwrap(),
            )
            .unwrap()])
    }
    fn client(&self) -> Client {
        self.client_builder().build().unwrap()
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
fn fixture_client_rejects_another_fixtures_certificate() {
    let server = Server::new(json!([{"status":200}]));
    let other = Server::new(json!([{"status":200}]));
    let error = runtime()
        .block_on(async { other.client().get(&server.url).send().await })
        .unwrap_err();
    assert!(error.is_connect(), "{error:#}");
    let diagnostic = format!("{:#}", anyhow::Error::new(error));
    assert!(
        diagnostic.contains("invalid peer certificate"),
        "{diagnostic}"
    );
    assert!(!server.root.0.join("requests.json").exists());
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
    let server = Server::new(json!([{"body_delay":5,"prefix":40}]));
    let dir = Temp::new();
    let client = server
        .client_builder()
        // Allow TLS/headers on loaded CI runners; the body still stalls well
        // beyond this budget, after sending a prefix to exercise an idle read.
        .read_timeout(Duration::from_secs(2))
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
    assert_eq!(
        fs::read(dir.0.join("test.gguf.partial")).unwrap(),
        [b'x'; 40]
    );
    assert!(!dir.0.join("test.gguf").exists());
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
                assert_eq!(ops.acquire().err().as_deref(), Some(BUSY));
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
        assert_eq!(ops.acquire().err().as_deref(), Some(BUSY));
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
    assert_eq!(result[0]["result"], "missing");
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
fn import_space_counts_only_present_regular_pins_and_reports_missing_chat() {
    let source = Temp::new();
    let installed = Temp::new();
    let mut embedding = pin("https://unused.invalid");
    embedding.file = "embedding.gguf".into();
    let mut chat = embedding.clone();
    chat.file = "chat.gguf".into();
    fs::write(source.0.join(&embedding.file), [b'x'; 100]).unwrap();
    let pins = [embedding, chat];
    assert_eq!(space_needed(&source.0, &pins, true).unwrap(), pins[0].size);
    let ops = Operations::default();
    let results = runtime()
        .block_on(import(&installed.0, &pins, &source.0, ops.start()))
        .unwrap();
    assert_eq!(results[0]["result"], "imported");
    assert_eq!(results[1]["result"], "missing");
    assert!(verified(&installed.0, &pins[0]));
    assert!(!installed.0.join(&pins[1].file).exists());
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
    assert_eq!(space_needed(&dir.0, &[pin.clone()], true).unwrap(), 0);
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
    drop(state);
    for (folder, message) in [
        ("", "Choose a folder to index"),
        ("   ", "Choose a folder to index"),
        (dir.0.join("missing").to_str().unwrap(), "Folder not found"),
    ] {
        // Snapshot only while SQLite is closed: Windows locks portions of an
        // open database, and reopening/searching may update its bookkeeping.
        let before = index_snapshot(&dir.0).unwrap();
        assert_eq!(validate_folder(folder).unwrap_err(), message);
        assert_eq!(index_snapshot(&dir.0).unwrap(), before);
        let state = State::open(&dir.0).unwrap();
        assert_eq!(state.documents().unwrap()[0].doc_id, "doc");
        assert_eq!(
            runtime
                .block_on(state.search("orbital archive", &[1.0, 0.0, 0.0, 0.0], 1))
                .unwrap()[0]
                .chunk
                .text,
            search
        );
        drop(state);
    }
}

// Exercise the same cache/operation entry points as desktop IPC with cheap loaders.
#[derive(Default)]
struct Loads {
    embed: std::cell::Cell<usize>,
    chat: std::cell::Cell<usize>,
    drops: std::cell::Cell<usize>,
}
struct FakeEngines {
    counts: std::rc::Rc<Loads>,
    chat: std::cell::Cell<bool>,
}
impl CachedEngine for FakeEngines {
    fn ensure_chat(&self) -> Result<()> {
        if !self.chat.replace(true) {
            self.counts.chat.set(self.counts.chat.get() + 1);
        }
        Ok(())
    }
}
impl Drop for FakeEngines {
    fn drop(&mut self) {
        self.counts.drops.set(self.counts.drops.get() + 1);
    }
}
fn fake_load(counts: &std::rc::Rc<Loads>) -> Result<FakeEngines> {
    counts.embed.set(counts.embed.get() + 1);
    Ok(FakeEngines {
        counts: counts.clone(),
        chat: std::cell::Cell::new(false),
    })
}
#[test]
fn second_search_reuses_embedding_and_search_never_loads_chat() {
    let ops = OperationState::<FakeEngines>::default();
    let counts = std::rc::Rc::new(Loads::default());
    for _ in 0..2 {
        ops.acquire()
            .unwrap()
            .get_or_load(false, || fake_load(&counts))
            .unwrap();
    }
    assert_eq!(counts.embed.get(), 1);
    assert_eq!(counts.chat.get(), 0);
    for _ in 0..2 {
        ops.acquire()
            .unwrap()
            .get_or_load(true, || fake_load(&counts))
            .unwrap();
    }
    assert_eq!(counts.embed.get(), 1);
    assert_eq!(counts.chat.get(), 1);
    // Negative control: forcibly invalidated per-command loads reproduce the bug.
    for _ in 0..2 {
        drop(ops.acquire_mutation().unwrap());
        ops.acquire()
            .unwrap()
            .get_or_load(false, || fake_load(&counts))
            .unwrap();
    }
    assert_eq!(counts.embed.get(), 3);
    assert_eq!(counts.chat.get(), 1);
}
#[test]
fn mutations_drop_models_before_writing_and_next_use_reloads() {
    let ops = OperationState::<FakeEngines>::default();
    let counts = std::rc::Rc::new(Loads::default());
    for _ in 0..3 {
        ops.acquire()
            .unwrap()
            .get_or_load(true, || fake_load(&counts))
            .unwrap();
        let _mutation = ops.acquire_mutation().unwrap();
        let _cancel = ops.start();
        assert_eq!(
            counts.drops.get(),
            counts.embed.get(),
            "drop before download/import or cleanup"
        );
        assert!(ops.acquire().is_err(), "no inference while mutating");
        ops.cancel(); // Must never deadlock the acquisition/cleanup guard.
    }
    ops.acquire()
        .unwrap()
        .get_or_load(false, || fake_load(&counts))
        .unwrap();
    assert_eq!(counts.embed.get(), 4);
    assert_eq!(counts.chat.get(), 3);
}
#[test]
fn refused_mutation_keeps_cache_and_cancel_invalidates_idle_or_active_cache() {
    let ops = OperationState::<FakeEngines>::default();
    let counts = std::rc::Rc::new(Loads::default());
    let mut held = ops.acquire().unwrap();
    held.get_or_load(false, || fake_load(&counts)).unwrap();
    assert!(ops.acquire_mutation().is_err());
    assert_eq!(counts.drops.get(), 0);
    ops.cancel();
    assert_eq!(
        counts.drops.get(),
        0,
        "active engines remain valid until release"
    );
    drop(held);
    ops.acquire()
        .unwrap()
        .get_or_load(false, || fake_load(&counts))
        .unwrap();
    assert_eq!(counts.drops.get(), 1);
    ops.cancel();
    assert_eq!(counts.drops.get(), 2);
}
#[test]
fn failed_load_is_retried_and_failed_chat_preserves_embedding() {
    struct ChatFails;
    impl CachedEngine for ChatFails {
        fn ensure_chat(&self) -> Result<()> {
            bail!("bad chat hash")
        }
    }
    let ops = OperationState::<ChatFails>::default();
    let mut held = ops.acquire().unwrap();
    assert!(held
        .get_or_load(false, || bail!("bad embedding hash"))
        .is_err());
    assert!(held.get_or_load(true, || Ok(ChatFails)).is_err());
    held.get_or_load(false, || panic!("embedding should survive a chat failure"))
        .unwrap();
}

#[test]
fn invalid_import_preserves_loaded_cache_and_valid_import_invalidates_it() {
    let ops = OperationState::<FakeEngines>::default();
    let counts = std::rc::Rc::new(Loads::default());
    ops.acquire()
        .unwrap()
        .get_or_load(false, || fake_load(&counts))
        .unwrap();
    let source = Temp::new();
    let destination = Temp::new();
    let file = source.0.join("file");
    fs::write(&file, b"not a directory").unwrap();
    for (folder, message) in [
        (PathBuf::from("relative"), "Type an absolute folder path"),
        (source.0.join("missing"), "Import folder not found"),
        (file, "Import folder not found"),
    ] {
        assert_eq!(ops.acquire_import(&folder).err().unwrap(), message);
        assert_eq!(counts.drops.get(), 0);
        ops.acquire()
            .unwrap()
            .get_or_load(false, || fake_load(&counts))
            .unwrap();
        assert_eq!(counts.embed.get(), 1, "invalid import must preserve cache");
    }
    let pin = pin("https://unused.invalid");
    fs::write(source.0.join(&pin.file), [b'x'; 100]).unwrap();
    {
        let _guard = ops.acquire_import(&source.0).unwrap();
        assert_eq!(counts.drops.get(), 1, "drop before importing models");
        let results = runtime()
            .block_on(import(
                &destination.0,
                std::slice::from_ref(&pin),
                &source.0,
                ops.start(),
            ))
            .unwrap();
        assert_eq!(results[0]["result"], "imported");
        assert!(verified(&destination.0, &pin));
    }
    ops.acquire()
        .unwrap()
        .get_or_load(false, || fake_load(&counts))
        .unwrap();
    assert_eq!(counts.embed.get(), 2);
}

#[test]
fn release_tiers_and_fixture_compatibility() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let release = root.join("spike/fixtures/models.release.json");
    let candidates: Value =
        serde_json::from_slice(&std::fs::read(root.join("spike/bakeoff/candidates.json")).unwrap())
            .unwrap();
    for (bytes, name) in [
        (8u64 << 30, "qwen3-4b-instruct-2507"),
        (16u64 << 30, "ministral-3-8b"),
        (24u64 << 30, "ministral-3-8b"),
        (32u64 << 30, "qwen3.6-35b-a3b"),
        (64u64 << 30, "qwen3.6-35b-a3b"),
        (17_000_000_000, "ministral-3-8b"),
        (34_000_000_000, "qwen3.6-35b-a3b"),
    ] {
        let selected = selected_pins(&release, bytes).unwrap();
        assert_eq!(selected["chat"]["name"], name);
        for key in ["embedding", "chat"] {
            let pin = &selected[key];
            assert_eq!(
                pin,
                candidates
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|p| p["name"] == pin["name"])
                    .unwrap()
            );
            for field in ["url", "sha256", "license", "source"] {
                assert!(!pin[field].as_str().unwrap().is_empty());
            }
            assert!(pin["size"].as_u64().unwrap() > 0);
        }
        assert_eq!(pins_from_value(&selected).unwrap().len(), 2);
    }
    let fixture = root.join("spike/fixtures/models.json");
    assert_eq!(
        selected_pins(&fixture, 8 << 30).unwrap(),
        selected_pins(&fixture, 64 << 30).unwrap()
    );
}

#[test]
fn old_recipe_is_detected_before_models_are_downloaded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = Temp::new();
    let temp = &directory.0.join("phase0-index");
    std::fs::create_dir(temp).unwrap();
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(root.join("spike/fixtures/models.json")).unwrap())
            .unwrap();
    let release = selected_pins(&root.join("spike/fixtures/models.release.json"), 8 << 30).unwrap();
    let old = vz_core::engine::Recipe::from_pins(&fixture, 384).unwrap();
    std::fs::write(temp.join("recipe.json"), serde_json::to_vec(&old).unwrap()).unwrap();
    assert!(!needs_reindex(temp, &fixture).unwrap());
    assert!(needs_reindex(&vz_core::active_index::resolve(&directory.0), &release).unwrap());
    let new = vz_core::engine::Recipe::from_pins(&release, 1024).unwrap();
    let error = vz_core::storage::State::open_index(temp, &new)
        .err()
        .unwrap();
    assert!(error.to_string().contains("re-index required"));
    assert_eq!(
        std::fs::read(temp.join("recipe.json")).unwrap(),
        serde_json::to_vec(&old).unwrap()
    );
    std::fs::remove_dir_all(temp).unwrap();
}
