//! Engagement mode, for assessments and pentests: one switch that keeps a
//! hash-chained log of every command with its output, can record every
//! pane, warns before a network tool touches something outside the scope,
//! and collects loot (hashes, credentials, open ports) from what scrolls
//! past. Everything lives in `~/.config/vibex/engagements/<name>/`:
//!
//! ```text
//! engagement.txt   name, when it started, scope (one entry per line)
//! audit.jsonl      the command log (see audit.rs)
//! loot.tsv         what was found: time, kind, host, value, command
//! recordings/      .cast files (when "record every pane" is on)
//! report.html      written by "Export engagement report"
//! ```

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub fn root() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("engagements"))
}

/// A folder-safe engagement name.
pub fn slug(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(60).collect();
    s.trim_matches('-').to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Engagement {
    pub name: String,
    pub dir: PathBuf,
    pub started: u64,
    pub scope: Scope,
}

impl Engagement {
    pub fn audit_path(&self) -> PathBuf {
        self.dir.join("audit.jsonl")
    }
    pub fn loot_path(&self) -> PathBuf {
        self.dir.join("loot.tsv")
    }
    pub fn recordings(&self) -> PathBuf {
        self.dir.join("recordings")
    }

    /// Open (or create) the engagement called `name`.
    pub fn open(name: &str) -> Result<Engagement, String> {
        let slug = slug(name);
        if slug.is_empty() {
            return Err("give the engagement a name".into());
        }
        let dir = root().ok_or("no config folder")?.join(&slug);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let file = dir.join("engagement.txt");
        match std::fs::read_to_string(&file) {
            Ok(text) => Ok(parse_file(&text, &slug, dir)),
            Err(_) => {
                let e = Engagement { name: slug, dir, started: crate::history::now(), scope: Scope::default() };
                e.save()?;
                Ok(e)
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let mut s = format!("# Vibex engagement\nname = {}\nstarted = {}\n", self.name, self.started);
        for r in &self.scope.text {
            s.push_str(&format!("scope = {r}\n"));
        }
        std::fs::write(self.dir.join("engagement.txt"), s).map_err(|e| e.to_string())
    }

    pub fn set_scope(&mut self, text: &str) -> Result<(), String> {
        self.scope = Scope::parse(text)?;
        self.save()
    }
}

fn parse_file(text: &str, slug: &str, dir: PathBuf) -> Engagement {
    let mut e = Engagement { name: slug.to_string(), dir, started: 0, scope: Scope::default() };
    let mut scope = Vec::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "name" => e.name = v.trim().to_string(),
            "started" => e.started = v.trim().parse().unwrap_or(0),
            "scope" => scope.push(v.trim().to_string()),
            _ => {}
        }
    }
    e.scope = Scope::parse(&scope.join("\n")).unwrap_or_default();
    e
}

/// Every engagement, newest first.
pub fn list() -> Vec<String> {
    let Some(r) = root() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(r) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, String)> = rd
        .flatten()
        .filter(|e| e.path().join("engagement.txt").is_file())
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.file_name().to_string_lossy().into_owned())))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, n)| n).collect()
}

// ------------------------------------------------------------ scope

