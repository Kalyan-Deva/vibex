//! A pane: one terminal + its shell, with per-pane selection, search,
//! scroll animation and mouse-report state.

use crate::layout::PaneId;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;
use vibex_pty::{Command, Pty, PtyEvent, Waker};
use vibex_render::{DrawState, Highlight, PxRect, Theme, VisibleSelection};
use vibex_term::{Link, Match, Terminal};
use vibex_vt::Parser;

/// Selection anchored in absolute (col, line) coordinates so it survives
/// scrolling.
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub head: (usize, usize),
}

impl Selection {
    pub fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        let (a, h) = (self.anchor, self.head);
        if (a.1, a.0) <= (h.1, h.0) {
            (a, h)
        } else {
            (h, a)
        }
    }
}

pub struct Search {
    pub query: String,
    /// Treat the query as a regular expression (Ctrl+R in the find bar).
    pub regex: bool,
    /// Match case exactly (Alt+C); otherwise case-insensitive.
    pub case: bool,
    /// The query isn't a valid regex.
    pub error: bool,
    pub matches: Vec<Match>,
    /// Index into `matches` of the focused hit.
    pub current: Option<usize>,
    pub dirty: bool,
}

/// What happened while draining a pane's output.
#[derive(Default)]
pub struct Drained {
    pub output: bool,
    pub title: Option<String>,
    pub bell: bool,
    pub exited: bool,
    /// The screen may look different (output that changed something).
    pub changed: bool,
    /// OSC 52 writes.
    pub clipboard: Vec<String>,
    /// OSC 52 read requests.
    pub clipboard_reads: usize,
    /// OSC 9 / 777: (title, body).
    pub notifications: Vec<(String, String)>,
}

pub struct Pane {
    pub term: Terminal,
    parser: Parser,
    pty: Pty,
    rx: Receiver<PtyEvent>,
    pub profile: String,
    pub exited: bool,
    pub rect: PxRect,
    pub selection: Option<Selection>,
    pub search: Option<Search>,
    /// The search closed from copy mode (for n / N).
    pub last_search: Option<Search>,
    /// Vi-style copy mode, when active.
    pub copy: Option<crate::copymode::CopyMode>,
    pub hover_link: Option<Link>,
    pub scroll_px: f32,
    pub scroll_accum: f64,
    pub last_frame: Instant,
    pub buttons_down: Option<u8>,
    pub last_report_cell: Option<(usize, usize)>,
    /// Output arrived while this pane's tab was in the background.
    pub activity: bool,
    /// Keep the pane open after the program exits (`vibex -- cmd`), so its
    /// output can be read; Enter closes it.
    pub hold: bool,
    /// The program exited and the pane is waiting to be closed.
    pub held: bool,
    /// Trigger scanning: next line to look at (stable numbering, see
    /// `Terminal::line_base`), highlights made, last time each fired.
    pub trigger_next: u64,
    pub trigger_epoch: u64,
    pub trigger_marks: std::collections::VecDeque<(u64, usize, usize, u32)>,
    pub trigger_fired: std::collections::HashMap<usize, Instant>,
    /// "Notify when quiet": armed, and whether output came since.
    pub monitor_silence: bool,
    /// Loot scanning: next stable line, the host the output is about, and
    /// the wrap epoch it was counted in.
    pub loot_next: u64,
    pub loot_host: String,
    pub loot_epoch: u64,
    /// The notes panel beside this pane.
    pub notes: Option<crate::notes::Notes>,
    /// Recording this pane's output.
    pub recorder: Option<crate::record::Recorder>,
    /// Playing a recording here (the pane's own program is ignored).
    pub player: Option<crate::record::Player>,
    /// Connected over SSH to this host (from the picker, or a typed `ssh`).
    pub ssh_host: Option<String>,
    /// Opened by the SSH picker: the whole pane is the connection.
    pub ssh_fixed: bool,
    pub output_since_quiet: bool,
    pub last_output: Instant,
}

pub struct PaneOptions {
    pub scrollback: usize,
    pub cursor_blink: bool,
    pub cursor_shape: vibex_term::CursorShape,
}

