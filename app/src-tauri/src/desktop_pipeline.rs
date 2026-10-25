//! Read-only presentation helpers for the desktop query commands.
use std::{collections::HashMap, path::Path};
use vz_core::storage::State;

pub(crate) fn require_index(root: &Path) -> Result<(), String> {
    if matches!(root.join("recipe.json").try_exists(), Ok(false)) {
        return Err("Index a folder first.".into());
    }
    // Permission failures, invalid recipes, and other faults retain the original
    // pipeline error path. Only a known absent recipe gets the friendly message.
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

    #[test]
    fn queries_without_index_are_friendly_and_do_not_create_state() {
        let temp = Temp::new();
        let root = temp.0.join("not-created");
        // Both pipeline_search and pipeline_ask use this gate before loading engines.
        assert_eq!(require_index(&root), Err("Index a folder first.".into()));
        assert!(!root.exists());
        let state = State::open(&temp.0).unwrap();
        assert_eq!(
            require_index(&state.root),
            Err("Index a folder first.".into())
        );
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
        assert_eq!(require_index(&state.root.join("recipe.json")), Ok(()));
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
