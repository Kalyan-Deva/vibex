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
//! too). The feed itself needs no trust.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

#[cfg(windows)]
mod http;
#[cfg(windows)]
pub use http::get;

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
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
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
                "notes" => f.notes = v,
                _ => {}
            }
        }
        if parse_version(&f.version).is_none() {
            return Err("feed: bad or missing version".into());
        }
        if !f.url.starts_with("https://") {
            return Err("feed: the installer URL must be https".into());
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
        vk.verify(&message(&self.version, &self.sha256), &Signature::from_bytes(&sig)).is_ok()
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
    let sha256 = sha256_hex(installer);
    let sig = key.sign(&message(version, &sha256));
    let feed = Feed { version: version.into(), url: url.into(), sha256, signature: hex(&sig.to_bytes()), notes: notes.into() };
    Feed::parse(&feed.to_text())?;
    Ok(feed)
}

/// Public key (hex) for a secret seed (hex).
pub fn public_key(secret_hex: &str) -> Option<String> {
    let seed = unhex(secret_hex).and_then(|b| <[u8; 32]>::try_from(b).ok())?;
    Some(hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes()))
}

/// `1.2.3` (an optional `-suffix` is ignored for ordering).
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u32>().ok());
    let v = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(v)
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
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
