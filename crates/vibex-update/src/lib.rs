//! Signed updates.
//!
//! A feed is a small `key = value` text file:
//!
//! ```text
//! version = 0.2.0
//! url = https://example.com/Vibex-Setup-0.2.0.exe
//! sha256 = <64 hex chars of the installer>
//! signature = <128 hex chars>
//! notes = one line shown to the user
//! ```
//!
//! The signature is Ed25519 over `vibex-update\n<version>\n<sha256>\n`, made
//! with a key that never leaves the release machine; Vibex has the public
//! key built in. So a changed installer fails the hash, and an old signed
//! installer can't be passed off as a newer version (the version is signed
//! too). The feed itself needs no trust for what gets installed; `url` and
//! `notes` are not signed, so `notes` is cut to one short line, and `url`
//! only decides where the (hash-checked) download comes from.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

#[cfg(windows)]
mod http;
#[cfg(windows)]
pub use http::get;

/// Longest `notes` kept from a feed (they are shown in a toast, unsigned).
pub const NOTES_MAX: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feed {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub signature: String,
    pub notes: String,
}

fn message(version: &str, sha256: &str) -> Vec<u8> {
    format!("vibex-update\n{version}\n{}\n", sha256.to_ascii_lowercase()).into_bytes()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    // only hex digits: `from_str_radix` alone would also take a `+` sign
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

impl Feed {
    pub fn parse(text: &str) -> Result<Feed, String> {
        let mut f = Feed { version: String::new(), url: String::new(), sha256: String::new(), signature: String::new(), notes: String::new() };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().to_string();
            match k.trim() {
                "version" => f.version = v,
                "url" => f.url = v,
                "sha256" => f.sha256 = v.to_ascii_lowercase(),
                "signature" => f.signature = v,
                "notes" => f.notes = clean_notes(&v),
                _ => {}
            }
        }
        if parse_version(&f.version).is_none() {
            return Err("feed: bad or missing version".into());
        }
        if !f.url.starts_with("https://") || split_https_url(&f.url).is_none() {
            return Err("feed: the installer URL must be a plain https URL".into());
        }
        if unhex(&f.sha256).map(|b| b.len()) != Some(32) {
            return Err("feed: bad sha256".into());
        }
        if unhex(&f.signature).map(|b| b.len()) != Some(64) {
            return Err("feed: bad signature".into());
        }
        Ok(f)
    }

    pub fn to_text(&self) -> String {
        format!(
            "version = {}\nurl = {}\nsha256 = {}\nsignature = {}\nnotes = {}\n",
            self.version,
            self.url,
            self.sha256,
            self.signature,
            self.notes.replace('\n', " ")
        )
    }

    /// The feed's signature is valid for `public_key` (hex).
    pub fn signature_ok(&self, public_key: &str) -> bool {
        let Some(pk) = unhex(public_key).and_then(|b| <[u8; 32]>::try_from(b).ok()) else { return false };
        let Ok(vk) = VerifyingKey::from_bytes(&pk) else { return false };
        let Some(sig) = unhex(&self.signature).and_then(|b| <[u8; 64]>::try_from(b).ok()) else { return false };
        // strict: no small-order keys or non-canonical signatures
        vk.verify_strict(&message(&self.version, &self.sha256), &Signature::from_bytes(&sig)).is_ok()
    }

    /// Check a downloaded installer against the (signed) feed.
    pub fn verify_installer(&self, public_key: &str, data: &[u8]) -> Result<(), String> {
        if !self.signature_ok(public_key) {
            return Err("the update's signature is not valid".into());
        }
        if sha256_hex(data) != self.sha256 {
            return Err("the downloaded installer doesn't match the signed hash".into());
        }
        Ok(())
    }
}

/// Sign a release: returns the feed for `installer` published at `url`.
pub fn sign(secret_hex: &str, version: &str, url: &str, installer: &[u8], notes: &str) -> Result<Feed, String> {
    let seed = unhex(secret_hex).and_then(|b| <[u8; 32]>::try_from(b).ok()).ok_or("bad secret key")?;
    let key = SigningKey::from_bytes(&seed);
    let version = version.trim();
    if parse_version(version).is_none() {
        return Err(format!("bad version {version:?} (want e.g. 1.2.3 or 1.2.3-rc1)"));
    }
    let sha256 = sha256_hex(installer);
    let sig = key.sign(&message(version, &sha256));
    let feed = Feed { version: version.into(), url: url.trim().into(), sha256, signature: hex(&sig.to_bytes()), notes: clean_notes(notes) };
    // what clients will read must be exactly what was signed
    let parsed = Feed::parse(&feed.to_text())?;
    if parsed != feed {
        return Err("the feed doesn't read back as written (a newline in the URL?)".into());
    }
    Ok(parsed)
}

