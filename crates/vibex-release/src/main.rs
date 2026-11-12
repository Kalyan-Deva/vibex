//! vibex-release: make the update signing key and signed update feeds.
//!
//!   vibex-release keygen [KEYFILE]
//!       new secret key (default %USERPROFILE%\.vibex\update-signing.key,
//!       never overwritten); prints the public key to build into Vibex
//!   vibex-release pubkey [KEYFILE]
//!   vibex-release feed --version V --url URL --installer FILE [--notes TEXT]
//!                      [--key KEYFILE] [--out update.txt]
//!       sign an installer and write the feed to publish next to it

use std::path::PathBuf;

fn default_key() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default();
    PathBuf::from(home).join(".vibex").join("update-signing.key")
}

fn read_key(path: &PathBuf) -> Result<String, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(s.trim().to_string())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = args.get(1).filter(|a| !a.starts_with('-')).map(PathBuf::from).unwrap_or_else(default_key);
            if path.exists() {
                return Err(format!("{} already exists; not overwriting a signing key", path.display()));
            }
            let mut seed = [0u8; 32];
            getrandom::getrandom(&mut seed).map_err(|e| e.to_string())?;
            let secret = vibex_update::hex(&seed);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, format!("{secret}\n")).map_err(|e| e.to_string())?;
            let public = vibex_update::public_key(&secret).unwrap();
            std::fs::write(path.with_extension("pub"), format!("{public}\n")).map_err(|e| e.to_string())?;
            println!("secret key: {} (keep it off GitHub; back it up)", path.display());
            println!("public key: {public}");
            Ok(())
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
            std::fs::write(&out, feed.to_text()).map_err(|e| e.to_string())?;
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
