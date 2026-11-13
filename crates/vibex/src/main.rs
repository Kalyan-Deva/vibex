#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod access;
mod audit;
mod bundle;
mod commands;
mod config;
mod copymode;
mod engagement;
mod export;
mod export_ui;
mod history;
mod hotkey;
mod input;
mod keys;
mod layout;
mod lua;
mod mouse;
mod notes;
mod notify;
mod palette;
mod pane;
mod record;
mod secrets;
mod session;
mod settings;
mod settings_ui;
mod show;
mod single;
mod ssh;
mod update;

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use config::{Config, Decorations, RendererKind, TabBarMode};
use keys::{Action, KeySpec, Mods};
use layout::{Divider, Node, PaneId, SplitDir};
use lua::{Cmd, ConfigWatcher, Engine, EventArg, Reload, Snapshot};
use palette::Palette;
use pane::{Pane, PaneOptions, Selection};
use vibex_render::{
    Chrome, ChromeLayout, DrawState, FontSet, Frame, GpuRenderer, MenuIcon, MenuItem, MenuView, Overlay, PxRect, Renderer, SettingsHit, StatusLine,
    StatusSegment, TabBar, TabBarLayout, TabHit, TabIcon, TabInfo,
};
use vibex_term::{MouseMode, Terminal};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

#[derive(Debug)]
enum AppEvent {
    /// Output from some pane (any window).
    Pty,
    /// Another `vibex` launch handed over a window to open.
    Open(single::OpenRequest),
    /// A screen reader wants the accessibility tree (or an action).
    Access(accesskit_winit::Event),
    /// The Quake-style hotkey was pressed.
    Quake,
    /// `vibex cli ...`: arguments, and where the reply goes.
    Remote(Vec<String>, std::sync::mpsc::Sender<String>),
    /// Update check / download finished (`manual` = the user asked).
    Update(update::Msg, bool),
    /// The bundled editors: a check or an update finished.
    Editors(bundle::Msg, bool),
}

impl From<accesskit_winit::Event> for AppEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        AppEvent::Access(e)
    }
}

enum Backend {
    Gpu(Box<GpuRenderer>),
    Cpu {
        _context: softbuffer::Context<Arc<Window>>,
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        /// The last frame presented, to send Windows only what changed.
        last: Vec<u32>,
        scratch: Vec<u32>,
    },
}

/// Bands of scanlines that differ between two frames, as damage rects.
fn changed_bands(old: &[u32], new: &[u32], w: usize, h: usize) -> Vec<softbuffer::Rect> {
    let mut out: Vec<softbuffer::Rect> = Vec::new();
    let mut start: Option<usize> = None;
    let full_w = NonZeroU32::new(w as u32).unwrap();
    for y in 0..=h {
        let differs = y < h && old[y * w..(y + 1) * w] != new[y * w..(y + 1) * w];
        match (differs, start) {
            (true, None) => start = Some(y),
            (false, Some(s0)) => {
                out.push(softbuffer::Rect { x: 0, y: s0 as u32, width: full_w, height: NonZeroU32::new((y - s0) as u32).unwrap() });
                start = None;
            }
            _ => {}
        }
    }
    out
}

struct Tab {
    root: Node,
    active: PaneId,
    zoomed: bool,
    /// Set by renaming; otherwise the focused pane's title is shown.
    title: Option<String>,
    color: Option<u32>,
    /// Keyboard input goes to every pane in the tab.
    broadcast: bool,
    /// A pane floating over the splits, and whether it's showing.
    float: Option<PaneId>,
    float_shown: bool,
}

impl Tab {
    fn new(id: PaneId) -> Tab {
        Tab { root: Node::Leaf(id), active: id, zoomed: false, title: None, color: None, broadcast: false, float: None, float_shown: false }
    }

    /// The floating pane, if it's on screen.
    fn shown_float(&self) -> Option<PaneId> {
        self.float.filter(|_| self.float_shown)
    }
}

/// What a menu item does.
#[derive(Clone)]
enum MenuCmd {
    Act(Action),
    OpenLink(String),
    Copy(String),
    /// Something done to the tab at this index (the tab right-click menu).
    Tab(TabCmd, usize),
    /// Type this at the prompt (a completion).
    Type(String),
    /// A Settings dropdown choice.
    Setting(settings::Change),
    /// Do this with the lines in `export_job`.
    Export(export_ui::ExportOut),
    /// Chose which lines: now the format menu.
    ExportPick(export_ui::ExportJob),
    /// Save a PNG of these lines and copy it.
    Shot(export_ui::ExportJob),
    /// Forget "Mark start".
    ClearMark,
    /// Mark the start at this absolute line.
    MarkAt(usize),
}

#[derive(Clone)]
enum TabCmd {
    Rename,
    Color,
    Duplicate,
    NewWindow,
    /// Move it into another window (by window id).
    ToWindow(u64),
    CloseOthers,
    CloseRight,
    Close,
}

/// A tab (with its running shells) on its way to another window; App
/// carries it out, since it owns every window.
struct TabMove {
    tab: Tab,
    panes: Vec<(PaneId, Pane)>,
    to: MoveTarget,
}

enum MoveTarget {
    /// A new window, its top-left here on screen (None = beside this one).
    NewWindow(Option<PhysicalPosition<i32>>),
    /// Another window, at this tab index (None = the end).
    Window(u64, Option<usize>),
    /// Dropped at this screen point: into the tab bar of the window there,
    /// else a new window there.
    Drop(PhysicalPosition<i32>),
}

/// What happens when a question's "yes" is chosen.
enum ConfirmAct {
    Paste(String),
    /// Send the held Enter to this pane.
    Enter(PaneId),
    CloseTab(usize),
    ClosePane(PaneId),
    CloseWindow,
    /// Download and swap in newer nano / Neovim.
    UpdateEditors(bundle::Found),
}

/// A yes / no question over the window.
struct Confirm {
    view: vibex_render::ConfirmView,
    act: ConfirmAct,
}

/// A closed tab, kept so it can come back (scrollback in `buffers/`).
struct ClosedTab {
    tab: session::SavedTab,
    index: usize,
    buffers: Vec<String>,
}

/// A tab arriving in a new window.
struct Adopt {
    tab: Tab,
    panes: Vec<(PaneId, Pane)>,
    pos: Option<PhysicalPosition<i32>>,
}

/// The dropdown next to the + button, or the right-click menu.
struct Menu {
    items: Vec<(MenuItem, MenuCmd)>,
    /// A completions popup: typing goes on to the shell (and closes it).
    typing: bool,
    /// First item shown (long lists scroll), and the narrowest it may be.
    first: usize,
    min_width: f32,
    selected: Option<usize>,
    x: f32,
    y: f32,
}

/// A tab being dragged in the tab bar.
#[derive(Clone, Copy)]
struct TabDrag {
    index: usize,
    start_x: f64,
    moved: bool,
    /// Pulled away from the tab bar: dropping it moves it to another
    /// window, or a new one.
    out: bool,
}

/// Quick-select mode: labelled targets on the focused pane.
struct Quick {
    pane: PaneId,
    targets: Vec<(String, vibex_term::Target)>,
    typed: String,
}

/// Pixels from the window edge that start a resize (custom title bar).
const EDGE: f64 = 6.0;

struct Session {
    window: Arc<Window>,
    backend: Backend,
    renderer: Renderer,
    proxy: EventLoopProxy<AppEvent>,
    opacity: f32,
    smooth_scroll: bool,
    mods: ModifiersState,
    focused: bool,
    clipboard: Option<arboard::Clipboard>,
    mouse: PhysicalPosition<f64>,
    base_font_px: f32,
    quit: bool,
    // tabs + panes
    tabs: Vec<Tab>,
    active_tab: usize,
    panes: HashMap<PaneId, Pane>,
    next_pane: PaneId,
    dividers: Vec<Divider>,
    dragging: Option<Divider>,
    /// Pane that owns the current mouse selection drag.
    selecting_pane: Option<PaneId>,
    // window chrome
    custom_chrome: bool,
    tabbar: TabBarLayout,
    tab_hover: Option<TabHit>,
    edge: Option<ResizeDirection>,
    last_bar_click: Option<Instant>,
    fullscreen: bool,
    palette: Option<Palette>,
    quick: Option<Quick>,
    /// IME composition in progress (drawn at the cursor).
    preedit: Option<String>,
    /// Renaming a tab: (tab index, text so far).
    rename: Option<(usize, String)>,
    tab_drag: Option<TabDrag>,
    last_tab_click: Option<(Instant, usize)>,
    // multi-click
    last_click: Option<(Instant, (usize, usize))>,
    click_count: u8,
    multi_click: Duration,
    // cursor blink
    blink_on: bool,
    next_blink: Instant,
    blink_interval: Duration,
    // config
    config: Config,
    engine: Engine,
    watcher: ConfigWatcher,
    next_config_poll: Instant,
    /// Shown until the config loads cleanly again.
    config_error: Option<String>,
    toast: Option<(String, Instant)>,
    /// Lines waiting in the format menu.
    export_job: Option<export_ui::ExportJob>,
    /// "Mark start": where some lines to save begin.
    mark: Option<export_ui::LineMark>,
    status: Option<StatusLine>,
    next_status: Instant,
    leader_until: Option<Instant>,
    /// Palette-made overrides (settings.lua).
    settings: settings::Settings,
    // session restore
    session_dirty: bool,
    next_session_save: Instant,
    /// Last non-maximized logical size, for the session file.
    restored_size: Option<(f64, f64)>,
    /// New windows this one asked for (opened by `App`, which owns the
    /// event loop).
    open_requests: Vec<single::OpenRequest>,
    /// This window writes session.lua.
    saves_session: bool,
    /// Screen reader bridge (taken out briefly while updating).
    access: Option<accesskit_winit::Adapter>,
    /// A newer signed release, once found.
    update: Option<vibex_update::Feed>,
    /// What the last update check said (Settings → Updates).
    update_status: String,
    /// Newer nano / Neovim, once found, and what the last look said.
    editors_found: Option<bundle::Found>,
    editors_status: String,
    /// When to look again while Vibex stays open (weekly).
    next_editor_check: Instant,
    /// The Settings tab.
    settings_ui: settings_ui::SettingsUi,
    /// A slider being dragged in Settings: row and track.
    settings_drag: Option<(usize, PxRect)>,
    /// Where the Settings dialog was drawn last.
    settings_rect: PxRect,
    /// Hit regions from the last frame (Settings page, open menu).
    chrome_layout: ChromeLayout,
    /// The ⌄ menu, when open.
    menu: Option<Menu>,
    /// Workspace name (this window).
    name: String,
    /// Hidden by "Detach"; its shells keep running. When it was detached.
    detached: Option<Instant>,
    /// Every workspace (window id, name, detached), kept current by `App`.
    workspaces: Vec<(u64, String, bool)>,
    /// Workspace switches / quit-all asked for here, carried out by `App`.
    ws_requests: Vec<Action>,
    /// This is the Quake-style dropdown.
    quake: bool,
    /// Tabs leaving this window (carried out by `App`).
    tab_moves: Vec<TabMove>,
    /// Past commands for suggestions, newest first (loaded on first use).
    suggest: Option<Vec<(String, String)>>,
    /// An open yes / no question.
    confirm: Option<Confirm>,
    /// Show what the secret guard masks.
    reveal_secrets: bool,
    /// Recently closed tabs, newest last.
    closed: Vec<ClosedTab>,
    /// Focus mode: full screen, no bars, notifications held.
    focus_mode: bool,
    /// Focus mode turned full screen on (so leaving turns it off).
    focus_fullscreen: bool,
    held_notes: Vec<(String, String)>,
    /// The engagement that's on.
    eng: Option<engagement::Engagement>,
}

/// Everything decided before the window exists.
struct Startup {
    config: Config,
    engine: Engine,
    config_path: Option<PathBuf>,
    error: Option<String>,
    profile: String,
    /// `--cwd`: the first tab starts here.
    cwd: Option<String>,
    /// `-- program args...`: run this instead of a profile.
    exec: Option<Vec<String>>,
    /// Last session to reopen.
    restore: Option<session::Saved>,
    settings: settings::Settings,
    /// Workspace name (from a layout), if any.
    name: Option<String>,
    /// The Quake-style dropdown window.
    quake: bool,
    /// Open with this tab, moved from another window.
    adopt: Option<Adopt>,
}

struct App {
    proxy: EventLoopProxy<AppEvent>,
    /// The first window, opened on `resumed`.
    startup: Option<Startup>,
    /// One per window.
    sessions: HashMap<WindowId, Session>,
    /// Shown as a toast in the first window (e.g. hotkey already taken).
    notice: Option<String>,
}

/// Load the config (fresh, so edits since start apply) for a new window.
fn startup_for(req: single::OpenRequest) -> Startup {
    let config_path = req.config.clone().or_else(config::config_file);
    let settings = settings::path().map(|p| settings::Settings::load(&p)).unwrap_or_default();
    let (engine, mut config, mut error) = match Engine::load(config_path.as_deref()) {
        Ok((engine, cfg)) => (engine, cfg, None),
        Err(e) => (Engine::empty(), Config::default(), Some(e)),
    };
    settings.apply(&mut config);
    let config = config.with_env();
    let profile = match req.profile {
        Some(p) if config.profile(&p).is_some() => p,
        Some(p) => {
            error = Some(format!("no profile named '{p}'; using '{}'", config.default_profile));
            config.default_profile.clone()
        }
        None => config.default_profile.clone(),
    };
    let exec = req.exec.filter(|a| !a.is_empty());
    let mut restore = None;
    let mut name = None;
    if let Some(l) = &req.layout {
        let file = session::layouts_dir().map(|d| d.join(format!("{}.lua", session::layout_file_name(l))));
        match file.map(|f| session::load(&f)) {
            Some(Ok(saved)) => {
                restore = Some(saved);
                name = Some(l.clone());
            }
            Some(Err(e)) => error = Some(format!("layout '{l}': {e}")),
            None => {}
        }
    }
    Startup { config, engine, config_path, error, profile, cwd: req.cwd, exec, restore, settings, name, quake: false, adopt: None }
}

#[cfg(test)]
mod damage_tests {
    #[test]
    fn bands() {
        let (w, h) = (4, 6);
        let old = vec![0u32; w * h];
        let mut new = old.clone();
        new[w + 2] = 1; // row 1
        new[2 * w] = 1; // row 2
        new[5 * w + 3] = 1; // row 5
        let b = super::changed_bands(&old, &new, w, h);
        let rows: Vec<(u32, u32)> = b.iter().map(|r| (r.y, r.height.get())).collect();
        assert_eq!(rows, [(1, 2), (5, 1)]);
        assert!(super::changed_bands(&old, &old, w, h).is_empty());
    }
}

fn font_px(pt: f32, scale: f64) -> f32 {
    pt * 96.0 / 72.0 * scale as f32
}

fn to_px(r: layout::Rect) -> PxRect {
    PxRect { x: r.x, y: r.y, w: r.w, h: r.h }
}

/// `vibex cli ...`: send the request to the running Vibex, print the reply.
/// The Settings dialog: centred over the content, most of it.
fn settings_dialog_rect(content: PxRect, scale: f32) -> PxRect {
    let s = scale.max(0.5);
    let w = (content.w * 0.9).min(1180.0 * s).max(content.w.min(520.0));
    let h = (content.h * 0.9).min(860.0 * s).max(content.h.min(360.0));
    PxRect { x: (content.x + (content.w - w) / 2.0).round(), y: (content.y + (content.h - h) / 2.0).round(), w: w.round(), h: h.round() }
}

/// `vibex quake`: toggle the dropdown in the running Vibex, starting one
/// (with the dropdown open) if none is running. Bind this to a key in your
/// desktop's shortcut settings.
fn run_quake() -> i32 {
    if single::send(&single::Request::Quake).is_some() {
        return 0;
    }
    match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--quake").stdin(std::process::Stdio::null()).spawn()) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("vibex: couldn't start Vibex: {e}");
            1
        }
    }
}

fn run_cli(mut args: Vec<String>) -> i32 {
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        println!("{CLI_USAGE}");
        return 0;
    }
    // the pane this command runs in (if inside Vibex): default target
    if let Ok(p) = std::env::var("VIBEX_PANE") {
        args.push("--from-pane".into());
        args.push(p);
    }
    match single::send(&single::Request::Cli(args)) {
        Some(reply) if reply.starts_with("error:") => {
            eprintln!("vibex: {}", reply.trim_start_matches("error:").trim());
            1
        }
        Some(reply) if reply == "ok" || reply.starts_with("ok\n") => {
            let text = reply.strip_prefix("ok").unwrap_or("").strip_prefix('\n').unwrap_or("");
            if !text.is_empty() {
                println!("{text}");
            }
            0
        }
        _ => {
            eprintln!("vibex: Vibex isn't running");
            1
        }
    }
}

const CLI_USAGE: &str = "usage: vibex cli COMMAND [options]   (needs config.remote_control = true, or Settings → Terminal)

  list                                  windows, tabs and panes as JSON lines
  spawn [--cwd DIR] [--profile P] [-- PROGRAM ARGS]   new tab; prints its pane id
  split --right|--down [--pane N] [-- PROGRAM ARGS]   prints the new pane id
  send-text [--pane N] [--enter] TEXT   type TEXT into a pane
  get-text [--pane N] [--lines N]       print a pane's text (default: the screen)
  export [--pane N] [--format png|txt|ans|html|svg|md] [--out FILE] [--copy]
         [--block | --all | --last N | --from A --to B]
                                        save the screen (or the last command, or lines A to B,
                                        1-based) as a picture or text, secrets masked
  focus [--pane N]                      bring a pane (and its window) forward
  close [--pane N]
  set-tab-title [--pane N] TITLE
  popup [--pane N] -- PROGRAM ARGS      run in a floating pane that closes on exit
  notify TITLE [BODY]
  action [--pane N] NAME                run a Vibex action (SplitRight, ToggleRecording, HistorySearch, ...)
  enter [--pane N]                      press Enter (asks first for dangerous commands, like the keyboard)
  notes [--pane N] TEXT                 add a line to the pane's notes panel
  dialog [--answer yes|no]              the open question, and answer it
  prompt [--pane N]                     what's typed at the prompt, and the suggestion
  play FILE.cast                        replay a recording in a new tab
  engagement start NAME [--scope S] | stop | scope S | status | report
  move-tab [--pane N] [--window W | --new-window]   move a tab (shells and all) to another window
  settings [PAGE]                       open the Settings dialog (appearance, terminal, profiles, shortcuts, updates, about)

--pane defaults to the pane the command runs in ($VIBEX_PANE).";

/// Marks a hover "link" that is really a `file:line` reference.
const FILE_REF: &str = "vibex-file-ref:";

/// Split a command line into words: spaces separate, "double" or 'single'
/// quotes group.
pub(crate) fn split_command(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod split_tests {
    #[test]
    fn words_and_quotes() {
        assert_eq!(super::split_command("lazygit"), ["lazygit"]);
        assert_eq!(super::split_command(r#"nvim "my file.txt" -R"#), ["nvim", "my file.txt", "-R"]);
        assert_eq!(super::split_command("  git   log  "), ["git", "log"]);
        assert_eq!(super::split_command("echo ''"), ["echo", ""]);
    }
}

/// The key produced printable text despite Ctrl being held: AltGr.
fn altgr_text(event: &KeyEvent) -> bool {
    event.text.as_ref().is_some_and(|t| !t.is_empty() && t.chars().all(|c| !c.is_control()))
}

/// Status line used when vibex.lua has no `update-status` handler.
fn default_status(t: &vibex_render::Theme, s: &Snapshot) -> StatusLine {
    let mut left = vec![StatusSegment { text: format!(" {} ", s.profile), fg: Some(t.cursor_text), bg: Some(t.cursor), bold: true }];
    if !s.cwd.is_empty() {
        left.push(StatusSegment { text: format!(" {}", s.cwd), ..Default::default() });
    } else if !s.title.is_empty() {
        left.push(StatusSegment { text: format!(" {}", s.title), ..Default::default() });
    }
    let mut right = Vec::new();
    if s.leader {
        right.push(StatusSegment { text: " LEADER ".into(), fg: Some(t.cursor_text), bg: Some(t.palette[11]), bold: true });
        right.push(StatusSegment { text: " ".into(), ..Default::default() });
    }
    if let Some(code) = s.last_exit.filter(|c| *c != 0) {
        right.push(StatusSegment { text: format!("✗ {code}  "), fg: Some(t.palette[1]), bg: None, bold: true });
    }
    if s.pane_count > 1 {
        right.push(StatusSegment { text: format!("{} panes  ", s.pane_count), ..Default::default() });
    }
    right.push(StatusSegment { text: format!("{}×{} ", s.cols, s.rows), ..Default::default() });
    StatusLine { left, right }
}

/// Open a file with its associated app (falls back to Notepad).
fn open_path(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain(Some(0)).collect() };
        let (op, file) = (wide("open".as_ref()), wide(path.as_os_str()));
        let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        if r as isize > 32 {
            return true;
        }
        std::process::Command::new("notepad.exe").arg(path).spawn().is_ok()
    }
    #[cfg(not(windows))]
    {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        std::process::Command::new(opener).arg(path).spawn().is_ok()
    }
}

/// Open an http(s) or mailto URL with its default app. Other schemes
/// (file:, custom protocol handlers) are refused: a program in the terminal
/// must not be able to launch things through a link.
/// A VibeX icon for a menu item, by what it says.
fn menu_icon(label: &str) -> Option<MenuIcon> {
    let l = label.to_lowercase();
    let name = match () {
        _ if l.starts_with("open ") && l.contains("line") => "file",
        _ if l == "open link" => "link",
        _ if l.starts_with("copy link") || l == "copy path" => "link",
        _ if l == "copy the last output" => "file",
        _ if l.starts_with("copy") => "copy",
        _ if l.starts_with("paste") => "paste",
        _ if l == "select all" => "focus",
        _ if l == "find" => "search",
        _ if l == "split right" => "split-right",
        _ if l == "split down" => "split-down",
        _ if l == "new tab" || l.starts_with("move to a new window") => "tab-new",
        _ if l.starts_with("move to window") => "split-right",
        _ if l == "floating pane" => "float",
        _ if l.starts_with("run the last") => "play",
        _ if l == "clear scrollback" => "trash",
        _ if l == "command palette" => "command",
        _ if l == "rename tab" => "note",
        _ if l.starts_with("tab colour") => "appearance",
        _ if l == "duplicate tab" => "copy",
        _ if l.starts_with("close") => "close",
        _ if l == "about vibex" => "info",
        _ => return None,
    };
    Some(MenuIcon::Named(name))
}

/// Give a menu's items their icons (those that don't have one).
fn with_icons(items: &mut [(MenuItem, MenuCmd)]) {
    for (it, _) in items.iter_mut() {
        if it.icon.is_none() {
            it.icon = menu_icon(&it.label);
        }
    }
}

/// "3 NTLM hashes, 2 credentials" for Settings.
fn loot_summary(path: &std::path::Path) -> String {
    let loot = engagement::read_loot(path);
    if loot.is_empty() {
        return "Nothing collected yet".into();
    }
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (_, l, _) in &loot {
        match counts.iter_mut().find(|(k, _)| *k == l.kind) {
            Some(c) => c.1 += 1,
            None => counts.push((l.kind.clone(), 1)),
        }
    }
    counts.iter().map(|(k, n)| format!("{n} × {k}")).collect::<Vec<_>>().join(", ")
}

/// Delete saved scrollback files (in `buffers/`).
fn drop_buffers(names: &[String]) {
    if let Some(dir) = session::buffers_dir() {
        for n in names {
            let _ = std::fs::remove_file(dir.join(n));
        }
    }
}

/// Paint a pane's own background (a per-host theme) at the window's opacity.
fn scene_fill(scene: &mut vibex_render::Scene, r: PxRect, color: u32, opacity: f32) {
    vibex_render::fill_rect(scene, r, color, (opacity.clamp(0.0, 1.0) * 255.0) as u8);
}

/// "src/main.rs" for a long path: the last two parts.
fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        p.to_string()
    } else {
        parts[parts.len() - 2..].join("/")
    }
}

fn open_url(url: &str) {
    if !vibex_term::links::is_safe_to_open(url) || url.chars().any(|c| c.is_control()) {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect() };
        let (op, file) = (wide("open"), wide(url));
        unsafe {
            ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        }
    }
    #[cfg(not(windows))]
    {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(opener).arg(url).spawn();
    }
}

/// Windows 11: ask DWM for rounded corners on our frameless window.
/// Tell Windows whether the window is dark: the acrylic backdrop behind a
/// see-through window takes its tint from this (light grey otherwise).
fn set_dark_mode(window: &Window, background: u32) {
    let (r, g, b) = ((background >> 16) & 0xff, (background >> 8) & 0xff, background & 0xff);
    let is_dark = (r * 299 + g * 587 + b * 114) / 1000 < 128;
    // winit re-applies its own light/dark choice (the system's) on creation
    // and on settings changes, so the preference has to go through it
    window.set_theme(Some(if is_dark { winit::window::Theme::Dark } else { winit::window::Theme::Light }));
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let dark: i32 = is_dark as i32;
        if let Ok(h) = window.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                unsafe {
                    DwmSetWindowAttribute(
                        w.hwnd.get() as *mut std::ffi::c_void,
                        DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                        &dark as *const _ as *const std::ffi::c_void,
                        std::mem::size_of_val(&dark) as u32,
                    );
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (window, background);
}

/// See-through windows: put the acrylic blur behind the client area. A
/// window without the system frame gets no backdrop unless its frame is
/// extended over the whole client area.
fn enable_backdrop(window: &Window, glass: bool) {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct AccentPolicy {
            state: u32,
            flags: u32,
            gradient: u32,
            animation: u32,
        }
        #[repr(C)]
        struct CompositionData {
            attribute: u32,
            data: *mut std::ffi::c_void,
            size: usize,
        }
        // undocumented, so not in user32.lib: look it up at run time
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleA(name: *const u8) -> *mut std::ffi::c_void;
            fn GetProcAddress(module: *mut std::ffi::c_void, name: *const u8) -> *const std::ffi::c_void;
        }
        type SetWca = unsafe extern "system" fn(*mut std::ffi::c_void, *mut CompositionData) -> i32;
        let set_wca: Option<SetWca> = unsafe {
            let user32 = GetModuleHandleA(c"user32.dll".as_ptr().cast());
            let f = if user32.is_null() { std::ptr::null() } else { GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr().cast()) };
            (!f.is_null()).then(|| std::mem::transmute::<*const std::ffi::c_void, SetWca>(f))
        };
        const WCA_ACCENT_POLICY: u32 = 19;
        const ACCENT_DISABLED: u32 = 0;
        const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
        const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
        const DWMSBT_NONE: i32 = 1;
        use windows_sys::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE};
        use windows_sys::Win32::UI::Controls::MARGINS;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = window.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                let hwnd = w.hwnd.get() as *mut std::ffi::c_void;
                // the extended frame paints a solid fill when there's no
                // system backdrop, so glass keeps the frame to itself
                let m = if glass { 0 } else { -1 };
                let margins = MARGINS { cxLeftWidth: m, cxRightWidth: m, cyTopHeight: m, cyBottomHeight: m };
                // glass: Windows' plain blur, no acrylic tint or noise, so
                // colour comes through; otherwise the system acrylic
                let kind = if glass { DWMSBT_NONE } else { DWMSBT_TRANSIENTWINDOW };
                let mut accent = AccentPolicy {
                    // VIBEX_GLASS_ACCENT=3 tries the plain blur (no grain)
                    state: if !glass {
                        ACCENT_DISABLED
                    } else if std::env::var("VIBEX_GLASS_ACCENT").as_deref() == Ok("3") {
                        ACCENT_ENABLE_BLURBEHIND
                    } else {
                        ACCENT_ENABLE_ACRYLICBLURBEHIND
                    },
                    flags: 0,
                    // acrylic's own tint (AABBGGRR): nearly none, we paint ours
                    gradient: 0x0100_0000,
                    animation: 0,
                };
                let mut data = CompositionData {
                    attribute: WCA_ACCENT_POLICY,
                    data: &mut accent as *mut _ as *mut std::ffi::c_void,
                    size: std::mem::size_of::<AccentPolicy>(),
                };
                unsafe {
                    if let Some(f) = set_wca {
                        f(hwnd, &mut data);
                    }
                    DwmExtendFrameIntoClientArea(hwnd, &margins);
                    DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_SYSTEMBACKDROP_TYPE as u32,
                        &kind as *const _ as *const std::ffi::c_void,
                        std::mem::size_of_val(&kind) as u32,
                    );
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (window, glass);
}

/// How much of the theme background to paint over Windows' acrylic so
/// `opacity` looks like it does in Windows Terminal. WT's acrylic first
/// pulls the blur most of the way to the tint's brightness (its
/// "luminosity" layer), then adds the tint at `opacity`; Windows' own
/// acrylic instead lightens the blur toward grey. Measured over the same
/// background, WT at 0.4 matches about 0.85 here.
fn acrylic_opacity(opacity: f32) -> f32 {
    const LUMINOSITY: f32 = 0.75;
    let o = opacity.clamp(0.0, 1.0);
    1.0 - (1.0 - o) * (1.0 - LUMINOSITY)
}

/// The glass look keeps only a thin luminosity layer, so `opacity` is
/// close to what you see.
fn glass_opacity(opacity: f32) -> f32 {
    const LUMINOSITY: f32 = 0.3;
    let o = opacity.clamp(0.0, 1.0);
    1.0 - (1.0 - o) * (1.0 - LUMINOSITY)
}

#[cfg(test)]
mod acrylic_tests {
    #[test]
    fn mapping() {
        assert_eq!(super::acrylic_opacity(1.0), 1.0);
        assert!((super::acrylic_opacity(0.4) - 0.85).abs() < 1e-6);
        assert!(super::acrylic_opacity(0.0) > 0.7, "the luminosity layer is always there");
    }
}

