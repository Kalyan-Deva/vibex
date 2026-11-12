//! SSH hosts for the picker: `Host` names from `~/.ssh/config` (and files
//! it `Include`s), then hosts in `~/.ssh/known_hosts` that aren't hashed.
//! Patterns (`*`, `?`, `!`) are skipped since they aren't connectable.

use std::path::{Path, PathBuf};

fn ssh_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".ssh"))
}

fn usable(h: &str) -> bool {
    !h.is_empty() && !h.starts_with('-') && !h.contains(['*', '?', '!', ' ', '\t', '"', '\'', '\\', '`', '$'])
}

/// Hosts from an ssh_config text. `include` resolves `Include` targets.
pub fn parse_config(text: &str, include: &mut dyn FnMut(&str) -> Option<String>, depth: usize, out: &mut Vec<String>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // "Key value" or "Key=value"
        let (key, rest) = match line.find(|c: char| c.is_whitespace() || c == '=') {
            Some(i) => (&line[..i], line[i + 1..].trim_start_matches(|c: char| c.is_whitespace() || c == '=')),
            None => continue,
        };
        if key.eq_ignore_ascii_case("host") {
            for h in rest.split_whitespace() {
                if usable(h) && !out.iter().any(|o| o == h) {
                    out.push(h.to_string());
                }
            }
        } else if key.eq_ignore_ascii_case("include") && depth < 4 {
            for target in rest.split_whitespace() {
                if let Some(t) = include(target) {
                    parse_config(&t, include, depth + 1, out);
                }
            }
        }
    }
}

/// Unhashed host names from a known_hosts text.
pub fn parse_known_hosts(text: &str, out: &mut Vec<String>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('|') || line.starts_with('@') {
            continue;
        }
        let Some(field) = line.split_whitespace().next() else { continue };
        // "name,ip": the name alone is enough
        let names: Vec<&str> = field.split(',').collect();
        let named = names.iter().any(|n| bare(n).parse::<std::net::IpAddr>().is_err());
        for name in names {
            if named && bare(name).parse::<std::net::IpAddr>().is_ok() {
                continue;
            }
            // keep "[host]:port" as written; `command` splits the port off
            let host = bare(name);
            if usable(host) && !out.iter().any(|o| o == name) {
                out.push(name.to_string());
            }
        }
    }
}

/// `[host]:port` -> host.
fn bare(name: &str) -> &str {
    name.strip_prefix('[').and_then(|n| n.split(']').next()).unwrap_or(name)
}

fn read_include(dir: &Path, target: &str) -> Option<String> {
    let p = if let Some(rest) = target.strip_prefix("~/") {
        dir.parent()?.join(rest)
    } else {
        let p = PathBuf::from(target);
        if p.is_absolute() {
            p
        } else {
            dir.join(p)
        }
    };
    // simple one-level globs like config.d/*
    let name = p.file_name()?.to_string_lossy().to_string();
    if name == "*" {
        let mut all = String::new();
        let mut entries: Vec<_> = std::fs::read_dir(p.parent()?).ok()?.flatten().map(|e| e.path()).collect();
        entries.sort();
        for e in entries.into_iter().filter(|e| e.is_file()) {
            if let Ok(t) = std::fs::read_to_string(e) {
                all.push_str(&t);
                all.push('\n');
            }
        }
        return Some(all);
    }
    std::fs::read_to_string(p).ok()
}

/// Every host: the ones you've connected to (newest first), then
/// `~/.ssh/config`, then `known_hosts`.
pub fn hosts() -> Vec<String> {
    let Some(dir) = ssh_dir() else { return Vec::new() };
    let mut out: Vec<String> = recent().into_iter().map(|(h, _)| h).collect();
    if let Ok(text) = std::fs::read_to_string(dir.join("config")) {
        let d = dir.clone();
        parse_config(&text, &mut |t| read_include(&d, t), 0, &mut out);
    }
    if let Ok(text) = std::fs::read_to_string(dir.join("known_hosts")) {
        parse_known_hosts(&text, &mut out);
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|h| seen.insert(h.clone()));
    out.truncate(200);
    out
}

// ------------------------------------------------------------ remembered

fn recent_path() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("ssh_recent.tsv"))
}

/// Note a connection (`unix-seconds \t host`), best effort.
pub fn remember(host: &str) {
    use std::io::Write;
    let Some(p) = recent_path() else { return };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{}\t{}", crate::history::now(), host);
    }
}

/// Hosts you've connected to, newest first, each once (with when).
pub fn recent() -> Vec<(String, u64)> {
    let Some(text) = recent_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    parse_recent(&text)
}

pub fn parse_recent(text: &str) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = Vec::new();
    for line in text.lines().rev() {
        let Some((when, host)) = line.split_once('\t') else { continue };
        let (Ok(when), host) = (when.parse::<u64>(), host.trim()) else { continue };
        if (usable(host.trim_start_matches(|c: char| c != '@').trim_start_matches('@')) || usable(host)) && !out.iter().any(|(h, _)| h == host) {
            out.push((host.to_string(), when));
        }
        if out.len() >= 50 {
            break;
        }
    }
    out
}

// ------------------------------------------------------------ detection

