use super::*;
use crate::{
    active_index, engine::Engines, index::Progress, storage::State, worker::WORKER_REQUESTS,
};
use std::time::Duration;

include!("../../tests/support/deny_read.rs");

struct Fixture {
    temp: tempfile::TempDir,
    engines: Engines,
    assets: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let assets = repo.join(".assets");
        let engines =
            Engines::load_embedding(&assets, &repo.join("spike/fixtures/models.json"), true)
                .unwrap();
        let fixture = Self {
            temp: tempfile::tempdir().unwrap(),
            engines,
            assets,
        };
        fs::create_dir(fixture.source()).unwrap();
        for name in ["a", "b"] {
            fs::write(
                fixture.source().join(format!("{name}.txt")),
                format!("Orbital archive {name}."),
            )
            .unwrap();
        }
        let stats = fixture.scan().await;
        assert_eq!((stats.completed, stats.skipped, stats.failed), (2, 0, 0));
        fixture
    }

    fn source(&self) -> PathBuf {
        self.temp.path().join("source")
    }

    fn library(&self) -> PathBuf {
        self.temp.path().join("library")
    }

    fn root(&self) -> PathBuf {
        active_index::resolve(&self.library())
    }

    async fn scan(&self) -> Progress {
        active_index::reindex(
            &self.library(),
            &self.source(),
            &self.engines,
            &self.assets,
            |_| {},
            || false,
        )
        .await
        .unwrap()
    }

    fn rows(&self) -> Vec<ScanRow> {
        let mut rows = ScanRow::read(
            &State::review_connection(&self.root()).unwrap(),
            "documents",
        )
        .unwrap();
        rows.sort_by(|a, b| a.path.cmp(&b.path));
        rows
    }

    async fn assert_search(&self, available: usize, unavailable: &[&str]) {
        let state = State::open(&self.root()).unwrap();
        let report = state
            .search_with_availability("orbital", &self.engines.vector("orbital", true).unwrap(), 5)
            .await
            .unwrap();
        assert_eq!(report.hits.len(), available);
        assert_eq!(
            report
                .unavailable
                .iter()
                .map(|item| item.generation.as_str())
                .collect::<Vec<_>>(),
            unavailable,
        );
    }
}

fn origin(root: &Path, row: &ScanRow) -> Origin {
    read(root, &row.generation).unwrap().unwrap().origin
}

fn counters() -> (usize, usize) {
    (
        WORKER_REQUESTS.with(|n| n.get()),
        HASHED_FILES.with(|n| n.get()),
    )
}

fn assert_reused(stats: Progress) {
    assert_eq!((stats.completed, stats.skipped, stats.failed), (0, 2, 0));
}

#[tokio::test]
async fn warm_scan_search_scan_neither_hashes_nor_spawns_workers() {
    let f = Fixture::new().await;
    let rows = f.rows();
    let baseline = counters();
    for row in &rows {
        assert!(matches!(origin(&f.root(), row), Origin::Validated));
    }
    assert_reused(f.scan().await);
    f.assert_search(2, &[]).await;
    assert_reused(f.scan().await);
    assert_eq!(counters(), baseline);
    assert_eq!(f.rows(), rows);
    assert!(!f.root().join(".vz-scan/scan.sqlite").exists());
}

#[tokio::test]
async fn missing_and_malformed_seals_adopt_without_reembedding_then_stay_warm() {
    let f = Fixture::new().await;
    let root = f.root();
    let rows = f.rows();
    fs::remove_dir_all(root.join(DIRECTORY)).unwrap();
    let (workers, files) = counters();
    assert_reused(f.scan().await);
    let (new_workers, new_files) = counters();
    assert_eq!(new_workers, workers);
    assert!(new_files > files);
    for row in &rows {
        assert!(matches!(origin(&root, row), Origin::Adopted));
    }
    assert_reused(f.scan().await);
    assert_eq!(counters(), (new_workers, new_files));

    fs::write(path(&root, &rows[0].generation), b"{broken").unwrap();
    assert_reused(f.scan().await);
    assert!(matches!(origin(&root, &rows[0]), Origin::Adopted));
    assert_eq!(counters().0, workers);
    assert_eq!(f.rows(), rows);
}

