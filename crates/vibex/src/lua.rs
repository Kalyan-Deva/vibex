//! The Lua config engine.
//!
//! `vibex.lua` returns a table of options. Keybindings map to built-in
//! actions (`vibex.action.Copy`, `vibex.action.SendString("x")`) or Lua
//! functions. `vibex.on(event, fn)` registers event handlers. Functions get
//! a `win` object whose methods *queue* commands (`Cmd`) that the app
//! applies after the call, so Lua never holds references into the terminal.

use crate::config::{base_command, default_fallbacks, expand_home, resolve_family, Config, Decorations, Leader, Profile, RendererKind, TabBarMode};
use crate::keys::{self, Action, Binding, KeySpec, Mods};
use mlua::{Function, Lua, RegistryKey, Table, UserData, UserDataMethods, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;
use vibex_render::{StatusLine, StatusSegment, Theme};
use vibex_term::CursorShape;

/// Something a Lua callback asked the app to do.
#[derive(Debug)]
pub enum Cmd {
    SendText(String),
    CopyText(String),
    Paste,
    Toast(String, f32),
    SetTitle(String),
    SpawnWindow(Option<String>),
    SetFontSize(f32),
    SetOpacity(f32),
    SetTheme(Theme),
    Scroll(isize),
    Search(Option<String>),
    Reload,
    Action(Action),
    SetTabTitle(Option<String>),
    SetTabColor(Option<u32>),
}

/// Read-only window state handed to callbacks.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub title: String,
    pub selection: String,
    pub profile: String,
    pub cols: usize,
    pub rows: usize,
    pub focused: bool,
    pub leader: bool,
    pub font_size: f32,
    pub opacity: f32,
    pub alt_screen: bool,
    pub scrollback: usize,
    pub tab_count: usize,
    /// 1-based.
    pub tab_index: usize,
    pub pane_count: usize,
    pub cwd: String,
    pub last_exit: Option<i32>,
}

pub enum EventArg {
    None,
    Str(String),
    Bool(bool),
}

pub struct Engine {
    lua: Lua,
    callbacks: Vec<RegistryKey>,
    handlers: HashMap<String, Vec<RegistryKey>>,
    watchdog: Rc<Watchdog>,
}

fn rt(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(msg.into())
}

/// Lua runs on the UI thread: a call (or loading vibex.lua) that runs
/// longer than this is stopped, so a stuck loop can't freeze every window.
const LUA_LIMIT: Duration = Duration::from_millis(250);
pub const TIMEOUT_CALL: &str = "vibex.lua: a callback ran too long (stopped after 250 ms)";
pub const TIMEOUT_LOAD: &str = "vibex.lua took too long to load (stopped after 250 ms)";

/// When the running call must stop, and whether one was stopped.
#[derive(Default)]
struct Watchdog {
    deadline: std::cell::Cell<Option<std::time::Instant>>,
    fired: std::cell::Cell<bool>,
}

/// Disarms the watchdog when the call is over.
struct Armed(Rc<Watchdog>);

impl Drop for Armed {
    fn drop(&mut self) {
        self.0.deadline.set(None);
        CO_DEADLINE.with(|d| d.set(None));
    }
}

impl Watchdog {
    fn arm(self: &Rc<Self>, lua: &Lua, limit: Duration) -> Armed {
        if self.fired.replace(false) {
            // the last call was stopped: back to the cheap check
            watch(lua, self.clone(), WATCH_EVERY);
        }
        let due = std::time::Instant::now() + limit;
        self.deadline.set(Some(due));
        CO_DEADLINE.with(|d| d.set(Some(due)));
        CO_FIRED.with(|f| f.set(false));
        Armed(self.clone())
    }
}

/// Instructions between deadline checks.
const WATCH_EVERY: u32 = 10_000;

/// Check the watchdog's deadline every `every` instructions.
fn watch(lua: &Lua, dog: Rc<Watchdog>, every: u32) {
    lua.set_hook(mlua::HookTriggers::new().every_nth_instruction(every), move |lua, _| match dog.deadline.get() {
        Some(t) if std::time::Instant::now() >= t => {
            if !dog.fired.replace(true) {
                // from now on every instruction fails, so a script that
                // pcalls its loop can't keep catching the error: it gets
                // raised in the outermost function too
                watch(lua, dog.clone(), 1);
            }
            Err(rt(TIMEOUT_CALL))
        }
        _ => Ok(mlua::VmState::Continue),
    });
}

/// A Lua state whose instruction hook enforces the watchdog's deadline.
fn watched_lua() -> (Lua, Rc<Watchdog>) {
    let lua = Lua::new();
    let dog = Rc::new(Watchdog::default());
    watch(&lua, dog.clone(), WATCH_EVERY);
    if let Err(e) = watch_coroutines(&lua) {
        eprintln!("vibex: lua watchdog for coroutines: {e}");
    }
    (lua, dog)
}

// mlua keeps one hook, for one Lua thread: a loop inside a coroutine runs
// past it. So every coroutine gets a small hook of its own (plain Lua API)
// that checks the same deadline. Lua runs on the UI thread only, so the
// deadline lives in thread-locals.
thread_local! {
    static CO_DEADLINE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    static CO_FIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

unsafe extern "C-unwind" fn coroutine_hook(state: *mut mlua::ffi::lua_State, _ar: *mut mlua::ffi::lua_Debug) {
    let due = CO_DEADLINE.with(|d| d.get()).is_some_and(|t| std::time::Instant::now() >= t);
    if due {
        CO_FIRED.with(|f| f.set(true));
        // every instruction fails from here, so a pcall'd loop can't go on
        mlua::ffi::lua_sethook(state, Some(coroutine_hook), mlua::ffi::LUA_MASKCOUNT, 1);
        mlua::ffi::lua_pushlstring(state, TIMEOUT_CALL.as_ptr().cast(), TIMEOUT_CALL.len());
        mlua::ffi::lua_error(state);
    }
}

/// `coroutine.create` (and `wrap`, built on it) with the deadline check.
fn watch_coroutines(lua: &Lua) -> mlua::Result<()> {
    let create = lua.create_function(|lua, f: Function| {
        let th = lua.create_thread(f)?;
        // SAFETY: only reads the thread just pushed and sets its hook
        unsafe {
            lua.exec_raw::<()>(th.clone(), |state| {
                let co = mlua::ffi::lua_tothread(state, -1);
                if !co.is_null() {
                    mlua::ffi::lua_sethook(co, Some(coroutine_hook), mlua::ffi::LUA_MASKCOUNT, WATCH_EVERY as i32);
                }
                mlua::ffi::lua_pop(state, 1);
            })?;
        }
        Ok(th)
    })?;
    let co: Table = lua.globals().get("coroutine")?;
    co.set("create", create)?;
    lua.load(
        r#"
        local create, resume = coroutine.create, coroutine.resume
        coroutine.wrap = function(f)
            local co = create(f)
            return function(...)
                local r = table.pack(resume(co, ...))
                if not r[1] then error(r[2], 2) end
                return table.unpack(r, 2, r.n)
            end
        end
        "#,
    )
    .set_name("vibex coroutines")
    .exec()
}

// ------------------------------------------------------------- win object

struct Win {
    snap: Snapshot,
    cmds: Rc<RefCell<Vec<Cmd>>>,
}

impl Win {
    fn push(&self, c: Cmd) {
        self.cmds.borrow_mut().push(c);
    }
}

impl UserData for Win {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // reads
        m.add_method("title", |_, w, ()| Ok(w.snap.title.clone()));
        m.add_method("selection", |_, w, ()| Ok(w.snap.selection.clone()));
        m.add_method("profile", |_, w, ()| Ok(w.snap.profile.clone()));
        m.add_method("size", |_, w, ()| Ok((w.snap.cols, w.snap.rows)));
        m.add_method("is_focused", |_, w, ()| Ok(w.snap.focused));
        m.add_method("leader_active", |_, w, ()| Ok(w.snap.leader));
        m.add_method("font_size", |_, w, ()| Ok(w.snap.font_size));
        m.add_method("opacity", |_, w, ()| Ok(w.snap.opacity));
        m.add_method("is_alt_screen", |_, w, ()| Ok(w.snap.alt_screen));
        m.add_method("scrollback_lines", |_, w, ()| Ok(w.snap.scrollback));
        m.add_method("tab_count", |_, w, ()| Ok(w.snap.tab_count));
        m.add_method("tab_index", |_, w, ()| Ok(w.snap.tab_index));
        m.add_method("pane_count", |_, w, ()| Ok(w.snap.pane_count));
        m.add_method("cwd", |_, w, ()| Ok(w.snap.cwd.clone()));
        m.add_method("last_exit_code", |_, w, ()| Ok(w.snap.last_exit));
        // commands
        m.add_method("send_text", |_, w, s: String| {
            w.push(Cmd::SendText(s));
            Ok(())
        });
        m.add_method("copy", |_, w, s: String| {
            w.push(Cmd::CopyText(s));
            Ok(())
        });
        m.add_method("paste", |_, w, ()| {
            w.push(Cmd::Paste);
            Ok(())
        });
        m.add_method("toast", |_, w, (msg, secs): (String, Option<f32>)| {
            w.push(Cmd::Toast(msg, secs.unwrap_or(3.0)));
            Ok(())
        });
        m.add_method("set_title", |_, w, s: String| {
            w.push(Cmd::SetTitle(s));
            Ok(())
        });
        m.add_method("spawn_window", |_, w, p: Option<String>| {
            w.push(Cmd::SpawnWindow(p));
            Ok(())
        });
        m.add_method("set_font_size", |_, w, pt: f32| {
            w.push(Cmd::SetFontSize(pt));
            Ok(())
        });
        m.add_method("set_opacity", |_, w, o: f32| {
            w.push(Cmd::SetOpacity(o));
            Ok(())
        });
        m.add_method("set_theme", |_, w, v: Value| {
            let theme = parse_theme(&v).map_err(rt)?;
            w.push(Cmd::SetTheme(theme));
            Ok(())
        });
        m.add_method("scroll", |_, w, n: isize| {
            w.push(Cmd::Scroll(n));
            Ok(())
        });
        m.add_method("search", |_, w, q: Option<String>| {
            w.push(Cmd::Search(q));
            Ok(())
        });
        m.add_method("reload_config", |_, w, ()| {
            w.push(Cmd::Reload);
            Ok(())
        });
        m.add_method("set_tab_title", |_, w, s: Option<String>| {
            w.push(Cmd::SetTabTitle(s.filter(|s| !s.is_empty())));
            Ok(())
        });
        m.add_method("set_tab_color", |_, w, s: Option<String>| {
            let c = match s {
                Some(h) => Some(parse_color(&h).map_err(rt)?),
                None => None,
            };
            w.push(Cmd::SetTabColor(c));
            Ok(())
        });
        m.add_method("perform", |_, w, v: Value| {
            let a = parse_plain_action(&v).map_err(rt)?;
            w.push(Cmd::Action(a));
            Ok(())
        });
    }
}

