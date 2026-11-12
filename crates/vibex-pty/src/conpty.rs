//! Windows ConPTY backend.
//!
//! 1. Two anonymous pipes: we write the input pipe, we read the output pipe.
//! 2. `CreatePseudoConsole` takes the far ends of both pipes.
//! 3. The child is created with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`.
//! 4. A reader thread drains output continuously (ConPTY blocks if the
//!    output pipe fills, which would also stall the child).
//! 5. A waiter thread reports when the child exits.
//!
//! When `OpenConsole.exe` sits next to the executable it is used instead of
//! the built-in console host (see `openconsole`), because only the newer
//! host passes image sequences through.

use crate::openconsole::{self, OpenPc};
use crate::{send, Command, PtyEvent, Result, Waker};
use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, S_OK, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Console::{ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

/// An owned Win32 handle, closed on drop.
struct OwnedHandle(HANDLE);
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

enum PseudoConsole {
    Inbox(HPCON),
    Open(Box<OpenPc>),
}
unsafe impl Send for PseudoConsole {}
unsafe impl Sync for PseudoConsole {}

impl PseudoConsole {
    /// The value `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` takes.
    fn raw(&self) -> HPCON {
        match self {
            PseudoConsole::Inbox(h) => *h,
            PseudoConsole::Open(b) => &**b as *const OpenPc as HPCON,
        }
    }

    fn close(self) {
        match self {
            PseudoConsole::Inbox(h) => unsafe { ClosePseudoConsole(h) },
            PseudoConsole::Open(b) => drop(b),
        }
    }

    fn create(cols: u16, rows: u16, input: HANDLE, output: HANDLE) -> Result<Self> {
        if let Some(host) = openconsole::find() {
            match openconsole::create(&host, cols, rows, input, output) {
                Ok(pc) => return Ok(PseudoConsole::Open(pc)),
                // fall back to the built-in host rather than fail
                Err(e) => eprintln!("vibex: {}: {e}; using the built-in console host", host.display()),
            }
        }
        let mut hpc: HPCON = 0;
        let size = COORD { X: cols as i16, Y: rows as i16 };
        let hr = unsafe { CreatePseudoConsole(size, input, output, 0, &mut hpc) };
        if hr != S_OK {
            return Err(io::Error::other(format!("CreatePseudoConsole failed: 0x{hr:08x}")));
        }
        Ok(PseudoConsole::Inbox(hpc))
    }

    /// Which console host this is (for `vibex --version`-style reports).
    fn kind(&self) -> &'static str {
        match self {
            PseudoConsole::Inbox(_) => "conhost",
            PseudoConsole::Open(_) => "openconsole",
        }
    }
}

pub struct Pty {
    hpc: Option<PseudoConsole>,
    input: OwnedHandle,
    process: Arc<OwnedHandle>,
    _thread: OwnedHandle,
}

