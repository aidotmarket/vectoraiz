//! One disposable LPAC/profile/job per invocation. No fallback launch path.
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    ffi::{c_void, OsStr},
    fs::File,
    io::{Read, Write},
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, Isolation::*, *},
    Storage::FileSystem::*,
    System::{
        Diagnostics::ToolHelp::*, JobObjects::*, Pipes::*, ProcessStatus::*,
        RemoteDesktop::ProcessIdToSessionId, Threading::*,
    },
};

pub const MEMORY: usize = 1_610_612_736;
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn check(ok: i32, api: &str) -> Result<()> {
    if ok == 0 {
        bail!("{api}: {}", std::io::Error::last_os_error());
    }
    Ok(())
}
pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
// Owns handles, not shared with any caller.
unsafe impl Send for Handle {}

struct Profile {
    name: Vec<u16>,
    sid: PSID,
}
impl Profile {
    fn new(trace: &mut Value) -> Result<Self> {
        trace["profile_api"] = json!("CreateAppContainerProfile");
        let name = wide(format!(
            "vectoraiz.C1.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut sid = null_mut();
        let hr = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        trace["profile_hresult"] = json!(hr);
        ensure!(hr >= 0, "CreateAppContainerProfile HRESULT={hr:#x}");
        ensure!(
            !sid.is_null(),
            "CreateAppContainerProfile returned NULL SID"
        );
        Ok(Self { name, sid })
    }
    fn remove(&mut self) -> Result<()> {
        if !self.sid.is_null() {
            let hr = unsafe { DeleteAppContainerProfile(self.name.as_ptr()) };
            ensure!(hr >= 0, "DeleteAppContainerProfile HRESULT={hr:#x}");
            unsafe { FreeSid(self.sid) };
            self.sid = null_mut();
        }
        Ok(())
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        if let Err(e) = self.remove() {
            eprintln!("CLEANUP FAILURE {e:#}");
        }
    }
}

/// Save the original DACL, add only this package SID, restore before profile deletion.
struct Grant {
    path: Vec<u16>,
    descriptor: PSECURITY_DESCRIPTOR,
    old: *mut ACL,
}
impl Grant {
    fn new(path: &Path, sid: PSID, mask: u32, inherit: u32) -> Result<Self> {
        let path = wide(path);
        let mut descriptor = null_mut();
        let mut old = null_mut();
        let status = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut old,
                null_mut(),
                &mut descriptor,
            )
        };
        ensure!(status == 0, "GetNamedSecurityInfoW={status}");
        let guard = Self {
            path,
            descriptor,
            old,
        };
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: mask,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: inherit,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: sid.cast(),
            },
        };
        let mut acl = null_mut();
        let status = unsafe { SetEntriesInAclW(1, &entry, old, &mut acl) };
        ensure!(status == 0, "SetEntriesInAclW={status}");
        let status = unsafe {
            SetNamedSecurityInfoW(
                guard.path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null(),
            )
        };
        unsafe { LocalFree(acl.cast()) };
        ensure!(status == 0, "SetNamedSecurityInfoW={status}");
        Ok(guard)
    }
    fn restore(&mut self) -> Result<()> {
        if !self.descriptor.is_null() {
            let status = unsafe {
                SetNamedSecurityInfoW(
                    self.path.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    self.old,
                    null(),
                )
            };
            ensure!(status == 0, "restore DACL={status}");
            unsafe { LocalFree(self.descriptor) };
            self.descriptor = null_mut();
        }
        Ok(())
    }
}
impl Drop for Grant {
    fn drop(&mut self) {
        if let Err(e) = self.restore() {
            eprintln!("CLEANUP FAILURE {e:#}");
        }
    }
}

struct Attributes {
    // Attribute lists require pointer alignment, not Vec<u8> alignment.
    _storage: Vec<usize>,
    ptr: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn new(count: u32) -> Result<Self> {
        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut bytes) };
        ensure!(bytes > 0, "attribute list size query");
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let ptr = storage.as_mut_ptr().cast();
        check(
            unsafe { InitializeProcThreadAttributeList(ptr, count, 0, &mut bytes) },
            "InitializeProcThreadAttributeList",
        )?;
        Ok(Self {
            _storage: storage,
            ptr,
        })
    }
    fn set<T>(&self, key: u32, value: &T) -> Result<()> {
        self.set_bytes(key, value as *const T as *const c_void, size_of::<T>())
    }
    fn set_bytes(&self, key: u32, value: *const c_void, bytes: usize) -> Result<()> {
        check(
            unsafe {
                UpdateProcThreadAttribute(
                    self.ptr,
                    0,
                    key as usize,
                    value,
                    bytes,
                    null_mut(),
                    null(),
                )
            },
            "UpdateProcThreadAttribute",
        )
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.ptr) };
    }
}

