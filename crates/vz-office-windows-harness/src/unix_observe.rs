//! Control-only, current-thread observation of mswsock's ntdll IAT entry.
//! No executable code is rewritten. Raw packets never leave this module.
use super::*;
use std::{
    cell::Cell,
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

type Ioctl = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
    *mut IoStatus,
    u32,
    *const std::ffi::c_void,
    u32,
    *mut std::ffi::c_void,
    u32,
) -> i32;
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
#[derive(Clone, Copy)]
struct Call {
    code: u32,
    input_len: u32,
    output_len: u32,
    bytes: [u8; 256],
}
const EMPTY: Call = Call {
    code: 0,
    input_len: 0,
    output_len: 0,
    bytes: [0; 256],
};
#[derive(Clone, Copy)]
struct Capture {
    calls: [Call; 16],
    count: usize,
    overflow: bool,
}
thread_local! { static CAPTURE: Cell<Option<Capture>> = const { Cell::new(None) }; }

unsafe extern "system" fn observe(
    file: HANDLE,
    event: HANDLE,
    apc: *mut std::ffi::c_void,
    context: *mut std::ffi::c_void,
    iosb: *mut IoStatus,
    code: u32,
    input: *const std::ffi::c_void,
    input_len: u32,
    output: *mut std::ffi::c_void,
    output_len: u32,
) -> i32 {
    // No allocation, locks, Win32 calls, or formatting in the hook. The caller
    // owns these input buffers for this call. Other threads simply forward.
    CAPTURE.with(|cell| {
        if let Some(mut capture) = cell.get() {
            if capture.count < capture.calls.len() {
                let mut call = Call {
                    code,
                    input_len,
                    output_len,
                    ..EMPTY
                };
                if !input.is_null() && input_len <= 256 {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            input.cast::<u8>(),
                            call.bytes.as_mut_ptr(),
                            input_len as usize,
                        )
                    };
                }
                capture.calls[capture.count] = call;
                capture.count += 1;
            } else {
                capture.overflow = true;
            }
            cell.set(Some(capture));
        }
    });
    let original: Ioctl = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    unsafe {
        original(
            file, event, apc, context, iosb, code, input, input_len, output, output_len,
        )
    }
}

struct Hook {
    slot: &'static AtomicUsize,
    original: usize,
    protection: u32,
}
impl Drop for Hook {
    fn drop(&mut self) {
        self.slot.store(self.original, Ordering::SeqCst);
        let mut old = 0;
        if unsafe {
            VirtualProtect(
                (self.slot as *const AtomicUsize).cast(),
                size_of::<usize>(),
                self.protection,
                &mut old,
            )
        } == 0
        {
            // Never continue with a writable import page after a failed restore.
            std::process::abort();
        }
    }
}