/// The host in a typed command line: `ssh [options] [user@]host [cmd]`
/// (also `ssh.exe`, a full path, or behind `sudo`). None if it isn't ssh.
pub fn host_in_command(line: &str) -> Option<String> {
    // options that take a value
    const WITH_ARG: &str = "BbcDEeFIiJLlmOoPpQRSWw";
    let words: Vec<&str> = line.split_whitespace().collect();
    let mut i = 0;
    if words.first() == Some(&"sudo") {
        i = 1;
    }
    let prog = words.get(i)?;
    let base = prog.rsplit(['/', '\\']).next().unwrap_or(prog).to_ascii_lowercase();
    if base != "ssh" && base != "ssh.exe" {
        return None;
    }
    i += 1;
    let mut user: Option<&str> = None;
    while let Some(w) = words.get(i) {
        if *w == "--" {
            i += 1;
            break;
        }
        if let Some(opts) = w.strip_prefix('-') {
            // -p 22, -p22, -lroot, -4v ...
            let mut chars = opts.chars();
            while let Some(c) = chars.next() {
                if WITH_ARG.contains(c) {
                    let rest: String = chars.collect();
                    let val = if rest.is_empty() {
                        i += 1;
                        words.get(i).copied()
                    } else {
                        None
                    };
                    if c == 'l' {
                        user = val.or(if rest.is_empty() { None } else { Some(&w[2..]) });
                    }
                    break;
                }
            }
            i += 1;
            continue;
        }
        break;
    }
    let target = words.get(i)?.trim_matches(|c| c == '"' || c == '\'');
    let target = target.strip_prefix("ssh://").unwrap_or(target);
    let host_part = target.rsplit('@').next().unwrap_or(target);
    let host_part = host_part.split(':').next().unwrap_or(host_part);
    if !usable(host_part) {
        return None;
    }
    Some(match user {
        Some(u) if !target.contains('@') => format!("{u}@{host_part}"),
        _ => target.split(':').next().unwrap_or(target).to_string(),
    })
}

/// `*` and `?` glob, case-insensitive.
pub fn glob(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

/// The first rule whose pattern matches the host (with or without the
/// `user@`).
pub fn rule_for<'a>(host: &str, rules: &'a [crate::config::SshRule]) -> Option<&'a crate::config::SshRule> {
    let bare = host.rsplit('@').next().unwrap_or(host);
    rules.iter().find(|r| glob(&r.pattern, host) || glob(&r.pattern, bare))
}

/// The command that connects to `host` (ssh.exe ships with Windows).
pub fn command(host: &str) -> vibex_pty::Command {
    let mut cmd = vibex_pty::Command::new(if cfg!(windows) { "ssh.exe" } else { "ssh" });
    let mut target = host;
    if let Some(rest) = host.strip_prefix('[') {
        if let Some((h, port)) = rest.split_once("]:") {
            if port.parse::<u16>().is_ok() {
                cmd = cmd.arg("-p").arg(port);
                target = h;
            }
        }
    }
    // "--" so a host name can never be read as an option
    cmd.arg("--").arg(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_hosts() {
        let cfg = "# comment\nHost pi devbox\n  HostName 10.0.0.2\nHost *.internal !bad\nhost=gpu\nInclude extra\nHost -oProxyCommand=calc\n";
        let mut out = Vec::new();
        parse_config(cfg, &mut |t| (t == "extra").then(|| "Host nas\nHost pi".to_string()), 0, &mut out);
        assert_eq!(out, ["pi", "devbox", "gpu", "nas"]);
    }

    #[test]
    fn detects_ssh_commands() {
        assert_eq!(host_in_command("ssh pi"), Some("pi".into()));
        assert_eq!(host_in_command("ssh -p 2222 me@box.lan uptime"), Some("me@box.lan".into()));
        assert_eq!(host_in_command("ssh -i ~/.ssh/k -o StrictHostKeyChecking=no prod-db"), Some("prod-db".into()));
        assert_eq!(host_in_command("ssh -l root -4v web1"), Some("root@web1".into()));
        assert_eq!(host_in_command("C:\\Windows\\System32\\OpenSSH\\ssh.exe -p22 gpu"), Some("gpu".into()));
        assert_eq!(host_in_command("sudo ssh admin@10.0.0.5"), Some("admin@10.0.0.5".into()));
        assert_eq!(host_in_command("ssh-keygen -t ed25519"), None);
        assert_eq!(host_in_command("echo ssh pi"), None);
        assert_eq!(host_in_command("ssh"), None);
    }

    #[test]
    fn globs_and_rules() {
        assert!(glob("*prod*", "eu-PROD-db"));
        assert!(glob("web?", "web1"));
        assert!(!glob("web?", "web10"));
        let rules = vec![
            crate::config::SshRule { pattern: "*prod*".into(), confirm: true, color: Some(1), label: Some("PROD".into()), theme: None },
            crate::config::SshRule { pattern: "pi".into(), confirm: false, color: Some(2), label: None, theme: None },
        ];
        assert_eq!(rule_for("root@prod-1", &rules).and_then(|r| r.color), Some(1));
        assert_eq!(rule_for("pi", &rules).and_then(|r| r.color), Some(2));
        assert!(rule_for("devbox", &rules).is_none());
    }

    #[test]
    fn recent_newest_first() {
        let r = parse_recent("1\tpi\n2\tprod-db\n3\tpi\nbad line\n");
        assert_eq!(r, vec![("pi".to_string(), 3), ("prod-db".to_string(), 2)]);
    }

    #[test]
    fn known_hosts() {
        let kh = "github.com,140.82.112.3 ssh-ed25519 AAAA\n|1|hashed= ssh-rsa AAAA\n[box.lan]:2222 ssh-ed25519 AAAA\n10.1.1.1 ssh-rsa AAA\n";
        let mut out = vec!["github.com".to_string()];
        parse_known_hosts(kh, &mut out);
        assert_eq!(out, ["github.com", "[box.lan]:2222", "10.1.1.1"]);
        let c = command("[box.lan]:2222");
        assert_eq!(c.args, ["-p", "2222", "--", "box.lan"]);
        assert_eq!(command("pi").args, ["--", "pi"]);
    }
}
