use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use vz_core::{
    pipeline::{Chunk, Document},
    storage::State,
};
fn document(text: &str) -> Document {
    Document {
        doc_id: "doc".into(),
        path: "source.txt".into(),
        sha256: text.into(),
        bytes: 1,
        pages: 1,
        kind: "txt".into(),
        chunks: vec![Chunk {
            chunk_id: "doc-0".into(),
            doc_id: "doc".into(),
            text: text.into(),
            page: 1,
            heading_path: vec![],
            token_count: 1,
            embedding: vec![1.0, 0.0, 0.0, 0.0],
        }],
    }
}
#[test]
fn crash_child() {
    if let Some(root) = std::env::var_os("VZ_RECOVERY_CHILD") {
        let root = Path::new(&root);
        let state = State::open(root).unwrap();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(state.stage(&document("replacement")))
            .unwrap();
        std::fs::write(root.join("staged"), "ready").unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}
#[test]
fn kill_between_lance_write_and_sqlite_flip_preserves_old_generation() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    {
        let mut state = State::open(temp.path()).unwrap();
        let doc = document("original");
        let generation = runtime.block_on(state.stage(&doc)).unwrap();
        state.commit(&doc, "1", &generation).unwrap();
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture"])
        .env("VZ_RECOVERY_CHILD", temp.path())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !temp.path().join("staged").exists() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "child did not finish staged write"
        );
        assert!(child.try_wait().unwrap().is_none(), "child failed");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let mut state = State::open(temp.path()).unwrap();
    assert_eq!(state.documents().unwrap()[0].chunks[0].text, "original");
    assert_eq!(
        std::fs::read_dir(temp.path().join("generations"))
            .unwrap()
            .count(),
        1
    );
    let doc = document("replacement");
    let generation = runtime.block_on(state.stage(&doc)).unwrap();
    state.commit(&doc, "2", &generation).unwrap();
    state.recover().unwrap();
    assert_eq!(state.documents().unwrap()[0].chunks[0].text, "replacement");
    assert_eq!(
        std::fs::read_dir(temp.path().join("generations"))
            .unwrap()
            .count(),
        1
    );
}
