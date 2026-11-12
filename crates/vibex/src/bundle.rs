//! The editors Vibex ships: nano (MSYS2's build) and Neovim (the official
//! one), in `<install dir>\tools`. Vibex's shells get them on PATH (after
//! your own, so an editor you installed yourself still wins), and once a
//! week Vibex looks for newer releases: a toast says so, and "Update nano
//! and Neovim" in the palette downloads, checks (SHA-256) and swaps them in.
//! Nothing installs by itself.
//!
//! The check: Neovim's latest GitHub release (the zip's published SHA-256),
//! and MSYS2's package database for nano and the runtime it needs. An
//! update writes a new lock file and runs the bundle's own `update.ps1`
//! (tools\fetch-bundle.ps1) into a fresh folder, then swaps folders.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const NVIM_RELEASE: &str = "https://api.github.com/repos/neovim/neovim/releases/latest";
const MSYS_DB: &str = "https://mirror.msys2.org/msys/x86_64/msys.db";
const MSYS_FILES: &str = "https://mirror.msys2.org/msys/x86_64/";
const WEEK: Duration = Duration::from_secs(7 * 24 * 3600);

/// `<folder of vibex.exe>\tools`, when the editors are installed there.
pub fn tools_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let d = exe.parent()?.join("tools");
    d.join("versions.txt").is_file().then_some(d)
}

/// Folders to add to PATH: Neovim's bin and nano's.
pub fn path_dirs() -> Vec<PathBuf> {
    let Some(t) = tools_dir() else { return Vec::new() };
    [t.join("nvim").join("bin"), t.join("nano").join("usr").join("bin")].into_iter().filter(|d| d.is_dir()).collect()
}

/// One line of a lock file: tool, version, sha256, url.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub url: String,
}

pub fn parse_lock(text: &str) -> Vec<Pin> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 4).then(|| Pin { name: f[0].into(), version: f[1].into(), sha256: f[2].to_ascii_lowercase(), url: f[3].into() })
        })
        .collect()
}

pub fn lock_text(pins: &[Pin]) -> String {
    let mut s = String::from("# written by Vibex's editor update\n");
    for p in pins {
        s.push_str(&format!("{}  {}  {}  {}\n", p.name, p.version, p.sha256, p.url));
    }
    s
}

/// The installed pins (tools\bundle.lock).
pub fn installed() -> Vec<Pin> {
    tools_dir().and_then(|d| std::fs::read_to_string(d.join("bundle.lock")).ok()).map(|t| parse_lock(&t)).unwrap_or_default()
}

/// "0.12.10" > "0.12.9", "9.3-2" > "9.3-1" (numbers compared as numbers).
pub fn newer(a: &str, b: &str) -> bool {
    let nums = |s: &str| -> Vec<u64> { s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect() };
    nums(a) > nums(b)
}

fn stamp_file() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("editor-check"))
}

/// True when the editors are installed and the last look was a week ago.
pub fn due() -> bool {
    if tools_dir().is_none() {
        return false;
    }
    let Some(p) = stamp_file() else { return false };
    match std::fs::metadata(&p).and_then(|m| m.modified()) {
        Ok(t) => SystemTime::now().duration_since(t).map_or(true, |d| d >= WEEK),
        Err(_) => true,
    }
}

/// What's new: (tool, installed, latest) and the lock file that gets it.
#[derive(Clone, Debug)]
pub struct Found {
    pub changes: Vec<(String, String, String)>,
    pub lock: Vec<Pin>,
}

impl Found {
    pub fn describe(&self) -> String {
        self.changes
            .iter()
            .map(|(t, have, new)| format!("{} {} (you have {})", if t == "nvim" { "Neovim" } else { t.as_str() }, new.trim_start_matches('v'), have))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug)]
pub enum Msg {
    Available(Found),
    UpToDate(String),
    /// Updated: what now.
    Done(String),
    Failed(String),
}

/// `"key": "value"` from a small JSON object (no nesting needed here).
fn json_str<'a>(text: &'a str, key: &str, from: usize) -> Option<(&'a str, usize)> {
    let pat = format!("\"{key}\"");
    let i = text[from..].find(&pat)? + from + pat.len();
    let rest = &text[i..];
    let q1 = rest.find('"')? + 1;
    let q2 = rest[q1..].find('"')? + q1;
    Some((&rest[q1..q2], i + q2))
}

/// The latest Neovim: (version, sha256, url of nvim-win64.zip).
pub fn parse_nvim_release(json: &str) -> Option<(String, String, String)> {
    let (tag, _) = json_str(json, "tag_name", 0)?;
    let at = json.find("\"name\":\"nvim-win64.zip\"").or_else(|| json.find("\"name\": \"nvim-win64.zip\""))?;
    // the asset's fields (digest, browser_download_url) run up to the next asset's name
    let end = json[at + 10..].find("\"name\"").map_or(json.len(), |n| n + at + 10);
    let asset = &json[at..end];
    let (digest, _) = json_str(asset, "digest", 0)?;
    let (url, _) = json_str(asset, "browser_download_url", 0)?;
    let sha = digest.strip_prefix("sha256:")?.to_ascii_lowercase();
    (sha.len() == 64 && url.starts_with("https://github.com/neovim/neovim/releases/download/"))
        .then(|| (tag.trim_start_matches('v').to_string(), sha, url.to_string()))
}

