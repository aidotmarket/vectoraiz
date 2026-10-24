//! Shell-only acquisition. Engines remain offline and recheck the same pins.
use anyhow::{bail, ensure, Context, Result};
use reqwest::{header, Client, StatusCode};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{watch, Mutex, OwnedMutexGuard};

pub const BUSY: &str = "Another model or index operation is running";
#[derive(Clone)]
pub struct Operations {
    lock: Arc<Mutex<()>>,
    cancel: watch::Sender<bool>,
}
impl Default for Operations {
    fn default() -> Self {
        Self {
            lock: Arc::new(Mutex::new(())),
            cancel: watch::channel(false).0,
        }
    }
}
impl Operations {
    pub fn acquire(&self) -> Result<OwnedMutexGuard<()>, String> {
        self.lock
            .clone()
            .try_lock_owned()
            .map_err(|_| BUSY.to_owned())
    }
    // Reset only after acquiring the mutation lock; a refused call changes nothing.
    pub fn start(&self) -> watch::Receiver<bool> {
        self.cancel.send_replace(false);
        self.cancel.subscribe()
    }
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }
}

#[derive(Clone, Debug)]
pub struct Pin {
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub metadata: Value,
}
pub fn pins(path: &Path) -> Result<Vec<Pin>> {
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    ["embedding", "chat"]
        .iter()
        .map(|key| {
            let p = &value[key];
            let file = p["file"].as_str().context("model file pin")?;
            ensure!(
                Path::new(file).components().count() == 1 && !file.starts_with('.'),
                "invalid model file pin"
            );
            Ok(Pin {
                file: file.into(),
                size: p["size"].as_u64().context("model size pin")?,
                sha256: p["sha256"].as_str().context("model hash pin")?.into(),
                metadata: p.clone(),
            })
        })
        .collect()
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
pub fn verified(dir: &Path, pin: &Pin) -> bool {
    regular(&dir.join(&pin.file), false).is_ok_and(|file| {
        file.metadata().is_ok_and(|m| m.len() == pin.size)
            && vz_core::pipeline::file_sha256(&dir.join(&pin.file)).is_ok_and(|h| h == pin.sha256)
    })
}
pub fn status(dir: &Path, pins: &[Pin]) -> Value {
    let files: Vec<_> = pins
        .iter()
        .map(|p| {
            let mut v = p.metadata.clone();
            v["verified"] = json!(verified(dir, p));
            v
        })
        .collect();
    json!({"ready": files.iter().all(|f| f["verified"] == true), "files": files})
}
pub fn validate_folder(folder: &str) -> Result<(), String> {
    if folder.trim().is_empty() {
        return Err("Choose a folder to index".into());
    }
    if !Path::new(folder).is_dir() {
        return Err("Folder not found".into());
    }
    Ok(())
}

fn client_builder() -> reqwest::ClientBuilder {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                attempt.error("model redirect must use HTTPS")
            } else if attempt.previous().len() > 5 {
                attempt.error("too many model redirects (maximum 5)")
            } else {
                attempt.follow()
            }
        }))
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        // Own the retry budget and always start from the pinned initial URL.
        .retry(reqwest::retry::never())
}

pub fn client() -> Result<Client> {
    Ok(client_builder().build()?)
}

pub fn space_needed(dir: &Path, pins: &[Pin], importing: bool) -> Result<u64> {
    let mut needed = 0;
    for pin in pins {
        if !importing && verified(dir, pin) {
            continue;
        }
        let partial = dir.join(format!("{}.partial", pin.file));
        let have = if !importing && partial.exists() {
            regular(&partial, false)?.metadata()?.len().min(pin.size)
        } else {
            0
        };
        needed += pin.size - have;
    }
    Ok(needed)
}
pub fn check_space(dir: &Path, needed: u64) -> Result<()> {
    let available = free_space(dir)?;
    ensure!(
        available >= needed,
        "Not enough free disk space: {needed} bytes needed; {available} bytes available"
    );
    Ok(())
}
#[cfg(target_os = "macos")]
fn free_space(dir: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    // Darwin sys/statvfs.h: unsigned long fields are 64-bit, but
    // fsblkcnt_t/fsfilcnt_t are unsigned int (32-bit), including on arm64.
    #[repr(C)]
    struct StatVfs {
        sizes: [u64; 2],
        counts: [u32; 6],
        flags: [u64; 3],
    }
    unsafe extern "C" {
        fn statvfs(path: *const std::ffi::c_char, data: *mut StatVfs) -> i32;
    }
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes())?;
    let mut data = StatVfs {
        sizes: [0; 2],
        counts: [0; 6],
        flags: [0; 3],
    };
    // SAFETY: a terminated path and a correctly sized, aligned output structure.
    if unsafe { statvfs(path.as_ptr(), &mut data) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(data.sizes[1].saturating_mul(u64::from(data.counts[2])))
}
#[cfg(windows)]
fn free_space(dir: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    // SAFETY: terminated UTF-16 path, valid output pointer; optional outputs null.
    if unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(available)
}
#[cfg(not(any(target_os = "macos", windows)))]
fn free_space(dir: &Path) -> Result<u64> {
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(dir)
        .output()?;
    ensure!(output.status.success(), "Cannot check free disk space");
    let text = String::from_utf8(output.stdout)?;
    Ok(text
        .lines()
        .last()
        .context("disk space")?
        .split_whitespace()
        .nth(3)
        .context("disk space")?
        .parse::<u64>()?
        * 1024)
}

