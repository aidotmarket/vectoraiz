//! Isolated candidate scans and atomic library/explicit-index publication.
use crate::{engine::Recipe, pipeline::sha256};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    future::Future,
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
        if !valid_directory(&pointer.dir, &pointer.recipe_id) {
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
    if let Some(target) = verified_target(path) {
        target
    } else if path.join("state.sqlite").is_file() {
        path.to_owned()
    } else if is_library_root(path) {
        resolve(path)
    } else {
        path.to_owned()
    }
}

// Recipe snapshots have a validated UUID suffix; arbitrary pointer paths remain refused.
fn valid_directory(relative: &str, id: &str) -> bool {
    relative == "phase0-index"
        || relative == format!("indexes/{id}")
        || relative
            .strip_prefix(&format!("indexes/{id}-"))
            .is_some_and(|suffix| {
                uuid::Uuid::parse_str(suffix).is_ok_and(|u| u.to_string() == suffix)
            })
}

/// Keep an explicitly opened selected snapshot attached to its library. Without
/// this, indexing that State would create a nested pointer inside the snapshot.
pub(crate) fn state_location(path: &Path) -> PathBuf {
    if path.file_name().is_some_and(|name| name == "phase0-index") {
        if let Some(data) = path.parent() {
            return if data.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                data.to_owned()
            };
        }
    }
    if let Some(indexes) = path
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "indexes"))
    {
        if let Some(data) = indexes
            .parent()
            .filter(|data| data.join("active-index.json").is_file())
        {
            let recipe = fs::read(path.join("recipe.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Recipe>(&b).ok());
            if let Some(recipe) = recipe {
                if recipe_id(&recipe).ok().is_some_and(|id| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|name| valid_directory(&format!("indexes/{name}"), &id))
                }) {
                    return if data.as_os_str().is_empty() {
                        PathBuf::from(".")
                    } else {
                        data.to_owned()
                    };
                }
            }
        }
    }
    path.to_owned()
}

#[cfg(test)]
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

// This marker survives anyhow context so callers can distinguish a visible
// publication from a failure that is still safe to roll back.
#[derive(Debug)]
pub(crate) struct PublishedFailure(pub(crate) &'static str);
impl std::fmt::Display for PublishedFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Index published; {} step must be retried", self.0)
    }
}
impl std::error::Error for PublishedFailure {}

