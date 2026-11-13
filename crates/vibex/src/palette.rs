//! Command palette model: a list of commands filtered by a fuzzy query.

use crate::config::Config;
use crate::keys::{Action, Binding};
use vibex_render::{PaletteView, Theme};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Action(Action),
    /// Change and save a setting (settings.lua).
    Setting(crate::settings::Change),
    OpenConfig,
    /// A command from the history: Enter types it at the prompt,
    /// Shift+Enter runs it.
    History(String),
    /// Copy this (loot).
    Copy(String),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub label: String,
    pub hint: String,
    pub command: Command,
}

pub struct Palette {
    pub query: String,
    pub selected: usize,
    entries: Vec<Entry>,
    filtered: Vec<usize>,
}

/// Subsequence fuzzy match. Higher is better; None if not all query chars
/// appear in order. Rewards consecutive runs and word starts.
pub fn score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut s = 0;
    let mut ti = 0;
    let mut prev: Option<usize> = None;
    // letters skipped between matches: "set lig" fits "Settings: ligatures"
    // better than "Settings: theme Ayu Light"
    let mut gaps = 0i32;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let pos = (ti..t.len()).find(|&i| t[i] == q)?;
        s += 1;
        if prev == Some(pos.wrapping_sub(1)) {
            s += 5;
        }
        if pos == 0 || !t[pos - 1].is_alphanumeric() {
            s += 3;
        }
        if let Some(p) = prev {
            gaps += (pos - p - 1).min(30) as i32;
        }
        prev = Some(pos);
        ti = pos + 1;
    }
    // then prefer shorter labels for equal matches
    Some(s * 100 - gaps * 4 - t.len() as i32)
}

fn hint_for(bindings: &[Binding], a: &Action) -> String {
    bindings.iter().find(|b| &b.action == a).map(|b| b.describe()).unwrap_or_default()
}

/// Everything the palette can do, given the current config.
pub fn entries(cfg: &Config) -> Vec<Entry> {
    let mut actions = vec![
        Action::NewTab(None),
        Action::SplitRight(None),
        Action::SplitDown(None),
        Action::ClosePane,
        Action::CloseTab,
        Action::NextTab,
        Action::PrevTab,
        Action::TogglePaneZoom,
        Action::RenameTab,
        Action::Search,
        Action::QuickSelect,
        Action::CopyMode,
        Action::ToggleBroadcast,
        Action::ToggleFloatingPane,
        Action::CopyLastOutput,
        Action::SelectAll,
        Action::HistorySearch,
        Action::ReopenClosedTab,
        Action::ToggleFocusMode,
        Action::ToggleNotes,
        Action::RevealSecrets,
        Action::AuditReport,
        Action::AuditVerify,
        Action::ShowLoot,
        Action::EngagementReport,
        Action::SetEngagement(None),
        Action::ToggleRecording,
        Action::CopyLastCommand,
        Action::RerunLastCommand,
        Action::ScreenshotPane,
        Action::ScreenshotCommand,
        Action::CopyImage,
        Action::SaveRange,
        Action::MarkStart,
        Action::MarkEnd,
        Action::ToggleMonitorSilence,
        Action::DetachWindow,
        Action::SaveLayout,
        Action::QuitAll,
        Action::SshPicker,
        Action::CheckForUpdates,
        Action::UpdateEditors,
        Action::InstallUpdate,
        Action::Copy,
        Action::Paste,
        Action::PreviousPrompt,
        Action::NextPrompt,
        Action::ClearScrollback,
        Action::ScrollToTop,
        Action::ScrollToBottom,
        Action::IncreaseFontSize,
        Action::DecreaseFontSize,
        Action::ResetFontSize,
        Action::ToggleFullscreen,
        Action::ReloadConfig,
        Action::SpawnWindow(None),
    ];
    for (_, c) in crate::keys::TAB_COLORS {
        actions.push(Action::TabColor(Some(*c)));
    }
    actions.push(Action::TabColor(None));
    for h in crate::ssh::hosts() {
        actions.push(Action::Ssh(h));
    }
    for n in crate::engagement::list().into_iter().take(10) {
        actions.push(Action::SetEngagement(Some(n)));
    }
    for r in crate::record::list().into_iter().take(10) {
        let p = r.to_string_lossy().into_owned();
        actions.push(Action::PlayRecording(p.clone()));
        actions.push(Action::ExportGif(p));
    }
    for p in &cfg.profiles {
        actions.push(Action::NewTab(Some(p.name.clone())));
        actions.push(Action::SplitRight(Some(p.name.clone())));
        actions.push(Action::SplitDown(Some(p.name.clone())));
    }
    let mut out: Vec<Entry> = actions.into_iter().map(|a| Entry { label: a.label(), hint: hint_for(&cfg.bindings, &a), command: Command::Action(a) }).collect();
    // SSH hosts: when you last connected, and the rule's label
    let recent = crate::ssh::recent();
    let now = crate::history::now();
    for e in out.iter_mut() {
        if let Command::Action(Action::Ssh(h)) = &e.command {
            let mut parts = Vec::new();
            if let Some(label) = crate::ssh::rule_for(h, &cfg.ssh_hosts).and_then(|r| r.label.clone()) {
                parts.push(label);
            }
            if let Some((_, when)) = recent.iter().find(|(r, _)| r == h) {
                parts.push(format!("last {}", crate::history::ago(*when, now)));
            }
            e.hint = parts.join(" · ");
        }
    }
    out.extend(setting_entries(cfg));
    out.push(Entry { label: "Open config file (vibex.lua)".into(), hint: String::new(), command: Command::OpenConfig });
    out
}

