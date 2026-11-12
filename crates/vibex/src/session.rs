//! Session restore: tabs, split layout, folders and window size are saved to
//! `~/.config/vibex/session.lua` and reopened on the next start.
//!
//! The file is a Lua table written by Vibex (Vibex already embeds Lua, so no
//! new parser is needed). It is data only: it is evaluated in a fresh,
//! sandboxed Lua state with no libraries.

use crate::layout::{Node, SplitDir};
use mlua::{Lua, LuaOptions, StdLib, Table, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum SavedNode {
    /// `buffer`: file (in `buffers/`) with the pane's saved scrollback;
    /// `run`: typed into the shell once it starts (layout files).
    Pane {
        profile: String,
        cwd: Option<String>,
        buffer: Option<String>,
        run: Option<String>,
    },
    Split {
        dir: SplitDir,
        ratio: f32,
        a: Box<SavedNode>,
        b: Box<SavedNode>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedTab {
    pub root: SavedNode,
    /// Index of the focused pane in the tree's left-to-right order.
    pub active: usize,
    pub zoomed: bool,
    /// Name set by renaming the tab.
    pub title: Option<String>,
    pub color: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Saved {
    pub tabs: Vec<SavedTab>,
    pub active_tab: usize,
    /// Logical (unscaled) inner size.
    pub size: Option<(f64, f64)>,
    pub maximized: bool,
}

pub fn path() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("session.lua"))
}

/// Layout files: `config_dir/layouts/<name>.lua`.
pub fn layouts_dir() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("layouts"))
}

/// A name usable as a layout file name.
pub fn layout_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "workspace".into()
    } else {
        s
    }
}

/// Names of the saved layouts.
pub fn layouts() -> Vec<String> {
    let Some(dir) = layouts_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<String> = rd.flatten().filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".lua").map(str::to_string)).collect();
    out.sort();
    out
}

/// Where saved scrollback goes (`config_dir/buffers`).
pub fn buffers_dir() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("buffers"))
}

/// Delete saved buffers that `keep` doesn't name.
pub fn prune_buffers(keep: &[String]) {
    let Some(dir) = buffers_dir() else { return };
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".vt") && !keep.contains(&name) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// A Lua string literal.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn node_to_lua(n: &SavedNode, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match n {
        SavedNode::Pane { profile, cwd, buffer, run } => {
            out.push_str(&format!("{{ pane = {{ profile = {}", quote(profile)));
            if let Some(c) = cwd {
                out.push_str(&format!(", cwd = {}", quote(c)));
            }
            if let Some(b) = buffer {
                out.push_str(&format!(", buffer = {}", quote(b)));
            }
            if let Some(r) = run {
                out.push_str(&format!(", run = {}", quote(r)));
            }
            out.push_str(" } }");
        }
        SavedNode::Split { dir, ratio, a, b } => {
            let d = match dir {
                SplitDir::Horizontal => "right",
                SplitDir::Vertical => "down",
            };
            out.push_str(&format!("{{ split = \"{d}\", ratio = {ratio:.4},\n{pad}  a = "));
            node_to_lua(a, indent + 1, out);
            out.push_str(&format!(",\n{pad}  b = "));
            node_to_lua(b, indent + 1, out);
            out.push_str(&format!("\n{pad}}}"));
        }
    }
}

pub fn to_lua(s: &Saved) -> String {
    let mut out = String::from("-- Written by Vibex on exit; reopened on the next start.\n");
    out.push_str("-- Safe to delete. Turn off with config.restore_session = false.\nreturn {\n");
    out.push_str("  version = 1,\n");
    out.push_str(&format!("  active_tab = {},\n", s.active_tab + 1));
    if let Some((w, h)) = s.size {
        out.push_str(&format!("  width = {w:.0}, height = {h:.0},\n"));
    }
    out.push_str(&format!("  maximized = {},\n", s.maximized));
    out.push_str("  tabs = {\n");
    for t in &s.tabs {
        out.push_str(&format!("    {{ active = {}, zoomed = {}, ", t.active + 1, t.zoomed));
        if let Some(title) = &t.title {
            out.push_str(&format!("title = {}, ", quote(title)));
        }
        if let Some(c) = t.color {
            out.push_str(&format!("color = \"#{c:06x}\", "));
        }
        out.push_str("root = ");
        node_to_lua(&t.root, 3, &mut out);
        out.push_str(" },\n");
    }
    out.push_str("  },\n}\n");
    out
}

fn get_str(t: &Table, k: &str) -> Option<String> {
    match t.get::<Value>(k).ok()? {
        Value::String(s) => Some(s.to_string_lossy().to_string()),
        _ => None,
    }
}

