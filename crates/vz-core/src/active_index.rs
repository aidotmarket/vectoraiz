//! Library selection only; per-document storage and recovery remain unchanged.
use crate::{engine::Recipe, pipeline::sha256};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Pointer {
    dir: String,
    recipe_id: String,
}

/// Canonical JSON: recursively sorted object keys, compact UTF-8 serialization.
/// Sort explicitly so dependency feature unification cannot change the ID.
pub fn recipe_id(recipe: &Recipe) -> Result<String> {
    canonical_id(serde_json::to_value(recipe)?)
}

fn canonical_id(value: serde_json::Value) -> Result<String> {
    fn sorted(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let entries: std::collections::BTreeMap<_, _> = map.into_iter().collect();
                serde_json::Value::Object(
                    entries.into_iter().map(|(k, v)| (k, sorted(v))).collect(),
                )
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(sorted).collect())
            }
            other => other,
        }
    }
    Ok(sha256(&serde_json::to_vec(&sorted(value))?)[..16].into())
}

pub fn recipe_dir(data: &Path, recipe: &Recipe) -> Result<PathBuf> {
    Ok(data.join("indexes").join(recipe_id(recipe)?))
}

/// Invalid, missing, escaping (including symlink escapes) pointers fall back to
/// the untouched legacy index. A missing legacy path is returned for first use.
pub fn resolve(data: &Path) -> PathBuf {
    verified_target(data).unwrap_or_else(|| data.join("phase0-index"))
}

/// An existing invalid pointer needs repair even if the legacy recipe matches.
pub fn needs_pointer_repair(data: &Path) -> bool {
    data.join("active-index.json").symlink_metadata().is_ok() && verified_target(data).is_none()
}

fn verified_target(data: &Path) -> Option<PathBuf> {
    (|| -> Option<PathBuf> {
        let pointer: Pointer =
            serde_json::from_slice(&fs::read(data.join("active-index.json")).ok()?).ok()?;
        let relative = Path::new(&pointer.dir);
        if pointer.recipe_id.len() != 16
            || !pointer
                .recipe_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return None;
        }
        let root = data.canonicalize().ok()?;
        let target = root.join(relative).canonicalize().ok()?;
        if !target.starts_with(&root) || target == root || !target.is_dir() {
            return None;
        }
        if pointer.dir != format!("indexes/{}", pointer.recipe_id) && pointer.dir != "phase0-index"
        {
            return None;
        }
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join("recipe.json")).ok()?).ok()?;
        // Validate the schema, but hash the complete stored JSON so deserialization
        // cannot silently discard unknown fields when verifying pointer identity.
        serde_json::from_value::<Recipe>(stored.clone()).ok()?;
        if canonical_id(stored).ok()? != pointer.recipe_id {
            return None;
        }
        Some(target)
    })()
}

/// Recognize libraries including an interrupted first rebuild with no pointer yet.
pub fn is_library_root(path: &Path) -> bool {
    path.join("active-index.json").symlink_metadata().is_ok()
        || path.join("phase0-index").is_dir()
        || path.join("indexes").is_dir()
}

/// Accept an explicit index path (CLI compatibility) or a library root.
/// Query, Ask, pack and future MCP readers share this resolver.
pub fn resolve_state(path: &Path) -> PathBuf {
    if is_library_root(path) {
        resolve(path)
    } else {
        path.to_owned()
    }
}

/// Reuse a matching legacy library in place; never rewrite an incompatible one.
fn publication_dir(data: &Path, recipe: &Recipe) -> Result<String> {
    let legacy = data.join("phase0-index").join("recipe.json");
    if fs::read(legacy)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .as_ref()
        == Some(&serde_json::to_value(recipe)?)
    {
        Ok("phase0-index".into())
    } else {
        Ok(format!("indexes/{}", recipe_id(recipe)?))
    }
}

/// Call only after the indexer returned successfully and cancellation was checked.
pub fn activate(data: &Path, recipe: &Recipe) -> Result<()> {
    activate_with(data, recipe, || Ok(()))
}

