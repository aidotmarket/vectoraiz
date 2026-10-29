//! Gateway export: verify outside the source, then publish with one directory rename.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    hash::{BuildHasher, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The server may have published the pack, but its rename acknowledgement was lost.
/// CLI callers must distinguish this from ordinary failure using exit code 3.
#[derive(Debug)]
pub struct PublicationOutcomeUnknown {
    destination: String,
    source: String,
    staging: String,
    id: String,
    cause: russh_sftp::client::error::Error,
}

impl std::fmt::Display for PublicationOutcomeUnknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SFTP publication outcome unknown: publication may or may not have happened at {}; list {}/vz/{}; if it exists with all files, the export completed; if not, the pack is still in {}/{}.partial-* and a retry with the same pack id is safe", self.destination, self.source, self.id, self.staging, self.id)
    }
}

impl std::error::Error for PublicationOutcomeUnknown {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Export CLI exit codes: 1 for ordinary failure, 3 for uncertain publication.
pub fn exit_code(error: &anyhow::Error) -> i32 {
    if error.is::<PublicationOutcomeUnknown>() {
        3
    } else {
        1
    }
}

#[derive(Clone, Debug)]
pub struct PackFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

fn hash_file(path: &Path) -> Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((size, format!("{:x}", hash.finalize())))
}

fn regular(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    !metadata.file_type().is_symlink()
}

/// Include the manifest itself; refuse links, special files and unmanifested payloads.
fn inventory(pack: &Path, id: &str) -> Result<Vec<PackFile>> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "pack-id must contain only ASCII letters, digits, '-' or '_' (1–128 characters)"
    );
    ensure!(
        regular(&fs::symlink_metadata(pack)?),
        "pack must not be a link"
    );
    let manifest = crate::verify(pack).context("pack verification failed; export refused")?;
    let expected = manifest["files"].as_object().context("manifest files")?;
    let mut files = BTreeMap::new();
    for entry in walkdir::WalkDir::new(pack).min_depth(1) {
        let entry = entry?;
        ensure!(
            regular(&fs::symlink_metadata(entry.path())?),
            "pack links/reparse points are not supported"
        );
        if entry.file_type().is_dir() {
            continue;
        }
        ensure!(
            entry.file_type().is_file(),
            "pack contains a non-regular file"
        );
        let mut parts = Vec::new();
        for component in entry.path().strip_prefix(pack)?.components() {
            let std::path::Component::Normal(part) = component else {
                anyhow::bail!("unsafe pack path");
            };
            let part = part.to_str().context("pack paths must be UTF-8")?;
            ensure!(!part.contains(['\\', ':']), "unsafe pack path");
            parts.push(part);
        }
        let name = parts.join("/");
        let (size, hash) = hash_file(entry.path())?;
        if name != "manifest.json" {
            ensure!(
                expected.get(&name).and_then(Value::as_str) == Some(hash.as_str()),
                "unmanifested or changed pack file: {name}"
            );
        }
        files.insert(
            name.clone(),
            PackFile {
                path: name,
                size,
                sha256: hash,
            },
        );
    }
    ensure!(
        files.len() == expected.len() + 1,
        "pack inventory does not match manifest"
    );
    Ok(files.into_values().collect())
}

fn partial_name(id: &str) -> String {
    // RandomState seeds each hasher from OS randomness, without another dependency.
    let random = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("{id}.partial-{random:016x}")
}

fn report(id: &str, mode: &str, files: &[PackFile], destination: String, start: Instant) -> Value {
    json!({"pack_id": id, "mode": mode, "files": files.iter().map(|f| json!({"path":f.path,"size":f.size,"sha256":f.sha256})).collect::<Vec<_>>(), "destination":destination, "elapsed_ms": start.elapsed().as_millis()})
}

#[cfg(unix)]
fn same_filesystem(a: &Path, b: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::metadata(a)?.dev() == fs::metadata(b)?.dev())
}

#[cfg(windows)]
fn same_filesystem(a: &Path, b: &Path) -> Result<bool> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    };
    fn volume(path: &Path) -> Result<u32> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // The directory handle is live and the output buffer has the required size.
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } != 0,
            "cannot read volume serial: {}",
            std::io::Error::last_os_error()
        );
        Ok(unsafe { info.assume_init() }.dwVolumeSerialNumber)
    }
    Ok(volume(a)? == volume(b)?)
}

/// Export to existing source/staging directories. Interrupted partials are left alone.
pub fn folder(pack: &Path, id: &str, source: &Path, staging: &Path) -> Result<Value> {
    folder_inner(
        pack,
        id,
        source,
        staging,
        same_filesystem,
        |_| Ok(()),
        || Ok(()),
    )
}