// ---------------------------------------------------------------- loading

const PRELUDE: &str = r##"
local vibex = { version = VIBEX_VERSION, themes = VIBEX_THEMES, __handlers = {} }
local act = {}
for _, name in ipairs(VIBEX_UNIT_ACTIONS) do act[name] = { __action = name } end
function act.SendString(s) return { __action = "SendString", arg = s } end
function act.SpawnWindow(profile) return { __action = "SpawnWindow", arg = profile } end
function act.NewTabWith(profile) return { __action = "NewTab", arg = profile } end
function act.SplitRightWith(profile) return { __action = "SplitRight", arg = profile } end
function act.SplitDownWith(profile) return { __action = "SplitDown", arg = profile } end
function act.ActivateTab(n) return { __action = "ActivateTab", arg = tostring(n) } end
function act.FocusPane(dir) return { __action = "FocusPane", arg = dir } end
function act.ResizePane(dir) return { __action = "ResizePane", arg = dir } end
function act.TabColor(hex) return { __action = "TabColor", arg = hex } end
function act.Popup(cmd) return { __action = "Popup", arg = cmd } end
vibex.action = act
function vibex.on(event, fn)
  assert(type(event) == "string", "vibex.on: event name must be a string")
  assert(type(fn) == "function", "vibex.on: handler must be a function")
  vibex.__handlers[event] = vibex.__handlers[event] or {}
  table.insert(vibex.__handlers[event], fn)
end
function vibex.hex(r, g, b) return string.format("#%02x%02x%02x", r, g, b) end
package.preload["vibex"] = function() return vibex end
VIBEX_MODULE = vibex
"##;

pub const EVENTS: &[&str] = &["startup", "config-reloaded", "title-changed", "focus-changed", "bell", "update-status"];

impl Engine {
    /// An engine with no user config (defaults only).
    pub fn empty() -> Engine {
        let (lua, watchdog) = watched_lua();
        Engine { lua, callbacks: Vec::new(), handlers: HashMap::new(), watchdog }
    }

    /// The message for a failed call: the watchdog's if it stopped it.
    fn call_error(&self, context: &str, e: &mlua::Error) -> String {
        if self.watchdog.fired.get() || CO_FIRED.with(|f| f.get()) {
            TIMEOUT_CALL.to_string()
        } else {
            format!("{context}{}", clean_error(&e.to_string()))
        }
    }

    /// Load `path` (if it exists) on top of the defaults.
    pub fn load(path: Option<&Path>) -> Result<(Engine, Config), String> {
        let mut config = Config::default();
        let Some(path) = path.filter(|p| p.is_file()) else {
            return Ok((Engine::empty(), config));
        };
        let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        // Notepad and others start UTF-8 files with a byte-order mark, which
        // Lua can't parse
        let src = without_bom(&src);
        let mut engine = Engine::empty();
        engine.install().map_err(|e| e.to_string())?;
        let _armed = engine.watchdog.arm(&engine.lua, LUA_LIMIT);
        let timed_out = |dog: &Watchdog, e: String| if dog.fired.get() { TIMEOUT_LOAD.to_string() } else { e };
        // let `require` find helper modules next to vibex.lua
        if let Some(dir) = path.parent() {
            let pkg: Table = engine.lua.globals().get("package").map_err(|e| e.to_string())?;
            let cur: String = pkg.get("path").unwrap_or_default();
            let _ = pkg.set("path", format!("{}\\?.lua;{cur}", dir.display()));
        }
        let value: Value = engine.lua.load(src).set_name("vibex.lua").eval().map_err(|e| timed_out(&engine.watchdog, clean_error(&e.to_string())))?;
        let table = match value {
            Value::Table(t) => t,
            Value::Nil => return Err("vibex.lua must `return` a config table".into()),
            other => return Err(format!("vibex.lua returned a {}, expected a table", other.type_name())),
        };
        let dog = engine.watchdog.clone();
        engine.apply(&table, &mut config).map_err(|e| timed_out(&dog, e))?;
        engine.collect_handlers().map_err(|e| timed_out(&dog, e.to_string()))?;
        Ok((engine, config))
    }

    fn install(&self) -> mlua::Result<()> {
        let g = self.lua.globals();
        g.set("VIBEX_VERSION", env!("CARGO_PKG_VERSION"))?;
        g.set("VIBEX_THEMES", self.lua.create_sequence_from(Theme::ids().into_iter())?)?;
        g.set("VIBEX_UNIT_ACTIONS", self.lua.create_sequence_from(Action::UNIT_NAMES.iter().copied())?)?;
        self.lua.load(PRELUDE).set_name("vibex-prelude").exec()?;
        let module: Table = g.get("VIBEX_MODULE")?;
        g.set("vibex", module)?;
        Ok(())
    }

    fn collect_handlers(&mut self) -> mlua::Result<()> {
        let module: Table = self.lua.globals().get("VIBEX_MODULE")?;
        let handlers: Table = module.get("__handlers")?;
        for pair in handlers.pairs::<String, Table>() {
            let (event, list) = pair?;
            if !EVENTS.contains(&event.as_str()) {
                return Err(rt(format!("vibex.on: unknown event '{event}' (known: {})", EVENTS.join(", "))));
            }
            for f in list.sequence_values::<Function>() {
                let key = self.lua.create_registry_value(f?)?;
                self.handlers.entry(event.clone()).or_default().push(key);
            }
        }
        Ok(())
    }

    pub fn has_handler(&self, event: &str) -> bool {
        self.handlers.get(event).is_some_and(|v| !v.is_empty())
    }

    // ---------------------------------------------------------- calling

    fn win(&self, snap: Snapshot) -> mlua::Result<(mlua::AnyUserData, Rc<RefCell<Vec<Cmd>>>)> {
        let cmds = Rc::new(RefCell::new(Vec::new()));
        let ud = self.lua.create_userdata(Win { snap, cmds: cmds.clone() })?;
        Ok((ud, cmds))
    }

    fn take(cmds: Rc<RefCell<Vec<Cmd>>>) -> Vec<Cmd> {
        std::mem::take(&mut *cmds.borrow_mut())
    }