impl Pane {
    pub fn spawn(id: PaneId, profile: &str, cmd: &Command, cols: usize, rows: usize, opts: &PaneOptions, waker: impl Waker) -> Result<Pane, String> {
        let mut term = Terminal::new(cols, rows);
        term.set_scrollback_limit(opts.scrollback);
        term.modes.cursor_blink = opts.cursor_blink;
        term.cursor_shape = opts.cursor_shape;
        let (tx, rx) = mpsc::channel();
        let pty = Pty::spawn(cmd, cols as u16, rows as u16, tx, waker).map_err(|e| e.to_string())?;
        let _ = id;
        Ok(Pane {
            term,
            parser: Parser::new(),
            pty,
            rx,
            profile: profile.to_string(),
            exited: false,
            rect: PxRect::default(),
            selection: None,
            search: None,
            copy: None,
            last_search: None,
            hover_link: None,
            scroll_px: 0.0,
            scroll_accum: 0.0,
            last_frame: Instant::now(),
            buttons_down: None,
            last_report_cell: None,
            activity: false,
            hold: false,
            held: false,
            trigger_next: 0,
            trigger_epoch: 0,
            trigger_marks: Default::default(),
            trigger_fired: Default::default(),
            monitor_silence: false,
            ssh_host: None,
            ssh_fixed: false,
            recorder: None,
            player: None,
            notes: None,
            loot_next: 0,
            loot_host: String::new(),
            loot_epoch: u64::MAX,
            output_since_quiet: false,
            last_output: Instant::now(),
        })
    }

