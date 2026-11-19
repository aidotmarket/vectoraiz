//! Approved S1987 publication contract exercised through the public scan APIs.
use std::{
    fs,
    path::{Path, PathBuf},
};
use vz_core::{
    active_index::{self, ScanControl},
    engine::Engines,
    storage::State,
};

fn fixture() -> (Engines, PathBuf) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = repo.join(".assets");
    (
        Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true).unwrap(),
        assets,
    )
}
fn documents(data: &Path) -> Vec<vz_core::pipeline::Document> {
    State::review_documents(&active_index::resolve_state(data)).unwrap()
}
fn rows(data: &Path) -> Vec<(String, String)> {
    let db = State::review_connection(&active_index::resolve_state(data)).unwrap();
    let result = db
        .prepare("SELECT path,generation FROM documents ORDER BY path")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    result
}

#[tokio::test]
async fn t1_t4_t6_recipe_change_content_revocation_reader_pin_and_reclamation() {
    let (mut engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("good.txt"), "First searchable revision.").unwrap();
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
    let old_root = active_index::resolve(data.path());
    let old = State::open(&old_root).unwrap();
    fs::write(source.path().join("good.txt"), "").unwrap();
    engines.recipe.model.push_str("-new");
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
    assert_eq!(result.failed, 1);
    assert!(documents(data.path()).is_empty());
    assert_eq!(
        old.documents().unwrap()[0].chunks[0].text,
        "First searchable revision."
    );
    assert!(old_root.exists());
    drop(old);
    {
        let _fence = active_index::selection_fence(data.path(), true).unwrap();
        active_index::cleanup_start(data.path()).unwrap();
    }
    assert!(!old_root.exists());
    for n in 0..4 {
        fs::write(
            source.path().join("good.txt"),
            format!("Revision number {n} is searchable."),
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
        assert_eq!(
            fs::read_dir(active_index::resolve(data.path()).join("generations"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            fs::read_dir(data.path().join("indexes")).unwrap().count(),
            1
        );
    }
}

#[tokio::test]
async fn t2_t5_t11_t12_refused_switch_keeps_bytes_jobs_and_resumes_both_recipes() {
    let (mut engines, assets) = fixture();
    for changed_recipe in [false, true] {
        let data = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("a.txt"), "Original selected folder content.").unwrap();
        active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
        let before = State::review_content(&active_index::resolve(data.path()))
            .unwrap()
            .0;
        let bytes = fs::read(active_index::resolve(data.path()).join("state.sqlite")).unwrap();
        let old_rows = rows(data.path());
        fs::write(b.path().join("a.txt"), "Private retry success.").unwrap();
        fs::write(b.path().join("b.pdf"), "not a PDF").unwrap();
        if changed_recipe {
            engines.recipe.model.push_str("-next");
        }
        assert!(active_index::reindex(
            data.path(),
            b.path(),
            &engines,
            &b.path().join("missing-assets"),
            |_| {},
            || false
        )
        .await
        .is_err());
        assert_eq!(
            before,
            State::review_content(&active_index::resolve(data.path()))
                .unwrap()
                .0
        );
        assert_eq!(
            bytes,
            fs::read(active_index::resolve(data.path()).join("state.sqlite")).unwrap()
        );
        assert_eq!(old_rows, rows(data.path()));
        assert_eq!(documents(data.path()).len(), 1);
        let result =
            active_index::reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
                .await
                .unwrap();
        assert_eq!((result.skipped, result.failed), (1, 1));
        assert!(documents(data.path())[0]
            .path
            .starts_with(b.path().canonicalize().unwrap().to_str().unwrap()));
        let root = active_index::resolve(data.path());
        let generation = rows(data.path())[0].1.clone();
        fs::write(
            root.join("generations")
                .join(generation)
                .join("document.json"),
            "broken",
        )
        .unwrap();
        let result =
            active_index::reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
                .await
                .unwrap();
        assert_eq!(result.completed, 1);
    }
}

#[tokio::test]
async fn t5_t7_cancel_reuses_only_confirmed_successes_and_empty_cancel_has_no_hint() {
    let (engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    fs::write(a.path().join("a.txt"), "Selected original source.").unwrap();
    active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(b.path().join(name), format!("Private content for {name}.")).unwrap();
    }
    let control = ScanControl::default();
    let cancel = control.clone();
    let old = rows(data.path());
    let error = active_index::reindex_controlled(
        data.path(),
        b.path(),
        &engines,
        &assets,
        |p| {
            if p.completed == 2 {
                assert!(cancel.cancel());
            }
        },
        &control,
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("resume"));
    assert_eq!(old, rows(data.path()));
    fs::remove_file(b.path().join("a.txt")).unwrap();
    fs::write(b.path().join("b.txt"), "").unwrap();
    let result = active_index::reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    assert_eq!((result.completed, result.skipped, result.failed), (1, 0, 1));
    let docs = documents(data.path());
    assert_eq!(docs.len(), 1);
    assert!(docs[0].path.ends_with("c.txt"));
    let control = ScanControl::default();
    assert!(control.cancel());
    let error = active_index::reindex_controlled(
        data.path(),
        b.path(),
        &engines,
        &assets,
        |_| {},
        &control,
    )
    .await
    .unwrap_err();
    assert!(!format!("{error:#}").contains("resume"));
}

#[tokio::test]
async fn t10_t15_explicit_flat_nochange_is_byte_exact_and_missing_current_revokes() {
    let (engines, assets) = fixture();
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("phase0-index");
    let source = tempfile::tempdir().unwrap();
    fs::write(
        source.path().join("a.txt"),
        "Explicit flat searchable text.",
    )
    .unwrap();
    active_index::index_explicit(&data, source.path(), &engines, &assets, |_| {})
        .await
        .unwrap();
    let before = fs::read(data.join("state.sqlite")).unwrap();
    let initial = rows(&data);
    assert!(!parent.path().join("indexes").exists());
    assert!(!data.join("active-index.json").exists());
    let result = active_index::index_explicit(&data, source.path(), &engines, &assets, |_| {})
        .await
        .unwrap();
    assert_eq!(result.skipped, 1);
    assert_eq!(before, fs::read(data.join("state.sqlite")).unwrap());
    assert!(!data.join(".vz-scan/scan.sqlite").exists());
    assert_eq!(initial, rows(&data));
    let path = source.path().to_owned();
    source.close().unwrap();
    assert!(
        active_index::index_explicit(&data, &path, &engines, &assets, |_| {})
            .await
            .is_err()
    );
    assert!(documents(&data).is_empty());
    assert_eq!(fs::read_dir(data.join("generations")).unwrap().count(), 0);
}

#[tokio::test]
async fn t9_dangling_pointer_keeps_fallback_until_fresh_rename() {
    let (engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(
        source.path().join("a.txt"),
        "Fallback remains available before publication.",
    )
    .unwrap();
    let legacy = data.path().join("phase0-index");
    active_index::index_explicit(&legacy, source.path(), &engines, &assets, |_| {})
        .await
        .unwrap();
    let id = active_index::recipe_id(&engines.recipe).unwrap();
    let pointer=serde_json::to_vec(&serde_json::json!({"dir":format!("indexes/{id}-00000000-0000-0000-0000-000000000000"),"recipe_id":id})).unwrap();
    fs::write(data.path().join("active-index.json"), &pointer).unwrap();
    active_index::reindex(
        data.path(),
        source.path(),
        &engines,
        &assets,
        |_| {
            assert_eq!(active_index::resolve_state(data.path()), legacy);
            assert_eq!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                pointer
            );
        },
        || false,
    )
    .await
    .unwrap();
    assert_ne!(active_index::resolve(data.path()), legacy);
    assert!(!legacy.exists());
    assert!(!active_index::needs_pointer_repair(data.path()));
}

#[tokio::test]
async fn t12_recipe_infrastructure_failure_revokes_changed_and_absent_but_retains_unchanged() {
    let (mut engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    for name in ["good.txt", "deleted.txt"] {
        fs::write(
            source.path().join(name),
            format!("Original {name} content."),
        )
        .unwrap();
    }
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spike/fixtures/documents/access.pdf"),
        source.path().join("unchanged.pdf"),
    )
    .unwrap();
    fs::copy(
        source.path().join("unchanged.pdf"),
        source.path().join("changed.pdf"),
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
    let old_root = active_index::resolve(data.path());
    let old = rows(data.path());
    fs::remove_file(source.path().join("deleted.txt")).unwrap();
    fs::write(
        source.path().join("good.txt"),
        "New privately staged success.",
    )
    .unwrap();
    let mut changed = fs::read(source.path().join("changed.pdf")).unwrap();
    changed.extend_from_slice(b"\n% changed revision\n");
    fs::write(source.path().join("changed.pdf"), changed).unwrap();
    engines.recipe.model.push_str("-new-recipe");
    let held = State::open(&old_root).unwrap();
    let error = active_index::reindex(
        data.path(),
        source.path(),
        &engines,
        &source.path().join("no-assets"),
        |_| {},
        || false,
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("revocation pending"));
    assert_eq!(old, rows(data.path()));
    drop(held);
    let original_pdf = fs::read(source.path().join("unchanged.pdf")).unwrap();
    assert!(active_index::reindex(
        data.path(),
        source.path(),
        &engines,
        &source.path().join("no-assets"),
        |p| {
            if p.path.ends_with("changed.pdf") && p.failed > 0 {
                fs::write(source.path().join("changed.pdf"), &original_pdf).unwrap();
            }
        },
        || false
    )
    .await
    .is_err());
    let remaining = rows(data.path());
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().all(|(p, _)| !p.ends_with("deleted.txt")));
    assert!(documents(data.path()).iter().any(|d| d
        .chunks
        .iter()
        .any(|c| c.text.contains("Original good.txt"))));
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
    assert_eq!((result.completed, result.skipped), (2, 1));
    assert!(!old_root.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn t3_t8_historical_selection_and_request_use_one_scan_identity() {
    use std::os::unix::fs::symlink;
    let (engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    let request = root.path().join("request");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    symlink(&a, &request).unwrap();
    fs::write(a.join("good.txt"), "Historical canonical selected source.").unwrap();
    active_index::reindex(data.path(), &request, &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    let historical = active_index::last_indexed_folder(data.path()).unwrap();
    fs::remove_file(&request).unwrap();
    symlink(&b, &request).unwrap();
    fs::write(b.join("bad.pdf"), "bad").unwrap();
    assert!(active_index::reindex(
        data.path(),
        &request,
        &engines,
        &root.path().join("no-assets"),
        |_| {},
        || false
    )
    .await
    .is_err());
    assert_eq!(
        active_index::last_indexed_folder(data.path()).unwrap(),
        historical
    );
    assert!(documents(data.path())[0].path.starts_with(&historical));
}

#[tokio::test]
async fn t2_overlapping_refusal_publishes_exact_failure_only_in_both_recipe_modes() {
    let (mut engines, assets) = fixture();
    for changed_recipe in [false, true] {
        let data = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let a = b.path().join("a");
        fs::create_dir(&a).unwrap();
        fs::write(a.join("failed.txt"), "Old content that must be revoked.").unwrap();
        fs::write(a.join("keep.txt"), "Old surviving revision.").unwrap();
        active_index::reindex(data.path(), &a, &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
        let active = active_index::resolve(data.path());
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        let kept = rows(data.path())
            .into_iter()
            .find(|(p, _)| p.ends_with("keep.txt"))
            .unwrap();
        let generation = active.join("generations").join(&kept.1);
        let before: Vec<_> = walkdir::WalkDir::new(&generation)
            .into_iter()
            .map(Result::unwrap)
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                (
                    e.path().strip_prefix(&generation).unwrap().to_owned(),
                    vz_core::pipeline::file_sha256(e.path()).unwrap(),
                )
            })
            .collect();
        fs::write(a.join("failed.txt"), "").unwrap();
        fs::write(
            a.join("keep.txt"),
            "New replacement stays private until retry.",
        )
        .unwrap();
        fs::write(
            b.path().join("new.txt"),
            "New B document stays private until retry.",
        )
        .unwrap();
        fs::write(b.path().join("bad.pdf"), "bad").unwrap();
        if changed_recipe {
            engines.recipe.model.push_str("-overlap");
        }
        assert!(active_index::reindex(
            data.path(),
            b.path(),
            &engines,
            &b.path().join("no-assets"),
            |_| {},
            || false
        )
        .await
        .is_err());
        assert_eq!(rows(data.path()), vec![kept.clone()]);
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        assert_eq!(
            active_index::last_indexed_folder(data.path()).unwrap(),
            a.canonicalize().unwrap().to_str().unwrap()
        );
        for (relative, hash) in before {
            assert_eq!(
                vz_core::pipeline::file_sha256(&generation.join(relative)).unwrap(),
                hash
            );
        }
        let db = State::review_connection(&active).unwrap();
        let failures: Vec<(String, String)> = db
            .prepare("SELECT path,reason FROM jobs WHERE status='failed'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].0.ends_with("failed.txt"));
        assert!(failures[0].1.contains("no text"));
        drop(db);
        assert_eq!(
            documents(data.path())[0].chunks[0].text,
            "Old surviving revision."
        );
        let result =
            active_index::reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
                .await
                .unwrap();
        assert_eq!((result.completed, result.skipped, result.failed), (0, 2, 2));
        assert_eq!(rows(data.path()).len(), 2);
        assert!(!generation.exists());
    }
}

#[tokio::test]
async fn t5_reverting_to_published_hash_removes_unconfirmed_staged_revision() {
    let (engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let good = source.path().join("good.txt");
    fs::write(&good, "Published original bytes.").unwrap();
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
    let before = rows(data.path());
    let active = active_index::resolve(data.path());
    fs::write(&good, "Private replacement bytes.").unwrap();
    fs::write(source.path().join("bad.pdf"), "bad").unwrap();
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
    let scan = active.join(".vz-scan/scan.sqlite");
    let staged = rusqlite::Connection::open(&scan)
        .unwrap()
        .query_row("SELECT generation FROM scan_documents", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    fs::write(&good, "Published original bytes.").unwrap();
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
    assert_eq!(before, rows(data.path()));
    let db = rusqlite::Connection::open(&scan).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM scan_documents", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    assert!(!active.join(".vz-scan/generations").join(staged).exists());
    drop(db);
    fs::remove_file(source.path().join("bad.pdf")).unwrap();
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
    assert_eq!(before, rows(data.path()));
    assert_eq!(
        documents(data.path())[0].chunks[0].text,
        "Published original bytes."
    );
}
