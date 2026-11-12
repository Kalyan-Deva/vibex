//! Updates: check a signed feed, download, verify, hand over to the
//! installer. Off until `config.update_feed` points at a published feed.

// downloading and installing is Windows-only for now
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use vibex_update::Feed;

/// Public half of the release signing key (the secret stays on the release
/// machine, see crates/vibex-release).
pub const PUBLIC_KEY: &str = "640e9cf3d5d6760e869d1d743da912758681ae487640af8bdab7809fc3540540";
const FEED_LIMIT: usize = 64 * 1024;
const INSTALLER_LIMIT: usize = 256 * 1024 * 1024;

#[derive(Debug)]
pub enum Msg {
    Available(Feed),
    UpToDate,
    /// Verified installer saved here.
    Ready(PathBuf, Feed),
    Failed(String),
}

fn stamp_file() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("update-check"))
}

/// True if the last automatic check was over a day ago.
pub fn due() -> bool {
    let Some(p) = stamp_file() else { return false };
    match std::fs::metadata(&p).and_then(|m| m.modified()) {
        Ok(t) => SystemTime::now().duration_since(t).map_or(true, |d| d > Duration::from_secs(20 * 3600)),
        Err(_) => true,
    }
}

fn touch_stamp() {
    if let Some(p) = stamp_file() {
        let _ = std::fs::write(p, b"");
    }
}

#[cfg(windows)]
pub fn check(feed_url: &str) -> Msg {
    touch_stamp();
    let text = match vibex_update::get(feed_url, FEED_LIMIT) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return Msg::Failed(format!("update check: {e}")),
    };
    let feed = match Feed::parse(&text) {
        Ok(f) => f,
        Err(e) => return Msg::Failed(e),
    };
    if !feed.signature_ok(PUBLIC_KEY) {
        return Msg::Failed("update check: the feed's signature is not valid".into());
    }
    if vibex_update::is_newer(&feed.version, env!("CARGO_PKG_VERSION")) {
        Msg::Available(feed)
    } else {
        Msg::UpToDate
    }
}

#[cfg(windows)]
pub fn download(feed: Feed) -> Msg {
    let data = match vibex_update::get(&feed.url, INSTALLER_LIMIT) {
        Ok(d) => d,
        Err(e) => return Msg::Failed(format!("download: {e}")),
    };
    if let Err(e) = feed.verify_installer(PUBLIC_KEY, &data) {
        return Msg::Failed(e);
    }
    let dir = std::env::temp_dir().join("vibex-update");
    let path = dir.join(format!("Vibex-Setup-{}.exe", feed.version));
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &data)) {
        return Msg::Failed(format!("cannot save the update: {e}"));
    }
    Msg::Ready(path, feed)
}

#[cfg(not(windows))]
pub fn check(_feed_url: &str) -> Msg {
    Msg::Failed("updates are Windows-only for now".into())
}

#[cfg(not(windows))]
pub fn download(_feed: Feed) -> Msg {
    Msg::Failed("updates are Windows-only for now".into())
}

/// Start the installer: it waits for this process to exit, installs
/// quietly and starts Vibex again.
pub fn launch_installer(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new(path).args(["--quiet", "--wait-pid", &std::process::id().to_string(), "--relaunch"]).spawn().map(|_| ())
}
