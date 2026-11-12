//! Command history across every pane, window and past session: each
//! command that finishes (shell integration reports it) is appended to
//! `~/.config/vibex/history.tsv`, one per line:
//!
//! `unix-seconds \t exit \t milliseconds \t cwd \t command`
//!
//! (exit / milliseconds empty when unknown; tabs, newlines and
//! backslashes in the text are escaped). Commands typed with a leading
//! space are left out, as in bash's `ignorespace`.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Rewrite the file down to this many lines once it passes `MAX_LINES`.
const KEEP_LINES: usize = 20_000;
const MAX_LINES: usize = 25_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub when: u64,
    pub exit: Option<i32>,
    pub duration: Option<Duration>,
    pub cwd: String,
    pub command: String,
}

pub fn path() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("history.tsv"))
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

fn unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            Some(c) => o.push(c),
            None => o.push('\\'),
        }
    }
    o
}

pub fn format_line(r: &Record) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}",
        r.when,
        r.exit.map(|e| e.to_string()).unwrap_or_default(),
        r.duration.map(|d| d.as_millis().to_string()).unwrap_or_default(),
        escape(&r.cwd),
        escape(&r.command)
    )
}

pub fn parse_line(line: &str) -> Option<Record> {
    let mut f = line.splitn(5, '\t');
    let when = f.next()?.parse().ok()?;
    let exit = f.next()?.parse().ok();
    let duration = f.next()?.parse::<u64>().ok().map(Duration::from_millis);
    let cwd = unescape(f.next()?);
    let command = unescape(f.next()?);
    (!command.trim().is_empty()).then_some(Record { when, exit, duration, cwd, command })
}

/// Should this command be kept?
pub fn worth_keeping(command: &str) -> bool {
    !command.trim().is_empty() && !command.starts_with(' ')
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Append one command (best effort: history never gets in the way).
pub fn append(r: &Record) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{}", format_line(r));
    }
}

/// Every recorded command, newest first, each command once (its latest run).
pub fn load() -> Vec<Record> {
    let Some(p) = path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&p) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > MAX_LINES {
        let keep = lines[lines.len() - KEEP_LINES..].join("\n") + "\n";
        let _ = std::fs::write(&p, keep);
    }
    dedup_newest_first(lines.iter().filter_map(|l| parse_line(l)).collect())
}

pub fn dedup_newest_first(mut all: Vec<Record>) -> Vec<Record> {
    all.reverse();
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert(r.command.trim().to_string()));
    all
}

/// "3m ago", "2h ago", "5d ago".
pub fn ago(when: u64, now: u64) -> String {
    let s = now.saturating_sub(when);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// The palette hint: exit status, how long it ran, where, when.
pub fn hint(r: &Record, now: u64) -> String {
    let mut parts = Vec::new();
    match r.exit {
        Some(0) => parts.push("✓".to_string()),
        Some(c) => parts.push(format!("✗{c}")),
        None => {}
    }
    if let Some(d) = r.duration {
        parts.push(crate::pane::format_duration(d));
    }
    if !r.cwd.is_empty() {
        let short: Vec<&str> = r.cwd.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
        parts.push(short.last().map_or(r.cwd.clone(), |s| s.to_string()));
    }
    parts.push(ago(r.when, now));
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_awkward_text() {
        let r = Record {
            when: 1_700_000_000,
            exit: Some(1),
            duration: Some(Duration::from_millis(2500)),
            cwd: "C:\\Users\\me\\code".into(),
            command: "echo \"a\tb\" \\ ; ls\nsecond".into(),
        };
        assert_eq!(parse_line(&format_line(&r)), Some(r));
        let unknown = Record {
            exit: None,
            duration: None,
            ..parse_line(&format_line(&Record { when: 5, exit: None, duration: None, cwd: String::new(), command: "x".into() })).unwrap()
        };
        assert_eq!(unknown.command, "x");
        assert_eq!(unknown.exit, None);
    }

    #[test]
    fn newest_first_and_once() {
        let mk = |w, c: &str| Record { when: w, exit: Some(0), duration: None, cwd: String::new(), command: c.into() };
        let got = dedup_newest_first(vec![mk(1, "ls"), mk(2, "git pull"), mk(3, "ls"), mk(4, "cargo test")]);
        let cmds: Vec<_> = got.iter().map(|r| (r.when, r.command.as_str())).collect();
        assert_eq!(cmds, vec![(4, "cargo test"), (3, "ls"), (2, "git pull")]);
    }

    #[test]
    fn leading_space_is_private() {
        assert!(worth_keeping("ls"));
        assert!(!worth_keeping(" export TOKEN=secret"));
        assert!(!worth_keeping("   "));
    }

    #[test]
    fn hints() {
        let r = Record { when: 100, exit: Some(2), duration: Some(Duration::from_secs(3)), cwd: "/home/me/proj".into(), command: "make".into() };
        let h = hint(&r, 100 + 7200);
        assert!(h.starts_with("✗2 · "), "{h}");
        assert!(h.contains("proj") && h.ends_with("2h ago"), "{h}");
    }
}