    /// Run a key-bound Lua function.
    /// A Lua function given `(win, text)` (triggers).
    pub fn call_with_text(&self, idx: usize, snap: Snapshot, text: &str) -> Result<Vec<Cmd>, String> {
        let key = self.callbacks.get(idx).ok_or("stale trigger")?;
        let f: Function = self.lua.registry_value(key).map_err(|e| e.to_string())?;
        let (win, cmds) = self.win(snap).map_err(|e| e.to_string())?;
        let _armed = self.watchdog.arm(&self.lua, LUA_LIMIT);
        f.call::<()>((win, text.to_string())).map_err(|e| self.call_error("", &e))?;
        Ok(Self::take(cmds))
    }

    pub fn call(&self, idx: usize, snap: Snapshot) -> Result<Vec<Cmd>, String> {
        let key = self.callbacks.get(idx).ok_or("stale key binding")?;
        let f: Function = self.lua.registry_value(key).map_err(|e| e.to_string())?;
        let (win, cmds) = self.win(snap).map_err(|e| e.to_string())?;
        let _armed = self.watchdog.arm(&self.lua, LUA_LIMIT);
        f.call::<()>(win).map_err(|e| self.call_error("", &e))?;
        Ok(Self::take(cmds))
    }

    /// Run every handler for `event`.
    pub fn emit(&self, event: &str, snap: Snapshot, arg: EventArg) -> Result<Vec<Cmd>, String> {
        let Some(list) = self.handlers.get(event) else { return Ok(Vec::new()) };
        let (win, cmds) = self.win(snap).map_err(|e| e.to_string())?;
        for key in list {
            let f: Function = self.lua.registry_value(key).map_err(|e| e.to_string())?;
            // each handler gets the full time
            let _armed = self.watchdog.arm(&self.lua, LUA_LIMIT);
            let r = match &arg {
                EventArg::None => f.call::<()>(win.clone()),
                EventArg::Str(s) => f.call::<()>((win.clone(), s.clone())),
                EventArg::Bool(b) => f.call::<()>((win.clone(), *b)),
            };
            r.map_err(|e| self.call_error(&format!("{event} handler: "), &e))?;
        }
        Ok(Self::take(cmds))
    }

    /// Ask the last `update-status` handler for the status line.
    pub fn status(&self, snap: Snapshot) -> Result<Option<(StatusLine, Vec<Cmd>)>, String> {
        let Some(key) = self.handlers.get("update-status").and_then(|l| l.last()) else { return Ok(None) };
        let f: Function = self.lua.registry_value(key).map_err(|e| e.to_string())?;
        let (win, cmds) = self.win(snap).map_err(|e| e.to_string())?;
        let _armed = self.watchdog.arm(&self.lua, LUA_LIMIT);
        let v: Value = f.call(win).map_err(|e| self.call_error("update-status: ", &e))?;
        let line = parse_status(&v).map_err(|e| format!("update-status: {e}"))?;
        Ok(Some((line, Self::take(cmds))))
    }

    // ---------------------------------------------------------- parsing

