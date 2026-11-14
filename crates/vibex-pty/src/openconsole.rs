//! Launching a newer console host (`OpenConsole.exe`, from the open-source
//! microsoft/terminal project) instead of the one built into Windows.
//!
//! The inbox console host re-renders everything the program writes and
//! drops what it doesn't understand, including sixel (DCS) and kitty
//! graphics (APC) image data. OpenConsole 1.22+ passes such sequences
//! through. Microsoft's `conpty.dll` is only a small launcher for it; this
//! module is that launcher:
//!
//! 1. Open a console server handle on `\Device\ConDrv\Server`.
//! 2. Start `OpenConsole.exe --headless --width W --height H --signal S
//!    --server H` with our pipe ends as its stdin/stdout, inheriting only
//!    those four handles.
//! 3. Open a `\Reference` handle on the server. The {signal, reference,
//!    process} triple has the same layout as the inbox HPCON, so
//!    `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` accepts a pointer to it.
//! 4. Resizes are 6-byte packets on the signal pipe; closing the signal
//!    pipe tells the host to shut down.

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, SetHandleInformation, DUPLICATE_SAME_ACCESS, GENERIC_ALL, GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT,
    UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{WriteFile, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
const OBJ_INHERIT: u32 = 0x02;
const OBJ_CASE_INSENSITIVE: u32 = 0x40;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
const PTY_SIGNAL_RESIZE_WINDOW: u16 = 8;

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *const UNICODE_STRING,
    attributes: u32,
    security_descriptor: *const c_void,
    security_quality_of_service: *const c_void,
}

#[repr(C)]
struct IoStatusBlock {
    status: isize,
    information: usize,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtOpenFile(file: *mut HANDLE, access: u32, attrs: *const ObjectAttributes, io: *mut IoStatusBlock, share: u32, options: u32) -> i32;
}

fn nt_open(name: &str, access: u32, parent: HANDLE, inherit: bool, options: u32) -> io::Result<HANDLE> {
    let wide: Vec<u16> = name.encode_utf16().collect();
    let us = UNICODE_STRING { Length: (wide.len() * 2) as u16, MaximumLength: (wide.len() * 2) as u16, Buffer: wide.as_ptr() as *mut u16 };
    let attrs = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: parent,
        object_name: &us,
        attributes: OBJ_CASE_INSENSITIVE | if inherit { OBJ_INHERIT } else { 0 },
        security_descriptor: null(),
        security_quality_of_service: null(),
    };
    let mut io_status: IoStatusBlock = unsafe { zeroed() };
    let mut h: HANDLE = null_mut();
    let status = unsafe { NtOpenFile(&mut h, access, &attrs, &mut io_status, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, options) };
    if status < 0 {
        return Err(io::Error::other(format!("NtOpenFile({name}) failed: 0x{:08x}", status as u32)));
    }
    Ok(h)
}

/// Same layout as the inbox pseudoconsole object.
#[repr(C)]
pub(crate) struct OpenPc {
    pub signal: HANDLE,
    pub reference: HANDLE,
    pub process: HANDLE,
}

unsafe impl Send for OpenPc {}
unsafe impl Sync for OpenPc {}

