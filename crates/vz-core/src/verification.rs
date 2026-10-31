//! Persistent verification shared by desktop status and all engine loaders.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::Mutex,
    time::SystemTime,
};

const RECORD: &str = "models.verified.json";
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Stamp {
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
type Records = BTreeMap<String, Entry>;

fn records(path: &Path) -> Records {
    fs::read(path.parent().unwrap().join(RECORD))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
fn save(path: &Path, sha256: &str, stamp: Stamp) -> Result<()> {
    let dir = path.parent().context("model directory")?;
    let mut entries = records(path);
    entries.insert(
        path.file_name()
            .context("model filename")?
            .to_string_lossy()
            .into_owned(),
        Entry {
            sha256: sha256.into(),
            stamp,
        },
    );
    let temp = dir.join(format!(".{RECORD}.{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec(&entries)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, dir.join(RECORD))?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

/// Trust only an unchanged regular file with a record for the current pin.
/// If the cache cannot be persisted, future calls still hash in full.
pub fn verify(path: &Path, expected: &str, size: Option<u64>) -> Result<()> {
    let _guard = LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("verification lock poisoned"))?;
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
    if records(path)
        .get(key.as_ref())
        .is_some_and(|entry| entry.sha256 == expected && entry.stamp == before)
    {
        return Ok(());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == expected,
        "model hash mismatch"
    );
    ensure!(
        before == Stamp::of(&file)? && before == Stamp::of(&regular(path, false)?)?,
        "model changed during verification"
    );
    let _ = save(path, expected, before);
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
    let _guard = LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("verification lock poisoned"))?;
    let stamp = Stamp::of(staged)?;
    ensure!(
        stamp.size == before_hash.len()
            && stamp.modified == before_hash.modified()?
            && stamp == Stamp::of(&regular(path, false)?)?,
        "model changed during promotion"
    );
    let _ = save(path, expected, stamp);
    Ok(())
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