    fn apply(&mut self, t: &Table, c: &mut Config) -> Result<(), String> {
        let mut user_keys = Vec::new();
        let mut disable_defaults = false;
        for pair in t.clone().pairs::<String, Value>() {
            let (k, v) = pair.map_err(|e| e.to_string())?;
            let ctx = |e: String| format!("config.{k}: {e}");
            match k.as_str() {
                "font" => parse_font(&v, c).map_err(ctx)?,
                "font_size" => c.font_size_pt = num(&v).map_err(ctx)? as f32,
                "theme" => c.theme = parse_theme(&v).map_err(ctx)?,
                "opacity" => c.opacity = (num(&v).map_err(ctx)? as f32).clamp(0.0, 1.0),
                "padding" => c.padding = parse_padding(&v).map_err(ctx)?,
                "window" => parse_window(&v, c).map_err(ctx)?,
                "cursor" => parse_cursor(&v, c).map_err(ctx)?,
                "scrollback" => c.scrollback = num(&v).map_err(ctx)?.max(0.0) as usize,
                "scroll" => parse_scroll(&v, c).map_err(ctx)?,
                "status_bar" => c.status_bar = boolean(&v).map_err(ctx)?,
                "ligatures" => c.ligatures = boolean(&v).map_err(ctx)?,
                "multi_click_ms" => c.multi_click_ms = num(&v).map_err(ctx)? as u64,
                "leader" => c.leader = Some(parse_leader(&v).map_err(ctx)?),
                "disable_default_keys" => disable_defaults = boolean(&v).map_err(ctx)?,
                "keys" => user_keys = self.parse_keys(&v).map_err(ctx)?,
                "triggers" => c.triggers = self.parse_triggers(&v).map_err(ctx)?,
                "ssh_hosts" => c.ssh_hosts = parse_ssh_hosts(&v).map_err(ctx)?,
                "secret_guard" => c.secret_guard = boolean(&v).map_err(ctx)?,
                "secret_patterns" => {
                    // { ["my token"] = "MYCO-[0-9a-f]{32}", ... }: added to the built-in ones
                    let t = table(&v).map_err(ctx)?;
                    let mut extra = Vec::new();
                    for pair in t.pairs::<String, String>() {
                        extra.push(pair.map_err(|e| ctx(e.to_string()))?);
                    }
                    extra.sort();
                    c.secrets = crate::secrets::Guard::new(&extra).map_err(ctx)?;
                }
                "confirm_commands" => {
                    // { "pattern", ... } or { ["why"] = "pattern" }; false = none
                    if matches!(v, Value::Boolean(false)) {
                        c.confirm_commands.clear();
                    } else {
                        let t = table(&v).map_err(ctx)?;
                        let mut out = crate::config::default_dangerous();
                        for pair in t.pairs::<Value, String>() {
                            let (k, pat) = pair.map_err(|e| ctx(e.to_string()))?;
                            let why = match k {
                                Value::String(s) => s.to_string_lossy().to_string(),
                                _ => format!("matches {pat}"),
                            };
                            let re = regex::Regex::new(&pat).map_err(|e| ctx(format!("bad pattern '{pat}': {e}")))?;
                            out.push((why, re));
                        }
                        c.confirm_commands = out;
                    }
                }
                "confirm_close" => c.confirm_close = boolean(&v).map_err(ctx)?,
                "audit_log" => c.audit_log = boolean(&v).map_err(ctx)?,
                "audit_output" => c.audit_output = boolean(&v).map_err(ctx)?,
                "confirm_risky" => c.confirm_risky = boolean(&v).map_err(ctx)?,
                "engagement" => {
                    let e = string(&v).map_err(ctx)?;
                    c.engagement = (!e.trim().is_empty()).then(|| e.trim().to_string());
                }
                "engagement_record" => c.engagement_record = boolean(&v).map_err(ctx)?,
                "scope_guard" => c.scope_guard = boolean(&v).map_err(ctx)?,
                "collect_loot" => c.collect_loot = boolean(&v).map_err(ctx)?,
                "quake" => {
                    c.quake = match &v {
                        Value::Boolean(false) => None,
                        Value::Table(t) => {
                            let key: String = t.get::<Option<String>>("key").map_err(|e| ctx(e.to_string()))?.unwrap_or_else(|| "`".into());
                            let mods: String = t.get::<Option<String>>("mods").map_err(|e| ctx(e.to_string()))?.unwrap_or_else(|| "WIN".into());
                            let h: f32 = t.get::<Option<f32>>("height").map_err(|e| ctx(e.to_string()))?.unwrap_or(0.45).clamp(0.15, 1.0);
                            crate::hotkey::parse(&key, &mods).map_err(ctx)?;
                            Some((key, mods, h))
                        }
                        _ => return Err(ctx("expected false or { key, mods, height }".into())),
                    }
                }
                "editor" => c.editor = Some(parse_editor(&v).map_err(ctx)?),
                "profiles" => parse_profiles(&v, c).map_err(ctx)?,
                "default_profile" => c.default_profile = string(&v).map_err(ctx)?,
                "tab_bar" => {
                    c.tab_bar = match string(&v).map_err(ctx)?.as_str() {
                        "auto" => TabBarMode::Auto,
                        "always" => TabBarMode::Always,
                        "never" => TabBarMode::Never,
                        o => return Err(ctx(format!("expected auto|always|never, got '{o}'"))),
                    }
                }
                "shell_marks" => c.shell_marks = boolean(&v).map_err(ctx)?,
                "dim_inactive_panes" => c.dim_inactive_panes = boolean(&v).map_err(ctx)?,
                "glass" => c.glass = boolean(&v).map_err(ctx)?,
                "prompt" => {
                    c.prompt = match string(&v).map_err(ctx)?.as_str() {
                        p @ ("default" | "dragon") => p.to_string(),
                        other => return Err(format!("config.prompt: \"{other}\" (use \"default\" or \"dragon\")")),
                    }
                }
                "title_bar" => {
                    c.title_bar = match string(&v).map_err(ctx)?.as_str() {
                        t @ ("tabs" | "classic") => t.to_string(),
                        other => return Err(format!("config.title_bar: \"{other}\" (use \"tabs\" or \"classic\")")),
                    }
                }
                "right_click" => {
                    c.right_click_menu = match string(&v).map_err(ctx)?.as_str() {
                        "menu" => true,
                        "paste" => false,
                        other => return Err(format!("config.right_click: \"{other}\" (use \"menu\" or \"paste\")")),
                    }
                }
                "restore_session" => c.restore_session = boolean(&v).map_err(ctx)?,
                "restore_scrollback" => c.restore_scrollback = boolean(&v).map_err(ctx)?,
                "command_history" => c.command_history = boolean(&v).map_err(ctx)?,
                "suggestions" => c.suggestions = boolean(&v).map_err(ctx)?,
                "linux_commands" => c.linux_commands = boolean(&v).map_err(ctx)?,
                "editor_updates" => c.editor_updates = boolean(&v).map_err(ctx)?,
                "clipboard_read" => c.clipboard_read = boolean(&v).map_err(ctx)?,
                "remote_control" => c.remote_control = boolean(&v).map_err(ctx)?,
                "notifications" => c.notifications = boolean(&v).map_err(ctx)?,
                "notify_long_commands" => {
                    c.notify_long_commands = num(&v).map_err(ctx)?.max(0.0) as f32;
                }
                "update_feed" => {
                    let url = string(&v).map_err(ctx)?;
                    if !url.is_empty() && !url.starts_with("https://") {
                        return Err(ctx("must be an https:// URL".into()));
                    }
                    c.update_feed = url;
                }
                "check_updates" => c.check_updates = boolean(&v).map_err(ctx)?,
                other => return Err(format!("unknown option config.{other}")),
            }
        }
        if c.profile(&c.default_profile).is_none() {
            return Err(format!(
                "config.default_profile: no profile named '{}' (have: {})",
                c.default_profile,
                c.profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        let defaults = if disable_defaults { Vec::new() } else { keys::default_bindings() };
        c.bindings = keys::merge(defaults, user_keys);
        if c.leader.is_none() && c.bindings.iter().any(|b| b.mods.leader) {
            return Err("config.keys: a binding uses LEADER but config.leader is not set".into());
        }
        Ok(())
    }

    fn parse_triggers(&mut self, v: &Value) -> Result<Vec<crate::config::Trigger>, String> {
        use crate::config::{Trigger, TriggerAction};
        let list = table(v)?;
        let mut out = Vec::new();
        for (i, item) in list.sequence_values::<Table>().enumerate() {
            let err = |e: String| format!("entry {}: {e}", i + 1);
            let item = item.map_err(|e| err(e.to_string()))?;
            let pattern: String = item.get("pattern").map_err(|_| err("missing `pattern` (a regex)".into()))?;
            let regex = regex::RegexBuilder::new(&pattern).size_limit(1 << 20).build().map_err(|e| err(format!("bad pattern: {e}")))?;
            let action: Value = item.get("action").map_err(|e| err(e.to_string()))?;
            let action = match action {
                Value::Function(f) => {
                    let k = self.lua.create_registry_value(f).map_err(|e| err(e.to_string()))?;
                    self.callbacks.push(k);
                    TriggerAction::Callback(self.callbacks.len() - 1)
                }
                Value::String(s) => match s.to_string_lossy().as_ref() {
                    "highlight" => {
                        let c: Value = item.get("color").unwrap_or(Value::Nil);
                        TriggerAction::Highlight(if c.is_nil() { 0x7a3b3b } else { color(&c).map_err(err)? })
                    }
                    "notify" => TriggerAction::Notify(item.get::<Option<String>>("title").ok().flatten().unwrap_or_else(|| "Trigger".into())),
                    "toast" => TriggerAction::Toast,
                    other => return Err(err(format!("unknown action '{other}' (highlight | notify | toast | a function)"))),
                },
                _ => return Err(err("`action` must be \"highlight\", \"notify\", \"toast\" or a function".into())),
            };
            out.push(Trigger { regex, action });
        }
        Ok(out)
    }

    fn parse_keys(&mut self, v: &Value) -> Result<Vec<Binding>, String> {
        let list = table(v)?;
        let mut out = Vec::new();
        for (i, item) in list.sequence_values::<Table>().enumerate() {
            let item = item.map_err(|e| format!("entry {}: {e}", i + 1))?;
            let err = |e: String| format!("entry {}: {e}", i + 1);
            let key: String = item.get("key").map_err(|_| err("missing `key`".into()))?;
            let mods: Option<String> = item.get("mods").map_err(|e| err(e.to_string()))?;
            let action: Value = item.get("action").map_err(|e| err(e.to_string()))?;
            let action = match action {
                Value::Function(f) => {
                    let k = self.lua.create_registry_value(f).map_err(|e| err(e.to_string()))?;
                    self.callbacks.push(k);
                    Action::Callback(self.callbacks.len() - 1)
                }
                Value::Nil => return Err(err("missing `action`".into())),
                other => parse_plain_action(&other).map_err(err)?,
            };
            out.push(Binding { key: KeySpec::parse(&key).map_err(err)?, mods: Mods::parse(mods.as_deref().unwrap_or("")).map_err(err)?, action });
        }
        Ok(out)
    }
}

/// mlua errors carry a traceback; keep the first line (it has file:line).
fn clean_error(e: &str) -> String {
    let first = e.lines().next().unwrap_or(e).trim();
    first.trim_start_matches("runtime error: ").trim_start_matches("syntax error: ").replace("[string \"vibex.lua\"]", "vibex.lua")
}

/// Watches vibex.lua for changes (mtime polling; cheap and has no
/// platform quirks).
pub struct ConfigWatcher {
    pub path: Option<std::path::PathBuf>,
    mtime: Option<std::time::SystemTime>,
}

pub enum Reload {
    Loaded(Box<(Engine, Config)>),
    Failed(String),
}

impl ConfigWatcher {
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        let mtime = Self::stat(&path);
        Self { path, mtime }
    }

    fn stat(path: &Option<std::path::PathBuf>) -> Option<std::time::SystemTime> {
        path.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
    }

    /// Reload now (manual reload).
    pub fn reload(&mut self) -> Reload {
        self.mtime = Self::stat(&self.path);
        match Engine::load(self.path.as_deref()) {
            Ok(r) => Reload::Loaded(Box::new(r)),
            Err(e) => Reload::Failed(e),
        }
    }

    /// Reload if the file changed since the last load attempt (including
    /// being created or deleted).
    pub fn poll(&mut self) -> Option<Reload> {
        if self.path.is_none() || Self::stat(&self.path) == self.mtime {
            return None;
        }
        Some(self.reload())
    }
}

// ------------------------------------------------------------ value helpers

fn table(v: &Value) -> Result<Table, String> {
    match v {
        Value::Table(t) => Ok(t.clone()),
        other => Err(format!("expected a table, got {}", other.type_name())),
    }
}

fn num(v: &Value) -> Result<f64, String> {
    match v {
        Value::Integer(i) => Ok(*i as f64),
        Value::Number(n) => Ok(*n),
        other => Err(format!("expected a number, got {}", other.type_name())),
    }
}

fn boolean(v: &Value) -> Result<bool, String> {
    match v {
        Value::Boolean(b) => Ok(*b),
        other => Err(format!("expected true/false, got {}", other.type_name())),
    }
}

fn string(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.to_string_lossy().to_string()),
        other => Err(format!("expected a string, got {}", other.type_name())),
    }
}

fn field(t: &Table, k: &str) -> Value {
    t.get::<Value>(k).unwrap_or(Value::Nil)
}

/// Reject keys not in `allowed` (catches typos).
fn check_keys(t: &Table, allowed: &[&str]) -> Result<(), String> {
    for pair in t.clone().pairs::<Value, Value>() {
        let (k, _) = pair.map_err(|e| e.to_string())?;
        if let Value::String(s) = &k {
            let s = s.to_string_lossy().to_string();
            if !allowed.contains(&s.as_str()) {
                return Err(format!("unknown field '{s}' (allowed: {})", allowed.join(", ")));
            }
        }
    }
    Ok(())
}