#[derive(Clone, Debug, PartialEq)]
enum Rule {
    /// network, mask
    Net(u32, u32),
    /// first..=last address
    Range(u32, u32),
    /// a domain and everything under it
    Domain(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    include: Vec<Rule>,
    exclude: Vec<Rule>,
    /// As written (for showing and saving).
    pub text: Vec<String>,
}

fn ipv4(s: &str) -> Option<u32> {
    let p: Vec<&str> = s.split('.').collect();
    if p.len() != 4 {
        return None;
    }
    let mut v = 0u32;
    for x in p {
        if x.is_empty() || x.len() > 3 || !x.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        v = (v << 8) | u32::from(x.parse::<u8>().ok()?);
    }
    Some(v)
}

fn parse_rule(s: &str) -> Result<Rule, String> {
    let s = s.trim().trim_start_matches("*.").to_lowercase();
    if let Some((ip, bits)) = s.split_once('/') {
        let ip = ipv4(ip).ok_or_else(|| format!("'{s}' isn't an address range"))?;
        let bits: u32 = bits.parse().ok().filter(|b| *b <= 32).ok_or_else(|| format!("'{s}': bad prefix length"))?;
        let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
        return Ok(Rule::Net(ip & mask, mask));
    }
    if let Some((a, b)) = s.split_once('-') {
        if let Some(first) = ipv4(a) {
            // 10.0.0.1-50 or 10.0.0.1-10.0.0.50
            let last = ipv4(b).or_else(|| b.parse::<u8>().ok().map(|n| (first & 0xffff_ff00) | u32::from(n))).ok_or_else(|| format!("'{s}' isn't a range"))?;
            return Ok(Rule::Range(first.min(last), first.max(last)));
        }
    }
    if let Some(ip) = ipv4(&s) {
        return Ok(Rule::Net(ip, u32::MAX));
    }
    if s.contains('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Ok(Rule::Domain(s.trim_matches('.').to_string()));
    }
    Err(format!("'{s}' isn't an address, range or domain"))
}

impl Rule {
    fn covers(&self, t: &Target) -> bool {
        match (self, t) {
            (Rule::Net(n, m), Target::V4(ip, tm)) => tm & m == *m && ip & m == *n,
            (Rule::Range(a, b), Target::V4(ip, tm)) => {
                let last = ip | !tm;
                *ip >= *a && last <= *b
            }
            (Rule::Domain(d), Target::Host(h)) => h == d || h.ends_with(&format!(".{d}")),
            _ => false,
        }
    }
}

impl Scope {
    /// Entries split by commas, spaces or lines; `!x` or `-x` leaves x out.
    pub fn parse(text: &str) -> Result<Scope, String> {
        let mut s = Scope::default();
        for raw in text.split(|c: char| c == ',' || c == '\n' || c == ';' || c.is_whitespace()) {
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            match raw.strip_prefix('!').or_else(|| raw.strip_prefix('-').filter(|r| !r.is_empty())) {
                Some(x) => s.exclude.push(parse_rule(x)?),
                None => s.include.push(parse_rule(raw)?),
            }
            s.text.push(raw.to_string());
        }
        Ok(s)
    }

    pub fn is_empty(&self) -> bool {
        self.include.is_empty()
    }

    /// In scope? (Nothing set: everything is.)
    pub fn allows(&self, t: &Target) -> bool {
        if self.exclude.iter().any(|r| r.covers(t)) {
            return false;
        }
        self.include.is_empty() || self.include.iter().any(|r| r.covers(t))
    }
}

// ------------------------------------------------------------ targets

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// address and mask (a /24 typed as a target is the whole range)
    V4(u32, u32),
    Host(String),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Target::V4(ip, m) => {
                let o = ip.to_be_bytes();
                write!(f, "{}.{}.{}.{}", o[0], o[1], o[2], o[3])?;
                if *m != u32::MAX {
                    write!(f, "/{}", m.count_ones())?;
                }
                Ok(())
            }
            Target::Host(h) => write!(f, "{h}"),
        }
    }
}

/// Tools that reach out to a host. Only their commands are checked.
const NETWORK_TOOLS: &[&str] = &[
    "nmap",
    "masscan",
    "rustscan",
    "naabu",
    "zmap",
    "ping",
    "ping6",
    "traceroute",
    "tracert",
    "mtr",
    "nc",
    "ncat",
    "netcat",
    "socat",
    "telnet",
    "curl",
    "wget",
    "httpx",
    "http",
    "nuclei",
    "nikto",
    "whatweb",
    "wpscan",
    "sqlmap",
    "gobuster",
    "ffuf",
    "feroxbuster",
    "dirb",
    "dirsearch",
    "wfuzz",
    "hydra",
    "medusa",
    "ncrack",
    "patator",
    "crackmapexec",
    "cme",
    "netexec",
    "nxc",
    "smbclient",
    "smbmap",
    "enum4linux",
    "enum4linux-ng",
    "rpcclient",
    "ldapsearch",
    "evil-winrm",
    "xfreerdp",
    "rdesktop",
    "mstsc",
    "ssh",
    "scp",
    "sftp",
    "ftp",
    "tftp",
    "snmpwalk",
    "onesixtyone",
    "dig",
    "nslookup",
    "host",
    "whois",
    "dnsrecon",
    "dnsenum",
    "fierce",
    "sslscan",
    "sslyze",
    "testssl",
    "testssl.sh",
    "openssl",
    "responder",
    "bloodhound-python",
    "kerbrute",
    "getuserspns",
    "getnpusers",
    "secretsdump",
    "psexec",
    "wmiexec",
    "smbexec",
    "atexec",
    "dcomexec",
    "test-netconnection",
    "tnc",
    "invoke-webrequest",
    "iwr",
    "invoke-restmethod",
    "irm",
    "msfconsole",
    "searchsploit",
];