fn round_corners(window: &Window) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = window.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                let pref = DWMWCP_ROUND;
                unsafe {
                    DwmSetWindowAttribute(
                        w.hwnd.get() as *mut std::ffi::c_void,
                        DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                        &pref as *const _ as *const std::ffi::c_void,
                        std::mem::size_of_val(&pref) as u32,
                    );
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

impl App {
    /// Open a window for `startup`; an error closes Vibex only when no
    /// other window is open.
    fn open(&mut self, el: &ActiveEventLoop, startup: Startup) {
        match self.start(el, startup) {
            Ok(mut s) => {
                // only one window writes session.lua (the first; then
                // whichever is left)
                s.saves_session = self.sessions.is_empty() && !s.quake;
                if s.quake {
                    s.name = "quake".into();
                }
                if s.saves_session && s.config.check_updates && update::due() {
                    s.check_updates(false);
                }
                if s.saves_session && s.config.editor_updates && bundle::due() {
                    s.check_editors(false);
                }
                s.window.set_visible(true);
                if let Some(n) = self.notice.take() {
                    s.toast(n, 8.0);
                }
                s.sync_engagement();
                s.emit("startup", EventArg::None);
                s.refresh_status();
                if std::env::var_os("VIBEX_BACKGROUND").is_none() {
                    s.window.focus_window();
                }
                self.sessions.insert(s.window.id(), s);
            }
            Err(e) => {
                eprintln!("vibex: {e}");
                if self.sessions.is_empty() {
                    el.exit();
                }
            }
        }
    }

    fn on_editors(&mut self, msg: bundle::Msg, manual: bool) {
        let status = match &msg {
            bundle::Msg::Available(f) => format!("New: {}", f.describe()),
            bundle::Msg::UpToDate(s) | bundle::Msg::Done(s) | bundle::Msg::Failed(s) => s.clone(),
        };
        let found = match &msg {
            bundle::Msg::Available(f) => Some(f.clone()),
            _ => None,
        };
        for s in self.sessions.values_mut() {
            s.editors_status = status.clone();
            s.editors_found = found.clone();
            s.window.request_redraw();
        }
        let pick = self.sessions.iter().find(|(_, s)| s.focused).or(self.sessions.iter().next()).map(|(w, _)| *w);
        let Some(s) = pick.and_then(|w| self.sessions.get_mut(&w)) else { return };
        match msg {
            bundle::Msg::Available(f) if manual => s.ask_update_editors(f),
            bundle::Msg::Available(f) => s.toast(format!("{} is out\npalette \u{2192} Update nano and Neovim", f.describe()), 8.0),
            bundle::Msg::UpToDate(t) if manual => s.toast(t, 3.0),
            bundle::Msg::Done(t) => s.toast(format!("{t}\nnew tabs use them"), 5.0),
            bundle::Msg::Failed(e) if manual => s.toast(e, 6.0),
            _ => {}
        }
    }

    fn on_update(&mut self, msg: update::Msg, manual: bool) {
        let status = match &msg {
            update::Msg::Available(f) => format!("Vibex {} is available", f.version),
            update::Msg::UpToDate => format!("Up to date ({})", env!("CARGO_PKG_VERSION")),
            update::Msg::Failed(e) => e.clone(),
            update::Msg::Ready(_, f) => format!("Installing {}…", f.version),
        };
        for s in self.sessions.values_mut() {
            s.update_status = status.clone();
            s.window.request_redraw();
        }
        let Some(s) = self.sessions.values_mut().next() else { return };
        match msg {
            update::Msg::Available(feed) => {
                let note = if feed.notes.is_empty() { String::new() } else { format!(": {}", feed.notes) };
                s.toast(format!("Vibex {} is available{note}\npalette → Install update and restart", feed.version), 8.0);
                for s in self.sessions.values_mut() {
                    s.update = Some(feed.clone());
                }
            }
            update::Msg::UpToDate if manual => s.toast(format!("Vibex {} is up to date", env!("CARGO_PKG_VERSION")), 3.0),
            update::Msg::UpToDate => {}
            update::Msg::Failed(e) if manual => s.toast(e, 6.0),
            update::Msg::Failed(_) => {}
            update::Msg::Ready(path, feed) => match update::launch_installer(&path) {
                Ok(()) => {
                    // the installer waits for us to exit, then reopens Vibex
                    let _ = feed;
                    for s in self.sessions.values_mut() {
                        s.close_window();
                    }
                }
                Err(e) => s.toast(format!("could not start the installer: {e}"), 6.0),
            },
        }
    }

    /// Carry out `vibex cli ...`; returns the text to print.
    fn remote(&mut self, el: &ActiveEventLoop, args: &[String]) -> String {
        let _ = el;
        if !self.sessions.values().any(|s| s.config.remote_control) {
            return "error: remote control is off: turn it on in Settings → Terminal, or config.remote_control = true".into();
        }
        // options and positional words
        let mut flags: HashMap<String, String> = HashMap::new();
        let mut words: Vec<String> = Vec::new();
        let mut program: Option<Vec<String>> = None;
        let mut it = args.iter().skip(1).cloned();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--" => {
                    program = Some(it.by_ref().take_while(|w| w != "--from-pane").collect());
                }
                "--right" | "--down" | "--enter" | "--new-window" | "--block" | "--all" | "--copy" => {
                    flags.insert(a, String::new());
                }
                f if f.starts_with("--") => {
                    flags.insert(f.to_string(), it.next().unwrap_or_default());
                }
                _ => words.push(a),
            }
        }
        // `--from-pane` may sit after `--`: pick it out of the argv too
        if let Some(pos) = args.iter().position(|a| a == "--from-pane") {
            if let Some(v) = args.get(pos + 1) {
                flags.entry("--from-pane".into()).or_insert_with(|| v.clone());
            }
        }
        let pane: Option<PaneId> = flags.get("--pane").or(flags.get("--from-pane")).and_then(|v| v.parse().ok());
        let find = |sessions: &HashMap<WindowId, Session>, id: PaneId| sessions.iter().find(|(_, s)| s.panes.contains_key(&id)).map(|(w, _)| *w);
        // the window to act in: the pane's, else a visible one
        let window = pane
            .and_then(|p| find(&self.sessions, p))
            .or_else(|| self.sessions.iter().find(|(_, s)| s.focused).map(|(w, _)| *w))
            .or_else(|| self.sessions.iter().find(|(_, s)| s.detached.is_none()).map(|(w, _)| *w));
        let cmd = args.first().map(String::as_str).unwrap_or("");
        let json = |s: &str| {
            let mut o = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    '\n' => o.push_str("\\n"),
                    c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                    c => o.push(c),
                }
            }
            o.push('"');
            o
        };
        let need = |p: Option<PaneId>| p.ok_or_else(|| "error: which pane? pass --pane N (see `vibex cli list`)".to_string());
        let r: Result<String, String> = (|| {
            match cmd {
                "list" => {
                    let mut out = Vec::new();
                    for (wid, s) in &self.sessions {
                        let bar = s.tab_bar_model();
                        for (ti, t) in s.tabs.iter().enumerate() {
                            let tab_title = bar.tabs.get(ti).map_or(String::new(), |b| b.title.clone());
                            let mut ids = t.root.panes();
                            ids.extend(t.float);
                            for id in ids {
                                let Some(p) = s.panes.get(&id) else { continue };
                                out.push(format!(
                                    "{{\"window\":{},\"workspace\":{},\"detached\":{},\"tab\":{},\"tab_title\":{},\"pane\":{id},\"title\":{},\"cwd\":{},\"active\":{},\"floating\":{}}}",
                                    u64::from(*wid),
                                    json(&s.name),
                                    s.detached.is_some(),
                                    ti + 1,
                                    json(&tab_title),
                                    json(&p.title()),
                                    p.term.cwd().map_or("null".into(), json),
                                    ti == s.active_tab && s.active_id() == id,
                                    t.float == Some(id),
                                ));
                            }
                        }
                    }
                    Ok(out.join("\n"))
                }
                "spawn" | "split" | "popup" => {
                    let w = window.ok_or("error: no window open")?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    if let Some(p) = pane {
                        s.reveal_pane(p);
                    }
                    if cmd == "popup" {
                        let argv = program.clone().filter(|p| !p.is_empty()).ok_or("error: popup needs -- PROGRAM ARGS")?;
                        s.popup(&argv.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" "));
                        return Ok(s.active_id().to_string());
                    }
                    let cwd = flags.get("--cwd").cloned().or_else(|| s.current_cwd());
                    let id = match &program {
                        Some(argv) if !argv.is_empty() => {
                            let mut c = config::base_command(argv[0].clone());
                            c.args = argv[1..].to_vec();
                            let c = pane::with_cwd(c, cwd.as_deref());
                            s.spawn_command(&argv[0], c).map_err(|e| format!("error: {e}"))?
                        }
                        _ => s.spawn_pane(flags.get("--profile").map(String::as_str), cwd).map_err(|e| format!("error: {e}"))?,
                    };
                    if cmd == "spawn" {
                        s.settings_ui.active = false;
                        s.tabs.push(Tab::new(id));
                        s.active_tab = s.tabs.len() - 1;
                    } else {
                        let dir = if flags.contains_key("--down") { SplitDir::Vertical } else { SplitDir::Horizontal };
                        let tab = &mut s.tabs[s.active_tab];
                        let target = tab.active;
                        tab.root.split(target, id, dir);
                        tab.active = id;
                        tab.float_shown = false;
                    }
                    s.after_focus_change();
                    Ok(id.to_string())
                }
                "send-text" => {
                    let id = need(pane)?;
                    let w = find(&self.sessions, id).ok_or("error: no such pane")?;
                    let mut text = words.join(" ");
                    if flags.contains_key("--enter") {
                        text.push('\r');
                    }
                    self.sessions.get_mut(&w).unwrap().panes.get_mut(&id).unwrap().write(text.as_bytes());
                    Ok(String::new())
                }
                "get-text" => {
                    let id = need(pane)?;
                    let w = find(&self.sessions, id).ok_or("error: no such pane")?;
                    let t = &self.sessions[&w].panes[&id].term;
                    let last = t.total_lines() - t.rows() + t.cursor.y;
                    let n = flags.get("--lines").and_then(|v| v.parse::<usize>().ok()).unwrap_or(t.rows());
                    let from = (last + 1).saturating_sub(n.max(1));
                    Ok(t.text_range((0, from), (t.cols() - 1, last)).trim_end().to_string())
                }
                "export" => {
                    use export::Format;
                    use export_ui::ExportOut;
                    let id = need(pane)?;
                    let w = find(&self.sessions, id).ok_or("error: no such pane")?;
                    let out_path = flags.get("--out").map(PathBuf::from);
                    let format = match flags.get("--format") {
                        Some(f) => Format::parse(f).ok_or_else(|| format!("error: unknown format '{f}' (png, txt, ans, html, svg, md)"))?,
                        None => out_path.as_ref().and_then(|p| p.extension()).and_then(|e| Format::parse(&e.to_string_lossy())).unwrap_or(Format::Png),
                    };
                    let s = self.sessions.get_mut(&w).unwrap();
                    let t = &s.panes[&id].term;
                    let last = t.total_lines() - t.rows() + t.cursor.y;
                    let num = |k: &str| flags.get(k).and_then(|v| v.parse::<usize>().ok());
                    let job = if flags.contains_key("--block") {
                        s.job_block(id, None).ok_or("error: no finished command (needs shell integration)")?
                    } else if flags.contains_key("--all") {
                        s.job_all(id).ok_or("error: no such pane")?
                    } else if let Some(n) = num("--last") {
                        s.job_lines(id, (last + 1).saturating_sub(n.max(1)), last).ok_or("error: no such pane")?
                    } else if num("--from").is_some() || num("--to").is_some() {
                        // 1-based, counted from the oldest line kept
                        let a = num("--from").unwrap_or(1).max(1) - 1;
                        let b = num("--to").unwrap_or(last + 1).max(1) - 1;
                        s.job_lines(id, a, b).ok_or("error: no such pane")?
                    } else {
                        s.job_screen(id).ok_or("error: no such pane")?
                    };
                    let out = match (flags.contains_key("--copy"), format) {
                        (true, Format::Png) => ExportOut::CopyImage,
                        (true, _) => ExportOut::CopyText,
                        (false, f) => ExportOut::Save(f),
                    };
                    s.run_export(&job, out, out_path).map_err(|e| format!("error: {e}"))
                }
                "focus" => {
                    let id = need(pane)?;
                    let w = find(&self.sessions, id).ok_or("error: no such pane")?;
                    self.sessions.get_mut(&w).unwrap().reveal_pane(id);
                    self.show_workspace(w);
                    Ok(String::new())
                }
                "close" => {
                    let id = need(pane)?;
                    let w = find(&self.sessions, id).ok_or("error: no such pane")?;
                    self.sessions.get_mut(&w).unwrap().close_pane(id);
                    Ok(String::new())
                }
                "set-tab-title" => {
                    let id = need(pane)?;
                    let w = find(&self.sessions, id).ok_or("error: no such pane")?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    let title = words.join(" ");
                    if let Some(t) = s.tabs.iter_mut().find(|t| t.root.contains(id) || t.float == Some(id)) {
                        t.title = (!title.is_empty()).then_some(title);
                    }
                    s.relayout_tabbar();
                    s.window.request_redraw();
                    Ok(String::new())
                }
                "notify" => {
                    let w = window.ok_or("error: no window open")?;
                    let title = words.first().cloned().unwrap_or_else(|| "Vibex".into());
                    let body = words.get(1..).map(|b| b.join(" ")).unwrap_or_default();
                    self.sessions.get_mut(&w).unwrap().notify(&title, &body);
                    Ok(String::new())
                }
                "action" => {
                    // any action by name: `vibex cli action SplitRight`
                    let w = window.ok_or("error: no window open")?;
                    let name = words.first().ok_or("error: action needs a NAME (keys.rs names, e.g. ToggleRecording)")?;
                    let a = keys::Action::from_id(name).ok_or_else(|| format!("error: no action named '{name}'"))?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    if let Some(p) = pane {
                        s.reveal_pane(p);
                    }
                    s.perform(a);
                    Ok(String::new())
                }
                "enter" => {
                    // press Enter, with the dangerous-command check
                    let w = window.ok_or("error: no window open")?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    if let Some(p) = pane {
                        s.reveal_pane(p);
                    }
                    if s.check_enter_now() {
                        let q = s.confirm.as_ref().map(|c| c.view.title.clone()).unwrap_or_default();
                        Ok(format!("asked: {q}"))
                    } else {
                        s.write_active(b"\r");
                        Ok("sent".into())
                    }
                }
                "notes" => {
                    // add a line to the pane's notes (opening them)
                    let w = window.ok_or("error: no window open")?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    if let Some(p) = pane {
                        s.reveal_pane(p);
                    }
                    let cwd = s.current_cwd();
                    let p = s.active_mut();
                    let n = p.notes.get_or_insert_with(|| notes::Notes::open(cwd.as_deref()));
                    n.focused = false;
                    if !words.is_empty() {
                        if !n.text.is_empty() && !n.text.ends_with('\n') {
                            n.text.push('\n');
                        }
                        n.cursor = n.text.len();
                        n.insert(&(words.join(" ") + "\n"));
                    }
                    s.relayout();
                    Ok(String::new())
                }
                "dialog" => {
                    // the open question, if any (and `--answer yes|no`)
                    let w = window.ok_or("error: no window open")?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    let shown = s.confirm.as_ref().map_or("null".to_string(), |c| {
                        format!("{{\"title\":{},\"lines\":[{}]}}", json(&c.view.title), c.view.lines.iter().map(|l| json(l)).collect::<Vec<_>>().join(","))
                    });
                    if let Some(a) = flags.get("--answer") {
                        s.confirm_answer(a == "yes");
                    }
                    Ok(shown)
                }
                "prompt" => {
                    // what's typed at the prompt, and Vibex's suggestion
                    let w = window.ok_or("error: no window open")?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    if let Some(p) = pane {
                        s.reveal_pane(p);
                    }
                    let input = s.active().term.prompt_input();
                    let sugg = s.suggestion();
                    Ok(format!(
                        "{{\"typed\":{},\"at_end\":{},\"suggestion\":{}}}",
                        input.as_ref().map_or("null".into(), |i| json(&i.text)),
                        input.as_ref().is_some_and(|i| i.at_end),
                        sugg.as_deref().map_or("null".into(), json),
                    ))
                }
                "play" => {
                    let w = window.ok_or("error: no window open")?;
                    let file = words.first().ok_or("error: play needs a .cast FILE")?;
                    self.sessions.get_mut(&w).unwrap().play_recording(file);
                    Ok(String::new())
                }
                "engagement" => {
                    // engagement start NAME [--scope S] | stop | scope S | status | report
                    let w = window.ok_or("error: no window open")?;
                    let sub = words.first().cloned().unwrap_or_else(|| "status".into());
                    let s = self.sessions.get_mut(&w).unwrap();
                    match sub.as_str() {
                        "start" => {
                            let name = words.get(1).ok_or("error: engagement start NAME")?;
                            let mut e = engagement::Engagement::open(name).map_err(|e| format!("error: {e}"))?;
                            if let Some(sc) = flags.get("--scope") {
                                e.set_scope(sc).map_err(|e| format!("error: {e}"))?;
                            }
                            s.apply_setting(settings::Change::Set("engagement", settings::Val::Str(e.name.clone())));
                            Ok(format!("on: {}", e.dir.display()))
                        }
                        "stop" => {
                            s.apply_setting(settings::Change::Set("engagement", settings::Val::Str(String::new())));
                            Ok("off".into())
                        }
                        "scope" => {
                            let text = words[1..].join(" ");
                            let e = s.eng.as_mut().ok_or("error: no engagement on")?;
                            e.set_scope(&text).map_err(|e| format!("error: {e}"))?;
                            Ok(e.scope.text.join(", "))
                        }
                        "report" => {
                            let e = s.eng.as_ref().ok_or("error: no engagement on")?;
                            let out = e.dir.join("report.html");
                            std::fs::write(&out, engagement::report(e)).map_err(|e| format!("error: {e}"))?;
                            Ok(out.display().to_string())
                        }
                        _ => Ok(match &s.eng {
                            Some(e) => format!(
                                "{{\"name\":{},\"dir\":{},\"scope\":{},\"loot\":{}}}",
                                json(&e.name),
                                json(&e.dir.display().to_string()),
                                json(&e.scope.text.join(", ")),
                                engagement::read_loot(&e.loot_path()).len()
                            ),
                            None => "null".into(),
                        }),
                    }
                }
                "move-tab" => {
                    // the tab holding --pane (default: the active one) goes to
                    // --window W, or a new window
                    let w = window.ok_or("error: no window open")?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    let i = match pane {
                        Some(p) => s.tabs.iter().position(|t| t.root.contains(p) || t.float == Some(p)).ok_or("error: no such pane")?,
                        None => s.active_tab,
                    };
                    let to = match flags.get("--window").and_then(|v| v.parse::<u64>().ok()) {
                        Some(id) if !flags.contains_key("--new-window") => MoveTarget::Window(id, None),
                        _ => MoveTarget::NewWindow(None),
                    };
                    s.move_tab(i, to);
                    Ok(String::new())
                }
                "settings" => {
                    let w = window.ok_or("error: no window open")?;
                    let s = self.sessions.get_mut(&w).unwrap();
                    s.settings_ui.show(words.first().and_then(|p| settings_ui::Page::named(p)));
                    Ok(String::new())
                }
                other => Err(format!("error: unknown command '{other}' (vibex cli help)")),
            }
        })();
        if let Some(w) = window {
            if let Some(s) = self.sessions.get(&w) {
                s.window.request_redraw();
            }
        }
        r.unwrap_or_else(|e| e)
    }

    /// Show and focus a workspace (attaching it if it was detached).
    fn show_workspace(&mut self, id: WindowId) {
        if let Some(s) = self.sessions.get_mut(&id) {
            s.detached = None;
            s.window.set_visible(true);
            s.window.set_minimized(false);
            s.window.focus_window();
            s.window.request_redraw();
        }
    }

    /// Open windows sessions asked for, drop closed ones, exit when none
    /// are left.
    /// Tabs moving between windows.
    fn move_tabs(&mut self, el: &ActiveEventLoop) {
        let moves: Vec<(WindowId, TabMove)> =
            self.sessions.iter_mut().flat_map(|(id, s)| std::mem::take(&mut s.tab_moves).into_iter().map(|m| (*id, m)).collect::<Vec<_>>()).collect();
        for (from, m) in moves {
            // where it goes: (window, index) or a new window at a point
            let target = match m.to {
                MoveTarget::Window(id, at) => self.sessions.keys().copied().find(|w| u64::from(*w) == id).map(|w| (w, at)),
                MoveTarget::NewWindow(_) => None,
                MoveTarget::Drop(p) => self.sessions.iter().find_map(|(id, s)| {
                    if *id == from || s.detached.is_some() || s.quake {
                        return None;
                    }
                    let origin = s.window.inner_position().ok()?;
                    let size = s.window.inner_size();
                    let (x, y) = ((p.x - origin.x) as f32, (p.y - origin.y) as f32);
                    let bar = s.tabbar.height.max(32.0);
                    (x >= 0.0 && y >= 0.0 && x < size.width as f32 && y < bar).then(|| {
                        let at = match s.tabbar.hit(x, bar / 2.0) {
                            Some(TabHit::Tab(i) | TabHit::CloseTab(i)) => Some(i),
                            _ => None,
                        };
                        (*id, at)
                    })
                }),
            };
            match target {
                Some((w, at)) => {
                    let s = self.sessions.get_mut(&w).unwrap();
                    let i = at.unwrap_or(s.tabs.len()).min(s.tabs.len());
                    s.tabs.insert(i, m.tab);
                    s.panes.extend(m.panes);
                    s.active_tab = i;
                    s.after_focus_change();
                    s.relayout();
                    s.relayout_tabbar();
                    s.session_dirty = true;
                    s.window.focus_window();
                    s.window.request_redraw();
                }
                None => {
                    let pos = match m.to {
                        MoveTarget::NewWindow(p) => p,
                        MoveTarget::Drop(p) => Some(PhysicalPosition::new(p.x - 120, p.y - 16)),
                        MoveTarget::Window(..) => None,
                    };
                    let mut st = startup_for(single::OpenRequest::default());
                    st.restore = None;
                    st.adopt = Some(Adopt { tab: m.tab, panes: m.panes, pos });
                    self.open(el, st);
                }
            }
        }
    }

    fn settle(&mut self, el: &ActiveEventLoop) {
        self.move_tabs(el);
        let requests: Vec<single::OpenRequest> = self.sessions.values_mut().flat_map(|s| std::mem::take(&mut s.open_requests)).collect();
        for req in requests {
            self.open(el, startup_for(req));
        }
        let ws: Vec<Action> = self.sessions.values_mut().flat_map(|s| std::mem::take(&mut s.ws_requests)).collect();
        for a in ws {
            match a {
                Action::Workspace(id, _) => {
                    if let Some(wid) = self.sessions.keys().copied().find(|w| u64::from(*w) == id) {
                        self.show_workspace(wid);
                    }
                }
                Action::QuitAll => {
                    for s in self.sessions.values_mut() {
                        s.close_window();
                    }
                }
                _ => {}
            }
        }
        // name new windows, and tell every window about the others
        let mut taken: Vec<String> = self.sessions.values().map(|s| s.name.clone()).filter(|n| !n.is_empty()).collect();
        for s in self.sessions.values_mut() {
            if s.name.is_empty() {
                // "main", then the first free "workspace-N"
                let name = std::iter::once("main".to_string()).chain((2..).map(|k| format!("workspace-{k}"))).find(|n| !taken.contains(n)).unwrap_or_default();
                taken.push(name.clone());
                s.name = name;
            }
        }
        let list: Vec<(u64, String, bool)> = self.sessions.iter().map(|(id, s)| (u64::from(*id), s.name.clone(), s.detached.is_some())).collect();
        for s in self.sessions.values_mut() {
            if s.workspaces != list {
                s.workspaces = list.clone();
            }
        }
        let closed: Vec<WindowId> = self.sessions.iter().filter(|(_, s)| s.quit).map(|(id, _)| *id).collect();
        for id in closed {
            if let Some(s) = self.sessions.remove(&id) {
                if s.saves_session {
                    if let Some(next) = self.sessions.values_mut().next() {
                        next.saves_session = true;
                    }
                }
            }
        }
        if self.sessions.is_empty() && self.startup.is_none() {
            el.exit();
        }
    }

    fn start(&mut self, el: &ActiveEventLoop, startup: Startup) -> Result<Session, String> {
        let cfg = &startup.config;
        let custom_chrome = cfg.decorations == Decorations::Vibex;
        let want_transparent = cfg.opacity < 1.0 && cfg.renderer != RendererKind::Cpu;
        let size = startup.restore.as_ref().and_then(|r| r.size).unwrap_or(cfg.window_size);
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut attrs = Window::default_attributes()
            .with_title("Vibex")
            .with_inner_size(LogicalSize::new(size.0, size.1))
            .with_maximized(startup.restore.as_ref().is_some_and(|r| r.maximized))
            .with_decorations(!custom_chrome)
            .with_transparent(want_transparent)
            // dark window chrome + dark acrylic for dark themes
            .with_theme(Some({
                let bg = cfg.theme.background;
                let luma = (((bg >> 16) & 0xff) * 299 + ((bg >> 8) & 0xff) * 587 + (bg & 0xff) * 114) / 1000;
                if luma < 128 {
                    winit::window::Theme::Dark
                } else {
                    winit::window::Theme::Light
                }
            }))
            // shown once the accessibility adapter is attached
            .with_visible(false)
            // VIBEX_BACKGROUND=1: open without taking focus (automated checks)
            .with_active(std::env::var_os("VIBEX_BACKGROUND").is_none());
        #[cfg(windows)]
        {
            use winit::platform::windows::{BackdropType, IconExtWindows, WindowAttributesExtWindows};
            use winit::window::Icon;
            if want_transparent {
                attrs = attrs.with_system_backdrop(BackdropType::TransientWindow);
                // No GDI surface: it sits behind the GPU surface and is
                // opaque white, which hid the acrylic (the "grey" look).
                attrs = attrs.with_no_redirection_bitmap(true);
            }
            // icon resource 1, embedded by build.rs from assets/vibex.ico
            if let Ok(icon) = Icon::from_resource(1, None) {
                attrs = attrs.with_window_icon(Some(icon));
            }
            if let Ok(icon) = Icon::from_resource(1, Some(winit::dpi::PhysicalSize::new(256, 256))) {
                attrs = attrs.with_taskbar_icon(Some(icon));
            }
        }
        if let Some(pos) = startup.adopt.as_ref().and_then(|a| a.pos) {
            attrs = attrs.with_position(pos).with_maximized(false);
        }
        if startup.quake {
            // borderless, on top, full width across the top of the screen
            // the pointer is on (else the primary one)
            let height = cfg.quake.as_ref().map_or(0.45, |q| q.2);
            let mon = el.primary_monitor().or_else(|| el.available_monitors().next());
            if let Some(m) = mon {
                let (pos, size) = (m.position(), m.size());
                attrs = attrs.with_position(pos).with_inner_size(winit::dpi::PhysicalSize::new(size.width, (size.height as f32 * height) as u32));
            }
            attrs = attrs.with_decorations(false).with_maximized(false).with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            #[cfg(windows)]
            {
                use winit::platform::windows::WindowAttributesExtWindows;
                attrs = attrs.with_skip_taskbar(true);
            }
        }
        let window = Arc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let access = accesskit_winit::Adapter::with_event_loop_proxy(el, &window, self.proxy.clone());
        window.set_cursor(CursorIcon::Text);
        // CJK input methods and the Win+. emoji picker
        window.set_ime_allowed(true);
        if custom_chrome {
            #[cfg(windows)]
            {
                use winit::platform::windows::WindowExtWindows;
                window.set_undecorated_shadow(true);
            }
            round_corners(&window);
        }
        set_dark_mode(&window, cfg.theme.background);
        let size = window.inner_size();

        let backend = match cfg.renderer {
            RendererKind::Cpu => None,
            _ => match GpuRenderer::new(window.clone(), size.width, size.height, want_transparent) {
                Ok(gpu) => {
                    eprintln!("vibex: GPU renderer on {}{}", gpu.adapter_info, if gpu.transparent { ", transparent" } else { "" });
                    if gpu.transparent {
                        enable_backdrop(&window, cfg.glass);
                    }
                    Some(Backend::Gpu(Box::new(gpu)))
                }
                Err(e) if cfg.renderer == RendererKind::Gpu => return Err(format!("GPU renderer failed: {e}")),
                Err(e) => {
                    eprintln!("vibex: GPU renderer unavailable ({e}); using CPU renderer");
                    None
                }
            },
        };
        let backend = match backend {
            Some(b) => b,
            None => {
                window.set_transparent(false);
                let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
                let surface = softbuffer::Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
                Backend::Cpu { _context: context, surface, last: Vec::new(), scratch: Vec::new() }
            }
        };

        let base_font_px = font_px(cfg.font_size_pt, window.scale_factor());
        let mut fonts = FontSet::load(&cfg.fonts, base_font_px)?;
        fonts.ligatures = cfg.ligatures;
        let mut renderer = Renderer::new(fonts, cfg.theme.clone());
        renderer.padding_x = cfg.padding.0;
        renderer.padding_y = cfg.padding.1;
        renderer.status_bar = cfg.status_bar;
        renderer.glass = cfg.glass;
        renderer.ui_scale = window.scale_factor() as f32;

        let blink_interval = Duration::from_millis(cfg.blink_interval_ms);
        let now = Instant::now();
        let cfg = startup.config.clone();
        let mut s = Session {
            window,
            backend,
            renderer,
            proxy: self.proxy.clone(),
            opacity: cfg.opacity,
            smooth_scroll: cfg.smooth_scroll,
            mods: ModifiersState::empty(),
            focused: true,
            clipboard: arboard::Clipboard::new().ok(),
            mouse: PhysicalPosition::new(0.0, 0.0),
            base_font_px,
            quit: false,
            tabs: Vec::new(),
            active_tab: 0,
            panes: HashMap::new(),
            next_pane: 1,
            dividers: Vec::new(),
            dragging: None,
            selecting_pane: None,
            custom_chrome,
            tabbar: TabBarLayout::default(),
            tab_hover: None,
            edge: None,
            last_bar_click: None,
            fullscreen: false,
            palette: None,
            quick: None,
            preedit: None,
            rename: None,
            tab_drag: None,
            last_tab_click: None,
            last_click: None,
            click_count: 0,
            multi_click: Duration::from_millis(cfg.multi_click_ms),
            blink_on: true,
            next_blink: now + blink_interval,
            blink_interval,
            engine: startup.engine,
            watcher: ConfigWatcher::new(startup.config_path),
            next_config_poll: now + Duration::from_millis(500),
            config_error: startup.error,
            toast: None,
            export_job: None,
            mark: None,
            status: None,
            next_status: now,
            leader_until: None,
            settings: startup.settings,
            session_dirty: false,
            next_session_save: now + Duration::from_secs(5),
            restored_size: None,
            open_requests: Vec::new(),
            saves_session: true,
            access: Some(access),
            update: None,
            update_status: String::new(),
            editors_found: None,
            editors_status: String::new(),
            next_editor_check: now + Duration::from_secs(3600),
            settings_ui: settings_ui::SettingsUi::default(),
            settings_drag: None,
            settings_rect: PxRect::default(),
            chrome_layout: ChromeLayout::default(),
            menu: None,
            name: startup.name.clone().unwrap_or_default(),
            detached: None,
            workspaces: Vec::new(),
            ws_requests: Vec::new(),
            quake: startup.quake,
            tab_moves: Vec::new(),
            suggest: None,
            confirm: None,
            reveal_secrets: false,
            closed: Vec::new(),
            focus_mode: false,
            focus_fullscreen: false,
            held_notes: Vec::new(),
            eng: None,
            config: cfg,
        };
        if let Some(a) = startup.adopt {
            // a tab dragged (or moved) here from another window
            s.tabs.push(a.tab);
            s.panes.extend(a.panes);
            s.active_tab = 0;
            s.after_focus_change();
            s.relayout();
            return Ok(s);
        }
        if let Some(saved) = startup.restore {
            s.restore_tabs(&saved);
            if !s.tabs.is_empty() {
                return Ok(s);
            }
        }
        match startup.exec {
            Some(argv) => {
                // `vibex -- program args...`
                let (program, args) = argv.split_first().ok_or("nothing to run after --")?;
                let mut cmd = config::base_command(program.clone());
                cmd.args = args.to_vec();
                let cmd = pane::with_cwd(cmd, startup.cwd.as_deref());
                let label = std::path::Path::new(program).file_stem().map_or(program.clone(), |s| s.to_string_lossy().into_owned());
                let id = s.spawn_command(&label, cmd)?;
                if let Some(p) = s.panes.get_mut(&id) {
                    p.hold = true;
                }
                s.tabs.push(Tab::new(id));
                s.active_tab = 0;
                s.after_focus_change();
            }
            None => s.new_tab_in(Some(&startup.profile), startup.cwd.clone())?,
        }
        Ok(s)
    }
}

impl Session {
    // ---------------------------------------------------- session restore

    fn restore_tabs(&mut self, saved: &session::Saved) {
        let force = false;
        self.restore_tabs_with(saved, force, "restored from the last session");
    }

    /// `force`: feed saved scrollback even if restore_scrollback is off.
    fn restore_tabs_with(&mut self, saved: &session::Saved, force: bool, note: &str) {
        let note = format!("\x1b[0m\x1b[2m\u{2500}\u{2500} {note} \u{2500}\u{2500}\x1b[0m\r\n");
        for t in &saved.tabs {
            let mut spawn = |n: &session::SavedNode| -> Option<PaneId> {
                let session::SavedNode::Pane { profile, cwd, buffer, run } = n else { return None };
                let profile = self.config.profile(profile).map(|p| p.name.clone());
                // only reuse folders that still exist (POSIX paths go to WSL)
                let cwd = cwd.as_deref().filter(|c| c.starts_with('/') || std::path::Path::new(c).is_dir()).map(str::to_string);
                let id = self.spawn_pane(profile.as_deref(), cwd).ok()?;
                let restore = force || self.config.restore_scrollback;
                if let Some(p) = self.panes.get_mut(&id) {
                    if let (true, Some(name), Some(dir)) = (restore, buffer, session::buffers_dir()) {
                        if let Ok(bytes) = std::fs::read(dir.join(name)) {
                            p.feed(&bytes);
                            p.feed(note.as_bytes());
                            // all of it into history, behind a fence: the
                            // new shell starts on a clean screen at the top,
                            // and growing the window won't pull it back
                            p.term.seal_history();
                        }
                    }
                    if let Some(cmd) = run {
                        p.write(format!("{cmd}\r").as_bytes());
                    }
                }
                Some(id)
            };
            if let Some(root) = session::rebuild(&t.root, &mut spawn) {
                let panes = root.panes();
                let active = panes.get(t.active).copied().unwrap_or(panes[0]);
                self.tabs.push(Tab {
                    zoomed: t.zoomed && panes.len() > 1,
                    broadcast: false,
                    float: None,
                    float_shown: false,
                    root,
                    active,
                    title: t.title.clone(),
                    color: t.color,
                });
            }
        }
        if !self.tabs.is_empty() {
            self.active_tab = saved.active_tab.min(self.tabs.len() - 1);
            self.after_focus_change();
        }
    }

    /// The layout tree as saved data; with `buffers`, each pane's
    /// scrollback is written to `buffers/` too (names collected in `out`).
    fn saved_node(&self, n: &Node, buffers: Option<&mut Vec<String>>) -> session::SavedNode {
        match n {
            Node::Leaf(id) => {
                let p = &self.panes[id];
                let mut buffer = None;
                if let (Some(list), Some(dir)) = (buffers, session::buffers_dir()) {
                    let mut text = p.term.dump_vt(2000);
                    // waiting at a prompt: that last line (and anything half
                    // typed on it) belongs to this session, not the next
                    if p.term.prompt_input().is_some() {
                        let body = text.trim_end_matches("\x1b[0m\r\n").len();
                        let cut = text[..body].rfind("\r\n").map_or(0, |i| i + 2);
                        text.truncate(cut);
                    }
                    if !text.is_empty() {
                        let name = format!("{}-{id}.vt", std::process::id());
                        if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(dir.join(&name), text).is_ok() {
                            list.push(name.clone());
                            buffer = Some(name);
                        }
                    }
                }
                session::SavedNode::Pane { profile: p.profile.clone(), cwd: p.term.cwd().map(str::to_string), buffer, run: None }
            }
            Node::Split { dir, ratio, a, b } => {
                let (a, b) = match buffers {
                    Some(list) => {
                        let a = self.saved_node(a, Some(&mut *list));
                        (a, self.saved_node(b, Some(list)))
                    }
                    None => (self.saved_node(a, None), self.saved_node(b, None)),
                };
                session::SavedNode::Split { dir: *dir, ratio: *ratio, a: Box::new(a), b: Box::new(b) }
            }
        }
    }

