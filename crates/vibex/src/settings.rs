//! Settings made in Vibex (the Settings tab, the command palette) are stored in
//! `~/.config/vibex/settings.lua`, a small machine-written override layer
//! applied after the user's `vibex.lua`. The user's own file is never
//! rewritten.

use crate::config::Config;
use mlua::{Lua, LuaOptions, StdLib, Table, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vibex_term::CursorShape;

pub fn path() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("settings.lua"))
}

/// One stored override.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl Val {
    fn lua(&self) -> String {
        match self {
            Val::Str(s) => format!("{s:?}"),
            Val::Num(n) => {
                if n.fract() == 0.0 {
                    format!("{n:.0}")
                } else {
                    format!("{n}")
                }
            }
            Val::Bool(b) => b.to_string(),
        }
    }
}

/// Keys we store. `cursor_*` and `smooth_scroll` map onto nested options.
pub const KEYS: &[&str] = &[
    "theme",
    "font_size",
    "opacity",
    "ligatures",
    "cursor_style",
    "cursor_blink",
    "status_bar",
    "smooth_scroll",
    "shell_marks",
    "dim_inactive_panes",
    "glass",
    "restore_session",
    "restore_scrollback",
    "command_history",
    "suggestions",
    "linux_commands",
    "editor_updates",
    "editor_updates",
    "secret_guard",
    "confirm_close",
    "audit_log",
    "font_family",
    "padding",
    "scrollback",
    "default_profile",
    "notifications",
    "clipboard_read",
    "remote_control",
    "check_updates",
    "update_feed",
    "confirm_risky",
    "audit_output",
    "engagement",
    "engagement_record",
    "scope_guard",
    "collect_loot",
    "right_click",
];

/// Top-level keys stored as-is (the cursor / scroll ones are nested).
const FLAT: &[&str] = &[
    "restore_scrollback",
    "command_history",
    "suggestions",
    "linux_commands",
    "theme",
    "font_size",
    "opacity",
    "ligatures",
    "status_bar",
    "shell_marks",
    "dim_inactive_panes",
    "glass",
    "restore_session",
    "font_family",
    "padding",
    "scrollback",
    "default_profile",
    "notifications",
    "clipboard_read",
    "remote_control",
    "check_updates",
    "update_feed",
    "secret_guard",
    "confirm_close",
    "confirm_risky",
    "audit_log",
    "audit_output",
    "engagement",
    "engagement_record",
    "scope_guard",
    "collect_loot",
    "right_click",
];

/// A shortcut set in Settings: key and modifiers, or None = no key.
pub type KeyOverride = Option<(String, String)>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub values: BTreeMap<String, Val>,
    /// Shortcut overrides by action id (`keys::Action::id`).
    pub keys: BTreeMap<String, KeyOverride>,
}

impl Settings {
    pub fn set(&mut self, key: &str, v: Val) {
        debug_assert!(KEYS.contains(&key), "{key}");
        self.values.insert(key.to_string(), v);
    }

    /// As a Lua table using the real config option names.
    pub fn to_lua(&self) -> String {
        let mut out = String::from(
            "-- Written by Vibex (Settings tab / command palette).\n\
             -- Applied after vibex.lua. Delete this file to undo them all.\nreturn {\n",
        );
        let mut cursor = Vec::new();
        for (k, v) in &self.values {
            match k.as_str() {
                "cursor_style" => cursor.push(format!("style = {}", v.lua())),
                "cursor_blink" => cursor.push(format!("blink = {}", v.lua())),
                "smooth_scroll" => out.push_str(&format!("  scroll = {{ smooth = {} }},\n", v.lua())),
                other => out.push_str(&format!("  {other} = {},\n", v.lua())),
            }
        }
        if !cursor.is_empty() {
            out.push_str(&format!("  cursor = {{ {} }},\n", cursor.join(", ")));
        }
        if !self.keys.is_empty() {
            out.push_str("  keys = {\n");
            for (id, v) in &self.keys {
                match v {
                    Some((key, mods)) => out.push_str(&format!("    [{id:?}] = {{ key = {key:?}, mods = {mods:?} }},\n")),
                    None => out.push_str(&format!("    [{id:?}] = false,\n")),
                }
            }
            out.push_str("  },\n");
        }
        out.push_str("}\n");
        out
    }

