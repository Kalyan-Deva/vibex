//! One process, many windows, and remote control.
//!
//! The first Vibex owns a named pipe. Later launches (Start menu, Win+R,
//! Explorer's "Open in Vibex", `vibex -- cmd`) connect to it, hand over
//! what they were asked to open and exit; the first process opens a new
//! window. `--new-process` (or `--config`) skips the handoff. `vibex cli
//! ...` uses the same pipe to control the running Vibex (when
//! `remote_control` is on) and prints the reply.
//!
//! The pipe is local-only (`PIPE_REJECT_REMOTE_CLIENTS`), named after the
//! user, and created with `FILE_FLAG_FIRST_PIPE_INSTANCE` so nobody can
//! squat on the name first. Its default security lets other accounts open
//! it for reading only, so only this user can send it requests.
//!
//! Wire format: the client writes `<length>\n<payload>`; the server replies
//! with text and hangs up.

// The handoff is Windows-only for now; elsewhere each launch is its own
// process (the request type still compiles for tests).
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::PathBuf;

/// What a new window should open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenRequest {
    pub profile: Option<String>,
    pub cwd: Option<String>,
    pub exec: Option<Vec<String>>,
    pub config: Option<PathBuf>,
    /// Open this layout file (`layouts/<name>.lua`) as a new workspace.
    pub layout: Option<String>,
    /// Bring back a detached workspace ("" = the most recent).
    pub attach: Option<String>,
}

impl OpenRequest {
    /// Nothing specific asked for (a plain `vibex` launch).
    pub fn is_plain(&self) -> bool {
        self.profile.is_none() && self.cwd.is_none() && self.exec.is_none() && self.layout.is_none() && self.attach.is_none()
    }
}

/// Anything sent over the pipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Open(OpenRequest),
    /// `vibex cli <args...>`.
    Cli(Vec<String>),
}

const REC: char = '\u{1e}';
const UNIT: char = '\u{1f}';
const MAX_REQUEST: usize = 1024 * 1024;

impl OpenRequest {
    pub fn encode(&self) -> String {
        let mut s = String::from("vibex-open-1");
        let clean = |v: &str| v.replace([REC, UNIT], "");
        if let Some(p) = &self.profile {
            s.push(REC);
            s.push_str(&format!("profile={}", clean(p)));
        }
        if let Some(c) = &self.cwd {
            s.push(REC);
            s.push_str(&format!("cwd={}", clean(c)));
        }
        if let Some(c) = &self.config {
            s.push(REC);
            s.push_str(&format!("config={}", clean(&c.to_string_lossy())));
        }
        if let Some(l) = &self.layout {
            s.push(REC);
            s.push_str(&format!("layout={}", clean(l)));
        }
        if let Some(a) = &self.attach {
            s.push(REC);
            s.push_str(&format!("attach={}", clean(a)));
        }
        if let Some(argv) = &self.exec {
            s.push(REC);
            s.push_str("exec=");
            s.push_str(&argv.iter().map(|a| clean(a)).collect::<Vec<_>>().join(&UNIT.to_string()));
        }
        s
    }

    pub fn decode(s: &str) -> Option<Self> {
        let mut parts = s.split(REC);
        if parts.next()? != "vibex-open-1" {
            return None;
        }
        let mut r = OpenRequest::default();
        for p in parts {
            let (k, v) = p.split_once('=')?;
            match k {
                "profile" => r.profile = Some(v.to_string()),
                "cwd" => r.cwd = Some(v.to_string()),
                "config" => r.config = Some(PathBuf::from(v)),
                "layout" => r.layout = Some(v.to_string()),
                "attach" => r.attach = Some(v.to_string()),
                "exec" => r.exec = Some(v.split(UNIT).map(str::to_string).collect()),
                _ => {}
            }
        }
        Some(r)
    }
}

impl Request {
    pub fn encode(&self) -> String {
        match self {
            Request::Open(r) => r.encode(),
            Request::Cli(args) => {
                let clean = |v: &str| v.replace([REC, UNIT], "");
                format!("vibex-cli-1{REC}{}", args.iter().map(|a| clean(a)).collect::<Vec<_>>().join(&UNIT.to_string()))
            }
        }
    }

    pub fn decode(s: &str) -> Option<Self> {
        if let Some(rest) = s.strip_prefix("vibex-cli-1") {
            let rest = rest.strip_prefix(REC).unwrap_or(rest);
            let args = if rest.is_empty() { Vec::new() } else { rest.split(UNIT).map(str::to_string).collect() };
            return Some(Request::Cli(args));
        }
        OpenRequest::decode(s).map(Request::Open)
    }
}

