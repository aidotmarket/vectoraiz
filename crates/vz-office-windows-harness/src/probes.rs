//! Hostile operations executed inside the actual, feature-gated parser worker.
//! Results describe API outcomes; the parent evaluates independent controls.
use crate::launch::{wide, Handle, MEMORY};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    mem::{size_of, size_of_val, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::*,
        Networking::WinSock::*,
        Storage::FileSystem::*,
        System::{
            Com::*, JobObjects::*, Memory::*, ProcessStatus::*, SystemInformation::*, Threading::*,
        },
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};

fn row(api: &str, rc: i64, success: bool, error: u32, namespace: &str) -> Value {
    let explicit = match namespace {
        "winsock" => error == WSAEACCES as u32,
        "hresult" => error == 0x80070005,
        "errno" => error == 12, // ENOMEM for allocation probes only
        _ => matches!(error, ERROR_ACCESS_DENIED | ERROR_PRIVILEGE_NOT_HELD),
    };
    json!({"api":api,"return_code":rc,"success":success,"error_code":error,
        "error_namespace":namespace,"explicit_denial":!success && explicit})
}
fn win(api: &str, rc: i64, success: bool) -> Value {
    let error = if success {
        0
    } else {
        unsafe { GetLastError() }
    };
    row(api, rc, success, error, "win32")
}
fn wsa(api: &str, rc: i64, success: bool) -> Value {
    let error = unsafe { WSAGetLastError() as u32 };
    let mut result = row(api, rc, success, if success { 0 } else { error }, "winsock");
    result["wsa_get_last_error"] = json!(error);
    result
}

fn arg<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .with_context(|| format!("probe argument {key}"))
}

pub fn run(v: &Value) -> Result<Value> {
    let op = arg(v, "op")?;
    let outcome = match op {
        "read" | "metadata" | "write" | "create" | "rename" | "delete" | "list" => files(v, op)?,
        "network" => network(v)?,
        "dns" => dns(),
        "shell" | "com" | "pipe" => broker(op),
        "child" | "escape" => child(op == "escape")?,
        "inherited" => inherited(v)?,
        "memory" => memory(arg(v, "kind")?, v)?,
        "alter-job" => alter_job(v)?,
        "scratch" => scratch(v)?,
        _ => bail!("unknown probe"),
    };
    Ok(json!({"probe":outcome,"self_memory":self_memory()}))
}

fn files(v: &Value, op: &str) -> Result<Value> {
    let path = wide(arg(v, "sentinel")?);
    unsafe { SetLastError(0) };
    let result = unsafe {
        match op {
            "metadata" => {
                let mut info: WIN32_FILE_ATTRIBUTE_DATA = zeroed();
                let rc = GetFileAttributesExW(
                    path.as_ptr(),
                    GetFileExInfoStandard,
                    &mut info as *mut _ as _,
                );
                win("GetFileAttributesExW", rc as i64, rc != 0)
            }
            "rename" => {
                let dest = wide(format!("{}.renamed", arg(v, "sentinel")?));
                let rc = MoveFileExW(path.as_ptr(), dest.as_ptr(), 0);
                win("MoveFileExW", rc as i64, rc != 0)
            }
            "delete" => {
                let rc = DeleteFileW(path.as_ptr());
                win("DeleteFileW", rc as i64, rc != 0)
            }
            "list" => {
                let glob = wide(format!("{}\\*", arg(v, "directory")?));
                let mut data: WIN32_FIND_DATAW = zeroed();
                let handle = FindFirstFileW(glob.as_ptr(), &mut data);
                let result = win(
                    "FindFirstFileW",
                    handle as i64,
                    handle != INVALID_HANDLE_VALUE,
                );
                if handle != INVALID_HANDLE_VALUE {
                    FindClose(handle);
                }
                result
            }
            _ => {
                let target = if op == "create" {
                    wide(arg(v, "created")?)
                } else {
                    path
                };
                let handle = CreateFileW(
                    target.as_ptr(),
                    if op == "read" {
                        GENERIC_READ
                    } else {
                        GENERIC_WRITE
                    },
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    null(),
                    if op == "create" {
                        CREATE_NEW
                    } else {
                        OPEN_EXISTING
                    },
                    FILE_ATTRIBUTE_NORMAL,
                    null_mut(),
                );
                let opened = win("CreateFileW", handle as i64, handle != INVALID_HANDLE_VALUE);
                if handle == INVALID_HANDLE_VALUE {
                    return Ok(opened);
                }
                let handle = Handle(handle);
                let mut count = 0;
                let mut bytes = [0u8; 64];
                let (api, rc) = if op == "read" {
                    (
                        "ReadFile",
                        ReadFile(
                            handle.0,
                            bytes.as_mut_ptr(),
                            bytes.len() as u32,
                            &mut count,
                            null_mut(),
                        ),
                    )
                } else {
                    (
                        "WriteFile",
                        WriteFile(handle.0, b"C1".as_ptr(), 2, &mut count, null_mut()),
                    )
                };
                let mut result = win(api, rc as i64, rc != 0 && count > 0);
                result["open"] = opened;
                result
            }
        }
    };
    Ok(result)
}