    /// The window's tabs as saved data (for session.lua and layout files).
    fn saved_tabs(&self, buffers: Option<&mut Vec<String>>) -> Vec<session::SavedTab> {
        let mut buffers = buffers;
        self.tabs
            .iter()
            .filter(|t| t.root.panes().iter().all(|id| self.panes.get(id).is_some_and(|p| !p.hold)))
            .map(|t| session::SavedTab {
                root: self.saved_node(&t.root, buffers.as_deref_mut()),
                active: t.root.panes().iter().position(|id| *id == t.active).unwrap_or(0),
                zoomed: t.zoomed,
                title: t.title.clone(),
                color: t.color,
            })
            .collect()
    }

    fn save_session(&mut self) {
        self.save_session_with(false);
    }

    /// `last`: the window is closing, so also save scrollback contents.
    fn save_session_with(&mut self, last: bool) {
        self.session_dirty = false;
        if !self.config.restore_session || !self.saves_session {
            return;
        }
        let Some(path) = session::path() else { return };
        let mut written: Vec<String> = Vec::new();
        let with_buffers = last && self.config.restore_scrollback;
        let tabs = self.saved_tabs(if with_buffers { Some(&mut written) } else { None });
        if last {
            // keep only this exit's buffers
            session::prune_buffers(&written);
        }
        if tabs.is_empty() {
            return;
        }
        let scale = self.window.scale_factor();
        let size = self.restored_size.or_else(|| {
            let s = self.window.inner_size();
            Some((s.width as f64 / scale, s.height as f64 / scale))
        });
        let saved = session::Saved { active_tab: self.active_tab.min(tabs.len() - 1), tabs, size, maximized: self.window.is_maximized() };
        if let Err(e) = session::save(&path, &saved) {
            eprintln!("vibex: could not save session: {e}");
        }
    }

    /// The user closed the last tab: nothing to reopen next time.
    fn forget_session(&self) {
        if let Some(path) = session::path() {
            let _ = std::fs::remove_file(path);
        }
    }

    // ------------------------------------------------------- panes + tabs

    fn active_id(&self) -> PaneId {
        let tab = &self.tabs[self.active_tab];
        tab.shown_float().unwrap_or(tab.active)
    }

    fn active(&self) -> &Pane {
        &self.panes[&self.active_id()]
    }

    fn active_mut(&mut self) -> &mut Pane {
        let id = self.active_id();
        self.panes.get_mut(&id).expect("active pane")
    }

    fn pane_options(&self) -> PaneOptions {
        PaneOptions { scrollback: self.config.scrollback, cursor_blink: self.config.cursor_blink, cursor_shape: self.config.cursor_shape }
    }

    /// Start a shell in a new pane (not yet placed in any tab).
    fn spawn_pane(&mut self, profile: Option<&str>, cwd: Option<String>) -> Result<PaneId, String> {
        let name = profile.unwrap_or(&self.config.default_profile).to_string();
        let p = self.config.profile(&name).ok_or_else(|| format!("no profile named '{name}'"))?.clone();
        let cmd = pane::with_cwd(p.command.clone(), cwd.as_deref());
        self.spawn_command(&p.name, cmd)
    }

    fn spawn_command(&mut self, label: &str, cmd: vibex_pty::Command) -> Result<PaneId, String> {
        let p = config::Profile { name: label.to_string(), command: cmd.clone() };
        let (cols, rows) = self.tabs.get(self.active_tab).and_then(|t| self.panes.get(&t.active)).map_or((80, 24), |p| (p.term.cols(), p.term.rows()));
        // unique across every window, so `vibex cli --pane N` is unambiguous
        static NEXT_PANE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        let id = NEXT_PANE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.next_pane = id + 1;
        let mut cmd = cmd;
        cmd.env.push(("VIBEX_PANE".into(), id.to_string()));
        commands::prepare(&mut cmd, self.config.linux_commands);
        let proxy = self.proxy.clone();
        let pane = Pane::spawn(id, &p.name, &cmd, cols, rows, &self.pane_options(), move || {
            let _ = proxy.send_event(AppEvent::Pty);
        })?;
        self.panes.insert(id, pane);
        Ok(id)
    }

    fn current_cwd(&self) -> Option<String> {
        self.tabs.get(self.active_tab)?;
        self.active().term.cwd().map(str::to_string)
    }

    fn new_tab(&mut self, profile: Option<&str>) -> Result<(), String> {
        let cwd = if self.tabs.is_empty() { None } else { self.current_cwd() };
        self.new_tab_in(profile, cwd)
    }

    fn new_tab_in(&mut self, profile: Option<&str>, cwd: Option<String>) -> Result<(), String> {
        let id = self.spawn_pane(profile, cwd)?;
        self.settings_ui.close();
        self.tabs.push(Tab::new(id));
        self.active_tab = self.tabs.len() - 1;
        self.after_focus_change();
        Ok(())
    }

    fn split(&mut self, dir: SplitDir, profile: Option<&str>) {
        let cwd = self.current_cwd();
        match self.spawn_pane(profile, cwd) {
            Ok(new) => {
                let tab = &mut self.tabs[self.active_tab];
                let target = tab.active;
                tab.root.split(target, new, dir);
                tab.active = new;
                tab.zoomed = false;
                self.after_focus_change();
            }
            Err(e) => self.toast(e, 4.0),
        }
    }

