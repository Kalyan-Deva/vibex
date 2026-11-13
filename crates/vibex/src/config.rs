//! Configuration: built-in defaults, overlaid by `~/.config/vibex/vibex.lua`
//! (see `lua.rs`), then by a few environment variables.

use crate::keys::{default_bindings, Binding, KeySpec, Mods};
use std::path::{Path, PathBuf};
use std::time::Duration;
use vibex_pty::Command;
use vibex_render::{FontPaths, Theme};
use vibex_term::CursorShape;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererKind {
    /// GPU, falling back to CPU if no adapter works.
    Auto,
    Gpu,
    Cpu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decorations {
    /// Vibex draws its own title bar (tabs + window buttons).
    Vibex,
    /// The standard Windows title bar.
    Native,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabBarMode {
    /// Shown when there is more than one tab (always with Vibex decorations).
    Auto,
    Always,
    Never,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub command: Command,
}

#[derive(Clone, Debug)]
pub struct Leader {
    pub key: KeySpec,
    pub mods: Mods,
    pub timeout: Duration,
}

/// What a trigger does when its pattern matches a line of output.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerAction {
    /// Colour the matched text's background.
    Highlight(u32),
    /// A notification (toast when focused, Windows notification if not).
    Notify(String),
    Toast,
    /// A Lua function: `function(win, text)`.
    Callback(usize),
}

/// How an SSH host looks: matched by a `*` / `?` pattern.
#[derive(Clone, Debug)]
pub struct SshRule {
    pub pattern: String,
    /// Ask before running any command on this host.
    pub confirm: bool,
    /// Tab colour and a border round the pane.
    pub color: Option<u32>,
    /// Shown before the host in the tab title ("PROD").
    pub label: Option<String>,
    /// Colours for the pane while it's connected.
    pub theme: Option<vibex_render::Theme>,
}

/// Until you set `config.ssh_hosts`: anything with "prod" in it is red.
pub fn default_ssh_rules() -> Vec<SshRule> {
    vec![SshRule { pattern: "*prod*".into(), confirm: true, color: Some(0xc42b1c), label: Some("PROD".into()), theme: None }]
}

/// Commands that ask before they run: (what it is, pattern).
pub const DANGEROUS: &[(&str, &str)] = &[
    (
        "deletes a whole tree from / or ~",
        r"(?i)\brm\s+(?:-[a-z]*\s+)*-[a-z]*(?:rf|fr)[a-z]*\s+(?:--no-preserve-root\s+)?(?:/|~|\*|\$HOME)(?:\s|$|/\*?(?:\s|$))",
    ),
    ("deletes everything here", r"(?i)\brm\s+(?:-[a-z]*\s+)*-[a-z]*(?:rf|fr)[a-z]*\s+(?:\.|\./\*|\*)(?:\s|$)"),
    ("force-deletes a folder tree", r"(?i)\bRemove-Item\b.*-Recurse\b.*-Force\b.*(?:\s[A-Z]:\\?(?:\s|$)|\s~|\s\$HOME|\s\\(?:\s|$))"),
    ("force-pushes", r"\bgit\s+push\b.*(?:\s--force(?:-with-lease)?\b|\s-f\b|\s\+\S)"),
    ("throws away local changes", r"\bgit\s+(?:reset\s+--hard|clean\s+-[a-z]*f[a-z]*d|clean\s+-[a-z]*d[a-z]*f|checkout\s+--\s+\.)"),
    ("drops a database table", r"(?i)\b(?:DROP\s+(?:TABLE|DATABASE|SCHEMA)|TRUNCATE\s+TABLE)\b"),
    ("deletes every row", r"(?i)\bDELETE\s+FROM\s+\w+\s*;?\s*$"),
    ("formats a disk", r"(?i)\b(?:mkfs(?:\.\w+)?\s|format\s+[a-z]:|Format-Volume\b|Clear-Disk\b|diskpart\b)"),
    ("writes straight to a disk", r"\bdd\b.*\bof=/dev/(?:sd|nvme|disk|hd|mmcblk)"),
    ("shuts the machine down", r"(?i)^\s*(?:sudo\s+)?(?:shutdown|poweroff|halt|reboot|Stop-Computer|Restart-Computer)\b"),
    ("is a fork bomb", r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:"),
    ("makes / world-writable", r"\bchmod\s+(?:-R\s+)?0?777\s+/(?:\s|$)"),
    ("deletes a Kubernetes namespace or everything", r"\bkubectl\s+delete\s+(?:ns|namespace|all\b|--all\b)"),
    ("destroys infrastructure", r"\b(?:terraform|tofu)\s+destroy\b"),
];

pub fn default_dangerous() -> Vec<(String, regex::Regex)> {
    DANGEROUS.iter().map(|(n, p)| (n.to_string(), regex::Regex::new(p).expect("built-in pattern"))).collect()
}

#[derive(Clone, Debug)]
pub struct Trigger {
    pub regex: regex::Regex,
    pub action: TriggerAction,
}

/// Where the editor opens for Ctrl+click on `file:line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenIn {
    /// A floating pane that closes when the editor exits.
    Popup,
    Tab,
    Split,
    /// A separate program (GUI editors like VS Code).
    App,
}

/// The editor command: `{file}`, `{line}`, `{col}` are filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    pub cmd: Vec<String>,
    pub env: Vec<(String, String)>,
    pub open_in: OpenIn,
}

