//! The Settings tab: what each page shows, built fresh from the config, and
//! what clicking or pressing keys on a row does. Changes go through
//! `settings::Change`, so they're saved to settings.lua (never to the
//! user's vibex.lua) and can be reset row by row.

use crate::config::Config;
use crate::keys::{Action, Binding};
use crate::settings::{Change, Settings, Val};
use vibex_render::{Control, SettingRow, SettingsHit, SettingsView, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Appearance,
    Terminal,
    Security,
    Profiles,
    Shortcuts,
    Updates,
    About,
}

pub const PAGES: [(Page, &str); 7] = [
    (Page::Appearance, "Appearance"),
    (Page::Terminal, "Terminal"),
    (Page::Security, "Security"),
    (Page::Profiles, "Profiles"),
    (Page::Shortcuts, "Shortcuts"),
    (Page::Updates, "Updates"),
    (Page::About, "About"),
];

impl Page {
    pub fn named(name: &str) -> Option<Page> {
        PAGES.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(p, _)| *p)
    }
    fn index(self) -> usize {
        PAGES.iter().position(|(p, _)| *p == self).unwrap_or(0)
    }
}

/// What a row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Header,
    Info,
    Theme,
    FontFamily,
    FontSize,
    Opacity,
    Padding,
    CursorStyle,
    Bool(&'static str),
    Scrollback,
    DefaultProfile,
    OpenProfile(String),
    /// A button that runs this action.
    Run(Action),
    EditConfig,
    Shortcut(Action),
    ResetShortcuts,
    UpdateFeed,
    CheckNow,
    InstallNow,
    OpenConfigFolder,
    /// Click copies this text.
    Copy(String),
    /// Which engagement is on (dropdown).
    Engagement,
    /// Type a name to start a new engagement.
    NewEngagement,
    /// The active engagement's scope (text).
    Scope,
    EngagementFolder,
    EngagementReport,
    ShowLoot,
    AuditReport,
    /// Right-click: menu or paste (dropdown).
    RightClick,
}

impl Field {
    /// The settings.lua key this row overrides (for "Reset").
    fn key(&self) -> Option<&'static str> {
        Some(match self {
            Field::Theme => "theme",
            Field::FontFamily => "font_family",
            Field::FontSize => "font_size",
            Field::Opacity => "opacity",
            Field::Padding => "padding",
            Field::CursorStyle => "cursor_style",
            Field::Bool(k) => k,
            Field::Scrollback => "scrollback",
            Field::DefaultProfile => "default_profile",
            Field::UpdateFeed => "update_feed",
            Field::Engagement => "engagement",
            Field::RightClick => "right_click",
            _ => return None,
        })
    }

    fn focusable(&self) -> bool {
        !matches!(self, Field::Header | Field::Info)
    }
}

/// What the app should do after an input.
#[derive(Debug, PartialEq)]
pub enum Effect {
    None,
    Redraw,
    Change(Change),
    Perform(Action),
    OpenConfigFile,
    OpenConfigFolder,
    CheckUpdates,
    InstallUpdate,
    Copy(String),
    /// Text field committed with a value that isn't allowed.
    Invalid(String),
    /// Open the dropdown for row `i`.
    Dropdown(usize),
    NewEngagement(String),
    SetScope(String),
    EngagementFolder,
    EngagementReport,
    ShowLoot,
    AuditReport,
}

/// Facts for the About and Updates pages.
#[derive(Clone, Debug, Default)]
pub struct AboutInfo {
    pub version: String,
    pub renderer: String,
    pub host: String,
    pub config_path: String,
    pub settings_path: String,
    pub update_status: String,
    pub update_available: Option<String>,
    pub openconsole: bool,
    /// The bundled editors: "nano 9.3-1 · Neovim 0.12.6", or "" when not shipped.
    pub editors: String,
    pub editors_status: String,
    /// What a newer release would bring, once found.
    pub editors_available: Option<String>,
}

#[derive(Default)]
pub struct SettingsUi {
    /// The Settings tab exists.
    pub open: bool,
    /// It's the tab being shown.
    pub active: bool,
    pub page: usize,
    pub focus: Option<usize>,
    pub scroll: f32,
    pub filter: String,
    /// Text being typed into the focused text field.
    pub editing: Option<String>,
    /// The focused shortcut row waits for a key.
    pub capturing: bool,
    pub hover: Option<SettingsHit>,
    /// Row meaning, parallel to the last built view's rows.
    pub fields: Vec<Field>,
    families: Vec<String>,
    /// The active engagement's scope, as typed (set by the app).
    pub scope_text: String,
    /// "3 NTLM hashes, 2 credentials…" (set by the app).
    pub loot_summary: String,
}

const PADDINGS: &[f64] = &[0.0, 4.0, 6.0, 8.0, 10.0, 12.0, 16.0, 20.0];
const SCROLLBACKS: &[f64] = &[1000.0, 5000.0, 10000.0, 50000.0, 100000.0];
const CURSORS: &[&str] = &["block", "bar", "underline"];