/// Flags whose value is a file, not a target.
const FILE_FLAGS: &[&str] =
    &["-o", "-oN", "-oX", "-oG", "-oA", "-oS", "-oJ", "-w", "-W", "-iL", "-L", "-P", "-U", "-C", "--output", "--wordlist", "-x", "--outfile", "-c", "-H"];

const FILE_EXTS: &[&str] = &[
    "txt", "lst", "xml", "json", "csv", "html", "htm", "gnmap", "nmap", "log", "py", "sh", "ps1", "exe", "out", "conf", "cfg", "yaml", "yml", "zip", "gz",
    "pcap", "md", "rules", "dic", "kirbi", "ccache", "hash", "hashes", "pem", "key", "crt",
];

fn tool_name(word: &str) -> String {
    let w = word.trim_matches(|c| c == '"' || c == '\'' || c == '&' || c == '(');
    let base = w.rsplit(['/', '\\']).next().unwrap_or(w).to_lowercase();
    let base = base.strip_suffix(".exe").unwrap_or(&base).to_string();
    // impacket-secretsdump, secretsdump.py
    let base = base.strip_prefix("impacket-").unwrap_or(&base).to_string();
    base.strip_suffix(".py").unwrap_or(&base).to_string()
}

fn as_target(tok: &str) -> Option<Target> {
    let t = tok.trim_matches(|c| c == '"' || c == '\'' || c == ',' || c == ';' || c == '(' || c == ')');
    // URLs: scheme://[user@]host[:port]/...
    let t = match t.split_once("://") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or(rest),
        None => t,
    };
    let t = t.rsplit('@').next().unwrap_or(t);
    // host:port (not an IPv6 address)
    let t = if t.matches(':').count() == 1 { t.split(':').next().unwrap_or(t) } else { t };
    // an address range: 10.0.0.0/24
    if let Some((ip, bits)) = t.split_once('/') {
        if let (Some(ip), Ok(bits)) = (ipv4(ip), bits.parse::<u32>()) {
            if bits > 32 {
                return None;
            }
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
            return Some(Target::V4(ip & mask, mask));
        }
    }
    // smb shares: //host/share, \\host\share
    let t = t.trim_start_matches(['/', '\\']).split(['/', '\\']).next().unwrap_or(t);
    if t.is_empty() {
        return None;
    }
    if let Some((a, b)) = t.split_once('-') {
        if let (Some(first), Ok(n)) = (ipv4(a), b.parse::<u8>()) {
            // 10.0.0.1-20: check the last address too, by its own /32
            let _ = n;
            return Some(Target::V4(first, u32::MAX));
        }
    }
    if let Some(ip) = ipv4(t) {
        return Some(Target::V4(ip, u32::MAX));
    }
    let lower = t.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    let label_ok = lower.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    if lower.contains('.') && label_ok && !FILE_EXTS.contains(&ext) && ext.chars().all(|c| c.is_ascii_alphabetic()) && ext.len() >= 2 {
        return Some(Target::Host(lower));
    }
    None
}