#[derive(Clone)]
pub struct Config {
    pub fonts: FontPaths,
    /// Font size in points (scaled by DPI).
    pub font_size_pt: f32,
    pub ligatures: bool,
    pub theme: Theme,
    pub profiles: Vec<Profile>,
    pub default_profile: String,
    pub scrollback: usize,
    pub padding: (usize, usize),
    pub window_size: (f64, f64),
    pub wheel_lines: usize,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub blink_interval_ms: u64,
    /// Max time between clicks for double/triple click.
    pub multi_click_ms: u64,
    /// Background opacity (1.0 = opaque). Below 1 the window shows the
    /// Windows acrylic backdrop behind the text (GPU renderer only).
    pub opacity: f32,
    /// Glass look over the acrylic: much more see-through, smoky title and
    /// status bars, a frosted sheen. Off = matches Windows Terminal.
    pub glass: bool,
    /// Right-click in the terminal: a menu, or Windows Terminal's
    /// copy-if-selected-else-paste.
    pub right_click_menu: bool,
    /// The shell prompt Vibex's shell scripts set: "default" (the shell's
    /// own) or "dragon" (Kali Linux's two-line prompt).
    pub prompt: String,
    /// The window's top: "tabs" (tabs in the title bar, like Windows
    /// Terminal) or "classic" (a title, a menu bar, and tabs only when
    /// there's more than one, like QTerminal on Kali).
    pub title_bar: String,
    pub renderer: RendererKind,
    pub smooth_scroll: bool,
    pub status_bar: bool,
    pub leader: Option<Leader>,
    pub bindings: Vec<Binding>,
    pub decorations: Decorations,
    pub tab_bar: TabBarMode,
    /// Coloured prompt marks in the left gutter (needs shell integration).
    pub shell_marks: bool,
    pub dim_inactive_panes: bool,
    /// Reopen the last tabs/splits/folders on start.
    pub restore_session: bool,
    /// Also bring back each pane's scrollback text (saved on exit).
    pub restore_scrollback: bool,
    /// Keep finished commands in history.tsv for Ctrl+Shift+H.
    pub command_history: bool,
    /// Fish-style suggestions from history while you type (Right accepts).
    pub suggestions: bool,
    /// Linux commands in PowerShell / cmd (ls -la, grep, rm -rf, which …)
    /// and Windows ones in WSL (where, ipconfig, cls …).
    pub linux_commands: bool,
    /// Look for new nano / Neovim releases once a week (the bundled ones).
    pub editor_updates: bool,
    /// Let programs read the clipboard with OSC 52 (off: any program could
    /// read whatever you last copied).
    pub clipboard_read: bool,
    /// `vibex cli` may control Vibex (list, type into, read panes...).
    /// Off: any program running in a pane could use it.
    pub remote_control: bool,
    /// Show OSC 9 / OSC 777 notifications from programs.
    pub notifications: bool,
    /// The Quake-style dropdown: global hotkey (key, mods) and height (0..1
    /// of the screen). None = off.
    pub quake: Option<(String, String, f32)>,
    /// Regex triggers on output (highlight / notify / Lua).
    pub triggers: Vec<Trigger>,
    /// Per-host looks for SSH sessions.
    pub ssh_hosts: Vec<SshRule>,
    /// Mask secrets on screen, ask before pasting one, keep them out of
    /// history / recordings / the audit log.
    pub secret_guard: bool,
    pub secrets: crate::secrets::Guard,
    /// Ask before running a command that matches one of these.
    pub confirm_commands: Vec<(String, regex::Regex)>,
    /// Ask before closing a pane, tab or window with something running.
    pub confirm_close: bool,
    /// Hash-chained log of every command (`audit.jsonl`).
    pub audit_log: bool,
    /// Also keep each command's output (secrets redacted) in the log.
    pub audit_output: bool,
    /// Ask before running a risky command (`confirm_commands`).
    pub confirm_risky: bool,
    /// The engagement that's on (a folder in engagements/).
    pub engagement: Option<String>,
    /// During an engagement: record every pane.
    pub engagement_record: bool,
    /// During an engagement: ask before network tools leave the scope.
    pub scope_guard: bool,
    /// During an engagement: collect hashes, credentials, open ports.
    pub collect_loot: bool,
    /// Ctrl+click on `file:line` opens this (None: VS Code if installed,
    /// else the system's default app for the file).
    pub editor: Option<Editor>,
    /// Notify when a command that ran at least this many seconds finishes
    /// while you aren't watching it (0 = never). Needs shell integration.
    pub notify_long_commands: f32,
    /// Where to look for updates (a signed feed); empty = never.
    pub update_feed: String,
    /// Look for updates once a day by itself (when a feed is set).
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        let profiles = default_profiles();
        Self {
            fonts: default_fonts(),
            font_size_pt: 11.0,
            ligatures: true,
            theme: Theme::default(),
            default_profile: profiles[0].name.clone(),
            profiles,
            scrollback: 10_000,
            padding: (10, 8),
            window_size: (1100.0, 680.0),
            wheel_lines: 3,
            cursor_shape: CursorShape::Block,
            cursor_blink: true,
            blink_interval_ms: 530,
            multi_click_ms: 400,
            // 40% acrylic, as in Windows Terminal (see acrylic_opacity)
            opacity: 0.4,
            renderer: RendererKind::Auto,
            smooth_scroll: true,
            // Windows Terminal has none; turn it on in Settings or vibex.lua
            status_bar: false,
            glass: false,
            right_click_menu: true,
            prompt: "default".into(),
            title_bar: "tabs".into(),
            leader: None,
            bindings: default_bindings(),
            decorations: Decorations::Vibex,
            tab_bar: TabBarMode::Auto,
            shell_marks: true,
            dim_inactive_panes: true,
            restore_session: true,
            restore_scrollback: true,
            command_history: true,
            suggestions: true,
            linux_commands: true,
            editor_updates: true,
            clipboard_read: false,
            remote_control: false,
            notifications: true,
            notify_long_commands: 10.0,
            quake: Some(("`".into(), "WIN".into(), 0.45)),
            triggers: Vec::new(),
            ssh_hosts: default_ssh_rules(),
            secret_guard: true,
            secrets: crate::secrets::Guard::default(),
            confirm_commands: default_dangerous(),
            confirm_close: true,
            audit_log: false,
            audit_output: false,
            confirm_risky: true,
            engagement: None,
            engagement_record: false,
            scope_guard: true,
            collect_loot: true,
            editor: None,
            update_feed: String::new(),
            check_updates: true,
        }
    }
}