fn cursor_name(c: &Config) -> &'static str {
    match c.cursor_shape {
        vibex_term::CursorShape::Bar => "bar",
        vibex_term::CursorShape::Underline => "underline",
        _ => "block",
    }
}

/// The family name a font path belongs to ("PlemolJPConsoleNF-Regular.ttf"
/// -> "PlemolJPConsoleNF"; Windows' own files by their family).
pub fn family_of(path: &str) -> String {
    let file = std::path::Path::new(path).file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    match file.as_str() {
        "consola.ttf" => return "Consolas".into(),
        "cour.ttf" => return "Courier New".into(),
        "lucon.ttf" => return "Lucida Console".into(),
        _ => {}
    }
    let stem = std::path::Path::new(path).file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    stem.strip_suffix("-Regular").unwrap_or(&stem).to_string()
}

/// "PlemolJPConsoleNF" -> "PlemolJP Console NF", "CascadiaCode" -> "Cascadia Code".
pub fn pretty_family(name: &str) -> String {
    if name.contains(' ') {
        return name.to_string();
    }
    let c: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &ch) in c.iter().enumerate() {
        if i > 0 && ch.is_uppercase() {
            let next_lower = c.get(i + 1).is_some_and(|n| n.is_lowercase());
            let space = if c[i - 1].is_lowercase() {
                // a capital run: one letter starts a word; a run to the end
                // is an acronym word ("NF"); a run glued to a word stays
                // with it ("PlemolJP")
                let run = c[i..].iter().take_while(|x| x.is_uppercase()).count();
                run == 1 || i + run == c.len()
            } else {
                // the last capital of a run that starts a word ("JPConsole")
                c[i - 1].is_uppercase() && next_lower
            };
            if space {
                out.push(' ');
            }
        }
        out.push(ch);
    }
    out
}

/// Monospace-looking families installed for this user or everyone.
fn installed_families() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut dirs: Vec<std::path::PathBuf> = crate::config::config_dir().map(|d| d.join("fonts")).into_iter().collect();
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        dirs.push(std::path::Path::new(&l).join("Microsoft").join("Windows").join("Fonts"));
    }
    if let Ok(w) = std::env::var("WINDIR") {
        dirs.push(std::path::Path::new(&w).join("Fonts"));
    }
    let mono = |n: &str| {
        let l = n.to_ascii_lowercase();
        ["mono", "code", "console", "nf", "term", "consolas", "courier"].iter().any(|k| l.contains(k))
    };
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix("-Regular.ttf") {
                if mono(stem) && crate::config::resolve_family(stem).is_some() && !out.iter().any(|f| f == stem) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    for f in ["Consolas", "Courier New", "Lucida Console"] {
        if crate::config::resolve_family(f).is_some() && !out.iter().any(|x| x == f) {
            out.push(f.to_string());
        }
    }
    out.sort_by_key(|f| f.to_ascii_lowercase());
    out
}

fn cycle<T: PartialEq + Clone>(list: &[T], current: &T, dir: i32) -> T {
    let n = list.len() as i32;
    let i = list.iter().position(|x| x == current).map_or(0, |i| i as i32);
    list[((i + dir).rem_euclid(n.max(1))) as usize].clone()
}

fn nearest(list: &[f64], v: f64) -> f64 {
    *list.iter().min_by(|a, b| (*a - v).abs().total_cmp(&(*b - v).abs())).unwrap()
}

impl SettingsUi {
    pub fn page(&self) -> Page {
        PAGES[self.page.min(PAGES.len() - 1)].0
    }

    pub fn show(&mut self, page: Option<Page>) {
        self.open = true;
        self.active = true;
        if let Some(p) = page {
            self.set_page(p.index());
        }
    }

