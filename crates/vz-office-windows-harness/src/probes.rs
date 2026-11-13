//! Hostile operations executed inside the actual, feature-gated parser worker.
//! Results describe API outcomes; the parent evaluates independent controls.
use crate::launch::{check, wide, Handle, MEMORY};
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
        System::{Com::*, JobObjects::*, Memory::*, ProcessStatus::*, Threading::*},
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
    let error = if success {
        0
    } else {
        unsafe { WSAGetLastError() as u32 }
    };
    row(api, rc, success, error, "winsock")
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
        "memory" => memory(arg(v, "kind")?)?,
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
        unsafe { closesocket(self.0) };
    }
}
struct Winsock;
impl Winsock {
    fn new() -> Result<Self> {
        let mut data = unsafe { zeroed() };
        let rc = unsafe { WSAStartup(0x0202, &mut data) };
        anyhow::ensure!(rc == 0, "WSAStartup={rc}");
        Ok(Self)
    }
}
impl Drop for Winsock {
    fn drop(&mut self) {
        unsafe { WSACleanup() };
    }
}

fn network(v: &Value) -> Result<Value> {
    let _winsock = Winsock::new()?;
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
    let creation = wsa(api, socket as i64, socket != INVALID_SOCKET);
    if operation == "create" {
        if socket != INVALID_SOCKET {
            unsafe { closesocket(socket) };
        }
        return Ok(creation);
    }
    if socket == INVALID_SOCKET {
        return Ok(json!({"api":operation,"return_code":null,"success":false,
            "error_code":creation["error_code"],"error_namespace":"winsock",
            "explicit_denial":creation["explicit_denial"],"api_executed":false,
            "reason":"socket creation refused; target API unrun", "socket_creation":creation}));
    }
    let socket = Socket(socket);
    let mut nonblocking = 1;
    check(
        unsafe { ioctlsocket(socket.0, FIONBIO, &mut nonblocking) } + 1,
        "ioctlsocket",
    )?;
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
    if operation == "listen" {
        let rc = unsafe { bind(socket.0, addr, length) };
        if rc != 0 {
            let mut result = wsa("bind before listen", rc as i64, false);
            result["api_executed"] = json!(false);
            result["socket_creation"] = creation;
            return Ok(result);
        }
    }
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
    Ok(result)
}

fn dns() -> Value {
    let _winsock = match Winsock::new() {
        Ok(w) => w,
        Err(e) => return json!({"api":"WSAStartup","success":false,"reason":e.to_string()}),
    };
    let mut address = null_mut();
    let rc = unsafe { GetAddrInfoW(wide("example.com").as_ptr(), null(), null(), &mut address) };
    let error = unsafe { WSAGetLastError() };
    if !address.is_null() {
        unsafe { FreeAddrInfoW(address) };
    }
    let mut result = row("GetAddrInfoW", rc as i64, rc == 0, rc as u32, "winsock");
    result["wsa_get_last_error"] = json!(error);
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
    result["child_was_suspended"] = json!(true);
    Ok(result)
}

fn inherited(v: &Value) -> Result<Value> {
    let handle = v["attack_handle"].as_u64().context("attack handle")? as usize as HANDLE;
    let mut bytes = [0u8; 64];
    let mut count = 0;
    unsafe { SetLastError(0) };
    let rc = unsafe {
        ReadFile(
            handle,
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            &mut count,
            null_mut(),
        )
    };
    let mut result = win(
        "ReadFile(omitted inheritable parent handle)",
        rc as i64,
        rc != 0,
    );
    // An omitted handle may have been reused; never call unrelated readable data proof.
    result["sentinel_match"] =
        json!(rc != 0 && bytes[..count as usize].starts_with(b"C1-private-sentinel"));
    if rc == 0 && result["error_code"] == ERROR_INVALID_HANDLE {
        result["explicit_denial"] = json!(true);
    }
    Ok(result)
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
        "peak_working_set_bytes":counters.PeakWorkingSetSize,"private_commit_bytes":counters.PrivateUsage})
}

#[link(name = "ucrt")]
unsafe extern "C" {
    fn malloc(size: usize) -> *mut std::ffi::c_void;
    fn free(ptr: *mut std::ffi::c_void);
    fn _errno() -> *mut i32;
}

fn memory(kind: &str) -> Result<Value> {
    let step = if kind == "small" {
        64 * 1024
    } else if kind == "burst" {
        2 * 1024 * 1024 * 1024
    } else {
        64 * 1024 * 1024
    };
    let target = 2 * 1024 * 1024 * 1024usize;
    let mut pointers = Vec::new();
    let mut mappings = Vec::new();
    let mut committed = 0usize;
    let before = self_memory();
    let mut result = json!(null);
    let api = match kind {
        "mapping" => "CreateFileMappingW(PAGE_READWRITE|SEC_COMMIT)",
        "malloc" => "malloc",
        "virtual" | "small" | "burst" => "VirtualAlloc(MEM_RESERVE|MEM_COMMIT)",
        _ => bail!("memory kind"),
    };
    while committed < target {
        unsafe { SetLastError(0) };
        let ok;
        if kind == "mapping" {
            let handle = unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    null(),
                    PAGE_READWRITE | SEC_COMMIT,
                    0,
                    step as u32,
                    null(),
                )
            };
            ok = !handle.is_null();
            result = win(api, handle as i64, ok);
            if ok {
                mappings.push(Handle(handle));
            }
        } else {
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
        if !ok {
            break;
        }
        committed += step;
    }
    let at_peak = self_memory();
    for ptr in pointers {
        if kind == "malloc" {
            unsafe { free(ptr) };
        } else {
            unsafe { VirtualFree(ptr, 0, MEM_RELEASE) };
        }
    }
    drop(mappings);
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
