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
    System::{JobObjects::*, Pipes::*, ProcessStatus::*, Threading::*},
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
    fn new() -> Result<Self> {
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
        ensure!(hr >= 0, "CreateAppContainerProfile HRESULT={hr:#x}");
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
        check(
            unsafe {
                UpdateProcThreadAttribute(
                    self.ptr,
                    0,
                    key as usize,
                    value as *const T as *const c_void,
                    size_of::<T>(),
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

fn token_readback(process: HANDLE) -> Result<Value> {
    let mut token = null_mut();
    check(
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) },
        "OpenProcessToken",
    )?;
    let token = Handle(token);
    let mut fields = Vec::new();
    for class in [TokenIsAppContainer, TokenIsLessPrivilegedAppContainer] {
        let mut value = 0u32;
        let mut bytes = 0;
        check(
            unsafe {
                GetTokenInformation(
                    token.0,
                    class,
                    &mut value as *mut _ as _,
                    size_of_val(&value) as u32,
                    &mut bytes,
                )
            },
            "GetTokenInformation",
        )?;
        ensure!(value == 1, "effective token class={class}, value={value}");
        fields.push(value);
    }
    let mut bytes = 0;
    unsafe { GetTokenInformation(token.0, TokenCapabilities, null_mut(), 0, &mut bytes) };
    ensure!(
        bytes >= size_of::<TOKEN_GROUPS>() as u32,
        "TokenCapabilities size"
    );
    let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
    check(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenCapabilities,
                buffer.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        },
        "TokenCapabilities",
    )?;
    let count = unsafe { (*(buffer.as_ptr() as *const TOKEN_GROUPS)).GroupCount };
    ensure!(count == 0, "effective token has capabilities={count}");
    Ok(json!({"appcontainer":fields[0],"lpac":fields[1],"capability_count":count}))
}

/// Arguments are fixed paths; never forward arbitrary command strings to CreateProcess.
pub fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() >= 6,
        "worker mode request.json result.json scratch attack-file [exact-input ...]"
    );
    let result_path = PathBuf::from(&args[3]);
    let result = run(&args);
    // Even setup errors have durable evidence. No private paths or request bytes in JSON.
    let report = match &result {
        Ok(v) => v.clone(),
        Err(e) => json!({"status":"fail", "phase":"launch-or-collect", "reason":format!("{e:#}")}),
    };
    std::fs::write(result_path, serde_json::to_vec_pretty(&report)?)?;
    result.map(|_| ())
}

fn run(args: &[std::ffi::OsString]) -> Result<Value> {
    let worker = std::fs::canonicalize(&args[0])?;
    let confined = match args[1].to_str() {
        Some("confined") => true,
        Some("control") => false,
        _ => bail!("invalid mode"),
    };
    let request = std::fs::read(&args[2])?;
    ensure!(request.len() <= 32 * 1024 * 1024, "request budget");
    let scratch = std::fs::canonicalize(&args[4])?;
    let mut profile = if confined {
        Some(Profile::new()?)
    } else {
        None
    };
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
    if let Some(probe) = value.get_mut("office_windows_probe") {
        probe["attack_handle"] = json!(attack.as_raw_handle() as usize);
    }
    let request = serde_json::to_vec(&value)?;
    let handles = [child_stdin.0, child_stdout.0, child_stderr.0];
    let attributes = Attributes::new(if confined { 3 } else { 1 })?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &handles)?;
    let mut caps: SECURITY_CAPABILITIES = unsafe { zeroed() };
    let opt_out: u32 = 1; // PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT
    if let Some(profile) = &profile {
        caps.AppContainerSid = profile.sid;
        attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &caps)?;
        // ProcThreadAttributeValue(15, FALSE, TRUE, FALSE), SDK macro not exported by windows-sys.
        attributes.set(0x0002_000f, &opt_out)?;
    }
    let job = if confined {
        let handle = Handle(unsafe { CreateJobObjectW(null(), null()) });
        ensure!(
            !handle.0.is_null(),
            "CreateJobObjectW: {}",
            std::io::Error::last_os_error()
        );
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_JOB_MEMORY
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.BasicLimitInformation.ActiveProcessLimit = 1;
        info.ProcessMemoryLimit = MEMORY;
        info.JobMemoryLimit = MEMORY;
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
                && got.BasicLimitInformation.ActiveProcessLimit == 1
                && got.ProcessMemoryLimit == MEMORY
                && got.JobMemoryLimit == MEMORY,
            "job limit readback mismatch"
        );
        Some(handle)
    } else {
        None
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
    // Minimal environment: no host credentials, proxies, home/profile, or search PATH.
    let system_root = std::env::var_os("SystemRoot").context("SystemRoot")?;
    let mut environment = wide(format!(
        "SystemRoot={}\0TEMP={}\0TMP={}\0",
        system_root.to_string_lossy(),
        scratch.display(),
        scratch.display()
    ));
    environment.push(0);
    check(
        unsafe {
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
                environment.as_ptr().cast(),
                wide(&scratch).as_ptr(),
                &startup.StartupInfo,
                &mut process,
            )
        },
        "CreateProcessW suspended",
    )?;
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
        Some(token_readback(process_handle.0)?)
    } else {
        None
    };
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
    fn collect(handle: Handle) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            file(handle)
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        })
    }
    let output = collect(stdout);
    let errors = collect(stderr);
    let start = Instant::now();
    let deadline = Duration::from_secs(45);
    let mut timed_out = false;
    loop {
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
    let output = output
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader panic"))??;
    let errors = errors
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panic"))??;
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
        json!({"status":if exit == 0 && !timed_out && response.is_some() {"pass"} else {"fail"},
        "mode":if confined {"confined"} else {"control"},"exit_code":exit,"deadline_kill":timed_out,
        "elapsed_ms":start.elapsed().as_millis(),"job_before_resume":before,"job_after":after,
        "token_before_resume":token,"sampler_enabled":false,"memory_counters_available":counters_ok,
        "working_set_bytes":if counters_ok {Some(counters.WorkingSetSize)} else {None},
        "peak_working_set_bytes":if counters_ok {Some(counters.PeakWorkingSetSize)} else {None},
        "private_commit_bytes":if counters_ok {Some(counters.PrivateUsage)} else {None},
        "stdin_error":write_error,"stderr_bytes":errors.len(),"response":response}),
    )
}
