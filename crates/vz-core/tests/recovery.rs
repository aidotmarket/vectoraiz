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

fn damage_fragment(table: &Path, damage: &str) {
    let fragment = std::fs::read_dir(table.join("data"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = std::fs::read(&fragment).unwrap();
    let footer = bytes.len() - 40;
    match damage {
        "footer version 2.0" => bytes[footer + 32..footer + 36].copy_from_slice(&[2, 0, 0, 0]),
        "column metadata tag" => {
            let start = u64::from_le_bytes(bytes[footer..footer + 8].try_into().unwrap()) as usize;
            bytes[start] = 0; // Protobuf field zero is invalid; record bounds stay intact.
        }
        "nested page encoding" | "page body" => {
            use prost::Message;
            let offsets =
                u64::from_le_bytes(bytes[footer + 8..footer + 16].try_into().unwrap()) as usize;
            let start =
                u64::from_le_bytes(bytes[offsets..offsets + 8].try_into().unwrap()) as usize;
            let size =
                u64::from_le_bytes(bytes[offsets + 8..offsets + 16].try_into().unwrap()) as usize;
            let mut column =
                lance_file::format::pbfile::ColumnMetadata::decode(&bytes[start..start + size])
                    .unwrap();
            let page = column.pages.first_mut().unwrap();
            if damage == "nested page encoding" {
                let location = page.encoding.as_mut().unwrap().location.as_mut().unwrap();
                let lance_file::format::pbfile::encoding::Location::Direct(direct) = location
                else {
                    panic!("expected direct encoding")
                };
                direct.encoding.fill(0xff);
                let encoded = column.encode_to_vec();
                assert_eq!(encoded.len(), size);
                bytes[start..start + size].copy_from_slice(&encoded);
                // The outer metadata remains valid; only its opaque nested Any is damaged.
                lance_file::format::pbfile::ColumnMetadata::decode(&bytes[start..start + size])
                    .unwrap();
            } else {
                for (&offset, &size) in page.buffer_offsets.iter().zip(&page.buffer_sizes) {
                    bytes[offset as usize..(offset + size) as usize].fill(0xff);
                }
            }
        }
        "column count" => {
            let count = u32::from_le_bytes(bytes[footer + 28..footer + 32].try_into().unwrap());
            bytes[footer + 28..footer + 32].copy_from_slice(&(count + 1).to_le_bytes());
        }
        "global buffer offset" => {
            let start =
                u64::from_le_bytes(bytes[footer + 16..footer + 24].try_into().unwrap()) as usize;
            let beyond = bytes.len() as u64 + 1;
            bytes[start..start + 8].copy_from_slice(&beyond.to_le_bytes());
        }
        "metadata offset zero" => bytes[footer..footer + 8].fill(0),
        _ => panic!("unknown damage {damage}"),
    }
    std::fs::write(fragment, bytes).unwrap();
}

fn damage_manifest_parent(table: &Path) {
    use prost::Message;
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
    let bytes = std::fs::read(&latest).unwrap();
    let offset =
        u64::from_le_bytes(bytes[bytes.len() - 16..bytes.len() - 8].try_into().unwrap()) as usize;
    let mut manifest =
        lance_table::format::pb::Manifest::decode(&bytes[offset + 4..bytes.len() - 16]).unwrap();
    manifest.fields[0].parent_id = i32::MAX;
    let message = manifest.encode_to_vec();
    let mut damaged = bytes[..offset].to_vec();
    damaged.extend_from_slice(&(message.len() as u32).to_le_bytes());
    damaged.extend_from_slice(&message);
    damaged.extend_from_slice(&bytes[bytes.len() - 16..]);
    std::fs::write(latest, damaged).unwrap();
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
    for damage in [
        "document",
        "manifest parent",
        "metadata offset zero",
        "footer version 2.0",
        "column metadata tag",
    ] {
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
        if damage == "document" {
            std::fs::write(
                root.join(".vz-scan/generations")
                    .join(&staged)
                    .join("document.json"),
                "damaged",
            )
            .unwrap();
        } else if damage == "manifest parent" {
            damage_manifest_parent(
                &root
                    .join(".vz-scan/generations")
                    .join(&staged)
                    .join("lance/chunks.lance"),
            );
        } else {
            damage_fragment(
                &root
                    .join(".vz-scan/generations")
                    .join(&staged)
                    .join("lance/chunks.lance"),
                damage,
            );
        }
        let result =
            active_index::reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
                .await
                .unwrap();
        assert_eq!((result.completed, result.skipped), (1, 1));
        assert!(!root.join(".vz-scan/generations").join(staged).exists());
        assert_eq!(State::review_documents(&root).unwrap().len(), 2);
        active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
    }
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
    for damage in [
        "column count",
        "global buffer offset",
        "footer version 2.0",
        "column metadata tag",
    ] {
        let generation: String = State::review_connection(&root)
            .unwrap()
            .query_row("SELECT generation FROM documents", [], |r| r.get(0))
            .unwrap();
        damage_fragment(
            &root
                .join("generations")
                .join(generation)
                .join("lance/chunks.lance"),
            damage,
        );
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
        "reader feature",
        "full text index",
        "data",
        "truncated fragment",
        "fragment version",
        "fragment metadata offset",
        "fragment metadata offset zero",
        "nested page encoding",
        "page body",
        "footer version 2.0",
        "column metadata tag",
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
        } else if damage == "reader feature" {
            use prost::Message;
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
            let bytes = std::fs::read(&latest).unwrap();
            let offset =
                u64::from_le_bytes(bytes[bytes.len() - 16..bytes.len() - 8].try_into().unwrap())
                    as usize;
            let mut manifest =
                lance_table::format::pb::Manifest::decode(&bytes[offset + 4..bytes.len() - 16])
                    .unwrap();
            manifest.reader_feature_flags |= 1 << 63;
            let message = manifest.encode_to_vec();
            let mut damaged = bytes[..offset].to_vec();
            damaged.extend_from_slice(&(message.len() as u32).to_le_bytes());
            damaged.extend_from_slice(&message);
            damaged.extend_from_slice(&bytes[bytes.len() - 16..]);
            std::fs::write(latest, damaged).unwrap();
        } else if damage == "full text index" {
            // Preserve the dataset and fragments; damage only the FTS artifacts.
            fn damage_files(path: &Path) {
                for entry in std::fs::read_dir(path).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        damage_files(&path);
                    } else {
                        std::fs::write(path, b"damaged full text index").unwrap();
                    }
                }
            }
            assert!(table.join("_indices").is_dir());
            damage_files(&table.join("_indices"));
        } else if damage == "data" {
            std::fs::remove_dir_all(table.join("data")).unwrap();
        } else if matches!(
            damage,
            "footer version 2.0" | "column metadata tag" | "nested page encoding" | "page body"
        ) {
            damage_fragment(&table, damage);
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
        if matches!(damage, "reader feature" | "full text index") {
            use futures::FutureExt;
            let state = State::open(&root).unwrap();
            let vector = engines.vector("structural recovery", true).unwrap();
            let result =
                std::panic::AssertUnwindSafe(state.search("structural recovery", &vector, 1))
                    .catch_unwind()
                    .await;
            assert!(
                !matches!(result, Ok(Ok(_))),
                "mutation must break real search: {damage}"
            );
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

#[tokio::test]
async fn noninitial_posting_damage_invalidates_fingerprint_and_rebuilds_only_one_document() {
    use arrow_array::{Array, LargeBinaryArray, ListArray, RecordBatch};
    use futures::FutureExt;
    use lance_core::cache::LanceCache;
    use lance_index::scalar::{inverted::TokenSet, lance_format::LanceIndexStore, IndexStore};
    use std::sync::Arc;
    use vz_core::{active_index, engine::Engines};
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    let engines =
        Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true).unwrap();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("a.txt"), "initial zebras").unwrap();
    std::fs::write(source.path().join("b.txt"), "untouched document").unwrap();
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
    let root = active_index::resolve_state(data.path());
    let rows = || {
        let db = State::review_connection(&root).unwrap();
        let mut stmt = db
            .prepare("SELECT generation FROM documents ORDER BY path")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let before = rows();
    let generation = root.join("generations").join(&before[0]);
    let marker_path = root
        .join(".vz-validation")
        .join(format!("{}.json", before[0]));
    assert!(marker_path.is_file());
    // An evicted/missing cache record triggers first validation without changing
    // any file bound by a listing review, and survives process-independent reuse.
    let snapshot = || {
        walkdir::WalkDir::new(&generation)
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.path().to_owned(),
                    (
                        entry.metadata().unwrap().len(),
                        entry.metadata().unwrap().modified().unwrap(),
                    ),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let bound_files = snapshot();
    std::fs::remove_file(&marker_path).unwrap();
    let first_validation = active_index::reindex(
        data.path(),
        source.path(),
        &engines,
        &assets,
        |_| {},
        || false,
    )
    .await
    .unwrap();
    assert_eq!(
        (first_validation.completed, first_validation.skipped),
        (0, 2)
    );
    assert_eq!(snapshot(), bound_files);
    assert!(!marker_path.exists());
    // Repeated reuse also leaves the evicted marker absent.
    let unchanged = active_index::reindex(
        data.path(),
        source.path(),
        &engines,
        &assets,
        |_| {},
        || false,
    )
    .await
    .unwrap();
    assert_eq!((unchanged.completed, unchanged.skipped), (0, 2));
    assert!(!marker_path.exists());
    let directory = std::fs::read_dir(generation.join("lance/chunks.lance/_indices"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let (objects, path) =
        lance_io::object_store::ObjectStore::from_uri(directory.to_str().unwrap())
            .await
            .unwrap();
    let store = LanceIndexStore::new(objects, path, Arc::new(LanceCache::no_cache()));
    let metadata = store.open_index_file("metadata.lance").await.unwrap();
    let ids: Vec<u64> = serde_json::from_str(&metadata.schema().metadata["partitions"]).unwrap();
    let id = ids[0];
    let format = metadata.schema().metadata["token_set_format"]
        .parse()
        .unwrap();
    let tokens = TokenSet::load(
        store
            .open_index_file(&format!("part_{id}_tokens.lance"))
            .await
            .unwrap(),
        format,
    )
    .await
    .unwrap();
    let target = tokens.get("zebra").expect("stemmed noninitial term") as usize;
    assert_ne!(target, tokens.get("initi").expect("initial term") as usize);
    let name = format!("part_{id}_invert.lance");
    let reader = store.open_index_file(&name).await.unwrap();
    let batch = reader.read_range(0..reader.num_rows(), None).await.unwrap();
    let posting_column = batch.schema().index_of("_posting").unwrap();
    let lists = batch
        .column(posting_column)
        .as_any()
        .downcast_ref::<ListArray>()
        .unwrap();
    let blocks = lists
        .values()
        .as_any()
        .downcast_ref::<LargeBinaryArray>()
        .unwrap();
    let target_block = lists.value_offsets()[target] as usize;
    // Change exactly one compressed posting payload; all other terms/columns survive.
    let damaged = LargeBinaryArray::from_iter_values((0..blocks.len()).map(|i| {
        if i == target_block {
            vec![0xff]
        } else {
            blocks.value(i).to_vec()
        }
    }));
    let lists = ListArray::try_new(
        match lists.data_type() {
            arrow_schema::DataType::List(field) => field.clone(),
            _ => unreachable!(),
        },
        lists.offsets().clone(),
        Arc::new(damaged),
        lists.nulls().cloned(),
    )
    .unwrap();
    let mut columns = batch.columns().to_vec();
    columns[posting_column] = Arc::new(lists);
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let mut writer = store
        .new_index_file("replacement.lance", batch.schema())
        .await
        .unwrap();
    writer.write_record_batch(batch).await.unwrap();
    writer
        .finish_with_metadata(reader.schema().metadata.clone())
        .await
        .unwrap();
    drop(writer);
    std::fs::rename(directory.join("replacement.lance"), directory.join(name)).unwrap();
    let state = State::open(&root).unwrap();
    let vector = engines.vector("zebras", true).unwrap();
    assert!(
        state.search("initial", &vector, 5).await.is_ok(),
        "initial term must remain readable"
    );
    let broken = std::panic::AssertUnwindSafe(state.search("zebras", &vector, 5))
        .catch_unwind()
        .await;
    assert!(
        !matches!(broken, Ok(Ok(_))),
        "noninitial posting must be unreadable"
    );
    drop(state);
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
    assert_eq!((result.completed, result.skipped), (1, 1));
    let after = rows();
    assert_ne!(before[0], after[0]);
    assert_eq!(before[1], after[1]);
    let state = State::open(&root).unwrap();
    assert!(state
        .search("zebras", &vector, 5)
        .await
        .unwrap()
        .iter()
        .any(|hit| hit.chunk.text.contains("zebras")));
}

#[tokio::test]
async fn full_validation_accepts_empty_full_text_vocabulary() {
    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path()).unwrap();
    for text in ["the and", "---", "🚀"] {
        let generation = state.stage(&document(text)).await.unwrap();
        assert!(root.path().join("generations").join(generation).is_dir());
    }
}

#[tokio::test]
async fn unavailable_marker_location_preserves_unchanged_and_publishes_new_document() {
    use vz_core::{active_index, engine::Engines};
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    let engines =
        Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true).unwrap();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("a.txt"), "healthy archive").unwrap();
    active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
        .await
        .unwrap();
    let rows = || {
        State::review_connection(data.path())
            .unwrap()
            .query_row(
                "SELECT generation FROM documents WHERE path LIKE '%a.txt'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
    };
    let original = rows();
    let cache = data.path().join(".vz-validation");
    if cache.exists() {
        std::fs::remove_dir_all(&cache).unwrap();
    }
    std::fs::write(&cache, "unusable marker location").unwrap();
    let unchanged =
        active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
            .await
            .unwrap();
    assert_eq!(
        (unchanged.completed, unchanged.skipped, unchanged.failed),
        (0, 1, 0)
    );
    assert_eq!(rows(), original);
    assert!(data.path().join("generations").join(&original).is_dir());
    std::fs::write(source.path().join("b.txt"), "new searchable document").unwrap();
    // Also prevent the staged generation's marker from being written.
    std::fs::create_dir_all(data.path().join(".vz-scan")).unwrap();
    std::fs::write(data.path().join(".vz-scan/.vz-validation"), "blocked").unwrap();
    let added = active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
        .await
        .unwrap();
    assert_eq!((added.completed, added.skipped, added.failed), (1, 1, 0));
    assert_eq!(rows(), original);
    assert_eq!(
        State::open(data.path()).unwrap().documents().unwrap().len(),
        2
    );
    // A usable directory with an unreplaceable marker exercises persistence
    // failure after successful deep validation, rather than directory failure.
    std::fs::remove_file(&cache).unwrap();
    std::fs::create_dir(&cache).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::create_dir(cache.join(format!("{original}.json"))).unwrap();
    let unchanged =
        active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
            .await
            .unwrap();
    assert_eq!(
        (unchanged.completed, unchanged.skipped, unchanged.failed),
        (0, 2, 0)
    );
    assert_eq!(rows(), original);
}

#[tokio::test]
async fn damaged_token_first_third_is_bounded_and_rebuilds_only_one_document() {
    use vz_core::{active_index, engine::Engines};
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    let engines =
        Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true).unwrap();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("a.txt"),
        "alpha bravo charlie delta echo foxtrot zebras zebras yankee",
    )
    .unwrap();
    std::fs::write(source.path().join("b.txt"), "untouched document").unwrap();
    active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
        .await
        .unwrap();
    let rows = || {
        let db = State::review_connection(data.path()).unwrap();
        let mut stmt = db
            .prepare("SELECT generation FROM documents ORDER BY path")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let original = rows();
    let generation = data.path().join("generations").join(&original[0]);
    let tokens = walkdir::WalkDir::new(generation.join("lance/chunks.lance/_indices"))
        .into_iter()
        .map(|e| e.unwrap())
        .find(|e| e.file_name() == "part_0_tokens.lance")
        .unwrap()
        .into_path();
    let mut bytes = std::fs::read(&tokens).unwrap();
    let third = bytes.len() / 3;
    for byte in &mut bytes[..third] {
        *byte = !*byte;
    }
    std::fs::write(tokens, bytes).unwrap();
    let start = Instant::now();
    let repaired =
        active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
            .await
            .unwrap();
    let elapsed = start.elapsed();
    println!(
        "fold16 damaged token first third: elapsed_ms={} completed={} skipped={} failed={}",
        elapsed.as_millis(),
        repaired.completed,
        repaired.skipped,
        repaired.failed
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "supervised validation must remain bounded"
    );
    assert_eq!(
        (repaired.completed, repaired.skipped, repaired.failed),
        (1, 1, 0)
    );
    let after = rows();
    assert_ne!(after[0], original[0]);
    assert_eq!(after[1], original[1]);
    let state = State::open(data.path()).unwrap();
    assert!(state
        .search("zebras", &engines.vector("zebras", true).unwrap(), 5)
        .await
        .unwrap()
        .iter()
        .any(|hit| hit.chunk.text.contains("zebras")));
}

#[tokio::test]
async fn markerless_legacy_index_retains_every_row_when_worker_is_unavailable() {
    use vz_core::{active_index, engine::Engines};
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    let engines =
        Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true).unwrap();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        std::fs::write(
            source.path().join(format!("{name}.txt")),
            format!("Healthy archive {name}"),
        )
        .unwrap();
    }
    active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
        .await
        .unwrap();
    let snapshot = || {
        walkdir::WalkDir::new(data.path())
            .into_iter()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.path().strip_prefix(data.path()).unwrap().to_owned(),
                    if entry.file_type().is_file() {
                        std::fs::read(entry.path()).unwrap()
                    } else {
                        Vec::new()
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let worker = vz_core::worker::executable().unwrap();
    let hidden = worker.with_extension("fold17-hidden");
    struct Restore(std::path::PathBuf, std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            std::fs::rename(&self.1, &self.0).expect("restore worker");
        }
    }
    std::fs::rename(&worker, &hidden).unwrap();
    let restore = Restore(worker, hidden);
    let warm = active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
        .await
        .unwrap();
    assert_eq!((warm.completed, warm.skipped, warm.failed), (0, 3, 0));
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
            },
        };
        let cache = data.path().join(".vz-validation");
        let name: Vec<u16> = cache.as_os_str().encode_wide().chain(Some(0)).collect();
        let sddl: Vec<u16> = "D:P(A;OICI;FA;;;WD)\0".encode_utf16().collect();
        let mut descriptor = std::ptr::null_mut();
        unsafe {
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut()
                ),
                0
            );
            let changed = SetFileSecurityW(
                name.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            );
            LocalFree(descriptor);
            assert_ne!(changed, 0);
        }
        // Matching markers exist, but their prior DACL must force a real probe.
        let error =
            active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
                .await
                .unwrap_err();
        assert!(format!("{error:#}").contains("infrastructure unavailable"));
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    }
    std::fs::remove_dir_all(data.path().join(".vz-validation")).unwrap();
    let before = snapshot();
    let error = active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
        .await
        .unwrap_err();
    drop(restore);
    assert!(
        format!("{error:#}").contains("infrastructure unavailable"),
        "{error:#}"
    );
    assert_eq!(
        snapshot(),
        before,
        "all rows, selection, jobs and generation bytes must survive"
    );
    assert_eq!(
        State::open(data.path()).unwrap().documents().unwrap().len(),
        3
    );
}
