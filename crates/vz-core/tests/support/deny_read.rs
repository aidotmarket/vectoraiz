// Keep the file, size and mtime intact while making read opens fail.
#[cfg(unix)]
fn deny_read(path: &std::path::Path) -> impl Drop {
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
    struct Restore(PathBuf, fs::Permissions);
    impl Drop for Restore {
        fn drop(&mut self) {
            fs::set_permissions(&self.0, self.1.clone()).unwrap();
        }
    }
    let restore = Restore(path.to_owned(), path.metadata().unwrap().permissions());
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
    restore
}

#[cfg(windows)]
fn deny_read(path: &std::path::Path) -> std::fs::File {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    OpenOptions::new().read(true).share_mode(0).open(path).unwrap()
}