fn scratch(v: &Value) -> Result<Value> {
    let root = std::path::Path::new(arg(v, "scratch")?);
    let path = root.join("allowed.txt");
    let next = root.join("renamed.txt");
    let operation = (|| -> std::io::Result<()> {
        std::fs::write(&path, b"scratch positive control")?;
        let bytes = std::fs::read(&path)?;
        if bytes != b"scratch positive control" {
            return Err(std::io::Error::other("scratch bytes"));
        }
        std::fs::rename(path, &next)?;
        std::fs::remove_file(next)
    })();
    Ok(row(
        "scratch create/read/rename/delete",
        if operation.is_ok() { 1 } else { 0 },
        operation.is_ok(),
        operation.err().and_then(|e| e.raw_os_error()).unwrap_or(0) as u32,
        "win32",
    ))
}

struct Socket(SOCKET);
impl Drop for Socket {
    fn drop(&mut self) {
        if self.0 != INVALID_SOCKET {
            unsafe { closesocket(self.0) };
        }
    }
}
struct Winsock(i32, i32);
impl Winsock {
    fn new() -> Self {
        let mut data = unsafe { zeroed() };
        let rc = unsafe { WSAStartup(0x0202, &mut data) };
        Self(rc, unsafe { WSAGetLastError() })
    }
    fn evidence(&self) -> Value {
        json!({"api":"WSAStartup", "return_code":self.0, "success":self.0 == 0,"wsa_get_last_error":self.1})
    }
}
impl Drop for Winsock {
    fn drop(&mut self) {
        if self.0 == 0 {
            unsafe { WSACleanup() };
        }
    }
}

