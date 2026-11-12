//! Key specs, modifier sets, actions and the default key table.

use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub sup: bool,
    pub leader: bool,
}

impl Mods {
    /// "CTRL|SHIFT", "ctrl+alt", "LEADER", "" ...
    pub fn parse(s: &str) -> Result<Mods, String> {
        let mut m = Mods::default();
        for part in s.split(['|', '+', ' ']).map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_uppercase().as_str() {
                "CTRL" | "CONTROL" => m.ctrl = true,
                "SHIFT" => m.shift = true,
                "ALT" | "OPT" | "META" => m.alt = true,
                "SUPER" | "WIN" | "CMD" => m.sup = true,
                "LEADER" => m.leader = true,
                "NONE" => {}
                other => return Err(format!("unknown modifier '{other}'")),
            }
        }
        Ok(m)
    }

    pub fn from_state(s: ModifiersState, leader: bool) -> Mods {
        Mods { ctrl: s.control_key(), shift: s.shift_key(), alt: s.alt_key(), sup: s.super_key(), leader }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeySpec {
    Char(char),
    Named(NamedKey),
}

impl KeySpec {
    pub fn parse(s: &str) -> Result<KeySpec, String> {
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c == ' ' {
                return Ok(KeySpec::Named(NamedKey::Space));
            }
            return Ok(KeySpec::Char(c.to_lowercase().next().unwrap_or(c)));
        }
        let named = match s.to_ascii_lowercase().as_str() {
            "enter" | "return" => NamedKey::Enter,
            "tab" => NamedKey::Tab,
            "escape" | "esc" => NamedKey::Escape,
            "backspace" => NamedKey::Backspace,
            "delete" | "del" => NamedKey::Delete,
            "insert" | "ins" => NamedKey::Insert,
            "home" => NamedKey::Home,
            "end" => NamedKey::End,
            "pageup" => NamedKey::PageUp,
            "pagedown" => NamedKey::PageDown,
            "up" | "uparrow" => NamedKey::ArrowUp,
            "down" | "downarrow" => NamedKey::ArrowDown,
            "left" | "leftarrow" => NamedKey::ArrowLeft,
            "right" | "rightarrow" => NamedKey::ArrowRight,
            "space" => NamedKey::Space,
            "f1" => NamedKey::F1,
            "f2" => NamedKey::F2,
            "f3" => NamedKey::F3,
            "f4" => NamedKey::F4,
            "f5" => NamedKey::F5,
            "f6" => NamedKey::F6,
            "f7" => NamedKey::F7,
            "f8" => NamedKey::F8,
            "f9" => NamedKey::F9,
            "f10" => NamedKey::F10,
            "f11" => NamedKey::F11,
            "f12" => NamedKey::F12,
            _ => return Err(format!("unknown key '{s}'")),
        };
        Ok(KeySpec::Named(named))
    }

    /// The key as pressed, ignoring what Shift/AltGr did to it
    /// (Ctrl+Shift+C is "c", Shift+1 is "1").
    pub fn from_event(ev: &KeyEvent) -> Option<KeySpec> {
        Self::from_key(&ev.key_without_modifiers())
    }

    pub fn from_key(key: &Key) -> Option<KeySpec> {
        match key {
            // a bare modifier press is never a key on its own
            Key::Named(
                NamedKey::Control
                | NamedKey::Shift
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::Hyper
                | NamedKey::CapsLock
                | NamedKey::NumLock,
            ) => None,
            Key::Named(NamedKey::Space) => Some(KeySpec::Named(NamedKey::Space)),
            Key::Named(n) => Some(KeySpec::Named(*n)),
            Key::Character(s) => {
                let c = s.chars().next()?;
                if c == ' ' {
                    Some(KeySpec::Named(NamedKey::Space))
                } else {
                    Some(KeySpec::Char(c.to_lowercase().next().unwrap_or(c)))
                }
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Copy,
    Paste,
    Search,
    ReloadConfig,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,
    ClearScrollback,
    ToggleFullscreen,
    SendString(String),
    /// Open a new Vibex window (optionally with a named profile).
    SpawnWindow(Option<String>),
    /// Send the key to the shell as normal (unbinds a default).
    PassThrough,
    /// A Lua function (index into the engine's callback table).
    Callback(usize),
    // tabs
    NewTab(Option<String>),
    CloseTab,
    NextTab,
    PrevTab,
    /// 1-based; 9 or more means the last tab.
    ActivateTab(usize),
    // panes
    SplitRight(Option<String>),
    SplitDown(Option<String>),
    ClosePane,
    FocusPane(crate::layout::Direction),
    ResizePane(crate::layout::Direction),
    TogglePaneZoom,
    RenameTab,
    /// Accent colour for the current tab (None clears it).
    TabColor(Option<u32>),
    // misc
    CommandPalette,
    PreviousPrompt,
    NextPrompt,
    QuickSelect,
    CopyMode,
    /// Type into every pane of the current tab at once (toggle).
    ToggleBroadcast,
    /// Open the palette on the SSH hosts.
    SshPicker,
    /// Connect to a host in a new tab.
    Ssh(String),
    CheckForUpdates,
    InstallUpdate,
    /// The Settings tab (optionally on a page: "about", "shortcuts", ...).
    OpenSettings(Option<String>),
    /// Show / hide this tab's floating pane (creates one with a shell).
    ToggleFloatingPane,
    /// Run a command in a floating pane that closes when it exits.
    Popup(String),
    /// Start / stop recording the focused pane (asciicast).
    ToggleRecording,
    /// Play a recording in a new tab.
    PlayRecording(String),
    /// Turn a recording into a GIF next to it.
    ExportGif(String),
    CopyLastOutput,
    CopyLastCommand,
    RerunLastCommand,
    /// Save the screen as a PNG (and copy it).
    ScreenshotPane,
    /// A picture of the last command and its output, with a title bar.
    ScreenshotCommand,
    /// Copy the selection as a picture.
    CopyImage,
    /// Save lines as text / ANSI / HTML / SVG / PNG / Markdown.
    SaveRange,
    /// Look for new nano / Neovim releases, and offer to install them.
    UpdateEditors,
    /// Mark the start of some lines to save (the end, if one is marked).
    Mark,
    MarkStart,
    MarkEnd,
    /// Select the whole scrollback and screen.
    SelectAll,
    /// Search every command run in any pane, window or past session.
    HistorySearch,
    /// A popup at the cursor: matching commands from history and file names.
    Completions,
    /// Show secrets that the guard is masking (toggle).
    RevealSecrets,
    /// Bring back the last closed tab, scrollback and all.
    ReopenClosedTab,
    /// Full screen, no bars, notifications held until you leave.
    ToggleFocusMode,
    /// The notes panel beside the pane.
    ToggleNotes,
    /// Write an HTML report of the audit log and open it.
    AuditReport,
    /// Check the audit log's hash chain.
    AuditVerify,
    /// Everything engagement mode collected (copy any of it).
    ShowLoot,
    /// Write the engagement's HTML report and open it.
    EngagementReport,
    /// Turn an engagement on (None = off).
    SetEngagement(Option<String>),
    /// Notify when this pane goes quiet (a build finishing, a log stopping).
    ToggleMonitorSilence,
    /// Hide this window; its shells keep running (`vibex attach` or a
    /// plain launch brings it back).
    DetachWindow,
    /// Save this window's tabs and splits as `layouts/<workspace>.lua`.
    SaveLayout,
    OpenLayout(String),
    /// Switch to (or attach) the workspace with this window id.
    Workspace(u64, String),
    /// Close every window, detached ones included.
    QuitAll,
}

/// Tab accent colours offered in the palette.
pub const TAB_COLORS: &[(&str, u32)] =
    &[("red", 0xe5534b), ("orange", 0xf38020), ("yellow", 0xe0a84a), ("green", 0x57ab5a), ("blue", 0x539bf5), ("purple", 0xb083f0), ("pink", 0xe879b9)];

pub fn parse_direction(s: &str) -> Option<crate::layout::Direction> {
    use crate::layout::Direction::*;
    Some(match s.to_ascii_lowercase().as_str() {
        "left" => Left,
        "right" => Right,
        "up" => Up,
        "down" => Down,
        _ => return None,
    })
}

impl Action {
    /// Built-in actions by name (as used by `vibex.action.X` in Lua).
    pub fn named(name: &str) -> Option<Action> {
        Some(match name {
            "Copy" => Action::Copy,
            "Paste" => Action::Paste,
            "Search" => Action::Search,
            "ReloadConfig" => Action::ReloadConfig,
            "IncreaseFontSize" => Action::IncreaseFontSize,
            "DecreaseFontSize" => Action::DecreaseFontSize,
            "ResetFontSize" => Action::ResetFontSize,
            "ScrollPageUp" => Action::ScrollPageUp,
            "ScrollPageDown" => Action::ScrollPageDown,
            "ScrollToTop" => Action::ScrollToTop,
            "ScrollToBottom" => Action::ScrollToBottom,
            "ClearScrollback" => Action::ClearScrollback,
            "ToggleFullscreen" => Action::ToggleFullscreen,
            "PassThrough" | "Nop" | "DisableDefault" => Action::PassThrough,
            "NewTab" => Action::NewTab(None),
            "CloseTab" => Action::CloseTab,
            "NextTab" => Action::NextTab,
            "PrevTab" => Action::PrevTab,
            "SplitRight" => Action::SplitRight(None),
            "SplitDown" => Action::SplitDown(None),
            "ClosePane" => Action::ClosePane,
            "TogglePaneZoom" => Action::TogglePaneZoom,
            "CommandPalette" => Action::CommandPalette,
            "PreviousPrompt" => Action::PreviousPrompt,
            "NextPrompt" => Action::NextPrompt,
            "QuickSelect" => Action::QuickSelect,
            "CopyMode" => Action::CopyMode,
            "ToggleBroadcast" => Action::ToggleBroadcast,
            "SshPicker" => Action::SshPicker,
            "CheckForUpdates" => Action::CheckForUpdates,
            "OpenSettings" => Action::OpenSettings(None),
            "ToggleFloatingPane" => Action::ToggleFloatingPane,
            "CopyLastOutput" => Action::CopyLastOutput,
            "SelectAll" => Action::SelectAll,
            "HistorySearch" => Action::HistorySearch,
            "Completions" => Action::Completions,
            "RevealSecrets" => Action::RevealSecrets,
            "ReopenClosedTab" => Action::ReopenClosedTab,
            "ToggleFocusMode" => Action::ToggleFocusMode,
            "ToggleNotes" => Action::ToggleNotes,
            "AuditReport" => Action::AuditReport,
            "AuditVerify" => Action::AuditVerify,
            "ShowLoot" => Action::ShowLoot,
            "EngagementReport" => Action::EngagementReport,
            "ToggleRecording" => Action::ToggleRecording,
            "CopyLastCommand" => Action::CopyLastCommand,
            "RerunLastCommand" => Action::RerunLastCommand,
            "ScreenshotPane" => Action::ScreenshotPane,
            "ScreenshotCommand" => Action::ScreenshotCommand,
            "CopyImage" => Action::CopyImage,
            "SaveRange" => Action::SaveRange,
            "Mark" => Action::Mark,
            "UpdateEditors" => Action::UpdateEditors,
            "MarkStart" => Action::MarkStart,
            "MarkEnd" => Action::MarkEnd,
            "ToggleMonitorSilence" => Action::ToggleMonitorSilence,
            "DetachWindow" => Action::DetachWindow,
            "SaveLayout" => Action::SaveLayout,
            "QuitAll" => Action::QuitAll,
            "About" => Action::OpenSettings(Some("about".into())),
            "InstallUpdate" => Action::InstallUpdate,
            "RenameTab" => Action::RenameTab,
            _ => return None,
        })
    }

    /// Human-readable name for the command palette.
    pub fn label(&self) -> String {
        use crate::layout::Direction::*;
        let dir = |d: &crate::layout::Direction| match d {
            Left => "left",
            Right => "right",
            Up => "up",
            Down => "down",
        };
        match self {
            Action::Copy => "Copy selection".into(),
            Action::Paste => "Paste".into(),
            Action::Search => "Find in scrollback".into(),
            Action::ReloadConfig => "Reload config".into(),
            Action::IncreaseFontSize => "Font size: bigger".into(),
            Action::DecreaseFontSize => "Font size: smaller".into(),
            Action::ResetFontSize => "Font size: reset".into(),
            Action::ScrollPageUp => "Scroll up a page".into(),
            Action::ScrollPageDown => "Scroll down a page".into(),
            Action::ScrollToTop => "Scroll to top".into(),
            Action::ScrollToBottom => "Scroll to bottom".into(),
            Action::ClearScrollback => "Clear scrollback".into(),
            Action::ToggleFullscreen => "Toggle fullscreen".into(),
            Action::SendString(s) => format!("Type {s:?}"),
            Action::SpawnWindow(None) => "New window".into(),
            Action::SpawnWindow(Some(p)) => format!("New window: {p}"),
            Action::PassThrough => "Pass key to shell".into(),
            Action::Callback(_) => "Lua function".into(),
            Action::NewTab(None) => "New tab".into(),
            Action::NewTab(Some(p)) => format!("New tab: {p}"),
            Action::CloseTab => "Close tab".into(),
            Action::NextTab => "Next tab".into(),
            Action::PrevTab => "Previous tab".into(),
            Action::ActivateTab(n) => format!("Go to tab {n}"),
            Action::SplitRight(None) => "Split right".into(),
            Action::SplitRight(Some(p)) => format!("Split right: {p}"),
            Action::SplitDown(None) => "Split down".into(),
            Action::SplitDown(Some(p)) => format!("Split down: {p}"),
            Action::ClosePane => "Close pane".into(),
            Action::FocusPane(d) => format!("Focus pane {}", dir(d)),
            Action::ResizePane(d) => format!("Grow pane {}", dir(d)),
            Action::TogglePaneZoom => "Zoom pane (toggle)".into(),
            Action::CommandPalette => "Command palette".into(),
            Action::PreviousPrompt => "Jump to previous prompt".into(),
            Action::NextPrompt => "Jump to next prompt".into(),
            Action::QuickSelect => "Quick select (copy a URL, path or hash)".into(),
            Action::CopyMode => "Copy mode (vi keys)".into(),
            Action::ToggleBroadcast => "Broadcast input to all panes in this tab (toggle)".into(),
            Action::SshPicker => "SSH: connect to a host…".into(),
            Action::Ssh(h) => format!("SSH: {h}"),
            Action::CheckForUpdates => "Check for updates".into(),
            Action::InstallUpdate => "Install update and restart".into(),
            Action::OpenSettings(None) => "Settings".into(),
            Action::OpenSettings(Some(p)) if p == "about" => "About Vibex".into(),
            Action::OpenSettings(Some(p)) => format!("Settings: {p}"),
            Action::ToggleFloatingPane => "Floating pane (show / hide)".into(),
            Action::Popup(cmd) => format!("Popup: {cmd}"),
            Action::ToggleRecording => "Record this pane (start / stop)".into(),
            Action::PlayRecording(p) => format!("Play recording: {}", crate::record::short_name(p)),
            Action::ExportGif(p) => format!("Export recording as GIF: {}", crate::record::short_name(p)),
            Action::CopyLastOutput => "Copy the last command's output".into(),
            Action::SelectAll => "Select all".into(),
            Action::HistorySearch => "Search command history".into(),
            Action::Completions => "Completions (history and files)".into(),
            Action::RevealSecrets => "Reveal masked secrets (toggle)".into(),
            Action::ReopenClosedTab => "Reopen closed tab".into(),
            Action::ToggleFocusMode => "Focus mode (toggle)".into(),
            Action::ToggleNotes => "Notes beside this pane (toggle)".into(),
            Action::AuditReport => "Export audit report".into(),
            Action::AuditVerify => "Verify the audit log".into(),
            Action::ShowLoot => "Engagement loot".into(),
            Action::EngagementReport => "Export engagement report".into(),
            Action::SetEngagement(None) => "Engagement: off".into(),
            Action::SetEngagement(Some(n)) => format!("Engagement: {n}"),
            Action::CopyLastCommand => "Copy the last command".into(),
            Action::RerunLastCommand => "Run the last command again".into(),
            Action::ScreenshotPane => "Screenshot this pane (save and copy)".into(),
            Action::ScreenshotCommand => "Screenshot the last command".into(),
            Action::CopyImage => "Copy the selection as an image".into(),
            Action::SaveRange => "Save lines as\u{2026} (text, ANSI, HTML, SVG, PNG, Markdown)".into(),
            Action::Mark => "Mark start / end of lines to save".into(),
            Action::UpdateEditors => "Update nano and Neovim".into(),
            Action::MarkStart => "Mark start of lines to save".into(),
            Action::MarkEnd => "Mark end and save the lines".into(),
            Action::ToggleMonitorSilence => "Notify when this pane goes quiet (toggle)".into(),
            Action::DetachWindow => "Detach this window (shells keep running)".into(),
            Action::SaveLayout => "Save this window as a layout".into(),
            Action::OpenLayout(n) => format!("Open layout: {n}"),
            Action::Workspace(_, n) => format!("Workspace: {n}"),
            Action::QuitAll => "Quit Vibex (every window and detached workspace)".into(),
            Action::RenameTab => "Rename tab".into(),
            Action::TabColor(None) => "Tab colour: none".into(),
            Action::TabColor(Some(c)) => {
                let name = TAB_COLORS.iter().find(|(_, v)| v == c).map_or(format!("#{c:06x}"), |(n, _)| n.to_string());
                format!("Tab colour: {name}")
            }
        }
    }

    pub const UNIT_NAMES: &'static [&'static str] = &[
        "Copy",
        "Paste",
        "Search",
        "ReloadConfig",
        "IncreaseFontSize",
        "DecreaseFontSize",
        "ResetFontSize",
        "ScrollPageUp",
        "ScrollPageDown",
        "ScrollToTop",
        "ScrollToBottom",
        "ClearScrollback",
        "ToggleFullscreen",
        "PassThrough",
        "NewTab",
        "CloseTab",
        "NextTab",
        "PrevTab",
        "SplitRight",
        "SplitDown",
        "ClosePane",
        "TogglePaneZoom",
        "CommandPalette",
        "PreviousPrompt",
        "NextPrompt",
        "QuickSelect",
        "CopyMode",
        "ToggleBroadcast",
        "SshPicker",
        "CheckForUpdates",
        "InstallUpdate",
        "OpenSettings",
        "About",
        "ToggleFloatingPane",
        "CopyLastOutput",
        "SelectAll",
        "HistorySearch",
        "Completions",
        "RevealSecrets",
        "ReopenClosedTab",
        "ToggleFocusMode",
        "ToggleNotes",
        "AuditReport",
        "AuditVerify",
        "ShowLoot",
        "EngagementReport",
        "ToggleRecording",
        "CopyLastCommand",
        "RerunLastCommand",
        "ScreenshotPane",
        "ScreenshotCommand",
        "CopyImage",
        "SaveRange",
        "Mark",
        "UpdateEditors",
        "MarkStart",
        "MarkEnd",
        "ToggleMonitorSilence",
        "DetachWindow",
        "SaveLayout",
        "QuitAll",
        "RenameTab",
    ];
}

impl KeySpec {
    /// The name `KeySpec::parse` reads back ("t", "enter", "f5", ",").
    pub fn name(&self) -> String {
        match self {
            KeySpec::Char(c) => c.to_string(),
            KeySpec::Named(n) => match n {
                NamedKey::Enter => "enter".into(),
                NamedKey::Tab => "tab".into(),
                NamedKey::Escape => "escape".into(),
                NamedKey::Backspace => "backspace".into(),
                NamedKey::Delete => "delete".into(),
                NamedKey::Insert => "insert".into(),
                NamedKey::Home => "home".into(),
                NamedKey::End => "end".into(),
                NamedKey::PageUp => "pageup".into(),
                NamedKey::PageDown => "pagedown".into(),
                NamedKey::ArrowUp => "up".into(),
                NamedKey::ArrowDown => "down".into(),
                NamedKey::ArrowLeft => "left".into(),
                NamedKey::ArrowRight => "right".into(),
                NamedKey::Space => "space".into(),
                other => format!("{other:?}").to_ascii_lowercase(),
            },
        }
    }
}

impl Mods {
    /// "CTRL|SHIFT" (what `Mods::parse` reads).
    pub fn spec(&self) -> String {
        let mut p = Vec::new();
        if self.leader {
            p.push("LEADER");
        }
        if self.ctrl {
            p.push("CTRL");
        }
        if self.alt {
            p.push("ALT");
        }
        if self.shift {
            p.push("SHIFT");
        }
        if self.sup {
            p.push("SUPER");
        }
        p.join("|")
    }
}

impl Action {
    /// A stable name for saving this action's shortcut in settings.lua, or
    /// None if it can't be rebound there (Lua functions, typed text...).
    pub fn id(&self) -> Option<String> {
        use crate::layout::Direction::*;
        let dir = |d: &crate::layout::Direction| match d {
            Left => "Left",
            Right => "Right",
            Up => "Up",
            Down => "Down",
        };
        Some(match self {
            Action::NewTab(Some(p)) => format!("NewTab:{p}"),
            Action::SplitRight(Some(p)) => format!("SplitRight:{p}"),
            Action::SplitDown(Some(p)) => format!("SplitDown:{p}"),
            Action::SpawnWindow(Some(p)) => format!("SpawnWindow:{p}"),
            Action::ActivateTab(n) => format!("ActivateTab:{n}"),
            Action::FocusPane(d) => format!("FocusPane:{}", dir(d)),
            Action::ResizePane(d) => format!("ResizePane:{}", dir(d)),
            Action::OpenSettings(Some(p)) if p == "about" => "About".into(),
            Action::OpenLayout(n) => format!("OpenLayout:{n}"),
            Action::Callback(_)
            | Action::SendString(_)
            | Action::Ssh(_)
            | Action::TabColor(_)
            | Action::PassThrough
            | Action::Popup(_)
            | Action::PlayRecording(_)
            | Action::SetEngagement(_)
            | Action::ExportGif(_)
            | Action::Workspace(..) => return None,
            other => Action::UNIT_NAMES.iter().find(|n| Action::named(n).as_ref() == Some(other))?.to_string(),
        })
    }

    pub fn from_id(id: &str) -> Option<Action> {
        if let Some((kind, arg)) = id.split_once(':') {
            return Some(match kind {
                "NewTab" => Action::NewTab(Some(arg.into())),
                "SplitRight" => Action::SplitRight(Some(arg.into())),
                "SplitDown" => Action::SplitDown(Some(arg.into())),
                "SpawnWindow" => Action::SpawnWindow(Some(arg.into())),
                "ActivateTab" => Action::ActivateTab(arg.parse().ok()?),
                "FocusPane" => Action::FocusPane(parse_direction(arg)?),
                "ResizePane" => Action::ResizePane(parse_direction(arg)?),
                "OpenLayout" => Action::OpenLayout(arg.into()),
                _ => return None,
            });
        }
        Action::named(id)
    }
}

impl Binding {
    /// Key chips for the Settings page: ["Ctrl", "Shift", "T"].
    pub fn chips(&self) -> Vec<String> {
        let d = self.describe();
        let d = d.strip_prefix("Leader ").map(|rest| format!("Leader+{rest}")).unwrap_or(d);
        // "Ctrl++" (the plus key) must stay one chip
        let mut out: Vec<String> = Vec::new();
        let mut rest = d.as_str();
        while let Some(i) = rest.find('+') {
            if i == 0 {
                break;
            }
            out.push(rest[..i].to_string());
            rest = &rest[i + 1..];
        }
        out.push(rest.to_string());
        out
    }

    /// "Ctrl+Shift+T", "LEADER g"
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.mods.ctrl {
            parts.push("Ctrl".into());
        }
        if self.mods.alt {
            parts.push("Alt".into());
        }
        if self.mods.shift {
            parts.push("Shift".into());
        }
        if self.mods.sup {
            parts.push("Win".into());
        }
        let key = match &self.key {
            KeySpec::Char(c) => c.to_uppercase().to_string(),
            KeySpec::Named(n) => match n {
                NamedKey::ArrowUp => "↑".into(),
                NamedKey::ArrowDown => "↓".into(),
                NamedKey::ArrowLeft => "←".into(),
                NamedKey::ArrowRight => "→".into(),
                other => format!("{other:?}"),
            },
        };
        parts.push(key);
        let combo = parts.join("+");
        if self.mods.leader {
            format!("Leader {combo}")
        } else {
            combo
        }
    }
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub key: KeySpec,
    pub mods: Mods,
    pub action: Action,
}

fn bind(key: &str, mods: &str, action: Action) -> Binding {
    Binding { key: KeySpec::parse(key).unwrap(), mods: Mods::parse(mods).unwrap(), action }
}

pub fn default_bindings() -> Vec<Binding> {
    vec![
        bind("c", "CTRL|SHIFT", Action::Copy),
        bind("v", "CTRL|SHIFT", Action::Paste),
        bind("Insert", "SHIFT", Action::Paste),
        bind("f", "CTRL|SHIFT", Action::Search),
        bind("r", "CTRL|SHIFT", Action::ReloadConfig),
        bind("n", "CTRL|SHIFT", Action::SpawnWindow(None)),
        bind("=", "CTRL", Action::IncreaseFontSize),
        bind("-", "CTRL", Action::DecreaseFontSize),
        bind("0", "CTRL", Action::ResetFontSize),
        bind("PageUp", "SHIFT", Action::ScrollPageUp),
        bind("PageDown", "SHIFT", Action::ScrollPageDown),
        bind("Home", "CTRL|SHIFT", Action::ScrollToTop),
        bind("End", "CTRL|SHIFT", Action::ScrollToBottom),
        bind("Enter", "ALT", Action::ToggleFullscreen),
        // tabs (Windows Terminal-style)
        bind("t", "CTRL|SHIFT", Action::NewTab(None)),
        bind("w", "CTRL|SHIFT", Action::ClosePane),
        bind("Tab", "CTRL", Action::NextTab),
        bind("Tab", "CTRL|SHIFT", Action::PrevTab),
        bind("1", "CTRL|ALT", Action::ActivateTab(1)),
        bind("2", "CTRL|ALT", Action::ActivateTab(2)),
        bind("3", "CTRL|ALT", Action::ActivateTab(3)),
        bind("4", "CTRL|ALT", Action::ActivateTab(4)),
        bind("5", "CTRL|ALT", Action::ActivateTab(5)),
        bind("6", "CTRL|ALT", Action::ActivateTab(6)),
        bind("7", "CTRL|ALT", Action::ActivateTab(7)),
        bind("8", "CTRL|ALT", Action::ActivateTab(8)),
        bind("9", "CTRL|ALT", Action::ActivateTab(9)),
        // panes
        bind("=", "ALT|SHIFT", Action::SplitRight(None)),
        bind("-", "ALT|SHIFT", Action::SplitDown(None)),
        bind("Left", "ALT", Action::FocusPane(crate::layout::Direction::Left)),
        bind("Right", "ALT", Action::FocusPane(crate::layout::Direction::Right)),
        bind("Up", "ALT", Action::FocusPane(crate::layout::Direction::Up)),
        bind("Down", "ALT", Action::FocusPane(crate::layout::Direction::Down)),
        bind("Left", "ALT|SHIFT", Action::ResizePane(crate::layout::Direction::Left)),
        bind("Right", "ALT|SHIFT", Action::ResizePane(crate::layout::Direction::Right)),
        bind("Up", "ALT|SHIFT", Action::ResizePane(crate::layout::Direction::Up)),
        bind("Down", "ALT|SHIFT", Action::ResizePane(crate::layout::Direction::Down)),
        bind("z", "CTRL|SHIFT", Action::TogglePaneZoom),
        // palette + shell integration
        bind("p", "CTRL|SHIFT", Action::CommandPalette),
        bind("Up", "CTRL|SHIFT", Action::PreviousPrompt),
        bind("Down", "CTRL|SHIFT", Action::NextPrompt),
        bind("q", "CTRL|SHIFT", Action::QuickSelect),
        bind("x", "CTRL|SHIFT", Action::CopyMode),
        bind("b", "CTRL|SHIFT", Action::ToggleBroadcast),
        bind(",", "CTRL", Action::OpenSettings(None)),
        bind("f", "ALT|SHIFT", Action::ToggleFloatingPane),
        bind("o", "CTRL|SHIFT", Action::CopyLastOutput),
        bind("s", "ALT|SHIFT", Action::ScreenshotPane),
        bind("c", "ALT|SHIFT", Action::ScreenshotCommand),
        bind("e", "ALT|SHIFT", Action::SaveRange),
        bind("m", "ALT|SHIFT", Action::Mark),
        bind("a", "CTRL|SHIFT", Action::SelectAll),
        bind("h", "CTRL|SHIFT", Action::HistorySearch),
        bind("space", "CTRL|SHIFT", Action::Completions),
        bind("t", "CTRL|ALT|SHIFT", Action::ReopenClosedTab),
        bind("Enter", "CTRL|SHIFT", Action::ToggleFocusMode),
        bind("n", "ALT|SHIFT", Action::ToggleNotes),
        bind("l", "ALT|SHIFT", Action::ShowLoot),
        bind("r", "ALT|SHIFT", Action::ToggleRecording),
        bind("d", "CTRL|SHIFT", Action::DetachWindow),
        bind("s", "CTRL|SHIFT", Action::SshPicker),
    ]
}

/// Labels for quick-select: home-row first; two letters when there are
/// more targets than letters (all labels the same length, so none is a
/// prefix of another).
pub fn hint_labels(n: usize) -> Vec<String> {
    const ALPHABET: &[u8] = b"asdfjklghqweruiopzxcvbnmty";
    if n <= ALPHABET.len() {
        return ALPHABET.iter().take(n).map(|&c| (c as char).to_string()).collect();
    }
    let mut out = Vec::with_capacity(n);
    'outer: for &a in ALPHABET {
        for &b in ALPHABET {
            if out.len() == n {
                break 'outer;
            }
            out.push(format!("{}{}", a as char, b as char));
        }
    }
    out
}