pub fn parse_color(s: &str) -> Result<u32, String> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return Err(format!("bad colour '{s}' (use \"#rrggbb\")"));
    }
    u32::from_str_radix(h, 16).map_err(|_| format!("bad colour '{s}'"))
}

fn color(v: &Value) -> Result<u32, String> {
    parse_color(&string(v)?)
}

/// `{ { match = "prod-*", color = "#c42b1c", label = "PROD", theme = "..." }, ... }`
fn parse_ssh_hosts(v: &Value) -> Result<Vec<crate::config::SshRule>, String> {
    let list = table(v)?;
    let mut out = Vec::new();
    for (i, item) in list.sequence_values::<Value>().enumerate() {
        let err = |e: String| format!("entry {}: {e}", i + 1);
        let item = item.map_err(|e| err(e.to_string()))?;
        let t = table(&item).map_err(err)?;
        check_keys(&t, &["match", "color", "label", "theme", "confirm"]).map_err(err)?;
        let pattern: String = t.get("match").map_err(|_| err("missing `match` (a host pattern like \"prod-*\")".into()))?;
        let opt = |k: &str| -> Value { t.get(k).unwrap_or(Value::Nil) };
        let color_v = opt("color");
        let theme_v = opt("theme");
        out.push(crate::config::SshRule {
            confirm: t.get::<Option<bool>>("confirm").map_err(|e| err(e.to_string()))?.unwrap_or(false),
            pattern,
            color: if color_v.is_nil() { None } else { Some(color(&color_v).map_err(err)?) },
            label: t.get::<Option<String>>("label").map_err(|e| err(e.to_string()))?,
            theme: if theme_v.is_nil() { None } else { Some(parse_theme(&theme_v).map_err(err)?) },
        });
    }
    Ok(out)
}

fn parse_plain_action(v: &Value) -> Result<Action, String> {
    let t = table(v).map_err(|_| "action must be vibex.action.<Name> or a function".to_string())?;
    let name: String = t.get("__action").map_err(|_| "not a vibex.action value".to_string())?;
    let arg: Option<String> = t.get("arg").map_err(|e| e.to_string())?;
    let dir = |a: &Option<String>| {
        a.as_deref().and_then(keys::parse_direction).ok_or_else(|| "direction must be \"Left\", \"Right\", \"Up\" or \"Down\"".to_string())
    };
    match name.as_str() {
        "SendString" => Ok(Action::SendString(arg.unwrap_or_default())),
        "Popup" => Ok(Action::Popup(arg.filter(|a| !a.trim().is_empty()).ok_or("Popup needs a command")?)),
        "SpawnWindow" => Ok(Action::SpawnWindow(arg)),
        "NewTab" => Ok(Action::NewTab(arg)),
        "SplitRight" => Ok(Action::SplitRight(arg)),
        "SplitDown" => Ok(Action::SplitDown(arg)),
        "ActivateTab" => {
            let n: usize = arg.as_deref().and_then(|s| s.parse().ok()).filter(|n| *n >= 1).ok_or("ActivateTab needs a tab number (1-based)")?;
            Ok(Action::ActivateTab(n))
        }
        "TabColor" => Ok(Action::TabColor(match arg {
            Some(h) => Some(parse_color(&h)?),
            None => None,
        })),
        "FocusPane" => Ok(Action::FocusPane(dir(&arg)?)),
        "ResizePane" => Ok(Action::ResizePane(dir(&arg)?)),
        n => Action::named(n).ok_or_else(|| format!("unknown action '{n}'")),
    }
}