fn network(v: &Value) -> Result<Value> {
    let winsock = Winsock::new();
    let family = v["family"].as_u64().context("family")? as i32;
    let udp = v["transport"] == "udp";
    let operation = arg(v, "operation")?;
    let protocol = if family == AF_UNIX as i32 {
        0
    } else if family == 34 {
        1
    } else if udp {
        IPPROTO_UDP
    } else {
        IPPROTO_TCP
    };
    let kind = if udp { SOCK_DGRAM } else { SOCK_STREAM };
    unsafe { WSASetLastError(0) };
    let socket = unsafe {
        if v["constructor"] == "WSASocketW" {
            WSASocketW(family, kind, protocol, null(), 0, WSA_FLAG_OVERLAPPED)
        } else {
            socket(family, kind, protocol)
        }
    };
    let api = if v["constructor"] == "WSASocketW" {
        "WSASocketW"
    } else {
        "socket"
    };
    let mut creation = wsa(api, socket as i64, socket != INVALID_SOCKET);
    creation["startup"] = winsock.evidence();
    creation["family"] = json!(family);
    creation["descriptor_returned"] = json!(socket != INVALID_SOCKET);
    if operation == "create" {
        if socket != INVALID_SOCKET {
            unsafe { closesocket(socket) };
        }
        return Ok(creation);
    }
    let socket = Socket(socket);
    let mut nonblocking = 1;
    let nonblocking_rc = unsafe { ioctlsocket(socket.0, FIONBIO, &mut nonblocking) };
    let nonblocking_result = wsa("ioctlsocket", nonblocking_rc as i64, nonblocking_rc == 0);
    let address: std::net::IpAddr = arg(v, "address")?.parse()?;
    let port = v["port"].as_u64().context("port")? as u16;
    let mut storage: SOCKADDR_STORAGE = unsafe { zeroed() };
    let length = match address {
        std::net::IpAddr::V4(ip) => {
            let value = SOCKADDR_IN {
                sin_family: AF_INET,
                sin_port: port.to_be(),
                sin_addr: IN_ADDR {
                    S_un: IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes(ip.octets()),
                    },
                },
                sin_zero: [0; 8],
            };
            unsafe { std::ptr::write(&mut storage as *mut _ as *mut SOCKADDR_IN, value) };
            size_of::<SOCKADDR_IN>() as i32
        }
        std::net::IpAddr::V6(ip) => {
            let value = SOCKADDR_IN6 {
                sin6_family: AF_INET6,
                sin6_port: port.to_be(),
                sin6_flowinfo: 0,
                sin6_addr: IN6_ADDR {
                    u: IN6_ADDR_0 { Byte: ip.octets() },
                },
                Anonymous: SOCKADDR_IN6_0 { sin6_scope_id: 0 },
            };
            unsafe { std::ptr::write(&mut storage as *mut _ as *mut SOCKADDR_IN6, value) };
            size_of::<SOCKADDR_IN6>() as i32
        }
    };
    let addr = &storage as *const _ as *const SOCKADDR;
    // A stream socket must connect before sendto. Poll bounds the local control.
    // Still execute sendto if connect fails, preserving the LPAC literal API result.
    let connect_before_sendto = if operation == "sendto" && !udp {
        let rc = unsafe { connect(socket.0, addr, length) };
        let mut result = wsa("connect before TCP sendto", rc as i64, rc == 0);
        if rc == SOCKET_ERROR && result["error_code"] == WSAEWOULDBLOCK as u32 {
            let mut poll = WSAPOLLFD {
                fd: socket.0,
                events: POLLOUT,
                revents: 0,
            };
            let polled = unsafe { WSAPoll(&mut poll, 1, 3000) };
            let mut error = 0i32;
            let mut len = size_of_val(&error) as i32;
            let queried = unsafe {
                getsockopt(
                    socket.0,
                    SOL_SOCKET,
                    SO_ERROR,
                    &mut error as *mut _ as _,
                    &mut len,
                )
            };
            result["poll_return"] = json!(polled);
            result["socket_error"] = json!(error);
            result["success"] =
                json!(polled > 0 && queried == 0 && error == 0 && poll.revents & POLLWRNORM != 0);
        }
        Some(result)
    } else {
        None
    };
    let bind_before_listen = if operation == "listen" {
        let rc = unsafe { bind(socket.0, addr, length) };
        Some(wsa("bind before listen", rc as i64, rc == 0))
    } else {
        None
    };
    unsafe { WSASetLastError(0) };
    let (api, rc) = unsafe {
        match operation {
            "connect" => ("connect", connect(socket.0, addr, length)),
            "bind" => ("bind", bind(socket.0, addr, length)),
            "listen" => ("listen", listen(socket.0, 1)),
            "sendto" => (
                "sendto",
                sendto(socket.0, b"C1-feasibility".as_ptr(), 14, 0, addr, length),
            ),
            "multicast" => {
                if family == AF_INET as i32 {
                    let member = IP_MREQ {
                        imr_multiaddr: IN_ADDR {
                            S_un: IN_ADDR_0 {
                                S_addr: u32::from_ne_bytes([239, 255, 42, 42]),
                            },
                        },
                        imr_interface: IN_ADDR {
                            S_un: IN_ADDR_0 { S_addr: 0 },
                        },
                    };
                    (
                        "setsockopt(IP_ADD_MEMBERSHIP)",
                        setsockopt(
                            socket.0,
                            IPPROTO_IP,
                            IP_ADD_MEMBERSHIP,
                            &member as *const _ as _,
                            size_of_val(&member) as i32,
                        ),
                    )
                } else {
                    let mut member: IPV6_MREQ = zeroed();
                    member.ipv6mr_multiaddr.u.Byte =
                        [0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
                    member.ipv6mr_interface = v["interface_index"].as_u64().unwrap_or(0) as u32;
                    (
                        "setsockopt(IPV6_JOIN_GROUP)",
                        setsockopt(
                            socket.0,
                            IPPROTO_IPV6,
                            IPV6_JOIN_GROUP,
                            &member as *const _ as _,
                            size_of_val(&member) as i32,
                        ),
                    )
                }
            }
            _ => bail!("network operation"),
        }
    };
    let mut result = wsa(api, rc as i64, rc >= 0);
    // Nonblocking connect success is proved by SO_ERROR, not WSAEWOULDBLOCK.
    if operation == "connect" && rc == SOCKET_ERROR && result["error_code"] == WSAEWOULDBLOCK as u32
    {
        let mut poll = WSAPOLLFD {
            fd: socket.0,
            events: POLLOUT,
            revents: 0,
        };
        let polled = unsafe { WSAPoll(&mut poll, 1, 3000) };
        let mut error = 0i32;
        let mut len = size_of_val(&error) as i32;
        let queried = unsafe {
            getsockopt(
                socket.0,
                SOL_SOCKET,
                SO_ERROR,
                &mut error as *mut _ as _,
                &mut len,
            )
        };
        let ok = polled > 0 && queried == 0 && error == 0 && poll.revents & POLLWRNORM != 0;
        result = row(
            "connect+WSAPoll+SO_ERROR",
            rc as i64,
            ok,
            if ok {
                0
            } else if polled == 0 {
                WSAETIMEDOUT as u32
            } else {
                error as u32
            },
            "winsock",
        );
        result["poll_return"] = json!(polled);
    }
    result["api_executed"] = json!(true);
    result["socket_creation"] = creation;
    result["nonblocking"] = nonblocking_result;
    result["bind_before_listen"] = json!(bind_before_listen);
    result["connect_before_sendto"] = json!(connect_before_sendto);
    Ok(result)
}

fn dns() -> Value {
    let winsock = Winsock::new();
    let mut address = null_mut();
    let rc = unsafe { GetAddrInfoW(wide("example.com").as_ptr(), null(), null(), &mut address) };
    let error = unsafe { WSAGetLastError() };
    if !address.is_null() {
        unsafe { FreeAddrInfoW(address) };
    }
    let mut result = row("GetAddrInfoW", rc as i64, rc == 0, rc as u32, "winsock");
    result["wsa_get_last_error"] = json!(error);
    result["startup"] = winsock.evidence();
    result
}

