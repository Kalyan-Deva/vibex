//! HTTPS GET through WinHTTP (Windows' own TLS, certificate store and
//! proxy settings), so the updater adds no TLS library.

use std::ptr::{null, null_mut};
use windows_sys::Win32::Networking::WinHttp::*;

struct H(*mut std::ffi::c_void);

impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn err(what: &str) -> String {
    format!("{what} failed ({})", std::io::Error::last_os_error())
}

/// Fetch an `https://` URL (redirects followed), up to `limit` bytes.
pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let rest = url.strip_prefix("https://").ok_or("only https URLs are allowed")?;
    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "bad port")?),
        None => (host_port, 443),
    };
    unsafe {
        let agent = wide(concat!("Vibex/", env!("CARGO_PKG_VERSION")));
        let session = H(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            return Err(err("WinHttpOpen"));
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 60_000);
        let host_w = wide(host);
        let conn = H(WinHttpConnect(session.0, host_w.as_ptr(), port, 0));
        if conn.0.is_null() {
            return Err(err("connect"));
        }
        let verb = wide("GET");
        let path_w = wide(path);
        let req = H(WinHttpOpenRequest(conn.0, verb.as_ptr(), path_w.as_ptr(), null(), null(), null(), WINHTTP_FLAG_SECURE));
        if req.0.is_null() {
            return Err(err("request"));
        }
        // never fall back from https on a redirect
        let policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
        WinHttpSetOption(req.0, WINHTTP_OPTION_REDIRECT_POLICY, &policy as *const u32 as *const _, 4);
        if WinHttpSendRequest(req.0, null(), 0, null(), 0, 0, 0) == 0 {
            return Err(err("send"));
        }
        if WinHttpReceiveResponse(req.0, null_mut()) == 0 {
            return Err(err("response"));
        }
        let mut status: u32 = 0;
        let mut len = 4u32;
        WinHttpQueryHeaders(req.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, null(), &mut status as *mut u32 as *mut _, &mut len, null_mut());
        if status != 200 {
            return Err(format!("server answered {status}"));
        }
        let mut out = Vec::new();
        loop {
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(req.0, &mut avail) == 0 {
                return Err(err("read"));
            }
            if avail == 0 {
                break;
            }
            let start = out.len();
            out.resize(start + avail as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(req.0, out[start..].as_mut_ptr() as *mut _, avail, &mut read) == 0 {
                return Err(err("read"));
            }
            out.truncate(start + read as usize);
            if out.len() > limit {
                return Err("download is larger than expected".into());
            }
        }
        Ok(out)
    }
}