/// `text` without a leading UTF-8 byte-order mark.
pub fn without_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Levenshtein distance (for "did you mean").
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (prev + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

pub fn parse_theme(v: &Value) -> Result<Theme, String> {
    match v {
        Value::String(_) => {
            let name = string(v)?;
            Theme::builtin(&name).ok_or_else(|| {
                // the closest ids: "catpuccin" -> catppuccin-*
                let mut close: Vec<(i32, &str)> =
                    Theme::ids().into_iter().filter_map(|id| crate::palette::score(&name.replace([' ', '-'], ""), id).map(|s| (s, id))).collect();
                close.sort_by(|a, b| b.0.cmp(&a.0));
                if close.is_empty() {
                    // a wrong letter rather than a missing one: "drakula"
                    let n = name.to_lowercase();
                    close = Theme::ids()
                        .into_iter()
                        .filter(|id| edit_distance(&n, id) <= 2 || id.split('-').next().is_some_and(|w| edit_distance(&n, w) <= 2))
                        .map(|id| (0, id))
                        .collect();
                }
                match close.iter().take(3).map(|(_, id)| *id).collect::<Vec<_>>() {
                    c if !c.is_empty() => {
                        format!("unknown theme '{name}': did you mean {}? (all {} are listed in docs/CONFIG.md)", c.join(", "), Theme::ids().len())
                    }
                    _ => format!("unknown theme '{name}' (built-in: {})", Theme::ids().join(", ")),
                }
            })
        }
        Value::Table(t) => {
            check_keys(t, &["base", "foreground", "background", "cursor", "cursor_text", "selection", "search_match", "ansi", "brights", "bold_is_bright"])?;
            let mut theme = match field(t, "base") {
                Value::Nil => Theme::default(),
                b => parse_theme(&b)?,
            };
            let set = |k: &str, slot: &mut u32| -> Result<(), String> {
                match field(t, k) {
                    Value::Nil => Ok(()),
                    v => {
                        *slot = color(&v).map_err(|e| format!("{k}: {e}"))?;
                        Ok(())
                    }
                }
            };
            set("foreground", &mut theme.foreground)?;
            set("background", &mut theme.background)?;
            set("cursor", &mut theme.cursor)?;
            set("cursor_text", &mut theme.cursor_text)?;
            set("selection", &mut theme.selection)?;
            set("search_match", &mut theme.search_match)?;
            for (k, offset) in [("ansi", 0), ("brights", 8)] {
                if let Value::Table(list) = field(t, k) {
                    for (i, c) in list.sequence_values::<Value>().enumerate().take(8) {
                        let c = c.map_err(|e| e.to_string())?;
                        theme.palette[offset + i] = color(&c).map_err(|e| format!("{k}[{}]: {e}", i + 1))?;
                    }
                }
            }
            if let Value::Boolean(b) = field(t, "bold_is_bright") {
                theme.bold_is_bright = b;
            }
            Ok(theme)
        }
        other => Err(format!("expected a theme name or table, got {}", other.type_name())),
    }
}

fn parse_font(v: &Value, c: &mut Config) -> Result<(), String> {
    let apply_family = |name: &str, c: &mut Config| -> Result<(), String> {
        let (r, b, i, bi) =
            resolve_family(name).ok_or_else(|| format!("font family '{name}' not found (looked in ~/.config/vibex/fonts and Windows fonts)"))?;
        c.fonts.regular = r;
        c.fonts.bold = b;
        c.fonts.italic = i;
        c.fonts.bold_italic = bi;
        Ok(())
    };
    match v {
        Value::String(_) => apply_family(&string(v)?, c),
        Value::Table(t) => {
            check_keys(t, &["family", "size", "ligatures", "regular", "bold", "italic", "bold_italic", "fallbacks"])?;
            if let v @ Value::String(_) = field(t, "family") {
                apply_family(&string(&v)?, c)?;
            }
            if let v @ (Value::Integer(_) | Value::Number(_)) = field(t, "size") {
                c.font_size_pt = num(&v)? as f32;
            }
            if let Value::Boolean(b) = field(t, "ligatures") {
                c.ligatures = b;
            }
            let path = |k: &str| -> Result<Option<String>, String> {
                match field(t, k) {
                    Value::Nil => Ok(None),
                    v => {
                        let p = expand_home(&string(&v)?);
                        if Path::new(&p).is_file() {
                            Ok(Some(p))
                        } else {
                            Err(format!("{k}: file not found: {p}"))
                        }
                    }
                }
            };
            if let Some(p) = path("regular")? {
                c.fonts.regular = p;
            }
            if let Some(p) = path("bold")? {
                c.fonts.bold = Some(p);
            }
            if let Some(p) = path("italic")? {
                c.fonts.italic = Some(p);
            }
            if let Some(p) = path("bold_italic")? {
                c.fonts.bold_italic = Some(p);
            }
            if let Value::Table(list) = field(t, "fallbacks") {
                let mut fb = Vec::new();
                for item in list.sequence_values::<String>() {
                    let item = item.map_err(|e| e.to_string())?;
                    let p = expand_home(&item);
                    if Path::new(&p).is_file() {
                        fb.push(p);
                    } else if let Some((r, ..)) = resolve_family(&item) {
                        fb.push(r);
                    } else {
                        return Err(format!("fallbacks: '{item}' is not a file or known family"));
                    }
                }
                fb.extend(default_fallbacks());
                c.fonts.fallbacks = fb;
            }
            Ok(())
        }
        other => Err(format!("expected a family name or table, got {}", other.type_name())),
    }
}

/// `editor = "nvim +{line} {file}"` or
/// `editor = { cmd = { "nvim", "+{line}", "{file}" }, env = { ... }, open_in = "popup" }`.
fn parse_editor(v: &Value) -> Result<crate::config::Editor, String> {
    use crate::config::{Editor, OpenIn};
    let open_in_of = |s: &str| -> Result<OpenIn, String> {
        Ok(match s {
            "popup" => OpenIn::Popup,
            "tab" => OpenIn::Tab,
            "split" => OpenIn::Split,
            "app" => OpenIn::App,
            o => return Err(format!("open_in: expected popup | tab | split | app, got '{o}'")),
        })
    };
    match v {
        Value::String(s) => {
            let cmd = crate::split_command(&s.to_string_lossy());
            if cmd.is_empty() {
                return Err("empty editor command".into());
            }
            Ok(Editor { cmd, env: Vec::new(), open_in: OpenIn::Popup })
        }
        Value::Table(t) => {
            let cmd = match t.get::<Value>("cmd").map_err(|e| e.to_string())? {
                Value::String(s) => crate::split_command(&s.to_string_lossy()),
                Value::Table(list) => list.sequence_values::<String>().collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?,
                _ => return Err("editor.cmd: a string or a list of strings".into()),
            };
            if cmd.is_empty() {
                return Err("editor.cmd is empty".into());
            }
            let mut env = Vec::new();
            if let Ok(Value::Table(e)) = t.get::<Value>("env") {
                for pair in e.pairs::<String, String>() {
                    env.push(pair.map_err(|e| e.to_string())?);
                }
            }
            let open_in = match t.get::<Option<String>>("open_in").map_err(|e| e.to_string())? {
                Some(s) => open_in_of(&s)?,
                None => OpenIn::Popup,
            };
            Ok(Editor { cmd, env, open_in })
        }
        other => Err(format!("expected a command string or table, got {}", other.type_name())),
    }
}

fn parse_padding(v: &Value) -> Result<(usize, usize), String> {
    match v {
        Value::Table(t) => {
            check_keys(t, &["x", "y"])?;
            let x = num(&field(t, "x")).unwrap_or(10.0).max(0.0) as usize;
            let y = num(&field(t, "y")).unwrap_or(8.0).max(0.0) as usize;
            Ok((x, y))
        }
        other => {
            let n = num(other)?.max(0.0) as usize;
            Ok((n, n))
        }
    }
}

fn parse_window(v: &Value, c: &mut Config) -> Result<(), String> {
    let t = table(v)?;
    check_keys(&t, &["width", "height", "padding", "opacity", "renderer", "decorations"])?;
    if let v @ Value::String(_) = field(&t, "decorations") {
        c.decorations = match string(&v)?.as_str() {
            "vibex" | "custom" => Decorations::Vibex,
            "native" | "windows" => Decorations::Native,
            o => return Err(format!("decorations: expected \"vibex\" or \"native\", got '{o}'")),
        };
    }
    if let v @ (Value::Integer(_) | Value::Number(_)) = field(&t, "width") {
        c.window_size.0 = num(&v)?;
    }
    if let v @ (Value::Integer(_) | Value::Number(_)) = field(&t, "height") {
        c.window_size.1 = num(&v)?;
    }
    match field(&t, "padding") {
        Value::Nil => {}
        p => c.padding = parse_padding(&p)?,
    }
    if let v @ (Value::Integer(_) | Value::Number(_)) = field(&t, "opacity") {
        c.opacity = (num(&v)? as f32).clamp(0.0, 1.0);
    }
    if let v @ Value::String(_) = field(&t, "renderer") {
        c.renderer = match string(&v)?.as_str() {
            "auto" => RendererKind::Auto,
            "gpu" => RendererKind::Gpu,
            "cpu" => RendererKind::Cpu,
            o => return Err(format!("renderer: expected auto|gpu|cpu, got '{o}'")),
        };
    }
    Ok(())
}

fn parse_cursor(v: &Value, c: &mut Config) -> Result<(), String> {
    let t = table(v)?;
    check_keys(&t, &["style", "blink", "blink_ms"])?;
    if let v @ Value::String(_) = field(&t, "style") {
        c.cursor_shape = match string(&v)?.as_str() {
            "block" => CursorShape::Block,
            "bar" | "beam" => CursorShape::Bar,
            "underline" => CursorShape::Underline,
            o => return Err(format!("style: expected block|bar|underline, got '{o}'")),
        };
    }
    if let Value::Boolean(b) = field(&t, "blink") {
        c.cursor_blink = b;
    }
    if let v @ (Value::Integer(_) | Value::Number(_)) = field(&t, "blink_ms") {
        c.blink_interval_ms = num(&v)?.max(50.0) as u64;
    }
    Ok(())
}

fn parse_scroll(v: &Value, c: &mut Config) -> Result<(), String> {
    let t = table(v)?;
    check_keys(&t, &["wheel_lines", "smooth"])?;
    if let v @ (Value::Integer(_) | Value::Number(_)) = field(&t, "wheel_lines") {
        c.wheel_lines = num(&v)?.max(1.0) as usize;
    }
    if let Value::Boolean(b) = field(&t, "smooth") {
        c.smooth_scroll = b;
    }
    Ok(())
}

fn parse_leader(v: &Value) -> Result<Leader, String> {
    let t = table(v)?;
    check_keys(&t, &["key", "mods", "timeout_ms"])?;
    let key = KeySpec::parse(&string(&field(&t, "key")).map_err(|e| format!("key: {e}"))?)?;
    let mods = match field(&t, "mods") {
        Value::Nil => Mods::default(),
        m => Mods::parse(&string(&m)?)?,
    };
    let timeout = num(&field(&t, "timeout_ms")).unwrap_or(1000.0).max(100.0) as u64;
    Ok(Leader { key, mods, timeout: Duration::from_millis(timeout) })
}

fn parse_profiles(v: &Value, c: &mut Config) -> Result<(), String> {
    let t = table(v)?;
    for pair in t.pairs::<String, Value>() {
        let (name, p) = pair.map_err(|e| e.to_string())?;
        let err = |e: String| format!("{name}: {e}");
        let pt = table(&p).map_err(err)?;
        check_keys(&pt, &["cmd", "cwd", "env"]).map_err(err)?;
        let argv: Vec<String> = match field(&pt, "cmd") {
            Value::String(s) => s.to_string_lossy().split_whitespace().map(str::to_string).collect(),
            Value::Table(list) => list.sequence_values::<String>().collect::<Result<_, _>>().map_err(|e| err(e.to_string()))?,
            Value::Nil => return Err(err("missing `cmd`".into())),
            o => return Err(err(format!("cmd: expected a string or list, got {}", o.type_name()))),
        };
        let Some((program, args)) = argv.split_first() else { return Err(err("cmd is empty".into())) };
        let mut cmd = base_command(program.clone());
        cmd.args = args.to_vec();
        if let v @ Value::String(_) = field(&pt, "cwd") {
            cmd.cwd = Some(expand_home(&string(&v)?));
        }
        if let Value::Table(env) = field(&pt, "env") {
            for kv in env.pairs::<String, String>() {
                let (k, v) = kv.map_err(|e| err(e.to_string()))?;
                cmd.env.push((k, v));
            }
        }
        c.profiles.retain(|p| !p.name.eq_ignore_ascii_case(&name));
        c.profiles.push(Profile { name, command: cmd });
    }
    Ok(())
}

fn parse_segments(v: &Value) -> Result<Vec<StatusSegment>, String> {
    match v {
        Value::Nil => Ok(Vec::new()),
        Value::String(_) => Ok(vec![StatusSegment { text: string(v)?, fg: None, bg: None, bold: false }]),
        Value::Table(t) => {
            let mut out = Vec::new();
            for item in t.clone().sequence_values::<Value>() {
                let item = item.map_err(|e| e.to_string())?;
                match &item {
                    Value::String(_) => out.push(StatusSegment { text: string(&item)?, fg: None, bg: None, bold: false }),
                    Value::Table(s) => {
                        check_keys(s, &["text", "fg", "bg", "bold"])?;
                        let opt_color = |k: &str| match field(s, k) {
                            Value::Nil => Ok(None),
                            v => color(&v).map(Some),
                        };
                        out.push(StatusSegment {
                            text: string(&field(s, "text")).map_err(|e| format!("segment text: {e}"))?,
                            fg: opt_color("fg")?,
                            bg: opt_color("bg")?,
                            bold: matches!(field(s, "bold"), Value::Boolean(true)),
                        });
                    }
                    o => return Err(format!("segment: expected string or table, got {}", o.type_name())),
                }
            }
            Ok(out)
        }
        o => Err(format!("expected a string or list of segments, got {}", o.type_name())),
    }
}

fn parse_status(v: &Value) -> Result<StatusLine, String> {
    match v {
        Value::Table(t) if !matches!(field(t, "left"), Value::Nil) || !matches!(field(t, "right"), Value::Nil) => {
            Ok(StatusLine { left: parse_segments(&field(t, "left"))?, right: parse_segments(&field(t, "right"))? })
        }
        other => Ok(StatusLine { left: parse_segments(other)?, right: Vec::new() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn load_src(src: &str) -> Result<(Engine, Config), String> {
        let dir = std::env::temp_dir().join(format!("vibex-test-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vibex.lua");
        std::fs::File::create(&path).unwrap().write_all(src.as_bytes()).unwrap();
        let r = Engine::load(Some(&path));
        let _ = std::fs::remove_dir_all(&dir);
        r
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    #[test]
    fn missing_file_gives_defaults() {
        let (_, c) = Engine::load(Some(Path::new("Z:/does/not/exist.lua"))).unwrap();
        assert_eq!(c.font_size_pt, 11.0);
    }

    #[test]
    fn basic_options() {
        let (_, c) = load_src(
            r##"return {
                font_size = 13,
                theme = "vibex-ember",
                opacity = 0.75,
                padding = { x = 4, y = 2 },
                cursor = { style = "bar", blink = false },
                scrollback = 500,
                status_bar = false,
            }"##,
        )
        .unwrap();
        assert_eq!(c.font_size_pt, 13.0);
        assert_eq!(c.theme.background, Theme::builtin("vibex-ember").unwrap().background);
        assert_eq!(c.opacity, 0.75);
        assert_eq!(c.padding, (4, 2));
        assert_eq!(c.cursor_shape, CursorShape::Bar);
        assert!(!c.cursor_blink);
        assert_eq!(c.scrollback, 500);
        assert!(!c.status_bar);
    }

    #[test]
    fn custom_theme_table() {
        let (_, c) = load_src(r##"return { theme = { base = "vibex-day", background = "#101010", ansi = { "#000001", "#000002" } } }"##).unwrap();
        assert_eq!(c.theme.background, 0x101010);
        assert_eq!(c.theme.palette[0], 1);
        assert_eq!(c.theme.palette[1], 2);
        assert_eq!(c.theme.foreground, Theme::builtin("vibex-day").unwrap().foreground);
    }

    #[test]
    fn typos_are_errors() {
        let e = load_src("return { fontsize = 12 }").err().unwrap();
        assert!(e.contains("unknown option config.fontsize"), "{e}");
        let e = load_src("return { cursor = { stlye = 'bar' } }").err().unwrap();
        assert!(e.contains("unknown field 'stlye'"), "{e}");
        let e = load_src("return { theme = 'nope' }").err().unwrap();
        assert!(e.contains("unknown theme"), "{e}");
    }

    #[test]
    fn syntax_error_reports_line() {
        let e = load_src("return {\n  font_size = ,\n}").err().unwrap();
        assert!(e.contains("vibex.lua"), "{e}");
        assert!(e.contains(":2:"), "{e}");
    }

    #[test]
    fn triggers_and_editor() {
        use crate::config::{OpenIn, TriggerAction};
        let (engine, c) = load_src(
            r##"return {
                triggers = {
                    { pattern = "error(\\[E\\d+\\])?", action = "highlight", color = "#f7768e" },
                    { pattern = "[Pp]assword:", action = "notify", title = "Password prompt" },
                    { pattern = "done", action = function(win, text) win:toast("saw " .. text) end },
                },
                editor = { cmd = { "nvim", "+{line}", "{file}" }, env = { NVIM_APPNAME = "nvim-dev" }, open_in = "tab" },
            }"##,
        )
        .unwrap();
        assert_eq!(c.triggers.len(), 3);
        assert_eq!(c.triggers[0].action, TriggerAction::Highlight(0xf7768e));
        assert!(c.triggers[0].regex.is_match("error[E0308]"));
        assert_eq!(c.triggers[1].action, TriggerAction::Notify("Password prompt".into()));
        let TriggerAction::Callback(idx) = c.triggers[2].action else { panic!() };
        let cmds = engine.call_with_text(idx, Snapshot::default(), "done!").unwrap();
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Toast(t, _) if t == "saw done!")));
        let ed = c.editor.unwrap();
        assert_eq!(ed.cmd, ["nvim", "+{line}", "{file}"]);
        assert_eq!(ed.env, [("NVIM_APPNAME".to_string(), "nvim-dev".to_string())]);
        assert_eq!(ed.open_in, OpenIn::Tab);
        // a bad regex is reported, not ignored
        let e = load_src(r##"return { triggers = { { pattern = "(", action = "toast" } } }"##).err().unwrap();
        assert!(e.contains("bad pattern"), "{e}");
        let (_, c) = load_src(r##"return { editor = "code -g {file}:{line}" }"##).unwrap();
        assert_eq!(c.editor.unwrap().cmd, ["code", "-g", "{file}:{line}"]);
    }

    #[test]
    fn keys_actions_and_callbacks() {
        let (engine, c) = load_src(
            r##"local vibex = require("vibex")
            local act = vibex.action
            return {
                leader = { key = "a", mods = "CTRL", timeout_ms = 800 },
                keys = {
                    { key = "c", mods = "CTRL|SHIFT", action = act.PassThrough },
                    { key = "g", mods = "LEADER", action = act.SendString("git status\r") },
                    { key = "w", mods = "LEADER", action = act.SpawnWindow("wsl") },
                    { key = "k", mods = "CTRL|ALT", action = function(win)
                        win:send_text("hi " .. win:profile())
                        win:toast("done")
                    end },
                },
            }"##,
        )
        .unwrap();
        let leader = c.leader.as_ref().unwrap();
        assert_eq!(leader.key, KeySpec::Char('a'));
        assert_eq!(leader.timeout, Duration::from_millis(800));
        let m = |s: &str| Mods::parse(s).unwrap();
        assert_eq!(keys::find(&c.bindings, &KeySpec::Char('c'), m("CTRL|SHIFT")).unwrap().action, Action::PassThrough);
        assert_eq!(keys::find(&c.bindings, &KeySpec::Char('g'), m("LEADER")).unwrap().action, Action::SendString("git status\r".into()));
        assert_eq!(keys::find(&c.bindings, &KeySpec::Char('w'), m("LEADER")).unwrap().action, Action::SpawnWindow(Some("wsl".into())));
        // defaults still present
        assert!(keys::find(&c.bindings, &KeySpec::Char('v'), m("CTRL|SHIFT")).is_some());
        let Action::Callback(idx) = keys::find(&c.bindings, &KeySpec::Char('k'), m("CTRL|ALT")).unwrap().action else { panic!("expected callback") };
        let snap = Snapshot { profile: "wsl".into(), ..Default::default() };
        let cmds = engine.call(idx, snap).unwrap();
        assert!(matches!(&cmds[0], Cmd::SendText(s) if s == "hi wsl"));
        assert!(matches!(&cmds[1], Cmd::Toast(s, _) if s == "done"));
    }

    #[test]
    fn tab_and_pane_actions() {
        let (_, c) = load_src(
            r##"local act = require("vibex").action
            return { keys = {
                { key = "1", mods = "ALT", action = act.ActivateTab(1) },
                { key = "u", mods = "ALT", action = act.SplitRightWith("wsl") },
                { key = "h", mods = "ALT", action = act.FocusPane("Left") },
                { key = "t", mods = "ALT", action = act.NewTab },
                { key = "p", mods = "ALT", action = act.CommandPalette },
            } }"##,
        )
        .unwrap();
        let m = Mods::parse("ALT").unwrap();
        let a = |k: char| keys::find(&c.bindings, &KeySpec::Char(k), m).unwrap().action.clone();
        assert_eq!(a('1'), Action::ActivateTab(1));
        assert_eq!(a('u'), Action::SplitRight(Some("wsl".into())));
        assert_eq!(a('h'), Action::FocusPane(crate::layout::Direction::Left));
        assert_eq!(a('t'), Action::NewTab(None));
        assert_eq!(a('p'), Action::CommandPalette);
        let e = load_src(r#"return { keys = { { key = "x", mods = "ALT", action = vibex.action.FocusPane("sideways") } } }"#).err().unwrap();
        assert!(e.contains("direction must be"), "{e}");
    }

    #[test]
    fn leader_binding_without_leader_is_error() {
        let e = load_src(r#"return { keys = { { key = "g", mods = "LEADER", action = vibex.action.Copy } } }"#).err().unwrap();
        assert!(e.contains("config.leader is not set"), "{e}");
    }

    #[test]
    fn events_and_status() {
        let (engine, _) = load_src(
            r##"local vibex = require("vibex")
            vibex.on("title-changed", function(win, title) win:set_title("<" .. title .. ">") end)
            vibex.on("update-status", function(win)
                local cols, rows = win:size()
                return {
                    left = { { text = " " .. win:profile() .. " ", bg = "#f38020", fg = "#000000", bold = true } },
                    right = cols .. "x" .. rows,
                }
            end)
            return {}"##,
        )
        .unwrap();
        let snap = Snapshot { profile: "pwsh".into(), cols: 80, rows: 24, ..Default::default() };
        let cmds = engine.emit("title-changed", snap.clone(), EventArg::Str("vim".into())).unwrap();
        assert!(matches!(&cmds[0], Cmd::SetTitle(s) if s == "<vim>"));
        let (line, _) = engine.status(snap).unwrap().unwrap();
        assert_eq!(line.left[0].text, " pwsh ");
        assert_eq!(line.left[0].bg, Some(0xf38020));
        assert!(line.left[0].bold);
        assert_eq!(line.right[0].text, "80x24");
    }

    #[test]
    fn unknown_event_is_error() {
        let e = load_src(r#"vibex.on("on-click", function() end) return {}"#).err().unwrap();
        assert!(e.contains("unknown event 'on-click'"), "{e}");
    }

    #[cfg(windows)]
    #[test]
    fn profiles() {
        let (_, c) = load_src(
            r##"return {
                default_profile = "dev",
                profiles = {
                    dev = { cmd = { "wsl.exe", "-d", "Ubuntu" }, cwd = "~", env = { FOO = "bar" } },
                    box = { cmd = "ssh me@devbox" },
                },
            }"##,
        )
        .unwrap();
        let dev = c.profile("dev").unwrap();
        assert_eq!(dev.command.program, "wsl.exe");
        assert_eq!(dev.command.args, vec!["-d", "Ubuntu"]);
        assert!(dev.command.cwd.as_deref().is_some_and(|d| !d.starts_with('~')));
        assert!(dev.command.env.iter().any(|(k, v)| k == "FOO" && v == "bar"));
        assert_eq!(c.profile("box").unwrap().command.args, vec!["me@devbox"]);
        assert!(c.profile("powershell").is_some(), "built-in profiles remain");
        let e = load_src(r#"return { default_profile = "nope" }"#).err().unwrap();
        assert!(e.contains("no profile named 'nope'"), "{e}");
    }

    #[test]
    fn watcher_reloads_on_change_and_reports_errors() {
        let dir = std::env::temp_dir().join(format!("vibex-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vibex.lua");
        std::fs::write(&path, "return { font_size = 12 }").unwrap();
        let mut w = ConfigWatcher::new(Some(path.clone()));
        assert!(w.poll().is_none(), "no change yet");

        // mtime resolution can be coarse; make sure it moves
        let bump = |p: &std::path::Path, src: &str| {
            std::fs::write(p, src).unwrap();
            let f = std::fs::File::options().write(true).open(p).unwrap();
            let t = std::time::SystemTime::now() + Duration::from_secs(rand_suffix() + 2);
            f.set_modified(t).unwrap();
        };
        bump(&path, "return { font_size = 14 }");
        match w.poll() {
            Some(Reload::Loaded(b)) => assert_eq!(b.1.font_size_pt, 14.0),
            _ => panic!("expected reload"),
        }
        assert!(w.poll().is_none(), "settled");

        bump(&path, "return { fontsize = 1 }");
        match w.poll() {
            Some(Reload::Failed(e)) => assert!(e.contains("unknown option"), "{e}"),
            _ => panic!("expected error"),
        }
        // a failed load doesn't retry until the file changes again
        assert!(w.poll().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_byte_order_mark_is_fine() {
        // what Notepad writes: EF BB BF, then the file
        let c = load_src("\u{feff}-- my config\nreturn { theme = \"dracula\" }").unwrap().1;
        assert_eq!(c.theme.identify(), Some("dracula"));
    }

    #[test]
    fn error_messages_name_the_file() {
        let e = load_src("return {\n  x = = 1\n}").err().unwrap();
        assert!(e.starts_with("vibex.lua:2:"), "{e}");
    }

    #[test]
    fn runtime_error_in_callback_is_reported() {
        let (engine, c) = load_src(r#"return { keys = { { key = "x", mods = "CTRL|ALT", action = function(win) error("boom") end } } }"#).unwrap();
        let Action::Callback(idx) = keys::find(&c.bindings, &KeySpec::Char('x'), Mods::parse("CTRL|ALT").unwrap()).unwrap().action else { panic!() };
        let e = engine.call(idx, Snapshot::default()).err().unwrap();
        assert!(e.contains("boom"), "{e}");
    }

    #[test]
    fn a_stuck_callback_is_stopped() {
        // a plain loop, and one that tries to pcall its way past the stop
        let src = r#"
            vibex.on("bell", function(win) while true do end end)
            return { keys = {
                { key = "x", mods = "CTRL|ALT", action = function(win) while true do pcall(function() while true do end end) end end },
                { key = "y", mods = "CTRL|ALT", action = function(win) win:toast("hi") end },
            } }"#;
        let (engine, c) = load_src(src).unwrap();
        let cb = |k| match keys::find(&c.bindings, &KeySpec::Char(k), Mods::parse("CTRL|ALT").unwrap()).unwrap().action {
            Action::Callback(idx) => idx,
            _ => panic!(),
        };
        let t = std::time::Instant::now();
        let e = engine.call(cb('x'), Snapshot::default()).err().unwrap();
        assert_eq!(e, TIMEOUT_CALL);
        assert!(t.elapsed() < Duration::from_secs(5));
        let e = engine.emit("bell", Snapshot::default(), EventArg::None).err().unwrap();
        assert_eq!(e, TIMEOUT_CALL);
        // the next call starts with a fresh deadline
        let cmds = engine.call(cb('y'), Snapshot::default()).unwrap();
        assert!(matches!(cmds.as_slice(), [Cmd::Toast(m, _)] if m == "hi"));
        let e = load_src("while true do end").err().unwrap();
        assert_eq!(e, TIMEOUT_LOAD);
    }

    #[test]
    fn a_stuck_coroutine_is_stopped_too() {
        let src = r#"
            return { keys = {
                { key = "c", mods = "CTRL|ALT", action = function(win) coroutine.wrap(function() while true do end end)() end },
                { key = "d", mods = "CTRL|ALT", action = function(win)
                    local co = coroutine.create(function() while true do pcall(function() while true do end end) end end)
                    while true do coroutine.resume(co) end
                end },
            } }"#;
        let (engine, c) = load_src(src).unwrap();
        let cb = |k| match keys::find(&c.bindings, &KeySpec::Char(k), Mods::parse("CTRL|ALT").unwrap()).unwrap().action {
            Action::Callback(idx) => idx,
            _ => panic!(),
        };
        for k in ['c', 'd'] {
            let (tx, rx) = std::sync::mpsc::channel();
            let start = std::time::Instant::now();
            // on this thread (the engine isn't Send); fail the test instead of hanging forever
            let watchdog = std::thread::spawn(move || {
                if rx.recv_timeout(Duration::from_secs(10)).is_err() {
                    eprintln!("coroutine loop '{k}' never stopped");
                    std::process::exit(3);
                }
            });
            let r = engine.call(cb(k), Snapshot::default());
            let _ = tx.send(());
            watchdog.join().unwrap();
            assert!(r.is_err(), "'{k}' should have been stopped");
            assert!(start.elapsed() < Duration::from_secs(5), "'{k}' took {:?}", start.elapsed());
        }
    }
}