impl Config {
    /// Environment overrides (handy for quick experiments):
    /// VIBEX_RENDERER=cpu|gpu, VIBEX_OPACITY=0.0-1.0, VIBEX_FONT=<path>.
    pub fn with_env(mut self) -> Self {
        match std::env::var("VIBEX_RENDERER").as_deref() {
            Ok("cpu") => self.renderer = RendererKind::Cpu,
            Ok("gpu") => self.renderer = RendererKind::Gpu,
            _ => {}
        }
        if let Some(o) = std::env::var("VIBEX_OPACITY").ok().and_then(|v| v.parse::<f32>().ok()) {
            self.opacity = o.clamp(0.0, 1.0);
        }
        if std::env::var("VIBEX_REMOTE_CONTROL").as_deref() == Ok("1") {
            self.remote_control = true;
        }
        if let Ok(path) = std::env::var("VIBEX_FONT") {
            if let Some(f) = family_from_file(&path) {
                self.fonts.regular = f.0;
                self.fonts.bold = f.1;
                self.fonts.italic = f.2;
                self.fonts.bold_italic = f.3;
            }
        }
        self
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

// ------------------------------------------------------------------ paths

/// `%USERPROFILE%\.config\vibex` (Unix: `~/.config/vibex`).
pub fn config_dir() -> Option<PathBuf> {
    // VIBEX_CONFIG_DIR: a separate config folder (tests, portable setups)
    if let Some(d) = std::env::var_os("VIBEX_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(Path::new(&home).join(".config").join("vibex"))
}

pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("vibex.lua"))
}

/// Expand a leading `~` to the home directory.
pub fn expand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix('~') {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            return format!("{}{}", home.to_string_lossy(), rest);
        }
    }
    p.to_string()
}

