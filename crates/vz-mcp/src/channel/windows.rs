use std::{io, os::windows::io::AsRawHandle, ptr};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, HANDLE},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::SECURITY_IDENTIFICATION,
    System::{
        Pipes::GetNamedPipeServerProcessId,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
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

fn process_sid(process: HANDLE) -> io::Result<String> {
    // Every handle and LocalAlloc allocation below is owned by an RAII guard.
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(token);
        let mut size = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        // usize storage supplies TOKEN_USER alignment; Windows returns the SID inside it.
        let mut storage = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = &*storage.as_ptr().cast::<TOKEN_USER>();
        let mut sid = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let allocated = Local(sid.cast());
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let result =
            String::from_utf16(std::slice::from_raw_parts(sid, len)).map_err(io::Error::other);
        drop(allocated);
        result
    }
}
pub fn current_sid() -> io::Result<String> {
    unsafe { process_sid(GetCurrentProcess()) }
}
pub fn path() -> io::Result<String> {
    Ok(format!(r"\\.\pipe\vectorAIz-mcp-{}", current_sid()?))
}
pub type Stream = NamedPipeClient;

pub fn create_server(first: bool) -> io::Result<NamedPipeServer> {
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", current_sid()?)
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
            return Err(io::Error::last_os_error());
        }
        let allocated = Local(descriptor);
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                path()?,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            );
        drop(allocated);
        result
    }
}
pub async fn connect() -> io::Result<Stream> {
    let pipe = ClientOptions::new()
        .security_qos_flags(SECURITY_IDENTIFICATION)
        .open(path()?)?;
    // A failed query is a mismatch. No bytes have been sent to the pipe.
    unsafe {
        let mut pid = 0;
        if GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let process = Handle(process);
        if !server_matches(&current_sid()?, process_sid(process.0)) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "server identity mismatch",
            ));
        }
    }
    Ok(pipe)
}

fn server_matches(own:&str, server:io::Result<String>) -> bool {
    server.is_ok_and(|sid|sid==own)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{Authorization::{GetSecurityInfo,ConvertSecurityDescriptorToStringSecurityDescriptorW,SE_KERNEL_OBJECT},DACL_SECURITY_INFORMATION};
    #[tokio::test]
    async fn dacl_first_instance_remote_rejection_and_fail_closed_identity() {
        let server=create_server(true).unwrap();
        assert!(create_server(true).is_err(),"a squatted first instance must never be retried");
        let sid=current_sid().unwrap();
        assert!(server_matches(&sid,Ok(sid.clone())));
        assert!(!server_matches(&sid,Ok("S-1-5-18".into())));
        assert!(!server_matches(&sid,Err(io::Error::from(io::ErrorKind::PermissionDenied))));
        unsafe {
            let mut descriptor=ptr::null_mut();
            assert_eq!(GetSecurityInfo(server.as_raw_handle(),SE_KERNEL_OBJECT,DACL_SECURITY_INFORMATION,ptr::null_mut(),ptr::null_mut(),ptr::null_mut(),ptr::null_mut(),&mut descriptor),0);
            let _descriptor=Local(descriptor);
            let mut text=ptr::null_mut();
            assert_ne!(ConvertSecurityDescriptorToStringSecurityDescriptorW(descriptor,1,DACL_SECURITY_INFORMATION,&mut text,ptr::null_mut()),0);
            let _text=Local(text.cast());let mut len=0;while *text.add(len)!=0{len+=1;}
            let sddl=String::from_utf16(std::slice::from_raw_parts(text,len)).unwrap();
            assert_eq!(sddl.matches("(A;").count(),2,"{sddl}");
            assert!(sddl.contains(&sid) && sddl.contains(";;;SY)"),"{sddl}");
        }
        let remote=path().unwrap().replacen(r"\\.\",r"\\localhost\",1);
        assert!(ClientOptions::new().open(remote).is_err(),"remote pipe clients must be rejected");
        let client=connect().await.unwrap();server.connect().await.unwrap();drop(client);
    }
}
