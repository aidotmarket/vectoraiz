//! macOS extended ACLs are independent of POSIX mode bits. Fail closed on
//! existing ACEs (including deny-only ACLs); new empty objects lose inherited
//! ACEs before their handles are returned to writers.
use anyhow::{ensure, Result};
use std::{ffi::c_void, fs::File, os::fd::AsRawFd};

type Acl = *mut c_void;
const EXTENDED: i32 = 0x100;
unsafe extern "C" {
    fn acl_get_fd_np(fd: i32, kind: i32) -> Acl;
    fn acl_set_fd_np(fd: i32, acl: Acl, kind: i32) -> i32;
    fn acl_init(count: i32) -> Acl;
    fn acl_free(acl: Acl) -> i32;
    fn acl_get_entry(acl: Acl, entry_id: i32, entry: *mut *mut c_void) -> i32;
}
struct OwnedAcl(Acl);
impl OwnedAcl {
    fn new(acl: Acl) -> Result<Self> {
        ensure!(
            !acl.is_null(),
            "cannot read/create private storage ACL: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(acl))
    }
}
impl Drop for OwnedAcl {
    fn drop(&mut self) {
        // SAFETY: this pointer is owned and freed exactly once.
        unsafe {
            acl_free(self.0);
        }
    }
}
pub(super) fn check(file: &File) -> Result<()> {
    // SAFETY: live descriptor, extended ACL type, owned returned allocation.
    let raw = unsafe { acl_get_fd_np(file.as_raw_fd(), EXTENDED) };
    // Darwin reports ENOENT when the object has no extended ACL. Other
    // retrieval failures must fail closed.
    if raw.is_null() && std::io::Error::last_os_error().raw_os_error() == Some(2) {
        return Ok(());
    }
    let acl = OwnedAcl::new(raw)?;
    let mut entry = std::ptr::null_mut();
    // Darwin returns zero for an entry and -1 with EINVAL at the end. Use
    // acl_get_entry only to reject nonempty ACLs; distinguish errors with errno.
    let result = unsafe { acl_get_entry(acl.0, 0, &mut entry) };
    ensure!(
        result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(22),
        "private storage must have no extended ACL entries"
    );
    Ok(())
}
pub(super) fn clear(file: &File) -> Result<()> {
    // SAFETY: create an empty owned ACL and apply it to a live descriptor.
    let acl = OwnedAcl::new(unsafe { acl_init(0) })?;
    ensure!(
        unsafe { acl_set_fd_np(file.as_raw_fd(), acl.0, EXTENDED) } == 0,
        "cannot remove inherited private storage ACL: {}",
        std::io::Error::last_os_error()
    );
    check(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path, process::Command};
    fn grant(path: &Path, ace: &str) {
        assert!(Command::new("chmod")
            .args(["+a", ace])
            .arg(path)
            .status()
            .unwrap()
            .success());
    }
    #[test]
    fn explicit_acl_is_rejected_despite_private_modes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let dir = root.join("private");
        crate::review_store::private_directory(&dir).unwrap();
        grant(&dir, "everyone allow read,search");
        assert!(crate::review_store::private_directory(&dir).is_err());
        let file = root.join("record");
        drop(crate::review_store::private_file(&file).unwrap());
        grant(&file, "everyone allow read");
        assert!(crate::review_store::check_private(&file).is_err());
    }
    #[test]
    fn inherited_acls_are_rejected_or_removed_before_private_writes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        grant(
            &root,
            "everyone allow read,search,file_inherit,directory_inherit",
        );
        let existing = root.join("existing");
        fs::create_dir(&existing).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(crate::review_store::private_directory(&existing).is_err());
        let fresh = root.join("fresh");
        crate::review_store::private_directory(&fresh).unwrap();
        check(&File::open(&fresh).unwrap()).unwrap();
        let file = crate::review_store::private_file(&root.join("fresh-record")).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 0);
        check(&file).unwrap();
        drop(file);
        // Synthetic ACE checks only. Cross-account read/traverse verification
        // on installed macOS remains a by-hand qualification step.
    }
}