#[cfg(windows)]
mod imp {
    use super::{OpenRequest, Request, MAX_REQUEST};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

    pub(super) fn name(tag: &str) -> Vec<u16> {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        let domain = std::env::var("USERDOMAIN").unwrap_or_default();
        let safe: String = format!("{domain}-{user}{tag}").chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
        format!(r"\\.\pipe\vibex-{safe}").encode_utf16().chain(Some(0)).collect()
    }

    fn write_all(h: HANDLE, mut data: &[u8]) -> bool {
        while !data.is_empty() {
            let mut n = 0u32;
            if unsafe { WriteFile(h, data.as_ptr(), data.len().min(64 * 1024) as u32, &mut n, null_mut()) } == 0 || n == 0 {
                return false;
            }
            data = &data[n as usize..];
        }
        true
    }

    /// Read until the other side hangs up (or `limit`).
    fn read_to_end(h: HANDLE, limit: usize) -> Vec<u8> {
        let mut data = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let mut n = 0u32;
            let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut()) } != 0;
            if !ok || n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n as usize]);
            if data.len() > limit {
                break;
            }
        }
        data
    }

    /// Read one framed request: "<len>\n<payload>".
    fn read_request(h: HANDLE) -> Option<String> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let mut n = 0u32;
            if unsafe { ReadFile(h, byte.as_mut_ptr(), 1, &mut n, null_mut()) } == 0 || n == 0 {
                return None;
            }
            if byte[0] == b'\n' {
                break;
            }
            head.push(byte[0]);
            if head.len() > 12 {
                return None;
            }
        }
        let len: usize = std::str::from_utf8(&head).ok()?.parse().ok()?;
        if len > MAX_REQUEST {
            return None;
        }
        let mut data = vec![0u8; len];
        let mut got = 0;
        while got < len {
            let mut n = 0u32;
            let ok = unsafe { ReadFile(h, data[got..].as_mut_ptr(), (len - got) as u32, &mut n, null_mut()) } != 0;
            if !ok || n == 0 {
                return None;
            }
            got += n as usize;
        }
        String::from_utf8(data).ok()
    }

    /// Send a request to the running Vibex and wait for its reply. None if
    /// no Vibex is running.
    pub fn send(req: &Request) -> Option<String> {
        send_to(&name(&instance()), req)
    }

    /// VIBEX_INSTANCE: run a separate Vibex with its own pipe (tests).
    fn instance() -> String {
        std::env::var("VIBEX_INSTANCE").map(|i| format!("-{i}")).unwrap_or_default()
    }

    pub(super) fn send_to(name: &[u16], req: &Request) -> Option<String> {
        for attempt in 0..2 {
            let h = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, null(), OPEN_EXISTING, 0, null_mut()) };
            if h == INVALID_HANDLE_VALUE {
                if attempt == 0 && unsafe { GetLastError() } == ERROR_PIPE_BUSY {
                    unsafe { WaitNamedPipeW(name.as_ptr(), 2000) };
                    continue;
                }
                return None;
            }
            // let the running Vibex bring its new window to the front
            let mut pid = 0u32;
            if unsafe { GetNamedPipeServerProcessId(h, &mut pid) } != 0 {
                unsafe { AllowSetForegroundWindow(pid) };
            }
            let payload = req.encode();
            let framed = format!("{}\n{payload}", payload.len());
            let reply = if write_all(h, framed.as_bytes()) {
                unsafe { FlushFileBuffers(h) };
                Some(String::from_utf8_lossy(&read_to_end(h, 64 * 1024 * 1024)).into_owned())
            } else {
                None
            };
            unsafe { CloseHandle(h) };
            return reply;
        }
        None
    }

    /// Hand a window request to a running Vibex. True if it took it.
    pub fn forward(req: &OpenRequest) -> bool {
        send(&Request::Open(req.clone())).is_some()
    }

    fn create(name: &[u16], first: bool) -> HANDLE {
        let flags = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
        unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                flags,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                null(),
            )
        }
    }

    /// Become the instance others talk to; `handle` turns each request
    /// into the reply (it may block a little, e.g. for remote control).
    /// False if another process already is.
    pub fn serve(handle: impl Fn(Request) -> String + Send + 'static) -> bool {
        serve_on(name(&instance()), handle)
    }

    pub(super) fn serve_on(name: Vec<u16>, handle: impl Fn(Request) -> String + Send + 'static) -> bool {
        let first = create(&name, true);
        if first == INVALID_HANDLE_VALUE {
            return false;
        }
        let first = first as usize;
        // requests are still answered one at a time, but each client is
        // read on its own thread: one that connects and sends nothing
        // can't hold up every later launch
        let handle = std::sync::Arc::new(std::sync::Mutex::new(handle));
        std::thread::Builder::new()
            .name("vibex-single-instance".into())
            .spawn(move || {
                let mut h = first as HANDLE;
                loop {
                    let connected =
                        unsafe { ConnectNamedPipe(h, null_mut()) } != 0 || matches!(unsafe { GetLastError() }, ERROR_PIPE_CONNECTED | ERROR_NO_DATA);
                    // keep an instance listening while this one is handled,
                    // so the name never disappears between clients
                    let next = loop {
                        let n = create(&name, false);
                        if n != INVALID_HANDLE_VALUE {
                            break n;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    };
                    let (client, handle) = (h as usize, handle.clone());
                    let served = std::thread::Builder::new().name("vibex-single-client".into()).spawn(move || {
                        let h = client as HANDLE;
                        if connected {
                            let reply = match read_request(h).as_deref().and_then(Request::decode) {
                                Some(req) => (handle.lock().unwrap_or_else(|e| e.into_inner()))(req),
                                None => "error: bad request".into(),
                            };
                            write_all(h, reply.as_bytes());
                            unsafe { FlushFileBuffers(h) };
                        }
                        unsafe {
                            DisconnectNamedPipe(h);
                            CloseHandle(h);
                        }
                    });
                    if served.is_err() {
                        // no thread for it: hang up on this client
                        unsafe {
                            DisconnectNamedPipe(h);
                            CloseHandle(h);
                        }
                    }
                    h = next;
                }
            })
            .is_ok()
    }
}