impl OpenPc {
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let packet: [u16; 3] = [PTY_SIGNAL_RESIZE_WINDOW, cols, rows];
        let mut n = 0u32;
        let ok = unsafe { WriteFile(self.signal, packet.as_ptr() as *const u8, 6, &mut n, null_mut()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for OpenPc {
    fn drop(&mut self) {
        unsafe {
            // closing the signal pipe makes the host flush and exit
            CloseHandle(self.signal);
            CloseHandle(self.reference);
            CloseHandle(self.process);
        }
    }
}

/// Where to find OpenConsole.exe: `VIBEX_OPENCONSOLE`, else next to the
/// running executable. `VIBEX_CONPTY=inbox` forces the built-in host.
pub(crate) fn find() -> Option<PathBuf> {
    if std::env::var("VIBEX_CONPTY").is_ok_and(|v| v.eq_ignore_ascii_case("inbox")) {
        return None;
    }
    if let Ok(p) = std::env::var("VIBEX_OPENCONSOLE") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join("OpenConsole.exe");
    p.is_file().then_some(p)
}

fn inheritable(h: HANDLE) -> io::Result<HANDLE> {
    let mut out: HANDLE = null_mut();
    let me = unsafe { GetCurrentProcess() };
    if unsafe { DuplicateHandle(me, h, me, &mut out, 0, 1, DUPLICATE_SAME_ACCESS) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(out)
}

fn hex(h: HANDLE) -> String {
    format!("0x{:x}", h as usize)
}

/// Start OpenConsole on the given pipe ends (it reads `input`, writes
/// `output`).
pub(crate) fn create(host: &Path, cols: u16, rows: u16, input: HANDLE, output: HANDLE) -> io::Result<Box<OpenPc>> {
    let server = nt_open("\\Device\\ConDrv\\Server", GENERIC_ALL, null_mut(), true, 0)?;
    let close = |h: HANDLE| unsafe {
        CloseHandle(h);
    };

    let (mut sig_host, mut sig_ours): (HANDLE, HANDLE) = (null_mut(), null_mut());
    if unsafe { CreatePipe(&mut sig_host, &mut sig_ours, null(), 0) } == 0 {
        let e = io::Error::last_os_error();
        close(server);
        return Err(e);
    }
    unsafe { SetHandleInformation(sig_host, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
    let (in_h, out_h) = match (inheritable(input), inheritable(output)) {
        (Ok(i), Ok(o)) => (i, o),
        (a, b) => {
            for h in [a.ok(), b.ok(), Some(server), Some(sig_host), Some(sig_ours)].into_iter().flatten() {
                close(h);
            }
            return Err(io::Error::last_os_error());
        }
    };

    let cmd = format!("\"{}\" --headless --width {cols} --height {rows} --signal {} --server {}", host.display(), hex(sig_host), hex(server));
    let mut cmdline: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();

    let handles = [server, in_h, out_h, sig_host];
    let mut attr_size = 0usize;
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut attr_size) };
    let mut attr_buf = vec![0u8; attr_size];
    let attrs = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    let mut pi: PROCESS_INFORMATION = unsafe { zeroed() };
    let ok = unsafe {
        InitializeProcThreadAttributeList(attrs, 1, 0, &mut attr_size) != 0
            && UpdateProcThreadAttribute(
                attrs,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                handles.as_ptr() as *const c_void,
                size_of::<[HANDLE; 4]>(),
                null_mut(),
                null(),
            ) != 0
            && {
                let mut si: STARTUPINFOEXW = zeroed();
                si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
                si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
                si.StartupInfo.hStdInput = in_h;
                si.StartupInfo.hStdOutput = out_h;
                si.StartupInfo.hStdError = out_h;
                si.lpAttributeList = attrs;
                CreateProcessW(
                    null(),
                    cmdline.as_mut_ptr(),
                    null(),
                    null(),
                    1,
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                    null(),
                    null(),
                    &si.StartupInfo,
                    &mut pi,
                ) != 0
            }
    };
    let err = (!ok).then(io::Error::last_os_error);
    unsafe { DeleteProcThreadAttributeList(attrs) };
    // the host has its own copies now
    close(in_h);
    close(out_h);
    close(sig_host);
    if let Some(e) = err {
        close(server);
        close(sig_ours);
        return Err(io::Error::new(e.kind(), format!("failed to start {}: {e}", host.display())));
    }
    close(pi.hThread);

    let reference = nt_open("\\Reference", GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE, server, false, FILE_SYNCHRONOUS_IO_NONALERT);
    close(server);
    match reference {
        Ok(reference) => Ok(Box::new(OpenPc { signal: sig_ours, reference, process: pi.hProcess })),
        Err(e) => {
            // the host started but can't be used: don't leave it running
            unsafe { TerminateProcess(pi.hProcess, 1) };
            close(sig_ours);
            close(pi.hProcess);
            Err(e)
        }
    }
}
