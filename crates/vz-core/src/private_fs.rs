//! Owner-only creation for WAL diagnostics.
use std::{fs::File, io, path::Path};

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
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use windows_sys::Win32::{
        Foundation::{LocalFree, INVALID_HANDLE_VALUE},
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::{CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE},
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