#[cfg(test)]
thread_local! {
    static FAULTS: std::cell::RefCell<Vec<(PathBuf, &'static str)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn publication_step(_data: &Path, _step: &'static str) -> Result<()> {
    #[cfg(test)]
    FAULTS.with(|faults| {
        if faults
            .borrow()
            .iter()
            .any(|(path, step)| path == _data && *step == _step)
        {
            anyhow::bail!("injected {_step} failure");
        }
        Ok(())
    })?;
    Ok(())
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
    ensure!(valid_directory(relative, &id), "invalid index directory");
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
        (|| -> Result<()> {
            publication_step(data, "directory sync")?;
            #[cfg(unix)]
            fs::File::open(data)?.sync_all()?;
            Ok(())
        })()
        .context(PublishedFailure("durability"))?;
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
    let resolved = resolve_state(root);
    let root = resolved.as_path();
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
pub(crate) fn source_identity(folder: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(folder).ok()?;
    for ancestor in absolute.ancestors() {
        if let Ok(parent) = ancestor.canonicalize() {
            return Some(parent.join(absolute.strip_prefix(ancestor).ok()?));
        }
    }
    None
}

// Existing paths canonicalize to the volume's spelling. For a removed tail,
// compare against the persisted spelling only after proving its surviving
// ancestor accepts alternate case. Case-sensitive volumes keep exact identity.
pub(crate) fn same_source(folder: &Path, previous: &Path) -> bool {
    let Some(current) = source_identity(folder) else {
        return false;
    };
    // The persisted canonical path is historical evidence. Resolving it again
    // could follow a new symlink to a different source.
    if current == previous {
        return true;
    }
    for ancestor in current.ancestors() {
        let Some(name) = ancestor.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let swapped: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect();
        if swapped == name {
            continue;
        }
        if let (Ok(actual), Ok(alternate)) = (
            ancestor.canonicalize(),
            ancestor.with_file_name(swapped).canonicalize(),
        ) {
            if actual == alternate {
                return current
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&previous.to_string_lossy());
            }
            return false;
        }
        if ancestor.exists() {
            return false;
        }
    }
    false
}

#[derive(Debug, PartialEq, Eq)]
enum ScanDecision {
    Publish,
    RevokeSource,
    RevokeFailedPaths,
    Discard,
}

/// Pure policy: a completed current-source walk always publishes its revocations.
fn decide_scan(
    outcome: &crate::index::ScanOutcome,
    current: bool,
    populated: bool,
) -> ScanDecision {
    if current && outcome.unavailable {
        return ScanDecision::RevokeSource;
    }
    if current && outcome.completed_walk {
        return ScanDecision::Publish;
    }
    if !outcome.unavailable
        && outcome.completed_walk
        && validate_publication(&outcome.stats, populated).is_ok()
    {
        return ScanDecision::Publish;
    }
    // An overlapping failed path still revokes its old revision, but successful
    // writes from a refused switch never cross the publication boundary.
    if !outcome.failed_content.is_empty() || !outcome.failed_infrastructure.is_empty() {
        return ScanDecision::RevokeFailedPaths;
    }
    ScanDecision::Discard
}

fn candidate_dir(data: &Path, recipe: &Recipe) -> Result<(String, PathBuf)> {
    let id = recipe_id(recipe)?;
    let relative = if !data.join(format!("indexes/{id}")).try_exists()? {
        format!("indexes/{id}")
    } else {
        format!("indexes/{id}-{}", uuid::Uuid::new_v4())
    };
    let root = data.join(&relative);
    fs::create_dir_all(data.join("indexes"))?;
    ensure!(
        data.join("indexes")
            .canonicalize()?
            .starts_with(data.canonicalize()?),
        "index outside data root"
    );
    fs::create_dir(&root)?;
    ensure!(
        root.canonicalize()?.starts_with(data.canonicalize()?),
        "index outside data root"
    );
    Ok((relative, root))
}

// VACUUM includes committed WAL rows without checkpointing or copying a live DB
// file. Generation files are immutable; copy them so recovery stays isolated.
fn copy_index(source: &Path, db: &rusqlite::Connection, destination: &Path) -> Result<()> {
    db.execute(
        "VACUUM INTO ?1",
        [destination
            .join("state.sqlite")
            .to_str()
            .context("UTF-8 index path")?],
    )?;
    let generations = db
        .prepare("SELECT generation FROM documents ORDER BY generation")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for generation in generations {
        ensure!(
            !generation.is_empty()
                && generation
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "unsafe index generation"
        );
        copy_tree(
            &source.join("generations").join(&generation),
            &destination.join("generations").join(generation),
        )?;
    }
    if source.join("recipe.json").is_file() {
        fs::copy(source.join("recipe.json"), destination.join("recipe.json"))?;
    }
    Ok(())
}

fn copy_active(
    source: &Path,
    explicit: Option<&crate::storage::State>,
    destination: &Path,
) -> Result<()> {
    if let Some(state) = explicit {
        return copy_index(&state.root, &state.db, destination);
    }
    // Fence a stable snapshot without opening a writer, running recovery, or
    // migrating the active SQLite schema.
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(source.join("writer.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file).context("index already open by another process")?;
    let _lock = SelectionFence(file);
    let db = crate::storage::State::review_connection(source)?;
    copy_index(source, &db, destination)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        !source.symlink_metadata()?.file_type().is_symlink(),
        "symlink in index snapshot"
    );
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        ensure!(
            !entry.file_type()?.is_symlink(),
            "symlink in index snapshot"
        );
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        } else {
            let path = destination.join(entry.file_name());
            fs::copy(entry.path(), &path)?;
            fs::File::open(path)?.sync_all()?;
        }
    }
    #[cfg(unix)]
    fs::File::open(destination)?.sync_all()?;
    Ok(())
}

fn publish_candidate(
    data: &Path,
    recipe: &Recipe,
    relative: &str,
    state: &crate::storage::State,
) -> Result<()> {
    // Remove revoked candidate generations before publication. Any failure here
    // leaves the current selection intact.
    state.recover()?;
    #[cfg(unix)]
    {
        fs::File::open(state.root.join("generations"))?.sync_all()?;
        fs::File::open(&state.root)?.sync_all()?;
        fs::File::open(data.join("indexes"))?.sync_all()?;
    }
    activate_with(data, recipe, relative, || Ok(()))?;
    publication_step(data, "generation cleanup")
        .and_then(|()| state.recover())
        .context(PublishedFailure("generation cleanup"))
}

