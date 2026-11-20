//! Owner-only creation for private validation records and WAL diagnostics.
use std::{fs::File, io, path::Path};

pub(crate) fn directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "insecure validation directory",
            ));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        windows::directory(path)
    }
}

pub(crate) fn create_file(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(windows)]
    {
        windows::create_file(path)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt, io::FromRawHandle};
    use windows_sys::Win32::{
        Foundation::{LocalFree, INVALID_HANDLE_VALUE},
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SetFileSecurityW,
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::{
            CreateDirectoryW, CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_GENERIC_WRITE,
        },
    };
    struct Security(*mut core::ffi::c_void);
    impl Drop for Security {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    impl Security {
        fn new() -> io::Result<Self> {
            // Protected DACL: only the object's owner has access. Inherit on
            // child files/directories; no inherited broad parent grants.
            let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)\0".encode_utf16().collect();
            let mut descriptor = std::ptr::null_mut();
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(descriptor))
        }
        fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.0,
                bInheritHandle: 0,
            }
        }
    }
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    pub(super) fn directory(path: &Path) -> io::Result<()> {
        let security = Security::new()?;
        let name = wide(path);
        let attrs = security.attributes();
        if unsafe { CreateDirectoryW(name.as_ptr(), &attrs) } == 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "insecure validation directory",
            ));
        }
        if unsafe {
            SetFileSecurityW(
                name.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                security.0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub(super) fn create_file(path: &Path) -> io::Result<File> {
        let security = Security::new()?;
        let attrs = security.attributes();
        let handle = unsafe {
            CreateFileW(
                wide(path).as_ptr(),
                FILE_GENERIC_WRITE,
                0,
                &attrs,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_handle(handle) })
    }
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn marker_directory_rejects_insecure_and_symlinked_locations() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("validation");
        super::directory(&cache).unwrap();
        assert_eq!(
            std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(super::directory(&cache).is_err());
        let link = root.path().join("linked");
        symlink(&cache, &link).unwrap();
        assert!(super::directory(&link).is_err());
    }
}