/// Fault injection seam for export acceptance tests. Normal callers use `folder`.
#[doc(hidden)]
pub fn folder_with_checks(
    pack: &Path,
    id: &str,
    source: &Path,
    staging: &Path,
    same_device: bool,
    before_rename: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    folder_inner(
        pack,
        id,
        source,
        staging,
        |_, _| Ok(same_device),
        |_| Ok(()),
        before_rename,
    )
}

/// Fault injection after copy and before staged verification. Normal callers use `folder`.
#[doc(hidden)]
pub fn folder_with_staged_check(
    pack: &Path,
    id: &str,
    source: &Path,
    staging: &Path,
    after_copy: impl FnOnce(&Path) -> Result<()>,
) -> Result<Value> {
    folder_inner(
        pack,
        id,
        source,
        staging,
        same_filesystem,
        after_copy,
        || Ok(()),
    )
}

fn folder_inner(
    pack: &Path,
    id: &str,
    source: &Path,
    staging: &Path,
    device_check: impl FnOnce(&Path, &Path) -> Result<bool>,
    after_copy: impl FnOnce(&Path) -> Result<()>,
    before_rename: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    let start = Instant::now();
    let files = inventory(pack, id)?;
    let source = fs::canonicalize(source).context("source must be an existing directory")?;
    let staging = fs::canonicalize(staging).context("staging must be an existing directory")?;
    ensure!(
        source.is_dir() && staging.is_dir(),
        "source and staging must be directories"
    );
    ensure!(
        !staging.starts_with(&source),
        "staging must be outside every gateway source"
    );
    ensure!(
        device_check(&source, &staging)?,
        "staging and source are on different filesystems; one atomic rename is required"
    );
    let parent = source.join("vz");
    match fs::create_dir(&parent) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    ensure!(
        regular(&fs::symlink_metadata(&parent)?) && parent.is_dir(),
        "source/vz must be a directory, not a link"
    );
    ensure!(
        same_filesystem(&parent, &staging)?,
        "source/vz and staging are on different filesystems"
    );
    let destination = parent.join(id);
    ensure!(
        fs::symlink_metadata(&destination).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "export destination already exists or cannot be inspected"
    );
    let stage = staging.join(partial_name(id));
    fs::create_dir(&stage)?;
    for file in &files {
        let target = stage.join(&file.path);
        fs::create_dir_all(target.parent().context("file parent")?)?;
        let mut input = File::open(pack.join(&file.path))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
    }
    after_copy(&stage)?;
    for file in &files {
        let target = stage.join(&file.path);
        ensure!(
            hash_file(&target)? == (file.size, file.sha256.clone()),
            "staged file verification failed: {}",
            file.path
        );
    }
    // Flush directory entries as well on Unix, including nested example directories.
    #[cfg(unix)]
    for entry in walkdir::WalkDir::new(&stage).contents_first(true) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            File::open(entry.path())?.sync_all()?;
        }
    }
    before_rename()?;
    ensure!(!destination.exists(), "export destination already exists");
    publish_folder(&stage, &destination).map_err(|error| {
        let message = folder_rename_error(&error);
        anyhow::Error::new(error).context(message)
    })?;
    Ok(report(
        id,
        "folder",
        &files,
        destination.to_string_lossy().into_owned(),
        start,
    ))
}

fn folder_rename_error(error: &std::io::Error) -> &'static str {
    #[cfg(target_os = "macos")]
    let unsupported = matches!(error.raw_os_error(), Some(45 | 22)); // ENOTSUP, EINVAL
    #[cfg(target_os = "linux")]
    let unsupported = matches!(error.raw_os_error(), Some(95 | 22 | 38)); // ENOTSUP, EINVAL, ENOSYS
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let unsupported = false;
    if unsupported {
        "filesystem does not support an exclusive (no-replace) rename; nothing was copied into the source; the pack is in staging"
    } else if error.kind() == std::io::ErrorKind::CrossesDevices {
        "atomic export rename failed (cross-device/EXDEV/ERROR_NOT_SAME_DEVICE); nothing was copied into the source; choose staging on the source filesystem"
    } else {
        "atomic export rename failed; nothing was copied into the source; the pack is in staging"
    }
}