fn broker(op: &str) -> Value {
    unsafe {
        SetLastError(0);
        match op {
            "shell" => {
                let rc = ShellExecuteW(
                    null_mut(),
                    wide("open").as_ptr(),
                    wide("https://example.com").as_ptr(),
                    null(),
                    null(),
                    SW_HIDE,
                ) as isize;
                let mut result = win("ShellExecuteW(open URL)", rc as i64, rc > 32);
                result["explicit_denial"] = json!(rc == SE_ERR_ACCESSDENIED as isize);
                let verb = wide("open");
                let url = wide("https://example.com");
                let mut ex: SHELLEXECUTEINFOW = zeroed();
                ex.cbSize = size_of_val(&ex) as u32;
                ex.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI;
                ex.lpVerb = verb.as_ptr();
                ex.lpFile = url.as_ptr();
                ex.nShow = SW_HIDE;
                SetLastError(0);
                let called = ShellExecuteExW(&mut ex);
                let error = if called != 0 { 0 } else { GetLastError() };
                result["shell_execute_ex"] = json!({"return_code":called,
                    "error_code":error,"hprocess":ex.hProcess as usize,
                    "process_id":if ex.hProcess.is_null() {0} else {GetProcessId(ex.hProcess)}});
                if !ex.hProcess.is_null() {
                    CloseHandle(ex.hProcess);
                }
                result
            }
            "pipe" => {
                let handle = CreateFileW(
                    wide(r"\\.\pipe\lsarpc").as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    null(),
                    OPEN_EXISTING,
                    0,
                    null_mut(),
                );
                let result = win(
                    "CreateFileW(system lsarpc pipe)",
                    handle as i64,
                    handle != INVALID_HANDLE_VALUE,
                );
                if handle != INVALID_HANDLE_VALUE {
                    CloseHandle(handle);
                }
                result
            }
            _ => {
                let init = CoInitializeEx(null(), COINIT_MULTITHREADED as u32);
                if init < 0 {
                    return row("CoInitializeEx", init as i64, false, init as u32, "hresult");
                }
                let class = GUID::from_u128(0x9ba05972_f6a8_11cf_a442_00a0c90a8f39); // ShellWindows
                let iid = GUID::from_u128(0x00000000_0000_0000_c000_000000000046); // IUnknown
                let mut object = null_mut();
                let hr =
                    CoCreateInstance(&class, null_mut(), CLSCTX_LOCAL_SERVER, &iid, &mut object);
                if hr >= 0 && !object.is_null() {
                    let table = *(object as *const *const *const std::ffi::c_void);
                    let release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                        std::mem::transmute(*table.add(2));
                    release(object);
                }
                CoUninitialize();
                row(
                    "CoCreateInstance(ShellWindows, LOCAL_SERVER)",
                    hr as i64,
                    hr >= 0,
                    hr as u32,
                    "hresult",
                )
            }
        }
    }
}

fn child(escape: bool) -> Result<Value> {
    let exe = std::env::current_exe()?;
    let mut command = wide(format!("\"{}\" --office-windows-child", exe.display()));
    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = size_of_val(&startup) as u32;
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    unsafe { SetLastError(0) };
    let rc = unsafe {
        CreateProcessW(
            wide(&exe).as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | if escape { CREATE_BREAKAWAY_FROM_JOB } else { 0 },
            null(),
            null(),
            &startup,
            &mut info,
        )
    };
    let mut result = win(
        if escape {
            "CreateProcessW(BREAKAWAY_FROM_JOB)"
        } else {
            "CreateProcessW"
        },
        rc as i64,
        rc != 0,
    );
    if rc != 0 {
        unsafe {
            TerminateProcess(info.hProcess, 0);
            WaitForSingleObject(info.hProcess, 5000);
            CloseHandle(info.hThread);
            CloseHandle(info.hProcess);
        }
    }
    if rc == 0 && result["error_code"] == ERROR_NOT_ENOUGH_QUOTA && !escape {
        let state = memory_snapshot();
        let job = &state["job"];
        let accounting = &state["accounting"];
        if job["available"] == true
            && accounting["available"] == true
            && job["flags"].as_u64().unwrap_or(0) & JOB_OBJECT_LIMIT_ACTIVE_PROCESS as u64 != 0
            && job["active_process_limit"] == 1
            && accounting["active_processes"] == 1
        {
            result["denial_category"] = json!("job_active_process_limit");
            result["explicit_denial"] = json!(true);
        }
        result["quota_cause_evidence"] = state;
    }
    result["child_was_suspended"] = json!(true);
    Ok(result)
}