fn edit_seal(root: &Path, row: &ScanRow, field: &str, value: serde_json::Value) {
    let path = path(root, &row.generation);
    let mut seal: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    seal[field] = value;
    fs::write(path, serde_json::to_vec(&seal).unwrap()).unwrap();
}

fn data_file(generation: &Path) -> PathBuf {
    fs::read_dir(generation.join("lance/chunks.lance/data"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
}

fn set_mtime(file: &Path, mtime: SystemTime) {
    fs::File::options()
        .write(true)
        .open(file)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(mtime))
        .unwrap();
}

fn mutate_byte(file: &Path) {
    let mut bytes = fs::read(file).unwrap();
    bytes[0] ^= 1;
    fs::write(file, bytes).unwrap();
}

#[tokio::test]
async fn reader_version_refresh_compares_old_hashes_and_keeps_validated_origin() {
    let f = Fixture::new().await;
    let root = f.root();
    let rows = f.rows();
    let (workers, hashes) = counters();
    for row in &rows {
        edit_seal(&root, row, "reader_format_version", 0.into());
    }
    assert_reused(f.scan().await);
    assert_eq!(counters().0, workers);
    assert!(counters().1 > hashes);
    for row in &rows {
        let seal = read(&root, &row.generation).unwrap().unwrap();
        assert_eq!(seal.reader_format_version, READER_FORMAT_VERSION);
        assert!(matches!(seal.origin, Origin::Validated));
    }
    assert_eq!(f.rows(), rows);

    // The version change must detect even a mutation with unchanged size and time.
    let file = data_file(&root.join("generations").join(&rows[0].generation));
    let mtime = file.metadata().unwrap().modified().unwrap();
    mutate_byte(&file);
    set_mtime(&file, mtime);
    edit_seal(&root, &rows[0], "reader_format_version", 0.into());
    let report = f.scan().await;
    assert_eq!((report.completed, report.skipped, report.failed), (1, 1, 0));
    assert_ne!(f.rows()[0].generation, rows[0].generation);
    assert_eq!(f.rows()[1], rows[1]);
    f.assert_search(2, &[]).await;
}

#[tokio::test]
async fn layout_bytes_links_and_foreign_bindings_rebuild_only_damaged_document() {
    let f = Fixture::new().await;
    let root = f.root();
    let control = f.rows().pop().unwrap();
    for case in [
        "generation node",
        "document node",
        "_versions node",
        "_indices node",
        "truncated",
        "changed bytes",
        "missing",
        "extra",
        "symlink",
        "generation",
        "source_hash",
        "recipe_id",
    ] {
        let row = f.rows().remove(0);
        let directory = root.join("generations").join(&row.generation);
        let table = directory.join("lance/chunks.lance");
        let doc = directory.join("document.json");
        match case {
            "generation node" => {
                fs::remove_dir_all(&directory).unwrap();
                fs::write(&directory, b"file").unwrap();
            }
            "document node" => {
                fs::remove_file(&doc).unwrap();
                fs::create_dir(&doc).unwrap();
            }
            "_versions node" | "_indices node" => {
                let node = table.join(case.split_whitespace().next().unwrap());
                fs::remove_dir_all(&node).unwrap();
                fs::write(node, b"file").unwrap();
            }
            "truncated" => {
                let file = data_file(&directory);
                let mtime = file.metadata().unwrap().modified().unwrap();
                fs::File::options()
                    .write(true)
                    .open(&file)
                    .unwrap()
                    .set_len(1)
                    .unwrap();
                set_mtime(&file, mtime);
            }
            "changed bytes" => {
                let file = data_file(&directory);
                let mtime = file.metadata().unwrap().modified().unwrap();
                mutate_byte(&file);
                set_mtime(&file, mtime + Duration::from_secs(2));
            }
            "missing" => fs::remove_file(data_file(&directory)).unwrap(),
            "extra" => fs::write(directory.join(".DS_Store"), b"extra").unwrap(),
            "symlink" => {
                let identical = f.temp.path().join("identical.json");
                fs::rename(&doc, &identical).unwrap();
                #[cfg(unix)]
                std::os::unix::fs::symlink(identical, doc).unwrap();
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(identical, doc).unwrap();
            }
            "generation" => edit_seal(&root, &row, case, control.generation.clone().into()),
            "source_hash" => edit_seal(&root, &row, case, control.hash.clone().into()),
            "recipe_id" => edit_seal(&root, &row, case, "foreign recipe".into()),
            _ => unreachable!(),
        }
        if case == "truncated" {
            let before = counters();
            assert!(matches!(
                check(
                    &root,
                    &row,
                    &active_index::recipe_id(&f.engines.recipe).unwrap()
                ),
                Err(Unavailable::Damaged)
            ));
            assert_eq!(counters(), before);
        }
        let report = f.scan().await;
        assert_eq!(
            (report.completed, report.skipped, report.failed),
            (1, 1, 0),
            "{case}"
        );
        assert_ne!(f.rows()[0].generation, row.generation, "{case}");
        assert_eq!(f.rows()[1], control, "{case}");
        assert!(!directory.exists(), "{case}");
        assert!(!path(&root, &row.generation).exists(), "{case}");
        f.assert_search(2, &[]).await;
    }
}

#[tokio::test]
async fn unknown_generation_retains_row_through_prune_and_publishes_unrelated_change() {
    let f = Fixture::new().await;
    let root = f.root();
    let rows = f.rows();
    let directory = root.join("generations").join(&rows[0].generation);
    let doc = directory.join("document.json");
    let seal = path(&root, &rows[0].generation);
    let before = counters();
    let access = deny_read(&doc);
    assert!(fs::File::open(&doc).is_err());
    assert_reused(f.scan().await);
    assert_eq!(counters(), before);
    f.assert_search(1, &[&rows[0].generation]).await;
    {
        let state = State::open(&root).unwrap();
        assert!(state.documents().is_err());
        assert!(State::review_documents(&root).is_err());
    }
    drop(access);
    f.assert_search(2, &[]).await;
    assert_eq!(f.rows(), rows);

    let lance = directory.join("lance");
    let saved = f.temp.path().join("saved-lance");
    fs::rename(&lance, &saved).unwrap();
    f.assert_search(1, &[&rows[0].generation]).await;
    fs::rename(saved, lance).unwrap();

    fs::remove_file(&seal).unwrap();
    let access = deny_read(&doc);
    fs::write(f.source().join("b.txt"), "The updated orbital archive.").unwrap();
    let stats = f.scan().await;
    assert_eq!(
        (
            stats.completed,
            stats.skipped,
            stats.failed,
            stats.infrastructure_failed
        ),
        (1, 0, 1, 0)
    );
    assert!(stats.failure_examples[0].contains("a.txt: generation unavailable"));
    assert_eq!(f.rows()[0], rows[0]);
    assert_ne!(f.rows()[1].generation, rows[1].generation);
    assert!(directory.is_dir());
    assert!(!seal.exists());
    f.assert_search(1, &[&rows[0].generation]).await;
    drop(access);
    f.assert_search(2, &[]).await;
    assert_eq!(f.rows()[0], rows[0]);
    assert_reused(f.scan().await);
    assert!(matches!(origin(&root, &rows[0]), Origin::Adopted));
}

#[tokio::test]
async fn unknown_survives_current_and_overlapping_failed_path_revocations() {
    for current in [true, false] {
        let f = Fixture::new().await;
        let root = f.root();
        let rows = f.rows();
        let retained = root.join("generations").join(&rows[0].generation);
        fs::remove_file(path(&root, &rows[0].generation)).unwrap();
        let access = deny_read(&retained.join("document.json"));
        fs::write(f.source().join("b.txt"), "").unwrap();
        fs::write(f.source().join("new.pdf"), "no parser asset").unwrap();
        let source = if current {
            f.source()
        } else {
            f.temp.path().to_owned()
        };
        let result = active_index::reindex(
            &f.library(),
            &source,
            &f.engines,
            &f.temp.path().join("no-assets"),
            |_| {},
            || false,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(f.rows(), rows[..1]);
        assert!(retained.is_dir());
        assert!(!path(&root, &rows[1].generation).exists());
        drop(access);
        f.assert_search(1, &[]).await;
    }
}

#[tokio::test]
async fn successful_damage_rebuild_is_kept_for_retry_after_unrelated_refusal() {
    let f = Fixture::new().await;
    let root = f.root();
    let rows = f.rows();
    let document = root
        .join("generations")
        .join(&rows[0].generation)
        .join("document.json");
    fs::remove_file(&document).unwrap();
    let failed = f.source().join("no-parser.pdf");
    fs::write(&failed, "no parser asset").unwrap();
    assert!(active_index::reindex(
        &f.library(),
        &f.source(),
        &f.engines,
        &f.temp.path().join("no-assets"),
        |_| {},
        || false,
    )
    .await
    .is_err());
    assert_eq!(f.rows(), rows);
    assert!(path(&root, &rows[0].generation).is_file());
    assert!(root.join(".vz-scan/scan.sqlite").is_file());
    fs::remove_file(failed).unwrap();
    assert_reused(f.scan().await);
    assert_ne!(f.rows()[0].generation, rows[0].generation);
    assert_eq!(f.rows()[1], rows[1]);
    f.assert_search(2, &[]).await;
}

#[tokio::test]
async fn unreadable_seal_or_file_at_time_refresh_reports_unknown() {
    let f = Fixture::new().await;
    let root = f.root();
    let rows = f.rows();
    let seal = path(&root, &rows[0].generation);
    let access = deny_read(&seal);
    let stats = f.scan().await;
    assert_eq!((stats.completed, stats.skipped, stats.failed), (0, 1, 1));
    assert_eq!(f.rows(), rows);
    drop(access);
    let doc = root
        .join("generations")
        .join(&rows[0].generation)
        .join("document.json");
    set_mtime(
        &doc,
        doc.metadata().unwrap().modified().unwrap() + Duration::from_secs(2),
    );
    let access = deny_read(&doc);
    let stats = f.scan().await;
    assert_eq!((stats.completed, stats.skipped, stats.failed), (0, 1, 1));
    assert_eq!(f.rows(), rows);
    drop(access);
    let (workers, hashes) = counters();
    assert_reused(f.scan().await);
    assert_eq!(counters(), (workers, hashes + 1));
}

#[cfg(unix)]
fn restrict_seal_directory(dir: &Path, readonly: bool) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        dir,
        fs::Permissions::from_mode(if readonly { 0o500 } else { 0o700 }),
    )
    .unwrap();
}

