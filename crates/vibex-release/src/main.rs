//! vibex-release: make the update signing key and signed update feeds.
//!
//!   vibex-release keygen [KEYFILE]
//!       new secret key (default %USERPROFILE%\.vibex\update-signing.key,
//!       never overwritten, never inside a git checkout); prints the public
//!       key to build into Vibex
//!   vibex-release pubkey [KEYFILE]
//!   vibex-release feed --version V --url URL --installer FILE [--notes TEXT]
//!                      [--key KEYFILE] [--out update.txt]
//!       sign an installer and write the feed to publish next to it
//!
//! The secret key is only ever read from / written to KEYFILE: it is never
//! printed, and errors name the file, not its contents.

use std::io::Write;
use std::path::{Path, PathBuf};

fn default_key() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default();
    PathBuf::from(home).join(".vibex").join("update-signing.key")
}

fn read_key(path: &Path) -> Result<String, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(s.trim().to_string())
}

/// The value after `--name`. A following `--flag` is not a value: Windows
/// PowerShell drops empty arguments, so `--notes "" --out f` arrives as
/// `--notes --out f`.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).filter(|v| !v.starts_with("--")).cloned()
}

/// The git work tree `path` would be in, if any (a key there is one
/// `git add -A` away from being published).
fn git_work_tree(path: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(path).ok()?;
    abs.ancestors().skip(1).find(|d| d.join(".git").exists()).map(Path::to_path_buf)
}

fn keygen(path: &Path) -> Result<(), String> {
    if let Some(repo) = git_work_tree(path) {
        return Err(format!("{} is inside the git checkout {}; keep the signing key out of repositories", path.display(), repo.display()));
    }
    let public_path = path.with_extension("pub");
    if public_path == path {
        return Err("the key file can't end in .pub (that name is for the public key)".into());
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| e.to_string())?;
    let secret = vibex_update::hex(&seed);
    // create_new: never overwrite a signing key, even one made a moment ago
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{} already exists; not overwriting a signing key", path.display()),
        _ => format!("cannot create {}: {e}", path.display()),
    })?;
    f.write_all(format!("{secret}\n").as_bytes()).and_then(|_| f.sync_all()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let public = vibex_update::public_key(&secret).ok_or("bad key")?;
    std::fs::write(&public_path, format!("{public}\n")).map_err(|e| format!("cannot write {}: {e}", public_path.display()))?;
    println!("secret key: {} (keep it off GitHub; back it up)", path.display());
    println!("public key: {public}");
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| flag(&args, name);
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = args.get(1).filter(|a| !a.starts_with('-')).map(PathBuf::from).unwrap_or_else(default_key);
            keygen(&path)
        }
        Some("pubkey") => {
            let path = args.get(1).map(PathBuf::from).unwrap_or_else(default_key);
            let public = vibex_update::public_key(&read_key(&path)?).ok_or("bad key file")?;
            println!("{public}");
            Ok(())
        }
        Some("feed") => {
            let version = flag("--version").ok_or("--version is required")?;
            let url = flag("--url").ok_or("--url is required")?;
            let installer = flag("--installer").ok_or("--installer is required")?;
            let notes = flag("--notes").unwrap_or_default();
            let key = flag("--key").map(PathBuf::from).unwrap_or_else(default_key);
            let out = flag("--out").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("update.txt"));
            let data = std::fs::read(&installer).map_err(|e| format!("cannot read {installer}: {e}"))?;
            let feed = vibex_update::sign(&read_key(&key)?, &version, &url, &data, &notes)?;
            std::fs::write(&out, feed.to_text()).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
            println!("wrote {} (sha256 {})", out.display(), feed.sha256);
            Ok(())
        }
        _ => {
            Err("usage: vibex-release keygen [KEYFILE] | pubkey [KEYFILE] | feed --version V --url URL --installer FILE [--notes T] [--key F] [--out F]".into())
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("vibex-release: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vibex-release-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn flags_skip_dropped_values() {
        let a: Vec<String> = ["feed", "--version", "1.0.0", "--notes", "--out", "x.txt"].iter().map(|s| s.to_string()).collect();
        assert_eq!(flag(&a, "--version").as_deref(), Some("1.0.0"));
        assert_eq!(flag(&a, "--notes"), None, "an empty --notes, dropped by PowerShell");
        assert_eq!(flag(&a, "--out").as_deref(), Some("x.txt"));
        assert_eq!(flag(&a, "--key"), None);
    }

    #[test]
    fn keygen_never_overwrites_and_stays_out_of_repos() {
        let d = scratch("keygen");
        let key = d.join("k.key");
        keygen(&key).unwrap();
        let first = std::fs::read_to_string(&key).unwrap();
        let public = std::fs::read_to_string(d.join("k.pub")).unwrap();
        assert_eq!(vibex_update::public_key(first.trim()).unwrap(), public.trim());
        assert!(keygen(&key).unwrap_err().contains("already exists"));
        assert_eq!(std::fs::read_to_string(&key).unwrap(), first, "untouched");
        assert!(keygen(&d.join("k2.pub")).is_err());

        let repo = d.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let e = keygen(&repo.join("sub").join("signing.key")).unwrap_err();
        assert!(e.contains("git checkout"), "{e}");
        assert!(!repo.join("sub").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
