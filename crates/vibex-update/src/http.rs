//! HTTPS GET through WinHTTP (Windows' own TLS, certificate store and
//! proxy settings), so the updater adds no TLS library.

use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
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

/// How long a whole GET may take: a minute plus 64 KiB/s for `limit`
/// bytes, at most half an hour. Each read also times out on its own (see
/// `WinHttpSetTimeouts`), so a stalled server fails fast and a server that
/// trickles bytes still can't keep a check going for ever.
pub(crate) fn deadline_for(limit: usize) -> Duration {
    let secs = 60 + (limit as u64) / (64 * 1024);
    Duration::from_secs(secs.min(30 * 60))
}

/// Fetch an `https://` URL (redirects followed, never to plain http), up to
/// `limit` bytes. Fails rather than blocks: see [`deadline_for`].
pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("only https URLs are allowed".into());
    }
    let (host, port, path) = crate::split_https_url(url).ok_or("not a plain https URL")?;
    let started = Instant::now();
    let deadline = deadline_for(limit);
    unsafe {
        let agent = wide(concat!("Vibex/", env!("CARGO_PKG_VERSION")));
        let session = H(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            return Err(err("WinHttpOpen"));
        }
        // resolve, connect, send, each receive
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);
        // TLS 1.2 or 1.3 only (1.3 isn't known before Windows 11 / Server 2022)
        let tls: u32 = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        if WinHttpSetOption(session.0, WINHTTP_OPTION_SECURE_PROTOCOLS, &tls as *const u32 as *const _, 4) == 0 {
            let tls12: u32 = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2;
            WinHttpSetOption(session.0, WINHTTP_OPTION_SECURE_PROTOCOLS, &tls12 as *const u32 as *const _, 4);
        }
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
        // never fall back from https on a redirect (also the default)
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
            if started.elapsed() > deadline {
                return Err("the download took too long".into());
            }
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(req.0, &mut avail) == 0 {
                return Err(err("read"));
            }
            if avail == 0 {
                break;
            }
            // never hold more than `limit + 1` bytes, whatever the server says
            let want = (avail as usize).min(limit.saturating_add(1) - out.len()).max(1);
            let start = out.len();
            out.resize(start + want, 0);
            let mut read = 0u32;
            if WinHttpReadData(req.0, out[start..].as_mut_ptr() as *mut _, want as u32, &mut read) == 0 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_get_has_a_deadline() {
        assert_eq!(deadline_for(64 * 1024), Duration::from_secs(61), "the feed check");
        assert!(deadline_for(256 * 1024 * 1024) <= Duration::from_secs(30 * 60), "the installer");
        assert!(deadline_for(usize::MAX) <= Duration::from_secs(30 * 60));
    }

    #[test]
    fn refuses_before_any_network() {
        assert!(get("http://example.com/", 10).is_err());
        assert!(get("https://user@example.com/", 10).is_err());
        assert!(get("https://exa mple.com/", 10).is_err());
    }
}