/// Shared transaction for library and explicit-index entry points. The explicit
/// caller may already own the active writer; only its snapshot is read here.
pub(crate) async fn scan_candidate(
    data: &Path,
    folder: &Path,
    engines: &crate::engine::Engines,
    assets: &Path,
    progress: impl FnMut(&crate::index::Progress),
    cancelled: impl Fn() -> bool,
    explicit: Option<&crate::storage::State>,
) -> Result<crate::index::Progress> {
    ensure!(!cancelled(), "Indexing cancelled; Re-index to resume");
    fs::create_dir_all(data)?;
    let _selection = if explicit.is_some() {
        Some(selection_fence(data, true)?)
    } else {
        None
    };
    let active = resolve_state(data);
    if let Some(state) = explicit {
        ensure!(
            state.root.canonicalize()? == active.canonicalize()?,
            "index selection changed; reopen before indexing"
        );
    }
    let previous = last_indexed_folder(&active);
    let current = previous
        .as_deref()
        .is_some_and(|p| same_source(folder, Path::new(p)));
    let populated = crate::storage::State::has_persisted_documents(&active).unwrap_or(true);
    let (relative, root) = candidate_dir(data, &engines.recipe)?;
    let mut scan_published = false;
    let result = async {
        let same_recipe = fs::read(active.join("recipe.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Recipe>(&b).ok())
            .is_some_and(|r| r == engines.recipe);
        if same_recipe {
            copy_active(&active, explicit, &root)?;
        }
        let mut state = crate::storage::State::open(&root)?;
        let mut outcome = {
            let run = crate::index::scan(&mut state, folder, engines, assets, progress);
            let mut run = Box::pin(run);
            futures::future::poll_fn(|cx| {
                if cancelled() {
                    std::task::Poll::Ready(Err(anyhow::anyhow!(
                        "Indexing cancelled; Re-index to resume"
                    )))
                } else {
                    run.as_mut().poll(cx)
                }
            })
            .await?
        };
        ensure!(!cancelled(), "Indexing cancelled; Re-index to resume");
        if outcome
            .source
            .as_ref()
            .is_some_and(|source| !source.is_dir())
        {
            outcome.unavailable = true;
            if outcome.error.is_none() {
                outcome.error = Some(anyhow::anyhow!("source missing"));
            }
        }
        let decision = decide_scan(&outcome, current, populated);
        match decision {
            ScanDecision::Publish => {
                let source = outcome.source.as_deref().context("source missing")?;
                // Detect disappearance after the last file callback, before publication.
                ensure!(source.is_dir(), "source missing");
                let previous_count: usize = if same_recipe {
                    state
                        .db
                        .query_row("SELECT count(*) FROM documents", [], |r| r.get(0))?
                } else {
                    0
                };
                let unchanged = current
                    && verified_target(data).as_deref() == Some(active.as_path())
                    && same_recipe
                    && outcome.stats.completed == 0
                    && outcome.stats.failed == 0
                    && outcome.stats.skipped == previous_count
                    && crate::storage::State::review_selected_folder(&active)
                        .ok()
                        .flatten()
                        .as_deref()
                        == Some(source);
                if unchanged {
                    return Ok(outcome.stats);
                }
                crate::index::prepare_source(&state, source, &outcome.succeeded)?;
                let publication = publish_candidate(data, &engines.recipe, &relative, &state);
                scan_published = publication.is_ok()
                    || publication
                        .as_ref()
                        .err()
                        .is_some_and(|e| e.is::<PublishedFailure>());
                publication?;
            }
            ScanDecision::RevokeSource | ScanDecision::RevokeFailedPaths => {
                // A refusal can publish only revocations of A, never B's writes.
                let active_recipe: Option<Recipe> = fs::read(active.join("recipe.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok());
                if let Some(recipe) = active_recipe {
                    let (rev_relative, rev_root) = candidate_dir(data, &recipe)?;
                    let mut rev_published = false;
                    let revocation = (|| -> Result<()> {
                        copy_active(&active, explicit, &rev_root)?;
                        let revoked = crate::storage::State::open(&rev_root)?;
                        let mut changed = 0;
                        if decision == ScanDecision::RevokeSource {
                            changed += revoked.db.execute("DELETE FROM documents", [])?;
                        } else {
                            for path in outcome
                                .failed_content
                                .iter()
                                .chain(&outcome.failed_infrastructure)
                            {
                                changed += revoked
                                    .db
                                    .execute("DELETE FROM documents WHERE path=?1", [path])?;
                            }
                        }
                        if changed > 0 {
                            let publication =
                                publish_candidate(data, &recipe, &rev_relative, &revoked);
                            rev_published = publication.is_ok()
                                || publication
                                    .as_ref()
                                    .err()
                                    .is_some_and(|e| e.is::<PublishedFailure>());
                            publication?;
                        }
                        Ok(())
                    })();
                    if !rev_published {
                        if let Err(cleanup) = fs::remove_dir_all(&rev_root) {
                            return match revocation {
                                Err(error) => Err(error.context(format!(
                                    "Generation cleanup also failed: {cleanup:#}"
                                ))),
                                Ok(()) => Err(cleanup.into()),
                            };
                        }
                    }
                    revocation?;
                }
            }
            ScanDecision::Discard => {}
        }
        if outcome.unavailable {
            return Err(outcome
                .error
                .take()
                .unwrap_or_else(|| anyhow::anyhow!("source unavailable during scan")))
            .context("source unavailable");
        }
        if explicit.is_none() {
            if decision != ScanDecision::Publish {
                validate_publication(&outcome.stats, populated)?;
                anyhow::bail!("Re-index did not finish; publication refused");
            }
            outcome
                .stats
                .ensure_publishable()
                .map_err(|e| e.context(PublishedFailure("incomplete scan")))?;
        }
        Ok(outcome.stats)
    }
    .await;
    // Publication is irreversible, including errors after pointer rename.
    if !scan_published {
        let cleanup = publication_step(data, "generation cleanup")
            .and_then(|()| fs::remove_dir_all(&root).map_err(Into::into));
        if let Err(cleanup) = cleanup {
            return match result {
                Err(error) => {
                    Err(error.context(format!("Generation cleanup also failed: {cleanup:#}")))
                }
                Ok(_) => Err(cleanup),
            };
        }
    }
    result
}

/// Scan an isolated snapshot and publish the selected outcome by pointer rename.
/// Poll cancellation at async boundaries and once more before publication.
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
    let mut cleanup_failed = false;
    let mut progress = progress;
    let result = async {
        ensure!(!cancelled(), "Indexing cancelled; Re-index to resume");
        fs::create_dir_all(data)?;
        let _selection = selection_fence(data, true)?;
        let previous = last_indexed_folder(&resolve_state(data));
        failed_switch = previous
            .as_ref()
            .is_some_and(|p| !same_source(folder, Path::new(p)));
        let result = scan_candidate(
            data,
            folder,
            engines,
            assets,
            |p| {
                started = true;
                latest = p.clone();
                progress(p);
            },
            &cancelled,
            None,
        )
        .await;
        cleanup_failed = result
            .as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("cleanup also failed"));
        result
    }
    .await;
    result.map_err(|e| {
        if e.is::<PublishedFailure>() { return e; }
        let message = e.to_string();
        let cause = if cancelled() || message.contains("cancelled") {
            "cancelled"
        } else if e.chain().any(|error| error.to_string().contains("source missing")) {
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
        if !failed_switch && message.starts_with("Re-index did not finish") {
            return e;
        }
        // Ordinary diagnostics stay path-free and do not reintroduce a discarded
        // switch's resume hint through the CLI's full-chain error rendering.
        // Preserve the original chain when cleanup also failed.
        let diagnostic = |message: String| {
            if cleanup_failed { e.context(message) } else { anyhow::anyhow!(message) }
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
            return diagnostic(format!("{summary}; the previous folder is still active. Re-index the new folder to try again."));
        }
        if !started {
            return diagnostic(format!("Re-index did not finish; {cause}; Re-index to resume"));
        }
        diagnostic(format!(
            "Re-index did not finish: {} indexed, {} skipped, {} failed, {} total; {cause}; Re-index to resume",
            latest.completed,
            latest.skipped,
            latest.failed,
            latest.total
        ))
    })
}

#[cfg(test)]
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
    fn candidate_decision_table() {
        use crate::index::{Progress, ScanOutcome};
        let mut outcome = ScanOutcome {
            completed_walk: true,
            stats: Progress {
                total: 1,
                failed: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(decide_scan(&outcome, true, true), ScanDecision::Publish);
        assert_eq!(decide_scan(&outcome, false, true), ScanDecision::Discard);
        assert_eq!(decide_scan(&outcome, false, false), ScanDecision::Publish);
        outcome.stats.infrastructure_failed = 1;
        outcome.failed_infrastructure.insert("failed.txt".into());
        assert_eq!(decide_scan(&outcome, true, true), ScanDecision::Publish);
        assert_eq!(
            decide_scan(&outcome, false, false),
            ScanDecision::RevokeFailedPaths
        );
        outcome.completed_walk = false;
        outcome.unavailable = true;
        assert_eq!(
            decide_scan(&outcome, true, true),
            ScanDecision::RevokeSource
        );
        outcome.failed_infrastructure.clear();
        assert_eq!(decide_scan(&outcome, false, true), ScanDecision::Discard);
        outcome = ScanOutcome {
            completed_walk: true,
            ..Default::default()
        };
        assert_eq!(decide_scan(&outcome, false, true), ScanDecision::Publish);
    }

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
        assert!(!format!("{error:#}").contains("resume"));
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
        assert!(!format!("{error:#}").contains("resume"));
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        assert_eq!(resolve(data.path()), old);
        assert_eq!(
            fs::read_dir(data.path().join("indexes")).unwrap().count(),
            1,
            "a refused different-source candidate is discarded entirely"
        );
        let reopened = crate::storage::State::open(&resolve(data.path())).unwrap();
        assert_eq!(reopened.documents().unwrap()[0].sha256, documents[0].sha256);
        assert_eq!(
            last_indexed_folder(&old).as_deref(),
            a.canonicalize().unwrap().to_str()
        );
    }

    #[tokio::test]
    async fn missing_persisted_library_source_revokes_documents_after_recipe_change() {
        let (mut engines, assets) = embedding_fixture();
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
        engines.recipe.model.push_str("-changed");
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
    async fn legacy_library_keeps_inferred_source_when_other_folder_is_missing() {
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
        assert!(error
            .to_string()
            .contains("previous folder is still active"));
        let state = crate::storage::State::open(&root).unwrap();
        assert_eq!(state.documents().unwrap().len(), 1);
        assert_eq!(fs::read_dir(root.join("generations")).unwrap().count(), 1);
        assert_eq!(
            last_indexed_folder(&root).as_deref(),
            source.path().canonicalize().unwrap().to_str()
        );
    }

    #[tokio::test]
    async fn completed_current_scan_revokes_deleted_paths_despite_missing_pdfium_both_entry_points()
    {
        let (mut engines, assets) = embedding_fixture();
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
                engines.recipe.model.push_str("-changed");
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
            let state = crate::storage::State::open(&resolve_state(data.path())).unwrap();
            assert!(state.documents().unwrap().is_empty());
            assert!(!state.root.join("generations").join(generation).exists());
            assert!(state
                .search("Deleted", &vec![0.; engines.recipe.dimensions], 3)
                .await
                .unwrap()
                .is_empty());
        }
    }

    #[tokio::test]
    async fn explicit_index_failed_different_source_keeps_selection_and_search() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("good.txt"), "Original searchable selection.").unwrap();
        let mut state = crate::storage::State::open(data.path()).unwrap();
        crate::index::index(&mut state, a.path(), &engines, &assets, |_| {})
            .await
            .unwrap();
        let generations = state.generations().unwrap();
        fs::write(b.path().join("bad.pdf"), "not a PDF").unwrap();
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        // Keep the zero-success regression and extend it with a successful B row.
        for successful_b_file in [false, true] {
            if successful_b_file {
                fs::write(
                    b.path().join("a-good.txt"),
                    "Candidate text must stay invisible.",
                )
                .unwrap();
            }
            let stats = crate::index::index(
                &mut state,
                b.path(),
                &engines,
                &b.path().join("missing-assets"),
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(stats.infrastructure_failed, 1);
            assert_eq!(stats.completed, usize::from(successful_b_file));
            assert_eq!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                pointer
            );
            drop(state);
            assert_eq!(
                last_indexed_folder(data.path()).as_deref(),
                a.path().canonicalize().unwrap().to_str()
            );
            state = crate::storage::State::open(data.path()).unwrap();
            assert_eq!(state.generations().unwrap(), generations);
            assert_eq!(state.documents().unwrap().len(), 1);
            let reviewed = crate::storage::State::review_documents(&state.root).unwrap();
            assert_eq!(reviewed.len(), 1);
            assert_eq!(reviewed[0].chunks[0].text, "Original searchable selection.");
            assert_eq!(
                crate::storage::State::review_selected_folder(&state.root)
                    .unwrap()
                    .as_deref(),
                Some(a.path().canonicalize().unwrap().as_path())
            );
            let hits = state
                .search("Original", &engines.vector("Original", true).unwrap(), 3)
                .await
                .unwrap();
            assert!(!hits.is_empty());
            assert!(hits.iter().all(|hit| hit.chunk.text.contains("Original")));
        }
    }

    #[tokio::test]
    async fn failed_current_replacement_under_new_recipe_revokes_old_search_and_generation() {
        let (mut engines, assets) = embedding_fixture();
        for library in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            let text = source.path().join("good.txt");
            fs::write(&text, "Obsolete searchable revision.").unwrap();
            if library {
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
            } else {
                let mut state = crate::storage::State::open(data.path()).unwrap();
                crate::index::index(&mut state, source.path(), &engines, &assets, |_| {})
                    .await
                    .unwrap();
            }
            let before = fs::read(data.path().join("active-index.json")).unwrap();
            let generations = crate::storage::State::open(data.path())
                .unwrap()
                .generations()
                .unwrap();
            fs::write(&text, "").unwrap();
            engines.recipe.model.push_str("-new-recipe");
            if library {
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
            } else {
                let mut state = crate::storage::State::open(data.path()).unwrap();
                let stats =
                    crate::index::index(&mut state, source.path(), &engines, &assets, |_| {})
                        .await
                        .unwrap();
                assert_eq!(
                    (stats.completed, stats.failed, stats.infrastructure_failed),
                    (0, 1, 0)
                );
            }
            assert_ne!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                before
            );
            let (state, _) =
                crate::storage::State::open_index(data.path(), &engines.recipe).unwrap();
            assert!(state.documents().unwrap().is_empty());
            assert!(state.generations().unwrap().is_empty());
            for generation in generations {
                assert!(!state.root.join("generations").join(generation).exists());
            }
            assert!(crate::storage::State::review_documents(&state.root)
                .unwrap()
                .is_empty());
            assert_eq!(
                last_indexed_folder(data.path()).as_deref(),
                source.path().canonicalize().unwrap().to_str()
            );
            assert!(state
                .search("Obsolete", &vec![0.; engines.recipe.dimensions], 3)
                .await
                .unwrap()
                .is_empty());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn historical_selection_does_not_follow_retargeted_symlink() {
        let (engines, assets) = embedding_fixture();
        for library in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let folders = tempfile::tempdir().unwrap();
            let a = folders.path().join("a");
            let b = folders.path().join("b");
            fs::create_dir(&a).unwrap();
            fs::create_dir(&b).unwrap();
            fs::write(a.join("good.txt"), "Historical searchable selection.").unwrap();
            if library {
                reindex(data.path(), &a, &engines, &assets, |_| {}, || false)
                    .await
                    .unwrap();
            } else {
                let mut state = crate::storage::State::open(data.path()).unwrap();
                crate::index::index(&mut state, &a, &engines, &assets, |_| {})
                    .await
                    .unwrap();
            }
            let selected = last_indexed_folder(data.path()).unwrap();
            let pointer = fs::read(data.path().join("active-index.json")).unwrap();
            let generations = crate::storage::State::open(data.path())
                .unwrap()
                .generations()
                .unwrap();
            fs::rename(&a, folders.path().join("moved-a")).unwrap();
            std::os::unix::fs::symlink(&b, &a).unwrap();
            fs::write(b.join("bad.pdf"), "not a PDF").unwrap();
            if library {
                let error = reindex(
                    data.path(),
                    &b,
                    &engines,
                    &b.join("missing-assets"),
                    |_| {},
                    || false,
                )
                .await
                .unwrap_err();
                assert!(error
                    .to_string()
                    .contains("previous folder is still active"));
            } else {
                let mut state = crate::storage::State::open(data.path()).unwrap();
                let stats = crate::index::index(
                    &mut state,
                    &b,
                    &engines,
                    &b.join("missing-assets"),
                    |_| {},
                )
                .await
                .unwrap();
                assert_eq!(stats.infrastructure_failed, 1);
            }
            assert_eq!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                pointer
            );
            assert_eq!(
                last_indexed_folder(data.path()).as_deref(),
                Some(selected.as_str())
            );
            let (state, _) =
                crate::storage::State::open_index(data.path(), &engines.recipe).unwrap();
            assert_eq!(state.generations().unwrap(), generations);
            assert_eq!(state.documents().unwrap().len(), 1);
            let reviewed = crate::storage::State::review_documents(&state.root).unwrap();
            assert_eq!(reviewed.len(), 1);
            assert!(reviewed[0].chunks[0].text.contains("Historical"));
            assert!(!state
                .search(
                    "Historical",
                    &engines.vector("Historical", true).unwrap(),
                    3
                )
                .await
                .unwrap()
                .is_empty());
        }
    }

    #[tokio::test]
    async fn readers_during_same_and_changed_recipe_scans_see_only_published_snapshot() {
        let (mut engines, assets) = embedding_fixture();
        for changed_recipe in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            fs::write(source.path().join("a.txt"), "Published original text.").unwrap();
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
            let pointer = fs::read(data.path().join("active-index.json")).unwrap();
            let generations = crate::storage::State::open(data.path())
                .unwrap()
                .generations()
                .unwrap();
            fs::write(source.path().join("a.txt"), "Candidate replacement text.").unwrap();
            fs::write(source.path().join("z.txt"), "Candidate new document.").unwrap();
            if changed_recipe {
                engines.recipe.model.push_str("-changed");
            }
            let mut checked = 0;
            let mut concurrent = None;
            reindex(
                data.path(),
                source.path(),
                &engines,
                &assets,
                |_| {
                    assert_eq!(
                        fs::read(data.path().join("active-index.json")).unwrap(),
                        pointer
                    );
                    let reader = crate::storage::State::open(&resolve(data.path())).unwrap();
                    assert_eq!(reader.generations().unwrap(), generations);
                    assert_eq!(reader.documents().unwrap().len(), 1);
                    assert_eq!(
                        reader.documents().unwrap()[0].chunks[0].text,
                        "Published original text."
                    );
                    assert_eq!(
                        crate::storage::State::review_documents(&resolve(data.path()))
                            .unwrap()
                            .len(),
                        1
                    );
                    assert_eq!(
                        last_indexed_folder(data.path()).as_deref(),
                        source.path().canonicalize().unwrap().to_str()
                    );
                    checked += 1;
                    drop(concurrent.take());
                    concurrent = Some(reader);
                    // Release before the next callback opens another State.
                    if checked == 1 {
                        concurrent = None;
                    }
                },
                || false,
            )
            .await
            .unwrap();
            assert_eq!(checked, 2);
            let mut original = concurrent.unwrap();
            assert_eq!(
                original.documents().unwrap()[0].chunks[0].text,
                "Published original text."
            );
            let (state, _) =
                crate::storage::State::open_index(data.path(), &engines.recipe).unwrap();
            assert_ne!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                pointer
            );
            assert_eq!(state.documents().unwrap().len(), 2);
            assert_eq!(state.generations().unwrap().len(), 2);
            assert_eq!(
                crate::storage::State::review_documents(&state.root)
                    .unwrap()
                    .len(),
                2
            );
            assert!(!state
                .search("Candidate", &engines.vector("Candidate", true).unwrap(), 3)
                .await
                .unwrap()
                .is_empty());
            drop(state);
            let published = fs::read(data.path().join("active-index.json")).unwrap();
            let error =
                crate::index::index(&mut original, source.path(), &engines, &assets, |_| {})
                    .await
                    .unwrap_err();
            assert!(error.to_string().contains("index selection changed"));
            assert_eq!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                published
            );
            assert_eq!(original.documents().unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn explicit_candidate_is_invisible_to_review_readers_between_files() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.txt"), "Original published source.").unwrap();
        let mut state = crate::storage::State::open(data.path()).unwrap();
        crate::index::index(&mut state, source.path(), &engines, &assets, |_| {})
            .await
            .unwrap();
        let generations = state.generations().unwrap();
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        fs::write(source.path().join("a.txt"), "Unpublished replacement.").unwrap();
        fs::write(source.path().join("z.txt"), "Unpublished extra document.").unwrap();
        let mut checks = 0;
        crate::index::index(&mut state, source.path(), &engines, &assets, |_| {
            assert_eq!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                pointer
            );
            let selected = resolve_state(data.path());
            let documents = crate::storage::State::review_documents(&selected).unwrap();
            assert_eq!(documents.len(), 1);
            assert_eq!(documents[0].chunks[0].text, "Original published source.");
            assert_eq!(
                crate::storage::State::review_selected_folder(&selected)
                    .unwrap()
                    .as_deref(),
                Some(source.path().canonicalize().unwrap().as_path())
            );
            checks += 1;
        })
        .await
        .unwrap();
        assert_eq!(checks, 2);
        assert_ne!(state.generations().unwrap(), generations);
        assert_ne!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        drop(state);
        let (state, _) = crate::storage::State::open_index(data.path(), &engines.recipe).unwrap();
        assert_eq!(state.documents().unwrap().len(), 2);
        assert_eq!(
            crate::storage::State::review_documents(&state.root)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(state.generations().unwrap().len(), 2);
        assert!(!state
            .search(
                "Unpublished",
                &engines.vector("Unpublished", true).unwrap(),
                3
            )
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn explicit_different_missing_source_preserves_selection() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("good.txt"), "Retained current source.").unwrap();
        let mut state = crate::storage::State::open(data.path()).unwrap();
        crate::index::index(&mut state, source.path(), &engines, &assets, |_| {})
            .await
            .unwrap();
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        let generations = state.generations().unwrap();
        let error = crate::index::index(
            &mut state,
            &source.path().join("different-missing"),
            &engines,
            &assets,
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("source unavailable"));
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        assert_eq!(state.generations().unwrap(), generations);
        assert_eq!(state.documents().unwrap().len(), 1);
        assert_eq!(
            last_indexed_folder(data.path()).as_deref(),
            source.path().canonicalize().unwrap().to_str()
        );
        drop(state);
        let (state, _) = crate::storage::State::open_index(data.path(), &engines.recipe).unwrap();
        assert_eq!(state.generations().unwrap(), generations);
        assert_eq!(
            crate::storage::State::review_documents(&state.root)
                .unwrap()
                .len(),
            1
        );
        assert!(!state
            .search("Retained", &engines.vector("Retained", true).unwrap(), 3)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn post_rename_failures_keep_published_documents_and_review_source() {
        let (mut engines, assets) = embedding_fixture();
        for step in ["directory sync", "generation cleanup"] {
            let data = tempfile::tempdir().unwrap();
            let a = tempfile::tempdir().unwrap();
            let b = tempfile::tempdir().unwrap();
            fs::write(a.path().join("good.txt"), "Previous selection.").unwrap();
            fs::write(b.path().join("good.txt"), "Published replacement.").unwrap();
            // Reuse a recipe candidate containing A, so rollback would visibly
            // restore A's rows/metadata after B's pointer has been published.
            reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
                .await
                .unwrap();
            let candidate = resolve(data.path());
            engines.recipe.model.push_str("-interim");
            reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
                .await
                .unwrap();
            engines
                .recipe
                .model
                .truncate(engines.recipe.model.len() - "-interim".len());
            FAULTS.with(|faults| faults.borrow_mut().push((data.path().to_owned(), step)));
            let error = reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
                .await
                .unwrap_err();
            FAULTS.with(|faults| faults.borrow_mut().clear());
            assert!(
                error.to_string().starts_with("Index published;"),
                "{error:#}"
            );
            assert!(!error.to_string().contains("previous folder"));
            let published = resolve(data.path());
            assert_ne!(
                published, candidate,
                "same-recipe scans publish a fresh snapshot"
            );
            assert_eq!(
                last_indexed_folder(&published).as_deref(),
                b.path().canonicalize().unwrap().to_str()
            );
            let docs = crate::storage::State::review_documents(&published).unwrap();
            assert_eq!(docs.len(), 1);
            assert!(Path::new(&docs[0].path).starts_with(b.path().canonicalize().unwrap()));
            let state = crate::storage::State::open(&resolve_state(data.path())).unwrap();
            assert_eq!(
                state.documents().unwrap()[0].chunks[0].text,
                "Published replacement."
            );
            assert!(!state
                .search("Published", &engines.vector("Published", true).unwrap(), 3)
                .await
                .unwrap()
                .is_empty());
        }
    }

    #[tokio::test]
    async fn failed_scan_keeps_original_error_when_cleanup_also_fails() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        FAULTS.with(|faults| {
            faults
                .borrow_mut()
                .push((data.path().to_owned(), "generation cleanup"))
        });
        let error = reindex(
            data.path(),
            &data.path().join("missing"),
            &engines,
            &assets,
            |_| {},
            || false,
        )
        .await
        .unwrap_err();
        FAULTS.with(|faults| faults.borrow_mut().clear());
        let chain = format!("{error:#}");
        assert!(chain.contains("source missing"), "{chain}");
        assert!(
            chain.contains("injected generation cleanup failure"),
            "{chain}"
        );
        assert!(!chain.contains("Index published"));
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn missing_source_tail_uses_volume_case_semantics() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("CaseParent");
        fs::create_dir(&parent).unwrap();
        let alternate = temp.path().join("cASEpARENT");
        let parent = parent.canonicalize().unwrap();
        let insensitive = alternate.canonicalize().ok() == parent.canonicalize().ok();
        assert_eq!(
            same_source(&parent.join("DOCS"), &parent.join("docs")),
            insensitive
        );
        assert!(!same_source(&parent.join("other"), &parent.join("docs")));
    }

    #[tokio::test]
    async fn refused_parent_switch_publishes_only_failed_path_revocation() {
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
        assert!(docs[0].chunks[0].text.contains("Original revision"));
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
        fs::write(
            root.join("recipe.json"),
            serde_json::to_vec(&engines.recipe).unwrap(),
        )
        .unwrap();
        activate(data.path(), &engines.recipe).unwrap();
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
