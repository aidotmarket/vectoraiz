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

/// Review validation must never recreate a lock removed since capture.
pub fn existing_selection_fence(data: &Path) -> Result<SelectionFence> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(data.join("index-selection.lock"))?;
    fs2::FileExt::try_lock_shared(&file)
        .context("native index is in use; retry after indexing/search")?;
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
        if let Some(data) = path
            .parent()
            .filter(|p| p.join("active-index.json").is_file())
        {
            return data.to_owned();
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
    static CANCEL_HOOK: std::cell::RefCell<Option<(PathBuf, &'static str, ScanControl)>> = const { std::cell::RefCell::new(None) };
}

fn publication_step(_data: &Path, _step: &'static str) -> Result<()> {
    #[cfg(any(test, all(debug_assertions, feature = "test-support")))]
    if std::env::var("VZ_PUBLICATION_CRASH_STEP").ok().as_deref() == Some(_step) {
        if let Some(ready) = std::env::var_os("VZ_PUBLICATION_READY") {
            fs::write(ready, _step)?;
            loop {
                if std::env::var_os("VZ_PUBLICATION_RELEASE")
                    .is_some_and(|path| Path::new(&path).exists())
                {
                    anyhow::bail!("injected pre-commit publication failure");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
    #[cfg(test)]
    CANCEL_HOOK.with(|hook| {
        if let Some((data, step, control)) = &*hook.borrow() {
            if data == _data && *step == _step {
                ensure!(
                    !control.cancel(),
                    "cancel accepted after entering publication"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })?;
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
    activate_with(
        data,
        recipe,
        &publication_dir(data, recipe)?,
        || Ok(()),
        &mut Default::default(),
    )
}

fn activate_with(
    data: &Path,
    recipe: &Recipe,
    relative: &str,
    before_rename: impl FnOnce() -> Result<()>,
    stats: &mut crate::index::Progress,
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
    let workspace = pointer_workspace(data, true)?;
    let temp = workspace.join(format!("pointer-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec(&pointer)?)?;
        file.sync_all()?;
        drop(file);
        before_rename()?;
        publication_step(data, "before pointer rename")?;
        fs::rename(&temp, data.join("active-index.json"))?;
        defer_cleanup(
            stats,
            data,
            "publication",
            publication_step(data, "after pointer rename"),
        );
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
            let tail = absolute.strip_prefix(ancestor).ok()?;
            return Some(if tail.as_os_str().is_empty() {
                parent
            } else {
                parent.join(tail)
            });
        }
    }
    None
}

// Compare the persisted spelling component by component. Never resolve or stat
// historical ancestors: a former parent may now be a symlink to another source.
// Only the request's containing directory supplies read-only case information.
pub(crate) fn same_source(current: &Path, previous: &Path) -> bool {
    let mut parent = PathBuf::new();
    let mut previous = previous.components();
    for component in current.components() {
        let Some(historical) = previous.next() else {
            return false;
        };
        if component != historical
            && !(matches!(component, Component::Normal(_))
                && matches!(historical, Component::Normal(_))
                && equivalent_missing_tail(
                    Path::new(component.as_os_str()),
                    Path::new(historical.as_os_str()),
                )
                && directory_is_case_insensitive(&parent))
        {
            return false;
        }
        parent.push(component.as_os_str());
    }
    previous.next().is_none()
}

fn directory_is_case_insensitive(directory: &Path) -> bool {
    // Case rules belong to this exact containing directory. A surviving
    // ancestor cannot tell us the rules of a removed case-sensitive child.
    if !directory.is_dir() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(directory.as_os_str().as_bytes()) else {
            return false;
        };
        unsafe { libc::pathconf(path.as_ptr(), libc::_PC_CASE_SENSITIVE) == 0 }
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            FileCaseSensitiveInfo, GetFileInformationByHandleEx, GetVolumeInformationByHandleW,
            FILE_CASE_SENSITIVE_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        };
        let Ok(handle) = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(directory)
        else {
            return false;
        };
        let mut info = FILE_CASE_SENSITIVE_INFO { Flags: 0 };
        unsafe {
            if GetFileInformationByHandleEx(
                handle.as_raw_handle(),
                FileCaseSensitiveInfo,
                (&mut info as *mut FILE_CASE_SENSITIVE_INFO).cast(),
                std::mem::size_of_val(&info) as u32,
            ) != 0
            {
                return info.Flags & 1 == 0; // FILE_CS_FLAG_CASE_SENSITIVE_DIR
            }
            // Older Windows/filesystems lack per-directory case information.
            // FILE_CASE_SENSITIVE_SEARCH means the volume supports sensitive
            // lookup, not that this directory enables it. Only use a successful
            // volume query for the default Windows insensitive semantics.
            let mut flags = 0;
            GetVolumeInformationByHandleW(
                handle.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                std::ptr::null_mut(),
                0,
            ) != 0
                && flags & 1 == 0
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = directory;
        false
    }
}

fn equivalent_missing_tail(current: &Path, previous: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
        let current: PathBuf = current.components().collect();
        let previous: PathBuf = previous.components().collect();
        let current: Vec<u16> = current.as_os_str().encode_wide().collect();
        let previous: Vec<u16> = previous.as_os_str().encode_wide().collect();
        let (Ok(current_len), Ok(previous_len)) =
            (i32::try_from(current.len()), i32::try_from(previous.len()))
        else {
            return false;
        };
        // NTFS ordinal case comparison does not normalize Unicode spellings.
        unsafe {
            CompareStringOrdinal(
                current.as_ptr(),
                current_len,
                previous.as_ptr(),
                previous_len,
                1,
            ) == CSTR_EQUAL
        }
    }
    #[cfg(not(windows))]
    {
        let fold = |path: &Path| {
            #[cfg(target_os = "macos")]
            {
                use unicode_normalization::UnicodeNormalization;
                path.to_string_lossy()
                    .nfd()
                    .collect::<String>()
                    .to_lowercase()
            }
            #[cfg(not(target_os = "macos"))]
            path.to_string_lossy().to_lowercase()
        };
        fold(current) == fold(previous)
    }
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
        return if outcome.stats.infrastructure_failed == 0 {
            ScanDecision::Publish
        } else {
            ScanDecision::RevokeSource
        };
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
    let relative = format!("indexes/{id}-{}", uuid::Uuid::new_v4());
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

#[derive(Clone, Default)]
pub struct ScanControl(std::sync::Arc<std::sync::Mutex<ScanPhase>>);
#[derive(Default)]
struct ScanPhase {
    phase: Phase,
    cancelled: bool,
}
#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Scanning,
    Committing,
    Done,
}
impl ScanControl {
    /// True means cancellation was accepted and publication cannot start.
    pub fn cancel(&self) -> bool {
        let mut state = self.0.lock().unwrap();
        if state.phase != Phase::Scanning {
            return false;
        }
        state.cancelled = true;
        true
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.lock().unwrap().cancelled
    }
    fn commit(&self) -> Result<()> {
        let mut state = self.0.lock().unwrap();
        ensure!(!state.cancelled, "Indexing cancelled");
        state.phase = Phase::Committing;
        Ok(())
    }
    fn complete(&self) -> Result<()> {
        let mut state = self.0.lock().unwrap();
        ensure!(!state.cancelled, "Indexing cancelled");
        state.phase = Phase::Done;
        Ok(())
    }
    fn finish(&self) {
        self.0.lock().unwrap().phase = Phase::Done;
    }
}
struct ScanCompletion(ScanControl);
impl Drop for ScanCompletion {
    fn drop(&mut self) {
        self.0.finish();
    }
}

fn writer_fence(root: &Path, create: bool) -> Result<SelectionFence> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create(create)
        .truncate(false)
        .open(root.join("writer.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file).context("index in use; retry after indexing/search")?;
    Ok(SelectionFence(file))
}

fn remove_unreferenced(
    root: &Path,
    db: &rusqlite::Connection,
    stats: &mut crate::index::Progress,
) -> Result<()> {
    let referenced: std::collections::HashSet<String> = db
        .prepare("SELECT generation FROM documents")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    crate::storage::recover_generations(root, &referenced, |warning| {
        cleanup_warning(stats, warning)
    });
    Ok(())
}

fn cleanup_warning(stats: &mut crate::index::Progress, warning: String) {
    if !stats.seal_warnings.contains(&warning) {
        eprintln!("{warning}");
        stats.seal_warnings.push(warning);
    }
}

fn defer_cleanup(stats: &mut crate::index::Progress, root: &Path, kind: &str, result: Result<()>) {
    if let Err(error) = result {
        cleanup_warning(
            stats,
            format!("Deferred {kind} cleanup in {}: {error:#}", root.display()),
        );
    }
}

const SCAN_FILES: &[&str] = &[
    "scan.sqlite-journal",
    "scan.sqlite-wal",
    "scan.sqlite-shm",
    "scan-writer.lock",
    "scan.sqlite",
];

fn remove_cleanup_node(path: &Path) -> std::io::Result<()> {
    match crate::storage::remove_index_node(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn remove_scan_files(root: &Path) -> Result<()> {
    for name in SCAN_FILES {
        let path = root.join(name);
        match fs::remove_file(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            result => result.with_context(|| path.display().to_string())?,
        }
    }
    Ok(())
}

fn remove_directory_contents(root: &Path, markers: &[&str]) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !markers.iter().any(|name| entry.file_name() == *name) {
            remove_cleanup_node(&entry.path())
                .with_context(|| entry.path().display().to_string())?;
        }
    }
    Ok(())
}

fn cleanup_scan_workspace(root: &Path, stats: &mut crate::index::Progress) {
    let result = (|| -> Result<()> {
        if !root.try_exists()? {
            return Ok(());
        }
        crate::generation_seal::ordinary_directory(root)?;
        remove_directory_contents(root, SCAN_FILES)?;
        remove_scan_files(root)?;
        fs::remove_dir(root)?;
        Ok(())
    })();
    defer_cleanup(stats, root, "private workspace", result);
}

// Names alone are not ownership: require an ordinary recipe file with its
// content-derived id, and a real index/workspace database plus existing lock.
fn owned_database(root: &Path, name: &str, table: &str) -> bool {
    let path = root.join(name);
    if !path.symlink_metadata().is_ok_and(|m| m.is_file()) {
        return false;
    }
    let connection = if name == "state.sqlite" {
        crate::storage::State::review_connection(root)
    } else {
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(anyhow::Error::from)
    };
    connection.ok().is_some_and(|db| {
        db.prepare(&format!(
            "SELECT doc_id,path,generation FROM {table} LIMIT 0"
        ))
        .is_ok()
    })
}

fn owned_legacy_directory(root: &Path) -> bool {
    root.symlink_metadata()
        .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        && root
            .join("writer.lock")
            .symlink_metadata()
            .is_ok_and(|m| m.is_file())
        && root
            .join("state.sqlite")
            .symlink_metadata()
            .is_ok_and(|m| m.is_file())
        && root
            .join("recipe.json")
            .symlink_metadata()
            .is_ok_and(|m| m.is_file())
        && owned_database(root, "state.sqlite", "documents")
        && fs::read(root.join("recipe.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Recipe>(&b).ok())
            .is_some()
}

fn owned_recipe_directory(data: &Path, root: &Path) -> bool {
    let Ok(relative) = root.strip_prefix(data) else {
        return false;
    };
    if !root
        .join("recipe.json")
        .symlink_metadata()
        .is_ok_and(|m| m.is_file())
    {
        return false;
    }
    let Ok(bytes) = fs::read(root.join("recipe.json")) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    if serde_json::from_value::<Recipe>(value.clone()).is_err() {
        return false;
    }
    let Ok(id) = canonical_id(value) else {
        return false;
    };
    valid_directory(&relative.to_string_lossy().replace('\\', "/"), &id)
        && root
            .canonicalize()
            .ok()
            .zip(data.canonicalize().ok())
            .is_some_and(|(root, data)| root.parent() == Some(data.join("indexes").as_path()))
        && root
            .join("writer.lock")
            .symlink_metadata()
            .is_ok_and(|m| m.is_file())
        && (owned_database(root, "state.sqlite", "documents")
            || owned_database(root, "scan.sqlite", "scan_documents"))
}

// A recovery record is tied to the canonical root and its retained recipe.
// The database digest records ownership evidence before destructive cleanup.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FlatCleanup {
    root: PathBuf,
    recipe: Recipe,
    database_sha256: String,
}

// Only traverse the real, direct workspace child. read_dir/create_dir_all by
// themselves follow a symlink and could stage or unlink outside the library.
fn pointer_workspace(data: &Path, create: bool) -> Result<PathBuf> {
    let root = data.canonicalize()?;
    let workspace = root.join(".vz-scan");
    if create && !workspace.try_exists()? {
        fs::create_dir(&workspace)?;
    }
    let metadata = fs::symlink_metadata(&workspace)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "invalid scan workspace"
    );
    ensure!(
        workspace.canonicalize()? == workspace,
        "scan workspace outside library"
    );
    Ok(workspace)
}
fn valid_flat_cleanup(root: &Path) -> bool {
    (|| -> Result<bool> {
        let marker = root.join(".vz-flat-cleanup");
        let metadata = marker.symlink_metadata()?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "cleanup marker collision"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(metadata.nlink() == 1, "cleanup marker link collision");
        }
        let record: FlatCleanup = serde_json::from_slice(&fs::read(marker)?)?;
        ensure!(
            root.join("recipe.json").symlink_metadata()?.is_file(),
            "cleanup recipe missing"
        );
        let recipe: Recipe = serde_json::from_slice(&fs::read(root.join("recipe.json"))?)?;
        Ok(record.root == root.canonicalize()?
            && record.recipe == recipe
            && record.database_sha256.len() == 64
            && record
                .database_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            && (!root.join("state.sqlite").try_exists()?
                || (owned_database(root, "state.sqlite", "documents")
                    && crate::pipeline::file_sha256(&root.join("state.sqlite"))?
                        == record.database_sha256)))
    })()
    .unwrap_or(false)
}

/// Run under the selection fence at application/scan start and after publication.
/// Reader-pinned indexes are retried on the next invocation.
pub fn cleanup_library(data: &Path, keep: Option<&Path>) -> Result<()> {
    // Preserve recoverable snapshots when the selection is damaged. A later
    // successful pointer publication will make reclamation safe again.
    if needs_pointer_repair(data) {
        return Ok(());
    }
    let active = resolve_state(data);
    let active = active.canonicalize().unwrap_or(active);
    if let Ok(workspace) = pointer_workspace(data, false) {
        for entry in fs::read_dir(workspace)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name
                .strip_prefix("pointer-")
                .and_then(|s| s.strip_suffix(".tmp"))
                .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok_and(|u| u.to_string() == s))
                && entry.path().symlink_metadata()?.is_file()
            {
                fs::remove_file(entry.path())?;
            }
        }
    }
    let mut roots = Vec::new();
    match fs::read_dir(data.join("indexes")) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_dir() && owned_recipe_directory(data, &entry.path()) {
                    roots.push(entry.path());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if owned_legacy_directory(&data.join("phase0-index")) {
        roots.push(data.join("phase0-index"));
    }
    if owned_legacy_directory(data) || valid_flat_cleanup(data) {
        roots.push(data.to_owned());
    }
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let canonical = root.canonicalize()?;
        if canonical == active || keep.is_some_and(|p| p == root || p == canonical) {
            continue;
        }
        let Ok(lock) = writer_fence(&root, false) else {
            continue;
        };
        if root == data {
            // Durable discovery survives removal of state.sqlite and any later
            // cleanup error or process death. Keep the recipe until the record is retired.
            let marker = root.join(".vz-flat-cleanup");
            match marker.symlink_metadata() {
                Ok(_) => ensure!(
                    valid_flat_cleanup(&root),
                    "flat cleanup marker collision; cleanup refused"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    ensure!(
                        owned_legacy_directory(&root),
                        "flat cleanup ownership unavailable"
                    );
                    let record = FlatCleanup {
                        root: root.canonicalize()?,
                        recipe: serde_json::from_slice(&fs::read(root.join("recipe.json"))?)?,
                        database_sha256: crate::pipeline::file_sha256(&root.join("state.sqlite"))?,
                    };
                    let mut file =
                        tempfile::NamedTempFile::new_in(pointer_workspace(&root, true)?)?;
                    file.write_all(&serde_json::to_vec(&record)?)?;
                    file.as_file().sync_all()?;
                    publication_step(data, "flat marker install")?;
                    // Atomic no-clobber installation preserves any collision.
                    file.persist_noclobber(&marker)?;
                }
                Err(error) => return Err(error.into()),
            }
            #[cfg(unix)]
            fs::File::open(&root)?.sync_all()?;
            for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"] {
                let path = root.join(name);
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
            publication_step(data, "flat database removal")?;
            for name in ["generations", ".vz-scan", ".vz-validation"] {
                let path = root.join(name);
                match crate::storage::remove_index_node(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            fs::remove_file(marker)?;
            fs::remove_file(root.join("recipe.json"))?;
        } else {
            // Keep ownership evidence until all content has been reclaimed.
            let markers = [
                "scan.sqlite-journal",
                "scan.sqlite-wal",
                "scan.sqlite-shm",
                "state.sqlite-journal",
                "state.sqlite-wal",
                "state.sqlite-shm",
                "scan.sqlite",
                "state.sqlite",
                "writer.lock",
                "recipe.json",
            ];
            remove_directory_contents(&root, &markers)?;
            // Windows requires the lock handle closed before deleting its directory.
            drop(lock);
            for name in markers {
                remove_cleanup_node(&root.join(name))?;
            }
            fs::remove_dir(&root)?;
        }
    }
    Ok(())
}

/// Preserve one recoverable candidate at startup. The next requested scan
/// narrows eligibility to its single resolved source identity and recipe.
pub fn cleanup_start(data: &Path) -> Result<()> {
    let keep = fs::read_dir(data.join("indexes"))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| owned_recipe_directory(data, p))
        .filter(|p| p.join("scan.sqlite").is_file())
        .max_by_key(|p| {
            fs::metadata(p.join("scan.sqlite"))
                .and_then(|m| m.modified())
                .ok()
        });
    let workspace = (|| -> Result<()> {
        if let Some(root) = published_candidate(data) {
            if let Ok(_lock) = writer_fence(&root, false) {
                remove_scan_files(&root)?;
            }
        }
        Ok(())
    })();
    cleanup_library(data, keep.as_deref()).and(workspace)
}

fn published_candidate(data: &Path) -> Option<PathBuf> {
    let root = verified_target(data)?;
    (root.parent()? == data.canonicalize().ok()?.join("indexes")).then_some(root)
}

fn retry_candidate(
    data: &Path,
    source: &str,
    recipe: &str,
    active: &Path,
) -> Option<(String, PathBuf)> {
    for entry in fs::read_dir(data.join("indexes")).ok()?.flatten() {
        let root = entry.path();
        if !owned_recipe_directory(data, &root) {
            continue;
        }
        if root.canonicalize().ok() == active.canonicalize().ok() {
            continue;
        }
        let matches = (|| -> Result<bool> {
            let db = rusqlite::Connection::open_with_flags(
                root.join("scan.sqlite"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            Ok(db.query_row(
                "SELECT source_identity,recipe_id FROM scan_state",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )? == (source.into(), recipe.into()))
        })()
        .unwrap_or(false);
        if matches {
            // Crash-left committed candidates get a fresh name before retry,
            // preserving successes without ever republishing a former target.
            if root.join("state.sqlite").exists() {
                let probe = writer_fence(&root, false).ok()?;
                let stored: Recipe =
                    serde_json::from_slice(&fs::read(root.join("recipe.json")).ok()?).ok()?;
                let (relative, fresh) = candidate_dir(data, &stored).ok()?;
                fs::remove_dir(&fresh).ok()?;
                drop(probe);
                fs::rename(&root, &fresh).ok()?;
                return Some((relative, fresh));
            }
            return Some((format!("indexes/{}", entry.file_name().to_str()?), root));
        }
    }
    None
}

#[derive(Default)]
pub(crate) struct ScanOptions<'a> {
    pub explicit: Option<&'a crate::storage::State>,
    pub flat: bool,
    pub control: Option<&'a ScanControl>,
}

/// Scan reads only committed published content and writes only private retry work.
pub(crate) async fn scan_candidate(
    data: &Path,
    folder: &Path,
    engines: &crate::engine::Engines,
    assets: &Path,
    progress: impl FnMut(&crate::index::Progress),
    cancelled: impl Fn() -> bool,
    options: ScanOptions<'_>,
) -> Result<crate::index::Progress> {
    let ScanOptions {
        explicit,
        flat,
        control,
    } = options;
    use crate::{
        index::{ScanRow, ScanWorkspace},
        storage::State,
    };
    let local = ScanControl::default();
    let control = control.unwrap_or(&local);
    let _completion = ScanCompletion(control.clone());
    ensure!(!cancelled(), "Indexing cancelled");
    fs::create_dir_all(data)?;
    let _selection = if flat {
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
    let identity = source_identity(folder).context("source identity unavailable")?;
    let source = identity.to_str().context("UTF-8 source")?;
    let stored = fs::read(active.join("recipe.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Recipe>(&b).ok());
    let same_recipe = (stored.as_ref() == Some(&engines.recipe) || (flat && stored.is_none()))
        && !needs_pointer_repair(data);
    ensure!(
        same_recipe || !flat || is_library_root(data),
        "embedding recipe mismatch; re-index required"
    );
    let id = recipe_id(&engines.recipe)?;
    let candidate = if same_recipe {
        None
    } else {
        Some(match retry_candidate(data, source, &id, &active) {
            Some(c) => c,
            None => candidate_dir(data, &engines.recipe)?,
        })
    };
    let root = candidate.as_ref().map(|c| c.1.as_path()).unwrap_or(&active);
    let _writer = if explicit.is_none() {
        Some(writer_fence(root, true)?)
    } else {
        None
    };
    if same_recipe {
        State::quarantine_orphan_wal(&active)?;
    } else if active.join("state.sqlite-wal").try_exists()?
        && !active.join("state.sqlite-shm").try_exists()?
    {
        // Recipe changes must also recover the old publication before cleanup.
        // Take its writer fence only for an orphan WAL; held readers otherwise
        // keep the existing recipe-change publication semantics.
        let _recovery_writer = writer_fence(&active, true)?;
        State::quarantine_orphan_wal(&active)?;
    }
    let mut cleanup_report = crate::index::Progress::default();
    if is_library_root(data) {
        defer_cleanup(
            &mut cleanup_report,
            data,
            "library",
            cleanup_library(data, candidate.as_ref().map(|c| c.1.as_path())),
        );
    }
    if same_recipe && published_candidate(data).as_deref() == Some(root) {
        defer_cleanup(
            &mut cleanup_report,
            root,
            "published workspace",
            remove_scan_files(root),
        );
    }
    let previous = last_indexed_folder(&active);
    let current = previous
        .as_deref()
        .is_some_and(|p| same_source(&identity, Path::new(p)));
    let populated = State::has_persisted_documents(&active).unwrap_or(true);
    let published = if same_recipe && active.join("state.sqlite").exists() {
        {
            let db = State::review_connection(&active)?;
            let cleanup = remove_unreferenced(&active, &db, &mut cleanup_report);
            defer_cleanup(&mut cleanup_report, &active, "generation", cleanup);
            ScanRow::read(&db, "documents")?
        }
    } else {
        vec![]
    };
    let workspace_root = if same_recipe {
        if root.join(".vz-scan").symlink_metadata().is_ok() {
            pointer_workspace(root, false)?;
        }
        root.join(".vz-scan")
    } else {
        root.to_owned()
    };
    let mut workspace =
        ScanWorkspace::new(&workspace_root, &active, published.clone(), source, &id)?;
    if !same_recipe {
        if let Ok(db) = State::review_connection(&active) {
            if let Ok(rows) = ScanRow::read(&db, "documents") {
                workspace.set_baseline(rows);
            }
        }
        crate::index::initialize_recipe(root, &engines.recipe, || Ok(()))?;
        workspace.initialize()?;
    }
    let outcome = {
        let run = crate::index::scan(&mut workspace, &identity, root, engines, assets, progress);
        let mut run = Box::pin(run);
        futures::future::poll_fn(|cx| {
            if cancelled() {
                std::task::Poll::Ready(Err(anyhow::anyhow!("Indexing cancelled")))
            } else {
                run.as_mut().poll(cx)
            }
        })
        .await
    };
    let mut outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(if !workspace.staged()?.is_empty() {
                error.context("Indexing cancelled or interrupted; Re-index to resume")
            } else {
                error
            })
        }
    };
    outcome
        .stats
        .seal_warnings
        .extend(cleanup_report.seal_warnings);
    ensure!(
        !cancelled(),
        if workspace.staged()?.is_empty() {
            "Indexing cancelled"
        } else {
            "Indexing cancelled; Re-index to resume"
        }
    );
    if outcome.source.as_deref().is_some_and(|path| !path.is_dir()) {
        outcome.unavailable = true;
    }
    let decision = decide_scan(&outcome, current, populated);
    // Enter the phase boundary only for an actual publication write set.
    match decision {
        ScanDecision::Publish => {
            let staged = workspace.staged()?;
            let changed = !staged.is_empty()
                || !outcome.jobs.is_empty()
                || published.iter().any(|r| {
                    !outcome.succeeded.contains(&r.path) && !outcome.retain.contains(&r.path)
                })
                || outcome
                    .unchanged
                    .iter()
                    .any(|r| !workspace.matches_published(r))
                || previous.as_deref() != Some(source)
                || candidate.is_some();
            if changed {
                if same_recipe {
                    if cancelled() {
                        control.cancel();
                    }
                    control.commit()?;
                    publication_step(data, "commit entered")?;
                }
                crate::index::initialize_recipe(root, &engines.recipe, || Ok(()))?;
                crate::storage::create_generations_if_missing(root)?;
                if !staged.is_empty() {
                    crate::generation_seal::ordinary_directory(&root.join("generations"))?;
                }
                for row in &staged {
                    let from = workspace_root.join("generations").join(&row.generation);
                    let to = root.join("generations").join(&row.generation);
                    if from != to {
                        fs::rename(&from, &to)?;
                        if let Err(error) =
                            crate::generation_seal::install(&workspace_root, root, &row.generation)
                        {
                            outcome.stats.seal_warning(Path::new(&row.path), &error);
                        }
                    }
                }
                #[cfg(unix)]
                if !staged.is_empty() {
                    fs::File::open(root.join("generations"))?.sync_all()?;
                }
                publication_step(data, "generation install")?;
                let db = State::publication_connection(root)?;
                let tx = db.unchecked_transaction()?;
                State::initialize_schema(&tx)?;
                tx.execute("UPDATE jobs SET status='failed',reason='interrupted before completion' WHERE status='running'",[])?;
                for row in ScanRow::read(&tx, "documents")? {
                    if !outcome.succeeded.contains(&row.path) && !outcome.retain.contains(&row.path)
                    {
                        tx.execute("DELETE FROM documents WHERE path=?1", [&row.path])?;
                    }
                }
                for row in staged.iter().chain(&outcome.unchanged) {
                    row.put(&tx)?;
                }
                for (path, status, reason) in &outcome.jobs {
                    tx.execute(
                        "INSERT INTO jobs(path,status,reason) VALUES(?1,?2,?3)",
                        rusqlite::params![path, status, reason],
                    )?;
                }
                tx.execute("INSERT INTO index_metadata VALUES('selected_folder',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[source])?;
                publication_step(data, "before sqlite commit")?;
                tx.commit()?;
                let after_commit = publication_step(data, "after sqlite commit");
                if same_recipe {
                    defer_cleanup(&mut outcome.stats, root, "publication", after_commit);
                } else {
                    after_commit?;
                }
                let checkpoint = db
                    .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                    .map_err(Into::into);
                if same_recipe {
                    defer_cleanup(&mut outcome.stats, root, "WAL checkpoint", checkpoint);
                } else {
                    checkpoint?;
                }
                if let Some((relative, _)) = &candidate {
                    activate_with(
                        data,
                        &engines.recipe,
                        relative,
                        || {
                            if cancelled() {
                                control.cancel();
                            }
                            control
                                .commit()
                                .context("Indexing cancelled; Re-index to resume")?;
                            publication_step(data, "commit entered")
                        },
                        &mut outcome.stats,
                    )?;
                }
                let cleanup = publication_step(data, "generation cleanup")
                    .and_then(|()| remove_unreferenced(root, &db, &mut outcome.stats));
                defer_cleanup(&mut outcome.stats, root, "generation", cleanup);
                drop(db);
                drop(workspace.state.take());
                if same_recipe {
                    cleanup_scan_workspace(&workspace_root, &mut outcome.stats);
                } else {
                    defer_cleanup(
                        &mut outcome.stats,
                        root,
                        "published workspace",
                        remove_scan_files(root),
                    );
                }
                if is_library_root(data) {
                    defer_cleanup(
                        &mut outcome.stats,
                        data,
                        "library",
                        cleanup_library(data, None),
                    );
                }
            } else if same_recipe && workspace_root.exists() {
                drop(workspace.state.take());
                cleanup_scan_workspace(&workspace_root, &mut outcome.stats);
            }
            if !outcome.seal_updates.is_empty() {
                if !changed {
                    if cancelled() {
                        control.cancel();
                    }
                    control.commit()?;
                }
                for (path, seal) in outcome.seal_updates {
                    if let Err(error) = seal.write(root) {
                        outcome.stats.seal_warning(Path::new(&path), &error);
                    }
                }
            }
        }
        ScanDecision::RevokeSource | ScanDecision::RevokeFailedPaths => {
            if active.join("state.sqlite").exists() {
                let db = State::review_connection(&active)?;
                let rows = ScanRow::read(&db, "documents")?;
                drop(db);
                let revoked: std::collections::HashSet<_> = rows
                    .iter()
                    .filter(|r| {
                        if outcome.retain.contains(&r.path) {
                            return false;
                        }
                        if decision == ScanDecision::RevokeFailedPaths {
                            outcome.damaged.contains(&r.path)
                                || outcome.failed_content.contains(&r.path)
                                || outcome.failed_infrastructure.contains(&r.path)
                        } else {
                            outcome.unavailable
                                || outcome.damaged.contains(&r.path)
                                || !outcome.seen.contains(&r.path)
                                || outcome.failed_content.contains(&r.path)
                                || (outcome.changed.contains(&r.path)
                                    && outcome.failed_infrastructure.contains(&r.path))
                        }
                    })
                    .map(|r| r.path.clone())
                    .collect();
                if !revoked.is_empty() {
                    let _old_lock = if same_recipe || explicit.is_some() {
                        None
                    } else {
                        Some(
                            writer_fence(&active, false)
                                .context("revocation pending; re-index to apply")?,
                        )
                    };

                    if cancelled() {
                        control.cancel();
                    }
                    control.commit()?;
                    publication_step(data, "commit entered")?;
                    let db = State::publication_connection(&active)?;
                    let tx = db.unchecked_transaction()?;
                    State::initialize_schema(&tx)?;
                    for path in &revoked {
                        tx.execute("DELETE FROM documents WHERE path=?1", [path])?;
                    }
                    for (path, status, reason) in &outcome.jobs {
                        if revoked.contains(path) {
                            tx.execute(
                                "INSERT INTO jobs(path,status,reason) VALUES(?1,?2,?3)",
                                rusqlite::params![path, status, reason],
                            )?;
                        }
                    }
                    publication_step(data, "before revoke commit")?;
                    tx.commit()?;
                    defer_cleanup(
                        &mut outcome.stats,
                        &active,
                        "publication",
                        publication_step(data, "after revoke commit"),
                    );
                    defer_cleanup(
                        &mut outcome.stats,
                        &active,
                        "WAL checkpoint",
                        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                            .map_err(Into::into),
                    );
                    let cleanup = remove_unreferenced(&active, &db, &mut outcome.stats);
                    defer_cleanup(&mut outcome.stats, &active, "generation", cleanup);
                }
            }
        }
        ScanDecision::Discard => {}
    }
    control.complete()?;
    if outcome.unavailable {
        let reason = outcome
            .error
            .take()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "source missing".into());
        let message = format!(
            "source unavailable: {reason}{}",
            if previous.is_some() && !current {
                "; the previous folder is still active"
            } else {
                ""
            }
        );
        let cleanup = (|| -> Result<()> {
            workspace.retain(&Default::default())?;
            if candidate.is_some() {
                // Nothing from an unavailable walk is reusable. Close both Windows
                // lock handles before reclaiming this unpublished candidate.
                drop(workspace.state.take());
                drop(_writer);
                fs::remove_dir_all(root)?;
            }
            Ok(())
        })();
        cleanup.with_context(|| format!("{message}; unpublished scan cleanup failed"))?;
        anyhow::bail!(message);
    }
    // Preserve the embedding API's per-file report; CLI/desktop callers use
    // the refusing entry points and receive the publication outcome as an error.
    if explicit.is_some() {
        return Ok(outcome.stats);
    }
    if decision != ScanDecision::Publish {
        let resume = !workspace.staged()?.is_empty();
        let error = validate_publication(&outcome.stats, populated)
            .err()
            .unwrap_or_else(|| anyhow::anyhow!("publication refused"));
        let message = error.to_string().replace("; Re-index to resume", "");
        anyhow::bail!(
            "{message}{}{}",
            if previous.is_some() && !current {
                "; the previous folder is still active"
            } else {
                ""
            },
            if resume { "; Re-index to resume" } else { "" }
        );
    }
    Ok(outcome.stats)
}

/// Library entry point; selection remains fenced through cleanup.
pub async fn reindex(
    data: &Path,
    folder: &Path,
    engines: &crate::engine::Engines,
    assets: &Path,
    progress: impl FnMut(&crate::index::Progress),
    cancelled: impl Fn() -> bool,
) -> Result<crate::index::Progress> {
    fs::create_dir_all(data)?;
    let _selection = selection_fence(data, true)?;
    scan_candidate(
        data,
        folder,
        engines,
        assets,
        progress,
        cancelled,
        ScanOptions::default(),
    )
    .await
    .map_err(|e| {
        if e.is::<PublishedFailure>() || e.to_string().starts_with("Re-index did not finish") {
            e
        } else {
            anyhow::anyhow!("Re-index did not finish: {e}")
        }
    })
}

/// Desktop cancellation and publication serialize on the same mutex.
pub async fn reindex_controlled(
    data: &Path,
    folder: &Path,
    engines: &crate::engine::Engines,
    assets: &Path,
    progress: impl FnMut(&crate::index::Progress),
    control: &ScanControl,
) -> Result<crate::index::Progress> {
    fs::create_dir_all(data)?;
    let _selection = selection_fence(data, true)?;
    scan_candidate(
        data,
        folder,
        engines,
        assets,
        progress,
        || control.is_cancelled(),
        ScanOptions {
            control: Some(control),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| {
        if e.is::<PublishedFailure>() || e.to_string().starts_with("Re-index did not finish") {
            e
        } else {
            anyhow::anyhow!("Re-index did not finish: {e}")
        }
    })
}

/// Explicit CLI state directories stay flat, even when named phase0-index.
pub async fn index_explicit(
    data: &Path,
    folder: &Path,
    engines: &crate::engine::Engines,
    assets: &Path,
    progress: impl FnMut(&crate::index::Progress),
) -> Result<crate::index::Progress> {
    scan_candidate(
        data,
        folder,
        engines,
        assets,
        progress,
        || false,
        ScanOptions {
            flat: true,
            ..Default::default()
        },
    )
    .await
    .map_err(|e| {
        if e.is::<PublishedFailure>() || e.to_string().starts_with("Re-index did not finish") {
            e
        } else {
            anyhow::anyhow!("Re-index did not finish: {e}")
        }
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
    let mut result = futures::future::poll_fn(|cx| {
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
    activate_with(data, recipe, selected, || Ok(()), &mut result)?;
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

    #[tokio::test]
    async fn t7_cancel_is_rejected_after_commit_entry_for_publish_revoke_and_pointer() {
        let (mut engines, assets) = embedding_fixture();
        for step in [
            "generation install",
            "before revoke commit",
            "before pointer rename",
        ] {
            let data = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            let file = source.path().join("good.txt");
            fs::write(&file, "Previously published content.").unwrap();
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
            if step == "before revoke commit" {
                fs::remove_file(&file).unwrap();
                fs::write(source.path().join("bad.pdf"), "bad").unwrap();
            } else {
                fs::write(&file, "Published replacement content.").unwrap();
            }
            if step == "before pointer rename" {
                engines.recipe.model.push_str("-next");
            }
            let control = ScanControl::default();
            CANCEL_HOOK.with(|hook| {
                *hook.borrow_mut() = Some((data.path().to_owned(), step, control.clone()))
            });
            let result = reindex_controlled(
                data.path(),
                source.path(),
                &engines,
                &source.path().join("no-assets"),
                |_| {},
                &control,
            )
            .await;
            CANCEL_HOOK.with(|hook| *hook.borrow_mut() = None);
            assert!(!control.is_cancelled());
            assert!(!control.cancel());
            let docs = crate::storage::State::review_documents(&resolve(data.path())).unwrap();
            if step == "before revoke commit" {
                assert!(result.is_err());
                assert!(docs.is_empty());
            } else {
                result.unwrap();
                assert_eq!(docs[0].chunks[0].text, "Published replacement content.");
            }
        }
    }

    #[test]
    fn publication_crash_child() {
        let Some(data) = std::env::var_os("VZ_PUBLICATION_DATA") else {
            return;
        };
        let source = PathBuf::from(std::env::var_os("VZ_PUBLICATION_SOURCE").unwrap());
        let (mut engines, assets) = embedding_fixture();
        if std::env::var_os("VZ_PUBLICATION_RECIPE_CHANGE").is_some() {
            engines.recipe.model.push_str("-crash");
        }
        let scanning =
            std::env::var("VZ_PUBLICATION_CRASH_STEP").ok().as_deref() == Some("scanning");
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(reindex(
                Path::new(&data),
                &source,
                &engines,
                &assets,
                |_| {
                    if scanning {
                        fs::write(
                            std::env::var_os("VZ_PUBLICATION_READY").unwrap(),
                            "scanning",
                        )
                        .unwrap();
                        loop {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                },
                || false,
            ))
            .unwrap();
    }

    #[tokio::test]
    async fn t5_t6_t7_process_death_at_production_publication_boundaries() {
        use std::process::{Command, Stdio};
        let (engines, assets) = embedding_fixture();
        for step in [
            "scanning",
            "generation install",
            "before sqlite commit",
            "after sqlite commit",
            "before pointer rename",
            "after pointer rename",
        ] {
            let data = tempfile::tempdir().unwrap();
            let a = tempfile::tempdir().unwrap();
            let b = tempfile::tempdir().unwrap();
            fs::write(
                a.path().join("a.txt"),
                "Original crash-safe published content.",
            )
            .unwrap();
            fs::write(b.path().join("b.txt"), "Replacement durable generation.").unwrap();
            reindex(data.path(), a.path(), &engines, &assets, |_| {}, || false)
                .await
                .unwrap();
            let old = resolve(data.path());
            let ready = data.path().join("child-ready");
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "active_index::tests::publication_crash_child",
                    "--nocapture",
                ])
                .env("VZ_PUBLICATION_DATA", data.path())
                .env("VZ_PUBLICATION_SOURCE", b.path())
                .env("VZ_PUBLICATION_READY", &ready)
                .env("VZ_PUBLICATION_CRASH_STEP", step)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if step.contains("pointer") {
                command.env("VZ_PUBLICATION_RECIPE_CHANGE", "1");
            }
            let mut child = command.spawn().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while !ready.exists() {
                if std::time::Instant::now() > deadline || child.try_wait().unwrap().is_some() {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child did not reach {step}");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            let root = resolve(data.path());
            // Read first, before any recovery or search: committed WAL content is visible.
            let docs = crate::storage::State::review_documents(&root).unwrap();
            let committed = step == "after sqlite commit" || step == "after pointer rename";
            assert_eq!(
                docs[0].chunks[0].text,
                if committed {
                    "Replacement durable generation."
                } else {
                    "Original crash-safe published content."
                },
                "{step}"
            );
            if step == "after pointer rename" {
                assert_ne!(root, old);
            }
            let state = crate::storage::State::open(&root).unwrap();
            for generation in state.generations().unwrap() {
                state.document(&generation).unwrap();
            }
            drop(state);
            {
                let _fence = selection_fence(data.path(), true).unwrap();
                cleanup_start(data.path()).unwrap();
            }
            if committed && step.contains("pointer") {
                assert!(!old.exists());
            }
            if step == "scanning" {
                let result = reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false)
                    .await
                    .unwrap();
                assert_eq!(result.skipped, 1);
            }
        }
    }

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
        assert_eq!(
            decide_scan(&outcome, true, true),
            ScanDecision::RevokeSource
        );
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
            &mut Default::default(),
        );
        assert!(error.is_err());
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            before
        );
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 3);
        assert_eq!(
            fs::read_dir(data.path().join(".vz-scan")).unwrap().count(),
            0
        );
        fs::remove_file(data.path().join("active-index.json")).unwrap();
        assert!(activate_with(
            data.path(),
            &new,
            &publication_dir(data.path(), &new).unwrap(),
            || anyhow::bail!("failed first write"),
            &mut Default::default(),
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
        assert!(format!("{error:#}").contains("resume"));
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
            "an unavailable source has no reusable candidate; only the active index remains"
        );
        let reopened = crate::storage::State::open(&resolve(data.path())).unwrap();
        assert_eq!(reopened.documents().unwrap()[0].sha256, documents[0].sha256);
        assert_eq!(
            reopened.documents().unwrap()[0].chunks[0].text,
            "A searchable source document."
        );
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
        let pointer = fs::read(data.path().join("active-index.json")).ok();
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
                fs::read(data.path().join("active-index.json")).ok(),
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
            let before = fs::read(data.path().join("active-index.json")).ok();
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
                let error =
                    crate::index::index(&mut state, source.path(), &engines, &assets, |_| {})
                        .await
                        .unwrap_err();
                assert!(error.to_string().contains("re-index required"));
                assert_eq!(state.documents().unwrap().len(), 1);
                continue;
            }
            assert_ne!(fs::read(data.path().join("active-index.json")).ok(), before);
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
            let pointer = fs::read(data.path().join("active-index.json")).ok();
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
                fs::read(data.path().join("active-index.json")).ok(),
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
                    if !changed_recipe {
                        assert!(crate::storage::State::open(&resolve(data.path())).is_err());
                        assert_eq!(
                            crate::storage::State::review_documents(&resolve(data.path())).unwrap()
                                [0]
                            .chunks[0]
                                .text,
                            "Published original text."
                        );
                        checked += 1;
                        return;
                    }
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
            if !changed_recipe {
                assert_eq!(
                    fs::read(data.path().join("active-index.json")).unwrap(),
                    pointer
                );
                assert_eq!(
                    crate::storage::State::review_documents(&resolve(data.path()))
                        .unwrap()
                        .len(),
                    2
                );
                continue;
            }
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
        let pointer = fs::read(data.path().join("active-index.json")).ok();
        fs::write(source.path().join("a.txt"), "Unpublished replacement.").unwrap();
        fs::write(source.path().join("z.txt"), "Unpublished extra document.").unwrap();
        let mut checks = 0;
        crate::index::index(&mut state, source.path(), &engines, &assets, |_| {
            assert_eq!(
                fs::read(data.path().join("active-index.json")).ok(),
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
        assert_eq!(
            fs::read(data.path().join("active-index.json")).ok(),
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
        let pointer = fs::read(data.path().join("active-index.json")).ok();
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
            fs::read(data.path().join("active-index.json")).ok(),
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
    async fn post_rename_durability_error_and_cleanup_warning_keep_publication() {
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
            engines.recipe.model.push_str("-published");
            FAULTS.with(|faults| faults.borrow_mut().push((data.path().to_owned(), step)));
            let result = reindex(data.path(), b.path(), &engines, &assets, |_| {}, || false).await;
            FAULTS.with(|faults| faults.borrow_mut().clear());
            if step == "directory sync" {
                let error = result.unwrap_err();
                assert!(error.is::<PublishedFailure>(), "{error:#}");
            } else {
                assert!(result.unwrap().seal_warnings.iter().any(|warning| {
                    warning.contains("Deferred generation cleanup")
                        && warning.contains("injected generation cleanup failure")
                }));
            }
            let published = resolve(data.path());
            assert_ne!(
                published, candidate,
                "recipe changes publish a fresh candidate"
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

    #[test]
    #[cfg(unix)]
    fn pointer_workspace_symlink_is_never_traversed() {
        let data = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let name = format!("pointer-{}.tmp", uuid::Uuid::new_v4());
        fs::write(external.path().join(&name), b"precious").unwrap();
        std::os::unix::fs::symlink(external.path(), data.path().join(".vz-scan")).unwrap();
        cleanup_start(data.path()).unwrap();
        cleanup_library(data.path(), None).unwrap();
        assert!(pointer_workspace(data.path(), true).is_err());
        let recipe = recipe();
        seed(data.path(), &recipe);
        assert!(activate(data.path(), &recipe).is_err());
        assert_eq!(fs::read(external.path().join(&name)).unwrap(), b"precious");
        fs::remove_file(data.path().join(".vz-scan")).unwrap();
        fs::create_dir(data.path().join(".vz-scan")).unwrap();
        let workspace = data.path().join(".vz-scan");
        fs::write(workspace.join(&name), b"crash leftover").unwrap();
        fs::write(workspace.join("pointer-unrelated.tmp"), b"keep").unwrap();
        std::os::unix::fs::symlink(
            external.path().join(&name),
            workspace.join(format!("pointer-{}.tmp", uuid::Uuid::new_v4())),
        )
        .unwrap();
        cleanup_start(data.path()).unwrap();
        assert!(!workspace.join(name).exists());
        assert!(workspace.join("pointer-unrelated.tmp").exists());
        assert_eq!(fs::read_dir(&workspace).unwrap().count(), 2);
    }

    #[test]
    fn flat_cleanup_preserves_unverified_empty_marker() {
        let data = tempfile::tempdir().unwrap();
        let recipe = recipe();
        fs::write(
            data.path().join("recipe.json"),
            serde_json::to_vec(&recipe).unwrap(),
        )
        .unwrap();
        drop(crate::storage::State::open(data.path()).unwrap());
        seed(data.path(), &recipe);
        activate(data.path(), &recipe).unwrap();
        fs::write(data.path().join(".vz-flat-cleanup"), b"").unwrap();
        let database = fs::read(data.path().join("state.sqlite")).unwrap();
        assert!(cleanup_start(data.path()).is_err());
        assert_eq!(
            fs::read(data.path().join("state.sqlite")).unwrap(),
            database
        );
        assert_eq!(fs::read(data.path().join(".vz-flat-cleanup")).unwrap(), b"");
        assert!(resolve(data.path()).join("recipe.json").exists());
    }

    #[test]
    fn flat_cleanup_interrupted_before_marker_install_retries() {
        let data = tempfile::tempdir().unwrap();
        let recipe = recipe();
        fs::write(
            data.path().join("recipe.json"),
            serde_json::to_vec(&recipe).unwrap(),
        )
        .unwrap();
        drop(crate::storage::State::open(data.path()).unwrap());
        seed(data.path(), &recipe);
        activate(data.path(), &recipe).unwrap();
        FAULTS.with(|faults| {
            faults
                .borrow_mut()
                .push((data.path().to_owned(), "flat marker install"))
        });
        assert!(cleanup_start(data.path()).is_err());
        assert!(!data.path().join(".vz-flat-cleanup").exists());
        assert!(data.path().join("state.sqlite").exists());
        FAULTS.with(|faults| faults.borrow_mut().clear());
        cleanup_start(data.path()).unwrap();
        assert!(!data.path().join("state.sqlite").exists());
    }

    #[test]
    fn flat_cleanup_marker_collision_fails_closed() {
        for link in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let recipe = recipe();
            fs::write(
                data.path().join("recipe.json"),
                serde_json::to_vec(&recipe).unwrap(),
            )
            .unwrap();
            drop(crate::storage::State::open(data.path()).unwrap());
            fs::write(data.path().join("generations/precious"), b"keep").unwrap();
            let elsewhere = seed(data.path(), &recipe);
            activate(data.path(), &recipe).unwrap();
            assert_eq!(resolve(data.path()), elsewhere.canonicalize().unwrap());
            let target = data.path().join("unrelated");
            fs::write(&target, b"unrelated bytes").unwrap();
            let marker = data.path().join(".vz-flat-cleanup");
            if link {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &marker).unwrap();
                #[cfg(not(unix))]
                fs::hard_link(&target, &marker).unwrap();
            } else {
                fs::write(&marker, b"untrusted marker").unwrap();
            }
            assert!(cleanup_library(data.path(), None).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"unrelated bytes");
            assert_eq!(
                fs::read(data.path().join("generations/precious")).unwrap(),
                b"keep"
            );
            // A marker alone cannot authorize restart cleanup.
            fs::remove_file(data.path().join("state.sqlite")).unwrap();
            cleanup_library(data.path(), None).unwrap();
            assert!(data.path().join("generations/precious").exists());
        }
    }

    #[tokio::test]
    async fn flat_cleanup_restarts_after_database_removal() {
        let (engines, assets) = embedding_fixture();
        for step in ["flat marker install", "flat database removal"] {
            let data = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            fs::write(
                source.path().join("a.txt"),
                "Flat cleanup durable generation.",
            )
            .unwrap();
            index_explicit(data.path(), source.path(), &engines, &assets, |_| {})
                .await
                .unwrap();
            let old = fs::read_dir(data.path().join("generations"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            use std::process::{Command, Stdio};
            let ready = data.path().join("cleanup-ready");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "active_index::tests::publication_crash_child",
                    "--nocapture",
                ])
                .env("VZ_PUBLICATION_DATA", data.path())
                .env("VZ_PUBLICATION_SOURCE", source.path())
                .env("VZ_PUBLICATION_READY", &ready)
                .env("VZ_PUBLICATION_CRASH_STEP", step)
                .env("VZ_PUBLICATION_RECIPE_CHANGE", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while !ready.exists() {
                if std::time::Instant::now() > deadline || child.try_wait().unwrap().is_some() {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child did not reach {step}");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            assert_eq!(
                data.path().join("state.sqlite").exists(),
                step == "flat marker install"
            );
            assert_eq!(
                data.path().join(".vz-flat-cleanup").exists(),
                step == "flat database removal"
            );
            assert!(old.exists());
            let active = resolve(data.path());
            cleanup_start(data.path()).unwrap();
            assert!(!old.exists());
            assert!(!data.path().join(".vz-flat-cleanup").exists());
            assert_eq!(
                crate::storage::State::review_documents(&active)
                    .unwrap()
                    .len(),
                1
            );
        }
    }

    #[tokio::test]
    async fn discard_does_not_run_publication_cleanup() {
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
            !chain.contains("injected generation cleanup failure"),
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
        fs::write(parent.join("CaseSentinel"), b"case control").unwrap();
        let parent = parent.canonicalize().unwrap();
        let insensitive =
            same_file::is_same_file(parent.join("CaseSentinel"), parent.join("cASEsENTINEL"))
                .unwrap_or(false);
        let modified = fs::metadata(&parent).unwrap().modified().unwrap();
        let entries = fs::read_dir(&parent).unwrap().count();
        assert_eq!(
            same_source(&parent.join("DOCS"), &parent.join("docs")),
            insensitive
        );
        assert!(!same_source(&parent.join("other"), &parent.join("docs")));
        assert_eq!(fs::metadata(&parent).unwrap().modified().unwrap(), modified);
        assert_eq!(fs::read_dir(&parent).unwrap().count(), entries);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retargeted_historical_parent_preserves_selection_documents_and_generations() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let sources = tempfile::tempdir().unwrap();
        let a = sources.path().join("A");
        let b = sources.path().join("B");
        let source = a.join("docs");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(
            source.join("good.txt"),
            "Historical source remains published.",
        )
        .unwrap();
        reindex(data.path(), &source, &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
        let root = resolve(data.path());
        let selection = last_indexed_folder(&root).unwrap();
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        let state = crate::storage::State::open(&root).unwrap();
        let generations = state.generations().unwrap();
        let documents = serde_json::to_value(state.documents().unwrap()).unwrap();
        drop(state);
        fs::rename(&a, sources.path().join("original-A")).unwrap();
        std::os::unix::fs::symlink(&b, &a).unwrap();
        let request = source_identity(&b.join("docs")).unwrap();
        assert!(!same_source(&request, Path::new(&selection)));
        assert!(reindex(
            data.path(),
            &b.join("docs"),
            &engines,
            &assets,
            |_| {},
            || false
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("previous folder is still active"));
        assert_eq!(last_indexed_folder(&root).unwrap(), selection);
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        let state = crate::storage::State::open(&root).unwrap();
        assert_eq!(state.generations().unwrap(), generations);
        assert_eq!(
            serde_json::to_value(state.documents().unwrap()).unwrap(),
            documents
        );
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

    #[test]
    fn missing_container_requires_exact_spelling_without_writes() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("S");
        let before = fs::metadata(parent.path()).unwrap().modified().unwrap();
        assert!(!same_source(&missing.join("docs"), &missing.join("Docs")));
        assert!(same_source(&missing.join("Docs"), &missing.join("Docs")));
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
        assert_eq!(
            fs::metadata(parent.path()).unwrap().modified().unwrap(),
            before
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn missing_tail_reads_mount_case_rules_without_writes() {
        // A pre-existing sensitive mount is optional; do not create a volume.
        let Some(mount) = std::env::var_os("VZ_CASE_SENSITIVE_TEST_ROOT") else {
            eprintln!("optional mount regression skipped: set VZ_CASE_SENSITIVE_TEST_ROOT");
            return;
        };
        let mount = Path::new(&mount);
        assert!(mount.is_dir(), "configured case-sensitive mount must exist");
        assert!(
            !directory_is_case_insensitive(mount),
            "configured mount must be case-sensitive"
        );
        eprintln!("case-sensitive mount regression ran on {}", mount.display());
        assert!(!same_source(&mount.join("Docs"), &mount.join("docs")));
        assert!(!same_source(
            &mount.join("2024/Docs"),
            &mount.join("2024/docs")
        ));
    }

    #[tokio::test]
    async fn renamed_container_preserves_case_variant_and_exact_request_revokes() {
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let outer = tempfile::tempdir().unwrap();
        let parent = outer.path().join("S");
        let source = parent.join("Docs");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("good.txt"),
            "Renamed container retains searchable document.",
        )
        .unwrap();
        reindex(data.path(), &source, &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
        let root = resolve(data.path());
        let selection = last_indexed_folder(&root);
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        let state = crate::storage::State::open(&root).unwrap();
        let generations = state.generations().unwrap();
        let documents = serde_json::to_value(state.documents().unwrap()).unwrap();
        drop(state);
        fs::rename(&parent, outer.path().join("renamed-S")).unwrap();
        assert!(reindex(
            data.path(),
            &parent.join("docs"),
            &engines,
            &assets,
            |_| {},
            || false
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("previous folder is still active"));
        assert_eq!(last_indexed_folder(&root), selection);
        assert_eq!(
            fs::read(data.path().join("active-index.json")).unwrap(),
            pointer
        );
        let state = crate::storage::State::open(&root).unwrap();
        assert_eq!(state.generations().unwrap(), generations);
        assert_eq!(
            serde_json::to_value(state.documents().unwrap()).unwrap(),
            documents
        );
        assert!(!state
            .search(
                "Renamed container",
                &engines.vector("Renamed container", true).unwrap(),
                3
            )
            .await
            .unwrap()
            .is_empty());
        drop(state);
        assert!(
            reindex(data.path(), &source, &engines, &assets, |_| {}, || false)
                .await
                .is_err()
        );
        assert_eq!(last_indexed_folder(&root), selection);
        assert!(crate::storage::State::review_documents(&root)
            .unwrap()
            .is_empty());
        assert_eq!(fs::read_dir(root.join("generations")).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn missing_current_drive_root_child_revokes_case_variant() {
        let (engines, assets) = embedding_fixture();
        let temp = tempfile::tempdir().unwrap();
        let drive: PathBuf = temp
            .path()
            .canonicalize()
            .unwrap()
            .components()
            .take(2)
            .collect();
        assert!(
            directory_is_case_insensitive(&drive),
            "drive-root regression requires Windows' default case-insensitive root"
        );
        let name = format!("VzDocs-{}", uuid::Uuid::new_v4());
        let source = drive.join(&name);
        fs::create_dir(&source).unwrap();
        fs::write(source.join("good.txt"), "Drive root selected document.").unwrap();
        reindex(temp.path(), &source, &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
        let root = resolve(temp.path());
        let selection = last_indexed_folder(&root);
        fs::remove_dir_all(&source).unwrap();
        let request = drive.join(name.to_uppercase());
        assert!(same_source(
            &request,
            Path::new(selection.as_ref().unwrap())
        ));
        assert!(
            reindex(temp.path(), &request, &engines, &assets, |_| {}, || false)
                .await
                .is_err()
        );
        assert_eq!(last_indexed_folder(&root), selection);
        assert!(crate::storage::State::review_documents(&root)
            .unwrap()
            .is_empty());
        assert_eq!(fs::read_dir(root.join("generations")).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn case_sensitive_container_preserves_distinct_source() {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            FileCaseSensitiveInfo, SetFileInformationByHandle, FILE_CASE_SENSITIVE_INFO,
            FILE_FLAG_BACKUP_SEMANTICS,
        };
        let (engines, assets) = embedding_fixture();
        let data = tempfile::tempdir().unwrap();
        let outer = tempfile::tempdir().unwrap();
        let parent = outer.path().join("S");
        fs::create_dir(&parent).unwrap();
        let handle = fs::OpenOptions::new()
            .access_mode(0x100) // FILE_WRITE_ATTRIBUTES
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent.as_path())
            .unwrap();
        let info = FILE_CASE_SENSITIVE_INFO { Flags: 1 };
        if unsafe {
            SetFileInformationByHandle(
                handle.as_raw_handle(),
                FileCaseSensitiveInfo,
                (&info as *const FILE_CASE_SENSITIVE_INFO).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            eprintln!(
                "per-directory case sensitivity unavailable: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        drop(handle);
        let source = parent.as_path().join("Docs");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("good.txt"),
            "Distinct case source remains published.",
        )
        .unwrap();
        reindex(data.path(), &source, &engines, &assets, |_| {}, || false)
            .await
            .unwrap();
        let root = resolve(data.path());
        let selection = last_indexed_folder(&root);
        let state = crate::storage::State::open(&root).unwrap();
        let generations = state.generations().unwrap();
        let documents = serde_json::to_value(state.documents().unwrap()).unwrap();
        drop(state);
        let pointer = fs::read(data.path().join("active-index.json")).unwrap();
        assert!(!directory_is_case_insensitive(&parent));
        for rename in [false, true] {
            if rename {
                fs::rename(&parent, outer.path().join("renamed-S")).unwrap();
            }
            assert!(reindex(
                data.path(),
                &parent.join("docs"),
                &engines,
                &assets,
                |_| {},
                || false
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("previous folder is still active"));
            assert_eq!(last_indexed_folder(&root), selection);
            assert_eq!(
                fs::read(data.path().join("active-index.json")).unwrap(),
                pointer
            );
            let state = crate::storage::State::open(&root).unwrap();
            assert_eq!(state.generations().unwrap(), generations);
            assert_eq!(
                serde_json::to_value(state.documents().unwrap()).unwrap(),
                documents
            );
            assert!(!state
                .search(
                    "Distinct case",
                    &engines.vector("Distinct case", true).unwrap(),
                    3
                )
                .await
                .unwrap()
                .is_empty());
        }
        // Exact stored spelling still identifies the unavailable selected source.
        assert!(
            reindex(data.path(), &source, &engines, &assets, |_| {}, || false)
                .await
                .is_err()
        );
        assert_eq!(last_indexed_folder(&root), selection);
        assert!(crate::storage::State::review_documents(&root)
            .unwrap()
            .is_empty());
        assert_eq!(fs::read_dir(root.join("generations")).unwrap().count(), 0);
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