fn pipe(parent_reads: bool) -> Result<(Handle, Handle)> {
    let mut read = null_mut();
    let mut write = null_mut();
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    check(
        unsafe { CreatePipe(&mut read, &mut write, &sa, 0) },
        "CreatePipe",
    )?;
    let read = Handle(read);
    let write = Handle(write);
    let (parent, child) = if parent_reads {
        (read, write)
    } else {
        (write, read)
    };
    check(
        unsafe { SetHandleInformation(parent.0, HANDLE_FLAG_INHERIT, 0) },
        "SetHandleInformation",
    )?;
    Ok((parent, child))
}
fn file(handle: Handle) -> File {
    let handle = std::mem::ManuallyDrop::new(handle);
    unsafe { File::from_raw_handle(handle.0) }
}
fn limits(job: HANDLE) -> Result<JOBOBJECT_EXTENDED_LIMIT_INFORMATION> {
    let mut info = unsafe { zeroed() };
    check(
        unsafe {
            QueryInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                null_mut(),
            )
        },
        "QueryInformationJobObject",
    )?;
    Ok(info)
}
fn limit_json(info: &JOBOBJECT_EXTENDED_LIMIT_INFORMATION) -> Value {
    json!({"flags":info.BasicLimitInformation.LimitFlags,
        "active_process_limit":info.BasicLimitInformation.ActiveProcessLimit,
        "process_commit_limit_bytes":info.ProcessMemoryLimit,"job_commit_limit_bytes":info.JobMemoryLimit,
        "peak_process_commit_bytes":info.PeakProcessMemoryUsed,"peak_job_commit_bytes":info.PeakJobMemoryUsed})
}

fn process_snapshot() -> Result<Vec<Value>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    ensure!(
        snapshot != INVALID_HANDLE_VALUE,
        "process snapshot: {}",
        std::io::Error::last_os_error()
    );
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of_val(&entry) as u32;
    let mut rows = Vec::new();
    let mut rc = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while rc != 0 {
        let mut session = 0;
        let session_ok = unsafe { ProcessIdToSessionId(entry.th32ProcessID, &mut session) };
        rows.push(json!({"pid":entry.th32ProcessID,"parent_pid":entry.th32ParentProcessID,
            "session":if session_ok != 0 {Some(session)} else {None},
            "image_name":String::from_utf16_lossy(&entry.szExeFile[..entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len())])}));
        rc = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    ensure!(
        unsafe { GetLastError() } == ERROR_NO_MORE_FILES,
        "process enumeration incomplete"
    );
    Ok(rows)
}

fn low_scratch(path: &Path) -> Result<()> {
    let mut descriptor = null_mut();
    check(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide("S:(ML;OICI;NW;;;LW)").as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        },
        "scratch low label",
    )?;
    let mut sacl = null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    let got =
        unsafe { GetSecurityDescriptorSacl(descriptor, &mut present, &mut sacl, &mut defaulted) };
    let status = if got != 0 && present != 0 {
        unsafe {
            SetNamedSecurityInfoW(
                wide(path).as_ptr(),
                SE_FILE_OBJECT,
                LABEL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null(),
                sacl,
            )
        }
    } else {
        ERROR_INVALID_SECURITY_DESCR
    };
    unsafe { LocalFree(descriptor) };
    ensure!(status == 0, "scratch mandatory low label={status}");
    Ok(())
}

/// Query each class independently so unsupported classes survive in failed-row evidence.
fn token_flag(
    token: HANDLE,
    class: TOKEN_INFORMATION_CLASS,
    name: &str,
    trace: &mut Value,
) -> Result<u32> {
    let mut value = 0u32;
    let mut bytes = 0;
    let ok = unsafe {
        GetTokenInformation(
            token,
            class,
            &mut value as *mut _ as _,
            size_of_val(&value) as u32,
            &mut bytes,
        )
    };
    let error = if ok == 0 {
        Some(std::io::Error::last_os_error())
    } else {
        None
    };
    trace["token_queries"][name] = json!({"class":class,"succeeded":ok != 0,
        "returned_bytes":bytes,"value":if ok != 0 {Some(value)} else {None},
        "error_code":error.as_ref().and_then(|e| e.raw_os_error())});
    if let Some(error) = error {
        return Err(error).with_context(|| format!("GetTokenInformation({name}, class={class})"));
    }
    ensure!(
        bytes == size_of_val(&value) as u32,
        "{name}: unexpected result size={bytes}"
    );
    Ok(value)
}