    fn close_pane(&mut self, id: PaneId) {
        if let Some(p) = self.panes.remove(&id) {
            p.kill();
        }
        if self.selecting_pane == Some(id) {
            self.selecting_pane = None;
        }
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.float == Some(id)) {
            tab.float = None;
            tab.float_shown = false;
            self.after_focus_change();
            return;
        }
        let Some(ti) = self.tabs.iter().position(|t| t.root.contains(id)) else { return };
        let tab = &mut self.tabs[ti];
        if !tab.root.remove(id) {
            // last pane of the tab
            self.tabs.remove(ti);
            if self.tabs.is_empty() {
                self.forget_session();
                self.quit = true;
                return;
            }
            if self.active_tab >= ti && self.active_tab > 0 {
                self.active_tab -= 1;
            }
        } else if tab.active == id {
            tab.active = tab.root.panes()[0];
            tab.zoomed = false;
        }
        self.after_focus_change();
    }

    /// Would closing this pane stop something? (a command shell
    /// integration says is running, or a full-screen app)
    fn busy(&self, id: PaneId) -> Option<String> {
        let p = self.panes.get(&id)?;
        if p.exited || p.player.is_some() || p.held {
            return None;
        }
        if let Some(c) = p.term.running_command() {
            let what = if c.text.is_empty() { "a command".to_string() } else { c.text.lines().next().unwrap_or("").to_string() };
            let took = c.started.map(|t| format!("  (running {})", pane::format_duration(t.elapsed()))).unwrap_or_default();
            return Some(format!("{what}{took}"));
        }
        p.term.is_alt_screen().then(|| format!("{} (full-screen)", p.title()))
    }

    /// "Update the editors?" with what changes.
    fn ask_update_editors(&mut self, found: bundle::Found) {
        let mut lines: Vec<String> =
            found.changes.iter().map(|(t, have, new)| format!("{}: {have} \u{2192} {new}", if t == "nvim" { "Neovim" } else { "nano" })).collect();
        lines.push(String::new());
        lines.push("From GitHub (Neovim) and MSYS2 (nano); every download is checked against its published SHA-256.".into());
        lines.push("Close nano and Neovim first.".into());
        self.ask("Update the editors?", lines, "Update", false, ConfirmAct::UpdateEditors(found));
    }

    fn ask(&mut self, title: &str, lines: Vec<String>, yes: &str, danger: bool, act: ConfirmAct) {
        self.menu = None;
        self.palette = None;
        self.confirm = Some(Confirm {
            view: vibex_render::ConfirmView { title: title.into(), lines, yes: yes.into(), no: "Cancel".into(), danger, yes_focused: !danger },
            act,
        });
        self.window.request_redraw();
    }

    fn request_close_tab(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        let busy: Vec<String> = tab.root.panes().into_iter().chain(tab.float).filter_map(|id| self.busy(id)).collect();
        if self.config.confirm_close && !busy.is_empty() {
            self.ask("Close this tab?", busy, "Close tab", true, ConfirmAct::CloseTab(i));
        } else {
            self.close_tab(i);
        }
    }

    fn request_close_pane(&mut self, id: PaneId) {
        match self.busy(id).filter(|_| self.config.confirm_close) {
            Some(b) => self.ask("Close this pane?", vec![b], "Close pane", true, ConfirmAct::ClosePane(id)),
            None => self.close_pane(id),
        }
    }

    fn request_close_window(&mut self) {
        let busy: Vec<String> = self.panes.keys().copied().collect::<Vec<_>>().into_iter().filter_map(|id| self.busy(id)).collect();
        if self.config.confirm_close && !busy.is_empty() {
            let title = if busy.len() == 1 {
                "Close the window? 1 thing is running".to_string()
            } else {
                format!("Close the window? {} things are running", busy.len())
            };
            self.ask(&title, busy, "Close window", true, ConfirmAct::CloseWindow);
        } else {
            self.close_window();
        }
    }

    /// Keep a closing tab (layout, folders, scrollback) for "Reopen".
    fn remember_closed(&mut self, i: usize) {
        let Some(t) = self.tabs.get(i) else { return };
        if t.root.panes().iter().any(|id| self.panes.get(id).is_none_or(|p| p.player.is_some())) {
            return;
        }
        let mut buffers = Vec::new();
        let root = self.saved_node(&t.root, Some(&mut buffers));
        let tab = session::SavedTab {
            root,
            active: t.root.panes().iter().position(|id| *id == t.active).unwrap_or(0),
            zoomed: t.zoomed,
            title: t.title.clone(),
            color: t.color,
        };
        self.closed.push(ClosedTab { tab, index: i, buffers });
        while self.closed.len() > 10 {
            let old = self.closed.remove(0);
            drop_buffers(&old.buffers);
        }
    }

    fn reopen_closed_tab(&mut self) {
        let Some(c) = self.closed.pop() else {
            self.toast("no closed tabs to reopen", 2.0);
            return;
        };
        let before = self.tabs.len();
        let saved = session::Saved { tabs: vec![c.tab], ..Default::default() };
        self.restore_tabs_with(&saved, true, "reopened");
        drop_buffers(&c.buffers);
        if self.tabs.len() > before {
            let t = self.tabs.remove(before);
            let at = c.index.min(self.tabs.len());
            self.tabs.insert(at, t);
            self.active_tab = at;
            self.after_focus_change();
            self.relayout();
            self.relayout_tabbar();
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.remember_closed(i);
        let tab = self.tabs.remove(i);
        for id in tab.root.panes().into_iter().chain(tab.float) {
            if let Some(p) = self.panes.remove(&id) {
                p.kill();
            }
        }
        if self.tabs.is_empty() {
            self.forget_session();
            self.quit = true;
            return;
        }
        if self.active_tab >= i && self.active_tab > 0 {
            self.active_tab -= 1;
        }
        self.after_focus_change();
    }

    /// Take tab `i` out of this window (shells still running), to move it
    /// elsewhere. The window closes if that was its last tab.
    fn take_tab(&mut self, i: usize) -> Option<(Tab, Vec<(PaneId, Pane)>)> {
        if i >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(i);
        let ids: Vec<PaneId> = tab.root.panes().into_iter().chain(tab.float).collect();
        let panes = ids.iter().filter_map(|id| self.panes.remove(id).map(|p| (*id, p))).collect();
        self.selecting_pane = None;
        self.quick = None;
        self.palette = None;
        self.menu = None;
        if self.tabs.is_empty() {
            // nothing left here; the shells live on in the other window
            self.quit = true;
            return Some((tab, panes));
        }
        if self.active_tab >= i && self.active_tab > 0 {
            self.active_tab -= 1;
        }
        self.session_dirty = true;
        self.after_focus_change();
        self.relayout();
        self.relayout_tabbar();
        Some((tab, panes))
    }

    fn move_tab(&mut self, i: usize, to: MoveTarget) {
        if let Some((tab, panes)) = self.take_tab(i) {
            self.tab_moves.push(TabMove { tab, panes, to });
        }
    }

    fn duplicate_tab(&mut self, i: usize) {
        self.activate_tab(i);
        let profile = self.active().profile.clone();
        let cwd = self.current_cwd();
        match self.new_tab_in(Some(&profile), cwd) {
            Ok(()) => {
                if let Some(t) = self.tabs.pop() {
                    let at = (i + 1).min(self.tabs.len());
                    self.tabs.insert(at, t);
                    self.active_tab = at;
                    self.after_focus_change();
                    self.relayout_tabbar();
                }
            }
            Err(e) => self.toast(e, 4.0),
        }
    }

    /// The tab right-click menu.
    fn open_tab_menu(&mut self, i: usize, x: f64, y: f64) {
        let hint = |a: &Action| self.config.bindings.iter().find(|b| &b.action == a).map(|b| b.describe()).unwrap_or_default();
        let it = |label: String, hint: String, sep: bool, cmd: TabCmd| (MenuItem { label, hint, separator: sep, icon: None }, MenuCmd::Tab(cmd, i));
        let mut items = vec![
            it("Rename tab".into(), hint(&Action::RenameTab), false, TabCmd::Rename),
            it("Tab colour…".into(), String::new(), false, TabCmd::Color),
            it("Duplicate tab".into(), String::new(), false, TabCmd::Duplicate),
        ];
        if self.tabs.len() > 1 {
            items.push(it("Move to a new window".into(), "drag it out".into(), true, TabCmd::NewWindow));
        }
        let me = u64::from(self.window.id());
        let others: Vec<(u64, String)> = self.workspaces.iter().filter(|(id, _, detached)| *id != me && !detached).map(|(id, n, _)| (*id, n.clone())).collect();
        for (k, (id, name)) in others.into_iter().enumerate() {
            items.push(it(format!("Move to window \"{name}\""), String::new(), k == 0 && self.tabs.len() <= 1, TabCmd::ToWindow(id)));
        }
        if self.tabs.len() > 1 {
            items.push(it("Close other tabs".into(), String::new(), true, TabCmd::CloseOthers));
            if i + 1 < self.tabs.len() {
                items.push(it("Close tabs to the right".into(), String::new(), false, TabCmd::CloseRight));
            }
        }
        items.push(it("Close tab".into(), hint(&Action::CloseTab), self.tabs.len() <= 1, TabCmd::Close));
        with_icons(&mut items);
        self.menu = Some(Menu { items, typing: false, first: 0, min_width: 0.0, selected: None, x: x as f32, y: y as f32 + 6.0 });
        self.palette = None;
        self.window.request_redraw();
    }

    fn run_tab_cmd(&mut self, cmd: TabCmd, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        match cmd {
            TabCmd::Rename => {
                self.activate_tab(i);
                self.perform(Action::RenameTab);
            }
            TabCmd::Color => {
                self.activate_tab(i);
                self.open_palette_with("tab colour");
            }
            TabCmd::Duplicate => self.duplicate_tab(i),
            TabCmd::NewWindow => {
                let pos = self.window.outer_position().ok().map(|p| PhysicalPosition::new(p.x + 40, p.y + 40));
                self.move_tab(i, MoveTarget::NewWindow(pos));
            }
            TabCmd::ToWindow(id) => self.move_tab(i, MoveTarget::Window(id, None)),
            TabCmd::CloseOthers => {
                for j in (0..self.tabs.len()).rev() {
                    if j != i {
                        self.close_tab(j);
                    }
                }
            }
            TabCmd::CloseRight => {
                for j in (i + 1..self.tabs.len()).rev() {
                    self.close_tab(j);
                }
            }
            TabCmd::Close => self.request_close_tab(i),
        }
        self.relayout_tabbar();
    }

    fn activate_tab(&mut self, i: usize) {
        self.settings_ui.close();
        if i < self.tabs.len() && i != self.active_tab {
            self.active_tab = i;
            self.after_focus_change();
            self.relayout_tabbar();
        }
    }

    fn focus_pane(&mut self, id: PaneId) {
        let tab = &mut self.tabs[self.active_tab];
        if tab.shown_float().is_some_and(|f| f != id) {
            // clicking the splits behind sends the floating pane away
            tab.float_shown = false;
            tab.active = id;
            self.after_focus_change();
            return;
        }
        if tab.root.contains(id) && tab.active != id {
            tab.active = id;
            self.after_focus_change();
        }
    }

    /// Re-layout, retitle and refresh after tabs/panes/focus change.
    fn after_focus_change(&mut self) {
        let ids = self.tabs[self.active_tab].root.panes();
        for id in ids {
            if let Some(p) = self.panes.get_mut(&id) {
                p.activity = false;
            }
        }
        self.relayout();
        self.update_window_title();
        self.next_status = Instant::now();
        self.window.request_redraw();
    }

    fn update_window_title(&self) {
        let title = self.active().title();
        if self.name.is_empty() {
            self.window.set_title(&format!("{title} — Vibex"));
        } else {
            self.window.set_title(&format!("{title} — {} — Vibex", self.name));
        }
    }

    fn tabbar_visible(&self) -> bool {
        if self.focus_mode || (self.fullscreen && self.tabs.len() <= 1) {
            return false;
        }
        if self.custom_chrome {
            return true;
        }
        match self.config.tab_bar {
            TabBarMode::Always => true,
            TabBarMode::Never => false,
            TabBarMode::Auto => self.tabs.len() > 1,
        }
    }

    fn tab_bar_model(&self) -> TabBar {
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let p = &self.panes[&t.active];
                let n = t.root.panes().len();
                let editing = self.rename.as_ref().filter(|(ti, _)| *ti == i).map(|(_, s)| s.clone());
                let rule = p.ssh_host.as_deref().and_then(|h| ssh::rule_for(h, &self.config.ssh_hosts));
                let base = match (&p.ssh_host, rule.and_then(|r| r.label.as_deref())) {
                    (Some(h), Some(label)) if !p.ssh_fixed || t.title.as_deref() == Some(h.as_str()) => format!("{label} · {h}"),
                    (Some(h), None) if !p.ssh_fixed && t.title.is_none() => h.clone(),
                    _ => t.title.clone().unwrap_or_else(|| p.title()),
                };
                let title = match &editing {
                    Some(s) => s.clone(),
                    None if n > 1 => format!("{base} ({n})"),
                    None => base,
                };
                let title = if t.broadcast && editing.is_none() { format!("⇶ {title}") } else { title };
                let recording = t.root.panes().iter().any(|id| self.panes[id].recorder.is_some());
                let title = if recording && editing.is_none() { format!("● {title}") } else { title };
                let alert = i != self.active_tab && t.root.panes().iter().any(|id| self.panes[id].activity);
                TabInfo {
                    title,
                    active: i == self.active_tab,
                    alert,
                    color: t.color.or(rule.and_then(|r| r.color)),
                    editing: editing.is_some(),
                    icon: TabIcon::Terminal,
                }
            })
            .collect::<Vec<_>>();
        TabBar {
            tabs,
            window_buttons: self.custom_chrome && !self.fullscreen,
            maximized: self.window.is_maximized(),
            hover: self.tab_hover,
            caption: self.eng.as_ref().map(|e| format!("\u{25cf} engagement: {}", e.name)),
        }
    }

    /// Lay out the active tab's panes and resize their grids.
    fn relayout(&mut self) {
        let size = self.window.inner_size();
        let (w, h) = (size.width as usize, size.height as usize);
        if w == 0 || h == 0 || self.tabs.is_empty() {
            return;
        }
        self.session_dirty = true;
        if !self.window.is_maximized() && !self.fullscreen {
            let scale = self.window.scale_factor();
            self.restored_size = Some((w as f64 / scale, h as f64 / scale));
        }
        let show_bar = self.tabbar_visible();
        self.tabbar = if show_bar {
            let model = self.tab_bar_model();
            self.renderer.tabbar_layout(&model, w)
        } else {
            TabBarLayout::default()
        };
        let content = self.renderer.content_rect(w, h, show_bar);
        let area = layout::Rect { x: content.x, y: content.y, w: content.w, h: content.h };
        let tab = &self.tabs[self.active_tab];
        let (rects, divs) = if tab.zoomed { (vec![(tab.active, area)], Vec::new()) } else { tab.root.layout(area) };
        self.dividers = divs;
        // images are sized in cells, so every terminal needs the cell size
        let cell = (self.renderer.fonts.cell_width as u32, self.renderer.fonts.cell_height as u32);
        for p in self.panes.values_mut() {
            p.term.set_cell_pixels(cell.0, cell.1);
        }
        let mut placed: Vec<(PaneId, PxRect)> = rects.into_iter().map(|(id, r)| (id, to_px(r))).collect();
        if let Some(f) = self.tabs[self.active_tab].float {
            placed.push((f, self.float_rect(area)));
        }
        let ui = self.renderer.ui_scale.max(0.5);
        for (id, rect) in placed {
            let mut rect = rect;
            if let Some(n) = self.panes.get_mut(&id).and_then(|p| p.notes.as_mut()) {
                let w = (rect.w * 0.34).clamp(220.0 * ui, 560.0 * ui).min(rect.w * 0.6).round();
                n.rect = PxRect { x: rect.x + rect.w - w, y: rect.y, w, h: rect.h };
                rect.w -= w;
            }
            let (cols, rows) = self.renderer.pane_grid(rect);
            if let Some(p) = self.panes.get_mut(&id) {
                p.rect = rect;
                p.set_grid(cols, rows);
            }
        }
    }

    /// Pane ids drawn in the active tab with their rects (a shown
    /// floating pane last, on top).
    fn visible_panes(&self) -> Vec<PaneId> {
        let tab = &self.tabs[self.active_tab];
        let mut v = if tab.zoomed { vec![tab.active] } else { tab.root.panes() };
        v.extend(tab.shown_float());
        v
    }

    fn pane_at(&self, x: f64, y: f64) -> Option<PaneId> {
        // topmost first: the floating pane covers the splits
        self.visible_panes().into_iter().rev().find(|id| self.panes[id].rect.contains(x as f32, y as f32))
    }

    /// Where a floating pane goes: centred, most of the content area.
    fn float_rect(&self, area: layout::Rect) -> PxRect {
        let w = (area.w * 0.72).max(200.0).min(area.w - 20.0);
        let h = (area.h * 0.72).max(120.0).min(area.h - 20.0);
        PxRect { x: (area.x + (area.w - w) / 2.0).round(), y: (area.y + (area.h - h) / 2.0).round(), w: w.round(), h: h.round() }
    }

    /// Switch to the tab holding `id` and focus it (floating panes shown).
    fn reveal_pane(&mut self, id: PaneId) {
        let Some(ti) = self.tabs.iter().position(|t| t.root.contains(id) || t.float == Some(id)) else { return };
        self.settings_ui.active = false;
        self.active_tab = ti;
        let tab = &mut self.tabs[ti];
        if tab.float == Some(id) {
            tab.float_shown = true;
        } else {
            tab.active = id;
            tab.float_shown = false;
        }
        self.after_focus_change();
    }

    /// Show/hide the floating pane, starting a shell in it the first time.
    fn toggle_float(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        if tab.float.is_some() {
            tab.float_shown = !tab.float_shown;
            self.after_focus_change();
            return;
        }
        let cwd = self.current_cwd();
        match self.spawn_pane(None, cwd) {
            Ok(id) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.float = Some(id);
                tab.float_shown = true;
                self.after_focus_change();
            }
            Err(e) => self.toast(e, 4.0),
        }
    }

    /// Run `cmd` in a floating pane that goes away when the program exits
    /// (like tmux's display-popup).
    fn popup(&mut self, cmd: &str) {
        let argv = split_command(cmd);
        let Some((program, args)) = argv.split_first() else { return };
        let mut c = config::base_command(program.clone());
        c.args = args.to_vec();
        let c = pane::with_cwd(c, self.current_cwd().as_deref());
        match self.spawn_command(program, c) {
            Ok(id) => {
                let old = self.tabs[self.active_tab].float.replace(id);
                self.tabs[self.active_tab].float_shown = true;
                if let Some(old) = old {
                    if let Some(p) = self.panes.remove(&old) {
                        p.kill();
                    }
                }
                self.after_focus_change();
            }
            Err(e) => self.toast(e, 4.0),
        }
    }

    fn drain_all(&mut self) {
        let active = self.active_id();
        let visible = self.visible_panes();
        let mut exited = Vec::new();
        let mut redraw = false;
        let ids: Vec<PaneId> = self.panes.keys().copied().collect();
        for id in ids {
            let d = self.panes.get_mut(&id).unwrap().drain();
            let shown = visible.contains(&id);
            redraw |= shown && d.changed;
            if d.changed && !shown {
                let p = self.panes.get_mut(&id).unwrap();
                // the tab bar's activity dot appears: one redraw
                redraw |= !p.activity;
                p.activity = true;
            }
            redraw |= d.title.is_some() || d.exited || d.bell || !d.notifications.is_empty();
            if let Some(title) = d.title {
                if id == active {
                    self.update_window_title();
                    self.emit("title-changed", EventArg::Str(title));
                    self.next_status = Instant::now();
                }
                if self.tabbar_visible() {
                    self.relayout_tabbar();
                }
            }
            if d.bell {
                self.emit("bell", EventArg::None);
            }
            for text in d.clipboard {
                if let Some(cb) = self.clipboard.as_mut() {
                    let _ = cb.set_text(text);
                }
            }
            if d.clipboard_reads > 0 {
                let text = if self.config.clipboard_read {
                    self.clipboard.as_mut().and_then(|cb| cb.get_text().ok()).unwrap_or_default()
                } else {
                    String::new() // refused: answer empty so the program doesn't hang
                };
                let p = self.panes.get_mut(&id).unwrap();
                for _ in 0..d.clipboard_reads.min(4) {
                    p.term.reply_clipboard(&text);
                }
                let replies = p.term.take_responses();
                p.write(&replies);
            }
            if self.config.notifications {
                for (title, body) in d.notifications {
                    self.notify(&title, &body);
                }
            }
            // long commands finishing while you weren't looking
            let finished = self.panes.get_mut(&id).map(|p| p.term.take_finished()).unwrap_or_default();
            let threshold = self.config.notify_long_commands;
            if self.config.command_history {
                let cwd = self.panes.get(&id).and_then(|p| p.term.cwd()).unwrap_or("").to_string();
                for f in finished.iter().filter(|f| history::worth_keeping(&f.text)) {
                    let text = if self.config.secret_guard { self.config.secrets.redact(&f.text) } else { f.text.clone() };
                    if let Some(list) = self.suggest.as_mut() {
                        let cmd = text.trim_end().to_string();
                        list.retain(|(c, _)| *c != cmd);
                        list.insert(0, (cmd, cwd.clone()));
                    }
                    history::append(&history::Record {
                        when: history::now(),
                        exit: f.exit,
                        duration: f.duration,
                        cwd: cwd.clone(),
                        command: text.trim_end().to_string(),
                    });
                }
            }
            if (self.config.audit_log || self.eng.is_some()) && !finished.is_empty() {
                let log = self.eng.as_ref().map(|e| e.audit_path()).or_else(audit::path);
                let keep_output = self.config.audit_output || self.eng.is_some();
                let p = &self.panes[&id];
                let output = p.term.last_finished_block().map(|b| p.term.block_output(&b)).unwrap_or_default();
                let output = if self.config.secret_guard { self.config.secrets.redact(&output) } else { output };
                let (machine, user) = audit::who();
                let mut failed = None;
                for f in &finished {
                    let command = if self.config.secret_guard { self.config.secrets.redact(&f.text) } else { f.text.clone() };
                    let e = audit::Entry {
                        time: history::now(),
                        machine: machine.clone(),
                        user: user.clone(),
                        workspace: self.name.clone(),
                        pane: id,
                        ssh: p.ssh_host.clone(),
                        cwd: p.term.cwd().unwrap_or("").to_string(),
                        command: command.trim_end().to_string(),
                        exit: f.exit,
                        duration_ms: f.duration.map(|d| d.as_millis() as u64),
                        output_lines: output.lines().count(),
                        output_sha256: audit::output_digest(&output),
                        output: keep_output.then(|| output.clone()),
                    };
                    let res = match &log {
                        Some(path) => audit::append_to(path, &e),
                        None => Err("no config folder".to_string()),
                    };
                    if let Err(err) = res {
                        failed = Some(err);
                    }
                }
                if let Some(err) = failed {
                    self.toast(format!("audit log: {err}"), 4.0);
                }
            }
            for f in finished {
                let long = threshold > 0.0 && f.duration.is_some_and(|d| d.as_secs_f32() >= threshold);
                let watching = self.focused && shown;
                if long && !watching {
                    let took = f.duration.map(pane::format_duration).unwrap_or_default();
                    let title = match f.exit {
                        Some(0) => "✓ Finished".to_string(),
                        Some(c) => format!("✗ Failed (exit {c})"),
                        None => "Finished".to_string(),
                    };
                    let what = if f.text.is_empty() { "command".to_string() } else { f.text.clone() };
                    self.notify(&title, &format!("{what} · {took}"));
                }
            }
            if d.changed && !self.config.triggers.is_empty() {
                self.run_triggers(id);
            }
            if d.changed {
                self.engagement_pane(id);
            }
            // a typed `ssh host` (shell integration reports the command)
            if let Some(p) = self.panes.get_mut(&id) {
                if !p.ssh_fixed {
                    let now = p.term.running_command().and_then(|c| ssh::host_in_command(&c.text));
                    if now != p.ssh_host {
                        if let Some(h) = &now {
                            ssh::remember(h);
                        }
                        p.ssh_host = now;
                        self.relayout_tabbar();
                        self.window.request_redraw();
                    }
                }
            }
            if d.changed {
                if let Some(p) = self.panes.get_mut(&id) {
                    p.last_output = Instant::now();
                    if p.monitor_silence {
                        p.output_since_quiet = true;
                    }
                }
            }
            if d.exited {
                exited.push(id);
            }
        }
        for id in exited {
            self.close_pane(id);
        }
        if !self.quit && self.tabs.iter().any(|t| t.root.panes().iter().any(|id| self.panes[id].activity)) {
            self.relayout_tabbar();
        }
        // output that only answered queries (cursor reports etc.) draws nothing
        if redraw {
            self.window.request_redraw();
        }
    }

    fn relayout_tabbar(&mut self) {
        let w = self.window.inner_size().width as usize;
        if self.tabbar_visible() {
            let model = self.tab_bar_model();
            self.tabbar = self.renderer.tabbar_layout(&model, w);
        }
    }

    fn write_active(&mut self, bytes: &[u8]) {
        let tab = &self.tabs[self.active_tab];
        if tab.broadcast {
            for id in tab.root.panes() {
                if let Some(p) = self.panes.get_mut(&id).filter(|p| !p.exited) {
                    p.term.scroll_view_to_bottom();
                    p.write(bytes);
                }
            }
            return;
        }
        let p = self.active_mut();
        p.term.scroll_view_to_bottom();
        p.write(bytes);
    }

    fn reset_blink(&mut self) {
        self.blink_on = true;
        self.next_blink = Instant::now() + self.blink_interval;
    }

    fn set_font_px(&mut self, px: f32) {
        self.renderer.fonts.set_size(px);
        self.relayout();
        self.window.request_redraw();
    }

    // ------------------------------------------------------------ drawing

    /// Look for a newer release in the background.
    fn check_updates(&mut self, manual: bool) {
        let url = self.config.update_feed.clone();
        if url.is_empty() {
            if manual {
                self.toast("updates aren't set up: set config.update_feed in vibex.lua", 4.0);
            }
            return;
        }
        if manual {
            self.toast("checking for updates…", 10.0);
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = proxy.send_event(AppEvent::Update(update::check(&url), manual));
        });
    }

    /// Look for newer nano / Neovim in the background.
    fn check_editors(&mut self, manual: bool) {
        if bundle::tools_dir().is_none() {
            if manual {
                self.toast("nano and Neovim aren't bundled with this Vibex (install it with the setup)", 4.0);
            }
            return;
        }
        if manual {
            self.toast("looking for new nano and Neovim releases…", 10.0);
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = proxy.send_event(AppEvent::Editors(bundle::check(), manual));
        });
    }

    /// Refresh what screen readers see (no-op unless one is listening).
    fn update_access(&mut self) {
        let Some(mut adapter) = self.access.take() else { return };
        if self.settings_ui.active {
            let about = self.about_info();
            let cfg = self.config.clone();
            let st = self.settings.clone();
            let view = self.settings_ui.view(&cfg, &st, &about);
            let lines = settings_ui::SettingsUi::spoken_rows(&view);
            let title = format!("Settings: {}", view.title);
            adapter.update_if_active(|| access::list_tree(&title, &lines, view.focus));
            self.access = Some(adapter);
            return;
        }
        adapter.update_if_active(|| {
            let active = self.active_id();
            let (px, py) = (self.renderer.padding_x as f64, self.renderer.padding_y as f64);
            let views: Vec<access::PaneView> = self
                .visible_panes()
                .into_iter()
                .filter_map(|id| self.panes.get(&id).map(|p| (id, p)))
                .map(|(id, p)| access::PaneView {
                    id: id,
                    term: &p.term,
                    title: p.title(),
                    origin: (p.rect.x as f64 + px, p.rect.y as f64 + py),
                    active: id == active,
                })
                .collect();
            let cell = (self.renderer.fonts.cell_width as f64, self.renderer.fonts.cell_height as f64);
            access::tree(&format!("Vibex: {}", self.active().title()), &views, cell)
        });
        self.access = Some(adapter);
    }

    /// Screen reader actions: focusing a pane moves keyboard focus there.
    fn access_action(&mut self, req: accesskit::ActionRequest) {
        if req.action == accesskit::Action::Focus {
            let pane = (req.target_node.0 >> 20) as PaneId;
            if self.panes.contains_key(&pane) && self.tabs[self.active_tab].root.panes().contains(&pane) {
                self.tabs[self.active_tab].active = pane;
                self.after_focus_change();
                self.window.request_redraw();
            }
        }
    }

    fn redraw(&mut self) {
        self.update_access();
        let size = self.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        let (wu, hu) = (w.get() as usize, h.get() as usize);
        let active = self.active_id();
        let suggestion = self.suggestion();
        let visible = self.visible_panes();
        let multi = visible.len() > 1;
        let float_id = self.tabs[self.active_tab].shown_float();

        let mut scene = self.renderer.begin(wu, hu);
        for id in &visible {
            let theme = self.renderer.theme.clone();
            let Some(p) = self.panes.get_mut(id) else { continue };
            p.refresh_search();
            p.animate_scroll();
            let is_active = *id == active;
            let dim = multi && !is_active && (self.config.dim_inactive_panes || self.focus_mode);
            let mut state: DrawState = p.draw_state(&theme, self.focused && is_active, self.blink_on, dim, self.config.shell_marks);
            if let Some(q) = self.quick.as_ref().filter(|q| q.pane == *id) {
                let top = p.term.top_visible_line();
                for (label, t) in &q.targets {
                    if !label.starts_with(&q.typed) || t.line < top || t.line >= top + p.term.rows() {
                        continue;
                    }
                    let row = t.line - top;
                    state.highlights.push(vibex_render::Highlight {
                        range: vibex_render::VisibleSelection { start: (t.start, row), end: (t.end, row), block: false },
                        bg: theme.search_match,
                        fg: None,
                    });
                    state.labels.push((t.start, row, label[q.typed.len()..].to_string()));
                }
            }
            if self.config.secret_guard && !self.reveal_secrets {
                let p = &self.panes[id];
                let top = p.term.top_visible_line();
                let mask = vibex_render::blend(theme.background, theme.foreground, 70);
                for m in p.term.find_regex_wrapped(&self.config.secrets.combined, top..top + p.term.rows()).into_iter().take(200) {
                    let row = m.line - top;
                    state.highlights.push(vibex_render::Highlight {
                        range: vibex_render::VisibleSelection { start: (m.start, row), end: (m.end, row), block: false },
                        bg: mask,
                        fg: Some(mask),
                    });
                }
            }
            if let Some(h) = self.mark_highlight(*id, &theme) {
                state.highlights.push(h);
            }
            if is_active {
                state.preedit = self.preedit.clone();
                state.ghost = suggestion.clone();
            }
            let p = &self.panes[id];
            if Some(*id) == float_id {
                // dividers belong under the floating pane
                let divs: Vec<PxRect> = self.dividers.iter().map(|d| to_px(d.rect)).collect();
                self.renderer.draw_dividers(&mut scene, &divs);
                self.renderer.draw_float_frame(&mut scene, p.rect, is_active);
            }
            let rule = p.ssh_host.as_deref().and_then(|h| ssh::rule_for(h, &self.config.ssh_hosts));
            match rule.and_then(|r| r.theme.clone()) {
                Some(t) => {
                    // the host's own colours, on its own background
                    scene_fill(&mut scene, p.rect, t.background, self.opacity);
                    let saved = std::mem::replace(&mut self.renderer.theme, t);
                    self.renderer.draw_pane(&mut scene, &p.term, &state, p.rect);
                    self.renderer.theme = saved;
                }
                None => self.renderer.draw_pane(&mut scene, &p.term, &state, p.rect),
            }
            if let Some(c) = rule.and_then(|r| r.color) {
                self.renderer.draw_pane_border(&mut scene, p.rect, c);
            }
            if let Some(n) = p.notes.as_ref() {
                let cw = self.renderer.fonts.cell_width.max(1) as f32;
                let ch = self.renderer.fonts.cell_height.max(1) as f32;
                let s = self.renderer.ui_scale.max(0.5);
                let cols = ((n.rect.w - 24.0 * s) / cw).floor().max(4.0) as usize;
                let rows = ((n.rect.h - 38.0 * s) / ch).floor().max(1.0) as usize;
                let (lines, caret) = n.wrap(cols);
                let scroll = (caret.0 + 1).saturating_sub(rows);
                let (title, rect, focused) = (n.title.clone(), n.rect, n.focused && is_active);
                self.renderer.draw_notes(&mut scene, rect, &title, &lines, scroll, Some(caret), focused);
            }
            if is_active {
                // keep the IME candidate window next to the cursor
                let r = &self.renderer;
                let (cw, ch) = (r.fonts.cell_width as f64, r.fonts.cell_height as f64);
                let cx = p.rect.x as f64 + r.padding_x as f64 + p.term.cursor.x as f64 * cw;
                let cy = p.rect.y as f64 + r.padding_y as f64 + (p.term.cursor.y + p.term.display_offset()) as f64 * ch;
                self.window.set_ime_cursor_area(PhysicalPosition::new(cx, cy), winit::dpi::PhysicalSize::new(cw, ch));
            }
        }

        let mut chrome = Chrome::default();
        if self.tabbar_visible() {
            chrome.tabbar = Some((self.tab_bar_model(), self.tabbar.clone()));
        }
        if float_id.is_none() {
            chrome.dividers = self.dividers.iter().map(|d| to_px(d.rect)).collect();
        }
        if self.settings_ui.active {
            let about = self.about_info();
            self.settings_ui.scope_text = self.eng.as_ref().map(|e| e.scope.text.join(", ")).unwrap_or_default();
            self.settings_ui.loot_summary = self.eng.as_ref().map(|e| loot_summary(&e.loot_path())).unwrap_or_default();
            let view = self.settings_ui.view(&self.config, &self.settings, &about);
            let content = self.renderer.content_rect(wu, hu, self.tabbar_visible());
            let rect = self.settings_dialog_rect(content);
            self.settings_rect = rect;
            chrome.settings = Some((view, rect, content));
        }
        if let Some(m) = &self.menu {
            chrome.menu = Some(MenuView {
                items: m.items.iter().map(|(i, _)| i.clone()).collect(),
                x: m.x,
                y: m.y,
                selected: m.selected,
                first: m.first,
                min_width: m.min_width,
            });
        }
        chrome.status = if self.focus_mode { None } else { self.status.clone() };
        chrome.confirm = self.confirm.as_ref().map(|c| c.view.clone());
        if self.tabs[self.active_tab].broadcast {
            let n = self.tabs[self.active_tab].root.panes().len();
            chrome
                .overlays
                .push(Overlay { text: format!("BROADCAST · typing goes to all {n} panes · Ctrl+Shift+B to stop"), border: self.renderer.theme.palette[1] });
        }
        if !self.settings_ui.active {
            if let Some(cm) = &self.active().copy {
                chrome.overlays.push(Overlay { text: cm.label(), border: self.renderer.theme.palette[3] });
            }
            if let Some(label) = self.active().search_label() {
                chrome.overlays.push(Overlay { text: label, border: self.renderer.theme.cursor });
            }
        }
        if let Some(err) = &self.config_error {
            chrome
                .overlays
                .push(Overlay { text: format!("vibex.lua: {err}\nfix it and save; Vibex reloads automatically"), border: self.renderer.theme.palette[9] });
        }
        if let Some((msg, until)) = &self.toast {
            if Instant::now() < *until {
                chrome.overlays.push(Overlay { text: msg.clone(), border: self.renderer.theme.cursor });
            }
        }
        chrome.palette = self.palette.as_ref().map(|p| p.view());
        self.chrome_layout = self.renderer.draw_chrome(&mut scene, &chrome);
        if let (Some(m), Some(l)) = (self.menu.as_mut(), self.chrome_layout.menu.as_ref()) {
            m.first = l.first;
        }
        if self.settings_ui.active {
            if let Some(l) = &self.chrome_layout.settings {
                // keep scrolling in range and the focused row on screen
                let mut scroll = self.settings_ui.scroll.min(l.max_scroll());
                if let Some(f) = self.settings_ui.focus {
                    let content = self.settings_rect;
                    if let Some((r, _)) = l.regions.iter().find(|(_, h)| *h == SettingsHit::Row(f)) {
                        let margin = 16.0;
                        if r.y < content.y + margin {
                            scroll -= content.y + margin - r.y;
                        } else if r.y + r.h > content.y + content.h - margin {
                            scroll += r.y + r.h - (content.y + content.h - margin);
                        }
                    }
                }
                let scroll = scroll.clamp(0.0, l.max_scroll().max(scroll.min(0.0)));
                if (scroll - self.settings_ui.scroll).abs() > 0.5 {
                    self.settings_ui.scroll = scroll.max(0.0);
                    self.window.request_redraw();
                }
            }
        }

        match &mut self.backend {
            Backend::Gpu(gpu) => {
                gpu.resize(w.get(), h.get());
                let o = if self.config.glass { glass_opacity(self.opacity) } else { acrylic_opacity(self.opacity) };
                gpu.render(&scene, &mut self.renderer.fonts, o);
            }
            Backend::Cpu { surface, last, scratch, .. } => {
                if surface.resize(w, h).is_err() {
                    return;
                }
                // draw off-screen, then hand Windows only the rows that
                // changed since the last frame
                scratch.resize(wu * hu, 0);
                let mut frame = Frame { pixels: scratch, width: wu, height: hu };
                vibex_render::cpu::draw(&scene, &mut self.renderer.fonts, &mut frame);
                let Ok(mut buffer) = surface.buffer_mut() else { return };
                let reusable = buffer.age() == 1 && last.len() == wu * hu;
                if reusable {
                    let bands = changed_bands(last, scratch, wu, hu);
                    if bands.is_empty() {
                        return;
                    }
                    for r in &bands {
                        let (a, b) = (r.y as usize * wu, (r.y + r.height.get()) as usize * wu);
                        buffer[a..b].copy_from_slice(&scratch[a..b]);
                    }
                    let _ = buffer.present_with_damage(&bands);
                } else {
                    buffer.copy_from_slice(scratch);
                    let _ = buffer.present();
                }
                std::mem::swap(last, scratch);
            }
        }
    }

    // -------------------------------------------------------------- mouse

    fn cell_in(&self, id: PaneId) -> (usize, usize) {
        let r = &self.renderer;
        self.panes[&id].cell_at(self.mouse.x, self.mouse.y, r.fonts.cell_width, r.fonts.cell_height, (r.padding_x, r.padding_y))
    }

    /// Absolute (col, line) under the mouse in pane `id`.
    fn abs_cell_in(&self, id: PaneId) -> (usize, usize) {
        let (x, y) = self.cell_in(id);
        (x, self.panes[&id].term.top_visible_line() + y)
    }

    /// Mouse events go to the application (unless Shift forces selection).
    fn reporting(&self) -> bool {
        self.active().term.modes.mouse_report != MouseMode::Off && !self.mods.shift_key()
    }

    fn report(&mut self, button: u8, pressed: bool, motion: bool) {
        let id = self.active_id();
        let (x, y) = self.cell_in(id);
        let mods = self.mods & !ModifiersState::SHIFT;
        let p = self.active_mut();
        if let Some(bytes) = mouse::encode(button, pressed, motion, mods, x, y, p.term.modes.mouse_sgr) {
            p.write(&bytes);
        }
        p.last_report_cell = Some((x, y));
    }

    fn edge_at(&self, x: f64, y: f64) -> Option<ResizeDirection> {
        if !self.custom_chrome || self.window.is_maximized() || self.fullscreen {
            return None;
        }
        let s = self.window.inner_size();
        let (w, h) = (s.width as f64, s.height as f64);
        let (l, r, t, b) = (x < EDGE, x >= w - EDGE, y < EDGE, y >= h - EDGE);
        Some(match (l, r, t, b) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (_, _, _, true) => ResizeDirection::South,
            _ => return None,
        })
    }

    fn divider_at(&self, x: f64, y: f64) -> Option<Divider> {
        // a few px of grab slack around the 2px line
        self.dividers.iter().copied().find(|d| {
            let r = d.rect;
            let grow = 3.0;
            x as f32 >= r.x - grow && (x as f32) < r.x + r.w + grow && y as f32 >= r.y - grow && (y as f32) < r.y + r.h + grow
        })
    }

    fn set_cursor_for(&self, x: f64, y: f64) {
        let icon = if let Some(e) = self.edge_at(x, y) {
            match e {
                ResizeDirection::East | ResizeDirection::West => CursorIcon::EwResize,
                ResizeDirection::North | ResizeDirection::South => CursorIcon::NsResize,
                ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::NeswResize,
                ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::NwseResize,
            }
        } else if self.dragging.is_some() || self.divider_at(x, y).is_some() {
            match self.dragging.or_else(|| self.divider_at(x, y)).map(|d| d.dir) {
                Some(SplitDir::Horizontal) => CursorIcon::ColResize,
                _ => CursorIcon::RowResize,
            }
        } else if self.tabbar.hit(x as f32, y as f32).is_some() {
            CursorIcon::Default
        } else if self.settings_ui.active || self.menu.is_some() {
            CursorIcon::Default
        } else if self.active().hover_link.is_some() {
            CursorIcon::Pointer
        } else {
            CursorIcon::Text
        };
        self.window.set_cursor(icon);
    }

    fn update_hover(&mut self) {
        let id = self.active_id();
        let link = if self.mods.control_key() && self.pane_at(self.mouse.x, self.mouse.y) == Some(id) {
            let (x, y) = self.abs_cell_in(id);
            let t = &self.active().term;
            t.url_at(x, y).or_else(|| {
                t.file_ref_at(x, y).map(|f| vibex_term::Link {
                    url: format!("{FILE_REF}{}\u{1f}{}\u{1f}{}", f.path, f.line.unwrap_or(0), f.col.unwrap_or(0)),
                    start: f.start,
                    end: f.end,
                })
            })
        } else {
            None
        };
        if link != self.active().hover_link {
            self.active_mut().hover_link = link;
            self.window.request_redraw();
        }
    }

    fn mouse_moved(&mut self, pos: PhysicalPosition<f64>) {
        self.mouse = pos;
        let (x, y) = (pos.x, pos.y);
        if let Some(mut drag) = self.tab_drag {
            if (x - drag.start_x).abs() > 6.0 {
                drag.moved = true;
            }
            // pulled well above or below the bar: it's leaving the window
            let bar_h = self.tabbar.height as f64;
            drag.out = y < -bar_h * 0.75 || y > bar_h * 1.75 || x < -24.0 || x > self.window.inner_size().width as f64 + 24.0;
            self.window.set_cursor(if drag.out { CursorIcon::Grabbing } else { CursorIcon::Default });
            if drag.moved && !drag.out {
                // move the tab to wherever the pointer is along the bar
                let bar_y = self.tabbar.height / 2.0;
                if let Some(TabHit::Tab(j) | TabHit::CloseTab(j)) = self.tabbar.hit(x as f32, bar_y) {
                    if j != drag.index && j < self.tabs.len() {
                        let tab = self.tabs.remove(drag.index);
                        self.tabs.insert(j, tab);
                        self.active_tab = j;
                        drag.index = j;
                        self.session_dirty = true;
                        self.relayout_tabbar();
                    }
                }
                self.window.request_redraw();
            }
            self.tab_drag = Some(drag);
            return;
        }
        if let Some((i, rect)) = self.settings_drag {
            let knob = 8.0 * self.renderer.ui_scale;
            let f = (x as f32 - rect.x - knob) / (rect.w - 2.0 * knob).max(1.0);
            let e = self.settings_ui.slide(i, f);
            if let settings_ui::Effect::Change(ch) = &e {
                // only save when the value actually moves
                let mut probe = self.config.clone();
                let mut st = self.settings.clone();
                settings::change(&mut st, &mut probe, ch);
                if probe.opacity != self.config.opacity || probe.font_size_pt != self.config.font_size_pt {
                    self.settings_effect(e);
                }
            }
            return;
        }
        if let Some(m) = self.menu.as_mut() {
            let sel = self.chrome_layout.menu.as_ref().and_then(|l| l.hit(x as f32, y as f32));
            if sel.is_some() && sel != m.selected {
                m.selected = sel;
                self.window.request_redraw();
            }
        }
        if self.settings_ui.active {
            let hover = self.chrome_layout.settings.as_ref().and_then(|l| l.hit(x as f32, y as f32)).map(|(h, _)| h);
            if hover != self.settings_ui.hover {
                self.settings_ui.hover = hover;
                self.window.request_redraw();
            }
        }
        if let Some(d) = self.dragging {
            self.tabs[self.active_tab].root.drag_divider(&d, x as f32, y as f32);
            self.relayout();
            // keep dragging the same split (its rect moved)
            self.dragging = self.dividers.iter().copied().find(|n| n.path == d.path && n.depth == d.depth);
            self.window.request_redraw();
            return;
        }
        let hover = self.tabbar.hit(x as f32, y as f32);
        if hover != self.tab_hover {
            self.tab_hover = hover;
            self.window.request_redraw();
        }
        self.edge = self.edge_at(x, y);
        self.update_hover();
        self.set_cursor_for(x, y);
        if hover.is_some() || self.edge.is_some() || self.settings_ui.active {
            return;
        }

        if self.reporting() && self.pane_at(x, y) == Some(self.active_id()) {
            let cell = self.cell_in(self.active_id());
            let p = self.active();
            if Some(cell) == p.last_report_cell {
                return;
            }
            match (p.term.modes.mouse_report, p.buttons_down) {
                (MouseMode::Motion, b) => self.report(b.unwrap_or(mouse::NONE), true, true),
                (MouseMode::Drag, Some(b)) => self.report(b, true, true),
                _ => {}
            }
            return;
        }
        if let Some(id) = self.selecting_pane {
            if self.panes.contains_key(&id) {
                let cell = self.abs_cell_in(id);
                if let Some(sel) = self.panes.get_mut(&id).and_then(|p| p.selection.as_mut()) {
                    sel.head = cell;
                }
                self.window.request_redraw();
            }
        }
    }

    fn tabbar_click(&mut self, hit: TabHit, button: MouseButton) {
        // the Settings tab sits after the terminal tabs
        if let TabHit::Tab(i) | TabHit::CloseTab(i) = hit {
            if i >= self.tabs.len() {
                if matches!(hit, TabHit::CloseTab(_)) || button == MouseButton::Middle {
                    self.settings_ui.close();
                    self.update_window_title();
                } else {
                    self.settings_ui.active = true;
                    self.window.set_title("Settings — Vibex");
                }
                self.relayout_tabbar();
                self.window.request_redraw();
                return;
            }
        }
        match (hit, button) {
            (TabHit::Tab(i), MouseButton::Middle) | (TabHit::CloseTab(i), _) => self.request_close_tab(i),
            (TabHit::Tab(i), MouseButton::Left) => {
                // double-click renames; press-and-drag reorders
                let now = Instant::now();
                let double = self.last_tab_click.is_some_and(|(t, j)| j == i && now - t < self.multi_click);
                self.last_tab_click = Some((now, i));
                self.activate_tab(i);
                if double {
                    self.last_tab_click = None;
                    self.perform(Action::RenameTab);
                } else {
                    self.tab_drag = Some(TabDrag { index: i, start_x: self.mouse.x, moved: false, out: false });
                }
            }
            (TabHit::Tab(i), MouseButton::Right) => {
                self.activate_tab(i);
                self.open_tab_menu(i, self.mouse.x, self.mouse.y);
            }
            (TabHit::Tab(i), _) => self.activate_tab(i),
            (TabHit::NewTab, MouseButton::Left) => {
                if let Err(e) = self.new_tab(None) {
                    self.toast(e, 4.0);
                }
            }
            (TabHit::NewTab, MouseButton::Right) | (TabHit::NewTabMenu, _) => self.open_menu(),
            (TabHit::Minimize, MouseButton::Left) => self.window.set_minimized(true),
            (TabHit::Maximize, MouseButton::Left) => self.toggle_maximize(),
            (TabHit::Close, MouseButton::Left) => self.request_close_window(),
            (TabHit::Drag, MouseButton::Left) => {
                let now = Instant::now();
                let double = self.last_bar_click.is_some_and(|t| now - t < self.multi_click);
                self.last_bar_click = Some(now);
                if double {
                    self.last_bar_click = None;
                    self.toggle_maximize();
                } else {
                    let _ = self.window.drag_window();
                }
            }
            _ => {}
        }
        self.window.request_redraw();
    }

    /// The ⌄ menu: profiles, then Settings, the palette and About.
    fn open_menu(&mut self) {
        let Some(r) = self.tabbar.rect(TabHit::NewTabMenu) else { return };
        let hint = |a: &Action| self.config.bindings.iter().find(|b| &b.action == a).map(|b| b.describe()).unwrap_or_default();
        let mut items: Vec<(MenuItem, MenuCmd)> = Vec::new();
        for p in &self.config.profiles {
            let a = Action::NewTab(Some(p.name.clone()));
            let mut h = hint(&a);
            if h.is_empty() && p.name == self.config.default_profile {
                h = hint(&Action::NewTab(None));
            }
            items.push((MenuItem { label: p.name.clone(), hint: h, separator: false, icon: Some(MenuIcon::Terminal) }, MenuCmd::Act(a)));
        }
        for (i, (label, a, icon)) in [
            ("Settings", Action::OpenSettings(None), Some(MenuIcon::Settings)),
            ("Command palette", Action::CommandPalette, None),
            ("About Vibex", Action::OpenSettings(Some("about".into())), None),
        ]
        .into_iter()
        .enumerate()
        {
            items.push((MenuItem { label: label.into(), hint: hint(&a), separator: i == 0, icon }, MenuCmd::Act(a)));
        }
        with_icons(&mut items);
        self.menu = Some(Menu { items, typing: false, first: 0, min_width: 0.0, selected: None, x: r.x, y: r.y + r.h + 4.0 });
        self.palette = None;
        self.window.request_redraw();
    }

    /// The right-click menu over a pane: the link under the pointer (if
    /// any), clipboard, splits and tabs, the last command, then Settings.
    /// Each item shows its shortcut.
    fn open_context_menu(&mut self, x: f64, y: f64) {
        let hint = |a: &Action| self.config.bindings.iter().find(|b| &b.action == a).map(|b| b.describe()).unwrap_or_default();
        let item = |label: &str, a: Action, sep: bool| (MenuItem { label: label.into(), hint: hint(&a), separator: sep, icon: None }, MenuCmd::Act(a));
        let mut items: Vec<(MenuItem, MenuCmd)> = Vec::new();
        let id = self.active_id();
        let (cx, cy) = self.abs_cell_in(id);
        let t = &self.active().term;
        if let Some(l) = t.url_at(cx, cy) {
            let plain = |label: &str, cmd| (MenuItem { label: label.into(), hint: "Ctrl+click".into(), separator: false, icon: None }, cmd);
            items.push(plain("Open link", MenuCmd::OpenLink(l.url.clone())));
            items.push((MenuItem { hint: String::new(), ..plain("Copy link", MenuCmd::Copy(l.url.clone())).0 }, MenuCmd::Copy(l.url)));
        } else if let Some(f) = t.file_ref_at(cx, cy) {
            let url = format!("{FILE_REF}{}\u{1f}{}\u{1f}{}", f.path, f.line.unwrap_or(0), f.col.unwrap_or(0));
            let label = match f.line {
                Some(n) => format!("Open {} at line {n}", short_path(&f.path)),
                None => format!("Open {}", short_path(&f.path)),
            };
            items.push((MenuItem { label, hint: "Ctrl+click".into(), separator: false, icon: None }, MenuCmd::OpenLink(url)));
            items.push((MenuItem { label: "Copy path".into(), hint: String::new(), separator: false, icon: None }, MenuCmd::Copy(f.path)));
        }
        let after_link = !items.is_empty();
        let has_sel = !self.active().selection_text().is_empty();
        if has_sel {
            items.push(item("Copy", Action::Copy, after_link));
        }
        items.push(item("Paste", Action::Paste, after_link && !has_sel));
        items.push(item("Select all", Action::SelectAll, false));
        items.push(item("Find", Action::Search, false));
        items.push(item("Split right", Action::SplitRight(None), true));
        items.push(item("Split down", Action::SplitDown(None), false));
        items.push(item("New tab", Action::NewTab(None), false));
        items.push(item("Floating pane", Action::ToggleFloatingPane, false));
        if self.active().term.last_finished_block().is_some() {
            items.push(item("Copy the last output", Action::CopyLastOutput, true));
            items.push(item("Run the last command again", Action::RerunLastCommand, false));
        }
        // pictures and saving lines
        let shot = |label: &str, cmd: MenuCmd, hint: String, sep: bool| (MenuItem { label: label.into(), hint, separator: sep, icon: None }, cmd);
        if has_sel {
            items.push(item("Copy as image", Action::CopyImage, true));
        }
        let block = self.active().term.block_at(cy).filter(|b| b.command.exit.is_some() || b.command.duration.is_some());
        let block_job = block.and_then(|_| self.job_block(id, Some(cy)));
        let first = !has_sel && block_job.is_none();
        if let Some(job) = block_job {
            items.push(shot("Screenshot this command", MenuCmd::Shot(job), String::new(), !has_sel));
        }
        items.push(item("Screenshot the pane", Action::ScreenshotPane, first));
        match self.mark_now() {
            Some((p, start)) if p == id => {
                let n = start.max(cy) + 1 - start.min(cy);
                if let Some(job) = self.job_lines(id, start, cy) {
                    items.push(shot(&format!("Mark end here: save {n} lines\u{2026}"), MenuCmd::ExportPick(job), String::new(), false));
                }
                items.push(shot("Clear the mark", MenuCmd::ClearMark, String::new(), false));
            }
            _ => {
                items.push(shot("Mark start here", MenuCmd::MarkAt(cy), export_ui::hint_for(&self.config, &Action::Mark), false));
            }
        }
        items.push(item("Save lines as\u{2026}", Action::SaveRange, false));
        items.push(item("Clear scrollback", Action::ClearScrollback, true));
        items.push(item("Command palette", Action::CommandPalette, false));
        let mut settings = item("Settings", Action::OpenSettings(None), false);
        settings.0.icon = Some(MenuIcon::Settings);
        items.push(settings);
        with_icons(&mut items);
        self.menu = Some(Menu { items, typing: false, first: 0, min_width: 0.0, selected: None, x: x as f32 + 2.0, y: y as f32 + 2.0 });
        self.palette = None;
        self.window.request_redraw();
    }

    fn run_menu_item(&mut self, i: usize) {
        if let Some(m) = self.menu.take() {
            match m.items.get(i).map(|(_, c)| c.clone()) {
                Some(MenuCmd::Act(a)) => self.perform(a),
                Some(MenuCmd::OpenLink(url)) => match url.strip_prefix(FILE_REF) {
                    Some(rest) => {
                        let mut it = rest.split('\u{1f}');
                        let path = it.next().unwrap_or("").to_string();
                        let num = |s: Option<&str>| s.and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0);
                        let (line, col) = (num(it.next()), num(it.next()));
                        self.open_file_ref(&path, line, col);
                    }
                    None => open_url(&url),
                },
                Some(MenuCmd::Tab(cmd, i)) => self.run_tab_cmd(cmd, i),
                Some(MenuCmd::Type(text)) => self.write_active(text.as_bytes()),
                Some(MenuCmd::Setting(ch)) => self.apply_setting(ch),
                Some(MenuCmd::Export(out)) => {
                    let job = self.export_job.take();
                    self.do_export(job, out);
                }
                Some(MenuCmd::ExportPick(job)) => self.export_menu(job),
                Some(MenuCmd::Shot(job)) => self.do_export(Some(job), export_ui::ExportOut::Shot),
                Some(MenuCmd::MarkAt(line)) => self.mark_start(Some(line)),
                Some(MenuCmd::ClearMark) => {
                    self.mark = None;
                    self.toast("mark cleared", 1.2);
                }
                Some(MenuCmd::Copy(text)) => {
                    if let Some(cb) = self.clipboard.as_mut() {
                        let _ = cb.set_text(text);
                    }
                    self.toast("copied", 1.0);
                }
                None => {}
            }
        }
        self.window.request_redraw();
    }

    /// Keys while the ⌄ menu is open: arrows, Enter, Esc.
    fn menu_key(&mut self, event: &KeyEvent) {
        let Some(m) = self.menu.as_mut() else { return };
        let n = m.items.len();
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) | Key::Named(NamedKey::Tab) => {
                m.selected = Some(m.selected.map_or(0, |i| (i + 1) % n));
            }
            Key::Named(NamedKey::ArrowUp) => m.selected = Some(m.selected.map_or(n - 1, |i| (i + n - 1) % n)),
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => {
                let i = m.selected.unwrap_or(0);
                self.run_menu_item(i);
                return;
            }
            _ => self.menu = None,
        }
        self.window.request_redraw();
    }

    /// The palette: the static list plus saved layouts and other workspaces.
    fn palette_entries(&self) -> Vec<palette::Entry> {
        let mut e = palette::entries(&self.config);
        let me = u64::from(self.window.id());
        for (id, name, detached) in &self.workspaces {
            if *id != me {
                let a = Action::Workspace(*id, name.clone());
                let hint = if *detached { "detached".to_string() } else { String::new() };
                e.push(palette::Entry { label: a.label(), hint, command: palette::Command::Action(a) });
            }
        }
        for l in session::layouts() {
            let a = Action::OpenLayout(l);
            e.push(palette::Entry { label: a.label(), hint: String::new(), command: palette::Command::Action(a) });
        }
        e
    }

    fn open_config_file(&mut self) {
        match self.watcher.path.clone() {
            Some(path) if path.is_file() => {
                if !open_path(&path) {
                    self.toast("could not open vibex.lua", 4.0);
                }
            }
            Some(path) => self.toast(format!("no config file yet: {}", path.display()), 5.0),
            None => self.toast("no config path", 3.0),
        }
    }

    /// Facts for Settings → About / Updates.
    fn about_info(&self) -> settings_ui::AboutInfo {
        let renderer = match &self.backend {
            Backend::Gpu(g) => format!("GPU: {}{}", g.adapter_info, if g.transparent { ", acrylic" } else { "" }),
            Backend::Cpu { .. } => "CPU (software)".into(),
        };
        let openconsole = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("OpenConsole.exe").is_file())).unwrap_or(false);
        let host = match self.active().host() {
            "openconsole" => "OpenConsole (images pass through)",
            "conhost" => "Windows console host (no inline images)",
            other => other,
        };
        settings_ui::AboutInfo {
            version: version(),
            renderer,
            host: host.into(),
            config_path: self.watcher.path.as_ref().map_or("(none)".into(), |p| p.display().to_string()),
            settings_path: settings::path().map_or("(none)".into(), |p| p.display().to_string()),
            update_status: self.update_status.clone(),
            update_available: self.update.as_ref().map(|f| f.version.clone()),
            openconsole,
            editors: {
                let pins = bundle::installed();
                let v = |n: &str| pins.iter().find(|p| p.name == n).map(|p| p.version.clone());
                match (v("nano"), v("nvim")) {
                    (Some(a), Some(b)) => format!("nano {a} \u{b7} Neovim {b}"),
                    _ => String::new(),
                }
            },
            editors_status: self.editors_status.clone(),
            editors_available: self.editors_found.as_ref().map(|f| f.describe()),
        }
    }

    /// Carry out what a Settings input asked for.
    fn settings_effect(&mut self, e: settings_ui::Effect) {
        use settings_ui::Effect;
        match e {
            Effect::None | Effect::Redraw => {}
            Effect::Change(ch) => self.apply_setting(ch),
            Effect::Perform(a) => {
                self.settings_ui.close();
                self.perform(a);
            }
            Effect::OpenConfigFile => self.open_config_file(),
            Effect::OpenConfigFolder => {
                if let Some(d) = config::config_dir() {
                    let _ = std::fs::create_dir_all(&d);
                    open_path(&d);
                }
            }
            Effect::CheckUpdates => self.check_updates(true),
            Effect::InstallUpdate => self.perform(Action::InstallUpdate),
            Effect::Copy(text) => {
                if let Some(cb) = self.clipboard.as_mut() {
                    let _ = cb.set_text(text);
                }
                self.toast("copied", 1.2);
            }
            Effect::Invalid(msg) => self.toast(msg, 4.0),
            Effect::Dropdown(i) => self.open_dropdown(i),
            Effect::NewEngagement(name) => match engagement::Engagement::open(&name) {
                Ok(e) => self.apply_setting(settings::Change::Set("engagement", settings::Val::Str(e.name))),
                Err(err) => self.toast(err, 4.0),
            },
            Effect::SetScope(text) => {
                if let Some(e) = self.eng.as_mut() {
                    match e.set_scope(&text) {
                        Ok(()) if e.scope.is_empty() => self.toast("scope cleared: nothing is checked", 3.0),
                        Ok(()) => {
                            let n = e.scope.text.len();
                            self.toast(format!("scope: {n} entr{}", if n == 1 { "y" } else { "ies" }), 2.5);
                        }
                        Err(msg) => self.toast(msg, 5.0),
                    }
                }
            }
            Effect::EngagementFolder => {
                if let Some(e) = &self.eng {
                    open_path(&e.dir);
                }
            }
            Effect::EngagementReport => self.engagement_report(),
            Effect::ShowLoot => self.show_loot(),
            Effect::AuditReport => self.audit_report(),
        }
        self.window.request_redraw();
    }

    /// Match triggers against the lines that finished since last time.
    fn run_triggers(&mut self, id: PaneId) {
        use config::TriggerAction;
        let Some(p) = self.panes.get_mut(&id) else { return };
        if p.term.is_alt_screen() {
            return;
        }
        let (epoch, evicted) = p.term.line_base();
        let cursor_abs = (p.term.total_lines() - p.term.rows() + p.term.cursor.y) as u64 + evicted;
        if epoch != p.trigger_epoch {
            // the text was re-wrapped: start again from here
            p.trigger_epoch = epoch;
            p.trigger_marks.clear();
            p.trigger_next = cursor_abs;
            return;
        }
        // only whole lines (the cursor's line may still be growing)
        let from = p.trigger_next.max(evicted);
        let to = cursor_abs.max(from).min(from + 2000);
        p.trigger_next = to;
        if from >= to {
            return;
        }
        let range = (from - evicted) as usize..(to - evicted) as usize;
        let mut fire: Vec<(usize, TriggerAction, String)> = Vec::new();
        for (ti, t) in self.config.triggers.iter().enumerate() {
            for m in p.term.find_regex_in(&t.regex, range.clone()).into_iter().take(200) {
                let stable = m.line as u64 + evicted;
                match &t.action {
                    TriggerAction::Highlight(c) => {
                        p.trigger_marks.push_back((stable, m.start, m.end, *c));
                        if p.trigger_marks.len() > 5000 {
                            p.trigger_marks.pop_front();
                        }
                    }
                    other => {
                        let text = p.term.text_range((m.start, m.line), (m.end, m.line));
                        fire.push((ti, other.clone(), text));
                    }
                }
            }
        }
        let now = Instant::now();
        let mut todo = Vec::new();
        for (ti, action, text) in fire {
            // at most once every 3 seconds per trigger
            let p = self.panes.get_mut(&id).unwrap();
            if p.trigger_fired.get(&ti).is_some_and(|t| now.duration_since(*t) < Duration::from_secs(3)) {
                continue;
            }
            p.trigger_fired.insert(ti, now);
            todo.push((action, text));
        }
        let title = self.panes[&id].title();
        for (action, text) in todo {
            match action {
                TriggerAction::Notify(t) => self.notify(&t, &format!("{text} — {title}")),
                TriggerAction::Toast => self.toast(text, 3.0),
                TriggerAction::Callback(idx) => match self.engine.call_with_text(idx, self.snapshot(), &text) {
                    Ok(cmds) => self.apply_cmds(cmds),
                    Err(e) => self.lua_error(e),
                },
                TriggerAction::Highlight(_) => {}
            }
        }
    }

    /// Ctrl+click on `file:line`: open it in the editor.
    fn open_file_ref(&mut self, path: &str, line: Option<u32>, col: Option<u32>) {
        use config::OpenIn;
        let cwd = self.current_cwd();
        let posix_cwd = cwd.as_deref().is_some_and(|c| c.starts_with('/'));
        let p = std::path::Path::new(path);
        let full = if p.is_absolute() || posix_cwd {
            p.to_path_buf()
        } else {
            match &cwd {
                Some(c) => std::path::Path::new(c).join(p),
                None => p.to_path_buf(),
            }
        };
        if !posix_cwd && !full.exists() {
            self.toast(format!("not found: {}", full.display()), 3.0);
            return;
        }
        let file = full.to_string_lossy().into_owned();
        let fill = |s: &str| s.replace("{file}", &file).replace("{line}", &line.unwrap_or(1).to_string()).replace("{col}", &col.unwrap_or(1).to_string());
        let editor = self.config.editor.clone().or_else(|| {
            config::find_on_path("code.cmd").map(|_| config::Editor {
                cmd: vec!["code.cmd".into(), "-g".into(), "{file}:{line}:{col}".into()],
                env: Vec::new(),
                open_in: OpenIn::App,
            })
        });
        let Some(ed) = editor else {
            if !open_path(&full) {
                self.toast("could not open the file", 3.0);
            }
            return;
        };
        let argv: Vec<String> = ed.cmd.iter().map(|a| fill(a)).collect();
        let Some((program, args)) = argv.split_first() else { return };
        if ed.open_in == OpenIn::App {
            let mut c = std::process::Command::new(program);
            c.args(args).envs(ed.env.iter().cloned());
            if let Some(d) = cwd.as_deref().filter(|d| !d.starts_with('/')) {
                c.current_dir(d);
            }
            if let Err(e) = c.spawn() {
                self.toast(format!("could not start {program}: {e}"), 4.0);
            }
            return;
        }
        // a terminal editor: run it in Vibex (through WSL for WSL folders)
        let mut cmd = if posix_cwd {
            let mut c = config::base_command("wsl.exe");
            c.args = vec!["--cd".into(), cwd.clone().unwrap_or_default(), "--".into()];
            c.args.extend(argv.iter().cloned());
            c
        } else {
            let mut c = config::base_command(program.clone());
            c.args = args.to_vec();
            c.cwd = cwd.clone();
            c
        };
        cmd.env.extend(ed.env.iter().cloned());
        let label = std::path::Path::new(&file).file_name().map_or(file.clone(), |n| n.to_string_lossy().into_owned());
        match self.spawn_command(&label, cmd) {
            Ok(id) => match ed.open_in {
                OpenIn::Popup => {
                    let old = self.tabs[self.active_tab].float.replace(id);
                    self.tabs[self.active_tab].float_shown = true;
                    if let Some(old) = old {
                        if let Some(p) = self.panes.remove(&old) {
                            p.kill();
                        }
                    }
                    self.after_focus_change();
                }
                OpenIn::Tab => {
                    self.tabs.push(Tab::new(id));
                    self.active_tab = self.tabs.len() - 1;
                    self.after_focus_change();
                }
                OpenIn::Split | OpenIn::App => {
                    let tab = &mut self.tabs[self.active_tab];
                    let target = tab.active;
                    tab.root.split(target, id, SplitDir::Horizontal);
                    tab.active = id;
                    tab.float_shown = false;
                    self.after_focus_change();
                }
            },
            Err(e) => self.toast(e, 4.0),
        }
    }

    /// The Settings dialog: centred over the content, most of it.
    fn settings_dialog_rect(&self, content: PxRect) -> PxRect {
        settings_dialog_rect(content, self.renderer.ui_scale)
    }

    /// Mouse press on the Settings page.
    fn settings_click(&mut self, button: MouseButton) {
        let (x, y) = (self.mouse.x as f32, self.mouse.y as f32);
        let Some((hit, rect)) = self.chrome_layout.settings.as_ref().and_then(|l| l.hit(x, y)) else {
            if !self.settings_rect.contains(x, y) {
                // a click outside the dialog closes it
                self.settings_ui.close();
            } else {
                self.settings_ui.focus = None;
            }
            self.window.request_redraw();
            return;
        };
        if button != MouseButton::Left {
            return;
        }
        // clicking away from a text field keeps what was typed
        if self.settings_ui.editing.is_some() && !matches!(hit, SettingsHit::Control(_)) {
            let e = self.settings_ui.commit_text();
            self.settings_effect(e);
        }
        self.settings_ui.capturing = false;
        let cfg = self.config.clone();
        let e = match hit {
            SettingsHit::Page(i) => {
                self.settings_ui.set_page(i);
                settings_ui::Effect::Redraw
            }
            SettingsHit::Footer => settings_ui::Effect::Change(settings::Change::Reset),
            SettingsHit::Close => {
                self.settings_ui.close();
                settings_ui::Effect::Redraw
            }
            SettingsHit::Filter => {
                self.settings_ui.focus = None;
                settings_ui::Effect::Redraw
            }
            SettingsHit::Row(i) | SettingsHit::Control(i) => self.settings_ui.activate(i, &cfg),
            SettingsHit::Prev(i) => self.settings_ui.step(i, -1, &cfg),
            SettingsHit::Next(i) => self.settings_ui.step(i, 1, &cfg),
            SettingsHit::Reset(i) => self.settings_ui.reset(i),
            SettingsHit::Slider(i) => {
                self.settings_drag = Some((i, rect));
                let knob = 8.0 * self.renderer.ui_scale;
                let f = (x - rect.x - knob) / (rect.w - 2.0 * knob).max(1.0);
                self.settings_ui.slide(i, f)
            }
        };
        self.settings_effect(e);
    }

    /// Keys on the Settings page. Ctrl/Alt combos fall through to the key
    /// bindings (Ctrl+Tab, Ctrl+Shift+P...) unless a field is taking input.
    fn settings_key(&mut self, event: &KeyEvent) -> bool {
        if !self.settings_ui.active {
            return false;
        }
        let (ctrl, alt, shift) = (self.mods.control_key(), self.mods.alt_key(), self.mods.shift_key());
        // waiting for a new shortcut
        if let Some(action) = self.settings_ui.capture_target() {
            let Some(spec) = KeySpec::from_event(event) else { return true };
            let mods = Mods::from_state(self.mods, false);
            let plain = !(mods.ctrl || mods.alt || mods.shift || mods.sup);
            self.settings_ui.capturing = false;
            let id = action.id().unwrap_or_default();
            match &spec {
                KeySpec::Named(NamedKey::Escape) if plain => {}
                KeySpec::Named(NamedKey::Backspace | NamedKey::Delete) if plain => {
                    self.apply_setting(settings::Change::Bind(id, None));
                }
                _ => {
                    if let Some(other) = keys::find(&self.config.bindings, &spec, mods).filter(|b| b.action != action) {
                        let combo = keys::Binding { key: spec.clone(), mods, action: action.clone() }.describe();
                        self.toast(format!("{combo} was {}; now it's {}", other.action.label(), action.label()), 4.0);
                    }
                    self.apply_setting(settings::Change::Bind(id, Some((spec.name(), mods.spec()))));
                }
            }
            self.window.request_redraw();
            return true;
        }
        // typing in a text field
        if self.settings_ui.editing.is_some() {
            match &event.logical_key {
                Key::Named(NamedKey::Escape) => self.settings_ui.editing = None,
                Key::Named(NamedKey::Enter) => {
                    let e = self.settings_ui.commit_text();
                    self.settings_effect(e);
                }
                Key::Named(NamedKey::Backspace) => {
                    if let Some(b) = self.settings_ui.editing.as_mut() {
                        b.pop();
                    }
                }
                Key::Character(c) if ctrl && c.eq_ignore_ascii_case("v") => {
                    let text = self.clipboard.as_mut().and_then(|cb| cb.get_text().ok()).unwrap_or_default();
                    let line = text.lines().next().unwrap_or("").to_string();
                    if let Some(b) = self.settings_ui.editing.as_mut() {
                        b.push_str(&line);
                    }
                }
                _ => {
                    if !ctrl && !alt {
                        if let Some(t) = event.text.as_ref().filter(|t| t.chars().all(|c| !c.is_control())) {
                            if let Some(b) = self.settings_ui.editing.as_mut() {
                                b.push_str(t);
                            }
                        }
                    }
                }
            }
            self.window.request_redraw();
            return true;
        }
        if ctrl || alt {
            return false;
        }
        let cfg = self.config.clone();
        let shortcuts = self.settings_ui.page() == settings_ui::Page::Shortcuts;
        let ui = &mut self.settings_ui;
        let mut effect = settings_ui::Effect::Redraw;
        match &event.logical_key {
            Key::Named(NamedKey::Tab) => ui.move_focus(if shift { -1 } else { 1 }),
            Key::Named(NamedKey::ArrowDown) => ui.move_focus(1),
            Key::Named(NamedKey::ArrowUp) => ui.move_focus(-1),
            Key::Named(NamedKey::PageDown) => ui.scroll += 300.0,
            Key::Named(NamedKey::PageUp) => ui.scroll = (ui.scroll - 300.0).max(0.0),
            Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::ArrowRight) => {
                let dir = if matches!(event.logical_key, Key::Named(NamedKey::ArrowLeft)) { -1 } else { 1 };
                match ui.focus {
                    Some(i) => effect = ui.step(i, dir, &cfg),
                    // no row focused: arrows move between pages
                    None => {
                        let n = settings_ui::PAGES.len();
                        let p = (ui.page as i32 + dir).rem_euclid(n as i32) as usize;
                        ui.set_page(p);
                    }
                }
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => {
                if let Some(i) = ui.focus {
                    effect = ui.activate(i, &cfg);
                } else if matches!(event.logical_key, Key::Named(NamedKey::Space)) && shortcuts {
                    ui.filter.push(' ');
                }
            }
            Key::Named(NamedKey::Escape) => {
                if shortcuts && !ui.filter.is_empty() {
                    ui.filter.clear();
                } else {
                    ui.close();
                }
            }
            Key::Named(NamedKey::Backspace) if shortcuts => {
                ui.filter.pop();
                ui.focus = None;
            }
            _ => {
                if shortcuts {
                    if let Some(t) = event.text.as_ref().filter(|t| t.chars().all(|c| !c.is_control())) {
                        ui.filter.push_str(t);
                        ui.focus = None;
                        ui.scroll = 0.0;
                    }
                }
            }
        }
        self.settings_effect(effect);
        true
    }

    fn toggle_maximize(&mut self) {
        self.window.set_maximized(!self.window.is_maximized());
    }

    /// A program asked for a notification (OSC 9 / 777): a toast while the
    /// window is focused, otherwise a Windows notification and a taskbar
    /// flash.
    fn notify(&mut self, title: &str, body: &str) {
        let clean = |s: &str| s.chars().filter(|c| !c.is_control()).take(200).collect::<String>();
        let (title, body) = (clean(title), clean(body));
        if self.focus_mode {
            self.held_notes.push((title, body));
            return;
        }
        if self.focused {
            let msg = if title.is_empty() {
                body
            } else if body.is_empty() {
                title
            } else {
                format!("{title}: {body}")
            };
            self.toast(msg, 4.0);
        } else {
            let title = if title.is_empty() { "Vibex".to_string() } else { title };
            notify::show(&self.window, &title, &body);
        }
    }

    fn close_window(&mut self) {
        notify::remove(&self.window);
        if !self.quit {
            self.save_session_with(true);
        }
        for p in self.panes.values() {
            p.kill();
        }
        self.quit = true;
    }

    fn mouse_button(&mut self, button: MouseButton, state: ElementState) {
        let pressed = state == ElementState::Pressed;
        let (x, y) = (self.mouse.x, self.mouse.y);
        if pressed && button == MouseButton::Left && self.confirm.is_none() && !self.tabs.is_empty() {
            let id = self.active_id();
            if let Some(n) = self.panes.get_mut(&id).and_then(|p| p.notes.as_mut()) {
                if n.rect.contains(x as f32, y as f32) {
                    n.focused = true;
                    self.window.request_redraw();
                    return;
                }
                n.focused = false;
            }
        }
        if self.confirm.is_some() {
            if pressed && button == MouseButton::Left {
                let hit = self.chrome_layout.confirm;
                match hit {
                    Some(l) if l.yes.contains(x as f32, y as f32) => self.confirm_answer(true),
                    Some(l) if l.no.contains(x as f32, y as f32) || !l.frame.contains(x as f32, y as f32) => self.confirm_answer(false),
                    _ => {}
                }
            }
            return;
        }

        if !pressed && button == MouseButton::Left {
            self.dragging = None;
            if self.settings_drag.take().is_some() {
                return;
            }
            if let Some(drag) = self.tab_drag.take() {
                self.window.set_cursor(CursorIcon::Text);
                if drag.out {
                    // dropped outside: into another window's tab bar, else a
                    // new window (a lone tab stays: there'd be nothing left)
                    if let Ok(origin) = self.window.inner_position() {
                        let p = PhysicalPosition::new(origin.x + x as i32, origin.y + y as i32);
                        let elsewhere = self.workspaces.iter().any(|(id, _, d)| *id != u64::from(self.window.id()) && !d);
                        if self.tabs.len() > 1 || elsewhere {
                            self.move_tab(drag.index, MoveTarget::Drop(p));
                        }
                    }
                }
                return;
            }
        }
        if pressed && self.menu.is_some() {
            let reopen = button == MouseButton::Right && self.tabbar.hit(x as f32, y as f32).is_none();
            match self.chrome_layout.menu.as_ref().and_then(|l| l.hit(x as f32, y as f32)) {
                Some(i) if matches!(button, MouseButton::Left | MouseButton::Right) => {
                    self.run_menu_item(i);
                    return;
                }
                Some(_) => return,
                None => {
                    // a click elsewhere closes it; on the ⌄ itself, that's all
                    self.menu = None;
                    self.window.request_redraw();
                    if self.tabbar.hit(x as f32, y as f32) == Some(TabHit::NewTabMenu) {
                        return;
                    }
                    if self.tabbar.hit(x as f32, y as f32).is_none() && !reopen {
                        return;
                    }
                }
            }
        }
        if pressed {
            if let (Some(dir), MouseButton::Left) = (self.edge_at(x, y), button) {
                let _ = self.window.drag_resize_window(dir);
                return;
            }
            if let Some(hit) = self.tabbar.hit(x as f32, y as f32) {
                self.tabbar_click(hit, button);
                return;
            }
            if self.settings_ui.active && self.palette.is_none() {
                self.settings_click(button);
                return;
            }
            if self.palette.is_some() {
                self.palette = None;
                self.window.request_redraw();
                return;
            }
            if button == MouseButton::Left {
                if let Some(d) = self.divider_at(x, y) {
                    self.dragging = Some(d);
                    return;
                }
            }
            // clicking a pane focuses it
            if let Some(id) = self.pane_at(x, y) {
                self.focus_pane(id);
            } else {
                return;
            }
        }

        // command blocks: the sticky header jumps to its prompt; the left
        // margin selects the whole block under it
        if pressed && button == MouseButton::Left && !self.mods.control_key() {
            let id = self.active_id();
            let (pad_x, pad_y, ch) = (self.renderer.padding_x as f64, self.renderer.padding_y as f64, self.renderer.fonts.cell_height as f64);
            let theme = self.renderer.theme.clone();
            let p = &self.panes[&id];
            let r = p.rect;
            if p.sticky_header(&theme).is_some() && y < r.y as f64 + ch + pad_y {
                let top = p.term.top_visible_line();
                if let Some(b) = p.term.block_at(top) {
                    self.active_mut().term.set_view_top(b.prompt);
                    self.window.request_redraw();
                    return;
                }
            }
            if self.config.shell_marks && x < r.x as f64 + pad_x && pad_x >= 4.0 {
                let (_, line) = self.abs_cell_in(id);
                if let Some(b) = self.panes[&id].term.block_at(line) {
                    let cols = self.panes[&id].term.cols();
                    self.active_mut().selection = Some(Selection { anchor: (0, b.prompt), head: (cols - 1, b.end) });
                    self.toast("block selected · Ctrl+Shift+C copies it", 1.5);
                    self.window.request_redraw();
                    return;
                }
            }
        }

        // Ctrl+click opens links, regardless of mouse mode.
        if pressed && button == MouseButton::Left && self.mods.control_key() {
            if let Some(link) = self.active().hover_link.clone() {
                if let Some(rest) = link.url.strip_prefix(FILE_REF) {
                    let mut it = rest.split('\u{1f}');
                    let path = it.next().unwrap_or("").to_string();
                    let num = |s: Option<&str>| s.and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0);
                    let (line, col) = (num(it.next()), num(it.next()));
                    self.open_file_ref(&path, line, col);
                } else {
                    open_url(&link.url);
                }
                return;
            }
        }

        if self.reporting() {
            let code = match button {
                MouseButton::Left => mouse::LEFT,
                MouseButton::Middle => mouse::MIDDLE,
                MouseButton::Right => mouse::RIGHT,
                _ => return,
            };
            if pressed {
                self.active_mut().buttons_down = Some(code);
                self.report(code, true, false);
            } else if self.active_mut().buttons_down.take().is_some() {
                self.report(code, false, false);
            }
            return;
        }

        let id = self.active_id();
        match (button, pressed) {
            (MouseButton::Left, true) => {
                let cell = self.abs_cell_in(id);
                let now = Instant::now();
                self.click_count = match self.last_click {
                    Some((t, c)) if now - t < self.multi_click && c == cell => self.click_count % 3 + 1,
                    _ => 1,
                };
                self.last_click = Some((now, cell));
                let clicks = self.click_count;
                let p = self.active_mut();
                p.selection = match clicks {
                    2 => {
                        let (s, e) = p.term.word_at(cell.0, cell.1);
                        Some(Selection { anchor: s, head: e })
                    }
                    3 => {
                        let (s, e) = p.term.line_selection(cell.1);
                        Some(Selection { anchor: s, head: e })
                    }
                    _ => Some(Selection { anchor: cell, head: cell }),
                };
                self.selecting_pane = (self.click_count == 1).then_some(id);
                self.window.request_redraw();
            }
            (MouseButton::Left, false) => {
                self.selecting_pane = None;
                let single = self.click_count == 1;
                let p = self.active_mut();
                if let Some(sel) = p.selection {
                    if sel.anchor == sel.head && single {
                        p.selection = None;
                        self.window.request_redraw();
                    }
                }
            }
            (MouseButton::Right, true) if self.config.right_click_menu => self.open_context_menu(x, y),
            (MouseButton::Right, true) => {
                // Windows Terminal style: copy if selected, else paste
                if self.copy_selection() {
                    self.active_mut().selection = None;
                    self.window.request_redraw();
                } else {
                    self.paste();
                }
            }
            (MouseButton::Middle, true) => self.paste(),
            _ => {}
        }
    }

    fn wheel(&mut self, delta: MouseScrollDelta) {
        if let Some(m) = self.menu.as_mut() {
            let dy = match delta {
                MouseScrollDelta::LineDelta(_, y) => y.round() as i32,
                MouseScrollDelta::PixelDelta(p) => (p.y / 30.0).round() as i32,
            };
            m.first = (m.first as i32 - dy * 2).max(0) as usize;
            m.selected = None;
            self.window.request_redraw();
            return;
        }
        let wheel_lines = self.config.wheel_lines;
        let ch = self.renderer.fonts.cell_height;
        if self.settings_ui.active {
            let px = match delta {
                MouseScrollDelta::LineDelta(_, y) => y * 60.0 * self.renderer.ui_scale,
                MouseScrollDelta::PixelDelta(p) => p.y as f32,
            };
            let max = self.chrome_layout.settings.as_ref().map_or(0.0, |l| l.max_scroll());
            self.settings_ui.scroll = (self.settings_ui.scroll - px).clamp(0.0, max);
            self.window.request_redraw();
            return;
        }
        // scroll the pane under the pointer
        if let Some(id) = self.pane_at(self.mouse.x, self.mouse.y) {
            self.focus_pane(id);
        }
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => y as f64 * wheel_lines as f64,
            MouseScrollDelta::PixelDelta(p) => p.y / ch as f64,
        };
        let reporting = self.reporting();
        let smooth = self.smooth_scroll;
        let p = self.active_mut();
        p.scroll_accum += lines;
        let whole = p.scroll_accum.trunc();
        p.scroll_accum -= whole;
        let n = whole as isize;
        if n == 0 {
            return;
        }
        if reporting {
            let code = if n > 0 { mouse::WHEEL_UP } else { mouse::WHEEL_DOWN };
            for _ in 0..n.unsigned_abs() {
                self.report(code, true, false);
            }
        } else if p.term.is_alt_screen() {
            let key: &[u8] = match (n > 0, p.term.modes.app_cursor) {
                (true, false) => b"\x1b[A",
                (true, true) => b"\x1bOA",
                (false, false) => b"\x1b[B",
                (false, true) => b"\x1bOB",
            };
            let seq = key.repeat(n.unsigned_abs());
            p.write(&seq);
        } else {
            let before = p.term.display_offset() as isize;
            p.term.scroll_view(n);
            let moved = p.term.display_offset() as isize - before;
            if smooth && moved != 0 {
                let limit = p.term.rows() as f32 * ch as f32;
                p.scroll_px = (p.scroll_px - moved as f32 * ch as f32).clamp(-limit, limit);
                p.last_frame = Instant::now();
            }
            self.window.request_redraw();
        }
    }

    // ---------------------------------------------------------- clipboard

    fn copy_selection(&mut self) -> bool {
        let text = self.active().selection_text();
        if text.is_empty() {
            return false;
        }
        if let Some(cb) = self.clipboard.as_mut() {
            let _ = cb.set_text(text);
        }
        true
    }

    fn paste(&mut self) {
        let text = self.clipboard.as_mut().and_then(|cb| cb.get_text().ok());
        let Some(text) = text else { return };
        if self.config.secret_guard && self.config.secrets.has_secret(&text) {
            let kinds = self.config.secrets.kinds(&text);
            let mut lines: Vec<String> = self.config.secrets.preview(&text);
            lines.push(String::new());
            lines.push(format!("Looks like: {}", kinds.join(", ")));
            lines.push("Vibex keeps it out of its history, recordings and audit log.".into());
            self.ask("Paste a secret?", lines, "Paste", false, ConfirmAct::Paste(text));
            return;
        }
        self.paste_now(&text);
    }

    fn paste_now(&mut self, text: &str) {
        let bytes = input::paste_bytes(text, self.active().term.modes.bracketed_paste);
        self.write_active(&bytes);
    }

    // ------------------------------------------------------------ questions

    fn confirm_key(&mut self, event: &KeyEvent) {
        let Some(c) = self.confirm.as_mut() else { return };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.confirm_answer(false),
            Key::Named(NamedKey::Enter) => {
                let yes = c.view.yes_focused;
                self.confirm_answer(yes);
            }
            Key::Named(NamedKey::Tab | NamedKey::ArrowLeft | NamedKey::ArrowRight) => {
                c.view.yes_focused = !c.view.yes_focused;
                self.window.request_redraw();
            }
            Key::Character(ch) if ch.eq_ignore_ascii_case("y") => self.confirm_answer(true),
            Key::Character(ch) if ch.eq_ignore_ascii_case("n") => self.confirm_answer(false),
            _ => {}
        }
    }

    fn confirm_answer(&mut self, yes: bool) {
        let Some(c) = self.confirm.take() else { return };
        self.window.request_redraw();
        if !yes {
            return;
        }
        match c.act {
            ConfirmAct::Paste(text) => self.paste_now(&text),
            ConfirmAct::Enter(id) => {
                if let Some(p) = self.panes.get_mut(&id) {
                    p.write(b"\r");
                }
            }
            ConfirmAct::CloseTab(i) => self.close_tab(i),
            ConfirmAct::ClosePane(id) => self.close_pane(id),
            ConfirmAct::CloseWindow => self.close_window(),
            ConfirmAct::UpdateEditors(found) => {
                self.toast("downloading nano and Neovim…", 30.0);
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let _ = proxy.send_event(AppEvent::Editors(bundle::apply(&found), true));
                });
            }
        }
    }

    /// Enter at a prompt: ask first if the line is a dangerous command, or
    /// the pane is on a host that asks for everything.
    fn check_enter(&mut self, event: &KeyEvent) -> bool {
        if !matches!(event.logical_key, Key::Named(NamedKey::Enter)) || self.mods.control_key() || self.mods.alt_key() || self.mods.shift_key() {
            return false;
        }
        self.check_enter_now()
    }

    /// The Enter check itself: true if it asked (and holds the Enter).
    fn check_enter_now(&mut self) -> bool {
        let id = self.active_id();
        let p = self.active();
        if p.term.is_alt_screen() || p.player.is_some() || p.exited || p.term.running_command().is_some() && p.ssh_host.is_none() {
            return false;
        }
        // what's typed: from shell integration, else the cursor's line
        // after the prompt (`$ `, `# `, `> `, `% `)
        let line = match p.term.prompt_input() {
            Some(i) => i.text,
            None => {
                let l = p.term.top_visible_line() + p.term.cursor.y + p.term.display_offset();
                let row = p.term.text_range((0, l), (p.term.cols().saturating_sub(1), l));
                let cut = ["$ ", "# ", "> ", "% "].iter().filter_map(|m| row.rfind(m).map(|i| i + m.len())).max().unwrap_or(0);
                row[cut..].to_string()
            }
        };
        let typed = line.trim();
        if typed.is_empty() {
            return false;
        }
        let mut why: Vec<String> = if self.config.confirm_risky {
            self.config.confirm_commands.iter().filter(|(_, re)| re.is_match(typed)).map(|(w, _)| format!("It {w}.")).collect()
        } else {
            Vec::new()
        };
        if let Some(e) = self.eng.as_ref().filter(|e| self.config.scope_guard && !e.scope.is_empty()) {
            let out: Vec<String> = engagement::targets(typed).into_iter().filter(|t| !e.scope.allows(t)).map(|t| t.to_string()).collect();
            if !out.is_empty() {
                why.push(format!("It targets {}, outside the scope of {}.", out.join(", "), e.name));
            }
        }
        if let Some(h) = p.ssh_host.as_deref() {
            if ssh::rule_for(h, &self.config.ssh_hosts).is_some_and(|r| r.confirm) && !typed.ends_with(':') {
                why.push(format!("This pane is connected to {h}."));
            }
        }
        if why.is_empty() {
            return false;
        }
        let mut lines = vec![typed.chars().take(200).collect::<String>(), String::new()];
        lines.extend(why);
        let danger = lines.iter().any(|l| l.starts_with("It "));
        self.ask("Run this?", lines, "Run it", danger, ConfirmAct::Enter(id));
        true
    }

    // ------------------------------------------------------------ engagement

    /// Load (or drop) the engagement the config names.
    fn sync_engagement(&mut self) {
        let want = self.config.engagement.clone().map(|n| engagement::slug(&n));
        if want.as_deref() == self.eng.as_ref().map(|e| e.name.as_str()) {
            return;
        }
        for p in self.panes.values_mut() {
            if p.recorder.as_ref().is_some_and(|r| r.engagement) {
                if let Some(r) = p.recorder.take() {
                    r.finish();
                }
            }
        }
        let was = self.eng.take().map(|e| e.name);
        if let Some(n) = want {
            match engagement::Engagement::open(&n) {
                Ok(e) => {
                    let scope =
                        if e.scope.is_empty() { "no scope yet (Settings → Security)".to_string() } else { format!("scope: {}", e.scope.text.join(", ")) };
                    self.toast(format!("Engagement {} on: every command and its output is logged · {scope}", e.name), 5.0);
                    // count from here, not the whole scrollback
                    for p in self.panes.values_mut() {
                        p.loot_epoch = u64::MAX;
                    }
                    self.eng = Some(e);
                }
                Err(err) => self.toast(format!("engagement: {err}"), 4.0),
            }
        } else if let Some(n) = was {
            self.toast(format!("Engagement {n} off"), 2.5);
        }
        self.relayout_tabbar();
        self.window.request_redraw();
    }

    /// Per pane, when output arrives: start / stop its recording, and look
    /// for loot in the lines that finished.
    fn engagement_pane(&mut self, id: PaneId) {
        let record = self.config.engagement_record;
        let dir = self.eng.as_ref().map(|e| e.recordings());
        let guard = self.config.secret_guard.then(|| self.config.secrets.clone());
        if let Some(p) = self.panes.get_mut(&id) {
            match (&dir, record) {
                (Some(dir), true) if p.recorder.is_none() && p.player.is_none() && !p.exited => {
                    let title = p.title();
                    if let Ok(mut r) = record::Recorder::start_in(dir, p.term.cols(), p.term.rows(), &title) {
                        r.engagement = true;
                        r.guard = guard;
                        p.recorder = Some(r);
                    }
                }
                (None, _) | (_, false) if p.recorder.as_ref().is_some_and(|r| r.engagement) => {
                    if let Some(r) = p.recorder.take() {
                        r.finish();
                    }
                }
                _ => {}
            }
        }
        if self.eng.is_none() || !self.config.collect_loot {
            return;
        }
        let Some(p) = self.panes.get_mut(&id) else { return };
        if p.term.is_alt_screen() {
            return;
        }
        let (epoch, evicted) = p.term.line_base();
        let cursor_abs = (p.term.total_lines() - p.term.rows() + p.term.cursor.y) as u64 + evicted;
        if epoch != p.loot_epoch {
            p.loot_epoch = epoch;
            p.loot_next = cursor_abs;
            return;
        }
        let from = p.loot_next.max(evicted);
        let to = cursor_abs.max(from).min(from + 3000);
        if from >= to {
            return;
        }
        // whole logical lines only: stop before one that wraps past `to`
        let cols = p.term.cols();
        let mut found = Vec::new();
        let mut buf = String::new();
        let mut done = from;
        for l in (from - evicted) as usize..(to - evicted) as usize {
            let Some(row) = p.term.absolute_row(l) else { break };
            let wrapped = row.wrapped;
            buf.push_str(&p.term.text_range((0, l), (cols.saturating_sub(1), l)));
            if wrapped {
                continue;
            }
            found.extend(engagement::loot_in(&buf, &mut p.loot_host, &self.config.secrets));
            buf.clear();
            done = l as u64 + 1 + evicted;
        }
        p.loot_next = done;
        if found.is_empty() {
            return;
        }
        let cmd = p.term.running_command().map(|c| c.text.clone()).unwrap_or_default();
        let cmd = self.config.secrets.redact(&cmd);
        let Some(e) = self.eng.as_ref() else { return };
        let n = engagement::record_loot(&e.loot_path(), &found, &cmd);
        if n > 0 {
            let mut kinds: Vec<String> = found.iter().map(|l| l.kind.clone()).collect();
            kinds.dedup();
            self.toast(format!("loot: {n} new ({}) · Alt+Shift+L", kinds.join(", ")), 3.0);
        }
    }

    /// Alt+Shift+L: what the engagement collected; Enter copies one.
    fn show_loot(&mut self) {
        let Some(e) = &self.eng else {
            self.toast("no engagement on (Settings → Security)", 3.0);
            return;
        };
        let loot = engagement::read_loot(&e.loot_path());
        if loot.is_empty() {
            self.toast("nothing collected yet", 2.0);
            return;
        }
        let now = history::now();
        let show = self.reveal_secrets;
        let entries = loot
            .into_iter()
            .map(|(t, l, cmd)| {
                let value = if show || l.kind == "open port" { l.value.clone() } else { engagement::mask(&l.value) };
                let host = if l.host.is_empty() { String::new() } else { format!("{} · ", l.host) };
                palette::Entry {
                    label: format!("{} · {value}", l.kind),
                    hint: format!(
                        "{host}{}{}",
                        history::ago(t, now),
                        if cmd.is_empty() { String::new() } else { format!(" · {}", cmd.chars().take(30).collect::<String>()) }
                    ),
                    command: palette::Command::Copy(l.value),
                }
            })
            .collect();
        self.settings_ui.close();
        self.palette = Some(Palette::new(entries));
        self.window.request_redraw();
    }

    fn engagement_report(&mut self) {
        let Some(e) = &self.eng else {
            self.toast("no engagement on (Settings → Security)", 3.0);
            return;
        };
        let out = e.dir.join("report.html");
        match std::fs::write(&out, engagement::report(e)) {
            Ok(()) => {
                open_path(&out);
                self.toast(format!("report: {}", out.display()), 4.0);
            }
            Err(err) => self.toast(format!("can't write the report: {err}"), 4.0),
        }
    }

    /// A Settings dropdown under its box.
    fn open_dropdown(&mut self, i: usize) {
        let cfg = self.config.clone();
        let Some((opts, cur)) = self.settings_ui.options(i, &cfg) else { return };
        let rect = self.chrome_layout.settings.as_ref().and_then(|l| l.regions.iter().find(|(_, h)| *h == SettingsHit::Control(i)).map(|(r, _)| *r));
        let Some(rect) = rect else { return };
        let (items, first) = dropdown_items(opts, cur);
        let items: Vec<(MenuItem, MenuCmd)> = items.into_iter().map(|(m, ch)| (m, MenuCmd::Setting(ch))).collect();
        self.menu = Some(Menu { items, typing: false, first, min_width: rect.w, selected: Some(cur), x: rect.x, y: rect.y + rect.h + 4.0 });
        self.window.request_redraw();
    }

    // ------------------------------------------------------------ notes

    fn toggle_notes(&mut self) {
        let cwd = self.current_cwd();
        let p = self.active_mut();
        match p.notes.as_mut() {
            Some(n) if n.focused => {
                n.save();
                p.notes = None;
            }
            Some(n) => n.focused = true,
            None => p.notes = Some(notes::Notes::open(cwd.as_deref())),
        }
        self.relayout();
        self.window.request_redraw();
    }

    /// Keys while the notes have the keyboard. Shortcuts with Ctrl+Shift
    /// or Alt still work.
    fn notes_key(&mut self, event: &KeyEvent) -> bool {
        if self.tabs.is_empty() || !self.active().notes.as_ref().is_some_and(|n| n.focused) {
            return false;
        }
        let (ctrl, shift, alt) = (self.mods.control_key(), self.mods.shift_key(), self.mods.alt_key());
        if alt || (ctrl && shift) {
            return false;
        }
        let paste = if ctrl && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("v")) {
            self.clipboard.as_mut().and_then(|cb| cb.get_text().ok())
        } else {
            None
        };
        let Some(n) = self.active_mut().notes.as_mut() else { return false };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => n.focused = false,
            Key::Named(NamedKey::Enter) => n.insert("\n"),
            Key::Named(NamedKey::Tab) => n.insert("  "),
            Key::Named(NamedKey::Backspace) => n.backspace(),
            Key::Named(NamedKey::Delete) => n.delete(),
            Key::Named(NamedKey::ArrowLeft) => n.left(),
            Key::Named(NamedKey::ArrowRight) => n.right(),
            Key::Named(NamedKey::ArrowUp) => n.vertical(false),
            Key::Named(NamedKey::ArrowDown) => n.vertical(true),
            Key::Named(NamedKey::Home) => n.home(),
            Key::Named(NamedKey::End) => n.end(),
            _ if paste.is_some() => n.insert(&paste.unwrap_or_default().replace("\r\n", "\n")),
            _ if ctrl => {}
            _ => {
                if let Some(t) = event.text.as_ref().filter(|t| t.chars().all(|c| !c.is_control())) {
                    n.insert(t);
                }
            }
        }
        self.window.request_redraw();
        true
    }

    // ------------------------------------------------------------ audit

    fn audit_report(&mut self) {
        let Some(path) = audit::path() else { return };
        let Ok(text) = std::fs::read_to_string(&path) else {
            self.toast("no audit log yet: turn it on in Settings → Terminal → Audit log", 4.0);
            return;
        };
        let html = audit::report(&text, 0);
        let Some(dir) = config::config_dir().map(|d| d.join("audit-reports")) else { return };
        let out = dir.join(format!("audit-{}.html", audit::iso(history::now()).replace([' ', ':'], "-").trim_end_matches('Z')));
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&out, html)) {
            Ok(()) => {
                open_path(&out);
                self.toast(format!("report: {}", out.display()), 5.0);
            }
            Err(e) => self.toast(format!("can't write the report: {e}"), 4.0),
        }
    }

    fn audit_verify(&mut self) {
        let Some(path) = audit::path() else { return };
        let msg = match audit::verify(&path) {
            Ok(audit::Verdict::Intact { entries, first, last }) => {
                format!("✓ audit log intact: {}, {} → {}", audit::entries(entries), audit::iso(first), audit::iso(last))
            }
            Ok(audit::Verdict::Broken { line, reason, good }) => {
                format!("✗ audit log broken at line {line}: {reason} ({} before it intact)", audit::entries(good))
            }
            Err(_) => "no audit log yet: turn it on in Settings → Terminal → Audit log".to_string(),
        };
        self.toast(msg, 6.0);
    }

    // ------------------------------------------------------------ focus mode

    fn toggle_focus_mode(&mut self) {
        self.focus_mode = !self.focus_mode;
        if self.focus_mode {
            if !self.fullscreen {
                self.fullscreen = true;
                self.focus_fullscreen = true;
                self.window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            }
            self.renderer.status_bar = false;
            self.toast("Focus mode: notifications wait until you leave (Ctrl+Shift+Enter)", 3.0);
        } else {
            if self.focus_fullscreen {
                self.fullscreen = false;
                self.focus_fullscreen = false;
                self.window.set_fullscreen(None);
            }
            self.renderer.status_bar = self.config.status_bar;
            let held = std::mem::take(&mut self.held_notes);
            if !held.is_empty() {
                let mut msg =
                    format!("{} while you were focused:", if held.len() == 1 { "1 notification".to_string() } else { format!("{} notifications", held.len()) });
                for (t, b) in held.iter().rev().take(6) {
                    msg.push_str(&format!("\n· {t}{}{b}", if b.is_empty() { "" } else { ": " }));
                }
                self.toast(msg, 10.0);
            }
        }
        self.relayout();
        self.relayout_tabbar();
        self.window.request_redraw();
    }

    // -------------------------------------------------------- config + Lua

    fn snapshot(&self) -> Snapshot {
        let p = self.active();
        let tab = &self.tabs[self.active_tab];
        Snapshot {
            title: p.term.title().to_string(),
            selection: p.selection_text(),
            profile: p.profile.clone(),
            cols: p.term.cols(),
            rows: p.term.rows(),
            focused: self.focused,
            leader: self.leader_active(),
            font_size: self.config.font_size_pt,
            opacity: self.opacity,
            alt_screen: p.term.is_alt_screen(),
            scrollback: p.term.scrollback_len(),
            tab_count: self.tabs.len(),
            tab_index: self.active_tab + 1,
            pane_count: tab.root.panes().len(),
            cwd: p.term.cwd().unwrap_or_default().to_string(),
            last_exit: p.term.last_exit(),
        }
    }

    fn leader_active(&self) -> bool {
        self.leader_until.is_some_and(|t| Instant::now() < t)
    }

    fn toast(&mut self, msg: impl Into<String>, secs: f32) {
        self.toast = Some((msg.into(), Instant::now() + Duration::from_secs_f32(secs.max(0.2))));
        self.window.request_redraw();
    }

    fn lua_error(&mut self, e: String) {
        eprintln!("vibex: {e}");
        self.toast(format!("lua: {e}"), 6.0);
    }

    fn emit(&mut self, event: &str, arg: EventArg) {
        if !self.engine.has_handler(event) {
            return;
        }
        match self.engine.emit(event, self.snapshot(), arg) {
            Ok(cmds) => self.apply_cmds(cmds),
            Err(e) => self.lua_error(e),
        }
    }

    fn apply_cmds(&mut self, cmds: Vec<Cmd>) {
        for cmd in cmds {
            match cmd {
                Cmd::SendText(s) => self.write_active(s.as_bytes()),
                Cmd::CopyText(s) => {
                    if let Some(cb) = self.clipboard.as_mut() {
                        let _ = cb.set_text(s);
                    }
                }
                Cmd::Paste => self.paste(),
                Cmd::Toast(msg, secs) => self.toast(msg, secs),
                Cmd::SetTitle(t) => self.window.set_title(&t),
                Cmd::SpawnWindow(p) => self.perform(Action::SpawnWindow(p)),
                Cmd::SetFontSize(pt) => {
                    self.config.font_size_pt = pt.clamp(4.0, 72.0);
                    self.base_font_px = font_px(self.config.font_size_pt, self.window.scale_factor());
                    let px = self.base_font_px;
                    self.set_font_px(px);
                }
                Cmd::SetOpacity(o) => self.set_opacity(o),
                Cmd::SetTheme(t) => {
                    set_dark_mode(&self.window, t.background);
                    self.renderer.theme = t;
                    self.next_status = Instant::now();
                }
                Cmd::Scroll(n) => self.active_mut().term.scroll_view(n),
                Cmd::Search(q) => {
                    let p = self.active_mut();
                    p.open_search();
                    if let (Some(q), Some(s)) = (q, p.search.as_mut()) {
                        s.query = q;
                        p.search_changed();
                    }
                }
                Cmd::Reload => self.reload_config(true),
                Cmd::Action(a) => self.perform(a),
                Cmd::SetTabTitle(t) => {
                    self.tabs[self.active_tab].title = t;
                    self.session_dirty = true;
                    self.relayout_tabbar();
                }
                Cmd::SetTabColor(c) => self.perform(Action::TabColor(c)),
            }
        }
        self.window.request_redraw();
    }

    fn set_opacity(&mut self, o: f32) {
        let o = o.clamp(0.0, 1.0);
        let transparent = matches!(&self.backend, Backend::Gpu(g) if g.transparent);
        if o < 1.0 && !transparent {
            self.toast("transparency needs a restart (Vibex started opaque)", 4.0);
        }
        self.opacity = o;
    }

    fn perform(&mut self, action: Action) {
        let page = self.active().term.rows().saturating_sub(1).max(1) as isize;
        match action {
            Action::Copy => {
                self.copy_selection();
            }
            Action::Paste => self.paste(),
            Action::Search => {
                let p = self.active_mut();
                if p.search.is_some() {
                    p.search = None;
                } else {
                    p.open_search();
                }
            }
            Action::ReloadConfig => self.reload_config(true),
            Action::IncreaseFontSize => {
                let px = self.renderer.fonts.size_px() + 1.0;
                self.set_font_px(px);
            }
            Action::DecreaseFontSize => {
                let px = self.renderer.fonts.size_px() - 1.0;
                self.set_font_px(px);
            }
            Action::ResetFontSize => {
                let px = self.base_font_px;
                self.set_font_px(px);
            }
            Action::ScrollPageUp => self.active_mut().term.scroll_view(page),
            Action::ScrollPageDown => self.active_mut().term.scroll_view(-page),
            Action::ScrollToTop => self.active_mut().term.scroll_view(isize::MAX / 2),
            Action::ScrollToBottom => self.active_mut().term.scroll_view_to_bottom(),
            Action::ClearScrollback => {
                let p = self.active_mut();
                p.term.clear_scrollback();
                p.selection = None;
            }
            Action::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                self.window.set_fullscreen(self.fullscreen.then_some(winit::window::Fullscreen::Borderless(None)));
                self.relayout();
            }
            Action::SendString(s) => self.write_active(s.as_bytes()),
            Action::SpawnWindow(profile) => {
                if let Some(p) = &profile {
                    if self.config.profile(p).is_none() {
                        self.toast(format!("no profile named '{p}'"), 4.0);
                        return;
                    }
                }
                // a new window in this same process
                let cwd = self.current_cwd();
                self.open_requests.push(single::OpenRequest { profile, cwd, config: self.watcher.path.clone(), ..Default::default() });
            }
            Action::PassThrough => {}
            Action::Callback(idx) => match self.engine.call(idx, self.snapshot()) {
                Ok(cmds) => self.apply_cmds(cmds),
                Err(e) => self.lua_error(e),
            },
            Action::NewTab(p) => {
                if let Err(e) = self.new_tab(p.as_deref()) {
                    self.toast(e, 4.0);
                }
            }
            Action::CloseTab => self.request_close_tab(self.active_tab),
            Action::NextTab => {
                let n = self.tabs.len();
                self.activate_tab((self.active_tab + 1) % n);
            }
            Action::PrevTab => {
                let n = self.tabs.len();
                self.activate_tab((self.active_tab + n - 1) % n);
            }
            Action::ActivateTab(i) => {
                let last = self.tabs.len() - 1;
                self.activate_tab(if i >= 9 { last } else { (i - 1).min(last) });
            }
            Action::SplitRight(p) => self.split(SplitDir::Horizontal, p.as_deref()),
            Action::SplitDown(p) => self.split(SplitDir::Vertical, p.as_deref()),
            Action::ClosePane => self.request_close_pane(self.active_id()),
            Action::FocusPane(dir) => {
                let size = self.window.inner_size();
                let content = self.renderer.content_rect(size.width as usize, size.height as usize, self.tabbar_visible());
                let area = layout::Rect { x: content.x, y: content.y, w: content.w, h: content.h };
                let (rects, _) = self.tabs[self.active_tab].root.layout(area);
                if let Some(id) = layout::neighbor(&rects, self.active_id(), dir) {
                    self.tabs[self.active_tab].zoomed = false;
                    self.focus_pane(id);
                }
            }
            Action::ResizePane(dir) => {
                let id = self.active_id();
                if self.tabs[self.active_tab].root.resize(id, dir, 0.05) {
                    self.relayout();
                }
            }
            Action::TogglePaneZoom => {
                let tab = &mut self.tabs[self.active_tab];
                if tab.root.panes().len() > 1 {
                    tab.zoomed = !tab.zoomed;
                    self.relayout();
                }
            }
            Action::CommandPalette => {
                if self.palette.is_some() {
                    self.palette = None;
                } else {
                    self.palette = Some(Palette::new(self.palette_entries()));
                }
            }
            Action::RenameTab => {
                let current = self.tabs[self.active_tab].title.clone().unwrap_or_else(|| self.active().title());
                self.rename = Some((self.active_tab, current));
                self.relayout_tabbar();
            }
            Action::TabColor(c) => {
                self.tabs[self.active_tab].color = c;
                self.session_dirty = true;
                self.relayout_tabbar();
            }
            Action::ToggleBroadcast => {
                let tab = &mut self.tabs[self.active_tab];
                tab.broadcast = !tab.broadcast;
                if tab.broadcast && tab.root.panes().len() < 2 {
                    self.toast("broadcast is on: split the tab to type into several panes", 3.0);
                }
                self.relayout_tabbar();
            }
            Action::SshPicker => {
                if ssh::hosts().is_empty() {
                    self.toast("no hosts found in ~/.ssh/config or ~/.ssh/known_hosts", 4.0);
                } else {
                    self.open_palette_with("ssh ");
                }
            }
            Action::Ssh(host) => match self.spawn_command(&host, ssh::command(&host)) {
                Ok(id) => {
                    if let Some(p) = self.panes.get_mut(&id) {
                        p.ssh_host = Some(host.clone());
                        p.ssh_fixed = true;
                    }
                    ssh::remember(&host);
                    let mut tab = Tab::new(id);
                    tab.title = Some(host.clone());
                    self.tabs.push(tab);
                    self.active_tab = self.tabs.len() - 1;
                    self.after_focus_change();
                }
                Err(e) => self.toast(e, 4.0),
            },
            Action::CheckForUpdates => self.check_updates(true),
            Action::UpdateEditors => match self.editors_found.clone() {
                Some(found) => self.ask_update_editors(found),
                None => self.check_editors(true),
            },
            Action::ToggleFloatingPane => self.toggle_float(),
            Action::DetachWindow => {
                self.save_session();
                self.detached = Some(Instant::now());
                self.window.set_visible(false);
                self.palette = None;
                self.menu = None;
            }
            Action::SaveLayout => {
                let name = session::layout_file_name(&self.name);
                let Some(dir) = session::layouts_dir() else { return };
                let saved = session::Saved { tabs: self.saved_tabs(None), active_tab: self.active_tab, size: None, maximized: false };
                let file = dir.join(format!("{name}.lua"));
                match std::fs::create_dir_all(&dir).and_then(|_| session::save(&file, &saved)) {
                    Ok(()) => self.toast(format!("saved layout '{name}': {}\nOpen it from the palette (Open layout: {name})", file.display()), 5.0),
                    Err(e) => self.toast(format!("could not save the layout: {e}"), 4.0),
                }
            }
            Action::OpenLayout(name) => {
                self.open_requests.push(single::OpenRequest { layout: Some(name), ..Default::default() });
            }
            a @ (Action::Workspace(..) | Action::QuitAll) => self.ws_requests.push(a),
            Action::HistorySearch => self.open_history(),
            Action::Completions => self.open_completions(),
            Action::RevealSecrets => {
                self.reveal_secrets = !self.reveal_secrets;
                self.toast(if self.reveal_secrets { "secrets shown (palette again to hide)" } else { "secrets masked" }, 2.0);
            }
            Action::ReopenClosedTab => self.reopen_closed_tab(),
            Action::ToggleFocusMode => self.toggle_focus_mode(),
            Action::ToggleNotes => self.toggle_notes(),
            Action::AuditReport => self.audit_report(),
            Action::AuditVerify => self.audit_verify(),
            Action::ShowLoot => self.show_loot(),
            Action::EngagementReport => self.engagement_report(),
            Action::SetEngagement(name) => self.apply_setting(settings::Change::Set("engagement", settings::Val::Str(name.unwrap_or_default()))),
            Action::ToggleRecording => {
                let title = self.active().title();
                let guard = self.config.secret_guard.then(|| self.config.secrets.clone());
                let (cols, rows) = (self.active().term.cols(), self.active().term.rows());
                let p = self.active_mut();
                match p.recorder.take() {
                    Some(r) => {
                        let path = r.finish();
                        self.toast(format!("saved {}\npalette → Play recording / Export recording as GIF", path.display()), 6.0);
                    }
                    None => match record::Recorder::start(cols, rows, &title) {
                        Ok(mut r) => {
                            let shown = r.path.display().to_string();
                            r.guard = guard;
                            p.recorder = Some(r);
                            self.toast(format!("● recording → {shown}\nAlt+Shift+R stops"), 3.0);
                        }
                        Err(e) => self.toast(format!("can't record: {e}"), 4.0),
                    },
                }
                self.relayout_tabbar();
            }
            Action::PlayRecording(path) => self.play_recording(&path),
            Action::ExportGif(path) => {
                // a separate process, so a long recording doesn't hold up typing
                let gif = std::path::Path::new(&path).with_extension("gif");
                match std::env::current_exe() {
                    Ok(exe) => {
                        let mut c = std::process::Command::new(exe);
                        c.args(["cast2gif", &path, &gif.to_string_lossy(), "--open"]);
                        #[cfg(windows)]
                        {
                            use std::os::windows::process::CommandExt;
                            c.creation_flags(0x0800_0000); // no console window
                        }
                        match c.spawn() {
                            Ok(_) => self.toast(format!("making {} (it opens when ready)", gif.display()), 4.0),
                            Err(e) => self.toast(format!("can't export: {e}"), 4.0),
                        }
                    }
                    Err(e) => self.toast(format!("can't export: {e}"), 4.0),
                }
            }
            Action::SelectAll => {
                let p = self.active_mut();
                let (cols, total) = (p.term.cols(), p.term.total_lines());
                if total > 0 {
                    p.selection = Some(Selection { anchor: (0, 0), head: (cols.saturating_sub(1), total - 1) });
                }
                self.window.request_redraw();
            }
            a @ (Action::CopyLastOutput | Action::CopyLastCommand) => {
                let want_output = a == Action::CopyLastOutput;
                let p = self.active();
                match p.term.last_finished_block() {
                    Some(b) => {
                        let text = if want_output { p.term.block_output(&b) } else { b.command.text.clone() };
                        if text.is_empty() {
                            self.toast(if want_output { "the last command printed nothing" } else { "no command text (needs shell integration)" }, 2.5);
                        } else {
                            let n = text.lines().count();
                            if let Some(cb) = self.clipboard.as_mut() {
                                let _ = cb.set_text(text);
                            }
                            self.toast(if want_output { format!("copied {n} lines of output") } else { "copied the command".into() }, 1.5);
                        }
                    }
                    None => self.toast("no finished command yet: needs shell integration (docs/SHELL_INTEGRATION.md)", 4.0),
                }
            }
            Action::ScreenshotPane => {
                let job = self.job_screen(self.active_id());
                self.do_export(job, export_ui::ExportOut::Shot);
            }
            Action::ScreenshotCommand => {
                let id = self.active_id();
                match self.job_block(id, None) {
                    Some(job) => self.do_export(Some(job), export_ui::ExportOut::Shot),
                    None => self.toast("no finished command yet: needs shell integration (docs/SHELL_INTEGRATION.md)", 4.0),
                }
            }
            Action::CopyImage => match self.job_selection(self.active_id()) {
                Some(job) => self.do_export(Some(job), export_ui::ExportOut::CopyImage),
                None => self.toast("select something first", 2.0),
            },
            Action::SaveRange => self.save_range(),
            Action::Mark => {
                if self.mark_now().is_some_and(|(p, _)| p == self.active_id()) {
                    self.mark_end(None);
                } else {
                    self.mark_start(None);
                }
            }
            Action::MarkStart => self.mark_start(None),
            Action::MarkEnd => self.mark_end(None),
            Action::RerunLastCommand => {
                let text = self.active().term.last_finished_block().map(|b| b.command.text).unwrap_or_default();
                if text.is_empty() {
                    self.toast("no command to rerun (needs shell integration)", 3.0);
                } else {
                    self.write_active(format!("{text}\r").as_bytes());
                }
            }
            Action::ToggleMonitorSilence => {
                let p = self.active_mut();
                p.monitor_silence = !p.monitor_silence;
                p.output_since_quiet = false;
                let on = p.monitor_silence;
                self.toast(if on { "I'll notify you when this pane goes quiet" } else { "silence monitor off" }, 2.0);
            }
            Action::Popup(cmd) => self.popup(&cmd),
            Action::OpenSettings(page) => {
                self.palette = None;
                self.menu = None;
                if self.settings_ui.active && page.is_none() {
                    // Ctrl+, again closes it
                    self.settings_ui.close();
                } else {
                    self.settings_ui.show(page.as_deref().and_then(settings_ui::Page::named));
                }
            }
            Action::InstallUpdate => match self.update.clone() {
                Some(feed) => {
                    self.toast(format!("downloading Vibex {}…", feed.version), 30.0);
                    let proxy = self.proxy.clone();
                    std::thread::spawn(move || {
                        let _ = proxy.send_event(AppEvent::Update(update::download(feed), true));
                    });
                }
                None => self.check_updates(true),
            },
            Action::CopyMode => {
                let p = self.active_mut();
                if p.copy.is_some() {
                    p.copy = None;
                } else {
                    p.selection = None;
                    p.copy = Some(copymode::CopyMode::new(&p.term));
                }
            }
            Action::QuickSelect => {
                let targets = self.active().term.quick_targets();
                if targets.is_empty() {
                    self.toast("nothing to quick-select on screen", 2.0);
                } else {
                    let labels = keys::hint_labels(targets.len());
                    self.quick = Some(Quick { pane: self.active_id(), targets: labels.into_iter().zip(targets).collect(), typed: String::new() });
                }
            }
            Action::PreviousPrompt | Action::NextPrompt => {
                let older = action == Action::PreviousPrompt;
                let p = self.active_mut();
                if p.term.prompt_lines().is_empty() {
                    self.toast("no prompt marks yet: enable shell integration (see docs/SHELL_INTEGRATION.md)", 4.0);
                } else {
                    p.jump_prompt(older);
                }
            }
        }
        self.window.request_redraw();
    }

    /// Past commands, loaded once per window.
    fn suggest_list(&mut self) -> &Vec<(String, String)> {
        self.suggest.get_or_insert_with(|| history::load().into_iter().take(20_000).map(|r| (r.command, r.cwd)).collect())
    }

    /// The rest of the newest past command that starts with what's typed
    /// (preferring ones run in this folder). None when there's nothing to
    /// suggest, or the shell already shows something after the cursor.
    fn suggestion(&mut self) -> Option<String> {
        if !self.config.suggestions || self.tabs.is_empty() {
            return None;
        }
        let input = self.active().term.prompt_input()?;
        if !input.at_end || input.text.trim().chars().count() < 2 || input.text.contains('\n') {
            return None;
        }
        let cwd = self.current_cwd().unwrap_or_default();
        let typed = input.text;
        let list = self.suggest_list();
        let fits = |c: &str| c.len() > typed.len() && c.starts_with(&typed) && !c.contains('\n');
        let best = list.iter().find(|(c, d)| *d == cwd && fits(c)).or_else(|| list.iter().find(|(c, _)| fits(c)))?;
        Some(best.0[typed.len()..].to_string())
    }

    /// Right / End takes the suggestion, Ctrl+Right the next word of it.
    fn accept_suggestion(&mut self, event: &KeyEvent) -> bool {
        let (ctrl, other) = (self.mods.control_key(), self.mods.alt_key() || self.mods.shift_key() || self.mods.super_key());
        let whole = matches!(event.logical_key, Key::Named(NamedKey::ArrowRight | NamedKey::End)) && !ctrl && !other;
        let word = matches!(event.logical_key, Key::Named(NamedKey::ArrowRight)) && ctrl && !other;
        if !whole && !word {
            return false;
        }
        let Some(rest) = self.suggestion() else { return false };
        let take = if whole {
            rest
        } else {
            // up to and including the next word's end
            let trimmed = rest.trim_start();
            let lead = rest.len() - trimmed.len();
            let end = trimmed.find(|c: char| c.is_whitespace() || "/\\".contains(c)).map_or(trimmed.len(), |i| i + 1);
            rest[..lead + end].to_string()
        };
        self.active_mut().selection = None;
        self.write_active(take.as_bytes());
        self.window.request_redraw();
        true
    }

    /// Ctrl+Shift+Space: commands from history that start with what's
    /// typed, then files and folders matching the last word.
    fn open_completions(&mut self) {
        let Some(input) = self.active().term.prompt_input() else {
            self.toast("completions work at a shell prompt (needs shell integration)", 3.0);
            return;
        };
        let typed = input.text.clone();
        let mut items: Vec<(MenuItem, MenuCmd)> = Vec::new();
        let list = self.suggest_list().clone();
        let mut seen = std::collections::HashSet::new();
        for (c, _) in list.iter().filter(|(c, _)| c.len() > typed.len() && c.starts_with(&typed) && !c.contains('\n')) {
            if seen.insert(c.clone()) {
                let label: String = c.chars().take(70).collect();
                items.push((
                    MenuItem { label, hint: "history".into(), separator: false, icon: Some(MenuIcon::Named("history")) },
                    MenuCmd::Type(c[typed.len()..].to_string()),
                ));
            }
            if items.len() >= 8 {
                break;
            }
        }
        // the last word as a path: list its folder
        let word = typed.rsplit(|c: char| c.is_whitespace()).next().unwrap_or("").trim_matches('"').to_string();
        let (dir_part, name_part) = match word.rfind(['/', '\\']) {
            Some(i) => (&word[..=i], &word[i + 1..]),
            None => ("", word.as_str()),
        };
        let base = self.current_cwd().map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let dir = if dir_part.is_empty() {
            base.clone()
        } else if std::path::Path::new(dir_part).is_absolute() {
            PathBuf::from(dir_part)
        } else {
            base.join(dir_part)
        };
        let mut files: Vec<(bool, String)> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let lower = name.to_lowercase();
                        (lower.starts_with(&name_part.to_lowercase()) && (!name.starts_with('.') || name_part.starts_with('.')))
                            .then(|| (e.file_type().map(|t| t.is_dir()).unwrap_or(false), name))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.to_lowercase().cmp(&b.1.to_lowercase())));
        let sep_slash = if cfg!(windows) { '\\' } else { '/' };
        for (k, (is_dir, name)) in files.into_iter().take(14).enumerate() {
            // type what's missing (case as on disk: replace the typed part)
            let mut insert = String::new();
            for _ in 0..name_part.chars().count() {
                insert.push('\u{7f}');
            }
            let quoted = if name.contains(' ') && !word.starts_with('"') { format!("\"{dir_part}{name}") } else { name.clone() };
            if name.contains(' ') && !word.starts_with('"') {
                // retype the whole word quoted
                insert = "\u{7f}".repeat(word.chars().count());
            }
            insert.push_str(&quoted);
            if is_dir {
                insert.push(sep_slash);
            } else if name.contains(' ') && !word.starts_with('"') {
                insert.push('"');
            }
            let label = if is_dir { format!("{name}{sep_slash}") } else { name };
            items.push((
                MenuItem {
                    label,
                    hint: if is_dir { "folder".into() } else { "file".into() },
                    separator: k == 0 && !seen.is_empty(),
                    icon: Some(MenuIcon::Named(if is_dir { "folder" } else { "file" })),
                },
                MenuCmd::Type(insert),
            ));
        }
        if items.is_empty() {
            self.toast("nothing to complete", 1.5);
            return;
        }
        // under the cursor
        let p = self.active();
        let r = &self.renderer;
        let (cw, ch) = (r.fonts.cell_width as f32, r.fonts.cell_height as f32);
        let x = p.rect.x + r.padding_x as f32 + p.term.cursor.x as f32 * cw;
        let y = p.rect.y + r.padding_y as f32 + (p.term.cursor.y + p.term.display_offset() + 1) as f32 * ch + 2.0;
        self.menu = Some(Menu { items, typing: true, first: 0, min_width: 0.0, selected: Some(0), x, y });
        self.palette = None;
        self.window.request_redraw();
    }

    /// A recording in a new tab, on its own clock; any key closes it.
    fn play_recording(&mut self, path: &str) {
        let cast = match record::load(std::path::Path::new(path)) {
            Ok(c) => c,
            Err(e) => {
                self.toast(format!("can't play: {e}"), 4.0);
                return;
            }
        };
        // a program that just waits for a key: closing the replay
        let waiter = if cfg!(windows) {
            let mut c = config::base_command("cmd.exe");
            c.args = vec!["/c".into(), "pause >nul".into()];
            c
        } else {
            let mut c = config::base_command("sh");
            c.args = vec!["-c".into(), "read x".into()];
            c
        };
        match self.spawn_command("replay", waiter) {
            Ok(id) => {
                if let Some(p) = self.panes.get_mut(&id) {
                    p.player = Some(record::Player::new(cast));
                }
                let mut tab = Tab::new(id);
                tab.title = Some(format!("▶ {}", record::short_name(path)));
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
                self.after_focus_change();
            }
            Err(e) => self.toast(e, 4.0),
        }
    }

    /// Ctrl+Shift+H: every command from history.tsv, newest first, plus
    /// what's in this window that isn't saved (history off).
    fn open_history(&mut self) {
        let mut recs = history::load();
        if recs.is_empty() && !self.config.command_history {
            self.toast("command history is off (Settings → Terminal)", 3.0);
            return;
        }
        if recs.is_empty() {
            self.toast("no commands yet (needs shell integration: docs/SHELL_INTEGRATION.md)", 4.0);
            return;
        }
        recs.truncate(5000);
        let now = history::now();
        let entries = recs
            .into_iter()
            .map(|r| palette::Entry { hint: history::hint(&r, now), label: r.command.replace('\n', " ⏎ "), command: palette::Command::History(r.command) })
            .collect();
        self.palette = Some(Palette::new(entries));
        self.menu = None;
        self.window.request_redraw();
    }

    fn open_palette_with(&mut self, query: &str) {
        let mut p = Palette::new(self.palette_entries());
        p.push(query);
        self.palette = Some(p);
        self.window.request_redraw();
    }

    /// Keys while a tab is being renamed. Returns true if consumed.
    fn rename_key(&mut self, event: &KeyEvent) -> bool {
        let Some((i, text)) = self.rename.as_mut() else { return false };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.rename = None,
            Key::Named(NamedKey::Enter) => {
                let (i, text) = (*i, text.trim().to_string());
                self.rename = None;
                if let Some(t) = self.tabs.get_mut(i) {
                    // an empty name goes back to following the shell's title
                    t.title = (!text.is_empty()).then_some(text);
                }
                self.session_dirty = true;
            }
            Key::Named(NamedKey::Backspace) => {
                text.pop();
            }
            _ => {
                if !(self.mods.control_key() || self.mods.alt_key()) {
                    if let Some(t) = event.text.as_ref().filter(|t| t.chars().all(|c| !c.is_control())) {
                        text.push_str(t);
                    }
                }
            }
        }
        self.relayout_tabbar();
        self.window.request_redraw();
        true
    }

    /// Keys in quick-select mode. Returns true if consumed.
    fn quick_key(&mut self, event: &KeyEvent) -> bool {
        let Some(q) = self.quick.as_mut() else { return false };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.quick = None,
            Key::Named(NamedKey::Backspace) => {
                q.typed.pop();
            }
            _ => {
                let Some(c) = event.text.as_ref().and_then(|t| t.chars().next()).filter(|c| c.is_alphabetic()) else {
                    return true;
                };
                // capital letter (or Shift) = copy and type it into the shell
                let paste = c.is_uppercase() || self.mods.shift_key();
                q.typed.extend(c.to_lowercase());
                let hit = q.targets.iter().find(|(l, _)| *l == q.typed).map(|(_, t)| t.text.clone());
                if let Some(text) = hit {
                    self.quick = None;
                    if let Some(cb) = self.clipboard.as_mut() {
                        let _ = cb.set_text(text.clone());
                    }
                    if paste {
                        self.write_active(text.as_bytes());
                    }
                    let shown: String = text.chars().take(60).collect();
                    self.toast(format!("copied {shown}"), 1.5);
                } else if !q.targets.iter().any(|(l, _)| l.starts_with(&q.typed)) {
                    q.typed.clear();
                }
            }
        }
        self.window.request_redraw();
        true
    }

    /// Keys while the palette is open. Returns true if consumed.
    fn palette_key(&mut self, event: &KeyEvent) -> bool {
        let Some(p) = self.palette.as_mut() else { return false };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.palette = None,
            Key::Named(NamedKey::ArrowUp) => p.move_selection(-1),
            Key::Named(NamedKey::ArrowDown) => p.move_selection(1),
            Key::Named(NamedKey::Tab) => p.move_selection(if self.mods.shift_key() { -1 } else { 1 }),
            Key::Named(NamedKey::Backspace) => p.pop(),
            Key::Named(NamedKey::Enter) => {
                let cmd = p.current().map(|e| e.command.clone());
                self.palette = None;
                match cmd {
                    Some(palette::Command::Action(a)) => self.perform(a),
                    Some(palette::Command::Setting(change)) => self.apply_setting(change),
                    Some(palette::Command::Copy(text)) => {
                        if let Some(cb) = self.clipboard.as_mut() {
                            let _ = cb.set_text(text);
                        }
                        self.toast("copied", 1.2);
                    }
                    Some(palette::Command::History(cmd)) => {
                        // Enter types it at the prompt; Shift+Enter runs it
                        let mut bytes = input::paste_bytes(&cmd, self.active().term.modes.bracketed_paste);
                        if self.mods.shift_key() {
                            bytes.push(b'\r');
                        }
                        self.active_mut().term.scroll_view_to_bottom();
                        self.write_active(&bytes);
                    }
                    Some(palette::Command::OpenConfig) => match self.watcher.path.clone() {
                        Some(path) if path.is_file() => {
                            if !open_path(&path) {
                                self.toast("could not open vibex.lua", 4.0);
                            }
                        }
                        Some(path) => self.toast(format!("no config file yet: {}", path.display()), 5.0),
                        None => self.toast("no config path", 3.0),
                    },
                    None => {}
                }
            }
            _ => {
                if !(self.mods.control_key() || self.mods.alt_key()) {
                    if let Some(t) = event.text.as_ref().filter(|t| t.chars().all(|c| !c.is_control())) {
                        let t = t.to_string();
                        if let Some(p) = self.palette.as_mut() {
                            p.push(&t);
                        }
                    }
                }
            }
        }
        self.window.request_redraw();
        true
    }

    /// A palette setting: apply now and save to settings.lua.
    fn apply_setting(&mut self, change: settings::Change) {
        let Some(path) = settings::path() else { return };
        if change == settings::Change::Reset {
            self.settings.values.clear();
            let _ = self.settings.save(&path);
            self.reload_config(false);
            self.toast("settings reset to vibex.lua", 2.5);
            return;
        }
        let mut cfg = self.config.clone();
        settings::change(&mut self.settings, &mut cfg, &change);
        self.apply_config(cfg);
        match self.settings.save(&path) {
            Ok(()) => self.toast("saved (settings.lua)", 1.5),
            Err(e) => self.toast(format!("couldn't save settings: {e}"), 4.0),
        }
        self.next_status = Instant::now();
    }

    /// Reload vibex.lua. On error the old config stays active and the
    /// error is shown until the file loads cleanly.
    fn reload_config(&mut self, manual: bool) {
        let result = self.watcher.reload();
        self.apply_reload(result, manual);
    }

    fn apply_reload(&mut self, result: Reload, manual: bool) {
        match result {
            Reload::Loaded(loaded) => {
                let (engine, mut cfg) = *loaded;
                self.engine = engine;
                eprintln!("vibex: config reloaded");
                self.settings.apply(&mut cfg);
                self.apply_config(cfg.with_env());
                let was_error = self.config_error.take().is_some();
                if manual || was_error {
                    self.toast("config reloaded", 2.0);
                } else {
                    self.toast("vibex.lua reloaded", 1.5);
                }
                self.emit("config-reloaded", EventArg::None);
            }
            Reload::Failed(e) => {
                eprintln!("vibex: config error: {e}");
                self.config_error = Some(e);
            }
        }
        self.next_status = Instant::now();
        self.window.request_redraw();
    }

    fn apply_config(&mut self, cfg: Config) {
        let font_changed = cfg.fonts != self.config.fonts || cfg.font_size_pt != self.config.font_size_pt || cfg.ligatures != self.config.ligatures;
        if font_changed {
            let px = font_px(cfg.font_size_pt, self.window.scale_factor());
            match FontSet::load(&cfg.fonts, px) {
                Ok(mut fonts) => {
                    fonts.ligatures = cfg.ligatures;
                    self.renderer.fonts = fonts;
                    self.base_font_px = px;
                }
                Err(e) => self.toast(format!("font: {e}"), 5.0),
            }
        }
        if cfg.decorations != self.config.decorations {
            self.toast("window.decorations changes apply to new windows", 4.0);
        }
        self.renderer.theme = cfg.theme.clone();
        set_dark_mode(&self.window, cfg.theme.background);
        self.renderer.padding_x = cfg.padding.0;
        self.renderer.padding_y = cfg.padding.1;
        self.renderer.status_bar = cfg.status_bar && !self.focus_mode;
        let engagement_changed = cfg.engagement != self.config.engagement;
        if cfg.glass != self.renderer.glass && matches!(&self.backend, Backend::Gpu(g) if g.transparent) {
            enable_backdrop(&self.window, cfg.glass);
        }
        self.renderer.glass = cfg.glass;
        if cfg.opacity != self.opacity {
            self.set_opacity(cfg.opacity);
        }
        for p in self.panes.values_mut() {
            p.term.cursor_shape = cfg.cursor_shape;
            p.term.modes.cursor_blink = cfg.cursor_blink;
            p.term.set_scrollback_limit(cfg.scrollback);
        }
        self.blink_interval = Duration::from_millis(cfg.blink_interval_ms);
        self.multi_click = Duration::from_millis(cfg.multi_click_ms);
        self.smooth_scroll = cfg.smooth_scroll;
        self.config = cfg;
        if engagement_changed {
            self.sync_engagement();
        }
        self.relayout();
    }

    fn refresh_status(&mut self) {
        self.next_status = Instant::now() + Duration::from_secs(1);
        if !self.config.status_bar {
            self.status = None;
            return;
        }
        let line = match self.engine.status(self.snapshot()) {
            Ok(Some((line, cmds))) => {
                if !cmds.is_empty() {
                    self.apply_cmds(cmds);
                }
                line
            }
            Ok(None) => default_status(&self.renderer.theme, &self.snapshot()),
            Err(e) => {
                self.lua_error(e);
                default_status(&self.renderer.theme, &self.snapshot())
            }
        };
        if self.status.as_ref() != Some(&line) {
            self.status = Some(line);
            self.window.request_redraw();
        }
    }

    /// Timers: status bar, config hot-reload, leader timeout, toast expiry.
    fn tick(&mut self) -> Option<Instant> {
        let now = Instant::now();
        let mut next: Option<Instant> = None;
        let mut at = |t: Instant| next = Some(next.map_or(t, |n: Instant| n.min(t)));

        if self.config.status_bar {
            if now >= self.next_status {
                self.refresh_status();
            }
            at(self.next_status);
        } else if self.status.is_some() {
            self.status = None;
        }
        // replays: feed what's due, wake for the next chunk
        let mut replayed = false;
        for p in self.panes.values_mut() {
            let Some(pl) = p.player.as_mut() else { continue };
            if pl.next >= pl.events.len() {
                continue;
            }
            let (text, next_at) = pl.due();
            if !text.is_empty() {
                p.feed(text.as_bytes());
                replayed = true;
            }
            match next_at {
                Some(t) => at(t),
                None => p.feed(b"\r\n\x1b[0m\x1b[2m[end of recording \xe2\x80\x94 press any key to close]\x1b[0m"),
            }
        }
        if replayed {
            self.window.request_redraw();
        }
        // "notify when quiet": 10s without output after some output
        let quiet: Vec<String> = self
            .panes
            .values_mut()
            .filter(|p| p.monitor_silence && p.output_since_quiet && now.duration_since(p.last_output) >= Duration::from_secs(10))
            .map(|p| {
                p.output_since_quiet = false;
                p.title()
            })
            .collect();
        for t in quiet {
            self.notify("Pane went quiet", &t);
        }
        if self.panes.values().any(|p| p.monitor_silence && p.output_since_quiet) {
            at(now + Duration::from_secs(1));
        }
        // the weekly look for new nano / Neovim, when Vibex stays open
        if self.saves_session && self.config.editor_updates && now >= self.next_editor_check {
            self.next_editor_check = now + Duration::from_secs(6 * 3600);
            if bundle::due() {
                self.check_editors(false);
            }
        }
        if self.watcher.path.is_some() {
            if now >= self.next_config_poll {
                self.next_config_poll = now + Duration::from_millis(500);
                if let Some(result) = self.watcher.poll() {
                    self.apply_reload(result, false);
                }
            }
            at(self.next_config_poll);
        }
        if let Some(t) = self.leader_until {
            if now >= t {
                self.leader_until = None;
                self.next_status = now;
                self.window.request_redraw();
            } else {
                at(t);
            }
        }
        if let Some((_, until)) = &self.toast {
            if now >= *until {
                self.toast = None;
                self.window.request_redraw();
            } else {
                at(*until);
            }
        }
        // crash safety: write the session a few seconds after it changes
        if self.session_dirty && !self.quit {
            if now >= self.next_session_save {
                self.next_session_save = now + Duration::from_secs(5);
                self.save_session();
            } else {
                at(self.next_session_save);
            }
        }
        next
    }

    /// Leader key and configured bindings. Returns true if consumed.
    fn binding(&mut self, event: &KeyEvent) -> bool {
        let Some(spec) = KeySpec::from_event(event) else { return false };
        // AltGr arrives as Ctrl+Alt. If it typed a character ('@' on German
        // AltGr+Q, '{' on AltGr+7), that's text for the shell, not a
        // Ctrl+Alt shortcut.
        if self.mods.control_key() && self.mods.alt_key() && altgr_text(event) {
            return false;
        }
        if self.leader_active() {
            self.leader_until = None;
            self.next_status = Instant::now();
            let mods = Mods::from_state(self.mods, true);
            if let Some(b) = keys::find(&self.config.bindings, &spec, mods) {
                let a = b.action.clone();
                self.perform(a);
            }
            self.window.request_redraw();
            return true;
        }
        let mods = Mods::from_state(self.mods, false);
        if let Some(leader) = &self.config.leader {
            if leader.key == spec && leader.mods == mods {
                self.leader_until = Some(Instant::now() + leader.timeout);
                self.next_status = Instant::now();
                self.window.request_redraw();
                return true;
            }
        }
        match keys::find(&self.config.bindings, &spec, mods).map(|b| b.action.clone()) {
            Some(Action::PassThrough) | None => false,
            Some(a) => {
                self.perform(a);
                true
            }
        }
    }

    /// Keys in copy mode. Everything is consumed except Ctrl+Shift
    /// bindings and keys meant for an open search bar.
    fn copy_key(&mut self, event: &KeyEvent) -> bool {
        use copymode::{CKey, Outcome};
        let (ctrl, shift, alt) = (self.mods.control_key(), self.mods.shift_key(), self.mods.alt_key());
        let p = self.active();
        if p.copy.is_none() || p.search.is_some() || (ctrl && shift) || alt {
            return false;
        }
        let key = match &event.logical_key {
            Key::Named(NamedKey::Escape) => CKey::Esc,
            Key::Named(NamedKey::Enter) => CKey::Enter,
            Key::Named(NamedKey::ArrowLeft) => CKey::Left,
            Key::Named(NamedKey::ArrowRight) => CKey::Right,
            Key::Named(NamedKey::ArrowUp) => CKey::Up,
            Key::Named(NamedKey::ArrowDown) => CKey::Down,
            Key::Named(NamedKey::Home) => CKey::Home,
            Key::Named(NamedKey::End) => CKey::End,
            Key::Named(NamedKey::PageUp) => CKey::PageUp,
            Key::Named(NamedKey::PageDown) => CKey::PageDown,
            Key::Character(c) if ctrl => match c.chars().next() {
                Some(ch) => CKey::Ctrl(ch.to_ascii_lowercase()),
                None => return true,
            },
            _ => match event.text.as_ref().and_then(|t| t.chars().next()) {
                Some(ch) if !ch.is_control() => CKey::Char(ch),
                _ => return true,
            },
        };
        let p = self.active_mut();
        let Some(cm) = p.copy.as_mut() else { return false };
        match cm.key(&p.term, key) {
            Outcome::Nothing => {}
            Outcome::Moved => {
                let line = cm.cursor.1;
                p.reveal_line(line);
            }
            Outcome::Copy(text) => {
                p.copy = None;
                let shown = text.chars().count();
                if let Some(cb) = self.clipboard.as_mut() {
                    let _ = cb.set_text(text);
                }
                self.toast(format!("copied {shown} characters"), 1.5);
            }
            Outcome::Exit => p.copy = None,
            Outcome::Search => p.open_search(),
            Outcome::SearchStep(older) => {
                p.copy_search_step(older);
                if let Some(line) = p.copy.as_ref().map(|c| c.cursor.1) {
                    p.reveal_line(line);
                }
            }
        }
        self.window.request_redraw();
        true
    }

    /// Keys while the search bar is open. Returns true if consumed.
    fn search_key(&mut self, event: &KeyEvent) -> bool {
        let shift = self.mods.shift_key();
        let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
        let p = self.active_mut();
        if p.search.is_none() {
            return false;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                // back in copy mode, continue from the hit (n / N keep going)
                p.copy_to_search_hit();
                if p.copy.is_some() {
                    p.last_search = p.search.take();
                } else {
                    p.search = None;
                }
            }
            Key::Character(c) if ctrl && !shift && c.eq_ignore_ascii_case("r") => {
                if let Some(s) = p.search.as_mut() {
                    s.regex = !s.regex;
                }
                p.search_changed();
            }
            Key::Character(c) if alt && !ctrl && c.eq_ignore_ascii_case("c") => {
                if let Some(s) = p.search.as_mut() {
                    s.case = !s.case;
                }
                p.search_changed();
            }
            Key::Named(NamedKey::Enter) => p.step_search(!shift),
            Key::Named(NamedKey::ArrowUp) => p.step_search(true),
            Key::Named(NamedKey::ArrowDown) => p.step_search(false),
            Key::Named(NamedKey::Backspace) => {
                if let Some(s) = p.search.as_mut() {
                    s.query.pop();
                }
                p.search_changed();
            }
            _ => {
                if ctrl || alt {
                    return false;
                }
                let Some(text) = event.text.as_ref().filter(|t| t.chars().all(|c| !c.is_control())) else {
                    return true;
                };
                if let Some(s) = p.search.as_mut() {
                    s.query.push_str(text);
                }
                p.search_changed();
            }
        }
        self.window.request_redraw();
        true
    }

    fn key(&mut self, event: KeyEvent) {
        let kitty = self.active().term.kitty_flags();
        if event.state != ElementState::Pressed {
            // releases only matter to apps that asked for them (kitty flag 2)
            let overlay = self.palette.is_some() || self.quick.is_some() || self.active().search.is_some();
            if kitty & input::kitty::REPORT_EVENTS != 0 && !overlay {
                if let Some(bytes) = input::encode_kitty(&event, self.mods, kitty).filter(|b| !b.is_empty()) {
                    self.active_mut().write(&bytes);
                }
            }
            return;
        }
        self.reset_blink();
        if self.confirm.is_some() {
            self.confirm_key(&event);
            return;
        }
        if self.notes_key(&event) {
            return;
        }
        // a finished `vibex -- cmd` pane: Enter (or Esc) closes it
        if self.active().held {
            if matches!(event.logical_key, Key::Named(NamedKey::Enter | NamedKey::Escape)) {
                self.close_pane(self.active_id());
            }
            return;
        }
        if let Some(m) = &self.menu {
            let nav = matches!(event.logical_key, Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown | NamedKey::Enter | NamedKey::Tab | NamedKey::Escape));
            if m.typing && !nav {
                // keep typing: the popup goes away, the key goes to the shell
                self.menu = None;
                self.window.request_redraw();
            } else {
                if m.typing && matches!(event.logical_key, Key::Named(NamedKey::Tab)) {
                    let i = m.selected.unwrap_or(0);
                    self.run_menu_item(i);
                    return;
                }
                self.menu_key(&event);
                return;
            }
        }
        if self.rename_key(&event)
            || self.quick_key(&event)
            || self.palette_key(&event)
            || self.settings_key(&event)
            || self.copy_key(&event)
            || self.binding(&event)
            || self.search_key(&event)
        {
            return;
        }
        if self.settings_ui.active {
            // the shell isn't on screen: don't type into it
            return;
        }
        if self.check_enter(&event) || self.accept_suggestion(&event) {
            return;
        }
        if kitty != 0 {
            if let Some(bytes) = input::encode_kitty(&event, self.mods, kitty) {
                if !bytes.is_empty() {
                    self.active_mut().selection = None;
                    self.write_active(&bytes);
                    self.window.request_redraw();
                }
                return;
            }
        }
        let app_cursor = self.active().term.modes.app_cursor;
        if let Some(bytes) = input::encode(&event, self.mods, app_cursor) {
            self.active_mut().selection = None;
            self.write_active(&bytes);
            self.window.request_redraw();
        }
    }

    /// Input-method events: CJK composition, the Win+. emoji picker.
    fn ime(&mut self, ime: winit::event::Ime) {
        use winit::event::Ime;
        match ime {
            Ime::Preedit(text, _) => {
                self.preedit = (!text.is_empty()).then_some(text);
            }
            Ime::Commit(text) => {
                self.preedit = None;
                if text.is_empty() {
                    return;
                }
                if let Some((_, name)) = self.rename.as_mut() {
                    name.push_str(&text);
                    self.relayout_tabbar();
                } else if self.settings_ui.active && self.palette.is_none() {
                    if let Some(b) = self.settings_ui.editing.as_mut() {
                        b.push_str(&text);
                    } else if self.settings_ui.page() == settings_ui::Page::Shortcuts {
                        self.settings_ui.filter.push_str(&text);
                    }
                } else if let Some(p) = self.palette.as_mut() {
                    p.push(&text);
                } else if self.active().search.is_some() {
                    let p = self.active_mut();
                    if let Some(s) = p.search.as_mut() {
                        s.query.push_str(&text);
                    }
                    p.search_changed();
                } else if self.active().notes.as_ref().is_some_and(|n| n.focused) {
                    if let Some(n) = self.active_mut().notes.as_mut() {
                        n.insert(&text);
                    }
                } else if self.quick.is_none() && !self.active().held {
                    self.active_mut().selection = None;
                    self.write_active(text.as_bytes());
                }
            }
            Ime::Enabled | Ime::Disabled => self.preedit = None,
        }
        self.reset_blink();
        self.window.request_redraw();
    }

    fn tick_blink(&mut self) -> Option<Instant> {
        let modes = &self.active().term.modes;
        let blinking = self.focused && modes.cursor_blink && modes.show_cursor;
        if !blinking {
            if !self.blink_on {
                self.blink_on = true;
                self.window.request_redraw();
            }
            return None;
        }
        let now = Instant::now();
        if now >= self.next_blink {
            self.blink_on = !self.blink_on;
            self.next_blink = now + self.blink_interval;
            self.window.request_redraw();
        }
        Some(self.next_blink)
    }

    fn animating(&self) -> bool {
        self.visible_panes().iter().any(|id| self.panes[id].scroll_px != 0.0)
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if let Some(startup) = self.startup.take() {
            self.open(el, startup);
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: AppEvent) {
        match ev {
            AppEvent::Pty => {
                for s in self.sessions.values_mut() {
                    s.drain_all();
                }
            }
            AppEvent::Open(req) => {
                // `vibex attach`, or a plain launch while something is
                // detached: bring that back instead of a new window
                let want = req.attach.clone().or_else(|| req.is_plain().then(String::new));
                let target = want.and_then(|name| {
                    self.sessions
                        .iter()
                        .filter(|(_, s)| s.detached.is_some() && (name.is_empty() || s.name == name))
                        .max_by_key(|(_, s)| s.detached)
                        .map(|(id, _)| *id)
                });
                match target {
                    Some(id) => self.show_workspace(id),
                    None if req.attach.is_some() => {
                        if let Some(s) = self.sessions.values_mut().next() {
                            s.toast("nothing is detached", 2.5);
                        }
                    }
                    None => self.open(el, startup_for(req)),
                }
            }
            AppEvent::Update(msg, manual) => self.on_update(msg, manual),
            AppEvent::Editors(msg, manual) => self.on_editors(msg, manual),
            AppEvent::Remote(args, reply) => {
                let text = self.remote(el, &args);
                let _ = reply.send(text);
            }
            AppEvent::Quake => {
                let q = self.sessions.iter().find(|(_, s)| s.quake).map(|(id, s)| (*id, s.window.is_visible() == Some(true) && s.focused));
                match q {
                    // showing and focused: tuck it away
                    Some((id, true)) => {
                        if let Some(s) = self.sessions.get_mut(&id) {
                            s.window.set_visible(false);
                        }
                    }
                    Some((id, false)) => self.show_workspace(id),
                    None => {
                        let mut st = startup_for(single::OpenRequest::default());
                        st.quake = true;
                        self.open(el, st);
                    }
                }
            }
            AppEvent::Access(ev) => {
                if let Some(s) = self.sessions.get_mut(&ev.window_id) {
                    match ev.window_event {
                        accesskit_winit::WindowEvent::InitialTreeRequested => s.update_access(),
                        accesskit_winit::WindowEvent::ActionRequested(req) => s.access_action(req),
                        accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
                    }
                }
            }
        }
        self.settle(el);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.settle(el);
        let mut wake: Option<Instant> = None;
        let mut earliest = |t: Instant| wake = Some(wake.map_or(t, |w: Instant| w.min(t)));
        for s in self.sessions.values_mut() {
            if let Some(t) = s.tick_blink() {
                earliest(t);
            }
            if let Some(t) = s.tick() {
                earliest(t);
            }
            if s.animating() {
                s.window.request_redraw();
                earliest(Instant::now() + Duration::from_millis(8));
            }
        }
        el.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(s) = self.sessions.get_mut(&id) else { return };
        if let Some(a) = s.access.as_mut() {
            a.process_event(&s.window, &event);
        }
        let font_pt = s.config.font_size_pt;
        match event {
            WindowEvent::CloseRequested => s.request_close_window(),
            WindowEvent::Resized(_) => {
                s.relayout();
                s.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                s.base_font_px = font_px(font_pt, scale_factor);
                let px = s.base_font_px;
                s.renderer.ui_scale = scale_factor as f32;
                s.set_font_px(px);
            }
            WindowEvent::Focused(f) => {
                s.focused = f;
                s.reset_blink();
                if s.active().term.modes.focus_events {
                    s.active_mut().write(if f { b"\x1b[I" } else { b"\x1b[O" });
                }
                s.emit("focus-changed", EventArg::Bool(f));
                s.window.request_redraw();
            }
            WindowEvent::Ime(ime) => s.ime(ime),
            WindowEvent::ModifiersChanged(m) => {
                s.mods = m.state();
                s.update_hover();
            }
            WindowEvent::KeyboardInput { event, .. } => s.key(event),
            WindowEvent::MouseWheel { delta, .. } => s.wheel(delta),
            WindowEvent::CursorMoved { position, .. } => s.mouse_moved(position),
            WindowEvent::CursorLeft { .. } => {
                if s.tab_hover.take().is_some() {
                    s.window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => s.mouse_button(button, state),
            WindowEvent::RedrawRequested => s.redraw(),
            _ => {}
        }
        self.settle(el);
    }
}

// ----------------------------------------------------------------- preview

const SAMPLE_LEFT: &str = "\x1b]0;~/code/vibex\x07\x1b]133;A\x07\
    \x1b[38;5;244m~/code/vibex\x1b[0m on \x1b[35mmain\x1b[0m \x1b[32m❯\x1b[0m cargo test\r\n\
    \x1b[32m   Compiling\x1b[0m vibex v0.1.0\r\n\
    \x1b[32m    Finished\x1b[0m `test` profile in 2.41s\r\n\
    test result: \x1b[32mok\x1b[0m. 140 passed; 0 failed\r\n\x1b]133;D;0\x07\
    \x1b]133;A\x07\x1b[38;5;244m~/code/vibex\x1b[0m \x1b[32m❯\x1b[0m cat missing.txt\r\n\
    cat: missing.txt: No such file or directory\r\n\x1b]133;D;1\x07\
    \x1b]133;A\x07\x1b[38;5;244m~/code/vibex\x1b[0m \x1b[31m❯\x1b[0m ";

const SAMPLE_RIGHT: &str = "\x1b]133;A\x07PS C:\\code\\vibex> git log --oneline -4 -- crates/vibex/src\r\n\
    \x1b[33m3f2a91c\x1b[0m tabs, splits and a custom title bar\r\n\
    \x1b[33m8be04d7\x1b[0m lua config with hot reload\r\n\
    \x1b[33m1c9d3e0\x1b[0m wgpu renderer, ligatures -> != =>\r\n\
    \x1b[33m77a0b12\x1b[0m own vt parser and conpty\r\n\x1b]133;D;0\x07\
    \x1b]133;A\x07PS C:\\code\\vibex> ";

/// A Settings dropdown's items; themes get their colours and are grouped
/// (Vibex's, dark, light). Also where a long list opens scrolled to.
fn dropdown_items(opts: Vec<(String, settings::Change)>, cur: usize) -> (Vec<(MenuItem, settings::Change)>, usize) {
    let group = |ch: &settings::Change| match ch {
        settings::Change::Theme(id) => {
            let t = vibex_render::Theme::builtin(id)?;
            let g = if vibex_render::Theme::OWN.contains(&id.as_str()) {
                0
            } else if t.is_light() {
                2
            } else {
                1
            };
            Some((g, [t.background, t.palette[1], t.palette[2], t.palette[4]]))
        }
        _ => None,
    };
    let mut last_group = None;
    let items: Vec<(MenuItem, settings::Change)> = opts
        .into_iter()
        .enumerate()
        .map(|(k, (label, ch))| {
            let info = group(&ch);
            let separator = info.is_some_and(|(g, _)| last_group.is_some_and(|l| l != g));
            if let Some((g, _)) = info {
                last_group = Some(g);
            }
            let icon = info.map(|(_, sw)| MenuIcon::Swatch(sw));
            (MenuItem { label, hint: if k == cur { "\u{2713}".into() } else { String::new() }, separator, icon }, ch)
        })
        .collect();
    let first = if items.len() > 12 { cur.saturating_sub(5) } else { 0 };
    (items, first)
}

/// `vibex --preview out.bmp [--palette]`: render the window (custom title
/// bar, two tabs, a split, status bar) with the loaded config, no window.
fn bar_layout_menu_rect(chrome: &Chrome) -> Option<PxRect> {
    chrome.tabbar.as_ref().and_then(|(_, l)| l.rect(TabHit::NewTabMenu))
}

fn render_preview(out: &std::path::Path, cfg: &Config, engine: &Engine, profile: &str, palette: bool, quick: bool) -> Result<(), String> {
    let mut fonts = FontSet::load(&cfg.fonts, font_px(cfg.font_size_pt, 1.25))?;
    fonts.ligatures = cfg.ligatures;
    let mut r = Renderer::new(fonts, cfg.theme.clone());
    r.padding_x = cfg.padding.0;
    r.padding_y = cfg.padding.1;
    r.status_bar = cfg.status_bar;
    r.glass = cfg.glass;
    // VIBEX_PREVIEW_SCALE=1.25 previews the title bar at 125% display scaling
    r.ui_scale = std::env::var("VIBEX_PREVIEW_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let settings_page = std::env::var("VIBEX_PREVIEW_SETTINGS").ok();
    let (w, h) = if settings_page.is_some() {
        ((cfg.window_size.0 * 1.25) as usize, (cfg.window_size.1 * 1.25) as usize)
    } else {
        ((cfg.window_size.0 * 1.25) as usize, (cfg.window_size.1 * 0.75) as usize)
    };

    let mut bar = TabBar {
        tabs: vec![
            TabInfo { title: "~/code/vibex (2)".into(), active: true, alert: false, color: None, editing: false, icon: TabIcon::Terminal },
            TabInfo { title: "wsl: ~".into(), active: false, alert: true, color: Some(0x539bf5), editing: false, icon: TabIcon::Terminal },
            TabInfo { title: "server".into(), active: false, alert: false, color: Some(0x57ab5a), editing: false, icon: TabIcon::Terminal },
        ],
        window_buttons: cfg.decorations == Decorations::Vibex,
        maximized: false,
        hover: Some(TabHit::NewTab),
        caption: None,
    };
    if settings_page.is_some() {
        bar.hover = None;
    }
    // VIBEX_PREVIEW_SSH=1: the active pane connected to a "prod" host
    let ssh_preview = std::env::var_os("VIBEX_PREVIEW_SSH").is_some();
    let mut dropdown: Option<(usize, Vec<(MenuItem, settings::Change)>, usize, usize)> = None;
    // VIBEX_PREVIEW_SUGGEST=1: a suggestion after the cursor (and =menu: the popup)
    let suggest_preview = std::env::var("VIBEX_PREVIEW_SUGGEST").ok();
    let mut popup_at: Option<(f32, f32)> = None;
    if ssh_preview {
        bar.tabs[0].title = "PROD · admin@prod-db".into();
        bar.tabs[0].color = Some(0xc42b1c);
    }
    let bar_layout = r.tabbar_layout(&bar, w);
    let content = r.content_rect(w, h, true);
    let mut root = Node::Leaf(1);
    root.split(1, 2, SplitDir::Horizontal);
    let (rects, divs) = root.layout(layout::Rect { x: content.x, y: content.y, w: content.w, h: content.h });

    let mut scene = r.begin(w, h);
    let mut snap = Snapshot::default();
    // VIBEX_PREVIEW_FLOAT=1: a floating pane over the splits
    let float = std::env::var_os("VIBEX_PREVIEW_FLOAT").is_some();
    let mut rects: Vec<(PaneId, layout::Rect)> = rects;
    if float {
        let w = (content.w * 0.72).round();
        let h = (content.h * 0.72).round();
        rects.push((3, layout::Rect { x: (content.x + (content.w - w) / 2.0).round(), y: (content.y + (content.h - h) / 2.0).round(), w, h }));
        r.draw_dividers(&mut scene, &divs.iter().map(|d| to_px(d.rect)).collect::<Vec<_>>());
    }
    for (id, rect) in rects {
        let rect = to_px(rect);
        let (cols, rows) = r.pane_grid(rect);
        let mut term = Terminal::new(cols, rows);
        term.set_cell_pixels(r.fonts.cell_width as u32, r.fonts.cell_height as u32);
        term.cursor_shape = cfg.cursor_shape;
        if id == 3 {
            r.draw_float_frame(&mut scene, rect, true);
        }
        let sample = match id {
            1 => SAMPLE_LEFT,
            2 => SAMPLE_RIGHT,
            _ => "\x1b[1;34m lazygit\x1b[0m  (popup: closes when it exits)\r\n\r\n  \x1b[32m●\x1b[0m main  \x1b[2m3 ahead\x1b[0m\r\n  \x1b[33m●\x1b[0m settings tab  \x1b[2mmodified\x1b[0m\r\n",
        };
        // VIBEX_PREVIEW_STICKY=1: a long-running command whose prompt scrolled away
        let long_sample;
        let sample = if id == 1 && std::env::var_os("VIBEX_PREVIEW_STICKY").is_some() {
            let mut s = String::from("\x1b]133;A\x07~/code/vibex \x1b[31m❯\x1b[0m \x1b]133;B\x07cargo build --release\r\n\x1b]133;C\x07");
            for i in 0..40 {
                s.push_str(&format!("   \x1b[32mCompiling\x1b[0m crate-{i} v0.{i}.0\r\n"));
            }
            long_sample = s;
            long_sample.as_str()
        } else {
            sample
        };
        vibex_vt::Parser::new().advance_bytes(&mut term, sample.as_bytes());
        let active = if float {
            id == 3
        } else if quick {
            id == 2
        } else {
            id == 1
        };
        let sticky = if std::env::var_os("VIBEX_PREVIEW_STICKY").is_some() && id == 1 {
            let top = term.top_visible_line();
            term.block_at(top).filter(|b| b.prompt < top).map(|b| vibex_render::StickyHeader {
                command: b.command.text,
                detail: "running 2m 14s".into(),
                color: cfg.theme.cursor,
            })
        } else {
            None
        };
        let mut state =
            DrawState { sticky, focused: active, cursor_visible: true, dim: !active && cfg.dim_inactive_panes, marks: cfg.shell_marks, ..Default::default() };
        if quick && active {
            let targets = term.quick_targets();
            for (label, t) in keys::hint_labels(targets.len()).into_iter().zip(targets) {
                state.highlights.push(vibex_render::Highlight {
                    range: vibex_render::VisibleSelection { start: (t.start, t.line), end: (t.end, t.line), block: false },
                    bg: cfg.theme.search_match,
                    fg: None,
                });
                state.labels.push((t.start, t.line, label));
            }
        }
        let mut state = state;
        if active && suggest_preview.is_some() {
            if term.cursor.x > 3 {
                state.ghost = Some("cargo test --workspace".into());
            }
            let (cw, ch) = (r.fonts.cell_width as f32, r.fonts.cell_height as f32);
            popup_at = Some((rect.x + r.padding_x as f32 + term.cursor.x as f32 * cw, rect.y + r.padding_y as f32 + (term.cursor.y + 1) as f32 * ch + 2.0));
        }
        r.draw_pane(&mut scene, &term, &state, rect);
        if ssh_preview && active {
            r.draw_pane_border(&mut scene, rect, 0xc42b1c);
        }
        if active {
            snap = Snapshot {
                title: term.title().to_string(),
                profile: profile.to_string(),
                cols,
                rows,
                focused: true,
                font_size: cfg.font_size_pt,
                opacity: cfg.opacity,
                tab_count: 2,
                tab_index: 1,
                pane_count: 2,
                last_exit: term.last_exit(),
                ..Default::default()
            };
        }
    }
    let status = match engine.status(snap.clone())? {
        Some((line, _)) => line,
        None => default_status(&cfg.theme, &snap),
    };
    let mut chrome = Chrome {
        tabbar: Some((bar, bar_layout)),
        dividers: if float { Vec::new() } else { divs.iter().map(|d| to_px(d.rect)).collect() },
        status: Some(status),
        ..Default::default()
    };
    if let Some(page) = &settings_page {
        let mut ui = settings_ui::SettingsUi::default();
        ui.show(settings_ui::Page::named(page));
        let st = settings::path().map(|p| settings::Settings::load(&p)).unwrap_or_default();
        let about = settings_ui::AboutInfo {
            version: version(),
            renderer: "GPU: (preview)".into(),
            host: "OpenConsole (images pass through)".into(),
            config_path: config::config_file().map_or("(none)".into(), |p| p.display().to_string()),
            settings_path: settings::path().map_or("(none)".into(), |p| p.display().to_string()),
            openconsole: true,
            editors: "nano 9.3-1 \u{b7} Neovim 0.12.6".into(),
            editors_status: "nano 9.3-1 and Neovim 0.12.6 are up to date".into(),
            editors_available: std::env::var("VIBEX_PREVIEW_EDITORS").ok(),
            ..Default::default()
        };
        if let Ok(f) = std::env::var("VIBEX_PREVIEW_FOCUS") {
            ui.view(cfg, &st, &about);
            ui.focus = f.parse().ok();
            if std::env::var_os("VIBEX_PREVIEW_CAPTURE").is_some() {
                ui.capturing = true;
            }
        }
        if let Ok(q) = std::env::var("VIBEX_PREVIEW_FILTER") {
            ui.filter = q;
        }
        let view = ui.view(cfg, &st, &about);
        // VIBEX_PREVIEW_DROPDOWN=<row>: that row's dropdown, open
        if let Some(i) = std::env::var("VIBEX_PREVIEW_DROPDOWN").ok().and_then(|v| v.parse::<usize>().ok()) {
            if let Some((opts, cur)) = ui.options(i, cfg) {
                let (items, first) = dropdown_items(opts, cur);
                dropdown = Some((i, items, cur, first));
            }
        }
        chrome.settings = Some((view, settings_dialog_rect(content, r.ui_scale), content));
    }
    if let (Some("menu"), Some((x, y))) = (suggest_preview.as_deref(), popup_at) {
        let mut items: Vec<MenuItem> = ["cargo test --workspace", "cargo build --release -p vibex", "cargo clippy --all-targets"]
            .iter()
            .map(|c| MenuItem { label: (*c).into(), hint: "history".into(), separator: false, icon: None })
            .collect();
        for (i, (n, k)) in [("crates\\", "folder"), ("docs\\", "folder"), ("Cargo.toml", "file"), ("Cargo.lock", "file")].iter().enumerate() {
            items.push(MenuItem { label: (*n).into(), hint: (*k).into(), separator: i == 0, icon: None });
        }
        chrome.menu = Some(MenuView { items, x, y, selected: Some(0), ..Default::default() });
    }
    // VIBEX_PREVIEW_CONTEXT=1: the right-click menu over the left pane
    if std::env::var_os("VIBEX_PREVIEW_CONTEXT").is_some() {
        let hint = |a: &Action| cfg.bindings.iter().find(|b| &b.action == a).map(|b| b.describe()).unwrap_or_default();
        let mut items = vec![
            MenuItem {
                label: "Open src/main.rs at line 12".into(),
                hint: "Ctrl+click".into(),
                separator: false,
                icon: menu_icon("Open src/main.rs at line 12"),
            },
            MenuItem { label: "Copy path".into(), hint: String::new(), separator: false, icon: menu_icon("Copy path") },
        ];
        for (label, a, sep) in [
            ("Copy", Action::Copy, true),
            ("Paste", Action::Paste, false),
            ("Select all", Action::SelectAll, false),
            ("Find", Action::Search, false),
            ("Split right", Action::SplitRight(None), true),
            ("Split down", Action::SplitDown(None), false),
            ("New tab", Action::NewTab(None), false),
            ("Floating pane", Action::ToggleFloatingPane, false),
            ("Copy the last output", Action::CopyLastOutput, true),
            ("Run the last command again", Action::RerunLastCommand, false),
            ("Clear scrollback", Action::ClearScrollback, true),
            ("Command palette", Action::CommandPalette, false),
            ("Settings", Action::OpenSettings(None), false),
        ] {
            let icon = if matches!(a, Action::OpenSettings(_)) { Some(MenuIcon::Settings) } else { menu_icon(label) };
            items.push(MenuItem { label: label.into(), hint: hint(&a), separator: sep, icon });
        }
        chrome.menu = Some(MenuView { items, x: content.x + 260.0, y: content.y + 90.0, selected: Some(4), ..Default::default() });
    }
    if std::env::var_os("VIBEX_PREVIEW_MENU").is_some() {
        if let Some(rr) = bar_layout_menu_rect(&chrome) {
            let mut items: Vec<MenuItem> = cfg
                .profiles
                .iter()
                .map(|p| MenuItem { label: p.name.clone(), hint: String::new(), separator: false, icon: Some(MenuIcon::Terminal) })
                .collect();
            items.push(MenuItem { label: "Settings".into(), hint: "Ctrl+,".into(), separator: true, icon: Some(MenuIcon::Settings) });
            items.push(MenuItem { label: "Command palette".into(), hint: "Ctrl+Shift+P".into(), separator: false, icon: None });
            items.push(MenuItem { label: "About Vibex".into(), hint: String::new(), separator: false, icon: None });
            chrome.menu = Some(MenuView { items, x: rr.x, y: rr.y + rr.h + 4.0, selected: Some(1), ..Default::default() });
        }
    }
    if palette {
        let mut p = Palette::new(palette::entries(cfg));
        p.push(&std::env::var("VIBEX_PALETTE_QUERY").unwrap_or_else(|_| "split".into()));
        chrome.palette = Some(p.view());
    }
    let layout = r.draw_chrome(&mut scene, &chrome);
    if let Some((i, items, cur, first)) = dropdown {
        let rect = layout.settings.as_ref().and_then(|l| l.regions.iter().find(|(_, h)| *h == SettingsHit::Control(i)).map(|(r, _)| *r));
        if let Some(rect) = rect {
            let items = items.into_iter().map(|(m, _)| m).collect();
            let menu = Chrome {
                menu: Some(MenuView { items, x: rect.x, y: rect.y + rect.h + 4.0, selected: Some(cur), first, min_width: rect.w }),
                ..Default::default()
            };
            r.draw_chrome(&mut scene, &menu);
        }
    }
    let mut px = vec![0u32; w * h];
    vibex_render::cpu::draw(&scene, &mut r.fonts, &mut Frame { pixels: &mut px, width: w, height: h });
    vibex_render::write_bmp(out, &px, w, h).map_err(|e| e.to_string())
}

/// `vibex icons sheet OUT.bmp [--size N] [--dark]` | `vibex icons svg DIR` | `vibex icons list`
fn run_icons(args: Vec<String>) -> i32 {
    use vibex_render::icons;
    let mut it = args.into_iter();
    let cmd = it.next().unwrap_or_default();
    let rest: Vec<String> = it.collect();
    let size = rest.iter().position(|a| a == "--size").and_then(|i| rest.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(48u32);
    let dark = rest.iter().any(|a| a == "--dark");
    let target = rest.iter().find(|a| !a.starts_with("--") && a.parse::<u32>().is_err()).cloned();
    match cmd.as_str() {
        "list" => {
            for (n, _) in icons::ICONS {
                println!("{n}");
            }
            0
        }
        "sheet" => {
            let Some(out) = target else {
                eprintln!("usage: vibex icons sheet OUT.bmp [--size N] [--dark]");
                return 2;
            };
            let (ink, paper) = if dark { (0xe6e9ef, 0x14161c) } else { (0x111111, 0xf5f5f7) };
            let (px, w, h) = icons::sheet(size, 9, ink, paper);
            match vibex_render::write_bmp(std::path::Path::new(&out), &px, w, h) {
                Ok(()) => {
                    println!("wrote {out} ({} icons at {size}px)", icons::ICONS.len());
                    0
                }
                Err(e) => {
                    eprintln!("vibex icons: {e}");
                    1
                }
            }
        }
        "svg" => {
            let dir = PathBuf::from(target.unwrap_or_else(|| "vibex-icons".into()));
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("vibex icons: {e}");
                return 1;
            }
            for (n, _) in icons::ICONS {
                if let Some(svg) = icons::svg(n) {
                    let _ = std::fs::write(dir.join(format!("{n}.svg")), svg);
                }
            }
            println!("wrote {} SVGs to {}", icons::ICONS.len(), dir.display());
            0
        }
        _ => {
            eprintln!("usage: vibex icons list | sheet OUT.bmp [--size N] [--dark] | svg DIR");
            2
        }
    }
}

const AUDIT_USAGE: &str = "usage: vibex audit verify [FILE]
       vibex audit report [FILE] [--out OUT.html] [--since YYYY-MM-DD] [--open]

  The audit log (Settings → Terminal → Audit log) is ~/.config/vibex/audit.jsonl:
  every command with where and when, each line chained to the one before by
  SHA-256. verify checks the chain (exit 0 = intact, 1 = broken); report writes
  an HTML report.";

/// "2026-11-12" → unix seconds at midnight UTC.
fn parse_date(s: &str) -> Option<u64> {
    let mut it = s.split('-');
    let (y, m, d): (i64, i64, i64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400).ok()
}

/// `vibex editors [check|update]`: the bundled nano and Neovim.
fn run_editors(args: Vec<String>) -> i32 {
    let pins = bundle::installed();
    let Some(dir) = bundle::tools_dir().filter(|_| !pins.is_empty()) else {
        eprintln!("vibex: nano and Neovim aren't bundled with this Vibex");
        return 1;
    };
    match args.first().map(String::as_str) {
        None | Some("list") => {
            for p in &pins {
                if p.name == "nvim" || p.name == "nano" {
                    println!("{:<6} {}", if p.name == "nvim" { "Neovim" } else { "nano" }, p.version);
                }
            }
            println!("in {}", dir.display());
            0
        }
        Some(c @ ("check" | "update")) => match bundle::check() {
            bundle::Msg::Available(f) if c == "update" => {
                println!("updating: {}", f.describe());
                match bundle::apply(&f) {
                    bundle::Msg::Done(t) => {
                        println!("{t}");
                        0
                    }
                    bundle::Msg::Failed(e) | bundle::Msg::UpToDate(e) => {
                        eprintln!("vibex: {e}");
                        1
                    }
                    bundle::Msg::Available(_) => 1,
                }
            }
            bundle::Msg::Available(f) => {
                println!("new: {}\nrun `vibex editors update` (or palette \u{2192} Update nano and Neovim)", f.describe());
                0
            }
            bundle::Msg::UpToDate(t) | bundle::Msg::Done(t) => {
                println!("{t}");
                0
            }
            bundle::Msg::Failed(e) => {
                eprintln!("vibex: {e}");
                1
            }
        },
        Some(other) => {
            eprintln!("usage: vibex editors [list|check|update]   (not '{other}')");
            2
        }
    }
}

fn run_audit(args: Vec<String>) -> i32 {
    let mut it = args.into_iter();
    let cmd = it.next().unwrap_or_default();
    let (mut file, mut out, mut since, mut open) = (None::<PathBuf>, None::<PathBuf>, 0u64, false);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" | "-o" => out = it.next().map(PathBuf::from),
            "--since" => match it.next().as_deref().and_then(parse_date) {
                Some(t) => since = t,
                None => {
                    eprintln!("vibex audit: --since wants YYYY-MM-DD");
                    return 2;
                }
            },
            "--open" => open = true,
            _ => file = Some(PathBuf::from(a)),
        }
    }
    let Some(file) = file.or_else(audit::path) else {
        eprintln!("vibex audit: no config folder");
        return 2;
    };
    match cmd.as_str() {
        "verify" => match audit::verify(&file) {
            Ok(audit::Verdict::Intact { entries, first, last }) => {
                println!("intact: {}, {} to {}", audit::entries(entries), audit::iso(first), audit::iso(last));
                0
            }
            Ok(audit::Verdict::Broken { line, reason, good }) => {
                println!("BROKEN at line {line}: {reason}\n{} before it intact; nothing from line {line} on can be trusted", audit::entries(good));
                1
            }
            Err(e) => {
                eprintln!("vibex audit: {e}");
                2
            }
        },
        "report" => {
            let text = match std::fs::read_to_string(&file) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("vibex audit: {}: {e}", file.display());
                    return 2;
                }
            };
            let out = out.unwrap_or_else(|| file.with_extension("html"));
            match std::fs::write(&out, audit::report(&text, since)) {
                Ok(()) => {
                    println!("wrote {}", out.display());
                    if open {
                        open_path(&out);
                    }
                    0
                }
                Err(e) => {
                    eprintln!("vibex audit: {}: {e}", out.display());
                    1
                }
            }
        }
        _ => {
            eprintln!("{AUDIT_USAGE}");
            2
        }
    }
}