/// "Settings: …" entries, each showing the current value.
fn setting_entries(cfg: &Config) -> Vec<Entry> {
    use crate::settings::{bool_of, Change};
    use vibex_term::CursorShape;
    let mut out = Vec::new();
    let mark = |on: bool| if on { "● current".to_string() } else { String::new() };
    let mut push = |label: String, hint: String, change: Change| {
        out.push(Entry { label: format!("Settings: {label}"), hint, command: Command::Setting(change) });
    };
    let now = cfg.theme.identify();
    for id in Theme::ids() {
        push(format!("theme {}", Theme::title(id)), mark(now == Some(id)), Change::Theme(id.to_string()));
    }
    let pt = cfg.font_size_pt;
    push("font size bigger".into(), format!("{pt}pt"), Change::FontSize(pt + 1.0));
    push("font size smaller".into(), format!("{pt}pt"), Change::FontSize((pt - 1.0).max(4.0)));
    for o in [1.0f32, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4] {
        let current = (cfg.opacity - o).abs() < 0.005;
        push(format!("opacity {}%", (o * 100.0).round()), mark(current), Change::Opacity(o));
    }
    for (name, shape) in [("block", CursorShape::Block), ("bar", CursorShape::Bar), ("underline", CursorShape::Underline)] {
        push(format!("cursor {name}"), mark(cfg.cursor_shape == shape), Change::CursorStyle(name));
    }
    for (key, label) in [
        ("ligatures", "ligatures"),
        ("cursor_blink", "cursor blink"),
        ("status_bar", "status bar"),
        ("smooth_scroll", "smooth scrolling"),
        ("shell_marks", "prompt marks"),
        ("dim_inactive_panes", "dim inactive panes"),
        ("glass", "glass look"),
        ("restore_session", "restore session on start"),
    ] {
        let on = bool_of(cfg, key);
        push(format!("{label} (turn {})", if on { "off" } else { "on" }), if on { "on" } else { "off" }.into(), Change::Toggle(key));
    }
    push("reset all to vibex.lua".into(), String::new(), Change::Reset);
    out
}

impl Palette {
    pub fn new(entries: Vec<Entry>) -> Self {
        let mut p = Self { query: String::new(), selected: 0, filtered: Vec::new(), entries };
        p.refilter();
        p
    }

    fn refilter(&mut self) {
        let mut scored: Vec<(i32, usize)> = self.entries.iter().enumerate().filter_map(|(i, e)| score(&self.query, &e.label).map(|s| (s, i))).collect();
        if !self.query.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
    }

    pub fn push(&mut self, s: &str) {
        self.query.push_str(s);
        self.refilter();
    }

    pub fn pop(&mut self) {
        self.query.pop();
        self.refilter();
    }

    pub fn move_selection(&mut self, delta: isize) {
        let n = self.filtered.len();
        if n > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
        }
    }

    pub fn current(&self) -> Option<&Entry> {
        self.filtered.get(self.selected).map(|&i| &self.entries[i])
    }

    pub fn view(&self) -> PaletteView {
        PaletteView {
            query: self.query.clone(),
            items: self.filtered.iter().map(|&i| (self.entries[i].label.clone(), self.entries[i].hint.clone())).collect(),
            selected: self.selected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_scoring() {
        assert!(score("spr", "Split right").is_some());
        assert!(score("xyz", "Split right").is_none());
        assert!(score("sr", "Split right").unwrap() > score("sr", "Scroll to bottom: really").unwrap_or(i32::MIN));
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn filtering_and_selection() {
        let cfg = Config::default();
        let mut p = Palette::new(entries(&cfg));
        let total = p.view().items.len();
        assert!(total > 20);
        p.push("split right");
        let v = p.view();
        assert!(v.items.len() < total);
        assert!(v.items[0].0.starts_with("Split right"), "{:?}", v.items[0]);
        assert_eq!(p.current().unwrap().command, Command::Action(Action::SplitRight(None)));
        // the default binding is shown as a hint
        assert_eq!(v.items[0].1, "Alt+Shift+=");
        p.move_selection(1);
        assert_eq!(p.selected, 1);
        p.move_selection(-2);
        assert_eq!(p.selected, v.items.len() - 1);
        p.pop();
        assert!(p.query.ends_with("righ"));
    }

    #[cfg(windows)]
    #[test]
    fn profiles_and_themes_listed() {
        let cfg = Config::default();
        let labels: Vec<String> = entries(&cfg).into_iter().map(|e| e.label).collect();
        assert!(labels.iter().any(|l| l == "New tab: powershell"));
        assert!(labels.iter().any(|l| l == "Settings: theme Vibex Ember"));
        assert!(labels.iter().any(|l| l.starts_with("Open config")));
    }

    #[test]
    fn settings_show_current_values() {
        let cfg = Config::default();
        let e = entries(&cfg);
        let find = |l: &str| e.iter().find(|x| x.label == l).unwrap();
        assert_eq!(find("Settings: theme Tokyo Night").hint, "● current");
        assert_eq!(find("Settings: theme Vibex Night").hint, "");
        assert_eq!(find("Settings: opacity 40%").hint, "● current");
        assert_eq!(find("Settings: ligatures (turn off)").hint, "on");
        let mut p = Palette::new(e.clone());
        p.push("set lig");
        assert_eq!(p.current().unwrap().command, Command::Setting(crate::settings::Change::Toggle("ligatures")));
    }
}
