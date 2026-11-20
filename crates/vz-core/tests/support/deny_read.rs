// Keep the file, size and mtime intact while making read opens fail.
#[cfg(unix)]
struct DenyRead(std::path::PathBuf, std::fs::Permissions);

#[cfg(unix)]
impl Drop for DenyRead {
    fn drop(&mut self) {
        std::fs::set_permissions(&self.0, self.1.clone()).unwrap();
    }
}

#[cfg(unix)]
fn deny_read(path: &std::path::Path) -> DenyRead {
    use std::{fs, os::unix::fs::PermissionsExt};
    let restore = DenyRead(path.to_owned(), path.metadata().unwrap().permissions());
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
    restore
}

#[cfg(windows)]
type DenyRead = std::fs::File;

#[cfg(windows)]
fn deny_read(path: &std::path::Path) -> DenyRead {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    OpenOptions::new().read(true).share_mode(0).open(path).unwrap()
}
