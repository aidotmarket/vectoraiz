use super::regression_common::Fixture;
use super::{folder_unreviewed as folder, folder_with_checks, folder_with_staged_check};
use std::fs;

#[test]
fn complete_pack_and_nested_files() {
    let f = Fixture::new();
    let result = folder(&f.pack, "fixture", &f.source, &f.staging).unwrap();
    assert_eq!(result["mode"], "folder");
    assert_eq!(result["pack_id"], "fixture");
    f.assert_pack(&f.source.join("vz/fixture"), &result);
    assert_eq!(fs::read_dir(&f.staging).unwrap().count(), 0);
}

#[test]
fn inside_source_and_equal_source_refused() {
    let f = Fixture::new();
    let inside = f.source.join("inside");
    fs::create_dir(&inside).unwrap();
    for staging in [&inside, &f.source] {
        assert!(folder(&f.pack, "fixture", &f.source, staging)
            .unwrap_err()
            .to_string()
            .contains("outside"));
    }
    f.assert_source_empty();
}

#[test]
fn existing_destination_is_preserved() {
    let f = Fixture::new();
    let destination = f.source.join("vz/fixture");
    fs::create_dir_all(&destination).unwrap();
    fs::write(destination.join("sentinel"), b"keep").unwrap();
    assert!(folder(&f.pack, "fixture", &f.source, &f.staging)
        .unwrap_err()
        .to_string()
        .contains("already exists"));
    assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"keep");
}

#[test]
fn concurrent_destination_is_not_replaced() {
    let f = Fixture::new();
    let destination = f.source.join("vz/fixture");
    assert!(
        folder_with_checks(&f.pack, "fixture", &f.source, &f.staging, true, || {
            fs::create_dir(&destination)?;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn tampered_pack_and_extra_payload_refused() {
    for extra in [false, true] {
        let f = Fixture::new();
        fs::write(
            f.pack.join(if extra {
                "extra.txt"
            } else {
                "documents.parquet"
            }),
            b"tampered",
        )
        .unwrap();
        assert!(folder(&f.pack, "fixture", &f.source, &f.staging).is_err());
        f.assert_source_empty();
        assert_eq!(fs::read_dir(&f.staging).unwrap().count(), 0);
    }
}

#[test]
fn failure_before_rename_and_retry() {
    let f = Fixture::new();
    let error = folder_with_checks(&f.pack, "fixture", &f.source, &f.staging, true, || {
        anyhow::bail!("simulated interruption")
    })
    .unwrap_err();
    assert!(error.to_string().contains("simulated interruption"));
    f.assert_source_empty();
    assert_eq!(fs::read_dir(&f.staging).unwrap().count(), 0);
    let result = folder(&f.pack, "fixture", &f.source, &f.staging).unwrap();
    f.assert_pack(&f.source.join("vz/fixture"), &result);
}

#[test]
fn different_filesystems_refused_before_copy() {
    let f = Fixture::new();
    assert!(
        folder_with_checks(&f.pack, "fixture", &f.source, &f.staging, false, || panic!(
            "must not reach rename"
        ))
        .unwrap_err()
        .to_string()
        .contains("different filesystems")
    );
    f.assert_source_empty();
    assert_eq!(fs::read_dir(&f.staging).unwrap().count(), 0);
}

#[test]
fn unsafe_id_refused() {
    let f = Fixture::new();
    for id in ["", "..", "../escape", "C:drive", "a/b", "a\\b"] {
        assert!(folder(&f.pack, id, &f.source, &f.staging).is_err());
    }
}

#[cfg(unix)]
#[test]
fn canonical_staging_alias_inside_source_refused() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let alias = f.root.join("alias");
    symlink(&f.source, &alias).unwrap();
    assert!(folder(&f.pack, "fixture", &f.source, &alias)
        .unwrap_err()
        .to_string()
        .contains("outside"));
    let link = f.pack.join("link");
    symlink(f.pack.join("documents.parquet"), link).unwrap();
    assert!(folder(&f.pack, "fixture", &f.source, &f.staging).is_err());
}

#[test]
fn corrupted_staged_file_is_not_published() {
    let f = Fixture::new();
    let error = folder_with_staged_check(&f.pack, "fixture", &f.source, &f.staging, |stage| {
        // Same length: the staged SHA-256 check must catch this.
        let path = stage.join("documents.parquet");
        let mut bytes = fs::read(&path)?;
        bytes[0] ^= 1;
        fs::write(path, bytes)?;
        Ok(())
    })
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("staged file verification failed"));
    f.assert_source_empty();
    assert!(!f.source.join("vz/fixture").exists());
    assert_eq!(fs::read_dir(&f.staging).unwrap().count(), 0);
}

#[test]
fn late_staged_swap_refuses_promotion_and_preserves_other_jobs() {
    let f = Fixture::new();
    let other = f.staging.join("other-job.partial");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("keep"), b"unrelated seller data").unwrap();
    let result = folder_with_checks(&f.pack, "fixture", &f.source, &f.staging, true, || {
        let stage = fs::read_dir(&f.staging)?
            .filter_map(|entry| entry.ok())
            .find(|entry| entry.path() != other)
            .unwrap();
        fs::write(
            stage.path().join("documents.parquet"),
            b"late swapped payload",
        )?;
        Ok(())
    });
    assert!(result.is_err());
    assert!(!f.source.join("vz/fixture").exists());
    assert_eq!(
        fs::read(other.join("keep")).unwrap(),
        b"unrelated seller data"
    );
    assert_eq!(fs::read_dir(&f.staging).unwrap().count(), 1);
}

#[test]
fn multi_chunk_copy_checks_policy_per_chunk_and_binding_at_fences() {
    use std::cell::Cell;
    let f = Fixture::new();
    let bytes = vec![b'x'; 5 * 65536 + 7];
    fs::write(f.pack.join("documents.parquet"), &bytes).unwrap();
    let manifest_path = f.pack.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["files"]["documents.parquet"] = vz_core::pipeline::sha256(&bytes).into();
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let fences = Cell::new(0);
    let chunks = Cell::new(0);
    let result = super::folder_inner(
        &f.pack,
        "fixture",
        &f.source,
        &f.staging,
        |_, _| Ok(true),
        |_| Ok(()),
        || Ok(()),
        || {
            fences.set(fences.get() + 1);
            Ok(())
        },
        || {
            chunks.set(chunks.get() + 1);
            Ok(())
        },
    )
    .unwrap();
    // Three files, then pre-promotion and post-promotion checks. Full tree
    // validation must not scale with the number of 64 KiB payload chunks.
    assert_eq!(fences.get(), 5);
    assert_eq!(chunks.get(), 8);
    f.assert_pack(&f.source.join("vz/fixture"), &result);
}
