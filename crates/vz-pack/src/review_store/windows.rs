//! Private review objects are born with a protected, current-user-only DACL.
//! Existing objects are inspected, never repaired by broadening/replacing ACLs.
use anyhow::{ensure, Result};
use std::{
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetNamedSecurityInfoW, SE_FILE_OBJECT,
        },
        GetAce, GetSecurityDescriptorControl, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
        SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        CREATE_NEW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn wide(path: &Path) -> Result<Vec<u16>> {
    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
    ensure!(!value.contains(&0), "NUL in review path");
    value.push(0);
    Ok(value)
}
unsafe fn sid_text(sid: *mut core::ffi::c_void) -> Result<String> {
    let mut text = ptr::null_mut();
    if ConvertSidToStringSidW(sid, &mut text) == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let _allocated = Local(text.cast());
    let mut len = 0;
    while *text.add(len) != 0 {
        len += 1;
    }
    Ok(String::from_utf16(std::slice::from_raw_parts(text, len))?)
}
fn current_sid() -> Result<String> {
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let token = Handle(token);
        let mut size = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
        ensure!(size > 0, "token user unavailable");
        let mut storage = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        sid_text((*storage.as_ptr().cast::<TOKEN_USER>()).User.Sid)
    }
}
fn descriptor(directory: bool) -> Result<Local> {
    let sid = current_sid()?;
    let flags = if directory { "OICI" } else { "" };
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;{flags};FA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let mut descriptor = ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Local(descriptor))
    }
}
pub(super) fn create_directory(path: &Path) -> Result<()> {
    let path = wide(path)?;
    let descriptor = descriptor(true)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    unsafe {
        if CreateDirectoryW(path.as_ptr(), &attributes) == 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    Ok(())
}
pub(super) fn create_file(path: &Path) -> Result<File> {
    let descriptor = descriptor(false)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // create_new refuses existing objects, including reparse points. The DACL
    // is supplied to CreateFile atomically, before any private byte is written.
    let name = wide(path)?;
    let file = unsafe {
        let handle = CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            CREATE_NEW,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error().into());
        }
        File::from_raw_handle(handle)
    };
    check_private(path)?;
    Ok(file)
}
pub(super) fn check_private(path: &Path) -> Result<()> {
    let directory = std::fs::symlink_metadata(path)?.is_dir();
    let path = wide(path)?;
    let sid = current_sid()?;
    unsafe {
        let mut owner = ptr::null_mut();
        let mut acl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        let result = GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        );
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32).into());
        }
        let _descriptor = Local(descriptor);
        ensure!(
            !owner.is_null() && sid_text(owner)? == sid,
            "review owner must be current user"
        );
        let mut control = 0;
        let mut revision = 0;
        ensure!(
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) != 0,
            "unreadable review security descriptor"
        );
        ensure!(
            control & SE_DACL_PROTECTED != 0 && !acl.is_null(),
            "review DACL must be protected and non-null"
        );
        ensure!(
            (*acl).AceCount == 1,
            "review DACL must grant only its owner"
        );
        let mut ace = ptr::null_mut();
        ensure!(GetAce(acl, 0, &mut ace) != 0, "unreadable review ACE");
        let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
        ensure!(
            ace.Header.AceType == 0
                && ace.Header.AceFlags == if directory { 3 } else { 0 }
                && ace.Mask == FILE_ALL_ACCESS
                && sid_text(ptr::addr_of!(ace.SidStart).cast_mut().cast())? == sid,
            "review DACL is not owner-only full access"
        );
    }
    Ok(())
}
pub(super) fn check_node(path: &Path, regular: bool) -> Result<()> {
    ensure!(
        std::fs::symlink_metadata(path)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "review reparse point forbidden"
    );
    if regular {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        unsafe {
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            ensure!(
                GetFileInformationByHandle(file.as_raw_handle(), &mut info) != 0,
                "review link count unavailable"
            );
            ensure!(
                info.nNumberOfLinks == 1
                    && info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
                "linked review input forbidden"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn protected_owner_only_objects_survive_rename_and_reopen() {
        let root = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "vz-private-acl-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        create_directory(&root).unwrap();
        check_private(&root).unwrap();
        let stage = root.join("partial");
        create_directory(&stage).unwrap();
        let path = stage.join("record.json");
        let mut file = create_file(&path).unwrap();
        file.write_all(b"private record").unwrap();
        file.sync_all().unwrap();
        drop(file);
        check_private(&path).unwrap();
        check_node(&path, true).unwrap();
        assert!(create_file(&path).is_err());
        std::fs::hard_link(&path, stage.join("hardlink.json")).unwrap();
        assert!(check_node(&path, true).is_err());
        std::fs::remove_file(stage.join("hardlink.json")).unwrap();
        let promoted = root.join("generation");
        std::fs::rename(&stage, &promoted).unwrap();
        check_private(&promoted).unwrap();
        check_private(&promoted.join("record.json")).unwrap();
        assert_eq!(
            std::fs::read(promoted.join("record.json")).unwrap(),
            b"private record"
        );

        // A present NULL DACL grants everyone access; refuse it rather than
        // attempting an ACL repair on already exposed evidence.
        use windows_sys::Win32::Security::{
            Authorization::SetNamedSecurityInfoW, PROTECTED_DACL_SECURITY_INFORMATION,
        };
        let name = wide(&promoted).unwrap();
        unsafe {
            assert_eq!(
                SetNamedSecurityInfoW(
                    name.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null()
                ),
                0
            );
        }
        assert!(check_private(&promoted).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
