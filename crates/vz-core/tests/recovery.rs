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
        |_| stopped.store(true, Ordering::SeqCst),
        || stopped.load(Ordering::SeqCst)
    )
    .await
    .is_err());
    let staged = {
        let db = rusqlite::Connection::open(root.join(".vz-scan/scan.sqlite")).unwrap();
        db.query_row("SELECT generation FROM scan_documents", [], |r| {
            r.get::<_, String>(0)
        })
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
    assert_eq!((result.completed, result.skipped), (1, 0));
    assert!(!root.join(".vz-scan/generations").join(staged).exists());
    let generation = State::review_connection(&root)
        .unwrap()
        .query_row("SELECT generation FROM documents", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    std::fs::remove_dir_all(root.join("generations").join(&generation)).unwrap();
    let result = active_index::reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    assert_eq!((result.completed, result.skipped), (1, 0));
    assert_eq!(State::review_documents(&root).unwrap().len(), 1);
}

#[tokio::test]
async fn unreadable_published_generation_is_revoked_when_repair_has_infrastructure_failure() {
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
    let generation = State::review_connection(&root)
        .unwrap()
        .query_row("SELECT generation FROM documents", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    std::fs::remove_dir_all(root.join("generations").join(generation)).unwrap();
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

#[tokio::test]
async fn published_lance_manifest_and_data_damage_are_cache_misses() {
    use vz_core::{active_index, engine::Engines};
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    let engines =
        Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true).unwrap();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("a.txt"),
        "Readable generation structural recovery.",
    )
    .unwrap();
    std::fs::write(source.path().join("b.txt"), "Undamaged control document.").unwrap();
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
    for damage in [
        "manifest",
        "data",
        "truncated fragment",
        "fragment version",
        "fragment metadata offset",
        "fragment metadata offset zero",
    ] {
        let generation: String = State::review_connection(&root)
            .unwrap()
            .query_row(
                "SELECT generation FROM documents WHERE path LIKE '%a.txt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let table = root
            .join("generations")
            .join(&generation)
            .join("lance/chunks.lance");
        if damage == "manifest" {
            let latest = std::fs::read_dir(table.join("_versions"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .max_by_key(|p| {
                    p.file_stem()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                })
                .unwrap();
            let mut bytes = std::fs::read(&latest).unwrap();
            // Retain the footer, offset and message length, but corrupt protobuf.
            let offset =
                u64::from_le_bytes(bytes[bytes.len() - 16..bytes.len() - 8].try_into().unwrap())
                    as usize;
            bytes[offset + 4] = 0xff;
            std::fs::write(latest, bytes).unwrap();
        } else if damage == "data" {
            std::fs::remove_dir_all(table.join("data")).unwrap();
        } else {
            let fragment = std::fs::read_dir(table.join("data"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let bytes = std::fs::read(&fragment).unwrap();
            assert!(bytes.ends_with(b"LANC"));
            if damage == "truncated fragment" {
                // Preserve the complete footer but remove part of the data body.
                let mut truncated = bytes[..8].to_vec();
                truncated.extend_from_slice(&bytes[bytes.len() - 40..]);
                std::fs::write(fragment, truncated).unwrap();
            } else {
                let mut damaged = bytes.clone();
                let offset = bytes.len() - if damage == "fragment version" { 8 } else { 40 };
                let count = if damage == "fragment version" { 4 } else { 8 };
                damaged[offset..offset + count].fill(if damage.ends_with("zero") {
                    0
                } else {
                    0xff
                });
                assert_eq!(damaged.len(), bytes.len());
                assert!(damaged.ends_with(b"LANC"));
                std::fs::write(fragment, damaged).unwrap();
                // Prove the structural mutation breaks the real Lance reader
                // before asking re-index to repair it.
                use futures::{FutureExt, TryStreamExt};
                use lancedb::query::{ExecutableQuery, QueryBase};
                // Lance 0.38 can panic inside its query task on an invalid
                // offset. Either a query error or that panic proves unreadability.
                let unreadable = std::panic::AssertUnwindSafe(async {
                    let db = lancedb::connect(table.parent().unwrap().to_str().unwrap())
                        .execute()
                        .await?;
                    let table = db.open_table("chunks").execute().await?;
                    let mut stream = table.query().limit(1).execute().await?;
                    stream.try_next().await?;
                    Ok::<_, anyhow::Error>(())
                })
                .catch_unwind()
                .await;
                assert!(
                    !matches!(unreadable, Ok(Ok(()))),
                    "damage must make Lance unreadable: {damage}"
                );
            }
        }
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
        assert_eq!((result.completed, result.skipped), (1, 1), "{damage}");
        assert!(!root.join("generations").join(generation).exists());
        let state = State::open(&root).unwrap();
        assert!(
            state
                .search(
                    "structural recovery",
                    &engines.vector("structural recovery", true).unwrap(),
                    5
                )
                .await
                .unwrap()
                .iter()
                .any(|hit| hit
                    .chunk
                    .text
                    .contains("Readable generation structural recovery")),
            "search must recover the damaged document: {damage}"
        );
    }
}
