//! Persistent verification shared by desktop status and all engine loaders.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::SystemTime,
};

const RECORD: &str = "models.verified.json";
// Map access and record updates are short; neither lock spans a model hash.
static PATH_LOCKS: LazyLock<Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static RECORD_LOCK: Mutex<()> = Mutex::new(());

fn path_lock(path: &Path) -> Result<Arc<Mutex<()>>> {
    let path = path.canonicalize()?;
    let mut locks = PATH_LOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(Arc::clone(locks.entry(path).or_default()))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Stamp {
    size: u64,
    modified: SystemTime,
    identity: Option<(u64, u64)>,
}
impl Stamp {
    fn of(file: &File) -> Result<Self> {
        let metadata = file.metadata()?;
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            Some((metadata.dev(), metadata.ino()))
        };
        #[cfg(windows)]
        let identity = {
            use std::os::windows::io::AsRawHandle;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut u32)
                    -> i32;
            }
            // BY_HANDLE_FILE_INFORMATION: attributes, three FILETIMEs, volume,
            // size high/low, link count, file index high/low (13 DWORDs).
            let mut info = [0u32; 13];
            // SAFETY: live file handle and correctly sized/aligned output storage.
            ensure!(
                unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } != 0,
                "Cannot read model file identity"
            );
            Some((
                u64::from(info[7]),
                (u64::from(info[11]) << 32) | u64::from(info[12]),
            ))
        };
        #[cfg(not(any(unix, windows)))]
        let identity = None;
        Ok(Self {
            size: metadata.len(),
            modified: metadata.modified()?,
            identity,
        })
    }
}
#[derive(Serialize, Deserialize)]
struct Entry {
    sha256: String,
    stamp: Stamp,
}
const VERSION: u32 = 1;
#[derive(Serialize, Deserialize)]
struct Records {
    version: u32,
    entries: BTreeMap<String, Entry>,
}
impl Default for Records {
    fn default() -> Self {
        Self {
            version: VERSION,
            entries: BTreeMap::new(),
        }
    }
}

fn records_unlocked(path: &Path) -> Records {
    fs::read(path.parent().unwrap().join(RECORD))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Records>(&bytes).ok())
        .filter(|record| record.version == VERSION)
        .unwrap_or_default()
}
fn save(path: &Path, sha256: &str, stamp: Stamp) -> Result<()> {
    let dir = path.parent().context("model directory")?;
    let _guard = RECORD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut entries = records_unlocked(path);
    entries.entries.insert(
        path.file_name()
            .context("model filename")?
            .to_string_lossy()
            .into_owned(),
        Entry {
            sha256: sha256.into(),
            stamp,
        },
    );
    write_records(dir, &entries)
}