    /// Feed bytes into the terminal as if the program printed them.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance_bytes(&mut self.term, bytes);
    }

    /// Which console host runs this pane ("openconsole", "conhost", "pty").
    pub fn host(&self) -> &'static str {
        self.pty.host()
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.pty.write(bytes);
    }

    pub fn kill(&self) {
        self.pty.kill();
    }

    /// Feed pending output into the terminal and answer its queries.
    pub fn drain(&mut self) -> Drained {
        let mut d = Drained::default();
        let before = self.term.generation();
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                PtyEvent::Output(bytes) => {
                    if self.player.is_some() {
                        // a replay owns the screen
                        continue;
                    }
                    if let Some(r) = self.recorder.as_mut() {
                        r.output(&bytes);
                    }
                    self.parser.advance_bytes(&mut self.term, &bytes);
                    d.output = true;
                }
                PtyEvent::Exited(code) => {
                    self.exited = true;
                    if self.hold && !self.held {
                        self.held = true;
                        let msg = format!("\r\n\x1b[0m\x1b[2m[process exited with code {code} — press Enter to close]\x1b[0m",);
                        self.parser.advance_bytes(&mut self.term, msg.as_bytes());
                        d.output = true;
                    } else {
                        d.exited = true;
                    }
                }
            }
        }
        let replies = self.term.take_responses();
        if !replies.is_empty() {
            self.write(&replies);
        }
        d.title = self.term.take_title_change().map(str::to_string);
        d.bell = self.term.take_bell();
        d.changed = self.term.generation() != before;
        d.clipboard = self.term.take_clipboard_writes();
        d.clipboard_reads = self.term.take_clipboard_reads();
        d.notifications = self.term.take_notifications();
        if d.output {
            if let Some(s) = self.search.as_mut() {
                s.dirty = true;
            }
            self.hover_link = None;
        }
        d
    }

    /// Title for tabs: the shell's title, else the profile name. A bare
    /// program path ("C:\...\powershell.exe") says nothing, so it shows
    /// the profile name too.
    pub fn title(&self) -> String {
        let t = self.term.title().trim();
        if t.is_empty() || t.to_ascii_lowercase().ends_with(".exe") {
            self.profile.clone()
        } else {
            t.to_string()
        }
    }

    /// Resize the grid (and the PTY) if it changed.
    pub fn set_grid(&mut self, cols: usize, rows: usize) {
        if cols != self.term.cols() || rows != self.term.rows() {
            self.term.resize(cols, rows);
            let _ = self.pty.resize(cols as u16, rows as u16);
            if let Some(r) = self.recorder.as_mut() {
                r.resize(cols, rows);
            }
            self.selection = None;
            self.hover_link = None;
            if let Some(s) = self.search.as_mut() {
                s.dirty = true;
            }
        }
    }

    pub fn selection_text(&self) -> String {
        self.selection
            .map(|sel| {
                let (s, e) = sel.ordered();
                self.term.text_range(s, e)
            })
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ search

    pub fn refresh_search(&mut self) {
        let Some(search) = self.search.as_mut() else { return };
        if !search.dirty {
            return;
        }
        search.dirty = false;
        let prev = search.current.and_then(|i| search.matches.get(i).copied());
        search.error = false;
        search.matches = if search.regex {
            match regex::RegexBuilder::new(&search.query).case_insensitive(!search.case).size_limit(1 << 20).dfa_size_limit(4 << 20).build() {
                Ok(re) => self.term.find_regex(&re),
                Err(_) => {
                    search.error = !search.query.is_empty();
                    Vec::new()
                }
            }
        } else if search.case {
            self.term.find_regex(&regex::Regex::new(&regex::escape(&search.query)).unwrap())
        } else {
            self.term.find(&search.query)
        };
        search.current = match prev {
            Some(p) => search.matches.iter().position(|m| *m == p).or_else(|| search.matches.len().checked_sub(1)),
            None => search.matches.len().checked_sub(1),
        };
    }

    pub fn open_search(&mut self) {
        if self.search.is_none() {
            let query = Some(self.selection_text()).filter(|t| !t.contains('\n')).unwrap_or_default();
            self.search = Some(Search { query, regex: false, case: false, error: false, matches: Vec::new(), current: None, dirty: true });
            self.search_changed();
        }
    }

    pub fn search_changed(&mut self) {
        if let Some(s) = self.search.as_mut() {
            s.dirty = true;
            s.current = None;
        }
        self.refresh_search();
        self.jump_to_current();
    }

    fn jump_to_current(&mut self) {
        let hit = self.search.as_ref().and_then(|s| s.current.map(|i| s.matches[i]));
        if let Some(m) = hit {
            self.term.scroll_to_line(m.line);
        }
    }

    /// Scroll just enough to show absolute `line`.
    pub fn reveal_line(&mut self, line: usize) {
        let top = self.term.top_visible_line();
        let rows = self.term.rows();
        if line < top {
            self.term.set_view_top(line);
        } else if line >= top + rows {
            self.term.set_view_top(line + 1 - rows);
        }
    }

    /// Put the copy-mode cursor on the current search hit.
    pub fn copy_to_search_hit(&mut self) {
        let hit = self.search.as_ref().and_then(|s| s.current.map(|i| s.matches[i]));
        if let (Some(m), Some(cm)) = (hit, self.copy.as_mut()) {
            cm.cursor = (m.start, m.line);
        }
    }

    /// n / N in copy mode: step through the last search's hits even after
    /// its bar was closed.
    pub fn copy_search_step(&mut self, older: bool) {
        if self.search.is_none() {
            self.search = self.last_search.take();
        }
        if let Some(s) = self.search.as_mut() {
            s.dirty = true;
        }
        self.refresh_search();
        self.step_search(older);
        self.copy_to_search_hit();
        self.last_search = self.search.take();
    }

    pub fn step_search(&mut self, older: bool) {
        let Some(s) = self.search.as_mut() else { return };
        let n = s.matches.len();
        if n == 0 {
            return;
        }
        s.current = Some(match (s.current, older) {
            (Some(i), true) => (i + n - 1) % n,
            (Some(i), false) => (i + 1) % n,
            (None, _) => n - 1,
        });
        self.jump_to_current();
    }

    // ------------------------------------------------- shell integration

    /// Scroll to the previous (`older`) or next prompt relative to the top
    /// of the view. Returns false if there is none.
    pub fn jump_prompt(&mut self, older: bool) -> bool {
        let lines = self.term.prompt_lines();
        let top = self.term.top_visible_line();
        let target = if older { lines.iter().rev().find(|&&l| l < top).copied() } else { lines.iter().find(|&&l| l > top).copied() };
        match target {
            Some(l) => {
                self.term.set_view_top(l);
                true
            }
            None if !older => {
                self.term.scroll_view_to_bottom();
                false
            }
            None => false,
        }
    }

    // ----------------------------------------------------------- drawing

    /// Advance the smooth-scroll animation; true while still moving.
    pub fn animate_scroll(&mut self) -> bool {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        if self.scroll_px != 0.0 {
            self.scroll_px *= (-dt / 0.045).exp();
            if self.scroll_px.abs() < 0.5 {
                self.scroll_px = 0.0;
            }
        }
        self.scroll_px != 0.0
    }

    /// Absolute range → visible-row range, clipped to the viewport.
    pub fn to_visible(&self, s: (usize, usize), e: (usize, usize)) -> Option<VisibleSelection> {
        let top = self.term.top_visible_line();
        let rows = self.term.rows();
        let bottom = top + rows - 1;
        if e.1 < top || s.1 > bottom {
            return None;
        }
        let start = if s.1 < top { (0, 0) } else { (s.0, s.1 - top) };
        let end = if e.1 > bottom { (self.term.cols() - 1, rows - 1) } else { (e.0, e.1 - top) };
        Some(VisibleSelection { start, end, block: false })
    }

    /// Everything the renderer needs for this pane.
    pub fn draw_state(&self, theme: &Theme, focused: bool, cursor_on: bool, dim: bool, marks: bool) -> DrawState {
        let mut state = DrawState {
            sticky: self.sticky_header(theme),
            focused,
            cursor_visible: cursor_on || !self.term.modes.cursor_blink || !focused,
            y_shift: self.scroll_px,
            dim,
            marks,
            ..Default::default()
        };
        state.selection = self.selection.and_then(|sel| {
            let (s, e) = sel.ordered();
            self.to_visible(s, e)
        });
        if let Some(cm) = &self.copy {
            let top = self.term.top_visible_line();
            let (cx, cl) = cm.cursor;
            if cl >= top && cl < top + self.term.rows() {
                state.copy_cursor = Some((cx, cl - top));
            }
            state.cursor_visible = false;
            state.selection = cm.region(&self.term).and_then(|(s, e, block)| {
                if block {
                    let rows = self.term.rows();
                    let (l0, l1) = (s.1.max(top), e.1.min(top + rows - 1));
                    (l0 <= l1).then_some(VisibleSelection { start: (s.0, l0 - top), end: (e.0, l1 - top), block: true })
                } else {
                    self.to_visible(s, e)
                }
            });
        }
        state.link = self.hover_link.as_ref().and_then(|l| self.to_visible(l.start, l.end));
        // trigger highlights still in history
        let (epoch, evicted) = self.term.line_base();
        if epoch == self.trigger_epoch && !self.trigger_marks.is_empty() {
            let top = self.term.top_visible_line() as u64 + evicted;
            let rows = self.term.rows() as u64;
            for &(line, s, e, bg) in &self.trigger_marks {
                if line >= top && line < top + rows {
                    let vy = (line - top) as usize;
                    state.highlights.push(Highlight { range: VisibleSelection { start: (s, vy), end: (e, vy), block: false }, bg, fg: None });
                }
            }
        }
        if let Some(search) = &self.search {
            let top = self.term.top_visible_line();
            let bottom = top + self.term.rows();
            for (i, m) in search.matches.iter().enumerate() {
                if m.line < top || m.line >= bottom {
                    continue;
                }
                let range = VisibleSelection { start: (m.start, m.line - top), end: (m.end, m.line - top), block: false };
                let current = search.current == Some(i);
                state.highlights.push(Highlight {
                    range,
                    bg: if current { theme.cursor } else { theme.search_match },
                    fg: current.then_some(theme.cursor_text),
                });
            }
        }
        state
    }

    /// The block whose prompt scrolled off the top: its command, pinned.
    pub fn sticky_header(&self, theme: &Theme) -> Option<vibex_render::StickyHeader> {
        if self.term.is_alt_screen() {
            return None;
        }
        let top = self.term.top_visible_line();
        let b = self.term.block_at(top)?;
        if b.prompt >= top || b.command.text.is_empty() || b.end <= top {
            return None;
        }
        let mut detail = Vec::new();
        match (b.command.duration, b.command.started) {
            (Some(d), _) => detail.push(format_duration(d)),
            (None, Some(s)) => detail.push(format!("running {}", format_duration(s.elapsed()))),
            _ => {}
        }
        if let Some(code) = b.command.exit.filter(|c| *c != 0) {
            detail.push(format!("exit {code}"));
        }
        let color = match b.command.exit {
            Some(0) => theme.palette[2],
            Some(_) => theme.palette[1],
            None => theme.cursor,
        };
        Some(vibex_render::StickyHeader { command: b.command.text, detail: detail.join(" · "), color })
    }

    /// Text for the search overlay.
    pub fn search_label(&self) -> Option<String> {
        let search = self.search.as_ref()?;
        let count = match search.current {
            _ if search.error => "bad regex".into(),
            Some(i) => format!("{}/{}", i + 1, search.matches.len()),
            None if search.query.is_empty() => String::new(),
            None => "no matches".into(),
        };
        let kind = match (search.regex, search.case) {
            (true, true) => "find (regex, Aa)",
            (true, false) => "find (regex)",
            (false, true) => "find (Aa)",
            (false, false) => "find",
        };
        Some(format!("{kind}: {}_   {count}\nCtrl+R regex · Alt+C match case", search.query))
    }

    /// Window pixel position → visible (col, row) in this pane.
    pub fn cell_at(&self, x: f64, y: f64, cw: usize, ch: usize, pad: (usize, usize)) -> (usize, usize) {
        let cx = ((x - self.rect.x as f64 - pad.0 as f64) / cw as f64).floor().max(0.0) as usize;
        let cy = ((y - self.rect.y as f64 - pad.1 as f64) / ch as f64).floor().max(0.0) as usize;
        (cx.min(self.term.cols() - 1), cy.min(self.term.rows() - 1))
    }
}