// ------------------------------------------------------------------ fonts

#[cfg(windows)]
fn system_font(name: &str) -> Option<String> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let p = Path::new(&windir).join("Fonts").join(name);
    p.exists().then(|| p.to_string_lossy().into_owned())
}

/// Linux / macOS: font files live in nested folders; index them once.
#[cfg(not(windows))]
fn system_font(name: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static INDEX: OnceLock<HashMap<String, String>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        let roots = [
            "/usr/share/fonts".to_string(),
            "/usr/local/share/fonts".to_string(),
            format!("{home}/.local/share/fonts"),
            format!("{home}/.fonts"),
            "/System/Library/Fonts".to_string(),
            "/Library/Fonts".to_string(),
            format!("{home}/Library/Fonts"),
        ];
        let mut map = HashMap::new();
        fn walk(dir: &Path, depth: usize, map: &mut HashMap<String, String>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if depth < 5 {
                        walk(&p, depth + 1, map);
                    }
                } else if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
                    map.entry(n.to_ascii_lowercase()).or_insert_with(|| p.to_string_lossy().into_owned());
                }
            }
        }
        for r in roots {
            walk(Path::new(&r), 0, &mut map);
        }
        map
    });
    index.get(&name.to_ascii_lowercase()).cloned()
}

type Family = (String, Option<String>, Option<String>, Option<String>);

/// `Foo-Regular.ttf` plus `Foo-Bold/Italic/BoldItalic.ttf` siblings.
fn family_from_file(path: &str) -> Option<Family> {
    if !Path::new(path).is_file() {
        return None;
    }
    let sibling = |style: &str| {
        let p = path.replace("-Regular.", &format!("-{style}."));
        (p != path && Path::new(&p).is_file()).then_some(p)
    };
    Some((path.to_string(), sibling("Bold"), sibling("Italic"), sibling("BoldItalic")))
}