// ProcessHandleInformation uses pointer-sized fields, including both header counts.
#[repr(C)]
#[derive(Clone, Copy)]
struct HandleEntry {
    handle: HANDLE,
    handle_count: usize,
    pointer_count: usize,
    granted_access: u32,
    object_type_index: u32,
    attributes: u32,
    reserved: u32,
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process: HANDLE,
        class: u32,
        info: *mut std::ffi::c_void,
        len: u32,
        returned: *mut u32,
    ) -> i32;
}
pub(crate) fn file_identity(handle: HANDLE) -> Result<Value> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    anyhow::ensure!(
        unsafe { GetFileInformationByHandle(handle, &mut info) } != 0,
        "file identity query"
    );
    Ok(
        json!({"volume":info.dwVolumeSerialNumber,"index_high":info.nFileIndexHigh,"index_low":info.nFileIndexLow}),
    )
}
fn inherited(v: &Value) -> Result<Value> {
    let handle = v["attack_handle"].as_u64().context("attack handle")? as usize as HANDLE;
    let mut storage = vec![0usize; 131072];
    let mut returned = 0;
    let status = unsafe {
        NtQueryInformationProcess(
            GetCurrentProcess(),
            51,
            storage.as_mut_ptr().cast(),
            (storage.len() * size_of::<usize>()) as u32,
            &mut returned,
        )
    };
    if status < 0 {
        return Ok(
            json!({"api":"NtQueryInformationProcess(ProcessHandleInformation)","success":false,"enumeration_succeeded":false,"ntstatus":status as u32}),
        );
    }
    let count = storage[0];
    anyhow::ensure!(
        count
            <= (storage.len() * size_of::<usize>() - 2 * size_of::<usize>())
                / size_of::<HandleEntry>(),
        "handle table count exceeds buffer"
    );
    let entries =
        unsafe { std::slice::from_raw_parts(storage.as_ptr().add(2).cast::<HandleEntry>(), count) };
    let present = entries.iter().any(|entry| entry.handle == handle);
    // Only query a handle confirmed present in our own table. Never ReadFile on a raw value.
    let identity = if present {
        file_identity(handle).ok()
    } else {
        None
    };
    let matches = identity
        .as_ref()
        .is_some_and(|id| *id == v["attack_identity"]);
    Ok(
        json!({"api":"NtQueryInformationProcess(ProcessHandleInformation)","success":present && matches,
        "enumeration_succeeded":true,"ntstatus":status as u32,"present":present,"identity":identity,"identity_match":matches}),
    )
}

fn alter_job(v: &Value) -> Result<Value> {
    let name = wide(arg(v, "job_name")?);
    unsafe { SetLastError(0) };
    // JOB_OBJECT_SET_ATTRIBUTES=0x2, JOB_OBJECT_QUERY=0x4 (winnt.h).
    let handle = unsafe { OpenJobObjectW(0x0002 | 0x0004, 0, name.as_ptr()) };
    let opened = win(
        "OpenJobObjectW(SET_ATTRIBUTES)",
        handle as i64,
        !handle.is_null(),
    );
    if handle.is_null() {
        return Ok(opened);
    }
    let handle = Handle(handle);
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    let got = unsafe {
        QueryInformationJobObject(
            handle.0,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as _,
            size_of_val(&info) as u32,
            null_mut(),
        )
    };
    if got == 0 {
        return Ok(win("QueryInformationJobObject(worker)", 0, false));
    }
    info.BasicLimitInformation.LimitFlags &= !JOB_OBJECT_LIMIT_WORKINGSET;
    info.ProcessMemoryLimit = MEMORY * 4;
    info.JobMemoryLimit = MEMORY * 4;
    info.BasicLimitInformation.LimitFlags |=
        JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_JOB_MEMORY;
    let rc = unsafe {
        SetInformationJobObject(
            handle.0,
            JobObjectExtendedLimitInformation,
            &info as *const _ as _,
            size_of_val(&info) as u32,
        )
    };
    let mut result = win(
        "SetInformationJobObject(raise process and job limits)",
        rc as i64,
        rc != 0,
    );
    result["open"] = opened;
    Ok(result)
}

fn self_memory() -> Value {
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { zeroed() };
    let rc = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut _ as _,
            size_of_val(&counters) as u32,
        )
    };
    json!({"available":rc != 0,"working_set_bytes":counters.WorkingSetSize,
        "peak_working_set_bytes":counters.PeakWorkingSetSize,"private_commit_bytes":counters.PrivateUsage,
        "peak_process_commit_bytes":counters.PeakPagefileUsage})
}

