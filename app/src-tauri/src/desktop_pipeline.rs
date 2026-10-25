//! Read-only presentation helpers for the desktop query commands.
use std::{collections::HashMap, path::Path};
use vz_core::storage::State;

pub(crate) fn require_index(root: &Path) -> Result<(), String> {
    require_index_with(root, |path| path.try_exists())
}

fn require_index_with(
    root: &Path,
    exists: impl FnOnce(&Path) -> std::io::Result<bool>,
) -> Result<(), String> {
    if matches!(exists(&root.join("recipe.json")), Ok(false))
        && matches!(State::has_persisted_documents(root), Ok(false))
    {
        return Err("Index a folder first.".into());
    }
    // Existing data, invalid recipes, and unknown filesystem/storage errors keep
    // the original pipeline diagnostic (including missing-recipe/re-index).
    Ok(())
}

pub(crate) fn file_names(state: &State) -> HashMap<String, String> {
    // The SQLite-selected generations are the visibility authority. A failed
    // optional name lookup must not replace a query result with a display error.
    state
        .documents()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|doc| {
            // Recognize both platforms' separators; never send a directory to UI.
            let name = doc.path.rsplit(['/', '\\']).next()?;
            if name.is_empty() {
                return None;
            }
            Some((doc.doc_id, name.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "vz-desktop-query-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum SnapshotContents {
        Bytes(Vec<u8>),
        LockedFileSize(u64),
    }

    fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, SnapshotContents)> {
        fn visit(path: &Path, out: &mut Vec<(std::path::PathBuf, SnapshotContents)>) {
            if !path.exists() {
                return;
            }
            if path.is_dir() {
                out.push((path.to_owned(), SnapshotContents::Bytes(Vec::new())));
                for entry in std::fs::read_dir(path).unwrap() {
                    visit(&entry.unwrap().path(), out);
                }
            } else {
                // Never byte-read SQLite files, even with the writer closed.
                // State also holds a Windows byte-range lock on writer.lock.
                // Metadata failures must fail the test, not compare None == None.
                let contents = if path.file_name().is_some_and(|name| {
                    name == "state.sqlite"
                        || name == "state.sqlite-wal"
                        || name == "state.sqlite-shm"
                        || name == "writer.lock"
                }) {
                    SnapshotContents::LockedFileSize(std::fs::metadata(path).unwrap().len())
                } else {
                    SnapshotContents::Bytes(std::fs::read(path).unwrap())
                };
                out.push((path.to_owned(), contents));
            }
        }
        let mut out = Vec::new();
        visit(root, &mut out);
        out.sort();
        out
    }

    type LogicalDatabase = Vec<(String, Vec<Vec<String>>)>;

    fn logical_snapshot(
        root: &Path,
        writer: Option<&State>,
    ) -> anyhow::Result<Option<LogicalDatabase>> {
        if !root.join("state.sqlite").try_exists()? {
            return Ok(None);
        }
        fn dump(state: &State, schema: &str) -> anyhow::Result<LogicalDatabase> {
            let mut tables = state.db.prepare(&format!(
                "SELECT name FROM {schema}.sqlite_schema WHERE type='table' ORDER BY name"
            ))?;
            let mut names = vec!["sqlite_schema".to_owned()];
            names.extend(
                tables
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?,
            );
            let mut result = Vec::new();
            for name in names {
                let quoted = name.replace('"', "\"\"");
                let mut stmt = state
                    .db
                    .prepare(&format!("SELECT * FROM {schema}.\"{quoted}\""))?;
                let columns = stmt.column_count();
                let mut rows = stmt
                    .query_map([], |row| {
                        // ValueRef's Debug preserves SQLite types and every byte of
                        // text/blob values, including NULL and embedded separators.
                        (0..columns)
                            .map(|i| row.get_ref(i).map(|v| format!("{v:?}")))
                            .collect::<Result<Vec<_>, _>>()
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                rows.sort();
                result.push((name, rows));
            }
            Ok(result)
        }
        if let Some(state) = writer {
            return dump(state, "main").map(Some);
        }
        // Obtain an observer connection without opening/recovering the target
        // through State::open. ATTACH honors URI read-only flags; DETACH closes
        // the target before returning, so the snapshot holds no target locks.
        let observer_root = Temp::new();
        let observer = State::open(&observer_root.0)?;
        let path = root.join("state.sqlite");
        let path = path
            .to_str()
            .unwrap()
            .replace('%', "%25")
            .replace('?', "%3F")
            .replace('#', "%23");
        let mode = if root.join("state.sqlite-wal").try_exists()? {
            "readonly_shm=1"
        } else {
            "immutable=1"
        };
        observer.db.execute(
            "ATTACH DATABASE ?1 AS observed",
            [format!("file:{path}?mode=ro&{mode}")],
        )?;
        let result = dump(&observer, "observed");
        observer.db.execute_batch("DETACH DATABASE observed")?;
        result.map(Some)
    }

    fn check_preflight(root: &Path, writer: Option<&State>, expected: Result<(), String>) {
        assert_storage_unchanged(root, writer, || assert_eq!(require_index(root), expected));
    }

    fn assert_storage_unchanged(root: &Path, writer: Option<&State>, action: impl FnOnce()) {
        let files = snapshot(root);
        let database = logical_snapshot(root, writer).unwrap();
        action();
        assert_eq!(
            logical_snapshot(root, writer).unwrap(),
            database,
            "preflight changed database contents"
        );
        assert_eq!(snapshot(root), files, "preflight changed files");
    }

    #[test]
    #[should_panic(expected = "preflight changed database contents")]
    fn logical_comparison_rejects_same_size_persisted_value_change() {
        let temp = Temp::new();
        let state = State::open(&temp.0).unwrap();
        state
            .db
            .execute("INSERT INTO jobs VALUES (1, '/doc', 'ready', NULL)", [])
            .unwrap();
        // Seed one WAL frame for this page, then update it again so the WAL
        // frame count, as well as the main database size, remains unchanged.
        state
            .db
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        state
            .db
            .execute("UPDATE jobs SET status='other' WHERE id=1", [])
            .unwrap();
        state
            .db
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE)")
            .unwrap();
        let files = snapshot(&temp.0);
        assert_storage_unchanged(&temp.0, Some(&state), || {
            state
                .db
                .execute("UPDATE jobs SET status='ready' WHERE id=1", [])
                .unwrap();
            assert_eq!(
                snapshot(&temp.0),
                files,
                "negative control must preserve file sizes"
            );
        });
    }

    #[test]
    fn preflight_releases_connections_on_empty_populated_and_query_error_paths() {
        for contents in ["empty", "populated", "query error"] {
            let temp = Temp::new();
            let state = State::open(&temp.0).unwrap();
            let expected = match contents {
                "empty" => Err("Index a folder first.".into()),
                "populated" => {
                    state.db.execute("INSERT INTO documents VALUES ('doc','/doc',1,'mtime','hash','generation')", []).unwrap();
                    Ok(())
                }
                _ => {
                    state.db.execute_batch("DROP TABLE documents").unwrap();
                    Ok(())
                }
            };
            let observer_root = Temp::new();
            let observer = State::open(&observer_root.0).unwrap();
            observer.db.busy_timeout(std::time::Duration::ZERO).unwrap();
            observer
                .db
                .execute(
                    "ATTACH DATABASE ?1 AS probe",
                    [temp.0.join("state.sqlite").to_str().unwrap()],
                )
                .unwrap();
            observer
                .db
                .execute_batch("PRAGMA probe.locking_mode=EXCLUSIVE")
                .unwrap();
            let exclusive = || {
                observer.db.execute_batch(
                    "BEGIN EXCLUSIVE; SELECT count(*) FROM probe.sqlite_schema; ROLLBACK;",
                )
            };
            // Negative control: a live WAL connection really blocks this probe
            // on every platform, even when it has no outstanding statement.
            assert!(
                exclusive().is_err(),
                "live State did not block exclusive access"
            );
            // A failed exclusive attempt can itself obstruct a read-only open
            // on Windows. Release the control probe before calling the gate.
            observer.db.execute_batch("DETACH DATABASE probe").unwrap();
            #[cfg(windows)]
            assert_eq!(
                std::fs::read(temp.0.join("writer.lock"))
                    .unwrap_err()
                    .raw_os_error(),
                Some(33)
            );
            check_preflight(&temp.0, Some(&state), expected.clone());
            drop(state);
            #[cfg(windows)]
            assert!(std::fs::read(temp.0.join("writer.lock"))
                .unwrap()
                .is_empty());
            observer
                .db
                .execute(
                    "ATTACH DATABASE ?1 AS probe",
                    [temp.0.join("state.sqlite").to_str().unwrap()],
                )
                .unwrap();
            observer
                .db
                .execute_batch("PRAGMA probe.locking_mode=EXCLUSIVE")
                .unwrap();
            exclusive().expect("preflight leaked a connection after State was dropped");
            observer.db.execute_batch("DETACH DATABASE probe").unwrap();

            check_preflight(&temp.0, None, expected);
            observer
                .db
                .execute(
                    "ATTACH DATABASE ?1 AS probe",
                    [temp.0.join("state.sqlite").to_str().unwrap()],
                )
                .unwrap();
            observer
                .db
                .execute_batch("PRAGMA probe.locking_mode=EXCLUSIVE")
                .unwrap();
            exclusive().expect("closed-writer preflight leaked a connection");
            observer.db.execute_batch("DETACH DATABASE probe").unwrap();
        }
    }

    #[test]
    fn shared_search_and_ask_preflight_preserves_storage_and_original_errors() {
        for _command in ["pipeline_search", "pipeline_ask"] {
            let temp = Temp::new();
            let root = temp.0.join("index");
            check_preflight(&root, None, Err("Index a folder first.".into()));
            std::fs::create_dir(&root).unwrap();
            check_preflight(&root, None, Err("Index a folder first.".into()));
            let state = State::open(&root).unwrap();
            check_preflight(&root, Some(&state), Err("Index a folder first.".into()));
            drop(state);
            check_preflight(&root, None, Err("Index a folder first.".into()));
            let state = State::open(&root).unwrap();
            // SQLite document rows remain authoritative even if generation files
            // and the recipe have been lost. Preserve the core recipe diagnostic.
            state
                .db
                .execute(
                    "INSERT INTO documents VALUES ('doc','/doc',1,'mtime','hash','generation')",
                    [],
                )
                .unwrap();
            check_preflight(&root, Some(&state), Ok(()));
            let recipe: vz_core::engine::Recipe = serde_json::from_value(serde_json::json!({
                "model":"test", "sha256":"hash", "dimensions":4,
                "dtype":"float32", "pooling":"cls", "normalization":"l2",
                "tokenizer":"embedded", "max_tokens":512, "document_prefix":"",
                "query_prefix":"", "chunker":{"version":"vz-1"},
                "llama_commit":vz_core::engine::LLAMA_COMMIT
            }))
            .unwrap();
            let original = state.validate_recipe(&recipe).unwrap_err();
            assert!(format!("{original:#}").contains("index recipe missing; re-index required"));
            // Checkpointed data must also be read without recreating WAL/SHM.
            drop(state);
            check_preflight(&root, None, Ok(()));
            std::fs::write(
                root.join("recipe.json"),
                serde_json::to_vec(&recipe).unwrap(),
            )
            .unwrap();
            check_preflight(&root, None, Ok(()));
            let state = State::open(&root).unwrap();
            assert!(state.validate_recipe(&recipe).is_ok());
            assert!(state.documents().is_err());
        }
    }

    #[test]
    fn filesystem_errors_pass_through_without_creating_state() {
        let temp = Temp::new();
        let root = temp.0.join("absent");
        let before = snapshot(&temp.0);
        assert_eq!(
            require_index_with(&root, |_| Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected denial"
            ))),
            Ok(())
        );
        assert_eq!(snapshot(&temp.0), before);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn non_directory_root_matches_platform_filesystem_behavior() {
        let temp = Temp::new();
        let root = temp.0.join("file");
        std::fs::write(&root, b"not a directory").unwrap();
        let before = snapshot(&temp.0);
        let exists = root.join("recipe.json").try_exists();
        #[cfg(unix)]
        {
            assert_eq!(exists.unwrap_err().raw_os_error(), Some(20)); // ENOTDIR
            assert_eq!(require_index(&root), Ok(()));
        }
        #[cfg(windows)]
        {
            assert!(!exists.unwrap()); // Windows maps this to NotFound.
            assert_eq!(require_index(&root), Err("Index a folder first.".into()));
        }
        assert_eq!(snapshot(&temp.0), before);
    }

    #[test]
    fn incomplete_wal_state_is_not_initialized_by_preflight() {
        let temp = Temp::new();
        drop(State::open(&temp.0).unwrap());
        std::fs::write(temp.0.join("state.sqlite-wal"), b"damaged WAL").unwrap();
        assert!(!temp.0.join("state.sqlite-shm").exists());
        let before = snapshot(&temp.0);
        assert!(logical_snapshot(&temp.0, None).is_err());
        assert_eq!(require_index(&temp.0), Ok(()));
        assert!(logical_snapshot(&temp.0, None).is_err());
        assert_eq!(snapshot(&temp.0), before);
    }

    #[test]
    fn surviving_generation_without_database_is_not_a_new_index() {
        let temp = Temp::new();
        std::fs::create_dir_all(temp.0.join("generations/active")).unwrap();
        std::fs::write(temp.0.join("generations/active/document.json"), b"{}").unwrap();
        let before = snapshot(&temp.0);
        check_preflight(&temp.0, None, Ok(()));
        assert_eq!(snapshot(&temp.0), before);
    }

    #[test]
    fn seeded_recipe_and_other_errors_are_left_to_core() {
        let temp = Temp::new();
        let state = State::open(&temp.0).unwrap();
        let recipe = state.root.join("recipe.json");
        std::fs::write(&recipe, b"invalid recipe").unwrap();
        // Existence, not parsing or error text, controls this display-only gate.
        check_preflight(&state.root, Some(&state), Ok(()));
        assert_eq!(std::fs::read(recipe).unwrap(), b"invalid recipe");
    }

    #[tokio::test]
    async fn seeded_index_search_is_unchanged_by_query_gate() {
        let temp = Temp::new();
        let mut state = State::open(&temp.0).unwrap();
        let recipe: vz_core::engine::Recipe = serde_json::from_value(serde_json::json!({
            "model":"test", "sha256":"hash", "dimensions":4,
            "dtype":"float32", "pooling":"cls", "normalization":"l2",
            "tokenizer":"embedded", "max_tokens":512, "document_prefix":"",
            "query_prefix":"query: ", "chunker":{"version":"vz-1"},
            "llama_commit":vz_core::engine::LLAMA_COMMIT
        }))
        .unwrap();
        let recipe_bytes = serde_json::to_vec(&recipe).unwrap();
        std::fs::write(state.root.join("recipe.json"), &recipe_bytes).unwrap();
        let doc: vz_core::pipeline::Document = serde_json::from_value(serde_json::json!({
            "doc_id":"seed", "path":"/docs/lighthouse.txt", "sha256":"hash", "bytes":10,
            "pages":1, "kind":"txt", "chunks":[{
                "chunk_id":"chunk", "doc_id":"seed", "text":"lighthouse cats",
                "page":1, "heading_path":[], "token_count":2, "embedding":[1.0,0.0,0.0,0.0]
            }]
        }))
        .unwrap();
        let generation = state.stage(&doc).await.unwrap();
        state.commit(&doc, "mtime", &generation).unwrap();
        let before = state
            .search("lighthouse", &[1.0, 0.0, 0.0, 0.0], 3)
            .await
            .unwrap();
        assert_eq!(before.len(), 1);
        for _ in 0..2 {
            // Search and Ask use the same gate and core retrieval.
            check_preflight(&state.root, Some(&state), Ok(()));
            let after = state
                .search("lighthouse", &[1.0, 0.0, 0.0, 0.0], 3)
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&after).unwrap(),
                serde_json::to_value(&before).unwrap()
            );
        }
        assert_eq!(
            std::fs::read(state.root.join("recipe.json")).unwrap(),
            recipe_bytes
        );
        assert_eq!(state.generations().unwrap(), vec![generation]);
        std::fs::remove_file(state.root.join("recipe.json")).unwrap();
        // Also exercise the gate after the writer has checkpointed and closed.
        drop(state);
        for _command in ["pipeline_search", "pipeline_ask"] {
            let before = snapshot(&temp.0);
            check_preflight(&temp.0, None, Ok(()));
            assert_eq!(snapshot(&temp.0), before);
            let state = State::open(&temp.0).unwrap();
            assert!(format!("{:#}", state.validate_recipe(&recipe).unwrap_err())
                .contains("index recipe missing; re-index required"));
            drop(state);
        }
    }

    #[test]
    fn resolves_only_active_documents_to_basenames_and_missing_ids_fall_back() {
        let temp = Temp::new();
        let mut state = State::open(&temp.0).unwrap();
        for (id, path) in [
            ("unix", "/private/docs/lighthouse.txt"),
            ("windows", "C:\\private\\docs\\budget.txt"),
        ] {
            let doc: vz_core::pipeline::Document = serde_json::from_value(serde_json::json!({
                "doc_id": id, "path": path, "sha256": "hash", "bytes": 1,
                "pages": 1, "kind": "txt", "chunks": []
            }))
            .unwrap();
            let generation = format!("active-{id}");
            let dir = state.root.join("generations").join(&generation);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("document.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
            state.commit(&doc, "mtime", &generation).unwrap();
        }
        let inactive = state.root.join("generations/inactive");
        std::fs::create_dir(&inactive).unwrap();
        std::fs::write(inactive.join("document.json"), br#"{"doc_id":"inactive","path":"/private/hidden.txt","sha256":"hash","bytes":1,"pages":1,"kind":"txt","chunks":[]}"#).unwrap();
        let names = file_names(&state);
        assert!(!names.contains_key("inactive"));
        assert_eq!(
            names.get("unix").map(String::as_str),
            Some("lighthouse.txt")
        );
        assert_eq!(names.get("windows").map(String::as_str), Some("budget.txt"));
        // No name serializes as null; the UI retains its Document <id> fallback.
        assert_eq!(
            serde_json::json!({"file_name": names.get("missing")})["file_name"],
            serde_json::Value::Null
        );
        std::fs::remove_file(state.root.join("generations/active-unix/document.json")).unwrap();
        assert!(file_names(&state).is_empty());
    }
}