/// Resolve a family name: `<config>/fonts/<Name>-Regular.ttf` first (spaces
/// removed, so "Cascadia Code" finds CascadiaCode-*.ttf), then the Windows
/// fonts that ship with the OS.
pub fn resolve_family(name: &str) -> Option<Family> {
    let compact: String = name.chars().filter(|c| !c.is_whitespace()).collect();
    // <Name>-Regular.ttf in Vibex's fonts folder, then fonts installed for
    // this user or for everyone (e.g. "PlemolJP Console NF")
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut dirs: Vec<PathBuf> = config_dir().map(|d| d.join("fonts")).into_iter().collect();
    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            dirs.push(Path::new(&local).join("Microsoft").join("Windows").join("Fonts"));
        }
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        dirs.push(Path::new(&windir).join("Fonts"));
    }
    for dir in &dirs {
        let regular = dir.join(format!("{compact}-Regular.ttf"));
        if let Some(f) = family_from_file(&regular.to_string_lossy()) {
            return Some(f);
        }
    }
    #[cfg(not(windows))]
    if let Some(f) = system_font(&format!("{compact}-Regular.ttf")).and_then(|p| family_from_file(&p)) {
        return Some(f);
    }
    let files = match compact.to_ascii_lowercase().as_str() {
        "consolas" => ["consola.ttf", "consolab.ttf", "consolai.ttf", "consolaz.ttf"],
        "couriernew" | "courier" => ["cour.ttf", "courbd.ttf", "couri.ttf", "courbi.ttf"],
        "lucidaconsole" => ["lucon.ttf", "", "", ""],
        "dejavusansmono" => ["DejaVuSansMono.ttf", "DejaVuSansMono-Bold.ttf", "DejaVuSansMono-Oblique.ttf", "DejaVuSansMono-BoldOblique.ttf"],
        "liberationmono" => ["LiberationMono-Regular.ttf", "LiberationMono-Bold.ttf", "LiberationMono-Italic.ttf", "LiberationMono-BoldItalic.ttf"],
        "notosansmono" => ["NotoSansMono-Regular.ttf", "NotoSansMono-Bold.ttf", "", ""],
        "ubuntumono" => ["UbuntuMono-R.ttf", "UbuntuMono-B.ttf", "UbuntuMono-RI.ttf", "UbuntuMono-BI.ttf"],
        "menlo" => ["Menlo.ttc", "", "", ""],
        _ => return None,
    };
    let regular = system_font(files[0])?;
    let opt = |f: &str| if f.is_empty() { None } else { system_font(f) };
    Some((regular, opt(files[1]), opt(files[2]), opt(files[3])))
}

pub fn default_fallbacks() -> Vec<String> {
    #[cfg(windows)]
    let names = ["consola.ttf", "seguisym.ttf", "seguiemj.ttf", "msyh.ttc", "malgun.ttf", "msgothic.ttc", "arial.ttf"];
    #[cfg(not(windows))]
    let names = [
        "DejaVuSansMono.ttf",
        "DejaVuSans.ttf",
        "NotoSansSymbols2-Regular.ttf",
        "Twemoji.Mozilla.ttf",
        "NotoSansCJK-Regular.ttc",
        "Apple Color Emoji.ttc",
        "Arial Unicode.ttf",
    ];
    names.iter().filter_map(|f| system_font(f)).collect()
}

fn default_fonts() -> FontPaths {
    // PlemolJP Console NF (Windows Terminal "Dev" profile's font) when it's
    // installed, else Cascadia Code, else what ships with the OS
    let (regular, bold, italic, bold_italic) = resolve_family("PlemolJP Console NF")
        .or_else(|| resolve_family("Cascadia Code"))
        .or_else(|| resolve_family("Consolas"))
        .or_else(|| resolve_family("DejaVu Sans Mono"))
        .or_else(|| resolve_family("Liberation Mono"))
        .or_else(|| resolve_family("Noto Sans Mono"))
        .or_else(|| resolve_family("Ubuntu Mono"))
        .or_else(|| resolve_family("Menlo"))
        .or_else(|| resolve_family("Courier New"))
        .unwrap_or_else(|| ("cour.ttf".into(), None, None, None));
    let fallbacks = default_fallbacks().into_iter().filter(|f| *f != regular).collect();
    FontPaths { regular, bold, italic, bold_italic, fallbacks }
}

// --------------------------------------------------------------- profiles