    pub fn set_page(&mut self, i: usize) {
        if i != self.page {
            self.page = i.min(PAGES.len() - 1);
            self.focus = None;
            self.scroll = 0.0;
            self.editing = None;
            self.capturing = false;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.active = false;
        self.editing = None;
        self.capturing = false;
    }

    /// Build the page for drawing (and remember what each row means).
    pub fn view(&mut self, cfg: &Config, settings: &Settings, about: &AboutInfo) -> SettingsView {
        if self.families.is_empty() {
            self.families = installed_families();
        }
        let changed = |f: &Field| f.key().is_some_and(|k| settings.values.contains_key(k));
        let mut rows: Vec<(Field, SettingRow)> = Vec::new();
        macro_rules! push {
            ($f:expr, $r:expr $(,)?) => {{
                let f = $f;
                let mut r = $r;
                r.changed = changed(&f);
                rows.push((f, r));
            }};
        }
        let header = |t: &str| SettingRow::new(t, Control::Header);
        let toggle = |label: &str, detail: &str, key: &'static str| {
            (Field::Bool(key), SettingRow::new(label, Control::Toggle(crate::settings::bool_of(cfg, key))).detail(detail))
        };
        let mut filter = None;
        let mut note = String::from("Changes here are saved in settings.lua and override vibex.lua.");
        match self.page() {
            Page::Appearance => {
                push!(Field::Header, header("Colours"));
                let theme_name = Theme::BUILTIN
                    .iter()
                    .find(|n| Theme::builtin(n).is_some_and(|t| t.background == cfg.theme.background && t.foreground == cfg.theme.foreground))
                    .map_or("custom (vibex.lua)".to_string(), |n| n.to_string());
                let mut theme = SettingRow::new("Theme", Control::Choice(theme_name)).detail("Colours for text, background and cursor");
                theme.swatches = cfg.theme.palette[1..7].to_vec();
                theme.swatches.insert(0, cfg.theme.background);
                theme.swatches.push(cfg.theme.cursor);
                push!(Field::Theme, theme);
                push!(Field::Header, header("Text"));
                let family = pretty_family(&family_of(&cfg.fonts.regular));
                push!(Field::FontFamily, SettingRow::new("Font", Control::Choice(family)).detail("Monospace fonts installed on this PC"));
                let pt = cfg.font_size_pt;
                push!(Field::FontSize, SettingRow::new("Font size", Control::Slider { value: (pt - 6.0) / 26.0, text: format!("{pt} pt") }),);
                let (f, r) = toggle("Ligatures", "Draw -> != => as joined symbols", "ligatures");
                push!(f, r);
                push!(Field::Header, header("Window"));
                let o = cfg.opacity;
                push!(
                    Field::Opacity,
                    SettingRow::new("Background opacity", Control::Slider { value: o, text: format!("{}%", (o * 100.0).round()) })
                        .detail("Below 100% shows the acrylic blur (as in Windows Terminal)"),
                );
                let (f, r) = toggle("Glass", "See-through, frosted look over the blur (off = Windows Terminal's)", "glass");
                push!(f, r);
                push!(Field::Padding, SettingRow::new("Padding", Control::Choice(format!("{} px", cfg.padding.0))).detail("Space around the text"));
                let (f, r) = toggle("Status bar", "Profile, folder and exit code along the bottom", "status_bar");
                push!(f, r);
                let (f, r) = toggle("Dim inactive panes", "Fade splits that don't have focus", "dim_inactive_panes");
                push!(f, r);
                push!(Field::Header, header("Cursor"));
                push!(Field::CursorStyle, SettingRow::new("Cursor shape", Control::Choice(cursor_name(cfg).into())));
                let (f, r) = toggle("Blinking cursor", "", "cursor_blink");
                push!(f, r);
            }
            Page::Terminal => {
                push!(Field::Header, header("Behaviour"));
                let (f, r) = toggle("Restore last session", "Reopen tabs, splits and folders on start", "restore_session");
                push!(f, r);
                let (f, r) = toggle("Restore scrollback", "Bring back each pane's text too (saved in the config folder on exit)", "restore_scrollback");
                push!(f, r);
                let (f, r) =
                    toggle("Command history", "Keep finished commands for Ctrl+Shift+H (history.tsv; a leading space keeps one out)", "command_history");
                push!(f, r);
                let (f, r) = toggle(
                    "Suggestions while typing",
                    "Your past commands, dim after the cursor; Right accepts, Ctrl+Right takes a word, Ctrl+Shift+Space lists more",
                    "suggestions",
                );
                push!(f, r);
                let (f, r) = toggle(
                    "Linux and Windows commands",
                    "ls -la, grep, rm -rf, which, ps aux, df -h … in PowerShell and cmd; where, ipconfig, cls … in WSL (new tabs)",
                    "linux_commands",
                );
                push!(f, r);
                let rc = if cfg.right_click_menu { "Menu" } else { "Copy / paste" };
                push!(
                    Field::RightClick,
                    SettingRow::new("Right-click", Control::Choice(rc.into()))
                        .detail("A menu with shortcuts, or Windows Terminal's copy-if-selected-else-paste"),
                );
                let (f, r) = toggle("Smooth scrolling", "", "smooth_scroll");
                push!(f, r);
                let (f, r) = toggle("Prompt marks", "Coloured marks in the margin (needs shell integration)", "shell_marks");
                push!(f, r);
                push!(Field::Scrollback, SettingRow::new("Scrollback", Control::Choice(format!("{} lines", cfg.scrollback))).detail("History kept per pane"),);
                push!(Field::Header, header("Programs"));
                let (f, r) = toggle("Notifications from programs", "OSC 9 / 777: toast, or a Windows notification in the background", "notifications");
                push!(f, r);
                let (f, mut r) = toggle("Remote control (vibex cli)", "", "remote_control");
                if cfg.remote_control {
                    r.warning = Some("Programs in Vibex can list, read and type into your panes".into());
                } else {
                    r.detail = "Lets scripts drive Vibex: vibex cli list / spawn / send-text / get-text".into();
                }
                push!(f, r);
                let (f, mut r) = toggle("Let programs read the clipboard", "", "clipboard_read");
                if cfg.clipboard_read {
                    r.warning = Some("Any program in Vibex can read what you last copied".into());
                } else {
                    r.detail = "OSC 52 reads. Writing to the clipboard is always allowed".into();
                }
                push!(f, r);
            }
            Page::Security => {
                push!(Field::Header, header("Engagement"));
                let active = cfg.engagement.clone().unwrap_or_default();
                push!(
                    Field::Engagement,
                    SettingRow::new("Engagement", Control::Choice(if active.is_empty() { "Off".into() } else { active.clone() }))
                        .detail("Logs every command with its output, checks the scope, collects loot"),
                );
                let text_row = |field_at: usize, current: String, placeholder: &str| {
                    let editing = self.editing.is_some() && self.focus == Some(field_at);
                    let value = match (&self.editing, editing) {
                        (Some(buf), true) => buf.clone(),
                        _ => current,
                    };
                    Control::Text { value, editing, placeholder: placeholder.into() }
                };
                let at = rows.len();
                push!(
                    Field::NewEngagement,
                    SettingRow::new("Start a new engagement", text_row(at, String::new(), "name, then Enter (e.g. acme-q4-internal)"))
                        .detail("Makes its folder and turns it on"),
                );
                if !active.is_empty() {
                    let at = rows.len();
                    push!(
                        Field::Scope,
                        SettingRow::new("Scope", text_row(at, self.scope_text.clone(), "10.0.0.0/24, *.acme.com, !10.0.0.1"))
                            .detail("Addresses, ranges and domains; ! leaves one out. Network tools aimed elsewhere ask first"),
                    );
                    let (f, r) = toggle("Check the scope", "nmap, curl, ssh, sqlmap, netexec… ask before touching anything outside it", "scope_guard");
                    push!(f, r);
                    let (f, r) = toggle("Collect loot", "Hashes, credentials, keys and open ports from the output (Alt+Shift+L)", "collect_loot");
                    push!(f, r);
                    let (f, r) = toggle("Record every pane", "Each pane's session saved as a .cast in the engagement folder", "engagement_record");
                    push!(f, r);
                    push!(Field::ShowLoot, SettingRow::new("Loot", Control::Button("Show".into())).detail(self.loot_summary.clone()));
                    push!(
                        Field::EngagementReport,
                        SettingRow::new("Report", Control::Button("Export".into()))
                            .detail("Scope, findings (masked), recordings and the verified command log, as HTML")
                    );
                    push!(Field::EngagementFolder, SettingRow::new("Engagement folder", Control::Button("Open folder".into())));
                }
                push!(Field::Header, header("Safety"));
                let (f, r) = toggle(
                    "Secret guard",
                    "Mask API keys, tokens and private keys on screen; ask before pasting one; keep them out of history and recordings",
                    "secret_guard",
                );
                push!(f, r);
                let (f, r) = toggle(
                    "Ask before risky commands",
                    "rm -rf /, force-push, DROP TABLE, format, terraform destroy… (and every command on PROD hosts)",
                    "confirm_risky",
                );
                push!(f, r);
                let (f, r) = toggle("Ask before closing", "When a command is still running in the pane, tab or window", "confirm_close");
                push!(f, r);
                push!(Field::Header, header("Audit log"));
                let (f, r) = toggle("Log every command", "Where and when, hash-chained in audit.jsonl (always on during an engagement)", "audit_log");
                push!(f, r);
                let (f, r) = toggle("Keep each command's output", "Up to 64 KB per command, secrets redacted", "audit_output");
                push!(f, r);
                push!(
                    Field::AuditReport,
                    SettingRow::new("Audit report", Control::Button("Export".into())).detail("Checks the hash chain first; opens an HTML report")
                );
                note = "SSH host rules, extra secret patterns and risky-command patterns go in vibex.lua (docs/CONFIG.md).".into();
            }
            Page::Profiles => {
                push!(
                    Field::DefaultProfile,
                    SettingRow::new("Default profile", Control::Choice(cfg.default_profile.clone())).detail("Used for new tabs and windows"),
                );
                push!(Field::Header, header("Profiles"));
                for p in &cfg.profiles {
                    let mut cmd = p.command.program.clone();
                    for a in &p.command.args {
                        cmd.push(' ');
                        cmd.push_str(a);
                    }
                    if let Some(d) = &p.command.cwd {
                        cmd.push_str(&format!("   in {d}"));
                    }
                    let label = if p.name == cfg.default_profile { format!("{} (default)", p.name) } else { p.name.clone() };
                    push!(Field::OpenProfile(p.name.clone()), SettingRow::new(label, Control::Button("New tab".into())).detail(cmd));
                }
                push!(
                    Field::EditConfig,
                    SettingRow::new("Add or change profiles", Control::Button("Edit vibex.lua".into())).detail("Profiles are defined in config.profiles"),
                );
                note.clear();
            }
            Page::Shortcuts => {
                filter = Some(self.filter.clone());
                let q = self.filter.to_lowercase();
                // every action that has keys, then the rebindable ones without
                let mut actions: Vec<(Action, Vec<&Binding>)> = Vec::new();
                for b in &cfg.bindings {
                    match actions.iter_mut().find(|(a, _)| *a == b.action) {
                        Some((_, list)) => list.push(b),
                        None => actions.push((b.action.clone(), vec![b])),
                    }
                }
                for name in Action::UNIT_NAMES {
                    if let Some(a) = Action::named(name) {
                        if !actions.iter().any(|(x, _)| *x == a) {
                            actions.push((a, Vec::new()));
                        }
                    }
                }
                actions.sort_by_key(|(a, _)| a.label().to_lowercase());
                let any_changed = !settings.keys.is_empty();
                for (a, list) in actions {
                    let label = a.label();
                    let keys_text: String = list.iter().map(|b| b.describe()).collect::<Vec<_>>().join(" ");
                    if !q.is_empty() && !label.to_lowercase().contains(&q) && !keys_text.to_lowercase().contains(&q) {
                        continue;
                    }
                    let focused_here = self.capturing && self.focus == Some(rows.len());
                    let mut row = SettingRow::new(label, Control::Keys { chips: list.iter().map(|b| b.chips()).collect(), capturing: focused_here });
                    match a.id() {
                        Some(id) => row.changed = settings.keys.contains_key(&id),
                        None => row.detail = "Set in vibex.lua".into(),
                    }
                    rows.push((Field::Shortcut(a), row));
                }
                let mut reset = SettingRow::new("Reset all shortcuts", Control::Button("Reset".into())).detail("Back to the defaults and vibex.lua");
                reset.changed = false;
                if any_changed {
                    rows.push((Field::ResetShortcuts, reset));
                }
                note = "Click a shortcut (or press Enter) and press the new keys.".into();
            }
            Page::Updates => {
                let (f, r) = toggle("Check for updates automatically", "Once a day, when a feed is set", "check_updates");
                push!(f, r);
                let value = match (&self.editing, self.focus == Some(rows.len())) {
                    (Some(buf), true) => buf.clone(),
                    _ => cfg.update_feed.clone(),
                };
                let editing = self.editing.is_some() && self.focus == Some(rows.len());
                push!(
                    Field::UpdateFeed,
                    SettingRow::new("Update feed", Control::Text { value, editing, placeholder: "https://… /update.txt".into() },)
                        .detail("Where signed releases are published"),
                );
                let status = if about.update_status.is_empty() { "Not checked yet".to_string() } else { about.update_status.clone() };
                push!(Field::CheckNow, SettingRow::new("Check now", Control::Button("Check".into())).detail(status));
                if let Some(v) = &about.update_available {
                    push!(
                        Field::InstallNow,
                        SettingRow::new(format!("Vibex {v} is available"), Control::Button("Install and restart".into()))
                            .detail("Verified against the release signature before it runs"),
                    );
                }
                if !about.editors.is_empty() {
                    push!(Field::Header, header("nano and Neovim"));
                    let (f, r) = toggle(
                        "Check every week",
                        &format!("Installed: {}. A toast says when there's a newer release; nothing installs by itself", about.editors),
                        "editor_updates",
                    );
                    push!(f, r);
                    let status = if about.editors_status.is_empty() { "Not checked yet".to_string() } else { about.editors_status.clone() };
                    push!(Field::Run(Action::UpdateEditors), SettingRow::new("Editors", Control::Button("Check now".into())).detail(status));
                    if let Some(what) = &about.editors_available {
                        push!(
                            Field::Run(Action::UpdateEditors),
                            SettingRow::new("Update available", Control::Button("Update".into()))
                                .detail(format!("{what}; each download is checked against its published SHA-256")),
                        );
                    }
                }
                note = "Installers are checked with an Ed25519 signature built into Vibex.".into();
            }
            Page::About => {
                let info = |label: &str, value: &str| (Field::Copy(value.to_string()), SettingRow::new(label, Control::Info(value.to_string())));
                let rowsv = [
                    info("Version", &about.version),
                    info("Renderer", &about.renderer),
                    info("Console host", &about.host),
                    info("Config file", &about.config_path),
                    info("Settings file", &about.settings_path),
                ];
                for (f, r) in rowsv {
                    push!(f, r);
                }
                push!(Field::EditConfig, SettingRow::new("vibex.lua", Control::Button("Open".into())).detail("Your own config, hot-reloaded on save"));
                push!(Field::OpenConfigFolder, SettingRow::new("Config folder", Control::Button("Open folder".into())));
                push!(Field::Header, header("Credits"));
                push!(Field::Info, SettingRow::new("Vibex Terminal", Control::Info("© 2026 Kalyan Gopalam".into())));
                if about.openconsole {
                    push!(
                        Field::Info,
                        SettingRow::new("OpenConsole", Control::Info("MIT licence, © Microsoft".into()))
                            .detail("Console host from Windows Terminal (OpenConsole-LICENSE.txt)"),
                    );
                }
                push!(
                    Field::Info,
                    SettingRow::new("Built with", Control::Info(String::new()))
                        .detail("wgpu, winit, mlua (Lua 5.4), rustybuzz, ttf-parser, png, regex, AccessKit, arboard, ed25519-dalek, softbuffer",),
                );
                note = "Click a value to copy it.".into();
            }
        }
        // keep focus on a real row
        if let Some(f) = self.focus {
            if f >= rows.len() || !rows[f].0.focusable() {
                self.focus = None;
            }
        }
        self.fields = rows.iter().map(|(f, _)| f.clone()).collect();
        SettingsView {
            pages: PAGES.iter().map(|(_, n)| n.to_string()).collect(),
            page: self.page,
            title: PAGES[self.page].1.to_string(),
            rows: rows.into_iter().map(|(_, r)| r).collect(),
            focus: self.focus,
            hover: self.hover,
            scroll: self.scroll,
            filter,
            footer_button: (!settings.values.is_empty() || !settings.keys.is_empty()).then(|| "Reset all".to_string()),
            note,
            dialog: true,
        }
    }

