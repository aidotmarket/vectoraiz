use std::{io, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R, cap: usize) -> io::Result<Vec<u8>> {
    let len = r.read_u32_le().await? as usize;
    if len > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds limit",
        ));
    }
    let mut bytes = vec![0; len];
    r.read_exact(&mut bytes).await?;
    Ok(bytes)
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    bytes: &[u8],
    cap: usize,
) -> io::Result<()> {
    if bytes.len() > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds limit",
        ));
    }
    w.write_u32_le(bytes.len() as u32).await?;
    w.write_all(bytes).await
}

pub async fn call(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut stream = connect().await?;
    write_frame(&mut stream, bytes, crate::REQUEST_CAP).await?;
    tokio::time::timeout(
        Duration::from_secs(30),
        read_frame(&mut stream, crate::RESPONSE_CAP),
    )
    .await?
}

#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;
#[cfg(unix)]
pub type Listener = tokio::net::UnixListener;

#[cfg(unix)]
pub fn path() -> io::Result<std::path::PathBuf> {
    Ok(std::path::PathBuf::from(
        std::env::var_os("HOME").ok_or_else(|| io::Error::other("home unavailable"))?,
    )
    .join("Library/Application Support/vectorAIz/mcp.sock"))
}
#[cfg(unix)]
pub async fn connect() -> io::Result<Stream> {
    Stream::connect(path()?).await
}

#[cfg(unix)]
pub async fn bind_at(path: &std::path::Path) -> io::Result<Listener> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("missing parent"))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if path.try_exists()? {
        if Stream::connect(path).await.is_ok() {
            return Err(io::Error::new(io::ErrorKind::AddrInUse, "channel in use"));
        }
        std::fs::remove_file(path)?;
    }
    let listener = Listener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}
#[cfg(unix)]
pub async fn bind() -> io::Result<Listener> {
    bind_at(&path()?).await
}

#[cfg(unix)]
pub fn same_user(peer: u32) -> bool {
    // SAFETY: geteuid has no preconditions.
    peer == unsafe { libc::geteuid() }
}
#[cfg(target_os = "macos")]
pub fn verify_peer(stream: &Stream) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: valid socket fd and correctly sized writable output pointers.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if !same_user(uid) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "peer mismatch",
        ));
    }
    Ok(())
}
#[cfg(all(unix, not(target_os = "macos")))]
pub fn verify_peer(stream: &Stream) -> io::Result<()> {
    if !same_user(stream.peer_cred()?.uid()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "peer mismatch",
        ));
    }
    Ok(())
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn caps_and_partial_frames() {
        for cap in [crate::REQUEST_CAP, crate::RESPONSE_CAP] {
            let mut oversized = ((cap + 1) as u32).to_le_bytes().as_slice().to_vec();
            assert!(read_frame(&mut oversized.as_slice(), cap).await.is_err());
            oversized = 4u32.to_le_bytes().to_vec();
            oversized.push(1);
            assert!(read_frame(&mut oversized.as_slice(), cap).await.is_err());
            let mut out = vec![];
            write_frame(&mut out, b"hello", cap).await.unwrap();
            assert_eq!(
                read_frame(&mut out.as_slice(), cap).await.unwrap(),
                b"hello"
            );
            assert!(write_frame(&mut out, &vec![0; cap + 1], cap).await.is_err());
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn modes_peer_and_live_socket() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("channel/mcp.sock");
        let listener = bind_at(&path).await.unwrap();
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let client = Stream::connect(&path).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        verify_peer(&server).unwrap();
        assert!(!same_user(u32::MAX));
        assert!(bind_at(&path).await.is_err());
        drop(client);
        drop(server);
        drop(listener);
        bind_at(&path).await.unwrap();
    }
}