fn install() -> Result<Hook> {
    anyhow::ensure!(size_of::<usize>() == 8, "hook architecture");
    // socket() has already loaded the provider. Do not load or patch any other
    // process/module. Use the PE64 IAT directory, bounded by SizeOfImage.
    unsafe {
        let module = GetModuleHandleW(wide("mswsock.dll").as_ptr());
        anyhow::ensure!(!module.is_null(), "hook provider unavailable");
        let mut info: MODULEINFO = zeroed();
        anyhow::ensure!(
            K32GetModuleInformation(
                GetCurrentProcess(),
                module,
                &mut info,
                size_of::<MODULEINFO>() as u32
            ) != 0,
            "hook module info"
        );
        let base = module.cast::<u8>();
        let size = info.SizeOfImage as usize;
        let read32 = |offset: usize| -> Result<u32> {
            anyhow::ensure!(
                offset.checked_add(4).is_some_and(|end| end <= size),
                "hook header bounds"
            );
            Ok(std::ptr::read_unaligned(base.add(offset).cast::<u32>()))
        };
        anyhow::ensure!(read32(0)? & 0xffff == 0x5a4d, "hook DOS signature");
        let pe = read32(0x3c)? as usize;
        anyhow::ensure!(
            pe <= 4096 && read32(pe)? == 0x4550 && read32(pe + 24)? & 0xffff == 0x20b,
            "hook PE64 header"
        );
        anyhow::ensure!(read32(pe + 24 + 108)? > 12, "hook IAT directory");
        let directory = pe + 24 + 112 + 12 * 8;
        let rva = read32(directory)? as usize;
        let length = read32(directory + 4)? as usize;
        anyhow::ensure!(
            rva > 0
                && rva.is_multiple_of(8)
                && length.is_multiple_of(8)
                && rva.checked_add(length).is_some_and(|end| end <= size),
            "hook IAT bounds"
        );
        let ntdll = GetModuleHandleW(wide("ntdll.dll").as_ptr());
        let original = GetProcAddress(ntdll, c"NtDeviceIoControlFile".as_ptr().cast())
            .context("hook export unavailable")? as usize;
        let mut found = None;
        for offset in (rva..rva + length).step_by(8) {
            let slot = &*base.add(offset).cast::<AtomicUsize>();
            if slot.load(Ordering::Acquire) == original {
                anyhow::ensure!(found.is_none(), "hook ambiguous imports");
                found = Some(slot);
            }
        }
        let slot = found.context("hook import unavailable")?;
        let mut protection = 0;
        anyhow::ensure!(
            VirtualProtect(
                (slot as *const AtomicUsize).cast(),
                8,
                PAGE_READWRITE,
                &mut protection
            ) != 0,
            "hook protection"
        );
        let hook = Hook {
            slot,
            original,
            protection,
        };
        ORIGINAL.store(original, Ordering::Release);
        anyhow::ensure!(
            slot.compare_exchange(
                original,
                observe as *const () as usize,
                Ordering::SeqCst,
                Ordering::SeqCst
            )
            .is_ok(),
            "hook import changed"
        );
        Ok(hook)
    }
}

fn offset(call: &Call, path: &[u8]) -> Option<usize> {
    if call.input_len > 256 {
        return None;
    }
    let input = &call.bytes[..call.input_len as usize];
    // Only identify a family if the complete path and terminator match too.
    (0..input.len()).find(|&at| {
        input.get(at..at + 2) == Some(&[1, 0])
            && input.get(at + 2..at + 2 + path.len()) == Some(path)
            && input.get(at + 2 + path.len()) == Some(&0)
    })
}

fn safe_prefix(call: &Call, at: usize) -> bool {
    // Fail closed on pointer-like/unknown leading words. Never publish them or
    // replay a packet that might contain a buffer pointer from the first bind.
    at <= 16
        && call.bytes[..at]
            .chunks_exact(8)
            .all(|word| u64::from_le_bytes(word.try_into().unwrap()) <= u16::MAX as u64)
        && call.bytes[..at]
            .chunks_exact(4)
            .all(|word| u32::from_le_bytes(word.try_into().unwrap()) <= u16::MAX as u32)
}

pub(super) fn error(error: anyhow::Error) -> Value {
    let label = match error.to_string().as_str() {
        "hook architecture" => "architecture",
        "hook provider unavailable" => "provider_unavailable",
        "hook module info" => "module_info",
        "hook DOS signature" | "hook header bounds" | "hook PE64 header" => "pe_header",
        "hook IAT directory" | "hook IAT bounds" => "iat_bounds",
        "hook export unavailable" => "export_unavailable",
        "hook ambiguous imports" => "ambiguous_imports",
        "hook import unavailable" => "import_unavailable",
        "hook protection" => "protection",
        "hook import changed" => "import_changed",
        "observation socket" => "socket_creation",
        _ => "other",
    };
    json!({"diagnostic_only":true,"error":label,"replica":{"attempted":false}})
}