    /// Move keyboard focus by `dir` focusable rows.
    pub fn move_focus(&mut self, dir: i32) {
        let n = self.fields.len();
        if n == 0 {
            return;
        }
        let mut i = match self.focus {
            Some(f) => f as i32,
            None if dir > 0 => -1,
            None => n as i32,
        };
        loop {
            i += dir;
            if i < 0 || i >= n as i32 {
                // the search box sits above the first row
                if i < 0 && self.page() == Page::Shortcuts {
                    self.focus = None;
                }
                return;
            }
            if self.fields[i as usize].focusable() {
                self.focus = Some(i as usize);
                return;
            }
        }
    }

    /// Enter / Space / click on row `i`.
    pub fn activate(&mut self, i: usize, cfg: &Config) -> Effect {
        let Some(field) = self.fields.get(i).cloned() else { return Effect::None };
        self.focus = Some(i);
        match field {
            Field::Bool(k) => Effect::Change(Change::Toggle(k)),
            Field::Theme
            | Field::FontFamily
            | Field::Padding
            | Field::CursorStyle
            | Field::Scrollback
            | Field::DefaultProfile
            | Field::Engagement
            | Field::RightClick => Effect::Dropdown(i),
            Field::NewEngagement => {
                self.editing = Some(String::new());
                Effect::Redraw
            }
            Field::Scope => {
                self.editing = Some(self.scope_text.clone());
                Effect::Redraw
            }
            Field::EngagementFolder => Effect::EngagementFolder,
            Field::EngagementReport => Effect::EngagementReport,
            Field::ShowLoot => Effect::ShowLoot,
            Field::AuditReport => Effect::AuditReport,
            Field::FontSize | Field::Opacity => Effect::Redraw,
            Field::OpenProfile(p) => Effect::Perform(Action::NewTab(Some(p))),
            Field::Run(a) => Effect::Perform(a),
            Field::EditConfig => Effect::OpenConfigFile,
            Field::OpenConfigFolder => Effect::OpenConfigFolder,
            Field::Shortcut(a) => {
                if a.id().is_some() {
                    self.capturing = true;
                }
                Effect::Redraw
            }
            Field::ResetShortcuts => Effect::Change(Change::ResetKeys),
            Field::UpdateFeed => {
                self.editing = Some(cfg.update_feed.clone());
                Effect::Redraw
            }
            Field::CheckNow => Effect::CheckUpdates,
            Field::InstallNow => Effect::InstallUpdate,
            Field::Copy(text) => Effect::Copy(text),
            Field::Header | Field::Info => Effect::None,
        }
    }