/// The hosts / addresses a command will touch, if it's a network tool.
pub fn targets(command: &str) -> Vec<Target> {
    let words: Vec<&str> = command.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() && matches!(tool_name(words[i]).as_str(), "sudo" | "proxychains" | "proxychains4" | "time" | "doas") {
        i += 1;
    }
    let Some(first) = words.get(i) else { return Vec::new() };
    if !NETWORK_TOOLS.contains(&tool_name(first).as_str()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut skip = false;
    for w in &words[i + 1..] {
        if skip {
            skip = false;
            continue;
        }
        if FILE_FLAGS.contains(w) {
            skip = true;
            continue;
        }
        if w.starts_with('-') && !w.contains("://") {
            // --url=http://x: look at the value
            if let Some((_, v)) = w.split_once('=') {
                if let Some(t) = as_target(v) {
                    out.push(t);
                }
            }
            continue;
        }
        if let Some(t) = as_target(w) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

// ------------------------------------------------------------ loot

#[derive(Clone, Debug, PartialEq)]
pub struct Loot {
    pub kind: String,
    pub host: String,
    pub value: String,
}

struct LootRules {
    pwdump: Regex,
    netntlm: Regex,
    kerberos: Regex,
    crypt: Regex,
    hydra: Regex,
    cme: Regex,
    nmap_host: Regex,
    nmap_port: Regex,
}

fn rules() -> &'static LootRules {
    static R: OnceLock<LootRules> = OnceLock::new();
    R.get_or_init(|| LootRules {
        pwdump: Regex::new(r"(?:^|\s)([^\s:]{1,128}):\d+:([a-fA-F0-9]{32}):([a-fA-F0-9]{32}):::").unwrap(),
        netntlm: Regex::new(r"\b([^\s:]{1,64}::[^\s:]{1,64}:[a-fA-F0-9]{16}:[a-fA-F0-9]{32}:[a-fA-F0-9]{40,})").unwrap(),
        kerberos: Regex::new(r"\$krb5(?:tgs|asrep|pa)\$[^\s]{20,}").unwrap(),
        crypt: Regex::new(r"\$(?:2[aby]|6|5|y)\$[A-Za-z0-9./$=,]{20,}").unwrap(),
        hydra: Regex::new(r"host:\s*(\S+)\s+login:\s*(\S+)\s+password:\s*(\S.*?)\s*$").unwrap(),
        cme: Regex::new(r"\[\+\]\s+([^\s\\]+\\[^\s:]+):(\S+)").unwrap(),
        nmap_host: Regex::new(r"^Nmap scan report for (\S+)(?: \(([\d.]+)\))?").unwrap(),
        nmap_port: Regex::new(r"^(\d{1,5})/(tcp|udp)\s+open\s+(\S+)(?:\s+(.*?))?\s*$").unwrap(),
    })
}

/// What a line of output gives away. `host` is the host this output is
/// about (kept between lines, e.g. nmap's "scan report for").
pub fn loot_in(line: &str, host: &mut String, guard: &crate::secrets::Guard) -> Vec<Loot> {
    let r = rules();
    let mut out = Vec::new();
    let line = line.trim_end();
    if let Some(c) = r.nmap_host.captures(line) {
        *host = c.get(2).or(c.get(1)).map_or(String::new(), |m| m.as_str().to_string());
        return out;
    }
    let h = host.clone();
    let loot = |kind: &str, host: &str, value: String| Loot { kind: kind.into(), host: host.into(), value };
    if let Some(c) = r.nmap_port.captures(line) {
        let svc = c.get(4).map_or(String::new(), |m| format!(" {}", m.as_str()));
        out.push(loot("open port", &h, format!("{}/{} {}{svc}", &c[1], &c[2], &c[3])));
        return out;
    }
    for c in r.pwdump.captures_iter(line) {
        out.push(loot("NTLM hash", &h, format!("{}:{}:{}", &c[1], &c[2], &c[3])));
    }
    for c in r.netntlm.captures_iter(line) {
        out.push(loot("NetNTLMv2 hash", &h, c[1].to_string()));
    }
    for m in r.kerberos.find_iter(line) {
        out.push(loot("Kerberos hash", &h, m.as_str().to_string()));
    }
    for m in r.crypt.find_iter(line) {
        out.push(loot("password hash", &h, m.as_str().to_string()));
    }
    if let Some(c) = r.hydra.captures(line) {
        out.push(loot("credential", &c[1], format!("{}:{}", &c[2], &c[3])));
    }
    if let Some(c) = r.cme.captures(line) {
        let pass = c[2].trim_end_matches("(Pwn3d!)").to_string();
        out.push(loot("credential", &h, format!("{}:{pass}", &c[1])));
    }
    // keys and tokens, when the line wasn't already one of the above
    if out.is_empty() {
        for (kind, re) in &guard.rules {
            if kind == "password / secret" {
                continue; // too loose to call it loot
            }
            for m in re.find_iter(line) {
                out.push(loot(kind, &h, m.as_str().to_string()));
            }
        }
    }
    out
}

/// `ghp_…0123`: enough to recognise, not enough to use.
pub fn mask(v: &str) -> String {
    let c: Vec<char> = v.chars().collect();
    if c.len() <= 8 {
        return "•".repeat(c.len());
    }
    let keep = (c.len() / 6).clamp(2, 6);
    format!("{}{}{}", c[..keep].iter().collect::<String>(), "•".repeat(6), c[c.len() - keep..].iter().collect::<String>())
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// Add loot (once per kind + value). Returns how many were new.
pub fn record_loot(path: &Path, found: &[Loot], command: &str) -> usize {
    use std::io::Write;
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut seen: std::collections::HashSet<(String, String)> = existing
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 4).then(|| (f[1].to_string(), f[3].to_string()))
        })
        .collect();
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return 0 };
    let mut n = 0;
    for l in found {
        if seen.insert((l.kind.clone(), clean(&l.value))) {
            let _ = writeln!(f, "{}\t{}\t{}\t{}\t{}", crate::history::now(), clean(&l.kind), clean(&l.host), clean(&l.value), clean(command));
            n += 1;
        }
    }
    n
}