pub(super) fn run(scratch: &std::path::Path) -> Result<Value> {
    let path = scratch.join("w-ob1.sock");
    let path = path
        .to_str()
        .context("observation path encoding")?
        .as_bytes();
    let (address, length) = unix_address(path, false)?;
    let socket = unsafe { windows_sys::Win32::Networking::WinSock::socket(1, SOCK_STREAM, 0) };
    anyhow::ensure!(socket != INVALID_SOCKET, "observation socket");
    // RAII even if installation or packet validation fails.
    struct Socket(usize);
    impl Drop for Socket {
        fn drop(&mut self) {
            unsafe { closesocket(self.0) };
        }
    }
    let socket = Socket(socket);
    let hook = install()?;
    CAPTURE.with(|cell| {
        cell.set(Some(Capture {
            calls: [EMPTY; 16],
            count: 0,
            overflow: false,
        }))
    });
    let rc = unsafe {
        bind(
            socket.0,
            (&address as *const UnixAddress).cast(),
            length as i32,
        )
    };
    let error = unsafe { WSAGetLastError() };
    let captured = CAPTURE.with(|cell| cell.take()).unwrap();
    drop(hook); // Restore immediately, before reporting, closing, or direct I/O.
    let winsock = row(
        "bind(AF_UNIX; observed)",
        rc as i64,
        rc == 0,
        if rc == 0 { 0 } else { error as u32 },
        "winsock",
    );
    let calls: Vec<Value> = captured.calls[..captured.count].iter().map(|call| {
        let at = offset(call, path);
        let leading = at.filter(|&at| safe_prefix(call, at))
            .map(|at| call.bytes[..at].to_vec());
        json!({"ioctl_code":call.code,"input_bytes":call.input_len,"output_bytes":call.output_len,
            "sockaddr_offset":at,"path_matches":at.is_some(),"leading_bytes":leading,
            "leading_fields_redacted":at.is_some_and(|at| !safe_prefix(call, at))})
    }).collect();
    let binds: Vec<_> = captured.calls[..captured.count]
        .iter()
        .filter(|c| c.code == 0x12003)
        .collect();
    let mut replica =
        json!({"attempted":false,"reason":"one bounded pointer-free bind packet required"});
    if rc == 0 && !captured.overflow && binds.len() == 1 {
        let call = binds[0];
        if let Some(at) = offset(call, path) {
            let end = at + 2 + path.len() + 1;
            if safe_prefix(call, at)
                && call.output_len <= 256
                && call.bytes[end..call.input_len as usize]
                    .iter()
                    .all(|b| *b == 0)
            {
                let target = scratch.join("w-ob2.sock");
                let target = target.to_str().context("replica path encoding")?.as_bytes();
                anyhow::ensure!(target.len() == path.len(), "replica path length");
                #[repr(align(8))]
                struct Packet([u8; 256]);
                let mut packet = Packet(call.bytes);
                packet.0[at + 2..at + 2 + target.len()].copy_from_slice(target);
                let attempt = (|| -> Result<Value> {
                    let (creation, handle) = afd_open(&json!({"family":1,"transport":"stream"}))?;
                    let operation = if let Some(handle) = handle {
                        let mut operation = afd_ioctl(
                            handle.0,
                            call.code,
                            &packet,
                            call.input_len as usize,
                            call.output_len as usize,
                        )?;
                        operation["address_offset"] = json!(at);
                        operation["address_bytes"] = json!(call.input_len as usize - at);
                        operation
                    } else {
                        json!({"attempted":false,"reason":"AFD creation failed"})
                    };
                    Ok(
                        json!({"attempted":true,"creation":creation,"operation":operation,
                    "route":"direct AFD observed Winsock bind layout","layout_copied":true}),
                    )
                })();
                replica = attempt.unwrap_or_else(unix_comparison_error);
            }
        }
    }
    Ok(
        json!({"diagnostic_only":true,"hook":"mswsock PE64 IAT; current thread; restored",
        "winsock":winsock,"calls":calls,"capture_overflow":captured.overflow,"replica":replica}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_packet_bounds_and_redaction() {
        let path = b"fixture.sock";
        for at in [0, 4, 6, 8, 16] {
            let mut call = Call {
                input_len: (at + 2 + path.len() + 1) as u32,
                ..EMPTY
            };
            call.bytes[at..at + 2].copy_from_slice(&[1, 0]);
            call.bytes[at + 2..at + 2 + path.len()].copy_from_slice(path);
            assert_eq!(offset(&call, path), Some(at));
            assert!(safe_prefix(&call, at));
            assert_eq!(offset(&call, b"different"), None);
            call.input_len -= 1;
            assert_eq!(offset(&call, path), None); // NUL must be in bounds
        }
        let mut call = EMPTY;
        call.bytes[..8].copy_from_slice(&0x0000_0001_0000_0000u64.to_le_bytes());
        assert!(!safe_prefix(&call, 8)); // Even small DWORDs can be a pointer.
        call.input_len = 257;
        assert_eq!(offset(&call, path), None);
    }
}