    /// ← / → (or the ‹ › buttons) on row `i`.
    pub fn step(&mut self, i: usize, dir: i32, cfg: &Config) -> Effect {
        let Some(field) = self.fields.get(i).cloned() else { return Effect::None };
        self.focus = Some(i);
        let ch = match field {
            Field::Theme => {
                let names: Vec<String> = Theme::BUILTIN.iter().map(|s| s.to_string()).collect();
                let cur = Theme::BUILTIN
                    .iter()
                    .find(|n| Theme::builtin(n).is_some_and(|t| t.background == cfg.theme.background))
                    .map_or(names[0].clone(), |n| n.to_string());
                Change::Theme(cycle(&names, &cur, dir))
            }
            Field::FontFamily => {
                if self.families.is_empty() {
                    return Effect::None;
                }
                let cur = family_of(&cfg.fonts.regular);
                Change::Set("font_family", Val::Str(cycle(&self.families, &cur, dir)))
            }
            Field::FontSize => Change::FontSize((cfg.font_size_pt + 0.5 * dir as f32).clamp(6.0, 32.0)),
            Field::Opacity => Change::Opacity(((cfg.opacity + 0.05 * dir as f32) * 20.0).round() / 20.0),
            Field::Padding => Change::Set("padding", Val::Num(cycle(PADDINGS, &nearest(PADDINGS, cfg.padding.0 as f64), dir))),
            Field::CursorStyle => {
                let cur = cursor_name(cfg);
                let next = cycle(CURSORS, &cur, dir);
                Change::CursorStyle(next)
            }
            Field::Scrollback => Change::Set("scrollback", Val::Num(cycle(SCROLLBACKS, &nearest(SCROLLBACKS, cfg.scrollback as f64), dir))),
            Field::DefaultProfile => {
                let names: Vec<String> = cfg.profiles.iter().map(|p| p.name.clone()).collect();
                if names.is_empty() {
                    return Effect::None;
                }
                Change::Set("default_profile", Val::Str(cycle(&names, &cfg.default_profile, dir)))
            }
            Field::Bool(k) => Change::Toggle(k),
            Field::Engagement | Field::RightClick => {
                let Some((opts, cur)) = self.options(i, cfg) else { return Effect::None };
                let n = opts.len() as i32;
                return opts.into_iter().nth(((cur as i32 + dir).rem_euclid(n.max(1))) as usize).map_or(Effect::None, |(_, c)| Effect::Change(c));
            }
            _ => return Effect::None,
        };
        Effect::Change(ch)
    }