// NULL queries the caller's assigned job; query failures remain visible.
fn memory_snapshot() -> Value {
    unsafe {
        let mut job: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        let j = QueryInformationJobObject(
            null_mut(),
            JobObjectExtendedLimitInformation,
            &mut job as *mut _ as _,
            size_of_val(&job) as u32,
            null_mut(),
        );
        let je = if j == 0 { GetLastError() } else { 0 };
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
        let a = QueryInformationJobObject(
            null_mut(),
            JobObjectBasicAccountingInformation,
            &mut accounting as *mut _ as _,
            size_of_val(&accounting) as u32,
            null_mut(),
        );
        let ae = if a == 0 { GetLastError() } else { 0 };
        let mut violation: JOBOBJECT_LIMIT_VIOLATION_INFORMATION = zeroed();
        let v = QueryInformationJobObject(
            null_mut(),
            JobObjectLimitViolationInformation,
            &mut violation as *mut _ as _,
            size_of_val(&violation) as u32,
            null_mut(),
        );
        let ve = if v == 0 { GetLastError() } else { 0 };
        let mut system: PERFORMANCE_INFORMATION = zeroed();
        system.cb = size_of_val(&system) as u32;
        let p = GetPerformanceInfo(&mut system, size_of_val(&system) as u32);
        let process = self_memory();
        json!({"process":process,
            "job":{"available":j != 0,"error_code":je,
                "flags":job.BasicLimitInformation.LimitFlags,"active_process_limit":job.BasicLimitInformation.ActiveProcessLimit,
                "peak_process_commit_bytes":job.PeakProcessMemoryUsed,"peak_job_commit_bytes":job.PeakJobMemoryUsed},
            "accounting":{"available":a != 0,"error_code":ae,"active_processes":accounting.ActiveProcesses,
                "total_processes":accounting.TotalProcesses,"terminated_processes":accounting.TotalTerminatedProcesses,
                "page_faults":accounting.TotalPageFaultCount,"user_time_100ns":accounting.TotalUserTime,
                "kernel_time_100ns":accounting.TotalKernelTime},
            // Notification-time data is not a measurement of currently held job commit.
            // https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-jobobject_limit_violation_information
            "job_notification_commit":{"query_succeeded":v != 0,"error_code":ve,
                "limit_flags":violation.LimitFlags,"violation_flags":violation.ViolationLimitFlags,
                "bytes_at_notification":violation.JobMemory},
            "single_member_private_commit":{"available":a != 0 && accounting.ActiveProcesses == 1 && process["available"] == true,
                "bytes":process["private_commit_bytes"],
                "scope":"single member's measured private commit; excludes shared sections and job overhead"},
            "system":{"available":p != 0,"commit_total_bytes":system.CommitTotal * system.PageSize,
                "commit_limit_bytes":system.CommitLimit * system.PageSize,
                "physical_available_bytes":system.PhysicalAvailable * system.PageSize,
                "physical_total_bytes":system.PhysicalTotal * system.PageSize}})
    }
}

pub(crate) fn system_memory() -> Value {
    let mut info: PERFORMANCE_INFORMATION = unsafe { zeroed() };
    let ok = unsafe { GetPerformanceInfo(&mut info, size_of::<PERFORMANCE_INFORMATION>() as u32) };
    json!({"available":ok != 0,"commit_total_bytes":info.CommitTotal * info.PageSize,
        "commit_limit_bytes":info.CommitLimit * info.PageSize,
        "physical_available_bytes":info.PhysicalAvailable * info.PageSize,
        "physical_total_bytes":info.PhysicalTotal * info.PageSize})
}
fn working_set_readback() -> Value {
    let (mut min, mut max, mut flags) = (0, 0, 0);
    let rc =
        unsafe { GetProcessWorkingSetSizeEx(GetCurrentProcess(), &mut min, &mut max, &mut flags) };
    json!({"available":rc != 0,"minimum_bytes":min,"maximum_bytes":max,"flags":flags})
}
fn working_set_mutation(v: &Value) -> Result<Value> {
    let mut attempts = Vec::new();
    for (name, max, flags) in [
        ("raise", MEMORY * 2, QUOTA_LIMITS_HARDWS_MAX_ENABLE),
        ("disable", MEMORY * 2, QUOTA_LIMITS_HARDWS_MAX_DISABLE),
    ] {
        let before = working_set_readback();
        let rc = unsafe {
            SetProcessWorkingSetSizeEx(GetCurrentProcess(), 64 * 1024 * 1024, max, flags)
        };
        let call = win("SetProcessWorkingSetSizeEx(worker)", rc as i64, rc != 0);
        let after = working_set_readback();
        // Rerun in this same process after EACH mutation, retaining the kernel peak.
        let section = mapping(false, v, false)?;
        attempts.push(
            json!({"attempt":name,"call":call,"before":before,"after":after,"section":section}),
        );
    }
    Ok(json!({"api":"SetProcessWorkingSetSizeEx(worker)","success":true,"attempts":attempts}))
}