pub enum Source<'a> {
    Download(&'a Client),
    Import(File),
}
fn cancelled(cancel: &watch::Receiver<bool>) -> Result<()> {
    ensure!(
        !*cancel.borrow(),
        "Model acquisition cancelled; partial download kept for resume"
    );
    Ok(())
}
async fn cancellable<T>(
    cancel: &mut watch::Receiver<bool>,
    future: impl std::future::Future<Output = T>,
) -> Result<T> {
    cancelled(cancel)?;
    tokio::select! {
        value = future => Ok(value),
        _ = cancel.changed() => bail!("Model acquisition cancelled; partial download kept for resume"),
    }
}
static STAGE_ID: AtomicU64 = AtomicU64::new(0);
struct Stage {
    path: PathBuf,
    keep: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The sole acquisition path: stage on the target filesystem, verify size and
/// hash, close all handles, and atomically replace. The target is never opened
/// for writing. Only this operation owns this stage (the caller holds the lock).
pub async fn stage_verify_promote(
    dir: &Path,
    pin: &Pin,
    source: Source<'_>,
    mut cancel: watch::Receiver<bool>,
    progress: &mut (impl FnMut(Value) + Send),
) -> Result<()> {
    let download = matches!(source, Source::Download(_));
    let path = if download {
        dir.join(format!("{}.partial", pin.file))
    } else {
        dir.join(format!(
            "{}.import-{}-{}",
            pin.file,
            std::process::id(),
            STAGE_ID.fetch_add(1, Ordering::Relaxed)
        ))
    };
    // An existing partial must also be a regular file, not a link or reparse point.
    let mut stage_file = if download && path.try_exists()? {
        regular(&path, true)?
    } else {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?
    };
    let mut stage = Stage {
        path,
        keep: download,
    };
    match source {
        Source::Download(client) => {
            transfer(
                client,
                &mut stage_file,
                dir,
                pin,
                &mut cancel,
                progress,
                &mut stage.keep,
            )
            .await?;
        }
        Source::Import(mut input) => {
            check_space(dir, pin.size)?;
            let mut buffer = vec![0; 65536];
            let mut count = 0;
            loop {
                cancelled(&cancel)?;
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                ensure!(
                    count <= pin.size,
                    "{}: size mismatch (exceeds {} bytes)",
                    pin.file,
                    pin.size
                );
                stage_file.write_all(&buffer[..n])?;
                // Give Cancel and other IPC requests a chance even for cached disk copies.
                tokio::task::yield_now().await;
            }
        }
    }
    stage.keep = false; // completed or mismatched downloads are no longer resumable
    stage_file.sync_all()?;
    ensure!(
        stage_file.metadata()?.len() == pin.size,
        "{}: size mismatch (expected {} bytes)",
        pin.file,
        pin.size
    );
    drop(stage_file);
    let hash_path = stage.path.clone();
    let hash =
        tokio::task::spawn_blocking(move || vz_core::pipeline::file_sha256(&hash_path)).await??;
    ensure!(
        hash == pin.sha256,
        "{}: sha256 mismatch; staging file deleted",
        pin.file
    );
    if *cancel.borrow() {
        stage.keep = download;
    }
    cancelled(&cancel)?;
    fs::rename(&stage.path, dir.join(&pin.file))
        .with_context(|| format!("{}: could not promote verified model", pin.file))?;
    Ok(())
}

async fn transfer(
    client: &Client,
    file: &mut File,
    dir: &Path,
    pin: &Pin,
    cancel: &mut watch::Receiver<bool>,
    progress: &mut (impl FnMut(Value) + Send),
    keep: &mut bool,
) -> Result<()> {
    let url = pin.metadata["url"].as_str().context("model URL pin")?;
    let mut retries = 0;
    let mut restarted = false;
    loop {
        cancelled(cancel)?;
        let mut len = file.metadata()?.len();
        if len > pin.size {
            *keep = false;
            bail!("{}: partial exceeds pinned size", pin.file);
        }
        if len == pin.size {
            return Ok(());
        }
        check_space(dir, pin.size - len)?;
        progress(
            json!({"file": pin.file, "downloaded": len, "total": pin.size, "attempt": retries + 1}),
        );
        let mut request = client.get(url);
        if len > 0 {
            request = request.header(header::RANGE, format!("bytes={len}-"));
        }
        let response = cancellable(cancel, request.send()).await?;
        let attempt = match response {
            Err(error) if error.is_connect() || error.is_timeout() || error.is_body() => {
                Err(error.into())
            }
            Err(error) => return Err(error.into()),
            Ok(mut response) => {
                if response.status().is_server_error() {
                    Err(anyhow::anyhow!(
                        "Model server returned {}",
                        response.status()
                    ))
                } else {
                    let expected = format!("bytes {len}-{}/{}", pin.size - 1, pin.size);
                    let valid_range = response.status() == StatusCode::PARTIAL_CONTENT
                        && response
                            .headers()
                            .get(header::CONTENT_RANGE)
                            .and_then(|v| v.to_str().ok())
                            == Some(expected.as_str());
                    if response.status() == StatusCode::RANGE_NOT_SATISFIABLE
                        || (response.status() == StatusCode::PARTIAL_CONTENT && !valid_range)
                    {
                        file.set_len(0)?;
                        file.seek(SeekFrom::Start(0))?;
                        if restarted {
                            *keep = false;
                            bail!("{}: invalid Content-Range after restart", pin.file);
                        }
                        restarted = true;
                        continue;
                    }
                    ensure!(
                        response.status() == StatusCode::OK || valid_range,
                        "Model server returned {}",
                        response.status()
                    );
                    if response.status() == StatusCode::OK {
                        len = 0;
                        file.set_len(0)?;
                    }
                    // A 200 or invalid Range requires space for the whole replacement.
                    check_space(dir, pin.size - len)?;
                    file.seek(SeekFrom::Start(len))?;
                    let result = loop {
                        match cancellable(cancel, response.chunk()).await? {
                            Ok(Some(chunk)) => {
                                if chunk.len() as u64 > pin.size - len {
                                    *keep = false;
                                    bail!("{}: download exceeds pinned size", pin.file);
                                }
                                file.write_all(&chunk)?;
                                len += chunk.len() as u64;
                                progress(
                                    json!({"file": pin.file, "downloaded": len, "total": pin.size, "attempt": retries + 1}),
                                );
                            }
                            Ok(None) => break Ok(()),
                            Err(error) => break Err(error.into()),
                        }
                    };
                    if result.is_ok() {
                        return result;
                    }
                    result
                }
            }
        };
        if retries == 3 {
            return attempt;
        }
        retries += 1;
        progress(
            json!({"file": pin.file, "downloaded": file.metadata()?.len(), "total": pin.size, "attempt": retries + 1, "retry": true}),
        );
        cancellable(
            cancel,
            tokio::time::sleep(Duration::from_secs(1 << (retries - 1))),
        )
        .await?;
        // Both retries and Range resumes always request url, never response.url().
    }
}

pub async fn download(
    dir: &Path,
    pins: &[Pin],
    cancel: watch::Receiver<bool>,
    progress: &mut (impl FnMut(Value) + Send),
) -> Result<()> {
    fs::create_dir_all(dir)?;
    check_space(dir, space_needed(dir, pins, false)?)?;
    let client = client()?;
    for pin in pins {
        if !verified(dir, pin) {
            stage_verify_promote(
                dir,
                pin,
                Source::Download(&client),
                cancel.clone(),
                progress,
            )
            .await?;
        }
    }
    Ok(())
}
pub async fn import(
    dir: &Path,
    pins: &[Pin],
    folder: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<Vec<Value>> {
    ensure!(folder.is_absolute(), "Type an absolute folder path");
    ensure!(folder.is_dir(), "Import folder not found");
    fs::create_dir_all(dir)?;
    check_space(dir, space_needed(dir, pins, true)?)?;
    let mut results = Vec::new();
    for pin in pins {
        cancelled(&cancel)?;
        let path = folder.join(&pin.file);
        let result = match regular(&path, false) {
            Ok(file) => {
                stage_verify_promote(dir, pin, Source::Import(file), cancel.clone(), &mut |_| {})
                    .await
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Err(anyhow::anyhow!("missing"))
            }
            Err(error) => Err(error),
        };
        let message = match result {
            Ok(()) => "imported".into(),
            Err(error) => format!("{error:#}"),
        };
        results.push(json!({"file": pin.file, "result": message}));
    }
    Ok(results)
}

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "smoke-test"))]
pub fn index_snapshot(root: &Path) -> Result<Value> {
    fn visit(
        root: &Path,
        dir: &Path,
        files: &mut std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                visit(root, &entry.path(), files)?;
            } else {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)?
                        .to_string_lossy()
                        .into_owned(),
                    vz_core::pipeline::file_sha256(&entry.path())?,
                );
            }
        }
        Ok(())
    }
    let mut files = std::collections::BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(json!(files))
}
