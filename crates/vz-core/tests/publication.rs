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
    fs::write(
        a.join("second.txt"),
        "Second document stays in the resolved source.",
    )
    .unwrap();
    let mut retargeted = false;
    active_index::reindex(
        data.path(),
        &request,
        &engines,
        &assets,
        |_| {
            if !retargeted {
                fs::remove_file(&request).unwrap();
                symlink(&b, &request).unwrap();
                retargeted = true;
            }
        },
        || false,
    )
    .await
    .unwrap();
    assert!(retargeted);
    let historical = active_index::last_indexed_folder(data.path()).unwrap();
    assert_eq!(Path::new(&historical), a.canonicalize().unwrap());
    assert_eq!(documents(data.path()).len(), 2);
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

#[tokio::test]
async fn cleanup_preserves_unowned_siblings_and_reclaims_superseded_recipe() {
    let (mut engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("a.txt"), "Ownership regression content.").unwrap();
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
    let old = active_index::resolve(data.path());
    let unrelated = data.path().join("indexes/unrelated");
    fs::create_dir_all(unrelated.join("nested")).unwrap();
    fs::write(unrelated.join("nested/precious"), b"keep exactly\x00\xff").unwrap();
    let before = fs::read(unrelated.join("nested/precious")).unwrap();
    // A syntactically valid recipe name and copied recipe are insufficient:
    // a directory without a verified index schema is still not ours.
    let decoy = data
        .path()
        .join("indexes")
        .join(active_index::recipe_id(&engines.recipe).unwrap());
    fs::create_dir(&decoy).unwrap();
    fs::write(
        decoy.join("recipe.json"),
        serde_json::to_vec(&engines.recipe).unwrap(),
    )
    .unwrap();
    fs::write(decoy.join("writer.lock"), b"unrelated lock bytes").unwrap();
    fs::write(decoy.join("state.sqlite"), b"unrelated database bytes").unwrap();
    active_index::cleanup_start(data.path()).unwrap();
    assert!(!unrelated.join("writer.lock").exists());
    engines.recipe.model.push_str("-replacement");
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
    assert!(!old.exists());
    assert_eq!(
        fs::read(decoy.join("writer.lock")).unwrap(),
        b"unrelated lock bytes"
    );
    assert_eq!(
        fs::read(decoy.join("state.sqlite")).unwrap(),
        b"unrelated database bytes"
    );
    assert_eq!(fs::read_dir(&decoy).unwrap().count(), 3);
    assert_eq!(fs::read(unrelated.join("nested/precious")).unwrap(), before);
    assert_eq!(fs::read_dir(&unrelated).unwrap().count(), 1);
    assert!(!unrelated.join("writer.lock").exists());
}

#[tokio::test]
async fn missing_current_source_case_follows_actual_volume_semantics() {
    let (engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let source = parent.path().join("ÉcoleSource");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("a.txt"), "Volume-sensitive revocation content.").unwrap();
    active_index::reindex(data.path(), &source, &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    let alternate = parent.path().join("éCOLEsOURCE");
    let insensitive = same_file::is_same_file(&alternate, &source).unwrap_or(false);
    println!("missing-source identity: case_insensitive={insensitive}");
    fs::remove_dir_all(&source).unwrap();
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        let original = fs::metadata(parent.path()).unwrap().permissions();
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o555)).unwrap();
        original
    };
    assert!(
        active_index::reindex(data.path(), &alternate, &engines, &assets, |_| {}, || false)
            .await
            .is_err()
    );
    assert_eq!(documents(data.path()).is_empty(), insensitive);
    assert_eq!(
        fs::read_dir(parent.path()).unwrap().count(),
        0,
        "source identity comparison must not write probes"
    );
    #[cfg(unix)]
    fs::set_permissions(parent.path(), permissions).unwrap();
    if !insensitive {
        assert!(
            active_index::reindex(data.path(), &source, &engines, &assets, |_| {}, || false)
                .await
                .is_err()
        );
        assert!(documents(data.path()).is_empty());
    }
}