/// "850ms", "12.3s", "4m 05s", "1h 02m".
pub fn format_duration(d: std::time::Duration) -> String {
    let s = d.as_secs_f64();
    if s < 1.0 {
        format!("{}ms", d.as_millis())
    } else if s < 60.0 {
        format!("{s:.1}s")
    } else if s < 3600.0 {
        format!("{}m {:02}s", (s / 60.0) as u64, s as u64 % 60)
    } else {
        format!("{}h {:02}m", (s / 3600.0) as u64, (s as u64 % 3600) / 60)
    }
}

/// Start `cmd` in `cwd` where possible: a Windows path becomes the working
/// directory; a POSIX path is passed to wsl.exe with `--cd`.
pub fn with_cwd(mut cmd: Command, cwd: Option<&str>) -> Command {
    let Some(cwd) = cwd.filter(|c| !c.is_empty()) else { return cmd };
    let is_wsl = cmd.program.to_ascii_lowercase().ends_with("wsl.exe") || cmd.program.eq_ignore_ascii_case("wsl");
    if cwd.starts_with('/') {
        if is_wsl {
            cmd.args.retain(|a| a != "~");
            cmd.args.insert(0, cwd.to_string());
            cmd.args.insert(0, "--cd".into());
        }
    } else if std::path::Path::new(cwd).is_dir() {
        cmd.cwd = Some(cwd.to_string());
        if is_wsl {
            cmd.args.retain(|a| a != "~");
        }
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn cwd_for_windows_and_wsl() {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into());
        let ps = with_cwd(Command::new("powershell.exe"), Some(&home));
        assert_eq!(ps.cwd.as_deref(), Some(home.as_str()));
        let wsl = with_cwd(Command::new("wsl.exe").arg("~"), Some("/home/me/code"));
        assert_eq!(wsl.args, vec!["--cd", "/home/me/code"]);
        let ps2 = with_cwd(Command::new("powershell.exe"), Some("/home/me"));
        assert!(ps2.cwd.is_none(), "posix path ignored for Windows shells");
        let missing = with_cwd(Command::new("cmd.exe"), Some("Z:\\nope\\nope"));
        assert!(missing.cwd.is_none());
    }
}
