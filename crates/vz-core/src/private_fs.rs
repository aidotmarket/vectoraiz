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
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
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
            use windows_sys::Win32::Security::{Authorization::ConvertSidToStringSidW, TOKEN_USER};
            let user = current_user()?;
            let mut sid_string = std::ptr::null_mut();
            if unsafe {
                ConvertSidToStringSidW(
                    (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid,
                    &mut sid_string,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let _sid_string = Security(sid_string.cast());
            let mut length = 0;
            unsafe {
                while *sid_string.add(length) != 0 {
                    length += 1;
                }
            }
            let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_string, length) })
                .map_err(io::Error::other)?;
            // Set the owner explicitly: elevated tokens can otherwise default
            // new objects to an administrator group rather than this user.
            let sddl: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;OW)\0")
                .encode_utf16()
                .collect();
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
    fn current_user() -> io::Result<Vec<usize>> {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            Security::{GetTokenInformation, TokenUser, TOKEN_QUERY},
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        };
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut size = 0;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
            // usize backing keeps TOKEN_USER suitably aligned.
            let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
            let ok =
                GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size);
            CloseHandle(token);
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(data)
        }
    }

    // Reject unknown ACE forms conservatively. Only the current user (or the
    // owner-rights SID, after checking the owner) may hold any allowed access.
    fn trusted(path: &Path) -> io::Result<bool> {
        use windows_sys::Win32::Security::{
            Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
            EqualSid, GetAce, ACCESS_ALLOWED_ACE, ACL, OWNER_SECURITY_INFORMATION, TOKEN_USER,
        };
        unsafe {
            let mut owner = std::ptr::null_mut();
            let mut acl: *mut ACL = std::ptr::null_mut();
            let mut descriptor = std::ptr::null_mut();
            let error = GetNamedSecurityInfoW(
                wide(path).as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut acl,
                std::ptr::null_mut(),
                &mut descriptor,
            );
            if error != 0 {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            let _descriptor = Security(descriptor);
            let data = current_user()?;
            let user = (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid;
            if owner.is_null() || EqualSid(owner, user) == 0 || acl.is_null() {
                return Ok(false);
            }
            // S-1-3-4 = OWNER RIGHTS, used by our protected owner-only DACL.
            let owner_rights = [1u32 | (1 << 8), 3u32 << 24, 4u32];
            for i in 0..(*acl).AceCount {
                let mut ace = std::ptr::null_mut();
                if GetAce(acl, i as u32, &mut ace) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let allowed = &*(ace.cast::<ACCESS_ALLOWED_ACE>());
                match allowed.Header.AceType {
                    1 => continue, // deny ACE cannot grant access
                    0 => {
                        let sid = std::ptr::addr_of!(allowed.SidStart).cast_mut().cast();
                        if EqualSid(sid, user) == 0
                            && EqualSid(sid, owner_rights.as_ptr().cast_mut().cast()) == 0
                        {
                            return Ok(false);
                        }
                    }
                    _ => return Ok(false),
                }
            }
            Ok(true)
        }
    }

    pub(super) fn directory(path: &Path) -> io::Result<()> {
        // A second reader must not observe the hardened ACL before stale
        // markers have been removed by the first reader.
        static CHECK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = CHECK
            .lock()
            .map_err(|_| io::Error::other("validation directory check failed"))?;
        let security = Security::new()?;
        let name = wide(path);
        let attrs = security.attributes();
        let existed = unsafe { CreateDirectoryW(name.as_ptr(), &attrs) } == 0;
        if existed {
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
        let tainted = path.join(".untrusted");
        let clean = match std::fs::symlink_metadata(&tainted) {
            Ok(_) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => true,
            Err(error) => return Err(error),
        };
        let was_trusted = !existed || (clean && trusted(path)?);
        // create_new never follows an attacker-supplied sentinel symlink. The
        // Windows handle denies sharing/deletion until hardening is complete.
        let taint_handle = if !was_trusted {
            match create_file(&tainted) {
                Ok(file) => Some(file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        if unsafe {
            SetFileSecurityW(
                name.as_ptr(),
                DACL_SECURITY_INFORMATION
                    | OWNER_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION,
                security.0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if !was_trusted {
            // Remove untrusted contents after hardening. Otherwise a later
            // call could trust a forged marker merely because we reset the ACL.
            // Hardening first prevents broad-principal writes during deletion.
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                if entry.path() != tainted {
                    std::fs::remove_file(entry.path())?;
                }
            }
            drop(taint_handle);
            std::fs::remove_file(&tainted)?;
        }
        if !was_trusted {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "ignored markers from insecure validation directory",
            ));
        }
        Ok(())
    }
    #[test]
    fn existing_broad_dacl_discards_markers_before_hardening() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("validation");
        directory(&cache).unwrap();
        std::fs::write(cache.join("forged.json"), "matching forged marker").unwrap();
        let sddl: Vec<u16> = "D:P(A;OICI;FA;;;WD)\0".encode_utf16().collect();
        let mut descriptor = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let descriptor = Security(descriptor);
        assert_ne!(
            unsafe {
                SetFileSecurityW(
                    wide(&cache).as_ptr(),
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    descriptor.0,
                )
            },
            0
        );
        assert!(!trusted(&cache).unwrap());
        assert!(directory(&cache).is_err());
        assert!(!cache.join("forged.json").exists());
        directory(&cache).unwrap();
        assert!(trusted(&cache).unwrap());
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