const CAST2GIF_USAGE: &str = "usage: vibex cast2gif IN.cast [OUT.gif] [--fps N] [--speed N] [--idle SECONDS] [--open]

  Renders an asciicast recording (Alt+Shift+R in Vibex, or asciinema's) to a
  GIF with your font and theme. --fps 12, --speed 1, --idle 2 (long pauses
  shrink to this) by default; --open shows it when done.";

fn run_cast2gif(args: Vec<String>) -> i32 {
    let mut files = Vec::new();
    let (mut fps, mut speed, mut idle, mut open) = (12.0f64, 1.0f64, 2.0f64, false);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let num = |v: Option<String>| v.and_then(|v| v.parse::<f64>().ok());
        match a.as_str() {
            "--fps" => fps = num(it.next()).unwrap_or(fps).clamp(1.0, 50.0),
            "--speed" => speed = num(it.next()).unwrap_or(speed).clamp(0.1, 20.0),
            "--idle" => idle = num(it.next()).unwrap_or(idle).max(0.1),
            "--open" => open = true,
            "-h" | "--help" => {
                println!("{CAST2GIF_USAGE}");
                return 0;
            }
            _ => files.push(PathBuf::from(a)),
        }
    }
    let Some(input) = files.first().cloned() else {
        eprintln!("{CAST2GIF_USAGE}");
        return 2;
    };
    let output = files.get(1).cloned().unwrap_or_else(|| input.with_extension("gif"));
    let cfg = match Engine::load(config::config_file().as_deref()) {
        Ok((_, mut c)) => {
            settings::path().map(|p| settings::Settings::load(&p)).unwrap_or_default().apply(&mut c);
            c
        }
        Err(_) => Config::default(),
    };
    match cast_to_gif(&input, &output, &cfg, fps, speed, idle) {
        Ok((frames, secs)) => {
            println!("wrote {} ({frames} frames, {secs:.1}s)", output.display());
            if open {
                open_path(&output);
            }
            0
        }
        Err(e) => {
            eprintln!("vibex cast2gif: {e}");
            1
        }
    }
}