    pub fn parse(src: &str) -> Result<Settings, String> {
        let lua = Lua::new_with(StdLib::NONE, LuaOptions::default()).map_err(|e| e.to_string())?;
        let v: Value = lua.load(src).set_name("settings.lua").eval().map_err(|e| e.to_string())?;
        let Value::Table(t) = v else { return Err("settings.lua must return a table".into()) };
        let mut s = Settings::default();
        let get = |t: &Table, k: &str| -> Option<Val> {
            match t.get::<Value>(k).ok()? {
                Value::String(x) => Some(Val::Str(x.to_string_lossy().to_string())),
                Value::Integer(i) => Some(Val::Num(i as f64)),
                Value::Number(n) => Some(Val::Num(n)),
                Value::Boolean(b) => Some(Val::Bool(b)),
                _ => None,
            }
        };
        for k in FLAT.iter().copied() {
            if let Some(v) = get(&t, k) {
                s.values.insert(k.into(), v);
            }
        }
        if let Ok(Value::Table(c)) = t.get::<Value>("cursor") {
            if let Some(v) = get(&c, "style") {
                s.values.insert("cursor_style".into(), v);
            }
            if let Some(v) = get(&c, "blink") {
                s.values.insert("cursor_blink".into(), v);
            }
        }
        if let Ok(Value::Table(sc)) = t.get::<Value>("scroll") {
            if let Some(v) = get(&sc, "smooth") {
                s.values.insert("smooth_scroll".into(), v);
            }
        }
        if let Ok(Value::Table(keys)) = t.get::<Value>("keys") {
            for (id, v) in keys.pairs::<String, Value>().flatten() {
                match v {
                    Value::Boolean(false) => {
                        s.keys.insert(id, None);
                    }
                    Value::Table(b) => {
                        let key: Option<String> = b.get("key").ok();
                        let mods: String = b.get("mods").unwrap_or_default();
                        if let Some(key) = key {
                            s.keys.insert(id, Some((key, mods)));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(s)
    }

    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path).ok().and_then(|src| Settings::parse(&src).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if self.values.is_empty() && self.keys.is_empty() {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("lua.tmp");
        std::fs::write(&tmp, self.to_lua())?;
        std::fs::rename(&tmp, path)
    }

    /// Apply the overrides to a config (after vibex.lua).
    pub fn apply(&self, c: &mut Config) {
        for (k, v) in &self.values {
            match (k.as_str(), v) {
                ("theme", Val::Str(name)) => {
                    if let Some(t) = vibex_render::Theme::builtin(name) {
                        c.theme = t;
                    }
                }
                ("font_size", Val::Num(n)) => c.font_size_pt = (*n as f32).clamp(4.0, 72.0),
                ("opacity", Val::Num(n)) => c.opacity = (*n as f32).clamp(0.0, 1.0),
                ("ligatures", Val::Bool(b)) => c.ligatures = *b,
                ("cursor_style", Val::Str(s)) => {
                    c.cursor_shape = match s.as_str() {
                        "bar" => CursorShape::Bar,
                        "underline" => CursorShape::Underline,
                        _ => CursorShape::Block,
                    }
                }
                ("cursor_blink", Val::Bool(b)) => c.cursor_blink = *b,
                ("status_bar", Val::Bool(b)) => c.status_bar = *b,
                ("smooth_scroll", Val::Bool(b)) => c.smooth_scroll = *b,
                ("shell_marks", Val::Bool(b)) => c.shell_marks = *b,
                ("dim_inactive_panes", Val::Bool(b)) => c.dim_inactive_panes = *b,
                ("glass", Val::Bool(b)) => c.glass = *b,
                ("restore_session", Val::Bool(b)) => c.restore_session = *b,
                ("restore_scrollback", Val::Bool(b)) => c.restore_scrollback = *b,
                ("command_history", Val::Bool(b)) => c.command_history = *b,
                ("suggestions", Val::Bool(b)) => c.suggestions = *b,
                ("linux_commands", Val::Bool(b)) => c.linux_commands = *b,
                ("editor_updates", Val::Bool(b)) => c.editor_updates = *b,
                ("secret_guard", Val::Bool(b)) => c.secret_guard = *b,
                ("confirm_close", Val::Bool(b)) => c.confirm_close = *b,
                ("audit_log", Val::Bool(b)) => c.audit_log = *b,
                ("font_family", Val::Str(name)) => {
                    if let Some((regular, bold, italic, bold_italic)) = crate::config::resolve_family(name) {
                        c.fonts.fallbacks.retain(|f| *f != regular);
                        c.fonts.regular = regular;
                        c.fonts.bold = bold;
                        c.fonts.italic = italic;
                        c.fonts.bold_italic = bold_italic;
                    }
                }
                ("padding", Val::Num(n)) => {
                    let x = n.clamp(0.0, 64.0).round() as usize;
                    c.padding = (x, (x as f64 * 0.8).round() as usize);
                }
                ("scrollback", Val::Num(n)) => c.scrollback = n.clamp(0.0, 1_000_000.0) as usize,
                ("default_profile", Val::Str(p)) => {
                    if let Some(found) = c.profile(p).map(|p| p.name.clone()) {
                        c.default_profile = found;
                    }
                }
                ("notifications", Val::Bool(b)) => c.notifications = *b,
                ("clipboard_read", Val::Bool(b)) => c.clipboard_read = *b,
                ("remote_control", Val::Bool(b)) => c.remote_control = *b,
                ("check_updates", Val::Bool(b)) => c.check_updates = *b,
                ("confirm_risky", Val::Bool(b)) => c.confirm_risky = *b,
                ("audit_output", Val::Bool(b)) => c.audit_output = *b,
                ("engagement_record", Val::Bool(b)) => c.engagement_record = *b,
                ("scope_guard", Val::Bool(b)) => c.scope_guard = *b,
                ("collect_loot", Val::Bool(b)) => c.collect_loot = *b,
                ("engagement", Val::Str(e)) => c.engagement = (!e.trim().is_empty()).then(|| e.trim().to_string()),
                ("right_click", Val::Str(m)) => c.right_click_menu = m != "paste",
                ("update_feed", Val::Str(u)) if (u.is_empty() || u.starts_with("https://")) => {
                    c.update_feed = u.clone();
                }
                _ => {}
            }
        }
        for (id, v) in &self.keys {
            let Some(action) = crate::keys::Action::from_id(id) else { continue };
            c.bindings.retain(|b| b.action != action);
            if let Some((key, mods)) = v {
                if let (Ok(key), Ok(mods)) = (crate::keys::KeySpec::parse(key), crate::keys::Mods::parse(mods)) {
                    // a key does one thing: take it from whatever had it
                    c.bindings.retain(|b| !(b.key == key && b.mods == mods));
                    c.bindings.insert(0, crate::keys::Binding { key, mods, action });
                }
            }
        }
    }
}

/// A setting change chosen in the palette.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Theme(String),
    FontSize(f32),
    Opacity(f32),
    CursorStyle(&'static str),
    /// Flip a boolean option (one of the bool KEYS).
    Toggle(&'static str),
    /// Set any stored option.
    Set(&'static str, Val),
    /// Drop one stored option (back to vibex.lua's value).
    Unset(&'static str),
    /// Set (or, with None, remove) the key for an action id.
    Bind(String, KeyOverride),
    /// Drop a shortcut override.
    Unbind(String),
    /// Drop every shortcut override.
    ResetKeys,
    Reset,
}

pub fn bool_of(c: &Config, key: &str) -> bool {
    match key {
        "ligatures" => c.ligatures,
        "cursor_blink" => c.cursor_blink,
        "status_bar" => c.status_bar,
        "smooth_scroll" => c.smooth_scroll,
        "shell_marks" => c.shell_marks,
        "dim_inactive_panes" => c.dim_inactive_panes,
        "glass" => c.glass,
        "restore_session" => c.restore_session,
        "restore_scrollback" => c.restore_scrollback,
        "command_history" => c.command_history,
        "suggestions" => c.suggestions,
        "linux_commands" => c.linux_commands,
        "editor_updates" => c.editor_updates,
        "secret_guard" => c.secret_guard,
        "confirm_close" => c.confirm_close,
        "audit_log" => c.audit_log,
        "confirm_risky" => c.confirm_risky,
        "audit_output" => c.audit_output,
        "engagement_record" => c.engagement_record,
        "scope_guard" => c.scope_guard,
        "collect_loot" => c.collect_loot,
        "notifications" => c.notifications,
        "clipboard_read" => c.clipboard_read,
        "remote_control" => c.remote_control,
        "check_updates" => c.check_updates,
        _ => false,
    }
}

/// Apply `change` to `cfg` and record it in `s`.
pub fn change(s: &mut Settings, cfg: &mut Config, ch: &Change) {
    match ch {
        Change::Theme(name) => s.set("theme", Val::Str(name.clone())),
        Change::FontSize(pt) => s.set("font_size", Val::Num((*pt as f64 * 2.0).round() / 2.0)),
        Change::Opacity(o) => s.set("opacity", Val::Num((*o as f64 * 100.0).round() / 100.0)),
        Change::CursorStyle(st) => s.set("cursor_style", Val::Str(st.to_string())),
        Change::Toggle(key) => {
            let now = bool_of(cfg, key);
            s.set(key, Val::Bool(!now));
        }
        Change::Set(key, v) => s.set(key, v.clone()),
        Change::Unset(key) => {
            s.values.remove(*key);
        }
        Change::Bind(id, v) => {
            s.keys.insert(id.clone(), v.clone());
        }
        Change::Unbind(id) => {
            s.keys.remove(id);
        }
        Change::ResetKeys => s.keys.clear(),
        Change::Reset => {
            s.values.clear();
            s.keys.clear();
        }
    }
    s.apply(cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_apply() {
        let mut s = Settings::default();
        let mut c = Config::default();
        change(&mut s, &mut c, &Change::Theme("vibex-ember".into()));
        change(&mut s, &mut c, &Change::FontSize(12.5));
        change(&mut s, &mut c, &Change::Opacity(0.85));
        change(&mut s, &mut c, &Change::CursorStyle("bar"));
        change(&mut s, &mut c, &Change::Toggle("ligatures"));
        change(&mut s, &mut c, &Change::Toggle("smooth_scroll"));
        assert_eq!(c.font_size_pt, 12.5);
        assert_eq!(c.opacity, 0.85);
        assert_eq!(c.cursor_shape, CursorShape::Bar);
        assert!(!c.ligatures);
        assert!(!c.smooth_scroll);
        assert_eq!(c.theme.background, vibex_render::Theme::builtin("vibex-ember").unwrap().background);
        // the schemes beyond Vibex's own work the same way, and survive a save
        change(&mut s, &mut c, &Change::Theme("catppuccin-mocha".into()));
        assert_eq!(c.theme.identify(), Some("catppuccin-mocha"));
        let mut again = Config::default();
        Settings::parse(&s.to_lua()).unwrap().apply(&mut again);
        assert_eq!(again.theme.identify(), Some("catppuccin-mocha"));
        change(&mut s, &mut c, &Change::Theme("vibex-ember".into()));

        let src = s.to_lua();
        let back = Settings::parse(&src).unwrap();
        assert_eq!(back, s, "{src}");
        // the written file is also a valid vibex.lua-style table
        assert!(src.contains("cursor = { style = \"bar\" }"));
        assert!(src.contains("scroll = { smooth = false }"));

        change(&mut s, &mut c, &Change::Reset);
        assert!(s.values.is_empty());
    }

    #[test]
    fn new_options_and_shortcuts_round_trip() {
        let mut s = Settings::default();
        let mut c = Config::default();
        change(&mut s, &mut c, &Change::Set("padding", Val::Num(12.0)));
        change(&mut s, &mut c, &Change::Set("scrollback", Val::Num(50000.0)));
        change(&mut s, &mut c, &Change::Set("update_feed", Val::Str("https://example.com/u.txt".into())));
        change(&mut s, &mut c, &Change::Toggle("notifications"));
        change(&mut s, &mut c, &Change::Bind("NewTab".into(), Some(("n".into(), "CTRL|ALT".into()))));
        change(&mut s, &mut c, &Change::Bind("Copy".into(), None));
        assert_eq!(c.padding, (12, 10));
        assert_eq!(c.scrollback, 50000);
        assert_eq!(c.update_feed, "https://example.com/u.txt");
        assert!(!c.notifications);
        use crate::keys::{find, Action, KeySpec, Mods};
        let b = find(&c.bindings, &KeySpec::Char('n'), Mods::parse("CTRL|ALT").unwrap()).unwrap();
        assert_eq!(b.action, Action::NewTab(None));
        assert!(!c.bindings.iter().any(|b| b.action == Action::Copy), "unbound");
        let src = s.to_lua();
        assert_eq!(Settings::parse(&src).unwrap(), s, "{src}");
        // a bad feed URL is ignored, not applied
        change(&mut s, &mut c, &Change::Set("update_feed", Val::Str("http://evil".into())));
        assert_eq!(c.update_feed, "https://example.com/u.txt");
        change(&mut s, &mut c, &Change::Reset);
        assert!(s.values.is_empty() && s.keys.is_empty());
    }

    #[test]
    fn parse_is_sandboxed() {
        assert!(Settings::parse("os.remove('x') return {}").is_err());
    }
}