/// Notes are not signed: one line, no control characters, short.
fn clean_notes(s: &str) -> String {
    let one_line: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    one_line.trim().chars().take(NOTES_MAX).collect()
}

/// Split `https://host[:port]/path?query` into (host, port, path-and-query).
/// Refuses user info (`user@host`), empty hosts, a bad port, spaces and
/// control characters; drops a `#fragment`.
pub fn split_https_url(url: &str) -> Option<(&str, u16, &str)> {
    let rest = url.strip_prefix("https://")?;
    if rest.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return None;
    }
    let rest = rest.split('#').next()?;
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (host_port, path) = rest.split_at(end);
    if host_port.contains('@') {
        return None;
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok().filter(|&p| p != 0)?),
        None => (host_port, 443),
    };
    if host.is_empty() || host.contains(':') {
        return None;
    }
    Some((host, port, if path.is_empty() { "/" } else { path }))
}

/// Public key (hex) for a secret seed (hex).
pub fn public_key(secret_hex: &str) -> Option<String> {
    let seed = unhex(secret_hex).and_then(|b| <[u8; 32]>::try_from(b).ok())?;
    Some(hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes()))
}

/// `1.2.3`, `v1.2.3`, `1.2.3-rc1` or `1.2.3+build`. The suffix may only
/// hold `[0-9A-Za-z.-]` (the version ends up in a file name).
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    version_key(v).map(|(a, b, c, _)| (a, b, c))
}

/// (major, minor, patch, is-a-release): `1.2.3-rc1` sorts before `1.2.3`.
fn version_key(v: &str) -> Option<(u32, u32, u32, bool)> {
    let v = v.trim();
    let v = v.strip_prefix('v').unwrap_or(v);
    let end = v.find(['-', '+']).unwrap_or(v.len());
    let (core, suffix) = v.split_at(end);
    let ok_suffix = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let (pre, build) = match suffix.split_once('+') {
        Some((pre, build)) => (pre, Some(build)),
        None => (suffix, None),
    };
    if build.is_some_and(|b| !ok_suffix(b)) || (!pre.is_empty() && !ok_suffix(&pre[1..])) {
        return None;
    }
    let num = |p: &str| (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse::<u32>().ok()).flatten();
    let mut it = core.split('.').map(num);
    let key = (it.next()??, it.next()??, it.next()??, pre.is_empty());
    it.next().is_none().then_some(key)
}

