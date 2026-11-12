//! OSC 8 hyperlink targets, shared by all terminals (cells store a u16 id).

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

#[derive(Default)]
struct Table {
    urls: Vec<Box<str>>,
    ids: HashMap<Box<str>, u16>,
}

fn table() -> &'static RwLock<Table> {
    static T: OnceLock<RwLock<Table>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// Id for `url` (1..=65535); 0 if the table is full or the URL is empty.
pub fn intern(url: &str) -> u16 {
    if url.is_empty() || url.len() > 2048 {
        return 0;
    }
    if let Some(&id) = table().read().unwrap().ids.get(url) {
        return id;
    }
    let mut t = table().write().unwrap();
    if let Some(&id) = t.ids.get(url) {
        return id;
    }
    if t.urls.len() >= u16::MAX as usize {
        return 0;
    }
    t.urls.push(url.into());
    let id = t.urls.len() as u16;
    t.ids.insert(url.into(), id);
    id
}

pub fn url(id: u16) -> Option<String> {
    if id == 0 {
        return None;
    }
    table().read().unwrap().urls.get(id as usize - 1).map(|s| s.to_string())
}

/// Links Vibex will open on Ctrl+click. Anything else (file:, custom
/// schemes) could launch programs, so it is never opened.
pub fn is_safe_to_open(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

/// Standard base64 (RFC 4648) for OSC 52.
pub fn base64_encode(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
        if chunk.len() == 1 {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar", "héllo 🦀"] {
            let e = base64_encode(s.as_bytes());
            assert_eq!(base64_decode(&e).unwrap(), s.as_bytes(), "{s} -> {e}");
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert!(base64_decode("@@@@").is_none());
    }

    #[test]
    fn link_table() {
        let a = intern("https://example.com");
        assert!(a > 0);
        assert_eq!(intern("https://example.com"), a);
        assert_eq!(url(a).as_deref(), Some("https://example.com"));
        assert_eq!(intern(""), 0);
        assert!(is_safe_to_open("HTTPS://x.y"));
        assert!(!is_safe_to_open("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_safe_to_open("ms-settings:"));
    }
}