fn activate_with(
    data: &Path,
    recipe: &Recipe,
    before_rename: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let id = recipe_id(recipe)?;
    let relative = publication_dir(data, recipe)?;
    let root = data.canonicalize()?;
    let target = root.join(&relative).canonicalize()?;
    ensure!(
        target.starts_with(&root) && target.is_dir(),
        "index outside data root"
    );
    let value: serde_json::Value = serde_json::from_slice(&fs::read(target.join("recipe.json"))?)?;
    let stored: Recipe = serde_json::from_value(value.clone())?;
    ensure!(
        stored == *recipe && canonical_id(value)? == id,
        "index recipe mismatch"
    );
    let pointer = Pointer {
        dir: relative,
        recipe_id: id,
    };
    let temp = data.join(format!(".active-index-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec(&pointer)?)?;
        file.sync_all()?;
        drop(file);
        before_rename()?;
        fs::rename(&temp, data.join("active-index.json"))?;
        #[cfg(unix)]
        fs::File::open(data)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Read the old document paths without acquiring a writer or running recovery.
pub fn last_indexed_folder(root: &Path) -> Option<String> {
    let db_path = root.join("state.sqlite").canonicalize().ok()?;
    let mode = if root.join("state.sqlite-wal").exists() {
        "readonly_shm=1"
    } else {
        "immutable=1"
    };
    let escaped = db_path
        .to_str()?
        .replace('%', "%25")
        .replace('?', "%3F")
        .replace('#', "%23");
    let db = rusqlite::Connection::open_with_flags(
        format!("file:{escaped}?{mode}"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let mut query = db
        .prepare("SELECT path FROM documents ORDER BY path")
        .ok()?;
    let paths = query.query_map([], |r| r.get::<_, String>(0)).ok()?;
    // Safe for the current one-folder library. Revisit before supporting
    // multiple source folders: their common ancestor could be a volume root.
    let mut folder: Option<PathBuf> = None;
    for path in paths {
        let path = PathBuf::from(path.ok()?);
        let parent = path.parent()?;
        folder = Some(match folder {
            None => parent.to_owned(),
            Some(mut current) => {
                while !parent.starts_with(&current) {
                    if !current.pop() {
                        return None;
                    }
                }
                current
            }
        });
    }
    folder.map(|p| p.to_string_lossy().into_owned())
}

/// Rebuild separately, resume via the existing indexer, then publish on success.
/// Poll cancellation at async boundaries and once more before publishing. Work
/// already committed per document remains resumable after dropping this future.
pub async fn reindex(
    data: &Path,
    folder: &Path,
    engines: &crate::engine::Engines,
    assets: &Path,
    progress: impl FnMut(&crate::index::Progress),
    cancelled: impl Fn() -> bool,
) -> Result<crate::index::Progress> {
    let mut latest = crate::index::Progress::default();
    let mut progress = progress;
    let result = async {
        ensure!(!cancelled(), "Indexing cancelled; Re-index to resume");
        let root = data.join(publication_dir(data, &engines.recipe)?);
        fs::create_dir_all(&root)?;
        ensure!(
            root.canonicalize()?.starts_with(data.canonicalize()?),
            "index outside data root"
        );
        let mut state = crate::storage::State::open(&root)?;
        finish_reindex(
            data,
            &engines.recipe,
            crate::index::index(&mut state, folder, engines, assets, |p| {
                latest = p.clone();
                progress(p);
            }),
            cancelled,
        )
        .await
    }
    .await;
    result.map_err(|e| {
        anyhow::anyhow!(
            "Re-index did not finish: {} indexed, {} skipped, {} failed, {} total; {e:#}",
            latest.completed,
            latest.skipped,
            latest.failed,
            latest.total
        )
    })
}

async fn finish_reindex(
    data: &Path,
    recipe: &Recipe,
    run: impl std::future::Future<Output = Result<crate::index::Progress>>,
    cancelled: impl Fn() -> bool,
) -> Result<crate::index::Progress> {
    let mut run = Box::pin(run);
    let result = futures::future::poll_fn(|cx| {
        if cancelled() {
            std::task::Poll::Ready(Err(anyhow::anyhow!(
                "Indexing cancelled; Re-index to resume"
            )))
        } else {
            run.as_mut().poll(cx)
        }
    })
    .await?;
    ensure!(!cancelled(), "Indexing cancelled; Re-index to resume");
    result.ensure_publishable()?;
    activate(data, recipe)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Recipe {
        let pins =
            serde_json::from_str(include_str!("../../../spike/fixtures/models.json")).unwrap();
        Recipe::from_pins(&pins, 384).unwrap()
    }
    fn seed(data: &Path, recipe: &Recipe) -> PathBuf {
        let dir = recipe_dir(data, recipe).unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("recipe.json"), serde_json::to_vec(recipe).unwrap()).unwrap();
        dir
    }

    #[test]
    fn canonical_recipe_id_ignores_object_order() {
        let recipe = recipe();
        let mut reordered = recipe.clone();
        reordered.chunker = serde_json::from_str(r#"{"z":1,"a":{"y":2,"b":3}}"#).unwrap();
        let mut same = reordered.clone();
        same.chunker = serde_json::from_str(r#"{"a":{"b":3,"y":2},"z":1}"#).unwrap();
        assert_eq!(recipe_id(&reordered).unwrap(), recipe_id(&same).unwrap());
        assert_eq!(recipe_id(&recipe).unwrap().len(), 16);
        assert!(recipe_id(&recipe)
            .unwrap()
            .bytes()
            .all(|c| c.is_ascii_hexdigit()));
        let mut changed = recipe.clone();
        changed.query_prefix.push('x');
        assert_ne!(recipe_id(&recipe).unwrap(), recipe_id(&changed).unwrap());
    }

    #[test]
    fn atomic_pointer_failure_keeps_previous_complete_file() {
        let data = tempfile::tempdir().unwrap();
        let old = recipe();
        seed(data.path(), &old);
        activate(data.path(), &old).unwrap();
        let before = fs::read(data.path().join("active-index.json")).unwrap();
        let mut new = old.clone();
        new.model.push_str("-new");
        seed(data.path(), &new);
        let error = activate_with(data.path(), &new, || {
            // At the last boundary before rename readers still see the old file.
            assert_eq!(fs::read(data.path().join("active-index.json"))?, before);
            anyhow::bail!("simulated rename failure")
        });
        assert!(error.is_err());
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            before
        );
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 2);
        fs::remove_file(data.path().join("active-index.json")).unwrap();
        assert!(activate_with(data.path(), &new, || anyhow::bail!("failed first write")).is_err());
        assert!(!data.path().join("active-index.json").exists());
        activate(data.path(), &new).unwrap();
        assert_eq!(
            resolve(data.path()),
            recipe_dir(data.path(), &new)
                .unwrap()
                .canonicalize()
                .unwrap()
        );
    }

    #[test]
    fn unsafe_missing_and_invalid_pointers_fall_back() {
        let data = tempfile::tempdir().unwrap();
        let legacy = data.path().join("phase0-index");
        fs::create_dir(&legacy).unwrap();
        assert_eq!(resolve(data.path()), legacy);
        for dir in ["../outside", "/tmp", "indexes/missing", ".", ""] {
            fs::write(
                data.path().join("active-index.json"),
                serde_json::to_vec(&Pointer {
                    dir: dir.into(),
                    recipe_id: recipe_id(&recipe()).unwrap(),
                })
                .unwrap(),
            )
            .unwrap();
            assert_eq!(resolve(data.path()), legacy);
        }
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), data.path().join("escape")).unwrap();
            fs::write(
                data.path().join("active-index.json"),
                serde_json::to_vec(&Pointer {
                    dir: "escape".into(),
                    recipe_id: recipe_id(&recipe()).unwrap(),
                })
                .unwrap(),
            )
            .unwrap();
            assert_eq!(resolve(data.path()), legacy);
        }
        fs::write(data.path().join("active-index.json"), b"{partial").unwrap();
        assert_eq!(resolve(data.path()), legacy);
    }

    #[test]
    fn pointer_identity_and_activation_recipe_are_verified() {
        let data = tempfile::tempdir().unwrap();
        let recipe = recipe();
        let root = seed(data.path(), &recipe);
        let legacy = data.path().join("phase0-index");
        let mut different = recipe.clone();
        different.model.push_str("-different");
        for (dir, id) in [
            (
                format!("indexes/{}", recipe_id(&recipe).unwrap()),
                "0123456789abcdef".to_string(),
            ),
            (
                format!("indexes/{}", recipe_id(&different).unwrap()),
                recipe_id(&recipe).unwrap(),
            ),
        ] {
            fs::create_dir_all(data.path().join(&dir)).unwrap();
            fs::write(
                data.path().join(&dir).join("recipe.json"),
                serde_json::to_vec(&recipe).unwrap(),
            )
            .unwrap();
            fs::write(
                data.path().join("active-index.json"),
                serde_json::to_vec(&Pointer { dir, recipe_id: id }).unwrap(),
            )
            .unwrap();
            assert_eq!(resolve(data.path()), legacy);
            assert!(needs_pointer_repair(data.path()));
        }
        fs::remove_file(data.path().join("active-index.json")).unwrap();
        fs::write(
            root.join("recipe.json"),
            serde_json::to_vec(&different).unwrap(),
        )
        .unwrap();
        assert!(activate(data.path(), &recipe).is_err());
        assert!(!data.path().join("active-index.json").exists());
        let mut with_unknown = serde_json::to_value(&recipe).unwrap();
        with_unknown["unknown_recipe_field"] = serde_json::json!("must participate in the hash");
        fs::write(
            root.join("recipe.json"),
            serde_json::to_vec(&with_unknown).unwrap(),
        )
        .unwrap();
        assert!(activate(data.path(), &recipe).is_err());
        fs::write(
            data.path().join("active-index.json"),
            serde_json::to_vec(&Pointer {
                dir: format!("indexes/{}", recipe_id(&recipe).unwrap()),
                recipe_id: recipe_id(&recipe).unwrap(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(needs_pointer_repair(data.path()));
        assert_eq!(resolve(data.path()), legacy);
        fs::write(
            root.join("recipe.json"),
            serde_json::to_vec(&different).unwrap(),
        )
        .unwrap();
        // Correct-shaped dir/id with a different stored recipe also fails.
        fs::write(
            data.path().join("active-index.json"),
            serde_json::to_vec(&Pointer {
                dir: format!("indexes/{}", recipe_id(&recipe).unwrap()),
                recipe_id: recipe_id(&recipe).unwrap(),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(resolve(data.path()), legacy);
    }

    #[test]
    fn matching_legacy_is_adopted_in_place_and_verified() {
        let data = tempfile::tempdir().unwrap();
        let recipe = recipe();
        let legacy = data.path().join("phase0-index");
        fs::create_dir(&legacy).unwrap();
        let bytes = serde_json::to_vec(&recipe).unwrap();
        fs::write(legacy.join("recipe.json"), &bytes).unwrap();
        assert_eq!(
            publication_dir(data.path(), &recipe).unwrap(),
            "phase0-index"
        );
        activate(data.path(), &recipe).unwrap();
        assert_eq!(resolve(data.path()), legacy.canonicalize().unwrap());
        assert!(!data.path().join("indexes").exists());
        assert_eq!(fs::read(legacy.join("recipe.json")).unwrap(), bytes);
        let mut wrong = recipe.clone();
        wrong.model.push('x');
        fs::write(
            legacy.join("recipe.json"),
            serde_json::to_vec(&wrong).unwrap(),
        )
        .unwrap();
        // A legacy pointer is accepted only when its own recipe hashes to its ID.
        let pointer: Pointer =
            serde_json::from_slice(&fs::read(data.path().join("active-index.json")).unwrap())
                .unwrap();
        assert_ne!(recipe_id(&wrong).unwrap(), pointer.recipe_id);
        assert!(needs_pointer_repair(data.path()));
    }

    #[tokio::test]
    async fn publication_requires_success_and_only_content_failures_then_resumes() {
        use crate::index::Progress;
        let data = tempfile::tempdir().unwrap();
        let old = recipe();
        seed(data.path(), &old);
        activate(data.path(), &old).unwrap();
        let before = fs::read(data.path().join("active-index.json")).unwrap();
        let mut new = old.clone();
        new.model.push_str("-new");
        let candidate = seed(data.path(), &new);
        for stats in [
            Progress {
                total: 2,
                failed: 2,
                ..Default::default()
            },
            Progress {
                total: 3,
                completed: 1,
                failed: 2,
                infrastructure_failed: 1,
                ..Default::default()
            },
        ] {
            let error = finish_reindex(data.path(), &new, async { Ok(stats) }, || false)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("Re-index did not finish:"));
            assert!(error.to_string().contains("failed"));
            assert_eq!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                before
            );
            assert!(candidate.join("recipe.json").is_file());
        }
        // Resumed work may skip already committed documents and exclude corrupt ones.
        let stats = Progress {
            total: 3,
            skipped: 1,
            completed: 1,
            failed: 1,
            ..Default::default()
        };
        finish_reindex(data.path(), &new, async { Ok(stats) }, || false)
            .await
            .unwrap();
        assert_eq!(resolve(data.path()), candidate.canonicalize().unwrap());
        // A genuinely empty folder is publishable, too.
        finish_reindex(data.path(), &new, async { Ok(Progress::default()) }, || {
            false
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn interrupted_reindex_resumes_then_switches_and_searches() {
        use crate::{
            pipeline::{chunk, Document, Page},
            storage::State,
            Store,
        };
        use std::sync::atomic::{AtomicBool, Ordering};
        let data = tempfile::tempdir().unwrap();
        let legacy = data.path().join("phase0-index");
        fs::create_dir(&legacy).unwrap();
        let mut new = recipe();
        new.dimensions = 4;
        let mut old = new.clone();
        old.model = "old-model".into();
        let old_bytes = serde_json::to_vec(&old).unwrap();
        fs::write(legacy.join("recipe.json"), &old_bytes).unwrap();
        assert_eq!(resolve(data.path()), legacy);
        let root = seed(data.path(), &new);
        let source = data.path().join("source");
        fs::create_dir(&source).unwrap();
        let mut chunks = chunk(
            "doc",
            &[Page {
                page: 1,
                text: "Orbital archive retention is thirty days".into(),
                needs_ocr: false,
            }],
        );
        chunks[0].embedding = vec![1., 0., 0., 0.];
        let doc = Document {
            doc_id: "doc".into(),
            path: source
                .canonicalize()
                .unwrap()
                .join("one.md")
                .to_string_lossy()
                .into_owned(),
            sha256: "hash".into(),
            bytes: 40,
            pages: 1,
            kind: "md".into(),
            needs_ocr_pages: vec![],
            chunks,
        };
        let cancelled = AtomicBool::new(false);
        // Simulate interruption after the indexer's per-document commit, before
        // its future completes. Exercise the production publication boundary.
        let result = finish_reindex(
            data.path(),
            &new,
            async {
                let mut state = State::open(&root)?;
                let generation = state.stage(&doc).await?;
                state
                    .put(&doc, "mtime", &generation)
                    .map_err(anyhow::Error::msg)?;
                cancelled.store(true, Ordering::SeqCst);
                Ok(crate::index::Progress {
                    completed: 1,
                    total: 1,
                    ..Default::default()
                })
            },
            || cancelled.load(Ordering::SeqCst),
        )
        .await;
        assert!(result.is_err());
        assert!(!data.path().join("active-index.json").exists());
        assert_eq!(resolve(data.path()), legacy);
        assert_eq!(fs::read(legacy.join("recipe.json")).unwrap(), old_bytes);
        // Resuming sees the previously committed document; no rewrite needed.
        finish_reindex(
            data.path(),
            &new,
            async {
                let state = State::open(&root)?;
                assert_eq!(state.documents()?.len(), 1);
                Ok(crate::index::Progress {
                    skipped: 1,
                    total: 1,
                    ..Default::default()
                })
            },
            || false,
        )
        .await
        .unwrap();
        assert_eq!(
            last_indexed_folder(&resolve(data.path())),
            Some(
                source
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );
        let (state, stored) = State::open_index(data.path(), &new).unwrap();
        assert_eq!(stored, new);
        let hits = state
            .search("orbital archive retention", &[1., 0., 0., 0.], 5)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        drop(state);
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        assert!(finish_reindex(
            data.path(),
            &new,
            async { Ok(crate::index::Progress::default()) },
            || true
        )
        .await
        .is_err());
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        assert!(finish_reindex(
            data.path(),
            &new,
            async { Err::<crate::index::Progress, _>(anyhow::anyhow!("interrupted run")) },
            || false
        )
        .await
        .is_err());
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        assert_eq!(fs::read(legacy.join("recipe.json")).unwrap(), old_bytes);
    }
}
