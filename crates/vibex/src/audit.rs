//! A tamper-evident log of every command: `~/.config/vibex/audit.jsonl`,
//! one JSON object per line. Each line carries the SHA-256 of the line
//! before it (`prev`) and its own `hash` over everything else in it, so
//! editing, removing or reordering any line breaks the chain from there
//! on. `vibex audit verify` checks it; `vibex audit report` turns it into
//! an HTML report for an assessment write-up.
//!
//! Secrets are redacted before anything is written (see `secrets.rs`).

use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
/// Output kept per command (when `audit_output` is on).
const MAX_OUTPUT: usize = 64 * 1024;

pub fn path() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("audit.jsonl"))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub time: u64,
    pub machine: String,
    pub user: String,
    pub workspace: String,
    pub pane: u32,
    /// The SSH host, if the command ran over SSH.
    pub ssh: Option<String>,
    pub cwd: String,
    pub command: String,
    pub exit: Option<i32>,
    pub duration_ms: Option<u64>,
    pub output_lines: usize,
    pub output_sha256: String,
    pub output: Option<String>,
}

fn sha_hex(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn output_digest(output: &str) -> String {
    sha_hex(output.as_bytes())
}

fn js(s: &str) -> String {
    crate::record::json_string(s)
}

/// The line without its hash: fixed field order, so it hashes the same
/// way every time.
fn body(seq: u64, e: &Entry, prev: &str) -> String {
    let opt_i = |v: Option<i64>| v.map_or("null".to_string(), |v| v.to_string());
    format!(
        "{{\"seq\":{seq},\"time\":{},\"iso\":{},\"machine\":{},\"user\":{},\"workspace\":{},\"pane\":{},\"ssh\":{},\"cwd\":{},\"command\":{},\"exit\":{},\"duration_ms\":{},\"output_lines\":{},\"output_sha256\":{},\"output\":{},\"prev\":\"{prev}\"}}",
        e.time,
        js(&iso(e.time)),
        js(&e.machine),
        js(&e.user),
        js(&e.workspace),
        e.pane,
        e.ssh.as_deref().map_or("null".into(), js),
        js(&e.cwd),
        js(&e.command),
        opt_i(e.exit.map(i64::from)),
        opt_i(e.duration_ms.map(|d| d as i64)),
        e.output_lines,
        js(&e.output_sha256),
        e.output.as_deref().map_or("null".into(), js),
    )
}

/// `{...}` + its hash → the stored line.
fn seal(body: &str, prev: &str) -> String {
    let hash = sha_hex(format!("{prev}\n{body}").as_bytes());
    format!("{},\"hash\":\"{hash}\"}}", &body[..body.len() - 1])
}

/// Split a stored line into (body, hash).
fn unseal(line: &str) -> Option<(String, String)> {
    let at = line.rfind(",\"hash\":\"")?;
    let hash = line[at + 9..].strip_suffix("\"}")?;
    if hash.len() != 64 {
        return None;
    }
    Some((format!("{}}}", &line[..at]), hash.to_string()))
}

fn field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let k = format!("\"{key}\":");
    let at = body.find(&k)? + k.len();
    let rest = &body[at..];
    if let Some(r) = rest.strip_prefix('"') {
        // a string: up to the closing quote (not an escaped one)
        let mut esc = false;
        for (i, c) in r.char_indices() {
            match c {
                '\\' if !esc => esc = true,
                '"' if !esc => return Some(&rest[..i + 2]),
                _ => esc = false,
            }
        }
        None
    } else {
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        Some(&rest[..end])
    }
}

fn string_field(body: &str, key: &str) -> Option<String> {
    let raw = field(body, key)?;
    if raw == "null" {
        return None;
    }
    let c = crate::record::parse(&format!("{{\"width\":1,\"height\":1}}\n[0, \"o\", {raw}]\n")).ok()?;
    c.events.into_iter().next().map(|(_, s)| s)
}