/// The baseline driver is compiled from examples/r10-baseline.rs against the
/// pinned base, outside this checkout. Run serially on an idle machine.
#[tokio::test]
#[ignore = "requires VZ_R10_BASELINE_BIN built against ef34f139"]
async fn t15_r10_measured_nochange_against_ef34f139() {
    use std::{process::Command, time::Instant};
    let baseline = std::env::var_os("VZ_R10_BASELINE_BIN").expect("baseline driver");
    let (engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    for i in 0..200 {
        fs::write(
            source.path().join(format!("{i:04}.txt")),
            format!("Orbital archive retention is thirty days. Fixture document {i}. "),
        )
        .unwrap();
    }
    let initial =
        active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
            .await
            .unwrap();
    assert_eq!(initial.completed, 200);
    let generations = || {
        fs::read_dir(data.path().join("generations"))
            .unwrap()
            .count()
    };
    let count = generations();
    let pins = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spike/fixtures/models.json");
    let mut base_times = vec![];
    let mut candidate_times = vec![];
    // One warm-up pair and seven recorded pairs on identical files and index.
    for round in 0..8 {
        let output = Command::new("rtk")
            .arg("run")
            .arg(&baseline)
            .args([data.path(), source.path(), &assets, &pins])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let base: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(base["completed"], 0);
        assert_eq!(base["skipped"], 200);
        let before = fs::read(data.path().join("state.sqlite")).unwrap();
        let start = Instant::now();
        let result =
            active_index::index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
                .await
                .unwrap();
        let elapsed = start.elapsed().as_secs_f64();
        assert_eq!((result.completed, result.skipped), (0, 200));
        assert_eq!(generations(), count);
        assert!(!data.path().join(".vz-scan/scan.sqlite").exists());
        assert_eq!(fs::read(data.path().join("state.sqlite")).unwrap(), before);
        if round > 0 {
            base_times.push(base["seconds"].as_f64().unwrap());
            candidate_times.push(elapsed);
            println!("R10 round={round} base_ms={:.3} candidate_ms={:.3} generations={count}->{count} workspace_rows=0 reembeddings=0", base_times.last().unwrap()*1000., elapsed*1000.);
        }
    }
    base_times.sort_by(f64::total_cmp);
    candidate_times.sort_by(f64::total_cmp);
    let ratio = candidate_times[3] / base_times[3];
    println!(
        "R10 median base_ms={:.3} candidate_ms={:.3} ratio={ratio:.4}",
        base_times[3] * 1000.,
        candidate_times[3] * 1000.
    );
    assert!(ratio <= 1.2, "R10 exceeds 1.2x: {ratio:.4}");
}

#[tokio::test]
async fn refused_different_source_preserves_retry_workspace_until_first_write() {
    use std::sync::atomic::{AtomicBool, Ordering};
    fn snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn visit(
            root: &Path,
            path: &Path,
            files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
        ) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(root).unwrap().to_owned(),
                        fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut files = Default::default();
        visit(root, root, &mut files);
        files
    }
    let (engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    fs::write(
        a.path().join("a.txt"),
        "Original published workspace content.",
    )
    .unwrap();
    active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    fs::write(
        a.path().join("a.txt"),
        "Reusable private changed workspace content.",
    )
    .unwrap();
    let stop = AtomicBool::new(false);
    assert!(active_index::reindex(
        data.path(),
        a.path(),
        &engines,
        &assets,
        |_| stop.store(true, Ordering::SeqCst),
        || stop.load(Ordering::SeqCst)
    )
    .await
    .is_err());
    let workspace = active_index::resolve(data.path()).join(".vz-scan");
    let before = snapshot(&workspace);
    // An empty, completed source is publishable by the existing decision table.
    // Use a content-refused source to exercise a switch with no first write.
    fs::write(b.path().join("bad.pdf"), b"invalid PDF").unwrap();
    for other in [b.path().join("missing"), b.path().to_owned()] {
        assert!(
            active_index::reindex(data.path(), &other, &engines, &assets, |_| {}, || false)
                .await
                .is_err()
        );
        assert_eq!(snapshot(&workspace), before);
    }
    let resumed = active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    assert_eq!((resumed.completed, resumed.skipped), (0, 1));
}

#[tokio::test]
async fn t6_nochange_a_reclaims_refused_b_and_missing_recipe_candidates_are_bounded() {
    let (mut engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    fs::write(a.path().join("a.txt"), "Unchanged published content.").unwrap();
    active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    let root = active_index::resolve(data.path());
    let before = fs::read(root.join("state.sqlite")).unwrap();
    fs::write(b.path().join("good.txt"), "Private B reusable content.").unwrap();
    fs::write(b.path().join("bad.pdf"), "not a PDF").unwrap();
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
    assert!(root.join(".vz-scan/scan.sqlite").exists());
    let report = active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    assert_eq!((report.completed, report.skipped), (0, 1));
    assert!(!root.join(".vz-scan").exists());
    assert_eq!(fs::read(root.join("state.sqlite")).unwrap(), before);
    engines.recipe.model.push_str("-changed");
    for _ in 0..4 {
        assert!(active_index::reindex(
            data.path(),
            &b.path().join("missing"),
            &engines,
            &assets,
            |_| {},
            || false
        )
        .await
        .is_err());
        assert_eq!(
            fs::read_dir(data.path().join("indexes")).unwrap().count(),
            1
        );
    }
    active_index::reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
        .await
        .unwrap();
    assert_eq!(
        fs::read_dir(data.path().join("indexes")).unwrap().count(),
        1
    );
}

#[tokio::test]
async fn committed_publication_wal_without_shm_survives_reindex() {
    let (engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("a.txt"), "First publication document.").unwrap();
    fs::write(
        source.path().join("b.txt"),
        "Surviving publication document.",
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
    let root = active_index::resolve_state(data.path());
    let original = rows(data.path());
    let old_generation = &original[1].1;
    let generation = uuid::Uuid::new_v4().simple().to_string();
    // Stage an independently named generation that only the WAL references.
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
    copy_tree(
        &root.join("generations").join(old_generation),
        &root.join("generations").join(&generation),
    );
    let db = rusqlite::Connection::open(root.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE index_metadata SET value='old selection' WHERE key='selected_folder'",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let main = fs::read(root.join("state.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; BEGIN IMMEDIATE")
        .unwrap();
    db.execute("DELETE FROM documents WHERE path=?1", [&original[0].0])
        .unwrap();
    db.execute(
        "UPDATE documents SET generation=?1 WHERE path=?2",
        rusqlite::params![generation, original[1].0],
    )
    .unwrap();
    db.execute(
        "UPDATE index_metadata SET value=?1 WHERE key='selected_folder'",
        [source.path().canonicalize().unwrap().to_str().unwrap()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO jobs(path,status,reason) VALUES(?1,'completed','committed WAL publication')",
        [&original[1].0],
    )
    .unwrap();
    db.execute_batch("COMMIT").unwrap();
    let wal = fs::read(root.join("state.sqlite-wal")).unwrap();
    assert!(wal.len() > 32);
    drop(db);
    // Model a live backup/restore: committed DB + WAL, with only SHM absent.
    fs::write(root.join("state.sqlite"), main).unwrap();
    fs::write(root.join("state.sqlite-wal"), wal).unwrap();
    assert!(!root.join("state.sqlite-shm").exists());
    fs::remove_file(source.path().join("a.txt")).unwrap();
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
    assert_eq!((result.completed, result.skipped), (0, 1));
    assert_eq!(
        rows(data.path()),
        vec![(original[1].0.clone(), generation.clone())]
    );
    assert!(root.join("generations").join(&generation).exists());
    assert!(!root.join("generations").join(&original[0].1).exists());
    assert_eq!(
        State::review_selected_folder(&root).unwrap(),
        Some(source.path().canonicalize().unwrap())
    );
    let db = State::review_connection(&root).unwrap();
    let reason: String = db
        .query_row(
            "SELECT reason FROM jobs WHERE path=?1 ORDER BY id DESC LIMIT 1",
            [&original[1].0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reason, "committed WAL publication");
    assert!(!fs::read_dir(root.parent().unwrap()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".orphan-")));
}

#[tokio::test]
async fn unsupported_checksum_valid_wal_stops_before_cleanup() {
    let (mut engines, assets) = fixture();
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("a.txt"), "Publication to preserve.").unwrap();
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
    let orphan = root.join("generations/unreferenced-sentinel");
    fs::create_dir(&orphan).unwrap();
    fs::write(orphan.join("bytes"), b"must survive rejected recovery").unwrap();
    let main = fs::read(root.join("state.sqlite")).unwrap();
    let mut header = [0_u8; 32];
    header[..4].copy_from_slice(&0x377f0682_u32.to_be_bytes());
    header[4..8].copy_from_slice(&3007001_u32.to_be_bytes());
    header[8..12].copy_from_slice(&4096_u32.to_be_bytes());
    let mut sums = (0_u32, 0_u32);
    for pair in header[..24].chunks_exact(8) {
        sums.0 = sums
            .0
            .wrapping_add(u32::from_le_bytes(pair[..4].try_into().unwrap()))
            .wrapping_add(sums.1);
        sums.1 = sums
            .1
            .wrapping_add(u32::from_le_bytes(pair[4..].try_into().unwrap()))
            .wrapping_add(sums.0);
    }
    header[24..28].copy_from_slice(&sums.0.to_be_bytes());
    header[28..].copy_from_slice(&sums.1.to_be_bytes());
    let wal = root.join("state.sqlite-wal");
    fs::write(&wal, header).unwrap();
    for recipe_change in [false, true] {
        if recipe_change {
            engines.recipe.model.push_str("-new");
        }
        let error = active_index::reindex(
            data.path(),
            source.path(),
            &engines,
            &assets,
            |_| {},
            || false,
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("unsupported SQLite WAL format version 3007001"));
        assert!(orphan.join("bytes").exists());
    }
    assert_eq!(fs::read(&wal).unwrap(), header);
    assert_eq!(fs::read(root.join("state.sqlite")).unwrap(), main);
    assert!(orphan.join("bytes").exists());
}