fn write_records(dir: &Path, entries: &Records) -> Result<()> {
    let temp = dir.join(format!(".{RECORD}.{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec(entries)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, dir.join(RECORD))?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

/// A cheap identity snapshot for background verification result invalidation.
pub fn stamp(path: &Path) -> Result<Stamp> {
    Stamp::of(&regular(path, false)?)
}

/// Check only a saved verification record, without ever hashing.
pub fn recorded(path: &Path, expected: &str, size: u64) -> bool {
    let Ok(stamp) = stamp(path) else {
        return false;
    };
    if stamp.size != size {
        return false;
    }
    let _guard = RECORD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    path.file_name().is_some_and(|name| {
        records_unlocked(path)
            .entries
            .get(name.to_string_lossy().as_ref())
            .is_some_and(|entry| entry.sha256 == expected && entry.stamp == stamp)
    })
}

/// Hash the retained file from offset zero, without reopening its path.
pub fn sha256_reader(reader: &mut (impl Read + Seek)) -> Result<String> {
    sha256_reader_with_hook(reader, || {})
}
fn sha256_reader_with_hook(reader: &mut (impl Read + Seek), hook: impl FnOnce()) -> Result<String> {
    let mut hook = Some(hook);
    reader.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        if let Some(hook) = hook.take() {
            hook();
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Trust only an unchanged regular file with a record for the current pin.
/// If the cache cannot be persisted, future calls still hash in full.
pub fn verify(path: &Path, expected: &str, size: Option<u64>) -> Result<()> {
    verify_with_hook(path, expected, size, || {})
}

fn verify_with_hook(
    path: &Path,
    expected: &str,
    size: Option<u64>,
    after_open: impl FnOnce(),
) -> Result<()> {
    let lock = path_lock(path)?;
    let _guard = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut file = regular(path, false)?;
    let before = Stamp::of(&file)?;
    ensure!(
        size.is_none_or(|size| before.size == size),
        "model size mismatch"
    );
    let key = path
        .file_name()
        .context("model filename")?
        .to_string_lossy();
    let entries = {
        let _guard = RECORD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        records_unlocked(path)
    };
    if entries
        .entries
        .get(key.as_ref())
        .is_some_and(|entry| entry.sha256 == expected && entry.stamp == before)
    {
        return Ok(());
    }
    ensure!(
        sha256_reader_with_hook(&mut file, after_open)? == expected,
        "model hash mismatch"
    );
    ensure!(
        before == Stamp::of(&file)? && before == Stamp::of(&regular(path, false)?)?,
        "model changed during verification"
    );
    if let Err(error) = save(path, expected, before) {
        eprintln!("Could not save model verification record: {error:#}");
    }
    Ok(())
}

/// Persist an acquisition's already verified file after atomic promotion.
/// The opened staging handle must still identify the same unchanged file.
pub fn record_verified(
    path: &Path,
    expected: &str,
    staged: &File,
    before_hash: &fs::Metadata,
) -> Result<()> {
    let lock = path_lock(path)?;
    let _guard = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stamp = Stamp::of(staged)?;
    ensure!(
        stamp.size == before_hash.len()
            && stamp.modified == before_hash.modified()?
            && stamp == Stamp::of(&regular(path, false)?)?,
        "model changed during promotion"
    );
    if let Err(error) = save(path, expected, stamp) {
        eprintln!("Could not save model verification record: {error:#}");
    }
    Ok(())
}

/// Remove the verification record after deleting an unused model. The caller
/// serialises acquisition/deletion; a missing model need not canonicalise.
pub fn forget(path: &Path) -> Result<()> {
    let _guard = RECORD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut entries = records_unlocked(path);
    entries.entries.remove(
        path.file_name()
            .context("model filename")?
            .to_string_lossy()
            .as_ref(),
    );
    write_records(path.parent().context("model directory")?, &entries)
}

// Open first, without following links, then inspect the opened handle. Windows
// cloud placeholders are reparse points even when FileType::is_symlink is false.
pub fn regular(path: &Path, write: bool) -> Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        is_regular(&metadata),
        "{}: not a regular file",
        path.display()
    );
    let mut options = OpenOptions::new();
    options.read(true).write(write);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "macos")]
        const NOFOLLOW: i32 = 0x100;
        #[cfg(not(target_os = "macos"))]
        const NOFOLLOW: i32 = 0x20000;
        options.custom_flags(NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .with_context(|| format!("{}: not a regular file or cannot open", path.display()))?;
    ensure!(
        is_regular(&file.metadata()?),
        "{}: not a regular file",
        path.display()
    );
    Ok(file)
}
fn is_regular(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        } // FILE_ATTRIBUTE_REPARSE_POINT
    }
    metadata.is_file() && !metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_paths_verify_while_answer_hash_is_stalled() {
        let dir = tempfile::tempdir().unwrap();
        let answer = dir.path().join("answer.gguf");
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../spike/fixtures/models.json")).unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let filename = fixture["embedding"]["file"].as_str().unwrap();
        let search = dir.path().join(filename);
        fs::copy(root.join(".assets").join(filename), &search)
            .expect("fetch the fixture assets before Rust tests");
        let pins = dir.path().join("pins.json");
        fs::write(
            &pins,
            serde_json::json!({"embedding":fixture["embedding"]}).to_string(),
        )
        .unwrap();
        // Search was verified by status, but its engine cache is still empty.
        verify(&search,fixture["embedding"]["sha256"].as_str().unwrap(),fixture["embedding"]["size"].as_u64()).unwrap();
        fs::write(&answer, b"answer").unwrap();
        let answer_hash = crate::pipeline::file_sha256(&answer).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let held = std::thread::spawn(move || {
            verify_with_hook(&answer, &answer_hash, None, || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let assets = dir.path().to_owned();
        let first_search = std::thread::spawn(move || {
            let result = crate::engine::Engines::load_embedding(&assets, &pins, true)
                .and_then(|engine| engine.vector("orbital archive retention", true).map(|_| ()));
            done_tx.send(result).unwrap();
        });
        let completed = done_rx.recv_timeout(std::time::Duration::from_secs(30));
        release_tx.send(()).unwrap();
        held.join().unwrap().unwrap();
        first_search.join().unwrap();
        completed
            .expect("search verification waited behind answer hash")
            .unwrap();
        let records = records_unlocked(&search);
        assert_eq!(
            records.entries.len(),
            2,
            "parallel saves must retain both records"
        );
        forget(&dir.path().join("answer.gguf")).unwrap();
        assert_eq!(records_unlocked(&search).entries.len(), 1);
    }

    #[test]
    fn record_is_used_and_invalidated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.gguf");
        fs::write(&path, b"original").unwrap();
        let pin = crate::pipeline::file_sha256(&path).unwrap();
        verify(&path, &pin, Some(8)).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"modified").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        // Deliberately preserve the cache key: success proves contents weren't read.
        verify(&path, &pin, Some(8)).unwrap();
        assert!(verify(&path, "changed release pin", Some(8)).is_err());
        let file = File::options().write(true).open(&path).unwrap();
        file.set_modified(modified + std::time::Duration::from_secs(2))
            .unwrap();
        assert!(verify(&path, &pin, Some(8)).is_err());
        file.set_modified(modified).unwrap();
        file.set_len(9).unwrap();
        assert!(verify(&path, &pin, Some(8)).is_err());
    }

    #[test]
    fn missing_and_corrupt_records_hash_in_full() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.gguf");
        fs::write(&path, b"original").unwrap();
        let pin = crate::pipeline::file_sha256(&path).unwrap();
        verify(&path, &pin, None).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"modified").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        fs::write(dir.path().join(RECORD), b"corrupt").unwrap();
        assert!(verify(&path, &pin, None).is_err());
        fs::remove_file(dir.path().join(RECORD)).unwrap();
        assert!(verify(&path, &pin, None).is_err());
        fs::write(&path, b"original").unwrap();
        verify(&path, &pin, None).unwrap();
        fs::write(dir.path().join(RECORD), b"corrupt").unwrap();
        verify(&path, &pin, None).unwrap();
    }

    #[test]
    fn missing_and_unknown_record_versions_require_full_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.gguf");
        fs::write(&path, b"original").unwrap();
        let pin = crate::pipeline::file_sha256(&path).unwrap();
        verify(&path, &pin, None).unwrap();
        let record_path = dir.path().join(RECORD);
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
        assert_eq!(record["version"], 1);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"modified").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        for version in [None, Some(2)] {
            let mut record = record.clone();
            record.as_object_mut().unwrap().remove("version");
            if let Some(version) = version {
                record["version"] = version.into();
            }
            fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
            assert!(verify(&path, &pin, None).is_err());
        }
    }

    #[test]
    fn replacement_identity_invalidates_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.gguf");
        fs::write(&path, b"original").unwrap();
        let pin = crate::pipeline::file_sha256(&path).unwrap();
        verify(&path, &pin, None).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let replacement = dir.path().join("replacement");
        fs::write(&replacement, b"modified").unwrap();
        File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        fs::rename(replacement, &path).unwrap();
        assert!(verify(&path, &pin, None).is_err());
    }

    #[test]
    fn unwritable_record_keeps_full_verification_available() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.gguf");
        fs::write(&path, b"original").unwrap();
        let pin = crate::pipeline::file_sha256(&path).unwrap();
        // A directory at the record path makes atomic replacement fail on all hosts.
        fs::create_dir(dir.path().join(RECORD)).unwrap();
        verify(&path, &pin, None).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"modified").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert!(verify(&path, &pin, None).is_err());
    }
}