/// The last line's (seq, hash), reading only the end of the file.
fn tail(path: &Path) -> (u64, String) {
    let Ok(mut f) = std::fs::File::open(path) else { return (0, GENESIS.into()) };
    let len = f.metadata().map_or(0, |m| m.len());
    let mut window: u64 = 256 * 1024;
    loop {
        let from = len.saturating_sub(window);
        let mut buf = Vec::new();
        if f.seek(SeekFrom::Start(from)).is_err() || f.read_to_end(&mut buf).is_err() {
            return (0, GENESIS.into());
        }
        // bytes, not text: the seek can land inside a character. From the
        // middle of the file the first line is cut, so start after it.
        let start = if from == 0 { Some(0) } else { buf.iter().position(|&b| b == b'\n').map(|i| i + 1) };
        let text = start.map(|s| String::from_utf8_lossy(&buf[s..]).into_owned()).unwrap_or_default();
        if let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) {
            return match unseal(last) {
                Some((b, h)) => (field(&b, "seq").and_then(|v| v.parse().ok()).unwrap_or(0), h),
                None => (0, GENESIS.into()),
            };
        }
        if from == 0 {
            return (0, GENESIS.into());
        }
        // the last line is longer than what was read: read more
        window = window.saturating_mul(4);
    }
}

/// Append one entry, chained to the last line.
pub fn append_to(p: &Path, e: &Entry) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let (seq, prev) = tail(p);
    let mut e = e.clone();
    if let Some(o) = e.output.as_mut() {
        if o.len() > MAX_OUTPUT {
            let mut cut = MAX_OUTPUT;
            while !o.is_char_boundary(cut) {
                cut -= 1;
            }
            o.truncate(cut);
            o.push_str("\n… (cut)");
        }
    }
    let line = seal(&body(seq + 1, &e, &prev), &prev);
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p).map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| e.to_string())
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Every line chains: how many, first and last times.
    Intact { entries: usize, first: u64, last: u64 },
    /// Broken at this (1-based) line, and why.
    Broken { line: usize, reason: String, good: usize },
}

pub fn verify(p: &Path) -> Result<Verdict, String> {
    let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(verify_text(&text))
}

pub fn verify_text(text: &str) -> Verdict {
    let mut prev = GENESIS.to_string();
    let (mut n, mut first, mut last) = (0usize, 0u64, 0u64);
    // a byte-order mark (some editors add one) isn't part of the log
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let broken = |reason: &str| Verdict::Broken { line: i + 1, reason: reason.into(), good: n };
        let Some((b, hash)) = unseal(line) else { return broken("not a sealed line") };
        if field(&b, "prev").map(|v| v.trim_matches('"')) != Some(prev.as_str()) {
            return broken("doesn't follow the line before (a line was removed, added or moved)");
        }
        if sha_hex(format!("{prev}\n{b}").as_bytes()) != hash {
            return broken("its contents were changed");
        }
        if field(&b, "seq").and_then(|v| v.parse::<usize>().ok()) != Some(n + 1) {
            return broken("out of sequence");
        }
        let t = field(&b, "time").and_then(|v| v.parse().ok()).unwrap_or(0);
        if n == 0 {
            first = t;
        }
        last = t;
        prev = hash;
        n += 1;
    }
    Verdict::Intact { entries: n, first, last }
}

/// "2026-11-12 14:30:00Z"
pub fn iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = era * 400 + yoe + i64::from(m <= 2);
    let r = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", r / 3600, r % 3600 / 60, r % 60)
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// An HTML report: whether the chain holds, then every command (from
/// `since`, unix seconds).
pub fn report(text: &str, since: u64) -> String {
    report_with(text, since, "Command audit report", "", "")
}