/// Evaluate a synthetic file DACL with the effective child token, without resuming
/// it or changing any filesystem ACL. Everyone satisfies the normal user check;
/// the second ACE must satisfy the additional AppContainer package check.
/// https://learn.microsoft.com/windows/win32/api/securitybaseapi/nf-securitybaseapi-accesscheck
fn token_can_read(token: HANDLE, package: &str) -> Result<bool> {
    let mut descriptor = null_mut();
    check(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(format!("O:SYG:SYD:(A;;0x1;;;WD)(A;;0x1;;;{package})")).as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        },
        "LPAC read-check security descriptor",
    )?;
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ,
        GenericWrite: FILE_GENERIC_WRITE,
        GenericExecute: FILE_GENERIC_EXECUTE,
        GenericAll: FILE_ALL_ACCESS,
    };
    let mut privileges: PRIVILEGE_SET = unsafe { zeroed() };
    let mut bytes = size_of_val(&privileges) as u32;
    let mut granted = 0;
    let mut allowed = 0;
    let ok = unsafe {
        AccessCheck(
            descriptor,
            token,
            FILE_READ_DATA,
            &mapping,
            &mut privileges,
            &mut bytes,
            &mut granted,
            &mut allowed,
        )
    };
    // Capture the API error before LocalFree changes last-error state.
    let result = check(ok, "LPAC AccessCheck(FILE_READ_DATA)");
    unsafe { LocalFree(descriptor) };
    result?;
    ensure!(
        allowed == 0 || granted & FILE_READ_DATA != 0,
        "LPAC AccessCheck inconsistent grant"
    );
    Ok(allowed != 0)
}

fn lpac_access_readback(token: HANDLE, sid: PSID, lpac: bool, trace: &mut Value) -> Result<()> {
    let mut impersonation = null_mut();
    check(
        unsafe { DuplicateToken(token, SecurityImpersonation, &mut impersonation) },
        "LPAC DuplicateToken",
    )?;
    let impersonation = Handle(impersonation);
    let mut sid_text = null_mut();
    check(
        unsafe { ConvertSidToStringSidW(sid, &mut sid_text) },
        "LPAC package SID string",
    )?;
    let mut len = 0;
    unsafe {
        while *sid_text.add(len) != 0 {
            len += 1;
        }
    }
    let package = unsafe { String::from_utf16(std::slice::from_raw_parts(sid_text, len)) };
    unsafe { LocalFree(sid_text.cast()) };
    let package = package?;
    let specific = token_can_read(impersonation.0, &package);
    let all_packages = token_can_read(impersonation.0, "S-1-15-2-1");
    trace["lpac_access_check"] = json!({"method":"AccessCheck(FILE_READ_DATA)",
        "container_sid_read":specific.as_ref().ok(),
        "all_application_packages_read":all_packages.as_ref().ok(),
        "container_sid_error":specific.as_ref().err().map(|e| format!("{e:#}")),
        "all_application_packages_error":all_packages.as_ref().err().map(|e| format!("{e:#}"))});
    ensure!(
        specific?,
        "LPAC verification failed: container SID cannot read positive control"
    );
    ensure!(all_packages? != lpac, "LPAC verification failed: ALL APPLICATION PACKAGES access does not match requested isolation");
    Ok(())
}

fn token_readback(process: HANDLE, sid: PSID, lpac: bool, trace: &mut Value) -> Result<Value> {
    trace["token_queries"] = json!({});
    trace["lpac_verification"] = json!("unverified");
    let mut token = null_mut();
    check(
        unsafe { OpenProcessToken(process, TOKEN_QUERY | TOKEN_DUPLICATE, &mut token) },
        "OpenProcessToken",
    )?;
    let token = Handle(token);
    // Do not short-circuit: retain the outcome of each queried information class.
    let appcontainer = token_flag(token.0, TokenIsAppContainer, "TokenIsAppContainer", trace);
    let less_privileged = token_flag(
        token.0,
        TokenIsLessPrivilegedAppContainer,
        "TokenIsLessPrivilegedAppContainer",
        trace,
    );
    let capabilities = (|| -> Result<u32> {
        let mut bytes = 0;
        let ok =
            unsafe { GetTokenInformation(token.0, TokenCapabilities, null_mut(), 0, &mut bytes) };
        let error = if ok == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        trace["token_queries"]["TokenCapabilities"] = json!({"class":TokenCapabilities,
            "size_query_error_code":error,"required_bytes":bytes,"succeeded":false});
        ensure!(
            ok == 0 && error == ERROR_INSUFFICIENT_BUFFER && bytes >= size_of::<u32>() as u32,
            "TokenCapabilities size query: error={error}, bytes={bytes}"
        );
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenCapabilities,
                buffer.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        };
        let error = if ok == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        trace["token_queries"]["TokenCapabilities"]["error_code"] = json!(error);
        trace["token_queries"]["TokenCapabilities"]["succeeded"] = json!(ok != 0);
        ensure!(
            ok != 0,
            "GetTokenInformation(TokenCapabilities): os error {error}"
        );
        ensure!(
            bytes >= size_of::<u32>() as u32 && bytes as usize <= buffer.len() * size_of::<usize>(),
            "TokenCapabilities result size"
        );
        let count = unsafe { *(buffer.as_ptr() as *const u32) };
        trace["token_queries"]["TokenCapabilities"]["count"] = json!(count);
        Ok(count)
    })();
    let appcontainer = appcontainer?;
    ensure!(
        appcontainer == 1,
        "effective token is not an AppContainer: {appcontainer}"
    );
    let count = capabilities?;
    ensure!(count == 0, "effective token has capabilities={count}");
    let method = match less_privileged {
        Ok(value) => {
            ensure!(
                value == u32::from(lpac),
                "effective TokenIsLessPrivilegedAppContainer={value}, expected={lpac}"
            );
            "TokenIsLessPrivilegedAppContainer"
        }
        Err(error) => {
            let code = error
                .downcast_ref::<std::io::Error>()
                .and_then(|e| e.raw_os_error());
            ensure!(
                matches!(code, Some(code) if code == ERROR_INVALID_PARAMETER as i32 || code == ERROR_NOT_SUPPORTED as i32),
                "LPAC verification failed: {error:#}"
            );
            lpac_access_readback(token.0, sid, lpac, trace)
                .context("LPAC cannot be verified; refusing to resume worker")?;
            "AccessCheck: container SID allowed; ALL APPLICATION PACKAGES isolation checked"
        }
    };
    trace["lpac_verification"] = json!(method);
    Ok(json!({"appcontainer":appcontainer,"lpac":u32::from(lpac),
        "capability_count":count,"lpac_verification":method}))
}

