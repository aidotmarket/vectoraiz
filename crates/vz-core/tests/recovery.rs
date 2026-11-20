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
        needs_ocr_pages: vec![],
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
    let seals = temp.path().join(".vz-validation");
    assert_eq!(std::fs::read_dir(&seals).unwrap().count(), 2);
    let orphan = seals.join("orphan.json");
    let leftover = seals.join("seal-stale.tmp");
    std::fs::write(&orphan, b"orphan without a generation").unwrap();
    std::fs::write(&leftover, b"interrupted atomic write").unwrap();
    let mut state = State::open(temp.path()).unwrap();
    assert!(!orphan.exists());
    assert!(!leftover.exists());
    assert_eq!(std::fs::read_dir(&seals).unwrap().count(), 1);
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
    assert!(seals.join(format!("{generation}.json")).is_file());
    assert_eq!(std::fs::read_dir(&seals).unwrap().count(), 1);
    assert_eq!(state.documents().unwrap()[0].chunks[0].text, "replacement");
    assert_eq!(
        std::fs::read_dir(temp.path().join("generations"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn ocr_status_commits_with_generation_and_old_metadata_reopens() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut state = State::open(temp.path()).unwrap();
    let mut doc = document("searchable text page");
    doc.pages = 2;
    doc.needs_ocr_pages = vec![2];
    let generation = runtime.block_on(state.stage(&doc)).unwrap();
    state.commit(&doc, "1", &generation).unwrap();
    let mut replacement = doc.clone();
    replacement.needs_ocr_pages.clear();
    runtime.block_on(state.stage(&replacement)).unwrap();
    drop(state);
    let state = State::open(temp.path()).unwrap();
    let reopened = state.documents().unwrap().remove(0);
    assert_eq!(reopened.needs_ocr_pages, [2]);
    assert_eq!(reopened.chunks[0].page, 1);
    let mut old = serde_json::to_value(&reopened).unwrap();
    old.as_object_mut().unwrap().remove("needs_ocr_pages");
    std::fs::write(
        temp.path()
            .join("generations")
            .join(generation)
            .join("document.json"),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    drop(state);
    let state = State::open(temp.path()).unwrap();
    assert!(state.documents().unwrap()[0].needs_ocr_pages.is_empty());
}

#[tokio::test]
async fn damaged_generations_in_published_and_scan_databases_are_per_file_cache_misses() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use vz_core::{active_index, engine::Engines};
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    let engines =
        Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true).unwrap();
    let data = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    std::fs::write(a.path().join("a.txt"), "Published recovery document.").unwrap();
    std::fs::write(b.path().join("b.txt"), "Staged recovery document.").unwrap();
    std::fs::write(b.path().join("c.txt"), "Undamaged staged control.").unwrap();
    active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    let root = active_index::resolve(data.path());
    let stopped = AtomicBool::new(false);
    assert!(active_index::reindex(
        data.path(),
        b.path(),
        &engines,
        &assets,
        |p| {
            if p.completed == 2 {
                stopped.store(true, Ordering::SeqCst);
            }
        },
        || stopped.load(Ordering::SeqCst)
    )
    .await
    .is_err());
    let staged = {
        let db = rusqlite::Connection::open(root.join(".vz-scan/scan.sqlite")).unwrap();
        db.query_row(
            "SELECT generation FROM scan_documents WHERE path LIKE '%b.txt'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    };
    std::fs::write(
        root.join(".vz-scan/generations")
            .join(&staged)
            .join("document.json"),
        "damaged",
    )
    .unwrap();
    let result = active_index::reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    assert_eq!((result.completed, result.skipped), (1, 1));
    assert!(!root.join(".vz-scan/generations").join(staged).exists());
    assert_eq!(State::review_documents(&root).unwrap().len(), 2);
    active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    let generation = State::review_connection(&root)
        .unwrap()
        .query_row(
            "SELECT generation FROM documents WHERE path LIKE '%a.txt'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    std::fs::remove_dir_all(root.join("generations").join(&generation)).unwrap();
    let result = active_index::reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    assert_eq!((result.completed, result.skipped), (2, 0));
    assert_eq!(State::review_documents(&root).unwrap().len(), 2);
}

#[tokio::test]
async fn damaged_published_generation_is_revoked_when_repair_has_infrastructure_failure() {
    use vz_core::{active_index, engine::Engines};
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    let engines =
        Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true).unwrap();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::copy(
        repo.join("spike/fixtures/documents/access.pdf"),
        source.path().join("access.pdf"),
    )
    .unwrap();
    active_index::reindex(
        data.path(),
        source.path(),
        &engines,
        &assets,
        |_| {},
        || false,
    )
    .await
    .unwrap();
    let root = active_index::resolve(data.path());
    let generation: String = State::review_connection(&root)
        .unwrap()
        .query_row("SELECT generation FROM documents", [], |r| r.get(0))
        .unwrap();
    std::fs::remove_file(
        root.join("generations")
            .join(&generation)
            .join("document.json"),
    )
    .unwrap();
    assert!(active_index::reindex(
        data.path(),
        source.path(),
        &engines,
        &source.path().join("no-assets"),
        |_| {},
        || false
    )
    .await
    .is_err());
    assert!(State::review_documents(&root).unwrap().is_empty());
    assert!(!root
        .join(".vz-validation")
        .join(format!("{generation}.json"))
        .exists());
    let result = active_index::reindex(
        data.path(),
        source.path(),
        &engines,
        &assets,
        |_| {},
        || false,
    )
    .await
    .unwrap();
    assert_eq!(result.completed, 1);
    assert_eq!(State::review_documents(&root).unwrap().len(), 1);
}
