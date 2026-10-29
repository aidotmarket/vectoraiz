mod common;
use common::Fixture;
use std::fs;
use vz_pack::export::{folder, folder_with_checks};

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
    assert_eq!(fs::read_dir(&f.staging).unwrap().count(), 1);
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