#[cfg(windows)]
pub use imp::{forward, send, serve};

#[cfg(not(windows))]
pub fn forward(_req: &OpenRequest) -> bool {
    false
}

#[cfg(not(windows))]
pub fn send(_req: &Request) -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn serve(_handle: impl Fn(Request) -> String + Send + 'static) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let r = OpenRequest {
            profile: Some("wsl".into()),
            cwd: Some(r"C:\Users\x\my code".into()),
            exec: Some(vec!["git".into(), "log".into(), "--oneline".into()]),
            config: None,
            layout: Some("work".into()),
            attach: Some(String::new()),
        };
        assert_eq!(OpenRequest::decode(&r.encode()), Some(r.clone()));
        assert_eq!(OpenRequest::decode(&OpenRequest::default().encode()), Some(OpenRequest::default()));
        assert!(OpenRequest::decode("something else").is_none());
        let cli = Request::Cli(vec!["send-text".into(), "--pane".into(), "3".into(), "line one\nline two".into()]);
        assert_eq!(Request::decode(&cli.encode()), Some(cli));
        assert_eq!(Request::decode(&Request::Open(r.clone()).encode()), Some(Request::Open(r)));
        assert_eq!(Request::decode(&Request::Cli(Vec::new()).encode()), Some(Request::Cli(Vec::new())));
    }

    #[cfg(windows)]
    #[test]
    fn requests_and_replies_over_the_pipe() {
        use std::sync::mpsc;
        use std::time::Duration;
        let name = imp::name(&format!("-test-{}", std::process::id()));
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        assert!(imp::serve_on(name.clone(), move |r| {
            let reply = match &r {
                Request::Cli(a) => format!("echo {}", a.join(" ")),
                Request::Open(_) => "ok".into(),
            };
            tx.lock().unwrap().send(r).unwrap();
            reply
        }));
        // a second server on the same name is refused (no squatting)
        assert!(!imp::serve_on(name.clone(), |_| String::new()));
        // a client that connects and says nothing doesn't block the others
        let silent = {
            use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};
            let h = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
            assert!(h != INVALID_HANDLE_VALUE);
            h
        };
        for i in 0..3 {
            let req = Request::Open(OpenRequest { profile: Some(format!("p{i}")), ..Default::default() });
            assert_eq!(imp::send_to(&name, &req).as_deref(), Some("ok"), "send {i}");
            assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), req);
        }
        let big = "x".repeat(200_000);
        let reply = imp::send_to(&name, &Request::Cli(vec!["list".into(), big.clone()])).unwrap();
        assert_eq!(reply.len(), "echo list ".len() + big.len(), "large payloads both ways");
        assert!(imp::send_to(&imp::name("-nobody-listens"), &Request::Cli(vec![])).is_none());
        unsafe { windows_sys::Win32::Foundation::CloseHandle(silent) };
    }
}