fn mapping(reserve: bool, v: &Value, file_backed: bool) -> Result<Value> {
    let target = 2 * 1024 * 1024 * 1024usize;
    let step = 64 * 1024 * 1024usize;
    let mut system: SYSTEM_INFO = unsafe { zeroed() };
    unsafe { GetSystemInfo(&mut system) };
    let page = system.dwPageSize as usize;
    anyhow::ensure!(page > 0, "system page size");
    let before = memory_snapshot();
    let backing = if file_backed {
        let path = std::path::Path::new(arg(v, "scratch")?).join("mapped-scratch.bin");
        let handle = unsafe {
            CreateFileW(
                wide(path).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                CREATE_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        anyhow::ensure!(
            handle != INVALID_HANDLE_VALUE,
            "scratch backing file creation"
        );
        let handle = Handle(handle);
        crate::launch::check(
            unsafe { SetFilePointerEx(handle.0, target as i64, null_mut(), FILE_BEGIN) },
            "scratch size seek",
        )?;
        crate::launch::check(unsafe { SetEndOfFile(handle.0) }, "scratch file size")?;
        Some(handle)
    } else {
        None
    };
    let handle = unsafe {
        CreateFileMappingW(
            backing.as_ref().map_or(INVALID_HANDLE_VALUE, |h| h.0),
            null(),
            PAGE_READWRITE | if reserve { SEC_RESERVE } else { SEC_COMMIT },
            0,
            target as u32,
            null(),
        )
    };
    let creation = win("CreateFileMappingW", handle as i64, !handle.is_null());
    let after_creation = memory_snapshot();
    eprintln!(
        "MEMORY_PROGRESS {}",
        json!({"phase":"section_creation","result":creation,"before":before,"after":after_creation})
    );
    if handle.is_null() {
        return Ok(json!({"success":false,"explicit_denial":false,
        "creation":creation,"before":before,"after_creation":after_creation}));
    }
    let handle = Handle(handle);
    let view = unsafe { MapViewOfFile(handle.0, FILE_MAP_WRITE, 0, 0, target) };
    let mut result = win("MapViewOfFile", view.Value as i64, !view.Value.is_null());
    let after_mapping = memory_snapshot();
    eprintln!(
        "MEMORY_PROGRESS {}",
        json!({"phase":"map_view","result":result,"after":after_mapping})
    );
    let mut held = 0;
    let mut steps = Vec::new();
    while !view.Value.is_null() && held < target {
        let before_step = memory_snapshot();
        let ptr = unsafe { view.Value.cast::<u8>().add(held) };
        if reserve {
            let committed = unsafe { VirtualAlloc(ptr.cast(), step, MEM_COMMIT, PAGE_READWRITE) };
            result = win(
                "VirtualAlloc(MEM_COMMIT on SEC_RESERVE view)",
                committed as i64,
                !committed.is_null(),
            );
            if committed.is_null() {
                steps.push(json!({"held_bytes":held,"before":before_step,"after":memory_snapshot(),"result":result}));
                break;
            }
        }
        eprintln!(
            "MEMORY_PROGRESS {}",
            json!({"phase":"before_touch","held_bytes":held,"before":before_step})
        );
        for offset in (0..step).step_by(page) {
            unsafe { std::ptr::write_volatile(ptr.add(offset), 42) };
        }
        held += step;
        eprintln!(
            "MEMORY_PROGRESS {}",
            json!({"phase":"after_touch","held_bytes":held,"after":memory_snapshot()})
        );
        steps.push(json!({"held_bytes":held,"before":before_step,"after":memory_snapshot(),"result":result}));
    }
    let mut at_peak = memory_snapshot();
    at_peak["job_members"] = crate::launch::job_members(null_mut());
    if !view.Value.is_null() {
        unsafe { UnmapViewOfFile(view) };
    }
    result["creation"] = creation;
    let commit_growth = match (
        before["system"]["commit_total_bytes"].as_i64(),
        at_peak["system"]["commit_total_bytes"].as_i64(),
    ) {
        (Some(a), Some(b))
            if before["system"]["available"] == true && at_peak["system"]["available"] == true =>
        {
            Some(b - a)
        }
        _ => None,
    };
    let max_ws = steps
        .iter()
        .filter_map(|s| s["after"]["process"]["peak_working_set_bytes"].as_u64())
        .max();
    result["system_commit_growth_bytes"] = json!(commit_growth);
    result["system_commit_growth_scope"] =
        json!("system-wide before creation to held section; concurrent activity included");
    result["touch_completed"] = json!(held == target);
    result["max_observed_working_set_bytes"] = json!(max_ws);
    result["working_set_within_1_5_gib"] = json!(max_ws.map(|ws| ws <= MEMORY as u64 + 4096));
    result["paging_note"] = json!("Completed touch under resident cap is consistent with eviction; pagefile writes are not directly measured");
    result["before"] = before;
    result["after_creation"] = after_creation;
    result["after_mapping"] = after_mapping;
    result["steps"] = json!(steps);
    result["at_peak"] = at_peak;
    result["successfully_held_bytes"] = json!(held);
    result["target_bytes"] = json!(target);
    result["scratch_file_size_bytes"] = json!(if file_backed { target } else { 0 });
    result["section_mode"] = json!(if reserve { "SEC_RESERVE" } else { "SEC_COMMIT" });
    result["allocation_refused"] = json!(held < target);
    let code = result["error_code"].as_u64().unwrap_or(0) as u32;
    let refused = held < target
        && matches!(
            code,
            ERROR_COMMITMENT_LIMIT | ERROR_NOT_ENOUGH_MEMORY | ERROR_OUTOFMEMORY
        );
    result["allocation_limit_error"] = json!(refused);
    result["explicit_denial"] = json!(refused);
    Ok(result)
}

#[link(name = "ucrt")]
unsafe extern "C" {
    fn malloc(size: usize) -> *mut std::ffi::c_void;
    fn free(ptr: *mut std::ffi::c_void);
    fn _errno() -> *mut i32;
}

fn memory(kind: &str, v: &Value) -> Result<Value> {
    if kind == "self-ws" {
        return working_set_mutation(v);
    }
    if matches!(kind, "mapping" | "mapping-reserve" | "mapping-file") {
        return mapping(kind == "mapping-reserve", v, kind == "mapping-file");
    }
    let step = if kind == "small" {
        64 * 1024
    } else if kind == "burst" {
        2 * 1024 * 1024 * 1024
    } else {
        64 * 1024 * 1024
    };
    let target = 2 * 1024 * 1024 * 1024usize;
    let mut pointers = Vec::new();
    let mut committed = 0usize;
    let before = memory_snapshot();
    let mut samples = Vec::new();
    let mut max_held_private_commit = before["process"]["private_commit_bytes"]
        .as_u64()
        .unwrap_or(0);
    let mut max_held_job_commit = if before["single_member_private_commit"]["available"] == true {
        before["single_member_private_commit"]["bytes"].as_u64()
    } else {
        None
    };
    let mut result = json!(null);
    let api = match kind {
        "malloc" => "malloc",
        "virtual" | "small" | "burst" => "VirtualAlloc(MEM_RESERVE|MEM_COMMIT)",
        _ => bail!("memory kind"),
    };
    while committed < target {
        let before_call = memory_snapshot();
        unsafe { SetLastError(0) };
        let ok;
        {
            let ptr = if kind == "malloc" {
                unsafe {
                    *_errno() = 0;
                    malloc(step)
                }
            } else {
                unsafe { VirtualAlloc(null(), step, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) }
            };
            ok = !ptr.is_null();
            result = if kind == "malloc" {
                row(api, ptr as i64, ok, unsafe { *_errno() } as u32, "errno")
            } else {
                win(api, ptr as i64, ok)
            };
            if ok {
                // Commit accounting is the bound. Touch one page per block only;
                // do not conflate reserve/commit with resident memory.
                unsafe { std::ptr::write_volatile(ptr.cast::<u8>(), 42) };
                pointers.push(ptr);
            }
        }
        let after_call = memory_snapshot();
        if ok {
            committed += step;
            max_held_private_commit = max_held_private_commit.max(
                after_call["process"]["private_commit_bytes"]
                    .as_u64()
                    .unwrap_or(0),
            );
            if after_call["single_member_private_commit"]["available"] == true {
                let current = after_call["single_member_private_commit"]["bytes"]
                    .as_u64()
                    .unwrap_or(0);
                max_held_job_commit = Some(max_held_job_commit.unwrap_or(0).max(current));
            }
        }
        // Small allocations retain only the refusal boundary and final sample.
        if kind != "small" || !ok || committed == target {
            samples.push(json!({"held_bytes":committed,"succeeded":ok,"before_call":before_call,"after_call":after_call}));
        }
        if !ok {
            break;
        }
    }
    let mut at_peak = memory_snapshot();
    at_peak["job_members"] = crate::launch::job_members(null_mut());
    for ptr in pointers {
        if kind == "malloc" {
            unsafe { free(ptr) };
        } else {
            unsafe { VirtualFree(ptr, 0, MEM_RELEASE) };
        }
    }
    result["call_samples"] = json!(samples);
    result["max_observed_held_private_commit_bytes"] = json!(max_held_private_commit);
    result["max_observed_held_member_private_commit_bytes"] = json!(max_held_job_commit);
    if let Some(failure) = samples.iter().find(|sample| sample["succeeded"] == false) {
        let prior = &failure["before_call"];
        let after = &failure["after_call"];
        let peak_grew = match (
            prior["job"]["peak_job_commit_bytes"].as_u64(),
            after["job"]["peak_job_commit_bytes"].as_u64(),
        ) {
            (Some(a), Some(b))
                if prior["job"]["available"] == true && after["job"]["available"] == true =>
            {
                Some(b > a)
            }
            _ => None,
        };
        let held_grew = match (
            prior["single_member_private_commit"]["bytes"].as_u64(),
            after["single_member_private_commit"]["bytes"].as_u64(),
        ) {
            (Some(a), Some(b))
                if prior["single_member_private_commit"]["available"] == true
                    && after["single_member_private_commit"]["available"] == true =>
            {
                Some(b > a)
            }
            _ => None,
        };
        result["refusal_analysis"] = json!({"reported_job_peak_increased":peak_grew,
            "observed_held_member_private_commit_increased":held_grew,
            "held_allocation_bytes":committed,
            "transient_charge_inference":if peak_grew == Some(true) && held_grew == Some(false) {
                "consistent with a transient refused-request charge; private commit excludes shared sections and snapshots do not prove duration"
            } else { "not established; inspect before/after counters and query availability" }});
    }
    result["step_bytes"] = json!(step);
    result["successfully_held_bytes"] = json!(committed);
    result["target_bytes"] = json!(target);
    result["allocation_refused"] = json!(committed < target);
    result["before"] = before;
    result["at_peak"] = at_peak;
    result["sampler_enabled"] = json!(false);
    // Kernel commit refusals include ERROR_COMMITMENT_LIMIT and ERROR_NOT_ENOUGH_MEMORY.
    let code = result["error_code"].as_u64().unwrap_or(0) as u32;
    let limit_error = committed < target
        && (matches!(
            code,
            ERROR_COMMITMENT_LIMIT | ERROR_NOT_ENOUGH_MEMORY | ERROR_OUTOFMEMORY
        ) || kind == "malloc" && code == 12);
    result["allocation_limit_error"] = json!(limit_error);
    if limit_error {
        result["explicit_denial"] = json!(true);
    }
    Ok(result)
}