/// One package from an MSYS2 database `desc` file: (name, version, file, sha256).
pub fn parse_desc(desc: &str) -> Option<(String, String, String, String)> {
    let field = |key: &str| -> Option<String> {
        let mut it = desc.lines();
        while let Some(l) = it.next() {
            if l.trim() == format!("%{key}%") {
                return it.next().map(|v| v.trim().to_string());
            }
        }
        None
    };
    Some((field("NAME")?, field("VERSION")?, field("FILENAME")?, field("SHA256SUM")?.to_ascii_lowercase()))
}

#[cfg(windows)]
fn msys_packages() -> Result<Vec<(String, String, String, String)>, String> {
    let db = vibex_update::get(MSYS_DB, 16 * 1024 * 1024).map_err(|e| format!("MSYS2 package list: {e}"))?;
    let dir = std::env::temp_dir().join(format!("vibex-msysdb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join("msys.db");
    std::fs::write(&file, &db).map_err(|e| e.to_string())?;
    let ok = std::process::Command::new(tar_exe()).arg("-xf").arg(&file).arg("-C").arg(&dir).no_window().status().is_ok_and(|s| s.success());
    let mut out = Vec::new();
    if ok {
        for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
            if let Ok(t) = std::fs::read_to_string(e.path().join("desc")) {
                out.extend(parse_desc(&t));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if out.is_empty() {
        return Err("MSYS2 package list: couldn't read it".into());
    }
    Ok(out)
}

fn tar_exe() -> PathBuf {
    std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join(r"System32\tar.exe")
}

trait NoWindow {
    fn no_window(&mut self) -> &mut Self;
}
impl NoWindow for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(0x0800_0000)
        }
        #[cfg(not(windows))]
        self
    }
}

/// Compare the installed pins with what's out now.
pub fn compare(have: &[Pin], nvim: Option<(String, String, String)>, msys: &[(String, String, String, String)]) -> Found {
    let mut lock = Vec::new();
    let mut changes = Vec::new();
    for p in have {
        let mut next = p.clone();
        if p.name == "nvim" {
            if let Some((v, sha, url)) = &nvim {
                if newer(v, &p.version) {
                    changes.push(("nvim".into(), p.version.clone(), v.clone()));
                    next = Pin { name: p.name.clone(), version: v.clone(), sha256: sha.clone(), url: url.clone() };
                }
            }
        } else if let Some((_, v, file, sha)) = msys.iter().find(|m| m.0 == p.name) {
            if newer(v, &p.version) {
                // nano's own version is the one worth telling; its runtime just follows
                if p.name == "nano" {
                    changes.push(("nano".into(), p.version.clone(), v.clone()));
                }
                next = Pin { name: p.name.clone(), version: v.clone(), sha256: sha.clone(), url: format!("{MSYS_FILES}{file}") };
            }
        }
        lock.push(next);
    }
    Found { changes, lock }
}

/// Look for newer releases (runs on a background thread).
#[cfg(windows)]
pub fn check() -> Msg {
    if let Some(p) = stamp_file() {
        let _ = std::fs::write(p, b"");
    }
    let have = installed();
    if have.is_empty() {
        return Msg::Failed("nano and Neovim aren't installed with this Vibex".into());
    }
    let nvim = vibex_update::get(NVIM_RELEASE, 2 * 1024 * 1024).ok().and_then(|b| parse_nvim_release(&String::from_utf8_lossy(&b)));
    let msys = msys_packages();
    if nvim.is_none() && msys.is_err() {
        return Msg::Failed("couldn't reach GitHub or MSYS2 to check the editors".into());
    }
    let found = compare(&have, nvim, msys.as_deref().unwrap_or(&[]));
    if found.changes.is_empty() {
        let v = |n: &str| have.iter().find(|p| p.name == n).map(|p| p.version.clone()).unwrap_or_default();
        Msg::UpToDate(format!("nano {} and Neovim {} are up to date", v("nano"), v("nvim")))
    } else {
        Msg::Available(found)
    }
}

#[cfg(not(windows))]
pub fn check() -> Msg {
    Msg::Failed("the bundled editors are Windows-only".into())
}

/// Download, check and swap in what `found` names.
#[cfg(windows)]
pub fn apply(found: &Found) -> Msg {
    let Some(tools) = tools_dir() else { return Msg::Failed("the editors aren't installed".into()) };
    let script = tools.join("update.ps1");
    let tmp = std::env::temp_dir().join(format!("vibex-editors-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    if std::fs::create_dir_all(&tmp).is_err() {
        return Msg::Failed("couldn't make a temporary folder".into());
    }
    let lock = tmp.join("bundle.lock");
    let fresh = tools.with_file_name("tools.new");
    let _ = std::fs::remove_dir_all(&fresh);
    if std::fs::write(&lock, lock_text(&found.lock)).is_err() {
        return Msg::Failed("couldn't write the new lock file".into());
    }
    let run = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-Lock")
        .arg(&lock)
        .arg("-Out")
        .arg(&fresh)
        .arg("-Cache")
        .arg(tmp.join("cache"))
        .no_window()
        .output();
    let result = match run {
        Ok(o) if o.status.success() && fresh.join("versions.txt").is_file() => swap(&tools, &fresh),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("the update script failed").trim().to_string();
            Err(line)
        }
        Err(e) => Err(format!("couldn't run the update: {e}")),
    };
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&fresh);
    match result {
        Ok(()) => Msg::Done(format!("updated: {}", found.describe().replace(" (you have", " (was"))),
        Err(e) => Msg::Failed(format!("editor update: {e}")),
    }
}