/// Play the recording into a terminal of its own size and draw frames with
/// the CPU renderer, keeping only frames that changed.
fn cast_to_gif(input: &std::path::Path, output: &std::path::Path, cfg: &Config, fps: f64, speed: f64, idle: f64) -> Result<(usize, f64), String> {
    let mut cast = record::load(input)?;
    record::retime(&mut cast.events, idle, speed);
    let mut fonts = FontSet::load(&cfg.fonts, font_px(cfg.font_size_pt, 1.0))?;
    fonts.ligatures = cfg.ligatures;
    let mut r = Renderer::new(fonts, cfg.theme.clone());
    r.padding_x = cfg.padding.0;
    r.padding_y = cfg.padding.1;
    let (cols, rows) = (cast.width.clamp(10, 400), cast.height.clamp(3, 200));
    let w = cols * r.fonts.cell_width + 2 * r.padding_x;
    let h = rows * r.fonts.cell_height + 2 * r.padding_y;
    if w > 65_535 || h > 65_535 {
        return Err("too big for a GIF".into());
    }
    let state = vibex_render::DrawState { focused: true, cursor_visible: true, ..Default::default() };
    let rect = PxRect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 };
    let render = |term: &Terminal, r: &mut Renderer| {
        let mut scene = r.begin(w, h);
        r.draw_pane(&mut scene, term, &state, rect);
        let mut px = vec![0u32; w * h];
        vibex_render::cpu::draw(&scene, &mut r.fonts, &mut Frame { pixels: &mut px, width: w, height: h });
        px
    };
    // Walk the timeline, calling `f` with each frame that differs from the
    // last (and when it starts). Run twice: colours first, then the GIF,
    // so no frame has to be kept.
    let end = cast.events.last().map_or(0.0, |e| e.0) + 1.5;
    let step = 1.0 / fps;
    let events = cast.events;
    let mut walk = |f: &mut dyn FnMut(f64, &[u32])| {
        let mut term = Terminal::new(cols, rows);
        let mut parser = vibex_vt::Parser::new();
        let mut last = render(&term, &mut r);
        f(0.0, &last);
        let (mut next, mut t) = (0, step);
        while next < events.len() {
            let mut fed = false;
            while let Some((at, data)) = events.get(next) {
                if *at > t {
                    break;
                }
                parser.advance_bytes(&mut term, data.as_bytes());
                next += 1;
                fed = true;
            }
            if fed {
                let px = render(&term, &mut r);
                if px != last {
                    f(t, &px);
                    last = px;
                }
            }
            t += step;
        }
    };
    let mut counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut seen = 0usize;
    walk(&mut |_, px| {
        seen += 1;
        for &c in px.iter().step_by(5) {
            *counts.entry(c & 0xff_ffff).or_default() += 1;
        }
    });
    let mut all: Vec<(u32, u32)> = counts.into_iter().collect();
    all.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let palette: Vec<u32> = all.into_iter().take(256).map(|(c, _)| c).collect();
    let mut gif = record::Gif::new(w, h, palette);
    let mut frames = 0usize;
    // each frame is written once the next one says how long it showed
    let mut pending: Option<(f64, Vec<u32>)> = None;
    walk(&mut |at, px| {
        if let Some((start, pf)) = pending.take() {
            let delay = ((at - start) * 100.0).round().clamp(2.0, 65_535.0) as u16;
            if gif.frame(&pf, delay) {
                frames += 1;
            }
        }
        pending = Some((at, px.to_vec()));
    });
    if let Some((_, pf)) = pending {
        if gif.frame(&pf, 250) {
            frames += 1;
        }
    }
    let _ = seen;
    std::fs::write(output, gif.finish()).map_err(|e| format!("{}: {e}", output.display()))?;
    Ok((frames, end))
}