/// `candidate` is a strictly newer version than `current`. Anything that
/// doesn't parse is never newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((version_key(candidate), version_key(current)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";

    #[test]
    fn sign_and_verify() {
        let pk = public_key(SECRET).unwrap();
        let installer = b"pretend installer bytes";
        let feed = sign(SECRET, "0.2.0", "https://example.com/v.exe", installer, "faster").unwrap();
        let parsed = Feed::parse(&feed.to_text()).unwrap();
        assert_eq!(parsed, feed);
        assert!(parsed.verify_installer(&pk, installer).is_ok());
        // tampered installer
        assert!(parsed.verify_installer(&pk, b"evil").is_err());
        // an old signed installer relabelled as a newer version
        let relabelled = Feed { version: "9.9.9".into(), ..parsed.clone() };
        assert!(relabelled.verify_installer(&pk, installer).is_err());
        // someone else's key
        let other = public_key(&"11".repeat(32)).unwrap();
        assert!(parsed.verify_installer(&other, installer).is_err());
    }

    #[test]
    fn feed_rules() {
        let ok = sign(SECRET, "1.0.0", "https://x/y.exe", b"z", "").unwrap();
        assert!(Feed::parse(&ok.to_text().replace("https://", "http://")).is_err(), "plain http refused");
        assert!(Feed::parse("version = 1.0\n").is_err());
    }

    #[test]
    fn hostile_feeds_never_verify() {
        let pk = public_key(SECRET).unwrap();
        let installer = b"the real installer";
        let good = sign(SECRET, "0.3.0", "https://example.com/v.exe", installer, "").unwrap();
        // missing or malformed signature: refused when parsing
        let text = good.to_text();
        let unsigned: String = text.lines().filter(|l| !l.starts_with("signature")).map(|l| format!("{l}\n")).collect();
        assert!(Feed::parse(&unsigned).is_err());
        assert!(Feed::parse(&text.replace(&good.signature, &"zz".repeat(64))).is_err());
        // a well-formed but wrong signature: parses, never verifies
        let flipped = format!("{}{}", if good.signature.starts_with('0') { "1" } else { "0" }, &good.signature[1..]);
        let bad = Feed::parse(&text.replace(&good.signature, &flipped)).unwrap();
        assert!(!bad.signature_ok(&pk));
        assert!(bad.verify_installer(&pk, installer).is_err());
        // a different file, or a truncated download, fails the signed hash
        assert!(good.verify_installer(&pk, b"the real installe").is_err());
        assert!(good.verify_installer(&pk, b"").is_err());
        let mut swapped = good.clone();
        swapped.sha256 = sha256_hex(b"evil");
        assert!(swapped.verify_installer(&pk, b"evil").is_err(), "hash and signature go together");
        // `+` signs are not hex
        assert_eq!(unhex("+f+f"), None);
        assert_eq!(unhex("0aFf"), Some(vec![0x0a, 0xff]));
        assert_eq!(unhex("0a f"), None);
        // a replayed older (validly signed) feed is not an update
        let old = sign(SECRET, "0.2.9", "https://example.com/old.exe", b"old", "").unwrap();
        assert!(old.signature_ok(&pk));
        assert!(!is_newer(&old.version, "0.3.0"));
        assert!(!is_newer(&good.version, "0.3.0"), "same version is not newer");
    }

    #[test]
    fn feed_fields_are_checked() {
        let ok = sign(SECRET, "1.0.0", "https://x/y.exe", b"z", "").unwrap().to_text();
        for bad_version in ["1.0", "1.0.0.0", "1.0.x", "", "1.0.0-", "1.0.0-../../evil", "1.0.0-a b", "+1.0.0", "1.+2.3"] {
            let text = ok.replace("version = 1.0.0", &format!("version = {bad_version}"));
            assert!(Feed::parse(&text).is_err(), "version {bad_version:?}");
        }
        for bad_url in ["http://x/y.exe", "https://", "https://u:p@x/y.exe", "https://x:0/y", "https://x:99999/y", "ftp://x"] {
            let text = ok.replace("url = https://x/y.exe", &format!("url = {bad_url}"));
            assert!(Feed::parse(&text).is_err(), "url {bad_url:?}");
        }
        assert!(sign(SECRET, "1.0.0", "https://x/y.exe\nversion = 9.9.9", b"z", "").is_err(), "a newline can't smuggle a field");
        assert!(sign(SECRET, "1.0.0-../x", "https://x/y.exe", b"z", "").is_err());
        let long = sign(SECRET, "1.0.0", "https://x/y.exe", b"z", &format!("a\tb\r\nc{}", "x".repeat(500))).unwrap();
        assert!(long.notes.starts_with("a b  c") && long.notes.chars().count() == NOTES_MAX);
    }

    #[test]
    fn url_parts() {
        assert_eq!(split_https_url("https://example.com"), Some(("example.com", 443, "/")));
        assert_eq!(split_https_url("https://example.com:8443/a/b?c=d#frag"), Some(("example.com", 8443, "/a/b?c=d")));
        assert_eq!(split_https_url("https://example.com?q"), Some(("example.com", 443, "?q")));
        assert_eq!(split_https_url("https://good.com@evil.com/"), None);
        assert_eq!(split_https_url("https://evil.com\\@good.com/"), None);
        assert_eq!(split_https_url("https://a b/"), None);
        assert_eq!(split_https_url("https://[::1]:443/"), None, "IPv6 literals aren't supported");
        assert_eq!(split_https_url("http://example.com/"), None);
    }

    #[test]
    fn prereleases_sort_before_releases() {
        assert!(is_newer("0.2.0", "0.2.0-rc1"));
        assert!(!is_newer("0.2.0-rc2", "0.2.0-rc1"), "prereleases of one version are equal");
        assert!(is_newer("0.2.0-rc1", "0.1.9"));
        assert!(!is_newer("0.2.0+build5", "0.2.0"), "build metadata doesn't count");
        assert!(!is_newer("0.2.0", "garbage"));
        assert!(!is_newer("4294967296.0.0", "0.1.0"), "overflow is not a version");
        assert_eq!(parse_version(" v1.2.3-rc.1+abc "), Some((1, 2, 3)));
        assert_eq!(parse_version("vv1.2.3"), None);
    }

    #[test]
    fn versions() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-rc1", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert_eq!(parse_version("v0.3.1"), Some((0, 3, 1)));
    }
}

#[cfg(all(test, windows))]
mod net_tests {
    /// Needs the internet: `cargo test -p vibex-update -- --ignored`.
    #[test]
    #[ignore]
    fn https_get() {
        let body = crate::get("https://example.com/", 1 << 20).unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Example Domain"));
        assert!(crate::get("http://example.com/", 1 << 20).is_err(), "plain http refused");
        assert!(crate::get("https://example.com/", 10).is_err(), "size limit enforced");
    }
}