    /// What a dropdown offers: (label, change) and which one is current.
    pub fn options(&self, i: usize, cfg: &Config) -> Option<(Vec<(String, Change)>, usize)> {
        let pick = |list: Vec<(String, Change)>, current: &str| {
            let at = list.iter().position(|(l, _)| l == current).unwrap_or(0);
            Some((list, at))
        };
        match self.fields.get(i)? {
            Field::Theme => {
                let cur = Theme::BUILTIN
                    .iter()
                    .find(|n| Theme::builtin(n).is_some_and(|t| t.background == cfg.theme.background && t.foreground == cfg.theme.foreground))
                    .map_or(String::new(), |n| n.to_string());
                pick(Theme::BUILTIN.iter().map(|n| (n.to_string(), Change::Theme(n.to_string()))).collect(), &cur)
            }
            Field::FontFamily => {
                let cur = family_of(&cfg.fonts.regular);
                let list: Vec<(String, Change)> = self.families.iter().map(|f| (pretty_family(f), Change::Set("font_family", Val::Str(f.clone())))).collect();
                let at = self.families.iter().position(|f| *f == cur).unwrap_or(0);
                (!list.is_empty()).then_some((list, at))
            }
            Field::Padding => pick(
                PADDINGS.iter().map(|p| (format!("{p} px"), Change::Set("padding", Val::Num(*p)))).collect(),
                &format!("{} px", nearest(PADDINGS, cfg.padding.0 as f64)),
            ),
            Field::CursorStyle => pick(CURSORS.iter().map(|c| (c.to_string(), Change::CursorStyle(c))).collect(), cursor_name(cfg)),
            Field::Scrollback => pick(
                SCROLLBACKS.iter().map(|n| (format!("{n} lines"), Change::Set("scrollback", Val::Num(*n)))).collect(),
                &format!("{} lines", nearest(SCROLLBACKS, cfg.scrollback as f64)),
            ),
            Field::DefaultProfile => {
                pick(cfg.profiles.iter().map(|p| (p.name.clone(), Change::Set("default_profile", Val::Str(p.name.clone())))).collect(), &cfg.default_profile)
            }
            Field::Engagement => {
                let mut list = vec![("Off".to_string(), Change::Set("engagement", Val::Str(String::new())))];
                for n in crate::engagement::list() {
                    list.push((n.clone(), Change::Set("engagement", Val::Str(n))));
                }
                let cur = cfg.engagement.clone().unwrap_or_else(|| "Off".into());
                pick(list, &cur)
            }
            Field::RightClick => pick(
                vec![
                    ("Menu".to_string(), Change::Set("right_click", Val::Str("menu".into()))),
                    ("Copy / paste".to_string(), Change::Set("right_click", Val::Str("paste".into()))),
                ],
                if cfg.right_click_menu { "Menu" } else { "Copy / paste" },
            ),
            _ => None,
        }
    }