#[cfg(not(windows))]
pub fn apply(_found: &Found) -> Msg {
    Msg::Failed("the bundled editors are Windows-only".into())
}

/// tools -> tools.old, tools.new -> tools (a folder in use can't move:
/// nano or Neovim still open).
fn swap(tools: &Path, fresh: &Path) -> Result<(), String> {
    let old = tools.with_file_name("tools.old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(tools, &old).map_err(|_| "nano or Neovim is open: close it and update again".to_string())?;
    if let Err(e) = std::fs::rename(fresh, tools) {
        let _ = std::fs::rename(&old, tools);
        return Err(format!("couldn't put the new editors in place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("0.12.10", "0.12.9"));
        assert!(newer("9.3-2", "9.3-1"));
        assert!(newer("10.0-1", "9.3-1"));
        assert!(!newer("0.12.6", "0.12.6"));
        assert!(!newer("9.2-1", "9.3-1"));
    }

    #[test]
    fn reads_lock_and_round_trips() {
        let text = "# c\nnvim  0.12.6  ABCD  https://x/nvim.zip\nnano 9.3-1 ef https://m/nano.zst\n";
        let pins = parse_lock(text);
        assert_eq!(pins.len(), 2);
        assert_eq!(pins[0].sha256, "abcd");
        assert_eq!(parse_lock(&lock_text(&pins)), pins);
    }

    #[test]
    fn neovim_release_json() {
        let sha = "4e12b7103b601ffc8f25954b4da100f339faa73dccb69a5b135e897fa3d893ed";
        let json = format!(
            r#"{{"tag_name":"v0.12.7","assets":[{{"name":"nvim-linux-x86_64.tar.gz","digest":"sha256:{}","browser_download_url":"https://github.com/neovim/neovim/releases/download/v0.12.7/nvim-linux-x86_64.tar.gz"}},{{"name":"nvim-win64.zip","size":1,"digest":"sha256:{sha}","browser_download_url":"https://github.com/neovim/neovim/releases/download/v0.12.7/nvim-win64.zip"}}]}}"#,
            "0".repeat(64)
        );
        let (v, s, u) = parse_nvim_release(&json).unwrap();
        assert_eq!(v, "0.12.7");
        assert_eq!(s, sha);
        assert!(u.ends_with("/v0.12.7/nvim-win64.zip"));
        // a download from anywhere else is refused
        assert!(parse_nvim_release(&json.replace("github.com/neovim/neovim", "evil.example")).is_none());
    }

    #[test]
    fn msys_desc_and_compare() {
        let desc = "%FILENAME%\nnano-9.4-1-x86_64.pkg.tar.zst\n\n%NAME%\nnano\n\n%VERSION%\n9.4-1\n\n%SHA256SUM%\nAA11\n";
        let d = parse_desc(desc).unwrap();
        assert_eq!(d, ("nano".into(), "9.4-1".into(), "nano-9.4-1-x86_64.pkg.tar.zst".into(), "aa11".into()));
        let have = parse_lock("nvim 0.12.6 aa https://x\nnano 9.3-1 bb https://y\nzlib 1.4-1 cc https://z\n");
        let f = compare(&have, Some(("0.12.6".into(), "aa".into(), "https://x".into())), &[d, ("zlib".into(), "1.4-1".into(), "z".into(), "cc".into())]);
        assert_eq!(f.changes, vec![("nano".to_string(), "9.3-1".to_string(), "9.4-1".to_string())]);
        assert_eq!(f.lock[1].url, "https://mirror.msys2.org/msys/x86_64/nano-9.4-1-x86_64.pkg.tar.zst");
        assert_eq!(f.lock[0], have[0]);
        assert!(f.describe().contains("nano 9.4-1 (you have 9.3-1)"));
    }
}