#[cfg(windows)]
fn restrict_seal_directory(dir: &Path, readonly: bool) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SetFileSecurityW,
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        },
    };
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let rights = if readonly { "FRFX" } else { "FA" };
    let sddl: Vec<u16> = format!("D:P(A;;{rights};;;WD)\0").encode_utf16().collect();
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
        let result = SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        );
        LocalFree(descriptor);
        assert_ne!(result, 0);
    }
}

#[tokio::test]
async fn read_only_seal_directory_reports_adoption_and_retries_time_refresh() {
    let f = Fixture::new().await;
    let root = f.root();
    let rows = f.rows();
    let directory = root.join(DIRECTORY);
    let adoption = path(&root, &rows[0].generation);
    let refresh = path(&root, &rows[1].generation);
    let original = fs::read(&refresh).unwrap();
    fs::remove_file(&adoption).unwrap();
    let document = root
        .join("generations")
        .join(&rows[1].generation)
        .join("document.json");
    set_mtime(
        &document,
        document.metadata().unwrap().modified().unwrap() + Duration::from_secs(2),
    );
    restrict_seal_directory(&directory, true);
    let (workers, hashes) = counters();
    for _ in 0..2 {
        let stats = f.scan().await;
        assert_eq!(stats.seal_warnings.len(), 2);
        assert_reused(stats);
        assert_eq!(fs::read(&refresh).unwrap(), original);
        assert!(!adoption.exists());
    }
    restrict_seal_directory(&directory, false);
    let retry = f.scan().await;
    assert!(retry.seal_warnings.is_empty());
    assert_reused(retry);
    assert!(matches!(origin(&root, &rows[0]), Origin::Adopted));
    assert!(matches!(origin(&root, &rows[1]), Origin::Validated));
    let (new_workers, new_hashes) = counters();
    assert_eq!(new_workers, workers);
    assert!(new_hashes > hashes);
    assert_reused(f.scan().await);
    assert_eq!(counters(), (new_workers, new_hashes));
    assert_eq!(f.rows(), rows);
}