/// The report with its own title, a line under it, and sections (HTML)
/// before the command table.
pub fn report_with(text: &str, since: u64, title: &str, subtitle: &str, sections: &str) -> String {
    let verdict = verify_text(text);
    let mut rows = String::new();
    let mut count = 0;
    let mut hosts: Vec<String> = Vec::new();
    for line in text.lines() {
        let Some((b, hash)) = unseal(line) else { continue };
        let t: u64 = field(&b, "time").and_then(|v| v.parse().ok()).unwrap_or(0);
        if t < since {
            continue;
        }
        count += 1;
        let s = |k: &str| string_field(&b, k).unwrap_or_default();
        let machine = s("machine");
        let ssh = string_field(&b, "ssh");
        let host = ssh.clone().map_or(machine.clone(), |h| format!("{h} (ssh from {machine})"));
        if !hosts.contains(&host) {
            hosts.push(host.clone());
        }
        let exit = field(&b, "exit").unwrap_or("null");
        let exit_cell = match exit {
            "null" => "<td class=dim>–</td>".to_string(),
            "0" => "<td class=ok>0</td>".to_string(),
            e => format!("<td class=bad>{e}</td>"),
        };
        let dur = field(&b, "duration_ms")
            .and_then(|v| v.parse::<u64>().ok())
            .map_or("–".into(), |ms| crate::pane::format_duration(std::time::Duration::from_millis(ms)));
        let out = string_field(&b, "output")
            .map(|o| format!("<details><summary>output ({} lines)</summary><pre>{}</pre></details>", field(&b, "output_lines").unwrap_or("0"), esc_html(&o)))
            .unwrap_or_default();
        rows.push_str(&format!(
            "<tr><td class=num>{}</td><td class=time>{}</td><td>{}</td><td class=path>{}</td><td><code>{}</code>{out}</td>{exit_cell}<td class=num>{dur}</td><td class=hash title=\"{hash}\">{}</td></tr>\n",
            field(&b, "seq").unwrap_or(""),
            esc_html(&iso(t)),
            esc_html(&host),
            esc_html(&s("cwd")),
            esc_html(&s("command")),
            &hash[..12],
        ));
    }
    let status = match &verdict {
        Verdict::Intact { entries, first, last } => format!(
            "<div class=\"status good\">✓ Hash chain intact: {}, {} to {}</div>",
            self::entries(*entries),
            esc_html(&iso(*first)),
            esc_html(&iso(*last))
        ),
        Verdict::Broken { line, reason, good } => format!(
            "<div class=\"status broken\">✗ Hash chain broken at line {line}: {}. {} before it intact; nothing from line {line} on can be trusted.</div>",
            esc_html(reason),
            entries(*good)
        ),
    };
    let since_txt = if since > 0 { format!(" since {}", esc_html(&iso(since))) } else { String::new() };
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ --ink:#1d1d1f; --dim:#6e6e73; --line:#e3e3e6; --accent:#B4461E; --ok:#1f7a3a; --bad:#b3261e; --bg:#ffffff; --card:#f6f6f8; }}
@media (prefers-color-scheme: dark) {{ :root {{ --ink:#ececf1; --dim:#9a9aa3; --line:#34343a; --bg:#151517; --card:#1e1e22; --ok:#5cc47a; --bad:#ff6b5e; }} }}
body {{ background:var(--bg); color:var(--ink); font:14px/1.45 system-ui,-apple-system,"Segoe UI",sans-serif; margin:0; padding:32px 24px; }}
main {{ max-width:1200px; margin:0 auto; }}
h1 {{ color:var(--accent); font-size:24px; margin:0 0 4px; }}
.meta {{ color:var(--dim); margin:0 0 18px; }}
.status {{ padding:12px 14px; border-radius:8px; margin:0 0 20px; font-weight:600; }}
.good {{ background:color-mix(in srgb, var(--ok) 14%, transparent); color:var(--ok); }}
.broken {{ background:color-mix(in srgb, var(--bad) 14%, transparent); color:var(--bad); }}
.wrap {{ overflow-x:auto; border:1px solid var(--line); border-radius:8px; }}
table {{ border-collapse:collapse; width:100%; }}
th, td {{ text-align:left; padding:7px 10px; border-bottom:1px solid var(--line); vertical-align:top; }}
th {{ background:var(--card); font-weight:600; position:sticky; top:0; }}
td.num {{ text-align:right; white-space:nowrap; font-variant-numeric:tabular-nums; }}
td.time, td.hash {{ white-space:nowrap; font-variant-numeric:tabular-nums; color:var(--dim); }}
td.hash {{ font-family:ui-monospace,Consolas,monospace; font-size:12px; }}
td.path {{ font-family:ui-monospace,Consolas,monospace; font-size:12px; color:var(--dim); word-break:break-all; }}
td.ok {{ color:var(--ok); }} td.bad {{ color:var(--bad); font-weight:600; }} td.dim {{ color:var(--dim); }}
code, pre {{ font-family:ui-monospace,Consolas,monospace; font-size:13px; }}
code {{ white-space:pre-wrap; word-break:break-word; }}
pre {{ background:var(--card); padding:10px; border-radius:6px; overflow:auto; max-height:360px; margin:6px 0 0; }}
summary {{ color:var(--dim); cursor:pointer; margin-top:4px; font-size:12px; }}
.foot {{ color:var(--dim); font-size:12px; margin-top:16px; }}
h2 {{ font-size:16px; margin:26px 0 10px; }}
.chips {{ display:flex; flex-wrap:wrap; gap:6px; }}
.chip {{ background:var(--card); border:1px solid var(--line); border-radius:6px; padding:2px 8px; }}
ul {{ margin:6px 0; padding-left:20px; }}
</style></head><body><main>
<h1>{title}</h1>
<p class="meta">{subtitle}{count} commands{since_txt} · hosts: {hosts} · generated {gen}</p>
{status}
{sections}
<div class="wrap"><table>
<thead><tr><th>#</th><th>When (UTC)</th><th>Host</th><th>Folder</th><th>Command</th><th>Exit</th><th>Took</th><th>Hash</th></tr></thead>
<tbody>
{rows}</tbody></table></div>
<p class="foot">Each entry stores the SHA-256 of the one before it and of its own contents, so any edit, deletion or reordering shows up when the log is verified. Secrets (API keys, tokens, private keys, passwords) were redacted before they were written.</p>
</main></body></html>
"#,
        title = esc_html(title),
        subtitle = if subtitle.is_empty() { String::new() } else { format!("{} · ", esc_html(subtitle)) },
        hosts = esc_html(&hosts.join(", ")),
        gen = esc_html(&iso(crate::history::now())),
    )
}

/// "1 entry", "3 entries".
pub fn entries(n: usize) -> String {
    if n == 1 {
        "1 entry".into()
    } else {
        format!("{n} entries")
    }
}

/// The machine and user names for an entry.
pub fn who() -> (String, String) {
    let machine = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "this machine".into());
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    (machine, user)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(cmd: &str, t: u64) -> Entry {
        Entry {
            time: t,
            machine: "box".into(),
            user: "me".into(),
            workspace: "main".into(),
            pane: 1,
            ssh: None,
            cwd: "C:\\work".into(),
            command: cmd.into(),
            exit: Some(0),
            duration_ms: Some(1200),
            output_lines: 1,
            output_sha256: output_digest("ok"),
            output: Some("ok \"quoted\"".into()),
        }
    }

    fn sample() -> (tempdir::Dir, PathBuf) {
        let d = tempdir::Dir::new();
        let p = d.0.join("audit.jsonl");
        append_to(&p, &entry("nmap -sV 10.0.0.5", 100)).unwrap();
        append_to(&p, &entry("ssh admin@10.0.0.5", 200)).unwrap();
        append_to(&p, &Entry { exit: Some(1), ssh: Some("admin@10.0.0.5".into()), ..entry("cat /etc/shadow", 300) }).unwrap();
        (d, p)
    }

    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
                let p = std::env::temp_dir().join(format!("vibex-audit-test-{n}-{:?}", std::thread::current().id()).replace(['(', ')'], ""));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn a_big_log_with_non_ascii_keeps_its_chain() {
        let d = tempdir::Dir::new();
        let p = d.0.join("audit.jsonl");
        // grow the log past what tail() reads, until the place it seeks to
        // is in the middle of a character (then add one more entry)
        let mut n = 0;
        for k in 0.. {
            let out = format!("{}{}", "x".repeat(k % 3), "日本語 é ".repeat(2000));
            append_to(&p, &Entry { output: Some(out), ..entry("cat 日記.txt", 100 + k as u64) }).unwrap();
            n += 1;
            let bytes = std::fs::read(&p).unwrap();
            if bytes.len() > 300 * 1024 {
                let at = bytes.len() - 256 * 1024;
                if bytes[at] & 0xc0 == 0x80 {
                    break;
                }
            }
            assert!(k < 200, "never landed inside a character");
        }
        append_to(&p, &entry("whoami", 9999)).unwrap();
        n += 1;
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(matches!(verify_text(&text), Verdict::Intact { entries, .. } if entries == n), "{:?}", verify_text(&text));
        assert_eq!(tail(&p).0, n as u64);
        // a last line longer than the window is still found
        let big = Entry { output: Some("é".repeat(MAX_OUTPUT)), command: "ö".repeat(300 * 1024), ..entry("x", 1) };
        append_to(&p, &big).unwrap();
        append_to(&p, &entry("after", 2)).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(matches!(verify_text(&text), Verdict::Intact { entries, .. } if entries == n + 2), "{:?}", verify_text(&text));
    }

    #[test]
    fn chain_holds_and_breaks() {
        let (_d, p) = sample();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(verify_text(&text), Verdict::Intact { entries: 3, first: 100, last: 300 });
        // edit a command
        let edited = text.replacen("cat /etc/shadow", "cat /etc/hosts", 1);
        assert!(matches!(verify_text(&edited), Verdict::Broken { line: 3, good: 2, .. }), "{:?}", verify_text(&edited));
        // remove a line
        let lines: Vec<&str> = text.lines().collect();
        let removed = format!("{}\n{}\n", lines[0], lines[2]);
        assert!(matches!(verify_text(&removed), Verdict::Broken { line: 2, good: 1, .. }));
        // swap two lines
        let swapped = format!("{}\n{}\n{}\n", lines[1], lines[0], lines[2]);
        assert!(matches!(verify_text(&swapped), Verdict::Broken { line: 1, good: 0, .. }));
        // re-sealing an edited line doesn't help: the next line's prev breaks
        let (b, _) = unseal(lines[1]).unwrap();
        let b2 = b.replacen("ssh admin@10.0.0.5", "ls", 1);
        let prev = field(&b2, "prev").unwrap().trim_matches('"').to_string();
        let forged = format!("{}\n{}\n{}\n", lines[0], seal(&b2, &prev), lines[2]);
        assert!(matches!(verify_text(&forged), Verdict::Broken { line: 3, .. }));
    }

    #[test]
    fn report_lists_and_escapes() {
        let (_d, p) = sample();
        let text = std::fs::read_to_string(&p).unwrap();
        let html = report(&text, 0);
        assert!(html.contains("Hash chain intact: 3 entries"));
        assert!(html.contains("nmap -sV 10.0.0.5"));
        assert!(html.contains("admin@10.0.0.5 (ssh from box)"));
        assert!(html.contains("ok &quot;quoted&quot;"));
        assert!(html.contains("<td class=bad>1</td>"));
        assert!(!report(&text, 250).contains("nmap"));
        let broken = report(&text.replacen("nmap", "curl", 1), 0);
        assert!(broken.contains("broken at line 1"));
    }

    #[test]
    fn dates_and_fields() {
        assert_eq!(iso(0), "1970-01-01 00:00:00Z");
        assert_eq!(iso(1_794_460_894), "2026-11-12 05:21:34Z");
        let b = "{\"seq\":3,\"command\":\"a \\\"b\\\" c\",\"exit\":null}";
        assert_eq!(field(b, "seq"), Some("3"));
        assert_eq!(string_field(b, "command").as_deref(), Some("a \"b\" c"));
        assert_eq!(field(b, "exit"), Some("null"));
    }
}