fn parse_node(v: &Value, depth: usize) -> Result<SavedNode, String> {
    if depth > 32 {
        return Err("layout nested too deeply".into());
    }
    let Value::Table(t) = v else { return Err("layout node must be a table".into()) };
    if let Ok(Value::Table(p)) = t.get::<Value>("pane") {
        let profile = get_str(&p, "profile").ok_or("pane without a profile")?;
        // a buffer is only ever a plain file name inside buffers/
        let buffer = get_str(&p, "buffer")
            .filter(|b| !b.is_empty() && b.len() < 100 && b.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') && !b.contains(".."));
        return Ok(SavedNode::Pane { profile, cwd: get_str(&p, "cwd"), buffer, run: get_str(&p, "run") });
    }
    let dir = match get_str(t, "split").as_deref() {
        Some("right") => SplitDir::Horizontal,
        Some("down") => SplitDir::Vertical,
        _ => return Err("node is neither a pane nor a split".into()),
    };
    let ratio: f32 = t.get::<Option<f32>>("ratio").ok().flatten().unwrap_or(0.5).clamp(0.08, 0.92);
    let a = parse_node(&t.get::<Value>("a").map_err(|e| e.to_string())?, depth + 1)?;
    let b = parse_node(&t.get::<Value>("b").map_err(|e| e.to_string())?, depth + 1)?;
    Ok(SavedNode::Split { dir, ratio, a: Box::new(a), b: Box::new(b) })
}

pub fn parse(src: &str) -> Result<Saved, String> {
    // data only: no standard libraries at all
    let lua = Lua::new_with(StdLib::NONE, LuaOptions::default()).map_err(|e| e.to_string())?;
    let v: Value = lua.load(src).set_name("session.lua").eval().map_err(|e| e.to_string())?;
    let Value::Table(t) = v else { return Err("session.lua must return a table".into()) };
    let mut s = Saved::default();
    if let Ok(Value::Table(tabs)) = t.get::<Value>("tabs") {
        for tab in tabs.sequence_values::<Table>() {
            let tab = tab.map_err(|e| e.to_string())?;
            let root = parse_node(&tab.get::<Value>("root").map_err(|e| e.to_string())?, 0)?;
            let active = tab.get::<Option<usize>>("active").ok().flatten().unwrap_or(1).saturating_sub(1);
            let zoomed = tab.get::<Option<bool>>("zoomed").ok().flatten().unwrap_or(false);
            let title = get_str(&tab, "title");
            let color = get_str(&tab, "color").and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok());
            s.tabs.push(SavedTab { root, active, zoomed, title, color });
        }
    }
    s.active_tab = t.get::<Option<usize>>("active_tab").ok().flatten().unwrap_or(1).saturating_sub(1);
    let w = t.get::<Option<f64>>("width").ok().flatten();
    let h = t.get::<Option<f64>>("height").ok().flatten();
    if let (Some(w), Some(h)) = (w, h) {
        if w >= 200.0 && h >= 120.0 && w < 20000.0 && h < 20000.0 {
            s.size = Some((w, h));
        }
    }
    s.maximized = t.get::<Option<bool>>("maximized").ok().flatten().unwrap_or(false);
    if s.tabs.is_empty() {
        return Err("no tabs saved".into());
    }
    s.active_tab = s.active_tab.min(s.tabs.len() - 1);
    Ok(s)
}

pub fn load(path: &Path) -> Result<Saved, String> {
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse(&src)
}

/// Write atomically (temp file + rename) so a crash never leaves half a file.
pub fn save(path: &Path, s: &Saved) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("lua.tmp");
    std::fs::write(&tmp, to_lua(s))?;
    std::fs::rename(&tmp, path)
}

/// Rebuild a live layout tree, allocating pane ids with `spawn` (which
/// returns None if that pane couldn't start; the tree then closes up).
pub fn rebuild(n: &SavedNode, spawn: &mut dyn FnMut(&SavedNode) -> Option<u32>) -> Option<Node> {
    match n {
        SavedNode::Pane { .. } => spawn(n).map(Node::Leaf),
        SavedNode::Split { dir, ratio, a, b } => match (rebuild(a, spawn), rebuild(b, spawn)) {
            (Some(a), Some(b)) => Some(Node::Split { dir: *dir, ratio: *ratio, a: Box::new(a), b: Box::new(b) }),
            (Some(x), None) | (None, Some(x)) => Some(x),
            (None, None) => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Saved {
        Saved {
            tabs: vec![
                SavedTab {
                    root: SavedNode::Split {
                        dir: SplitDir::Horizontal,
                        ratio: 0.4,
                        a: Box::new(SavedNode::Pane {
                            profile: "powershell".into(),
                            cwd: Some(r#"C:\Users\me\"quoted" dir"#.into()),
                            buffer: Some("1-2.vt".into()),
                            run: Some("nvim .".into()),
                        }),
                        b: Box::new(SavedNode::Split {
                            dir: SplitDir::Vertical,
                            ratio: 0.5,
                            a: Box::new(SavedNode::Pane { profile: "wsl".into(), cwd: Some("/home/me".into()), buffer: None, run: None }),
                            b: Box::new(SavedNode::Pane { profile: "cmd".into(), cwd: None, buffer: None, run: None }),
                        }),
                    },
                    active: 2,
                    zoomed: false,
                    title: Some("build \"server\"".into()),
                    color: Some(0x57ab5a),
                },
                SavedTab {
                    root: SavedNode::Pane { profile: "ubuntu".into(), cwd: None, buffer: None, run: None },
                    active: 0,
                    zoomed: true,
                    title: None,
                    color: None,
                },
            ],
            active_tab: 1,
            size: Some((1100.0, 680.0)),
            maximized: false,
        }
    }

    #[test]
    fn round_trip() {
        let s = sample();
        let src = to_lua(&s);
        let back = parse(&src).unwrap();
        assert_eq!(back, s, "{src}");
    }

    #[test]
    fn rejects_garbage_and_has_no_libraries() {
        assert!(parse("return 5").is_err());
        assert!(parse("return { tabs = {} }").is_err());
        // no os/io in the sandbox
        assert!(parse("os.execute('echo hi') return {}").is_err());
        assert!(parse("return { tabs = { { root = { nope = 1 } } } }").is_err());
    }

    #[test]
    fn rebuild_skips_panes_that_fail() {
        let s = sample();
        let mut next = 0;
        let mut spawn = |n: &SavedNode| {
            let SavedNode::Pane { profile, .. } = n else { return None };
            if profile == "wsl" {
                None
            } else {
                next += 1;
                Some(next)
            }
        };
        let tree = rebuild(&s.tabs[0].root, &mut spawn).unwrap();
        assert_eq!(tree.panes(), vec![1, 2]);
    }
}