/// Sorted UTF-16 entries, exactly double-NUL terminated. Only named directory
/// variables are copied; never PATH, credentials, proxies, or the full host env.
fn minimal_environment(
    scratch: &Path,
    local_app_data: bool,
    expanded: bool,
) -> Result<(Vec<u16>, Vec<&'static str>)> {
    let mut entries = std::collections::BTreeMap::new();
    entries.insert(
        "SystemRoot",
        std::env::var_os("SystemRoot").context("SystemRoot")?,
    );
    entries.insert("TEMP", scratch.as_os_str().to_owned());
    entries.insert("TMP", scratch.as_os_str().to_owned());
    if local_app_data || expanded {
        entries.insert("LOCALAPPDATA", scratch.as_os_str().to_owned());
    }
    if expanded {
        for key in [
            "USERPROFILE",
            "APPDATA",
            "ProgramData",
            "SystemDrive",
            "windir",
        ] {
            entries.insert(key, std::env::var_os(key).with_context(|| key)?);
        }
    }
    let mut entries: Vec<_> = entries.into_iter().collect();
    entries.sort_by_key(|(key, _)| key.to_ascii_uppercase());
    let keys = entries.iter().map(|(key, _)| *key).collect();
    let mut block = Vec::new();
    for (key, value) in entries {
        block.extend(OsStr::new(key).encode_wide());
        block.push(b'=' as u16);
        let value: Vec<_> = value.encode_wide().collect();
        ensure!(!value.contains(&0), "environment value contains NUL");
        block.extend(value);
        block.push(0);
    }
    block.push(0);
    Ok((block, keys))
}

/// Arguments are fixed paths; never forward arbitrary command strings to CreateProcess.
pub fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() >= 6,
        "worker mode request.json result.json scratch attack-file [exact-input ...]"
    );
    let result_path = PathBuf::from(&args[3]);
    let mut trace = json!({"profile_created_before_launch":false,"create_process_succeeded":false});
    let result = run(&args, &mut trace);
    // Even setup errors have durable evidence. No private paths or request bytes in JSON.
    let mut report = match &result {
        Ok(v) => v.clone(),
        Err(e) => json!({"status":"fail", "phase":"launch-or-collect", "reason":format!("{e:#}")}),
    };
    report["launch_trace"] = trace;
    std::fs::write(result_path, serde_json::to_vec_pretty(&report)?)?;
    result.map(|_| ())
}