const USAGE: &str = "usage: vibex [OPTIONS] [-- PROGRAM ARGS]
       vibex COMMAND ...

Commands:
  cli ...          drive the running Vibex (needs remote control): vibex cli help
  quake            show or hide the dropdown (starts Vibex if needed); bind it to a key
  show FILE...     draw PNG images and Markdown in the terminal
  audit verify|report   check / report the hash-chained command log
  editors [check|update]  the bundled nano and Neovim: versions, look for / install new ones
  cast2gif IN [OUT]     turn a recording into a GIF
  attach [NAME]    bring back a detached workspace (the latest if no NAME)

Options:
  --profile NAME   start with a profile from vibex.lua (default: config.default_profile)
  --cwd DIR        start the first tab in DIR
  -- PROGRAM ARGS  run PROGRAM in the first tab instead of a shell (stays open until Enter)
  --no-restore     don't reopen the last session
  --quake          start with the dropdown window
  --new-process    open in a new process instead of the running Vibex
  --layout NAME    open ~/.config/vibex/layouts/NAME.lua as a new workspace
  --config PATH    use this config file instead of ~/.config/vibex/vibex.lua
  --check-config   load the config, report errors, and exit
  --preview FILE   render a sample window with the config to FILE (.bmp), no window
  --palette        (with --preview) also draw the command palette";