    /// A slider dragged / clicked at fraction `f` (0..1) of its track.
    pub fn slide(&mut self, i: usize, f: f32) -> Effect {
        let f = f.clamp(0.0, 1.0);
        self.focus = Some(i);
        match self.fields.get(i) {
            Some(Field::FontSize) => Effect::Change(Change::FontSize(((6.0 + f * 26.0) * 2.0).round() / 2.0)),
            Some(Field::Opacity) => Effect::Change(Change::Opacity((f * 20.0).round() / 20.0)),
            _ => Effect::None,
        }
    }

    /// "Reset" on row `i`: back to vibex.lua's value.
    pub fn reset(&mut self, i: usize) -> Effect {
        match self.fields.get(i) {
            Some(Field::Shortcut(a)) => a.id().map_or(Effect::None, |id| Effect::Change(Change::Unbind(id))),
            Some(f) => f.key().map_or(Effect::None, |k| Effect::Change(Change::Unset(k))),
            None => Effect::None,
        }
    }

    /// Finish typing in the focused text field.
    pub fn commit_text(&mut self) -> Effect {
        let Some(buf) = self.editing.take() else { return Effect::None };
        let v = buf.trim().to_string();
        let field = self.focus.and_then(|i| self.fields.get(i)).cloned();
        match field {
            Some(Field::NewEngagement) if !v.is_empty() => Effect::NewEngagement(v),
            Some(Field::Scope) => Effect::SetScope(v),
            Some(Field::UpdateFeed) => {
                if v.is_empty() || v.starts_with("https://") {
                    Effect::Change(Change::Set("update_feed", Val::Str(v)))
                } else {
                    Effect::Invalid("the update feed must be an https:// address".into())
                }
            }
            _ => Effect::Redraw,
        }
    }

    /// The action of the focused shortcut row (while capturing).
    pub fn capture_target(&self) -> Option<Action> {
        match self.focus.and_then(|i| self.fields.get(i)) {
            Some(Field::Shortcut(a)) if self.capturing => Some(a.clone()),
            _ => None,
        }
    }