/// Merge user bindings over defaults: a user binding replaces any default
/// with the same key + modifiers.
pub fn merge(defaults: Vec<Binding>, user: Vec<Binding>) -> Vec<Binding> {
    let mut out: Vec<Binding> = defaults.into_iter().filter(|d| !user.iter().any(|u| u.key == d.key && u.mods == d.mods)).collect();
    out.extend(user);
    out
}

pub fn find<'a>(bindings: &'a [Binding], key: &KeySpec, mods: Mods) -> Option<&'a Binding> {
    bindings.iter().find(|b| &b.key == key && b.mods == mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for a in [
            Action::Copy,
            Action::NewTab(None),
            Action::NewTab(Some("Dev".into())),
            Action::ActivateTab(3),
            Action::FocusPane(crate::layout::Direction::Left),
            Action::OpenSettings(None),
            Action::OpenSettings(Some("about".into())),
        ] {
            let id = a.id().unwrap();
            assert_eq!(Action::from_id(&id), Some(a), "{id}");
        }
        assert!(Action::Callback(0).id().is_none());
        let b = Binding { key: KeySpec::Char('t'), mods: Mods::parse("CTRL|SHIFT").unwrap(), action: Action::NewTab(None) };
        assert_eq!(b.chips(), ["Ctrl", "Shift", "T"]);
        let plus = Binding { key: KeySpec::Char('+'), mods: Mods::parse("CTRL").unwrap(), action: Action::Copy };
        assert_eq!(plus.chips(), ["Ctrl", "+"]);
        assert_eq!(KeySpec::parse(&KeySpec::Named(NamedKey::PageUp).name()).unwrap(), KeySpec::Named(NamedKey::PageUp));
        assert_eq!(Mods::parse(&b.mods.spec()).unwrap(), b.mods);
    }

    #[test]
    fn parse_mods() {
        let m = Mods::parse("CTRL|SHIFT").unwrap();
        assert!(m.ctrl && m.shift && !m.alt && !m.leader);
        assert!(Mods::parse("ctrl+alt").unwrap().alt);
        assert!(Mods::parse("LEADER").unwrap().leader);
        assert_eq!(Mods::parse("").unwrap(), Mods::default());
        assert!(Mods::parse("HYPER").is_err());
    }

    #[test]
    fn parse_keys() {
        assert_eq!(KeySpec::parse("C").unwrap(), KeySpec::Char('c'));
        assert_eq!(KeySpec::parse("PageUp").unwrap(), KeySpec::Named(NamedKey::PageUp));
        assert_eq!(KeySpec::parse(" ").unwrap(), KeySpec::Named(NamedKey::Space));
        assert_eq!(KeySpec::parse("f12").unwrap(), KeySpec::Named(NamedKey::F12));
        assert!(KeySpec::parse("Bogus").is_err());
    }

    #[test]
    fn hint_labels_are_unique_and_prefix_free() {
        assert_eq!(hint_labels(3), vec!["a", "s", "d"]);
        let many = hint_labels(60);
        assert_eq!(many.len(), 60);
        assert!(many.iter().all(|l| l.len() == 2));
        let set: std::collections::HashSet<_> = many.iter().collect();
        assert_eq!(set.len(), 60);
    }

    #[test]
    fn user_overrides_default() {
        let user = vec![bind("c", "CTRL|SHIFT", Action::PassThrough)];
        let all = merge(default_bindings(), user);
        let m = Mods::parse("CTRL|SHIFT").unwrap();
        assert_eq!(find(&all, &KeySpec::Char('c'), m).unwrap().action, Action::PassThrough);
        assert_eq!(find(&all, &KeySpec::Char('v'), m).unwrap().action, Action::Paste);
    }
}