// Refuse a destination created by another exporter between inspection and rename.
// Both native calls perform a single, non-replacing directory rename.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn publish_folder(stage: &Path, destination: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(stage.as_os_str().as_bytes())?;
    let to = CString::new(destination.as_os_str().as_bytes())?;
    #[cfg(target_os = "macos")]
    let result = {
        unsafe extern "C" {
            fn renamex_np(
                from: *const std::ffi::c_char,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        // RENAME_EXCL: fail if the destination exists.
        unsafe { renamex_np(from.as_ptr(), to.as_ptr(), 4) }
    };
    #[cfg(target_os = "linux")]
    let result = {
        unsafe extern "C" {
            fn renameat2(
                from_fd: i32,
                from: *const std::ffi::c_char,
                to_fd: i32,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        // AT_FDCWD, RENAME_NOREPLACE.
        unsafe { renameat2(-100, from.as_ptr(), -100, to.as_ptr(), 1) }
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn publish_folder(stage: &Path, destination: &Path) -> std::io::Result<()> {
    // Windows refuses replacing an existing directory with MoveFileExW.
    fs::rename(stage, destination)
}

/// Absolute POSIX path with lexical dot/dot-dot normalization.
pub fn normalize_remote(path: &str) -> Result<String> {
    ensure!(
        path.starts_with('/') && !path.contains('\0') && !path.contains('\\'),
        "SFTP paths must be absolute POSIX paths"
    );
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn outside(source: &str, staging: &str) -> Result<()> {
    ensure!(
        source != "/" && staging != source && !staging.starts_with(&format!("{source}/")),
        "staging must be outside every gateway source"
    );
    Ok(())
}

pub struct SftpOptions {
    pub target: String,
    pub staging: String,
    pub identity: Option<PathBuf>,
    pub known_hosts: Option<PathBuf>,
}

fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .context("home directory unavailable")?,
    ))
}

pub fn default_known_hosts() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = home()?.join("Library/Application Support/com.vectoraiz.native");
    #[cfg(windows)]
    let base = PathBuf::from(std::env::var_os("APPDATA").context("APPDATA unavailable")?)
        .join("com.vectoraiz.native");
    #[cfg(not(any(target_os = "macos", windows)))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".local/share"))
        .join("com.vectoraiz.native");
    Ok(base.join("gateway-known-hosts.json"))
}

struct PinnedHost {
    host: String,
    file: PathBuf,
}

impl russh::client::Handler for PinnedHost {
    type Error = anyhow::Error;
    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool> {
        let fingerprint = key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();
        // Serialize first-use updates with an exclusive lock; never overwrite another pin.
        if let Some(parent) = self.file.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let lock_path = self.file.with_extension("lock");
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .context("cannot open host pin lock")?;
        // OS locks are released on process death, so an interrupted first use
        // cannot leave a stale lock that prevents a later export.
        lock.lock().context("cannot lock host pin file")?;
        let result = (|| {
            let mut pins: BTreeMap<String, String> = match fs::read(&self.file) {
                Ok(bytes) => serde_json::from_slice(&bytes).context("invalid host pin file")?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(e) => return Err(e.into()),
            };
            if let Some(pin) = pins.get(&self.host) {
                ensure!(
                    pin == &fingerprint,
                    "SFTP host key mismatch for {}; refusing connection",
                    self.host
                );
            } else {
                pins.insert(self.host.clone(), fingerprint.clone());
                let temporary = self.file.with_extension(partial_name("pins"));
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                output.write_all(&serde_json::to_vec_pretty(&pins)?)?;
                output.sync_all()?;
                drop(output);
                fs::rename(&temporary, &self.file)?;
                eprintln!("Pinned SFTP host key for {}: {fingerprint}", self.host);
            }
            Ok(true)
        })();
        drop(lock);
        result
    }
}

/// Key-only SFTP export. No shell commands or password/agent authentication.
pub async fn sftp(pack: &Path, id: &str, options: &SftpOptions) -> Result<Value> {
    let start = Instant::now();
    let files = inventory(pack, id)?;
    let (authority, path) = options
        .target
        .split_once(":/")
        .context("SFTP target must be user@host[:port]:/source/path")?;
    let (user, address) = authority
        .split_once('@')
        .context("SFTP target requires user@host")?;
    ensure!(
        !user.is_empty()
            && !address.is_empty()
            && !user.contains([':', '/', '@'])
            && !address.contains(['/', '@']),
        "invalid SFTP user or host"
    );
    let (host, port) = match address.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().context("invalid SFTP port")?),
        None => (address, 22),
    };
    let source = normalize_remote(&format!("/{path}"))?;
    let staging = normalize_remote(&options.staging)?;
    outside(&source, &staging)?;
    let identity = match &options.identity {
        Some(path) => path.clone(),
        None => home()?.join(".ssh/id_ed25519"),
    };
    let key = russh::keys::load_secret_key(&identity, None).context("cannot load identity: Phase 0 requires a valid, unencrypted private key; encrypted keys/passphrases are not supported")?;
    let handler = PinnedHost {
        host: format!("{host}:{port}"),
        file: match &options.known_hosts {
            Some(path) => path.clone(),
            None => default_known_hosts()?,
        },
    };
    let config = russh::client::Config {
        inactivity_timeout: Some(Duration::from_secs(120)),
        ..Default::default()
    };
    let mut session = russh::client::connect(Arc::new(config), (host, port), handler).await?;
    let algorithm = session.best_supported_rsa_hash().await?.flatten();
    ensure!(
        session
            .authenticate_publickey(
                user,
                russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), algorithm)
            )
            .await?
            .success(),
        "SFTP public-key authentication failed"
    );
    let channel = session.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let sftp = russh_sftp::client::SftpSession::new(channel.into_stream()).await?;
    // Resolve remote symlinks too: lexical checks alone cannot detect an alias into source.
    let source = normalize_remote(&sftp.canonicalize(source).await?)?;
    let staging = normalize_remote(&sftp.canonicalize(staging).await?)?;
    outside(&source, &staging)?;
    let parent = format!("{}/vz", source.trim_end_matches('/'));
    if !sftp.try_exists(&parent).await? {
        sftp.create_dir(&parent).await?;
    }
    ensure!(
        normalize_remote(&sftp.canonicalize(&parent).await?)? == parent,
        "source/vz must not be a symlink"
    );
    let destination = format!("{parent}/{id}");
    ensure!(
        !sftp.try_exists(&destination).await?,
        "export destination already exists"
    );
    let stage = format!("{}/{}", staging.trim_end_matches('/'), partial_name(id));
    sftp.create_dir(&stage).await?;
    for file in &files {
        let mut directory = stage.clone();
        let parts = file.path.split('/').collect::<Vec<_>>();
        for part in &parts[..parts.len() - 1] {
            directory.push('/');
            directory.push_str(part);
            if !sftp.try_exists(&directory).await? {
                sftp.create_dir(&directory).await?;
            }
        }
        let target = format!("{stage}/{}", file.path);
        let mut input = File::open(pack.join(&file.path))?;
        let mut remote = sftp.create(&target).await?;
        let mut buffer = [0; 65536];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            remote.write_all(&buffer[..n]).await?;
        }
        remote.shutdown().await?;
        ensure!(
            sftp.metadata(&target).await?.size == Some(file.size),
            "remote size mismatch: {}",
            file.path
        );
        let mut remote = sftp.open(&target).await?;
        let mut hash = Sha256::new();
        loop {
            let n = remote.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        ensure!(
            format!("{:x}", hash.finalize()) == file.sha256,
            "remote hash mismatch: {}",
            file.path
        );
        remote.shutdown().await?;
    }
    // SFTP v3 RENAME must fail if the destination exists; do not use posix-rename
    // extensions, which overwrite an existing destination.
    let destination_url = format!("sftp://{user}@{host}:{port}{destination}");
    if let Err(error) = sftp.rename(&stage, &destination).await {
        // Only a status reply explicitly rejects RENAME. All other variants can
        // represent a lost reply after the server committed publication.
        if matches!(error, russh_sftp::client::error::Error::Status(_)) {
            return Err(anyhow::Error::new(error).context("atomic SFTP rename failed (possibly cross-device); nothing was copied into the source; choose staging on the source filesystem"));
        }
        return Err(PublicationOutcomeUnknown {
            destination: destination_url,
            source,
            staging,
            id: id.into(),
            cause: error,
        }
        .into());
    }
    // The acknowledged rename is the commit boundary; cleanup is best effort.
    if let Err(error) = sftp.close().await {
        eprintln!("Warning: SFTP export completed, but closing the session failed: {error}");
    }
    Ok(report(id, "sftp", &files, destination_url, start))
}

#[cfg(test)]
mod tests {
    use super::folder_rename_error;
    use std::io::{Error, ErrorKind};

    #[test]
    fn rename_diagnostics_distinguish_cross_device_from_other_failures() {
        let cross_device = folder_rename_error(&Error::from(ErrorKind::CrossesDevices));
        assert!(cross_device.contains("cross-device"));
        let ordinary = folder_rename_error(&Error::from(ErrorKind::PermissionDenied));
        assert!(!ordinary.contains("cross-device"));
        assert!(ordinary.contains("the pack is in staging"));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn unsupported_exclusive_rename_has_actionable_diagnostic() {
        #[cfg(target_os = "macos")]
        let codes = [45, 22]; // ENOTSUP, EINVAL
        #[cfg(target_os = "linux")]
        let codes = [95, 22, 38]; // ENOTSUP, EINVAL, ENOSYS
        for code in codes {
            let message = folder_rename_error(&Error::from_raw_os_error(code));
            assert!(message.contains("does not support an exclusive (no-replace) rename"));
            assert!(message.contains("nothing was copied into the source"));
            assert!(message.contains("the pack is in staging"));
            assert!(!message.contains("cross-device"));
        }
    }
}
