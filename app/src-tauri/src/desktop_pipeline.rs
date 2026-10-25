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

    fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        fn visit(path: &Path, out: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
            if !path.exists() {
                return;
            }
            if path.is_dir() {
                out.push((path.to_owned(), Vec::new()));
                for entry in std::fs::read_dir(path).unwrap() {
                    visit(&entry.unwrap().path(), out);
                }
            } else {
                // An already-open writer shares SQLite's SHM mapping with a
                // read-only connection; its read-lock markers are coordination,
                // not persisted data. Still assert that no SHM file is created.
                let bytes = if path
                    .file_name()
                    .is_some_and(|name| name == "state.sqlite-shm")
                {
                    Vec::new()
                } else {
                    std::fs::read(path).unwrap()
                };
                out.push((path.to_owned(), bytes));
            }
        }
        let mut out = Vec::new();
        visit(root, &mut out);
        out.sort();
        out
    }

    #[test]
    fn shared_search_and_ask_preflight_preserves_storage_and_original_errors() {
        for command in ["pipeline_search", "pipeline_ask"] {
            let temp = Temp::new();
            let root = temp.0.join("index");
            let check = |expected| {
                let before = snapshot(&root);
                assert_eq!(require_index(&root), expected, "{command}");
                assert_eq!(snapshot(&root), before, "{command} changed storage");
            };
            check(Err("Index a folder first.".into()));
            std::fs::create_dir(&root).unwrap();
            check(Err("Index a folder first.".into()));
            let state = State::open(&root).unwrap();
            check(Err("Index a folder first.".into()));
            drop(state);
            check(Err("Index a folder first.".into()));
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
            check(Ok(()));
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
            check(Ok(()));
            std::fs::write(
                root.join("recipe.json"),
                serde_json::to_vec(&recipe).unwrap(),
            )
            .unwrap();
            check(Ok(()));
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
        assert_eq!(require_index(&temp.0), Ok(()));
        assert_eq!(snapshot(&temp.0), before);
    }

    #[test]
    fn surviving_generation_without_database_is_not_a_new_index() {
        let temp = Temp::new();
        std::fs::create_dir_all(temp.0.join("generations/active")).unwrap();
        std::fs::write(temp.0.join("generations/active/document.json"), b"{}").unwrap();
        let before = snapshot(&temp.0);
        assert_eq!(require_index(&temp.0), Ok(()));
        assert_eq!(snapshot(&temp.0), before);
    }

    #[test]
    fn seeded_recipe_and_other_errors_are_left_to_core() {
        let temp = Temp::new();
        let state = State::open(&temp.0).unwrap();
        let recipe = state.root.join("recipe.json");
        std::fs::write(&recipe, b"invalid recipe").unwrap();
        // Existence, not parsing or error text, controls this display-only gate.
        assert_eq!(require_index(&state.root), Ok(()));
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
            assert_eq!(require_index(&state.root), Ok(()));
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
        let before = snapshot(&state.root);
        for command in ["pipeline_search", "pipeline_ask"] {
            assert_eq!(require_index(&state.root), Ok(()), "{command}");
            assert!(format!("{:#}", state.validate_recipe(&recipe).unwrap_err())
                .contains("index recipe missing; re-index required"));
            assert_eq!(snapshot(&state.root), before);
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