/// Everything found so far: (time, loot, command), newest first.
pub fn read_loot(path: &Path) -> Vec<(u64, Loot, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut v: Vec<(u64, Loot, String)> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            (f.len() >= 4)
                .then(|| (f[0].parse().unwrap_or(0), Loot { kind: f[1].into(), host: f[2].into(), value: f[3].into() }, f.get(4).unwrap_or(&"").to_string()))
        })
        .collect();
    v.reverse();
    v
}

/// The engagement report: scope, loot (masked), recordings, then the
/// command log with its verification.
pub fn report(e: &Engagement) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut extra = String::new();
    extra.push_str("<h2>Scope</h2>");
    if e.scope.text.is_empty() {
        extra.push_str("<p class=meta>No scope was set.</p>");
    } else {
        extra.push_str("<p class=chips>");
        for s in &e.scope.text {
            extra.push_str(&format!("<code class=chip>{}</code> ", esc(s)));
        }
        extra.push_str("</p>");
    }
    let loot = read_loot(&e.loot_path());
    extra.push_str(&format!("<h2>Findings collected ({})</h2>", loot.len()));
    if loot.is_empty() {
        extra.push_str("<p class=meta>Nothing collected.</p>");
    } else {
        extra.push_str("<div class=wrap><table><thead><tr><th>When (UTC)</th><th>Kind</th><th>Host</th><th>Value</th><th>From</th></tr></thead><tbody>");
        for (t, l, cmd) in loot.iter().rev() {
            let value = if l.kind == "open port" { l.value.clone() } else { mask(&l.value) };
            extra.push_str(&format!(
                "<tr><td class=time>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td><code>{}</code></td></tr>",
                esc(&crate::audit::iso(*t)),
                esc(&l.kind),
                esc(&l.host),
                esc(&value),
                esc(&cmd.chars().take(80).collect::<String>())
            ));
        }
        extra.push_str("</tbody></table></div><p class=meta>Secrets and hashes are masked here; the full values are in loot.tsv in the engagement folder.</p>");
    }
    let casts: Vec<String> = std::fs::read_dir(e.recordings())
        .map(|rd| rd.flatten().map(|f| f.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".cast")).collect())
        .unwrap_or_default();
    if !casts.is_empty() {
        extra.push_str(&format!("<h2>Recordings ({})</h2><ul>", casts.len()));
        for c in &casts {
            extra.push_str(&format!("<li><code>recordings/{}</code></li>", esc(c)));
        }
        extra.push_str("</ul><p class=meta>Play with <code>vibex cli play FILE</code>, or make a GIF with <code>vibex cast2gif FILE</code>.</p>");
    }
    extra.push_str("<h2>Commands</h2>");
    let text = std::fs::read_to_string(e.audit_path()).unwrap_or_default();
    crate::audit::report_with(&text, 0, &format!("Engagement: {}", e.name), &format!("started {}", crate::audit::iso(e.started)), &extra)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_rules() {
        let s = Scope::parse("10.0.0.0/24, 192.168.1.10-20\n*.acme.com example.org !10.0.0.1 !vpn.acme.com").unwrap();
        let t = |x: &str| as_target(x).unwrap();
        assert!(s.allows(&t("10.0.0.5")));
        assert!(!s.allows(&t("10.0.0.1")), "excluded");
        assert!(!s.allows(&t("10.0.1.5")));
        assert!(s.allows(&t("10.0.0.0/25")), "a sub-range is inside");
        assert!(!s.allows(&t("10.0.0.0/16")), "a bigger range isn't");
        assert!(s.allows(&t("192.168.1.15")) && !s.allows(&t("192.168.1.21")));
        assert!(s.allows(&t("www.acme.com")) && s.allows(&t("acme.com")) && !s.allows(&t("vpn.acme.com")));
        assert!(s.allows(&t("https://api.example.org:8443/v1")));
        assert!(!s.allows(&t("evil.com")));
        assert!(Scope::default().allows(&t("anything.com")), "no scope: nothing is blocked");
        assert!(Scope::parse("10.0.0.0/40").is_err());
        assert!(Scope::parse("not a thing!").is_err());
    }

    #[test]
    fn finds_targets_in_commands() {
        let s = |c: &str| targets(c).iter().map(|t| t.to_string()).collect::<Vec<_>>();
        assert_eq!(s("nmap -sV -p- -oA scans/full 10.0.0.0/24"), vec!["10.0.0.0/24"]);
        assert_eq!(s("sudo nmap -iL hosts.txt 10.0.0.7"), vec!["10.0.0.7"]);
        assert_eq!(s("curl -sk https://portal.acme.com/login -o out.html"), vec!["portal.acme.com"]);
        assert_eq!(s("ssh admin@10.0.0.9 -p 2222"), vec!["10.0.0.9"]);
        assert_eq!(s("smbclient //10.0.0.4/C$ -U guest"), vec!["10.0.0.4"]);
        assert_eq!(s("impacket-secretsdump acme/admin@dc01.acme.local"), vec!["dc01.acme.local"]);
        assert_eq!(s("sqlmap --url=http://shop.acme.com/item?id=1 --batch"), vec!["shop.acme.com"]);
        assert_eq!(s("gobuster dir -u http://10.0.0.5 -w /usr/share/wordlists/dirb/common.txt"), vec!["10.0.0.5"]);
        assert!(s("cat notes.txt").is_empty(), "not a network tool");
        assert!(s("git push origin main").is_empty());
        assert!(s("python3 exploit.py 10.0.0.5").is_empty());
        assert!(s("nmap -oN results.nmap localhost").is_empty(), "localhost has no dot");
    }

    #[test]
    fn collects_loot() {
        let g = crate::secrets::Guard::default();
        let mut host = String::new();
        assert!(loot_in("Nmap scan report for dc01.acme.local (10.0.0.4)", &mut host, &g).is_empty());
        assert_eq!(host, "10.0.0.4");
        let l = loot_in("445/tcp open  microsoft-ds Windows Server 2019", &mut host, &g);
        assert_eq!(l, vec![Loot { kind: "open port".into(), host: "10.0.0.4".into(), value: "445/tcp microsoft-ds Windows Server 2019".into() }]);
        let l = loot_in("ACME\\Administrator:500:aad3b435b51404eeaad3b435b51404ee:31d6cfe0d16ae931b73c59d7e0c089c0:::", &mut host, &g);
        assert_eq!(l[0].kind, "NTLM hash");
        assert!(l[0].value.ends_with("31d6cfe0d16ae931b73c59d7e0c089c0"));
        let l = loot_in("[445][smb] host: 10.0.0.7   login: backup   password: Summer2026!", &mut host, &g);
        assert_eq!((l[0].kind.as_str(), l[0].host.as_str(), l[0].value.as_str()), ("credential", "10.0.0.7", "backup:Summer2026!"));
        let l = loot_in("SMB  10.0.0.4  445  DC01  [+] ACME\\svc_sql:P@ssw0rd (Pwn3d!)", &mut host, &g);
        assert_eq!(l[0].value, "ACME\\svc_sql:P@ssw0rd");
        let l = loot_in("$krb5tgs$23$*svc_sql$ACME.LOCAL$acme.local/svc_sql*$aa11bb22cc33dd44ee55ff66$0011223344", &mut host, &g);
        assert_eq!(l[0].kind, "Kerberos hash");
        let l = loot_in(&["found ", "AKIA", "IOSFODNN7EXAMPLE", " in config"].concat(), &mut host, &g);
        assert_eq!(l[0].kind, "AWS access key");
        assert!(loot_in("Starting Nmap 7.95", &mut host, &g).is_empty());
    }

    #[test]
    fn loot_is_kept_once() {
        let dir = std::env::temp_dir().join(format!("vibex-loot-{}", crate::history::now()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("loot.tsv");
        let a = Loot { kind: "credential".into(), host: "h".into(), value: "u:p".into() };
        assert_eq!(record_loot(&p, &[a.clone(), a.clone()], "hydra ..."), 1);
        assert_eq!(record_loot(&p, &[a], "again"), 0);
        let all = read_loot(&p);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].2, "hydra ...");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(mask("31d6cfe0d16ae931b73c59d7e0c089c0"), "31d6c••••••089c0");
        assert_eq!(slug("ACME Q4 / internal"), "ACME-Q4---internal");
    }
}