    /// Plain text for screen readers: one line per row.
    pub fn spoken_rows(view: &SettingsView) -> Vec<String> {
        view.rows
            .iter()
            .map(|r| {
                let value = match &r.control {
                    Control::Header => return format!("{} (section)", r.label),
                    Control::Info(v) | Control::Choice(v) | Control::Button(v) => v.clone(),
                    Control::Toggle(b) => if *b { "on" } else { "off" }.into(),
                    Control::Slider { text, .. } => text.clone(),
                    Control::Text { value, .. } => value.clone(),
                    Control::Keys { chips, .. } => {
                        if chips.is_empty() {
                            "not set".into()
                        } else {
                            chips.iter().map(|c| c.join("+")).collect::<Vec<_>>().join(" or ")
                        }
                    }
                };
                if r.detail.is_empty() {
                    format!("{}: {value}", r.label)
                } else {
                    format!("{}: {value}. {}", r.label, r.detail)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui_on(page: Page) -> (SettingsUi, Config, Settings) {
        let mut ui = SettingsUi::default();
        ui.show(Some(page));
        (ui, Config::default(), Settings::default())
    }

    #[test]
    fn appearance_rows_change_settings() {
        let (mut ui, mut cfg, mut st) = ui_on(Page::Appearance);
        let v = ui.view(&cfg, &st, &AboutInfo::default());
        assert!(v.rows.iter().any(|r| r.label == "Theme"));
        let theme_row = ui.fields.iter().position(|f| *f == Field::Theme).unwrap();
        let Effect::Change(ch) = ui.step(theme_row, 1, &cfg) else { panic!() };
        crate::settings::change(&mut st, &mut cfg, &ch);
        assert_ne!(cfg.theme.background, Theme::default().background, "next theme");
        let v = ui.view(&cfg, &st, &AboutInfo::default());
        assert!(v.rows[theme_row].changed, "shows Reset");
        let Effect::Change(ch) = ui.reset(theme_row) else { panic!() };
        crate::settings::change(&mut st, &mut cfg, &ch);
        assert!(!st.values.contains_key("theme"));

        let op = ui.fields.iter().position(|f| *f == Field::Opacity).unwrap();
        assert_eq!(ui.slide(op, 0.73), Effect::Change(Change::Opacity(0.75)));
        let fs = ui.fields.iter().position(|f| *f == Field::FontSize).unwrap();
        assert_eq!(ui.slide(fs, 0.0), Effect::Change(Change::FontSize(6.0)));
    }

    #[test]
    fn focus_skips_headers() {
        let (mut ui, cfg, st) = ui_on(Page::Appearance);
        ui.view(&cfg, &st, &AboutInfo::default());
        ui.move_focus(1);
        let f = ui.focus.unwrap();
        assert!(ui.fields[f].focusable());
        assert_eq!(ui.fields[0], Field::Header);
    }

    #[test]
    fn shortcuts_filter_and_capture() {
        let (mut ui, cfg, st) = ui_on(Page::Shortcuts);
        let all = ui.view(&cfg, &st, &AboutInfo::default()).rows.len();
        ui.filter = "palette".into();
        let v = ui.view(&cfg, &st, &AboutInfo::default());
        assert!(v.rows.len() < all && v.rows.iter().any(|r| r.label == "Command palette"));
        let i = ui.fields.iter().position(|f| *f == Field::Shortcut(Action::CommandPalette)).unwrap();
        ui.activate(i, &cfg);
        assert_eq!(ui.capture_target(), Some(Action::CommandPalette));
        let v = ui.view(&cfg, &st, &AboutInfo::default());
        assert!(matches!(v.rows[i].control, Control::Keys { capturing: true, .. }));
    }

    #[test]
    fn update_feed_must_be_https() {
        let (mut ui, cfg, st) = ui_on(Page::Updates);
        ui.view(&cfg, &st, &AboutInfo::default());
        let i = ui.fields.iter().position(|f| *f == Field::UpdateFeed).unwrap();
        ui.activate(i, &cfg);
        ui.editing = Some("http://example.com".into());
        assert!(matches!(ui.commit_text(), Effect::Invalid(_)));
        ui.editing = Some(" https://example.com/u.txt ".into());
        assert_eq!(ui.commit_text(), Effect::Change(Change::Set("update_feed", Val::Str("https://example.com/u.txt".into()))));
    }

    #[test]
    fn family_names() {
        // build paths with this platform's separator so the test runs everywhere
        let p = |file: &str| std::path::Path::new("fonts").join(file).to_string_lossy().into_owned();
        assert_eq!(family_of(&p("PlemolJPConsoleNF-Regular.ttf")), "PlemolJPConsoleNF");
        assert_eq!(family_of(&p("consola.ttf")), "Consolas");
        assert_eq!(pretty_family("PlemolJPConsoleNF"), "PlemolJP Console NF");
        assert_eq!(pretty_family("CascadiaCode"), "Cascadia Code");
        assert_eq!(pretty_family("Courier New"), "Courier New");
    }
}
