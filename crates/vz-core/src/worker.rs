//! Bounded length-prefixed JSON worker protocol and process supervision.
use crate::pipeline::Page;
use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub const MAX_FRAME: usize = 32 * 1024 * 1024;
pub const MEMORY_BYTES: u64 = 1536 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
pub struct Request {
    pub path: PathBuf,
    pub pdfium: Option<PathBuf>,
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
    Ok(std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join(name))
}
pub fn run(worker: &Path, request: &Request, timeout: Duration) -> Result<Vec<Page>> {
    let mut cmd = command(worker);
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
        .stderr(Stdio::null());
    let mut child = cmd.spawn().context("start parser worker")?;
    #[cfg(windows)]
    let _job = windows_limit(&child)?;
    let mut stdin = child.stdin.take().context("worker stdin")?;
    if let Err(error) = write_frame(&mut stdin, request) {
        let _ = terminate(&mut child);
        let _ = child.wait();
        return Err(error);
    }
    drop(stdin);
    let stdout = child.stdout.take().context("worker stdout")?;
    let reader = std::thread::spawn(move || read_frame::<Response>(stdout));
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let response = reader
                .join()
                .map_err(|_| anyhow::anyhow!("worker reader panic"));
            if !status.success() {
                bail!("worker crashed: {status}");
            }
            let response = response??;
            if let Some(error) = response.error {
                return Err(if response.content_failure {
                    crate::failures::content(error)
                } else {
                    anyhow::anyhow!(error)
                });
            }
            return Ok(response.pages);
        }
        #[cfg(target_os = "macos")]
        if resident_bytes(child.id()) > MEMORY_BYTES {
            terminate(&mut child)?;
            child.wait()?;
            let _ = reader.join();
            bail!("worker resident memory exceeds 1.5 GiB");
        }
        if start.elapsed() >= timeout {
            terminate(&mut child)?;
            child.wait()?;
            let _ = reader.join();
            bail!("worker timeout after {}ms", timeout.as_millis());
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
        Command::new(worker)
    }
}
/// Called before reading the request, before a parser can allocate memory.
pub fn apply_memory_limit() -> Result<()> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let limit = libc::rlimit {
            rlim_cur: MEMORY_BYTES,
            rlim_max: MEMORY_BYTES,
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
fn windows_limit(child: &std::process::Child) -> Result<impl Drop> {
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
        info.ProcessMemoryLimit = MEMORY_BYTES as usize;
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