fn run(args: &[std::ffi::OsString], trace: &mut Value) -> Result<Value> {
    let mode = args[1].to_str().context("mode")?;
    let diagnostic = mode.starts_with("diagnostic-");
    ensure!(
        !diagnostic
            || matches!(
                mode,
                "diagnostic-baseline"
                    | "diagnostic-null-environment"
                    | "diagnostic-expanded-environment"
                    | "diagnostic-null-directory"
                    | "diagnostic-plain-appcontainer"
                    | "diagnostic-localappdata"
            ),
        "invalid diagnostic"
    );
    let lpac = mode != "diagnostic-plain-appcontainer";
    let worker = std::fs::canonicalize(&args[0])?;
    let confined = match args[1].to_str() {
        Some("confined" | "confined-process" | "confined-job") => true,
        Some("control") => false,
        _ if diagnostic => true,
        _ => bail!("invalid mode"),
    };
    trace["isolation_requested"] = json!(if !confined {
        "unconfined"
    } else if lpac {
        "lpac"
    } else {
        "appcontainer"
    });
    let request = std::fs::read(&args[2])?;
    ensure!(request.len() <= 32 * 1024 * 1024, "request budget");
    let scratch = std::fs::canonicalize(&args[4])?;
    let mut profile = if confined {
        Some(Profile::new(trace)?)
    } else {
        None
    };
    if profile.is_some() {
        trace["profile_created_before_launch"] = json!(true);
        trace["profile_api"] = json!("CreateAppContainerProfile");
        trace["profile_capability_count"] = json!(0);
    }
    let mut grants = Vec::new();
    if let Some(profile) = &profile {
        low_scratch(&scratch)?;
        grants.push(Grant::new(
            &worker,
            profile.sid,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            0,
        )?);
        grants.push(Grant::new(
            &scratch,
            profile.sid,
            FILE_ALL_ACCESS,
            SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        )?);
        for input in &args[6..] {
            let path = std::fs::canonicalize(input)?;
            ensure!(path.is_file(), "exact regular file grants only");
            grants.push(Grant::new(&path, profile.sid, FILE_GENERIC_READ, 0)?);
        }
    }
    let (stdin, child_stdin) = pipe(false)?;
    let (stdout, child_stdout) = pipe(true)?;
    let (stderr, child_stderr) = pipe(true)?;
    // Intentionally inheritable host handle, omitted from the explicit allowlist.
    let attack = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&args[5])?;
    use std::os::windows::io::AsRawHandle;
    check(
        unsafe {
            SetHandleInformation(
                attack.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        },
        "attack handle inheritability",
    )?;
    // Probe request is JSON, ordinary parser requests keep their wire shape.
    let mut value: Value = serde_json::from_slice(&request)?;
    let job_name = format!(
        "Local\\vectoraiz-C1-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    if let Some(probe) = value.get_mut("office_windows_probe") {
        probe["attack_handle"] = json!(attack.as_raw_handle() as usize);
        probe["job_name"] = json!(job_name);
    }
    let request = serde_json::to_vec(&value)?;
    let mut handles = vec![child_stdin.0, child_stdout.0, child_stderr.0];
    if !confined {
        handles.push(attack.as_raw_handle());
    }
    let attributes = Attributes::new(if confined {
        if lpac {
            3
        } else {
            2
        }
    } else {
        1
    })?;
    attributes.set_bytes(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        handles.as_ptr().cast(),
        size_of_val(handles.as_slice()),
    )?;
    let mut caps: SECURITY_CAPABILITIES = unsafe { zeroed() };
    let opt_out: u32 = 1; // PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT
    if let Some(profile) = &profile {
        caps.AppContainerSid = profile.sid;
        attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &caps)?;
        // ProcThreadAttributeValue(15, FALSE, TRUE, FALSE), SDK macro not exported by windows-sys.
        if lpac {
            attributes.set(0x0002_000f, &opt_out)?;
        }
    }
    let job = {
        let handle = Handle(unsafe { CreateJobObjectW(null(), wide(&job_name).as_ptr()) });
        ensure!(
            !handle.0.is_null(),
            "CreateJobObjectW: {}",
            std::io::Error::last_os_error()
        );
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        info.BasicLimitInformation.LimitFlags = if confined {
            JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_JOB_MEMORY
                | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        } else {
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK
        };
        info.BasicLimitInformation.ActiveProcessLimit = if confined { 1 } else { 0 };
        // Independent diagnostics retain the LPAC and process-count restriction.
        // Only the non-tested ceiling is loosened; ordinary runs retain both 1.5 GiB limits.
        info.ProcessMemoryLimit = if args[1] == "confined-job" {
            MEMORY * 2
        } else if confined {
            MEMORY
        } else {
            0
        };
        info.JobMemoryLimit = if args[1] == "confined-process" {
            MEMORY * 2
        } else if confined {
            MEMORY
        } else {
            0
        };
        check(
            unsafe {
                SetInformationJobObject(
                    handle.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as _,
                    size_of_val(&info) as u32,
                )
            },
            "SetInformationJobObject",
        )?;
        let got = limits(handle.0)?;
        ensure!(
            got.BasicLimitInformation.LimitFlags == info.BasicLimitInformation.LimitFlags
                && got.BasicLimitInformation.ActiveProcessLimit
                    == info.BasicLimitInformation.ActiveProcessLimit
                && got.ProcessMemoryLimit == info.ProcessMemoryLimit
                && got.JobMemoryLimit == info.JobMemoryLimit,
            "job limit readback mismatch"
        );
        Some(handle)
    };
    let before = job
        .as_ref()
        .map(|j| limits(j.0).map(|i| limit_json(&i)))
        .transpose()?;
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of_val(&startup) as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = child_stdin.0;
    startup.StartupInfo.hStdOutput = child_stdout.0;
    startup.StartupInfo.hStdError = child_stderr.0;
    startup.lpAttributeList = attributes.ptr;
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    let application = wide(&worker);
    let mut command = wide(format!("\"{}\"", worker.display()));
    // AppContainer setup rewrites profile-dependent variables. LOCALAPPDATA is
    // required even with an explicit, otherwise minimal environment (error 203).
    // https://learn.microsoft.com/windows/win32/secauthz/implementing-an-appcontainer
    let mut environment = minimal_environment(
        &scratch,
        !diagnostic || mode == "diagnostic-localappdata",
        mode == "diagnostic-expanded-environment",
    )?;
    trace["environment_keys"] = if mode == "diagnostic-null-environment" {
        Value::Null
    } else {
        json!(environment.1)
    };
    trace["environment_inherited"] = json!(mode == "diagnostic-null-environment");
    trace["current_directory_null"] = json!(mode == "diagnostic-null-directory");
    let directory = wide(&scratch);
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED
                | EXTENDED_STARTUPINFO_PRESENT
                | CREATE_UNICODE_ENVIRONMENT
                | CREATE_NO_WINDOW,
            if mode == "diagnostic-null-environment" {
                null()
            } else {
                environment.0.as_mut_ptr().cast()
            },
            if mode == "diagnostic-null-directory" {
                null()
            } else {
                directory.as_ptr()
            },
            &startup.StartupInfo,
            &mut process,
        )
    };
    if created == 0 {
        let error = std::io::Error::last_os_error();
        trace["create_process_error_code"] = json!(error.raw_os_error());
        bail!("CreateProcessW suspended: {error}");
    }
    trace["create_process_succeeded"] = json!(true);
    let process_handle = Handle(process.hProcess);
    let thread = Handle(process.hThread);
    // An error after creation always kills the suspended/running worker.
    struct Kill(HANDLE);
    impl Drop for Kill {
        fn drop(&mut self) {
            unsafe {
                TerminateProcess(self.0, 125);
                WaitForSingleObject(self.0, 5000);
            }
        }
    }
    let kill = Kill(process_handle.0);
    if let Some(job) = &job {
        check(
            unsafe { AssignProcessToJobObject(job.0, process_handle.0) },
            "AssignProcessToJobObject before resume",
        )?;
        let mut assigned = 0;
        check(
            unsafe { IsProcessInJob(process_handle.0, job.0, &mut assigned) },
            "IsProcessInJob",
        )?;
        ensure!(assigned != 0, "job assignment readback");
    }
    let token = if confined {
        Some(token_readback(
            process_handle.0,
            profile.as_ref().context("confined profile")?.sid,
            lpac,
            trace,
        )?)
    } else {
        None
    };
    if diagnostic {
        // In particular, NULL environment inherits secrets: never resume any
        // diagnostic child. Probe rows always use the sanitized LPAC path.
        check(
            unsafe { TerminateProcess(process_handle.0, 0) },
            "diagnostic termination",
        )?;
        ensure!(
            unsafe { WaitForSingleObject(process_handle.0, 5000) } == WAIT_OBJECT_0,
            "diagnostic termination wait"
        );
        drop(kill);
        drop(thread);
        drop(process_handle);
        drop(job);
        for grant in grants.iter_mut().rev() {
            grant.restore()?;
        }
        if let Some(profile) = &mut profile {
            profile.remove()?;
        }
        return Ok(json!({"status":"pass","mode":mode,"worker_resumed":false,
            "token_before_resume":token,"job_before_resume":before}));
    }
    let observe_shell = value["office_windows_probe"]["op"] == "shell";
    let shell_before = if observe_shell {
        Some(process_snapshot())
    } else {
        None
    };
    let shell_start = Instant::now();
    ensure!(
        unsafe { ResumeThread(thread.0) } != u32::MAX,
        "ResumeThread failed"
    );
    drop(thread);
    drop(child_stdin);
    drop(child_stdout);
    drop(child_stderr);
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut stdin = file(stdin);
        stdin.write_all(&(request.len() as u32).to_le_bytes())?;
        stdin.write_all(&request)
    });
    fn collect(
        handle: Handle,
        path: PathBuf,
    ) -> std::thread::JoinHandle<std::io::Result<(Vec<u8>, usize)>> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut total = 0;
            let mut source = file(handle);
            let mut durable = File::create(path)?;
            let mut chunk = [0u8; 8192];
            loop {
                let count = source.read(&mut chunk)?;
                if count == 0 {
                    break;
                }
                durable.write_all(&chunk[..count])?;
                total += count;
                let keep = count.min((32 * 1024 * 1024 + 5usize).saturating_sub(bytes.len()));
                bytes.extend_from_slice(&chunk[..keep]);
            }
            Ok((bytes, total))
        })
    }
    let result_path = Path::new(&args[3]);
    let output = collect(stdout, result_path.with_extension("stdout.bin"));
    let errors = collect(stderr, result_path.with_extension("stderr.log"));
    let mut shell_seen = std::collections::BTreeMap::new();
    let mut shell_errors = Vec::new();
    let mut sample_shell = || {
        if !observe_shell {
            return;
        }
        match process_snapshot() {
            Ok(rows) => {
                for row in rows {
                    let pid = row["pid"].as_u64().unwrap_or(0);
                    if let Some(Ok(before)) = &shell_before {
                        if !before.iter().any(|old| {
                            old["pid"] == row["pid"] && old["image_name"] == row["image_name"]
                        }) {
                            shell_seen.insert(pid, row);
                        }
                    }
                }
            }
            Err(error) => shell_errors.push(error.to_string()),
        }
    };
    let start = Instant::now();
    let deadline = Duration::from_secs(45);
    let mut timed_out = false;
    loop {
        sample_shell();
        let wait = unsafe { WaitForSingleObject(process_handle.0, 50) };
        if wait == WAIT_OBJECT_0 {
            break;
        }
        ensure!(wait == WAIT_TIMEOUT, "WaitForSingleObject failed");
        if start.elapsed() >= deadline {
            check(
                unsafe { TerminateProcess(process_handle.0, 124) },
                "deadline termination",
            )?;
            timed_out = true;
            unsafe { WaitForSingleObject(process_handle.0, 5000) };
            break;
        }
    }
    let shell_after_worker = Instant::now();
    while observe_shell && shell_after_worker.elapsed() < Duration::from_secs(5) {
        sample_shell();
        std::thread::sleep(Duration::from_millis(50));
    }
    let shell_observation = if observe_shell {
        json!({"window_ms":shell_start.elapsed().as_millis(),"after_worker_window_ms":5000,"poll_interval_ms":50,
            "baseline_available":matches!(shell_before, Some(Ok(_))),
            "baseline_error":shell_before.as_ref().and_then(|r| r.as_ref().err()).map(|e| e.to_string()),
            "new_processes":shell_seen.values().collect::<Vec<_>>(),"snapshot_errors":shell_errors,
            "launch_ruled_out":false,
            "limitation":"Snapshots can miss short-lived processes and reuse of an existing handler; absence cannot rule out launch"})
    } else {
        Value::Null
    };
    let mut exit = 0;
    check(
        unsafe { GetExitCodeProcess(process_handle.0, &mut exit) },
        "GetExitCodeProcess",
    )?;
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { zeroed() };
    let counters_ok = unsafe {
        GetProcessMemoryInfo(
            process_handle.0,
            &mut counters as *mut _ as _,
            size_of_val(&counters) as u32,
        )
    } != 0;
    let after = job
        .as_ref()
        .map(|j| limits(j.0).map(|i| limit_json(&i)))
        .transpose()?;
    drop(kill);
    // Kill-on-close also closes descendants in a control-escape failure.
    drop(job);
    let write_error = writer
        .join()
        .map_err(|_| anyhow::anyhow!("writer panic"))?
        .err()
        .map(|e| e.to_string());
    let (output, output_total) = output
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader panic"))??;
    let (errors, errors_total) = errors
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panic"))??;
    // Full streams were spooled privately; only bounded prefixes enter memory.
    let capture_overflow = output_total > 32 * 1024 * 1024 + 4 || errors_total > 32 * 1024 * 1024;
    let mut stderr_excerpt = String::from_utf8_lossy(&errors).into_owned();
    for argument in args {
        let path = argument.to_string_lossy();
        if path.len() > 3 {
            stderr_excerpt = stderr_excerpt.replace(path.as_ref(), "<redacted-path>");
        }
    }
    for key in ["sentinel", "directory", "created", "scratch", "address"] {
        if let Some(path) = value["office_windows_probe"][key].as_str() {
            if !path.is_empty() {
                stderr_excerpt = stderr_excerpt.replace(path, "<redacted>");
            }
        }
    }
    for key in ["USERPROFILE", "SystemRoot"] {
        if let Ok(path) = std::env::var(key) {
            if !path.is_empty() {
                stderr_excerpt = stderr_excerpt.replace(&path, "<redacted-path>");
            }
        }
    }
    let stderr_excerpt_truncated = stderr_excerpt.len() > 2048 || errors_total > errors.len();
    let mut excerpt_end = stderr_excerpt.len().min(2048);
    while !stderr_excerpt.is_char_boundary(excerpt_end) {
        excerpt_end -= 1;
    }
    stderr_excerpt.truncate(excerpt_end);
    let memory_progress: Vec<Value> = String::from_utf8_lossy(&errors)
        .lines()
        .filter_map(|line| line.strip_prefix("MEMORY_PROGRESS "))
        .filter_map(|line| serde_json::from_str(line).ok())
        .rev()
        .take(4)
        .collect();
    let response = if output.len() >= 4 {
        let len = u32::from_le_bytes(output[..4].try_into()?) as usize;
        if len <= 32 * 1024 * 1024 && output.len() == len + 4 {
            serde_json::from_slice::<Value>(&output[4..]).ok()
        } else {
            None
        }
    } else {
        None
    };
    for grant in grants.iter_mut().rev() {
        grant.restore()?;
    }
    // Profile deletion is explicit so cleanup failure cannot turn into a pass.
    if let Some(profile) = &mut profile {
        profile.remove()?;
    }
    Ok(
        json!({"status":if exit == 0 && !timed_out && !capture_overflow && write_error.is_none() && response.is_some() {"pass"} else {"fail"},
        "mode":args[1].to_string_lossy(),"exit_code":exit,"deadline_kill":timed_out,
        "elapsed_ms":start.elapsed().as_millis(),"job_before_resume":before,"job_after":after,
        "token_before_resume":token,"sampler_enabled":false,"memory_counters_available":counters_ok,
        "working_set_bytes":if counters_ok {Some(counters.WorkingSetSize)} else {None},
        "peak_working_set_bytes":if counters_ok {Some(counters.PeakWorkingSetSize)} else {None},
        "private_commit_bytes":if counters_ok {Some(counters.PrivateUsage)} else {None},
        "stdin_error":write_error,"stderr_capture_available":true,"stderr_bytes":errors_total,"stderr_excerpt":stderr_excerpt,
        "stderr_excerpt_truncated":stderr_excerpt_truncated,"shell_launch_observation":shell_observation,
        "capture_overflow":capture_overflow,"last_memory_progress_reverse_order":memory_progress,
        "capture_limit_bytes":32 * 1024 * 1024,"response":response}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_environment_uses_only_scratch_and_system_root() -> Result<()> {
        let scratch = Path::new(r"C:\container-scratch");
        let (block, keys) = minimal_environment(scratch, true, false)?;
        assert_eq!(keys, ["LOCALAPPDATA", "SystemRoot", "TEMP", "TMP"]);
        assert!(block.ends_with(&[0, 0]));
        let entries: Vec<_> = block
            .split(|unit| *unit == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf16(entry).unwrap())
            .collect();
        for key in ["LOCALAPPDATA", "TEMP", "TMP"] {
            assert!(entries.contains(&format!("{key}={}", scratch.display())));
        }
        Ok(())
    }

    #[test]
    fn suspended_lpac_and_plain_tokens_are_distinguished() -> Result<()> {
        let root = std::env::var_os("OFFICE_EVIDENCE_DIR")
            .or_else(|| std::env::var_os("KD_BRIDGE_JOB_EVIDENCE"))
            .context("token regression requires an external evidence directory")?;
        for lpac in [false, true] {
            let mut trace = json!({});
            let mut profile = Profile::new(&mut trace)?;
            let scratch =
                PathBuf::from(&root).join(format!("token-test-{}-{lpac}", std::process::id()));
            std::fs::create_dir(&scratch)?;
            low_scratch(&scratch)?;
            let worker = std::env::current_exe()?;
            let mut executable_grant = Grant::new(
                &worker,
                profile.sid,
                FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
                0,
            )?;
            let mut scratch_grant = Grant::new(
                &scratch,
                profile.sid,
                FILE_ALL_ACCESS,
                SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            )?;
            let attributes = Attributes::new(if lpac { 2 } else { 1 })?;
            let caps = SECURITY_CAPABILITIES {
                AppContainerSid: profile.sid,
                ..Default::default()
            };
            attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &caps)?;
            let opt_out = 1u32;
            if lpac {
                attributes.set(0x0002_000f, &opt_out)?;
            }
            let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.lpAttributeList = attributes.ptr;
            let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
            let mut environment = minimal_environment(&scratch, true, false)?.0;
            check(
                unsafe {
                    CreateProcessW(
                        wide(&worker).as_ptr(),
                        null_mut(),
                        null(),
                        null(),
                        0,
                        CREATE_SUSPENDED
                            | EXTENDED_STARTUPINFO_PRESENT
                            | CREATE_UNICODE_ENVIRONMENT
                            | CREATE_NO_WINDOW,
                        environment.as_mut_ptr().cast(),
                        wide(&scratch).as_ptr(),
                        &startup.StartupInfo,
                        &mut process,
                    )
                },
                "token regression CreateProcessW",
            )?;
            struct Suspended {
                process: Handle,
            }
            impl Drop for Suspended {
                fn drop(&mut self) {
                    unsafe {
                        TerminateProcess(self.process.0, 0);
                        WaitForSingleObject(self.process.0, 5000);
                    }
                }
            }
            let child = Suspended {
                process: Handle(process.hProcess),
            };
            let thread = Handle(process.hThread);
            let verified = token_readback(child.process.0, profile.sid, lpac, &mut trace)?;
            assert_eq!(verified["appcontainer"], 1);
            assert_eq!(verified["lpac"], u32::from(lpac));
            assert_eq!(verified["capability_count"], 0);
            // Exercise the fallback even on OS builds that implement class 46.
            let mut token = null_mut();
            check(
                unsafe {
                    OpenProcessToken(child.process.0, TOKEN_QUERY | TOKEN_DUPLICATE, &mut token)
                },
                "token regression OpenProcessToken",
            )?;
            let token = Handle(token);
            lpac_access_readback(token.0, profile.sid, lpac, &mut trace)?;
            assert_eq!(trace["lpac_access_check"]["container_sid_read"], true);
            assert_eq!(
                trace["lpac_access_check"]["all_application_packages_read"],
                !lpac
            );
            assert!(lpac_access_readback(token.0, profile.sid, !lpac, &mut json!({})).is_err());
            assert!(token_readback(child.process.0, profile.sid, !lpac, &mut json!({})).is_err());
            println!("token regression lpac={lpac}: {trace}");
            drop(thread);
            drop(child);
            scratch_grant.restore()?;
            executable_grant.restore()?;
            profile.remove()?;
            std::fs::remove_dir(scratch)?;
        }
        Ok(())
    }
}
