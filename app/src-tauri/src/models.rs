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
// The operation lock owns the cache: no model can stay mapped during mutation.
// Generic loader seam keeps lifecycle tests independent of native model files.
pub trait CachedEngine {
    fn ensure_chat(&self) -> Result<()>;
}
impl CachedEngine for vz_core::engine::Engines {
    fn ensure_chat(&self) -> Result<()> {
        self.load_chat()
    }
}
pub struct EngineCache<E> {
    engine: Option<E>,
    generation: u64,
}
impl<E: CachedEngine> EngineCache<E> {
    pub fn get_or_load(&mut self, chat: bool, load: impl FnOnce() -> Result<E>) -> Result<&E> {
        if self.engine.is_none() {
            self.engine = Some(load()?);
        }
        let engine = self.engine.as_ref().unwrap();
        if chat {
            engine.ensure_chat()?;
        }
        Ok(engine)
    }
    fn invalidate(&mut self) {
        self.engine = None;
    }
}
pub type Operations = OperationState<vz_core::engine::Engines>;
pub struct OperationState<E> {
    lock: Arc<Mutex<EngineCache<E>>>,
    generation: AtomicU64,
    cancel: watch::Sender<bool>,
}
impl<E> Default for OperationState<E> {
    fn default() -> Self {
        Self {
            lock: Arc::new(Mutex::new(EngineCache {
                engine: None,
                generation: 0,
            })),
            generation: AtomicU64::new(0),
            cancel: watch::channel(false).0,
        }
    }
}
impl<E: CachedEngine> OperationState<E> {
    pub fn acquire(&self) -> Result<OwnedMutexGuard<EngineCache<E>>, String> {
        let mut cache = self
            .lock
            .clone()
            .try_lock_owned()
            .map_err(|_| BUSY.to_owned())?;
        let generation = self.generation.load(Ordering::Acquire);
        if cache.generation != generation {
            cache.invalidate();
            cache.generation = generation;
        }
        Ok(cache)
    }
    pub fn acquire_mutation(&self) -> Result<OwnedMutexGuard<EngineCache<E>>, String> {
        let mut cache = self.acquire()?;
        cache.invalidate();
        Ok(cache)
    }
    pub fn acquire_import(&self, folder: &Path) -> Result<OwnedMutexGuard<EngineCache<E>>, String> {
        validate_import_folder(folder).map_err(|e| e.to_string())?;
        self.acquire_mutation()
    }
    // Reset only after acquiring the mutation lock; a refused call changes nothing.
    pub fn start(&self) -> watch::Receiver<bool> {
        self.cancel.send_replace(false);
        self.cancel.subscribe()
    }
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
        self.generation.fetch_add(1, Ordering::AcqRel);
        // During acquisition the cache is already empty. During inference it
        // must remain alive until the operation ends; acquire invalidates next.
        if let Ok(mut cache) = self.lock.try_lock() {
            cache.invalidate();
        }
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
    let value = selected_pins(path, total_ram_bytes()?)?;
    pins_from_value(&value)
}
pub fn pins_from_value(value: &Value) -> Result<Vec<Pin>> {
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

/// Resolve release tiers in the shell; the offline engine still consumes `chat`.
pub fn selected_pins(path: &Path, ram_bytes: u64) -> Result<Value> {
    let mut value: Value = serde_json::from_slice(&fs::read(path)?)?;
    if let Some(tiers) = value.get("chat_tiers") {
        let tiers = tiers.as_array().context("chat_tiers list")?;
        let mut chosen = None;
        let mut previous = None;
        for tier in tiers {
            let minimum = tier["min_ram_gb"].as_u64().context("RAM threshold")?;
            ensure!(
                previous.is_none_or(|p| minimum > p),
                "chat tiers must be ascending"
            );
            ensure!(
                previous.is_some() || minimum == 0,
                "first chat tier must start at zero"
            );
            previous = Some(minimum);
            if minimum <= ram_bytes.saturating_add(512 * 1024 * 1024) / (1024 * 1024 * 1024) {
                ensure!(tier["pin"].is_object(), "chat tier pin");
                chosen = Some(tier["pin"].clone());
            }
        }
        value["chat"] = chosen.context("no chat RAM tier")?;
        value.as_object_mut().unwrap().remove("chat_tiers");
    }
    Ok(value)
}

#[cfg(target_os = "macos")]
pub fn total_ram_bytes() -> Result<u64> {
    unsafe extern "C" {
        fn sysctlbyname(
            name: *const std::ffi::c_char,
            old: *mut std::ffi::c_void,
            len: *mut usize,
            new: *mut std::ffi::c_void,
            new_len: usize,
        ) -> i32;
    }
    let mut bytes = 0u64;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: constant terminated name and correctly sized writable output.
    let result = unsafe {
        sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut bytes as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    ensure!(result == 0 && len == 8, "Cannot read total physical RAM");
    Ok(bytes)
}
#[cfg(windows)]
pub fn total_ram_bytes() -> Result<u64> {
    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        values: [u64; 7],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
        fn GetPhysicallyInstalledSystemMemory(kib: *mut u64) -> i32;
    }
    let mut installed_kib = 0u64;
    // SAFETY: valid writable output; API returns installed RAM in KiB.
    if unsafe { GetPhysicallyInstalledSystemMemory(&mut installed_kib) } != 0 && installed_kib > 0 {
        return installed_kib
            .checked_mul(1024)
            .context("Installed RAM overflow");
    }
    let mut status = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        load: 0,
        values: [0; 7],
    };
    // SAFETY: MEMORYSTATUSEX layout and initialized length, valid writable pointer.
    ensure!(
        unsafe { GlobalMemoryStatusEx(&mut status) } != 0,
        "Cannot read total physical RAM"
    );
    Ok(status.values[0])
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn total_ram_bytes() -> Result<u64> {
    bail!("Physical RAM detection is supported on macOS and Windows")
}

/// Compare the complete recipe without loading or downloading a model. Reuse the
/// persisted dimension only for comparison; model name/hash still have to match.
pub fn needs_reindex(root: &Path, pins: &Value) -> Result<bool> {
    let path = root.join("recipe.json");
    if !path.try_exists()? {
        return vz_core::storage::State::has_persisted_documents(root);
    }
    let stored: vz_core::engine::Recipe = match serde_json::from_slice(&fs::read(path)?) {
        Ok(recipe) => recipe,
        Err(_) => return Ok(true),
    };
    Ok(stored != vz_core::engine::Recipe::from_pins(pins, stored.dimensions)?)
}

pub use vz_core::verification::regular;
pub fn verified(dir: &Path, pin: &Pin) -> bool {
    vz_core::verification::verify(&dir.join(&pin.file), &pin.sha256, Some(pin.size)).is_ok()
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
        // For import, dir is the typed source folder, not the installed models.
        if importing && regular(&dir.join(&pin.file), false).is_err() {
            continue;
        }
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
/// hash, and atomically replace. The target is never opened
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
    let before_hash = stage_file.metadata()?;
    let verified_file = stage_file.try_clone()?;
    drop(stage_file);
    progress(json!({"file": pin.file, "stage": "checking"}));
    let hash_path = stage.path.clone();
    let hash =
        tokio::task::spawn_blocking(move || vz_core::pipeline::file_sha256(&hash_path)).await??;
    ensure!(
        before_hash.len() == verified_file.metadata()?.len()
            && before_hash.modified()? == verified_file.metadata()?.modified()?,
        "model changed during verification"
    );
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
    vz_core::verification::record_verified(
        &dir.join(&pin.file),
        &pin.sha256,
        &verified_file,
        &before_hash,
    )?;
    progress(json!({"file": pin.file, "stage": "verified"}));
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
fn validate_import_folder(folder: &Path) -> Result<()> {
    ensure!(folder.is_absolute(), "Type an absolute folder path");
    ensure!(folder.is_dir(), "Import folder not found");
    Ok(())
}

pub async fn import(
    dir: &Path,
    pins: &[Pin],
    folder: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<Vec<Value>> {
    validate_import_folder(folder)?;
    fs::create_dir_all(dir)?;
    check_space(dir, space_needed(folder, pins, true)?)?;
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