/// A program on PATH.
pub fn find_on_path(exe: &str) -> Option<PathBuf> {
    find_in_path(exe)
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file())
}

/// Environment every shell gets.
pub fn base_command(program: impl Into<String>) -> Command {
    // WSLENV forwards these into WSL, so shell integration scripts there can
    // tell they're running in Vibex.
    let ours = "TERM_PROGRAM:TERM_PROGRAM_VERSION:COLORTERM:VIBEX_EXE/p";
    let wslenv = match std::env::var("WSLENV") {
        Ok(existing) if !existing.is_empty() => format!("{existing}:{ours}"),
        _ => ours.to_string(),
    };
    Command::new(program)
        .env("TERM_PROGRAM", "Vibex")
        .env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"))
        .env("COLORTERM", "truecolor")
        // `vshow` and friends in the shell scripts run this Vibex
        .env("VIBEX_EXE", std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
        .env("WSLENV", wslenv)
}

/// powershell (pwsh 7 if installed), cmd, and wsl if present.
/// Linux / macOS: the login shell from $SHELL, plus sh.
#[cfg(not(windows))]
fn default_profiles() -> Vec<Profile> {
    let home = std::env::var("HOME").ok();
    let shell = std::env::var("SHELL").ok().filter(|s| Path::new(s).is_file()).unwrap_or_else(|| "/bin/sh".into());
    let name = Path::new(&shell).file_name().map_or("shell".into(), |n| n.to_string_lossy().into_owned());
    let mut login = base_command(shell.clone()).arg("-l");
    login.cwd = home.clone();
    let mut out = vec![Profile { name, command: login }];
    if shell != "/bin/sh" {
        let mut sh = base_command("/bin/sh");
        sh.cwd = home;
        out.push(Profile { name: "sh".into(), command: sh });
    }
    out
}

#[cfg(windows)]
fn default_profiles() -> Vec<Profile> {
    let home = std::env::var("USERPROFILE").ok();
    let pwsh = find_in_path("pwsh.exe")
        .or_else(|| Some(PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe")).filter(|p| p.is_file()))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "powershell.exe".into());
    let mut out = Vec::new();
    let mut ps = base_command(pwsh).arg("-NoLogo");
    ps.cwd = home.clone();
    out.push(Profile { name: "powershell".into(), command: ps });
    let mut cmd = base_command("cmd.exe");
    cmd.cwd = home.clone();
    out.push(Profile { name: "cmd".into(), command: cmd });
    if find_in_path("wsl.exe").is_some() {
        out.push(Profile { name: "wsl".into(), command: base_command("wsl.exe").arg("~") });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_order() {
        let f = default_fonts();
        if resolve_family("PlemolJPConsoleNF").is_some() {
            assert!(f.regular.ends_with("PlemolJPConsoleNF-Regular.ttf"), "{}", f.regular);
            assert!(f.bold.is_some() && f.italic.is_some() && f.bold_italic.is_some());
        } else if resolve_family("CascadiaCode").is_some() {
            assert!(f.regular.ends_with("CascadiaCode-Regular.ttf"));
            assert!(f.bold.as_deref().is_some_and(|b| b.ends_with("CascadiaCode-Bold.ttf")));
            assert!(f.italic.is_some() && f.bold_italic.is_some());
        } else {
            assert!(f.regular.to_lowercase().ends_with(".ttf"));
        }
    }

    #[test]
    fn family_names_resolve() {
        if system_font("consola.ttf").is_some() {
            let (r, b, ..) = resolve_family("Consolas").unwrap();
            assert!(r.ends_with("consola.ttf") && b.is_some());
        }
        assert!(resolve_family("No Such Font").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn default_profiles_include_powershell() {
        let c = Config::default();
        assert!(c.profile("powershell").is_some());
        assert!(c.profile("PowerShell").is_some());
        assert_eq!(c.default_profile, "powershell");
    }

    #[test]
    fn home_expansion() {
        let e = expand_home("~/x");
        assert!(!e.starts_with('~'));
        assert_eq!(expand_home("C:/y"), "C:/y");
    }
}