fn last_err() -> io::Error {
    io::Error::last_os_error()
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Quote one argument per the MSVC command-line rules.
fn quote_arg(arg: &str, out: &mut String) {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
}

fn command_line(cmd: &Command) -> String {
    let mut s = String::new();
    quote_arg(&cmd.program, &mut s);
    for a in &cmd.args {
        s.push(' ');
        quote_arg(a, &mut s);
    }
    s
}

/// Inherited environment plus overrides, as a UTF-16 environment block.
fn env_block(extra: &[(String, String)]) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    for (k, v) in extra {
        vars.retain(|(ek, _)| !ek.eq_ignore_ascii_case(k));
        vars.push((k.clone(), v.clone()));
    }
    vars.sort_by_key(|a| a.0.to_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(std::ffi::OsStr::new(&format!("{k}={v}")).encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn pipe() -> Result<(OwnedHandle, OwnedHandle)> {
    let mut read: HANDLE = null_mut();
    let mut write: HANDLE = null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, null(), 0) } == 0 {
        return Err(last_err());
    }
    Ok((OwnedHandle(read), OwnedHandle(write)))
}

impl Pty {
    /// Spawn `cmd` attached to a new pseudoconsole of `cols` x `rows`.
    pub fn spawn(cmd: &Command, cols: u16, rows: u16, events: Sender<PtyEvent>, waker: impl Waker) -> Result<Pty> {
        let (in_read, in_write) = pipe()?;
        let (out_read, out_write) = pipe()?;

        let pc = PseudoConsole::create(cols, rows, in_read.0, out_write.0)?;
        // the console host owns duplicates of these now
        drop(in_read);
        drop(out_write);

        // attribute list carrying the pseudoconsole
        let mut attr_size: usize = 0;
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut attr_size) };
        let mut attr_buf = vec![0u8; attr_size];
        let attrs = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if unsafe { InitializeProcThreadAttributeList(attrs, 1, 0, &mut attr_size) } == 0 {
            return Err(last_err());
        }
        let ok = unsafe {
            UpdateProcThreadAttribute(attrs, 0, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, pc.raw() as *const c_void, size_of::<HPCON>(), null_mut(), null())
        };
        if ok == 0 {
            let e = last_err();
            unsafe { DeleteProcThreadAttributeList(attrs) };
            return Err(e);
        }

        let mut si: STARTUPINFOEXW = unsafe { zeroed() };
        si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        // don't let the child inherit our own std handles (if any)
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.lpAttributeList = attrs;

        let mut cmdline = wide(&command_line(cmd));
        let mut env = env_block(&cmd.env);
        let cwd = cmd.cwd.as_deref().map(wide);
        let mut pi: PROCESS_INFORMATION = unsafe { zeroed() };

        let ok = unsafe {
            CreateProcessW(
                null(),
                cmdline.as_mut_ptr(),
                null(),
                null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                env.as_mut_ptr() as *const c_void,
                cwd.as_ref().map_or(null(), |w| w.as_ptr()),
                &si.StartupInfo,
                &mut pi,
            )
        };
        let spawn_err = if ok == 0 { Some(last_err()) } else { None };
        unsafe { DeleteProcThreadAttributeList(attrs) };
        if let Some(e) = spawn_err {
            pc.close();
            return Err(io::Error::new(e.kind(), format!("failed to start {}: {e}", cmd.program)));
        }

        let process = Arc::new(OwnedHandle(pi.hProcess));
        let thread_handle = OwnedHandle(pi.hThread);
        let waker: Arc<dyn Waker + Sync> = Arc::new(SyncWaker(std::sync::Mutex::new(Box::new(waker))));

        // reader thread
        {
            let tx = events.clone();
            let waker = waker.clone();
            thread::Builder::new().name("vibex-pty-reader".into()).spawn(move || {
                let out = out_read;
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let mut n: u32 = 0;
                    let ok = unsafe { ReadFile(out.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut()) };
                    if ok == 0 || n == 0 {
                        break;
                    }
                    if !send(&tx, PtyEvent::Output(buf[..n as usize].to_vec()), &*waker) {
                        break;
                    }
                }
            })?;
        }

        // exit waiter thread
        {
            let tx = events;
            let process = process.clone();
            thread::Builder::new().name("vibex-pty-wait".into()).spawn(move || {
                let r = unsafe { WaitForSingleObject(process.0, INFINITE) };
                let mut code: u32 = 0;
                if r == WAIT_OBJECT_0 {
                    unsafe { GetExitCodeProcess(process.0, &mut code) };
                }
                send(&tx, PtyEvent::Exited(code), &*waker);
            })?;
        }

        Ok(Pty { hpc: Some(pc), input: in_write, process, _thread: thread_handle })
    }

    /// Send bytes to the child (keyboard input, replies).
    pub fn write(&self, mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            let mut n: u32 = 0;
            let ok = unsafe { WriteFile(self.input.0, data.as_ptr(), data.len() as u32, &mut n, null_mut()) };
            if ok == 0 {
                return Err(last_err());
            }
            data = &data[n as usize..];
        }
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        match &self.hpc {
            Some(PseudoConsole::Inbox(h)) => {
                let hr = unsafe { ResizePseudoConsole(*h, COORD { X: cols as i16, Y: rows as i16 }) };
                if hr != S_OK {
                    return Err(io::Error::other(format!("ResizePseudoConsole failed: 0x{hr:08x}")));
                }
            }
            Some(PseudoConsole::Open(pc)) => pc.resize(cols, rows)?,
            None => {}
        }
        Ok(())
    }

    /// "conhost" (built into Windows) or "openconsole".
    pub fn host(&self) -> &'static str {
        self.hpc.as_ref().map_or("none", |p| p.kind())
    }

    pub fn kill(&self) {
        unsafe { TerminateProcess(self.process.0, 1) };
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Some(pc) = self.hpc.take() {
            // ClosePseudoConsole can block until the output pipe is drained;
            // never do that on the UI thread.
            thread::spawn(move || pc.close());
        }
    }
}

struct SyncWaker(std::sync::Mutex<Box<dyn Waker>>);

impl Waker for SyncWaker {
    fn wake(&self) {
        if let Ok(w) = self.0.lock() {
            w.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn quoting() {
        let mut s = String::new();
        quote_arg(r#"a "b" c\"#, &mut s);
        assert_eq!(s, r#""a \"b\" c\\""#);
        let mut s = String::new();
        quote_arg("plain", &mut s);
        assert_eq!(s, "plain");
        let mut s = String::new();
        quote_arg("", &mut s);
        assert_eq!(s, r#""""#);
    }

    #[test]
    fn runs_cmd_and_reads_output() {
        let (tx, rx) = mpsc::channel();
        let cmd = Command::new("cmd.exe").arg("/c").arg("echo vibex-pty-ok");
        let _pty = Pty::spawn(&cmd, 80, 24, tx, || {}).expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut out = Vec::new();
        let mut exited = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(PtyEvent::Output(b)) => out.extend(b),
                Ok(PtyEvent::Exited(_)) => exited = true,
                Err(_) => {}
            }
            if exited && String::from_utf8_lossy(&out).contains("vibex-pty-ok") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("vibex-pty-ok"), "output was: {text:?}");
        assert!(exited);
    }

    /// Runs on whichever host `VIBEX_OPENCONSOLE` / `VIBEX_CONPTY` select.
    #[test]
    fn resize_reaches_child() {
        let (tx, rx) = mpsc::channel();
        let cmd = Command::new("powershell.exe")
            .arg("-NoLogo")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("Start-Sleep -Milliseconds 1500; $s = $Host.UI.RawUI.WindowSize; \"size=$($s.Width)x$($s.Height)\"");
        let pty = Pty::spawn(&cmd, 80, 24, tx, || {}).expect("spawn");
        std::thread::sleep(Duration::from_millis(300));
        pty.resize(101, 33).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut out = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(PtyEvent::Output(b)) => out.extend(b),
                Ok(PtyEvent::Exited(_)) => break,
                Err(_) => {}
            }
        }
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("size=101x33"), "host {}: {text:?}", pty.host());
    }
}