fn version() -> String {
    let hash = env!("VIBEX_GIT_HASH");
    if hash.is_empty() {
        format!("vibex {}", env!("CARGO_PKG_VERSION"))
    } else {
        format!("vibex {} ({hash})", env!("CARGO_PKG_VERSION"))
    }
}

/// Release builds are GUI-subsystem (no console window). When started with
/// command-line options from a terminal, borrow that terminal's console so
/// `--help`, `--version` and `--check-config` can print.
fn attach_parent_console() {
    #[cfg(all(windows, not(debug_assertions)))]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    if std::env::args().len() > 1 {
        attach_parent_console();
    }
    if std::env::args().nth(1).as_deref() == Some("cli") {
        std::process::exit(run_cli(std::env::args().skip(2).collect()));
    }
    if std::env::args().nth(1).as_deref() == Some("quake") {
        std::process::exit(run_quake());
    }
    if std::env::args().nth(1).as_deref() == Some("icons") {
        std::process::exit(run_icons(std::env::args().skip(2).collect()));
    }
    if std::env::args().nth(1).as_deref() == Some("show") {
        std::process::exit(show::run(std::env::args().skip(2).collect()));
    }
    if std::env::args().nth(1).as_deref() == Some("audit") {
        std::process::exit(run_audit(std::env::args().skip(2).collect()));
    }
    if std::env::args().nth(1).as_deref() == Some("cast2gif") {
        std::process::exit(run_cast2gif(std::env::args().skip(2).collect()));
    }
    if std::env::args().nth(1).as_deref() == Some("editors") {
        std::process::exit(run_editors(std::env::args().skip(2).collect()));
    }
    let mut profile: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut exec: Option<Vec<String>> = None;
    let mut no_restore = false;
    let mut quake_first = false;
    let mut new_process = false;
    let mut layout: Option<String> = None;
    let mut attach: Option<String> = None;
    let mut explicit_config = false;
    let mut config_path = config::config_file();
    let mut check_only = false;
    let mut preview_out: Option<PathBuf> = None;
    let mut preview_palette = false;
    let mut preview_quick = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--profile" | "-p" => profile = args.next(),
            "--cwd" | "-d" => {
                // Explorer passes e.g. `C:\` or `C:\Users\x` (strip stray quotes)
                cwd = args.next().map(|d| d.trim_matches('"').trim_end_matches('\\').to_string()).map(|d| {
                    if d.len() == 2 && d.ends_with(':') {
                        format!("{d}\\")
                    } else {
                        d
                    }
                });
            }
            "--config" | "-c" => {
                config_path = args.next().map(PathBuf::from);
                explicit_config = true;
            }
            "--check-config" => check_only = true,
            "--preview" => preview_out = args.next().map(PathBuf::from),
            "--palette" => preview_palette = true,
            "--quick" => preview_quick = true,
            "--no-restore" => no_restore = true,
            "--quake" => quake_first = true,
            "--new-process" => new_process = true,
            "--layout" | "-l" => layout = args.next(),
            "attach" => attach = Some(args.next().unwrap_or_default()),
            "--" => {
                exec = Some(args.by_ref().collect());
                break;
            }
            "--version" | "-V" => {
                println!("{}", version());
                return;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return;
            }
            other => {
                eprintln!("vibex: unknown argument '{other}'\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    // a Vibex is already running: it opens the window, this process exits
    let forwardable = !new_process && !explicit_config && !check_only && preview_out.is_none();
    // hold the single-instance name from here, before the slow config load,
    // so launches during startup wait for this process instead of becoming
    // a second Vibex
    let mut claim = None;
    if forwardable {
        let exec = exec.clone().filter(|a| !a.is_empty());
        // `vibex -- cmd` runs where it was typed
        let cwd = cwd.clone().or_else(|| exec.as_ref().and_then(|_| std::env::current_dir().ok()).map(|d| d.to_string_lossy().into_owned()));
        let req = single::OpenRequest { profile: profile.clone(), cwd, exec, config: None, layout: layout.clone(), attach: attach.clone() };
        // `--quake` asks a running Vibex for its dropdown, not a new window
        let hand_off = || if quake_first { single::send(&single::Request::Quake).is_some() } else { single::forward(&req) };
        if hand_off() {
            return;
        }
        claim = single::claim();
        // another launch took the name between our check and the claim
        if claim.is_none() && hand_off() {
            return;
        }
    }

    // vibex.lua, then the palette's settings.lua, then env overrides
    let settings = settings::path().map(|p| settings::Settings::load(&p)).unwrap_or_default();
    let (engine, mut config, error) = match Engine::load(config_path.as_deref()) {
        Ok((engine, cfg)) => (engine, cfg, None),
        Err(e) => (Engine::empty(), Config::default(), Some(e)),
    };
    settings.apply(&mut config);
    let config = config.with_env();
    if check_only {
        match &error {
            None => {
                let names: Vec<_> = config.profiles.iter().map(|p| p.name.as_str()).collect();
                println!(
                    "config ok: {} ({} bindings, profiles: {}, default: {})",
                    config_path.as_ref().map_or("<defaults>".into(), |p| p.display().to_string()),
                    config.bindings.len(),
                    names.join(", "),
                    config.default_profile
                );
                return;
            }
            Some(e) => {
                eprintln!("config error: {e}");
                std::process::exit(1);
            }
        }
    }
    let mut error = error;
    let explicit_profile = profile.is_some();
    let profile = match profile {
        Some(p) if config.profile(&p).is_some() => p,
        Some(p) => {
            error = Some(format!("--profile: no profile named '{p}'; using '{}'", config.default_profile));
            config.default_profile.clone()
        }
        None => config.default_profile.clone(),
    };
    if let Some(out) = preview_out {
        if let Some(e) = &error {
            eprintln!("config error: {e}");
            std::process::exit(1);
        }
        match render_preview(&out, &config, &engine, &profile, preview_palette, preview_quick) {
            Ok(()) => println!("wrote {}", out.display()),
            Err(e) => {
                eprintln!("preview failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let event_loop = match EventLoop::<AppEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("vibex: cannot create event loop: {e}");
            std::process::exit(1);
        }
    };
    let exec = exec.filter(|a| !a.is_empty());
    // reopen the last session unless asked for something specific
    let restore = (config.restore_session && !no_restore && exec.is_none() && cwd.is_none() && !explicit_profile)
        .then(session::path)
        .flatten()
        .filter(|p| p.is_file())
        .and_then(|p| match session::load(&p) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("vibex: ignoring session.lua: {e}");
                None
            }
        });
    let mut startup = Startup { config, engine, config_path, error, profile, cwd, exec, restore, settings, name: None, quake: quake_first, adopt: None };
    if quake_first {
        startup.restore = None;
    }
    if let Some(l) = layout {
        let from_layout = startup_for(single::OpenRequest { layout: Some(l), ..Default::default() });
        if from_layout.restore.is_some() {
            startup.restore = from_layout.restore;
            startup.name = from_layout.name;
        } else {
            startup.error = from_layout.error;
        }
    }
    let proxy = event_loop.create_proxy();
    if let Some(claim) = claim {
        let p = proxy.clone();
        // become the process later launches hand their windows to
        claim.serve(move |req| match req {
            single::Request::Open(r) => {
                let _ = p.send_event(AppEvent::Open(r));
                "ok".into()
            }
            single::Request::Quake => {
                let _ = p.send_event(AppEvent::Quake);
                "ok".into()
            }
            single::Request::Cli(args) => {
                let (tx, rx) = std::sync::mpsc::channel();
                if p.send_event(AppEvent::Remote(args, tx)).is_err() {
                    return "error: Vibex is shutting down".into();
                }
                match rx.recv_timeout(Duration::from_secs(10)) {
                    // "ok\n" marks success, so an empty reply means no answer
                    Ok(text) if !text.starts_with("error:") => format!("ok\n{text}"),
                    Ok(text) => text,
                    Err(_) => "error: Vibex didn't answer".into(),
                }
            }
        });
    }
    // the Quake-style dropdown's global hotkey (first instance only)
    let mut notice = None;
    // elsewhere there is no global hotkey: `vibex quake` is bound in the desktop
    if forwardable && cfg!(windows) {
        if let Some((key, mods, _)) = startup.config.quake.clone() {
            if let Ok((m, vk)) = hotkey::parse(&key, &mods) {
                let p = proxy.clone();
                if let Err(e) = hotkey::register(m, vk, move || {
                    let _ = p.send_event(AppEvent::Quake);
                }) {
                    let msg = format!(
                        "dropdown hotkey {mods}+{key} is in use by another program\n(Windows Terminal's Quake mode uses it). Pick another: config.quake = {{ key = \"F12\", mods = \"WIN\" }}"
                    );
                    eprintln!("vibex: {msg} ({e})");
                    notice = Some(msg);
                }
            }
        }
    }
    let mut app = App { proxy, startup: Some(startup), sessions: HashMap::new(), notice };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("vibex: {e}");
    }
}
