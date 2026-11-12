//! Bounded length-prefixed JSON worker protocol and process supervision.
use crate::pipeline::Page;
use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub const MAX_FRAME: usize = 32 * 1024 * 1024;
pub const MEMORY_BYTES: u64 = 1536 * 1024 * 1024;
pub const OFFICE_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
pub struct OfficeJob {
    pub name: String,
    pub output: PathBuf,
    pub edits: std::collections::BTreeMap<String, crate::office::Edit>,
    pub readback: bool,
    #[serde(default)]
    pub sanitized_input: bool,
    #[serde(default)]
    pub chart_sources: crate::office::ChartSources,
}
#[derive(Serialize, Deserialize)]
pub struct Request {
    pub path: PathBuf,
    pub pdfium: Option<PathBuf>,
    #[serde(default)]
    pub office: Option<OfficeJob>,
}
impl Request {
    pub fn office_label(&self) -> Option<&str> {
        if let Some(job) = &self.office {
            return Some(&job.name);
        }
        self.path
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| {
                ["docx", "xlsx", "pptx"]
                    .iter()
                    .any(|ext| s.eq_ignore_ascii_case(ext))
            })
            .and_then(|_| self.path.to_str())
    }
}
#[derive(Serialize, Deserialize)]
pub struct Response {
    pub pages: Vec<Page>,
    pub error: Option<String>,
    #[serde(default)]
    pub content_failure: bool,
}
pub fn write_frame<T: Serialize>(mut writer: impl Write, value: &T) -> Result<()> {
    let data = serde_json::to_vec(value)?;
    if data.len() > MAX_FRAME {
        bail!("worker frame exceeds limit");
    }
    writer.write_all(&(data.len() as u32).to_le_bytes())?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
}
pub fn read_frame<T: DeserializeOwned>(mut reader: impl Read) -> Result<T> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length > MAX_FRAME {
        bail!("worker frame exceeds limit");
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn executable() -> Result<PathBuf> {
    let name = if cfg!(windows) {
        "vz-parser-worker.exe"
    } else {
        "vz-parser-worker"
    };
    let current = std::env::current_exe()?;
    let directory = current.parent().context("executable directory")?;
    // Integration tests live in target/{profile}/deps; production binaries
    // remain siblings in the installation directory.
    let directory = if directory.file_name().is_some_and(|n| n == "deps") {
        directory.parent().context("profile directory")?
    } else {
        directory
    };
    Ok(directory.join(name))
}
pub fn run(worker: &Path, request: &Request, timeout: Duration) -> Result<Vec<Page>> {
    // sandbox-exec itself may spawn even when the actual worker is missing.
    anyhow::ensure!(
        worker.is_file(),
        "parser worker missing: {}",
        worker.display()
    );
    run_command(command(worker), request, timeout, cfg!(target_os = "macos"))
}

pub fn run_cancellable(
    worker: &Path,
    request: &Request,
    timeout: Duration,
    check: impl Fn() -> Result<()>,
) -> Result<Vec<Page>> {
    anyhow::ensure!(
        worker.is_file(),
        "parser worker missing: {}",
        worker.display()
    );
    run_command_checked(
        command(worker),
        request,
        timeout,
        cfg!(target_os = "macos"),
        check,
    )
}

fn run_command(
    cmd: Command,
    request: &Request,
    timeout: Duration,
    sandboxed: bool,
) -> Result<Vec<Page>> {
    run_command_checked(cmd, request, timeout, sandboxed, || Ok(()))
}
fn run_command_checked(
    mut cmd: Command,
    request: &Request,
    timeout: Duration,
    sandboxed: bool,
    check: impl Fn() -> Result<()>,
) -> Result<Vec<Page>> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A parser and its OCR descendants form one killable process group.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd.env_clear();
    // Windows needs SystemRoot to load OS DLLs. No credentials, home, proxy or PATH.
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        cmd.env("SystemRoot", root);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("start parser worker")?;
    let mut stderr = child.stderr.take().context("worker stderr")?;
    let (diagnostic_tx, diagnostic_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut diagnostic = Vec::new();
        let mut buffer = [0; 4096];
        while let Ok(size) = stderr.read(&mut buffer) {
            if size == 0 {
                break;
            }
            let retain = size.min(4096 - diagnostic.len());
            diagnostic.extend_from_slice(&buffer[..retain]);
        }
        let _ = diagnostic_tx.send(diagnostic);
    });
    #[cfg(windows)]
    let _job = windows_limit(
        &child,
        if request.office_label().is_some() {
            OFFICE_MEMORY_BYTES
        } else {
            MEMORY_BYTES
        },
    )?;
    let mut stdin = child.stdin.take().context("worker stdin")?;
    // A wrapper can exit before consuming the request. Inspect its status and
    // stdout before classifying a broken pipe as a document failure.
    let request_error = write_frame(&mut stdin, request).err();
    drop(stdin);
    let stdout = child.stdout.take().context("worker stdout")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        let no_response_bytes = matches!(stdout.fill_buf(), Ok([]));
        let _ = sender.send((no_response_bytes, read_frame::<Response>(stdout)));
    });
    let start = Instant::now();
    let mut last_usage = 0;
    loop {
        if let Err(error) = check() {
            let _ = terminate(&mut child);
            let _ = child.wait();
            return Err(error);
        }
        if let Some(status) = child.try_wait()? {
            // A descendant can keep stdout open after the worker exits. The
            // whole group goes with the worker, and collection stays bounded.
            let _ = terminate(&mut child);
            let remaining = timeout
                .saturating_sub(start.elapsed())
                .max(Duration::from_millis(250));
            let response = receiver
                .recv_timeout(remaining)
                .map_err(|_| anyhow::anyhow!("worker response not received in time"));
            if !status.success() {
                let diagnostic = diagnostic_rx
                    .recv_timeout(Duration::from_millis(100))
                    .unwrap_or_default();
                if let Some(label) = request.office_label() {
                    if diagnostic
                        .windows(b"memory allocation of ".len())
                        .any(|w| w == b"memory allocation of ")
                        && diagnostic
                            .windows(b"bytes failed".len())
                            .any(|w| w == b"bytes failed")
                    {
                        return Err(crate::failures::content(format!("OFFICE_MEMORY_LIMIT_EXCEEDED: {label}: hard cap {OFFICE_MEMORY_BYTES} bytes; last observed {last_usage} bytes; allocation denied")));
                    }
                }
                #[cfg(windows)]
                if let Some(label) = request.office_label() {
                    if status
                        .code()
                        .is_some_and(|c| matches!(c as u32, 0xc0000017 | 0xc000009a))
                    {
                        return Err(crate::failures::content(format!("OFFICE_MEMORY_LIMIT_EXCEEDED: {label}: hard cap {OFFICE_MEMORY_BYTES} bytes; last observed {last_usage} bytes; allocation denied")));
                    }
                }
                if sandboxed
                    && status.code().is_some_and(|code| (64..=78).contains(&code))
                    && response.as_ref().is_ok_and(|(empty, _)| *empty)
                {
                    bail!("parser sandbox failed to start the worker: {status}");
                }
                return Err(crate::failures::content(format!(
                    "worker crashed: {status}"
                )));
            }
            if let Some(error) = request_error {
                return Err(crate::failures::content(format!(
                    "worker request failed: {error:#}"
                )));
            }
            let response = response
                .and_then(|(_, r)| r)
                .map_err(|e| crate::failures::content(format!("worker response failed: {e:#}")))?;
            if let Some(error) = response.error {
                return Err(if response.content_failure {
                    crate::failures::content(error)
                } else {
                    anyhow::anyhow!(error)
                });
            }
            return Ok(response.pages);
        }
        last_usage = resident_bytes(child.id());
        if last_usage
            > if request.office_label().is_some() {
                OFFICE_MEMORY_BYTES
            } else {
                MEMORY_BYTES
            }
        {
            let usage = last_usage;
            terminate(&mut child)?;
            child.wait()?;
            return Err(crate::failures::content(
                if let Some(label) = request.office_label() {
                    format!("OFFICE_MEMORY_LIMIT_EXCEEDED: {}: limit {OFFICE_MEMORY_BYTES} bytes; last observed {usage} bytes",label)
                } else {
                    "worker resident memory exceeds 1.5 GiB".into()
                },
            ));
        }
        if start.elapsed() >= timeout {
            terminate(&mut child)?;
            child.wait()?;
            return Err(crate::failures::content(format!(
                "worker timeout after {}ms",
                timeout.as_millis()
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub fn command(worker: &Path) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("/usr/bin/sandbox-exec");
        cmd.args(["-p", "(version 1)(allow default)(deny network*)"]);
        cmd.arg(worker);
        cmd
    }
    #[cfg(not(target_os = "macos"))]
    {
        let cmd = Command::new(worker);
        #[cfg(windows)]
        let cmd = {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let mut cmd = cmd;
            cmd.creation_flags(CREATE_NO_WINDOW);
            cmd
        };
        cmd
    }
}
/// Called before reading the request, before a parser can allocate memory.
pub fn apply_memory_limit() -> Result<()> {
    apply_memory_limit_bytes(MEMORY_BYTES)
}
pub fn apply_office_memory_limit() -> Result<()> {
    apply_memory_limit_bytes(OFFICE_MEMORY_BYTES)
}
fn apply_memory_limit_bytes(bytes: u64) -> Result<()> {
    let _ = bytes;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let limit = libc::rlimit {
            rlim_cur: bytes,
            rlim_max: bytes,
        };
        // macOS rejects RLIMIT_AS. Use its heap limit plus the parent RSS watchdog.
        if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) } != 0 {
            bail!(
                "cannot set worker memory limit: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    Ok(())
}
#[cfg(windows)]
fn windows_limit(child: &std::process::Child, bytes: u64) -> Result<impl Drop> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
    struct Job(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    unsafe {
        let job = Job(CreateJobObjectW(std::ptr::null(), std::ptr::null()));
        if job.0.is_null() {
            bail!("create worker job failed");
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        info.ProcessMemoryLimit = bytes as usize;
        if SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &info as *const _ as _,
            std::mem::size_of_val(&info) as u32,
        ) == 0
            || AssignProcessToJobObject(job.0, child.as_raw_handle() as _) == 0
        {
            bail!("assign worker memory job failed");
        }
        Ok(job)
    }
}

/// Worker-backed parser implementing the swappable extraction boundary.
pub struct SupervisedParser {
    pub executable: PathBuf,
    pub timeout: Duration,
}
impl crate::Parser for SupervisedParser {
    fn parse(&self, request: &Request) -> crate::ComponentResult<Vec<Page>> {
        run(&self.executable, request, self.timeout).map_err(|e| format!("{e:#}"))
    }
}

fn terminate(child: &mut std::process::Child) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        if unsafe { libc::killpg(child.id() as i32, libc::SIGKILL) } == 0 {
            return Ok(());
        }
    }
    child.kill()
}
#[cfg(target_os = "macos")]
fn resident_bytes(pid: u32) -> u64 {
    #[repr(C)]
    struct TaskInfo {
        virtual_size: u64,
        resident_size: u64,
        times: [u64; 4],
        counters: [i32; 12],
    }
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut std::ffi::c_void,
            size: i32,
        ) -> i32;
    }
    let mut info = TaskInfo {
        virtual_size: 0,
        resident_size: 0,
        times: [0; 4],
        counters: [0; 12],
    };
    let size = std::mem::size_of::<TaskInfo>() as i32;
    let got = unsafe { proc_pidinfo(pid as i32, 4, 0, &mut info as *mut _ as _, size) };
    if got == size {
        info.resident_size
    } else {
        0
    }
}
#[cfg(target_os = "linux")]
fn resident_bytes(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/statm"))
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or(0)
        * unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 }
}
#[cfg(windows)]
fn resident_bytes(pid: u32) -> u64 {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
    };
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if process.is_null() {
            return 0;
        }
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        let size = std::mem::size_of_val(&counters) as u32;
        counters.cb = size;
        let ok = K32GetProcessMemoryInfo(process, &mut counters as *mut _ as _, size);
        CloseHandle(process);
        if ok != 0 {
            counters.PrivateUsage as u64
        } else {
            0
        }
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn resident_bytes(_pid: u32) -> u64 {
    0
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn failed_command(script: &str, sandboxed: bool) -> anyhow::Error {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", script]);
        run_command(
            cmd,
            &Request {
                path: "source.txt".into(),
                pdfium: None,
                office: None,
            },
            Duration::from_secs(5),
            sandboxed,
        )
        .unwrap_err()
    }

    #[test]
    fn sandbox_sysexits_without_response_are_infrastructure() {
        for code in 64..=78 {
            let error = failed_command(&format!("cat >/dev/null; exit {code}"), true);
            assert!(!crate::failures::is_content(&error), "{error:#}");
            assert!(error
                .to_string()
                .starts_with("parser sandbox failed to start the worker: "));
        }
        // Also cover a wrapper that exits before reading the request.
        assert!(!crate::failures::is_content(&failed_command(
            "exit 71", true
        )));
    }

    #[test]
    fn descendant_holding_stdout_does_not_outlast_the_worker() {
        // stdin closed before the request (forcing BrokenPipe when the write
        // loses the race), and a descendant keeps stdout open after exit.
        for (code, content) in [(71, false), (1, true)] {
            let start = Instant::now();
            let error = failed_command(&format!("exec 0<&-; /bin/sleep 5 & exit {code}"), true);
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "{:?}",
                start.elapsed()
            );
            assert_eq!(crate::failures::is_content(&error), content, "{error:#}");
        }
    }

    #[test]
    fn other_worker_exits_remain_content_failures() {
        for script in [
            "cat >/dev/null; exit 0",
            "cat >/dev/null; exit 1",
            "cat >/dev/null; exit 63",
            "cat >/dev/null; exit 79",
            "cat >/dev/null; kill -KILL $$",
            "cat >/dev/null; printf x; exit 71",
        ] {
            let error = failed_command(script, true);
            assert!(crate::failures::is_content(&error), "{error:#}");
        }
        let error = failed_command("cat >/dev/null; exit 71", false);
        assert!(crate::failures::is_content(&error), "{error:#}");
    }
}
