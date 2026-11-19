//! Library selection only; per-document storage and recovery remain unchanged.
use crate::{engine::Recipe, pipeline::sha256};
use anyhow::{ensure, Context, Result};
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

/// Cross-process selection fence. Reviewed operations take a shared fence
/// before resolving a library; re-index holds the exclusive fence through the
/// pointer promotion. It is separate from the selected index's writer lock.
pub struct SelectionFence(fs::File);
impl Drop for SelectionFence {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

pub fn selection_fence(data: &Path, exclusive: bool) -> Result<SelectionFence> {
    let path = data.join("index-selection.lock");
    if path.symlink_metadata().is_ok() {
        ensure!(
            fs::symlink_metadata(&path)?.is_file(),
            "invalid index selection lock"
        );
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    if exclusive {
        fs2::FileExt::try_lock_exclusive(&file)
    } else {
        fs2::FileExt::try_lock_shared(&file)
    }
    .context("index selection is in use; retry after the current operation")?;
    Ok(SelectionFence(file))
}

/// Canonical JSON: recursively sorted object keys, compact UTF-8 serialization.
/// Sort explicitly so dependency feature unification cannot change the ID.
pub fn recipe_id(recipe: &Recipe) -> Result<String> {
    canonical_id(serde_json::to_value(recipe)?)
}

fn canonical_id(mut value: serde_json::Value) -> Result<String> {
    value.sort_all_objects();
    Ok(sha256(&serde_json::to_vec(&value)?)[..16].into())
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
#[cfg(test)]
fn activate(data: &Path, recipe: &Recipe) -> Result<()> {
    activate_with(data, recipe, &publication_dir(data, recipe)?, || Ok(()))
}

fn activate_with(
    data: &Path,
    recipe: &Recipe,
    relative: &str,
    before_rename: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let id = recipe_id(recipe)?;
    ensure!(
        relative == "phase0-index" || relative == format!("indexes/{id}"),
        "invalid index directory"
    );
    let root = data.canonicalize()?;
    let target = root.join(relative).canonicalize()?;
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
        dir: relative.to_owned(),
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

/// Read the persisted seller selection, falling back to legacy document paths.
/// Never acquire a writer or run recovery.
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
    if let Some(selected) = crate::storage::State::selected_folder(&db).ok()? {
        return selected.to_str().map(str::to_owned);
    }
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

// Resolve surviving ancestors too: a removed selected directory still has the
// same identity, including when its caller used a symlinked temporary root.
fn source_identity(folder: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(folder).ok()?;
    for ancestor in absolute.ancestors() {
        if let Ok(parent) = ancestor.canonicalize() {
            return Some(parent.join(absolute.strip_prefix(ancestor).ok()?));
        }
    }
    None
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
    let mut started = false;
    let mut failed_switch = false;
    let mut progress = progress;
    let result = async {
        ensure!(!cancelled(), "Indexing cancelled; Re-index to resume");
        fs::create_dir_all(data)?;
        let _selection = selection_fence(data, true)?;
        let selected = publication_dir(data, &engines.recipe)?;
        let root = data.join(&selected);
        fs::create_dir_all(&root)?;
        ensure!(
            root.canonicalize()?.starts_with(data.canonicalize()?),
            "index outside data root"
        );
        let previous = last_indexed_folder(&resolve(data));
        let different_folder = previous.is_some_and(|previous| {
            source_identity(folder).as_ref() != Some(&PathBuf::from(previous))
        });
        let restore_on_failure =
            different_folder && root.canonicalize().ok() == resolve(data).canonicalize().ok();
        let replacing_documents =
            crate::storage::State::has_persisted_documents(&resolve(data)).unwrap_or(true);
        let mut state = crate::storage::State::open(&root)?;
        state
            .db
            .execute_batch("CREATE TEMP TABLE previous_documents AS SELECT * FROM documents")?;
        let revoke_unavailable =
            !different_folder || crate::storage::State::selected_folder(&state.db)?.is_none();
        let mut prepared = false;
        let mut source_revoked = false;
        let result = finish_reindex(
            data,
            &engines.recipe,
            &selected,
            async {
                let scan = crate::index::scan(&mut state, folder, engines, assets, |p| {
                    started = true;
                    latest = p.clone();
                    progress(p);
                })
                .await;
                let (stats, paths) = match scan {
                    Err(error)
                        if revoke_unavailable && crate::index::source_unavailable(&error) =>
                    {
                        state.db.execute("DELETE FROM documents", [])?;
                        source_revoked = true;
                        state.recover()?;
                        return Err(error);
                    }
                    result => result?,
                };
                // Current-source revocation is independent of publishability.
                if !different_folder {
                    crate::index::publish_source(&state, folder, &paths)?;
                }
                validate_publication(&stats, replacing_documents)?;
                state.db.execute_batch(
                    "CREATE TEMP TABLE publication_documents AS SELECT * FROM documents;
                     CREATE TEMP TABLE publication_metadata AS SELECT * FROM index_metadata;",
                )?;
                crate::index::prepare_source(&state, folder, &paths)?;
                prepared = true;
                Ok(stats)
            },
            &cancelled,
        )
        .await;
        if result.is_err() {
            failed_switch = different_folder && !source_revoked;
            // Restore publication-only changes, preserving per-file revocations
            // and successful revisions made during the scan.
            if prepared {
                state.db.execute_batch(
                    "BEGIN; DELETE FROM documents;
                     INSERT INTO documents SELECT * FROM publication_documents;
                     DELETE FROM index_metadata;
                     INSERT INTO index_metadata SELECT * FROM publication_metadata; COMMIT;",
                )?;
            }
            if restore_on_failure {
                state.db.execute_batch(
                    "DELETE FROM documents WHERE path NOT IN (SELECT path FROM previous_documents)",
                )?;
            }
        }
        state.recover()?;
        result
    }
    .await;
    result.map_err(|e| {
        let message = e.to_string();
        let cause = if cancelled() || message.contains("cancelled") {
            "cancelled"
        } else if message.contains("source missing") {
            "source missing"
        } else if e.chain().any(|e| {
            let message = e.to_string();
            message.contains("in use") || message.contains("lock")
                || message.contains("index already open by another process")
        }) {
            "index in use"
        } else {
            "indexing interrupted"
        };
        if failed_switch {
            let summary = if message.starts_with("Re-index did not finish") {
                message.strip_suffix("; Re-index to resume").unwrap_or(&message).to_owned()
            } else {
                format!(
                    "Re-index did not finish: {} indexed, {} skipped, {} failed ({} infrastructure), {} total; {cause}",
                    latest.completed, latest.skipped, latest.failed, latest.infrastructure_failed, latest.total
                )
            };
            return anyhow::anyhow!("{summary}; the previous folder is still active. Re-index the new folder to try again.");
        }
        if message.starts_with("Re-index did not finish") {
            return e;
        }
        if !started {
            return anyhow::anyhow!("Re-index did not finish; {cause}; Re-index to resume");
        }
        anyhow::anyhow!(
            "Re-index did not finish: {} indexed, {} skipped, {} failed, {} total; {cause}; Re-index to resume",
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
    selected: &str,
    run: impl std::future::Future<Output = Result<crate::index::Progress>>,
    cancelled: impl Fn() -> bool,
) -> Result<crate::index::Progress> {
    let replacing_documents =
        crate::storage::State::has_persisted_documents(&resolve(data)).unwrap_or(true);
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
    validate_publication(&result, replacing_documents)?;
    activate_with(data, recipe, selected, || Ok(()))?;
    Ok(result)
}

fn validate_publication(stats: &crate::index::Progress, replacing_documents: bool) -> Result<()> {
    stats.ensure_publishable()?;
    if stats.total > 0 && stats.completed + stats.skipped == 0 {
        ensure!(
            !replacing_documents,
            "Re-index did not finish: {} indexed, {} skipped, {} failed, {} total; no successful documents; publication refused; Re-index to resume",
            stats.completed, stats.skipped, stats.failed, stats.total
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_source_fallback_then_persisted_selection() {
        let temp = tempfile::tempdir().unwrap();
        let selected = temp.path().join("source");
        let deep = selected.join("reports");
        fs::create_dir_all(&deep).unwrap();
        let selected = selected.canonicalize().unwrap();
        let deep = deep.canonicalize().unwrap();
        let root = temp.path().join("index");
        fs::create_dir(&root).unwrap();
        let state = crate::storage::State::open(&root).unwrap();
        state
            .db
            .execute_batch("DROP TABLE index_metadata; DROP INDEX jobs_path;")
            .unwrap();
        state
            .db
            .execute(
                "INSERT INTO documents VALUES('legacy',?1,0,'mtime','hash','generation')",
                [deep.join("good.txt").to_str().unwrap()],
            )
            .unwrap();
        drop(state);
        assert_eq!(
            last_indexed_folder(&root),
            Some(deep.to_str().unwrap().into())
        );
        // Opening an old index adds metadata/index without losing legacy inference.
        let state = crate::storage::State::open(&root).unwrap();
        let has_index: bool = state.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='jobs_path')", [], |r| r.get(0)
        ).unwrap();
        assert!(has_index);
        drop(state);
        assert_eq!(
            last_indexed_folder(&root),
            Some(deep.to_str().unwrap().into())
        );
        let state = crate::storage::State::open(&root).unwrap();
        state
            .db
            .execute(
                "INSERT INTO index_metadata VALUES('selected_folder',?1)",
                [selected.to_str().unwrap()],
            )
            .unwrap();
        drop(state);
        assert_eq!(
            last_indexed_folder(&root),
            Some(selected.to_str().unwrap().into())
        );
    }

    #[test]
    fn selection_fence_serializes_publication_against_native_review_readers() {
        let data = tempfile::tempdir().unwrap();
        let first = selection_fence(data.path(), false).unwrap();
        let second = selection_fence(data.path(), false).unwrap();
        assert!(selection_fence(data.path(), true).is_err());
        drop(first);
        assert!(selection_fence(data.path(), true).is_err());
        drop(second);
        let writer = selection_fence(data.path(), true).unwrap();
        assert!(selection_fence(data.path(), false).is_err());
        drop(writer);
        selection_fence(data.path(), false).unwrap();
    }

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
        let error = activate_with(
            data.path(),
            &new,
            &publication_dir(data.path(), &new).unwrap(),
            || {
                // At the last boundary before rename readers still see the old file.
                assert_eq!(fs::read(data.path().join("active-index.json"))?, before);
                anyhow::bail!("simulated rename failure")
            },
        );
        assert!(error.is_err());
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            before
        );
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 2);
        fs::remove_file(data.path().join("active-index.json")).unwrap();
        assert!(activate_with(
            data.path(),
            &new,
            &publication_dir(data.path(), &new).unwrap(),
            || anyhow::bail!("failed first write")
        )
        .is_err());
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
    async fn replacing_populated_index_requires_success_and_only_content_failures_then_resumes() {
        use crate::index::Progress;
        let data = tempfile::tempdir().unwrap();
        let old = recipe();
        seed(data.path(), &old);
        activate(data.path(), &old).unwrap();
        let old_state = crate::storage::State::open(&resolve(data.path())).unwrap();
        old_state
            .db
            .execute(
                "INSERT INTO documents VALUES('old','old.txt',1,'mtime','hash','old')",
                [],
            )
            .unwrap();
        drop(old_state);
        let before = fs::read(data.path().join("active-index.json")).unwrap();
        let mut new = old.clone();
        new.model.push_str("-new");
        let candidate = seed(data.path(), &new);
        {
            let stats = Progress {
                total: 3,
                completed: 1,
                failed: 2,
                infrastructure_failed: 1,
                ..Default::default()
            };
            let error = finish_reindex(
                data.path(),
                &new,
                &publication_dir(data.path(), &new).unwrap(),
                async { Ok(stats) },
                || false,
            )
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
        let error = finish_reindex(
            data.path(),
            &new,
            &publication_dir(data.path(), &new).unwrap(),
            async {
                Ok(Progress {
                    total: 1,
                    failed: 1,
                    ..Default::default()
                })
            },
            || false,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Re-index did not finish"));
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            before
        );
        assert!(candidate.join("recipe.json").is_file());
        // Resumed work may skip already committed documents and exclude corrupt ones.
        let stats = Progress {
            total: 3,
            skipped: 1,
            completed: 1,
            failed: 1,
            ..Default::default()
        };
        finish_reindex(
            data.path(),
            &new,
            &publication_dir(data.path(), &new).unwrap(),
            async { Ok(stats) },
            || false,
        )
        .await
        .unwrap();
        assert_eq!(resolve(data.path()), candidate.canonicalize().unwrap());
        // A genuinely empty folder is publishable, too.
        finish_reindex(
            data.path(),
            &new,
            &publication_dir(data.path(), &new).unwrap(),
            async { Ok(Progress::default()) },
            || false,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn failed_or_cancelled_folder_switch_keeps_previous_review_source() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let engines = crate::engine::Engines::load_embedding(
            &repo.join(".assets"),
            &repo.join("spike/fixtures/models.json"),
            true,
        )
        .unwrap();
        let data = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("good.txt"), "A searchable source document.").unwrap();
        reindex(
            data.path(),
            a.path(),
            &engines,
            &repo.join(".assets"),
            |_| {},
            || false,
        )
        .await
        .unwrap();
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        // First successfully commit a B document, then hit an infrastructure failure.
        fs::write(b.path().join("a-good.txt"), "Another searchable document.").unwrap();
        fs::write(b.path().join("z-bad.pdf"), "not a PDF").unwrap();
        let error = reindex(
            data.path(),
            b.path(),
            &engines,
            &b.path().join("missing-assets"),
            |_| {},
            || false,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Re-index did not finish"));
        assert!(error.to_string().contains("1 indexed"));
        assert!(error.to_string().contains("1 failed"));
        assert!(error.to_string().contains("1 infrastructure"));
        assert!(!error.to_string().contains("resume"));
        for cancel_after_file in [false, true] {
            if cancel_after_file {
                let cancelled = AtomicBool::new(false);
                let error = reindex(
                    data.path(),
                    b.path(),
                    &engines,
                    &repo.join(".assets"),
                    |_| {
                        cancelled.store(true, Ordering::SeqCst);
                    },
                    || cancelled.load(Ordering::SeqCst),
                )
                .await
                .unwrap_err();
                assert!(error.to_string().contains("cancelled"));
            }
            let index = resolve(data.path());
            assert_eq!(
                last_indexed_folder(&index).as_deref(),
                a.path().canonicalize().unwrap().to_str()
            );
            assert_eq!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                pointer
            );
            let docs = crate::storage::State::review_documents(&index).unwrap();
            assert_eq!(docs.len(), 1);
            assert!(Path::new(&docs[0].path).starts_with(a.path().canonicalize().unwrap()));
        }
    }

    fn embedding_fixture() -> (crate::engine::Engines, PathBuf) {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let assets = repo.join(".assets");
        let engines = crate::engine::Engines::load_embedding(
            &assets,
            &repo.join("spike/fixtures/models.json"),
            true,
        )
        .unwrap();
        (engines, assets)
    }

    #[tokio::test]
    async fn publication_failure_after_scan_keeps_old_recipe_documents_and_source() {
        let (mut engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let sources = tempfile::tempdir().unwrap();
        let a = sources.path().join("a");
        let b = sources.path().join("b");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(a.join("good.txt"), "A searchable source document.").unwrap();
        fs::write(b.join("good.txt"), "B replacement source document.").unwrap();
        reindex(data.path(), &a, &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        let old = resolve(data.path());
        let documents = crate::storage::State::review_documents(&old).unwrap();
        engines.recipe.model.push_str("-new-recipe");
        let error = reindex(
            data.path(),
            &b,
            &engines,
            &assets,
            |p| {
                if p.completed + p.skipped + p.failed == p.total {
                    fs::rename(&b, sources.path().join("renamed-b")).unwrap();
                }
            },
            || false,
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("previous folder is still active"));
        assert!(!error.to_string().contains("resume"));
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        assert_eq!(resolve(data.path()), old);
        let candidate =
            crate::storage::State::open(&recipe_dir(data.path(), &engines.recipe).unwrap())
                .unwrap();
        assert_eq!(
            candidate.documents().unwrap().len(),
            1,
            "separate recipe candidates remain resumable"
        );
        drop(candidate);
        let reopened = crate::storage::State::open(&resolve(data.path())).unwrap();
        assert_eq!(reopened.documents().unwrap()[0].sha256, documents[0].sha256);
        assert_eq!(
            last_indexed_folder(&old).as_deref(),
            a.canonicalize().unwrap().to_str()
        );
    }

    #[tokio::test]
    async fn missing_persisted_library_source_revokes_documents() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("good.txt"), "Cached source text.").unwrap();
        reindex(
            data.path(),
            source.path(),
            &engines,
            &assets,
            |_| {},
            || false,
        )
        .await
        .unwrap();
        let folder = source.path().to_owned();
        source.close().unwrap();
        let error = reindex(data.path(), &folder, &engines, &assets, |_| {}, || false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("source missing"), "{error}");
        let state = crate::storage::State::open(&resolve(data.path())).unwrap();
        assert!(state.documents().unwrap().is_empty());
        assert_eq!(
            fs::read_dir(state.root.join("generations"))
                .unwrap()
                .count(),
            0
        );
        assert!(state
            .search("Cached", &vec![0.; engines.recipe.dimensions], 3)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn unavailable_source_without_persisted_library_folder_revokes_documents() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("good.txt"), "Legacy cached source.").unwrap();
        reindex(
            data.path(),
            source.path(),
            &engines,
            &assets,
            |_| {},
            || false,
        )
        .await
        .unwrap();
        let root = resolve(data.path());
        let state = crate::storage::State::open(&root).unwrap();
        state.db.execute("DELETE FROM index_metadata", []).unwrap();
        drop(state);
        let error = reindex(
            data.path(),
            &source.path().join("missing"),
            &engines,
            &assets,
            |_| {},
            || false,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("source missing"), "{error}");
        assert!(!error
            .to_string()
            .contains("previous folder is still active"));
        let state = crate::storage::State::open(&root).unwrap();
        assert!(state.documents().unwrap().is_empty());
        assert_eq!(fs::read_dir(root.join("generations")).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn completed_current_scan_revokes_deleted_paths_despite_missing_pdfium_both_entry_points()
    {
        let (engines, assets) = embedding_fixture();
        for library in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            let text = source.path().join("good.txt");
            fs::write(&text, "Deleted searchable source text.").unwrap();
            let root = if library {
                reindex(
                    data.path(),
                    source.path(),
                    &engines,
                    &assets,
                    |_| {},
                    || false,
                )
                .await
                .unwrap();
                resolve(data.path())
            } else {
                let mut state = crate::storage::State::open(data.path()).unwrap();
                crate::index::index(&mut state, source.path(), &engines, &assets, |_| {})
                    .await
                    .unwrap();
                data.path().to_owned()
            };
            let state = crate::storage::State::open(&root).unwrap();
            let generation = state.generations().unwrap().pop().unwrap();
            drop(state);
            fs::remove_file(text).unwrap();
            fs::write(source.path().join("broken.pdf"), "not a PDF").unwrap();
            let missing = source.path().join("missing-assets");
            if library {
                assert!(reindex(
                    data.path(),
                    source.path(),
                    &engines,
                    &missing,
                    |_| {},
                    || false
                )
                .await
                .is_err());
            } else {
                let mut state = crate::storage::State::open(&root).unwrap();
                let stats =
                    crate::index::index(&mut state, source.path(), &engines, &missing, |_| {})
                        .await
                        .unwrap();
                assert_eq!(stats.infrastructure_failed, 1);
            }
            let state = crate::storage::State::open(&root).unwrap();
            assert!(state.documents().unwrap().is_empty());
            assert!(!root.join("generations").join(generation).exists());
            assert!(state
                .search("Deleted", &vec![0.; engines.recipe.dimensions], 3)
                .await
                .unwrap()
                .is_empty());
        }
    }

    #[tokio::test]
    async fn failed_parent_switch_preserves_per_file_revocation_and_updated_revision() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let a = parent.path().join("a");
        fs::create_dir(&a).unwrap();
        fs::write(a.join("empty.txt"), "Old content must disappear.").unwrap();
        fs::write(a.join("updated.txt"), "Original revision.").unwrap();
        reindex(data.path(), &a, &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
        fs::write(a.join("empty.txt"), "").unwrap();
        fs::write(a.join("updated.txt"), "Replacement revision survives.").unwrap();
        fs::write(parent.path().join("new.txt"), "New parent document.").unwrap();
        fs::write(parent.path().join("broken.pdf"), "not a PDF").unwrap();
        let error = reindex(
            data.path(),
            parent.path(),
            &engines,
            &parent.path().join("missing-assets"),
            |_| {},
            || false,
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("previous folder is still active"));
        let root = resolve(data.path());
        assert_eq!(
            last_indexed_folder(&root).as_deref(),
            a.canonicalize().unwrap().to_str()
        );
        let state = crate::storage::State::open(&root).unwrap();
        let docs = state.documents().unwrap();
        assert_eq!(docs.len(), 1);
        assert!(docs[0].path.ends_with("updated.txt"));
        assert!(docs[0].chunks[0]
            .text
            .contains("Replacement revision survives"));
        assert_eq!(fs::read_dir(root.join("generations")).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn damaged_active_storage_allows_successful_recipe_migration() {
        let data = tempfile::tempdir().unwrap();
        let old = recipe();
        let old_root = seed(data.path(), &old);
        activate(data.path(), &old).unwrap();
        fs::write(old_root.join("state.sqlite"), "damaged database").unwrap();
        let mut new = old.clone();
        new.model.push_str("-recovery");
        let candidate = seed(data.path(), &new);
        let selected = publication_dir(data.path(), &new).unwrap();
        let error = finish_reindex(
            data.path(),
            &new,
            &selected,
            async {
                Ok(crate::index::Progress {
                    total: 1,
                    failed: 1,
                    ..Default::default()
                })
            },
            || false,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("publication refused"));
        assert!(!error.to_string().contains("active index retained"));
        finish_reindex(
            data.path(),
            &new,
            &selected,
            async {
                Ok(crate::index::Progress {
                    total: 1,
                    completed: 1,
                    ..Default::default()
                })
            },
            || false,
        )
        .await
        .unwrap();
        assert_eq!(resolve(data.path()), candidate.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn busy_index_reports_neutral_cause() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let root = recipe_dir(data.path(), &engines.recipe).unwrap();
        fs::create_dir_all(&root).unwrap();
        let _writer = crate::storage::State::open(&root).unwrap();
        let error = reindex(
            data.path(),
            source.path(),
            &engines,
            &assets,
            |_| {},
            || false,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("index in use"), "{error}");
        assert!(!error.to_string().contains(root.to_str().unwrap()));
    }

    #[tokio::test]
    async fn completed_document_plus_worker_timeout_publishes() {
        let data = tempfile::tempdir().unwrap();
        let recipe = recipe();
        let candidate = seed(data.path(), &recipe);
        let selected = publication_dir(data.path(), &recipe).unwrap();
        let _state = crate::storage::State::open(&candidate).unwrap();
        let timeout = crate::failures::content("worker timeout after 60000ms");
        finish_reindex(
            data.path(),
            &recipe,
            &selected,
            async {
                Ok(crate::index::Progress {
                    completed: 1,
                    failed: 1,
                    total: 2,
                    infrastructure_failed: usize::from(!crate::failures::is_content(&timeout)),
                    failure_examples: vec![format!("slow.pdf: {timeout}")],
                    ..Default::default()
                })
            },
            || false,
        )
        .await
        .unwrap();
        assert_eq!(resolve(data.path()), candidate.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn pdfium_io_mixed_success_keeps_pointer_then_retry_succeeds() {
        use crate::{
            failures,
            index::Progress,
            worker::{read_frame, write_frame, Response},
        };
        let data = tempfile::tempdir().unwrap();
        let old = recipe();
        seed(data.path(), &old);
        activate(data.path(), &old).unwrap();
        let before = fs::read(data.path().join("active-index.json")).unwrap();
        let mut new = old.clone();
        new.model.push_str("-new");
        let candidate = seed(data.path(), &new);
        let selected = publication_dir(data.path(), &new).unwrap();
        let _state = crate::storage::State::open(&candidate).unwrap();
        let error = failures::parser(
            pdfium_render::prelude::PdfiumError::IoError(std::io::Error::from(
                std::io::ErrorKind::Other,
            ))
            .into(),
        );
        let response = Response {
            pages: vec![],
            error: Some(format!("{error:#}")),
            content_failure: failures::is_content(&error),
        };
        let mut bytes = vec![];
        write_frame(&mut bytes, &response).unwrap();
        let received: Response = read_frame(bytes.as_slice()).unwrap();
        let stats = Progress {
            completed: 1,
            failed: 1,
            total: 2,
            infrastructure_failed: usize::from(!received.content_failure),
            infrastructure_examples: vec![format!("broken.pdf: {}", received.error.unwrap())],
            ..Default::default()
        };
        let error = finish_reindex(data.path(), &new, &selected, async { Ok(stats) }, || false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("broken.pdf:"), "{error:#}");
        assert!(error.to_string().contains("IoError"), "{error:#}");
        assert_eq!(
            error.to_string().matches("Re-index did not finish").count(),
            1
        );
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            before
        );
        finish_reindex(
            data.path(),
            &new,
            &selected,
            async {
                Ok(Progress {
                    skipped: 1,
                    completed: 1,
                    total: 2,
                    ..Default::default()
                })
            },
            || false,
        )
        .await
        .unwrap();
        assert_eq!(resolve(data.path()), candidate.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn legacy_appearing_after_selection_cannot_replace_locked_candidate() {
        let data = tempfile::tempdir().unwrap();
        let recipe = recipe();
        let selected = publication_dir(data.path(), &recipe).unwrap();
        let candidate = seed(data.path(), &recipe);
        let _state = crate::storage::State::open(&candidate).unwrap();
        finish_reindex(
            data.path(),
            &recipe,
            &selected,
            async {
                let legacy = data.path().join("phase0-index");
                fs::create_dir(&legacy)?;
                fs::write(legacy.join("recipe.json"), serde_json::to_vec(&recipe)?)?;
                assert_eq!(publication_dir(data.path(), &recipe)?, "phase0-index");
                assert!(
                    crate::storage::State::open(&candidate).is_err(),
                    "candidate lock must still be held"
                );
                Ok(crate::index::Progress {
                    completed: 1,
                    total: 1,
                    ..Default::default()
                })
            },
            || false,
        )
        .await
        .unwrap();
        assert_eq!(resolve(data.path()), candidate.canonicalize().unwrap());
        assert!(!data.path().join("phase0-index/state.sqlite").exists());
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
            &publication_dir(data.path(), &new).unwrap(),
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
            &publication_dir(data.path(), &new).unwrap(),
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
            &publication_dir(data.path(), &new).unwrap(),
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
            &publication_dir(data.path(), &new).unwrap(),
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
