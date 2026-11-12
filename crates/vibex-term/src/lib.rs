//! Terminal state: the screen grid, scrollback, cursor, modes and the
//! interpretation of control sequences (xterm-compatible subset).

mod blocks;
mod cell;
pub mod grapheme;
pub mod image;
mod kitty;
pub mod links;
mod perform;
mod reflow;
mod select;
mod serialize;
mod sixel;

pub use select::{classify, parse_file_ref, FileRef, Link, Match, Target, TargetKind};

pub use blocks::{Block, CommandInfo, Finished, PromptInput};
pub use cell::{flags, Cell, Color, PromptMark, Row};

use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;

pub const DEFAULT_SCROLLBACK: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    pub x: usize,
    pub y: usize,
    /// Attributes applied to newly printed characters.
    pub pen: Cell,
}

#[derive(Clone, Copy, Debug)]
struct SavedCursor {
    cursor: Cursor,
    origin: bool,
    wrap_pending: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Modes {
    pub autowrap: bool,
    pub origin: bool,
    pub insert: bool,
    /// LNM: LF also does CR.
    pub newline: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub show_cursor: bool,
    pub cursor_blink: bool,
    pub bracketed_paste: bool,
    pub focus_events: bool,
    pub mouse_report: MouseMode,
    pub mouse_sgr: bool,
    /// DECSCNM: the whole screen in reverse video (light background).
    pub reverse_screen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    Off,
    /// 1000: press/release.
    Click,
    /// 1002: + motion while a button is held.
    Drag,
    /// 1003: all motion.
    Motion,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            autowrap: true,
            origin: false,
            insert: false,
            newline: false,
            app_cursor: false,
            app_keypad: false,
            show_cursor: true,
            cursor_blink: false,
            bracketed_paste: false,
            focus_events: false,
            mouse_report: MouseMode::Off,
            mouse_sgr: false,
            reverse_screen: false,
        }
    }
}

pub struct Terminal {
    cols: usize,
    rows: usize,
    primary: Vec<Row>,
    alternate: Vec<Row>,
    alt_active: bool,
    scrollback: VecDeque<Row>,
    scrollback_limit: usize,
    pub cursor: Cursor,
    saved_primary: Option<SavedCursor>,
    saved_alt: Option<SavedCursor>,
    wrap_pending: bool,
    scroll_top: usize,
    scroll_bottom: usize,
    pub modes: Modes,
    pub cursor_shape: CursorShape,
    tabs: Vec<bool>,
    title: String,
    title_changed: bool,
    responses: Vec<u8>,
    /// How many rows the view is scrolled back into history (0 = live).
    display_offset: usize,
    last_char: Option<char>,
    bell: bool,
    /// Working directory reported by the shell (OSC 7 / OSC 9;9).
    cwd: Option<String>,
    /// Exit code of the last finished command (OSC 133;D).
    last_exit: Option<i32>,
    /// Kitty keyboard protocol flag stacks (primary, alternate screen).
    kitty_primary: Vec<u8>,
    kitty_alt: Vec<u8>,
    /// Last printed cell (x, y) and the cursor state right after it, so
    /// combining characters can join it.
    last_cell: Option<(usize, usize, usize, bool)>,
    /// The previous character was a ZWJ: join the next one too.
    join_next: bool,
    /// OSC 52 clipboard writes waiting for the app.
    clipboard_writes: Vec<String>,
    /// OSC 52 clipboard read requests (`?`) waiting for the app.
    clipboard_reads: usize,
    /// OSC 9 / OSC 777 notifications: (title, body).
    notifications: Vec<(String, String)>,
    /// Cell size in pixels (set by the app) for sizing images.
    cell_px: (u32, u32),
    kitty: kitty::KittyState,
    /// A sixel image being received (DCS q).
    sixel: Option<sixel::Sixel>,
    /// Bumped by everything that can change what's drawn; renderers skip
    /// frames when it hasn't moved.
    generation: u64,
    /// Command blocks (shell integration).
    blocks: blocks::BlockState,
    /// Lines dropped off the top of history so far, and a counter bumped
    /// when line numbering changes wholesale (re-wrap): absolute line +
    /// `evicted` is stable while `epoch` stays the same.
    evicted: u64,
    /// The first `history_fence` scrollback rows are restored text from an
    /// earlier session: growing the window or re-wrapping never pulls them
    /// back onto the screen.
    history_fence: usize,
    epoch: u64,
}

impl Terminal {
    /// A terminal whose screen is exactly `rows` (no cursor shown), for
    /// drawing or writing out a copy of some lines (exports).
    pub fn from_rows(cols: usize, rows: Vec<Row>) -> Self {
        let n = rows.len().max(1);
        let mut t = Terminal::new(cols, n);
        for (dst, mut src) in t.primary.iter_mut().zip(rows) {
            src.cells.resize(t.cols, Cell::default());
            *dst = src;
        }
        t.cursor.y = n - 1;
        t.modes.show_cursor = false;
        t
    }

    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(2);
        let rows = rows.max(1);
        Self {
            cols,
            rows,
            primary: blank_screen(cols, rows),
            alternate: blank_screen(cols, rows),
            alt_active: false,
            scrollback: VecDeque::new(),
            scrollback_limit: DEFAULT_SCROLLBACK,
            cursor: Cursor::default(),
            saved_primary: None,
            saved_alt: None,
            wrap_pending: false,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            modes: Modes::default(),
            cursor_shape: CursorShape::default(),
            tabs: default_tabs(cols),
            title: String::new(),
            title_changed: false,
            responses: Vec::new(),
            display_offset: 0,
            last_char: None,
            bell: false,
            cwd: None,
            last_exit: None,
            kitty_primary: Vec::new(),
            kitty_alt: Vec::new(),
            last_cell: None,
            join_next: false,
            clipboard_writes: Vec::new(),
            clipboard_reads: 0,
            notifications: Vec::new(),
            cell_px: (8, 16),
            kitty: Default::default(),
            sixel: None,
            generation: 0,
            blocks: Default::default(),
            evicted: 0,
            history_fence: 0,
            epoch: 0,
        }
    }

    /// Changes whenever the screen may look different (output, resize,
    /// scrolling the view). Equal values mean nothing to redraw.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Push everything on screen into scrollback, clear the screen and put
    /// the cursor top-left, and keep it there: resizing won't bring it back
    /// (restored scrollback from an earlier session).
    pub fn seal_history(&mut self) {
        if self.alt_active {
            return;
        }
        // only rows up to the cursor's (blank rows below aren't worth keeping)
        let keep = (self.cursor.y + 1).min(self.primary.len());
        let rows: Vec<Row> = self.primary.drain(..keep).collect();
        for row in rows {
            let _ = self.push_scrollback(row);
        }
        self.primary.clear();
        for _ in 0..self.rows {
            self.primary.push(Row::new(self.cols, Cell::default()));
        }
        self.cursor.x = 0;
        self.cursor.y = 0;
        self.history_fence = self.scrollback.len();
        self.display_offset = 0;
        self.touch();
    }

    /// (epoch, lines evicted): see the field docs.
    pub fn line_base(&self) -> (u64, u64) {
        (self.epoch, self.evicted)
    }

    pub(crate) fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Text apps asked to put on the clipboard (OSC 52).
    pub fn take_clipboard_writes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.clipboard_writes)
    }

    /// Number of OSC 52 clipboard reads requested since the last call.
    pub fn take_clipboard_reads(&mut self) -> usize {
        std::mem::take(&mut self.clipboard_reads)
    }

    /// Answer an OSC 52 read with the clipboard's text.
    pub fn reply_clipboard(&mut self, text: &str) {
        let s = format!("\x1b]52;c;{}\x07", links::base64_encode(text.as_bytes()));
        self.respond(s.as_bytes());
    }

    /// Notifications requested with OSC 9 / OSC 777: (title, body).
    pub fn take_notifications(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.notifications)
    }

    pub(crate) fn osc52(&mut self, data: &str) {
        if data == "?" {
            self.clipboard_reads += 1;
            return;
        }
        if let Some(bytes) = links::base64_decode(data) {
            if bytes.len() <= 1 << 20 {
                self.clipboard_writes.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }

    pub(crate) fn notify(&mut self, title: String, body: String) {
        if self.notifications.len() < 16 {
            self.notifications.push((title, body));
        }
    }

    /// OSC 8: start (or end, with an empty URI) a hyperlink.
    pub(crate) fn set_hyperlink(&mut self, uri: &str) {
        self.cursor.pen.link = links::intern(uri);
    }

    /// The OSC 8 link under (col, line), with the extent of that link on
    /// its logical line.
    pub fn hyperlink_at(&self, col: usize, line: usize) -> Option<Link> {
        let id = self.absolute_row(line)?.cells.get(col)?.link;
        let url = links::url(id)?;
        let (first, last) = self.logical_line_bounds(line);
        // walk outwards over cells with the same id
        let mut start = (col, line);
        let mut end = (col, line);
        let cell_id = |x: usize, l: usize| self.absolute_row(l).and_then(|r| r.cells.get(x)).map_or(0, |c| c.link);
        loop {
            let (x, l) = start;
            let prev = if x > 0 {
                Some((x - 1, l))
            } else if l > first {
                Some((self.cols - 1, l - 1))
            } else {
                None
            };
            match prev {
                Some((px, pl)) if cell_id(px, pl) == id => start = (px, pl),
                _ => break,
            }
        }
        loop {
            let (x, l) = end;
            let next = if x + 1 < self.cols {
                Some((x + 1, l))
            } else if l < last {
                Some((0, l + 1))
            } else {
                None
            };
            match next {
                Some((nx, nl)) if cell_id(nx, nl) == id => end = (nx, nl),
                _ => break,
            }
        }
        Some(Link { url, start, end })
    }

    // ------------------------------------------------ kitty keyboard protocol

    /// Active kitty keyboard flags (0 = legacy encoding).
    pub fn kitty_flags(&self) -> u8 {
        let stack = if self.alt_active { &self.kitty_alt } else { &self.kitty_primary };
        stack.last().copied().unwrap_or(0)
    }

    fn kitty_stack(&mut self) -> &mut Vec<u8> {
        if self.alt_active {
            &mut self.kitty_alt
        } else {
            &mut self.kitty_primary
        }
    }

    /// CSI > flags u
    pub(crate) fn kitty_push(&mut self, flags: u8) {
        let s = self.kitty_stack();
        if s.len() >= 16 {
            s.remove(0);
        }
        s.push(flags & 0x1f);
    }

    /// CSI < n u
    pub(crate) fn kitty_pop(&mut self, n: usize) {
        let s = self.kitty_stack();
        let keep = s.len().saturating_sub(n.max(1));
        s.truncate(keep);
    }

    /// CSI = flags ; mode u   (1 set, 2 or, 3 and-not)
    pub(crate) fn kitty_set(&mut self, flags: u8, mode: u16) {
        let cur = self.kitty_flags();
        let new = match mode {
            2 => cur | flags,
            3 => cur & !flags,
            _ => flags,
        } & 0x1f;
        let s = self.kitty_stack();
        match s.last_mut() {
            Some(top) => *top = new,
            None => s.push(new),
        }
    }

    /// CSI ? u
    pub(crate) fn kitty_query(&mut self) {
        let f = self.kitty_flags();
        self.respond(format!("\x1b[?{f}u").as_bytes());
    }

    // ---------------------------------------------------- shell integration

    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    pub fn last_exit(&self) -> Option<i32> {
        self.last_exit
    }

    pub(crate) fn set_cwd(&mut self, cwd: String) {
        self.cwd = Some(cwd);
    }

    /// Absolute lines where prompts start (oldest first).
    pub fn prompt_lines(&self) -> Vec<usize> {
        (0..self.total_lines()).filter(|&l| self.absolute_row(l).is_some_and(|r| r.prompt.is_some())).collect()
    }

    // ---------------------------------------------------------------- queries

    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn is_alt_screen(&self) -> bool {
        self.alt_active
    }
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }
    pub fn display_offset(&self) -> usize {
        self.display_offset
    }
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the title if it changed since the last call.
    pub fn take_title_change(&mut self) -> Option<&str> {
        if std::mem::take(&mut self.title_changed) {
            Some(&self.title)
        } else {
            None
        }
    }

    /// Bytes the terminal wants to send back to the application
    /// (cursor position reports, device attributes, ...).
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
    }

    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.bell)
    }

    /// The `i`th visible row, accounting for the scrollback view offset.
    pub fn visible_row(&self, i: usize) -> &Row {
        if self.display_offset == 0 || self.alt_active {
            return &self.screen()[i];
        }
        let sb = self.scrollback.len();
        let idx = sb - self.display_offset + i;
        if idx < sb {
            &self.scrollback[idx]
        } else {
            &self.screen()[idx - sb]
        }
    }

    /// Absolute line index (scrollback + screen) of visible row 0.
    pub fn top_visible_line(&self) -> usize {
        if self.alt_active {
            0
        } else {
            self.scrollback.len() - self.display_offset
        }
    }

    /// Number of addressable lines (scrollback + screen).
    pub fn total_lines(&self) -> usize {
        if self.alt_active {
            self.rows
        } else {
            self.scrollback.len() + self.rows
        }
    }

    /// Row by absolute line index (0 = oldest scrollback line).
    pub fn absolute_row(&self, line: usize) -> Option<&Row> {
        if self.alt_active {
            return self.alternate.get(line);
        }
        let sb = self.scrollback.len();
        if line < sb {
            self.scrollback.get(line)
        } else {
            self.primary.get(line - sb)
        }
    }

    /// Text between two absolute (col, line) points, inclusive. Soft-wrapped
    /// rows are joined without a newline; trailing blanks are trimmed.
    pub fn text_range(&self, start: (usize, usize), end: (usize, usize)) -> String {
        let mut out = String::new();
        for line in start.1..=end.1 {
            let Some(row) = self.absolute_row(line) else { break };
            let from = if line == start.1 { start.0 } else { 0 };
            let to = if line == end.1 { (end.0 + 1).min(row.cells.len()) } else { row.cells.len() };
            let mut s = String::new();
            for cell in row.cells.get(from..to).unwrap_or(&[]) {
                if !cell.has(flags::WIDE_SPACER | flags::WRAP_PAD) {
                    s.push_str(&grapheme::text(cell.c));
                }
            }
            let last = line == end.1;
            if row.wrapped && !last {
                out.push_str(&s);
            } else {
                out.push_str(s.trim_end());
                if !last {
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Screen row (ignores scrollback view).
    pub fn screen_row(&self, i: usize) -> &Row {
        &self.screen()[i]
    }

    /// Plain-text dump of the live screen (tests, debugging).
    pub fn screen_text(&self) -> String {
        self.screen().iter().map(|r| r.text()).collect::<Vec<_>>().join("\n")
    }

    // ------------------------------------------------------------- viewport

    /// Scroll the view into history; positive = up (older).
    pub fn scroll_view(&mut self, delta: isize) {
        if self.alt_active {
            return;
        }
        self.touch();
        let max = self.scrollback.len() as isize;
        let new = (self.display_offset as isize + delta).clamp(0, max);
        self.display_offset = new as usize;
    }

    pub fn scroll_view_to_bottom(&mut self) {
        if self.display_offset != 0 {
            self.touch();
        }
        self.display_offset = 0;
    }

    pub fn clear_scrollback(&mut self) {
        self.evicted += self.scrollback.len() as u64;
        self.scrollback.clear();
        self.history_fence = 0;
        self.display_offset = 0;
    }

    pub fn set_scrollback_limit(&mut self, limit: usize) {
        self.scrollback_limit = limit;
        while self.scrollback.len() > limit {
            self.scrollback.pop_front();
            self.evicted += 1;
        }
        self.display_offset = self.display_offset.min(self.scrollback.len());
    }

    // --------------------------------------------------------------- resize

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.touch();
        let cols = cols.max(2);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }

        if cols != self.cols {
            // Width change: re-wrap the primary screen + history; the
            // alternate screen (full-screen apps) just gets cropped/padded,
            // the app repaints it anyway.
            let primary_cursor = if self.alt_active {
                self.saved_primary.map_or((0, 0), |s| (s.cursor.x + s.wrap_pending as usize, s.cursor.y))
            } else {
                (self.cursor.x + self.wrap_pending as usize, self.cursor.y)
            };
            let (x, y) = self.reflow_primary(cols, rows, primary_cursor);
            if self.alt_active {
                if let Some(s) = self.saved_primary.as_mut() {
                    s.cursor.x = x;
                    s.cursor.y = y;
                    s.wrap_pending = false;
                }
                resize_plain(&mut self.alternate, cols, rows, &mut self.cursor.y);
            } else {
                self.cursor.x = x;
                self.cursor.y = y;
                let mut unused = 0;
                resize_plain(&mut self.alternate, cols, rows, &mut unused);
            }
        } else if rows < self.rows {
            // Rows only: primary screen trades lines with scrollback so the
            // cursor line stays on screen and content isn't lost.
            let excess = self.rows - rows;
            let below = self.rows - 1 - self.cursor.y.min(self.rows - 1);
            let from_bottom = excess.min(below);
            let from_top = excess - from_bottom;
            for _ in 0..from_bottom {
                self.primary.pop();
                self.alternate.pop();
            }
            for _ in 0..from_top {
                let row = self.primary.remove(0);
                if !self.alt_active {
                    let _ = self.push_scrollback(row);
                }
                self.alternate.remove(0);
            }
            self.cursor.y = self.cursor.y.saturating_sub(from_top);
        } else if rows > self.rows {
            let extra = rows - self.rows;
            let mut pulled = 0;
            if !self.alt_active {
                while pulled < extra && self.scrollback.len() > self.history_fence {
                    match self.scrollback.pop_back() {
                        Some(mut row) => {
                            row.resize(self.cols);
                            self.primary.insert(0, row);
                            pulled += 1;
                        }
                        None => break,
                    }
                }
                self.cursor.y += pulled;
            }
            for _ in pulled..extra {
                self.primary.push(Row::new(self.cols, Cell::default()));
            }
            for _ in 0..extra {
                self.alternate.push(Row::new(self.cols, Cell::default()));
            }
        }

        for row in self.primary.iter_mut().chain(self.alternate.iter_mut()) {
            row.resize(cols);
        }

        self.cols = cols;
        self.rows = rows;
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.tabs = default_tabs(cols);
        self.cursor.x = self.cursor.x.min(cols - 1);
        self.cursor.y = self.cursor.y.min(rows - 1);
        self.wrap_pending = false;
        self.display_offset = self.display_offset.min(self.scrollback.len());
    }

    // ------------------------------------------------------------ internals

    fn screen(&self) -> &Vec<Row> {
        if self.alt_active {
            &self.alternate
        } else {
            &self.primary
        }
    }

    fn screen_mut(&mut self) -> &mut Vec<Row> {
        if self.alt_active {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    fn blank(&self) -> Cell {
        Cell::blank(&self.cursor.pen)
    }

    /// Push a row into history; returns the row evicted from the far end
    /// (when history is full) so callers can reuse its allocation.
    fn push_scrollback(&mut self, row: Row) -> Option<Row> {
        if self.scrollback_limit == 0 {
            return Some(row);
        }
        let evicted = if self.scrollback.len() >= self.scrollback_limit {
            self.evicted += 1;
            self.history_fence = self.history_fence.saturating_sub(1);
            self.scrollback.pop_front()
        } else {
            if self.display_offset > 0 {
                // keep the user's view anchored on the same content
                self.display_offset += 1;
            }
            None
        };
        self.scrollback.push_back(row);
        evicted
    }

    /// Try to join `c` onto the previously printed cell (combining marks,
    /// ZWJ sequences, skin tones, flag pairs, VS16). True if it was joined.
    fn join_cluster(&mut self, c: char, width: usize) -> bool {
        let Some((lx, ly, after_x, after_pending)) = self.last_cell else { return false };
        // only if nothing moved the cursor since that cell was printed
        if self.cursor.y != ly || self.cursor.x != after_x || self.wrap_pending != after_pending {
            return false;
        }
        let row_len = self.screen()[ly].cells.len();
        if lx >= row_len {
            return false;
        }
        let prev = self.screen()[ly].cells[lx].c;
        let base = grapheme::base(prev);
        let prev_text = grapheme::text(prev);
        let prev_is_single_ri = prev_text.chars().count() == 1 && grapheme::is_regional_indicator(base);
        let joins = width == 0
            || self.join_next
            || (grapheme::is_emoji_modifier(c) && grapheme::is_pictographic(base))
            || (grapheme::is_regional_indicator(c) && prev_is_single_ri);
        if !joins {
            return false;
        }
        self.join_next = c == grapheme::ZWJ;
        let cols = self.cols;
        let row = &mut self.screen_mut()[ly];
        let cell = &mut row.cells[lx];
        cell.c = grapheme::append(prev, c);
        // VS16 asks for emoji presentation: widen a narrow cell when there's room
        if c == grapheme::VS16 && !cell.has(flags::WIDE) && !after_pending && lx + 1 < cols {
            let pen = *cell;
            cell.flags |= flags::WIDE;
            let mut spacer = pen;
            spacer.c = ' ';
            spacer.flags = (spacer.flags & !flags::WIDE) | flags::WIDE_SPACER;
            row.cells[lx + 1] = spacer;
            let next = lx + 2;
            if next >= cols {
                self.cursor.x = cols - 1;
                self.wrap_pending = true;
            } else {
                self.cursor.x = next;
            }
            self.last_cell = Some((lx, ly, self.cursor.x, self.wrap_pending));
        }
        true
    }

    pub(crate) fn print(&mut self, c: char) {
        let mut width = match c.width() {
            Some(w) => w,
            None => return,
        };
        // a flag is two regional indicators drawn as one wide glyph
        if grapheme::is_regional_indicator(c) {
            width = 2;
        }
        if self.join_cluster(c, width) {
            return;
        }
        self.join_next = false;
        if width == 0 {
            // a stray combining mark with nothing to attach to
            return;
        }
        // real private-use-B characters get a handle too (see grapheme.rs)
        let c = if grapheme::is_cluster(c) { grapheme::intern(&c.to_string()) } else { c };
        self.last_char = Some(c);

        if self.wrap_pending && self.modes.autowrap {
            let y = self.cursor.y;
            self.screen_mut()[y].wrapped = true;
            self.cursor.x = 0;
            self.linefeed_inner();
        }
        self.wrap_pending = false;

        if width == 2 && self.cursor.x == self.cols - 1 {
            if self.modes.autowrap {
                let (x, y) = (self.cursor.x, self.cursor.y);
                let pad = Cell { flags: flags::WRAP_PAD, ..self.blank() };
                let row = &mut self.screen_mut()[y];
                row.cells[x] = pad;
                row.wrapped = true;
                self.cursor.x = 0;
                self.linefeed_inner();
            } else {
                return;
            }
        }

        let (x, y, cols) = (self.cursor.x, self.cursor.y, self.cols);
        let pen = self.cursor.pen;
        let insert = self.modes.insert;
        let blank = self.blank();
        let row = &mut self.screen_mut()[y];

        if insert {
            for _ in 0..width {
                row.cells.insert(x, blank);
                row.cells.truncate(cols);
            }
        }

        clear_wide_partner(&mut row.cells, x);
        let mut cell = pen;
        cell.c = c;
        cell.flags &= !(flags::WIDE | flags::WIDE_SPACER);
        if width == 2 {
            cell.flags |= flags::WIDE;
            clear_wide_partner(&mut row.cells, x + 1);
            row.cells[x] = cell;
            let mut spacer = pen;
            spacer.c = ' ';
            spacer.flags = (spacer.flags & !flags::WIDE) | flags::WIDE_SPACER;
            row.cells[x + 1] = spacer;
        } else {
            row.cells[x] = cell;
        }

        let next = x + width;
        if next >= cols {
            self.cursor.x = cols - 1;
            self.wrap_pending = true;
        } else {
            self.cursor.x = next;
        }
        self.last_cell = Some((x, y, self.cursor.x, self.wrap_pending));
    }

    fn linefeed_inner(&mut self) {
        if self.cursor.y == self.scroll_bottom {
            self.scroll_up(1);
        } else if self.cursor.y < self.rows - 1 {
            self.cursor.y += 1;
        }
    }

    pub(crate) fn linefeed(&mut self) {
        self.wrap_pending = false;
        self.linefeed_inner();
        if self.modes.newline {
            self.cursor.x = 0;
        }
    }

    pub(crate) fn reverse_index(&mut self) {
        self.wrap_pending = false;
        if self.cursor.y == self.scroll_top {
            self.scroll_down(1);
        } else if self.cursor.y > 0 {
            self.cursor.y -= 1;
        }
    }

    /// Scroll the scroll region up by `n` (content moves up).
    pub(crate) fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        let n = n.min(bottom - top + 1);
        let blank = self.blank();
        let to_history = top == 0 && !self.alt_active;
        let cols = self.cols;
        for _ in 0..n {
            let mut row = self.screen_mut().remove(top);
            if to_history {
                row = match self.push_scrollback(row) {
                    // recycle the evicted history row's allocation
                    Some(mut old) => {
                        old.cells.clear();
                        old.cells.resize(cols, blank);
                        old.wrapped = false;
                        old.prompt = None;
                        old.images.clear();
                        old.output = false;
                        old
                    }
                    None => Row::new(cols, blank),
                };
            } else {
                row.reset(blank);
            }
            self.screen_mut().insert(bottom, row);
        }
    }

    /// Scroll the scroll region down by `n` (content moves down).
    pub(crate) fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        let n = n.min(bottom - top + 1);
        let blank = self.blank();
        for _ in 0..n {
            let mut row = self.screen_mut().remove(bottom);
            row.reset(blank);
            self.screen_mut().insert(top, row);
        }
    }

    pub(crate) fn carriage_return(&mut self) {
        self.cursor.x = 0;
        self.wrap_pending = false;
    }

    pub(crate) fn backspace(&mut self) {
        if self.wrap_pending {
            self.wrap_pending = false;
        } else if self.cursor.x > 0 {
            self.cursor.x -= 1;
        }
    }

    pub(crate) fn tab(&mut self, n: usize) {
        for _ in 0..n {
            let mut x = self.cursor.x + 1;
            while x < self.cols - 1 && !self.tabs[x] {
                x += 1;
            }
            self.cursor.x = x.min(self.cols - 1);
        }
        self.wrap_pending = false;
    }

    pub(crate) fn back_tab(&mut self, n: usize) {
        for _ in 0..n {
            let mut x = self.cursor.x.saturating_sub(1);
            while x > 0 && !self.tabs[x] {
                x -= 1;
            }
            self.cursor.x = x;
        }
        self.wrap_pending = false;
    }

    pub(crate) fn set_tab(&mut self) {
        let x = self.cursor.x;
        self.tabs[x] = true;
    }

    pub(crate) fn clear_tabs(&mut self, mode: u16) {
        match mode {
            0 => {
                let x = self.cursor.x;
                self.tabs[x] = false;
            }
            3 => self.tabs.fill(false),
            _ => {}
        }
    }

    /// Absolute cursor move. `y` is relative to the scroll region in origin mode.
    pub(crate) fn goto(&mut self, x: usize, y: usize) {
        let (min_y, max_y) = if self.modes.origin { (self.scroll_top, self.scroll_bottom) } else { (0, self.rows - 1) };
        self.cursor.x = x.min(self.cols - 1);
        self.cursor.y = (y + min_y).min(max_y);
        self.wrap_pending = false;
    }

    pub(crate) fn goto_row(&mut self, y: usize) {
        let x = self.cursor.x;
        self.goto(x, y);
    }

    pub(crate) fn goto_col(&mut self, x: usize) {
        self.cursor.x = x.min(self.cols - 1);
        self.wrap_pending = false;
    }

    pub(crate) fn move_up(&mut self, n: usize) {
        let min = if self.cursor.y >= self.scroll_top { self.scroll_top } else { 0 };
        self.cursor.y = self.cursor.y.saturating_sub(n).max(min);
        self.wrap_pending = false;
    }

    pub(crate) fn move_down(&mut self, n: usize) {
        let max = if self.cursor.y <= self.scroll_bottom { self.scroll_bottom } else { self.rows - 1 };
        self.cursor.y = (self.cursor.y + n).min(max);
        self.wrap_pending = false;
    }

    pub(crate) fn move_right(&mut self, n: usize) {
        self.cursor.x = (self.cursor.x + n).min(self.cols - 1);
        self.wrap_pending = false;
    }

    pub(crate) fn move_left(&mut self, n: usize) {
        self.cursor.x = self.cursor.x.saturating_sub(n);
        self.wrap_pending = false;
    }

    pub(crate) fn erase_display(&mut self, mode: u16) {
        let blank = self.blank();
        let (x, y) = (self.cursor.x, self.cursor.y);
        let rows = self.rows;
        match mode {
            0 => {
                self.erase_line(0);
                for r in y + 1..rows {
                    self.screen_mut()[r].reset(blank);
                }
            }
            1 => {
                for r in 0..y {
                    self.screen_mut()[r].reset(blank);
                }
                let row = &mut self.screen_mut()[y];
                for c in &mut row.cells[..=x] {
                    *c = blank;
                }
            }
            2 => {
                for r in self.screen_mut() {
                    r.reset(blank);
                }
            }
            3 => {
                self.evicted += self.scrollback.len() as u64;
                self.scrollback.clear();
                self.history_fence = 0;
                self.display_offset = 0;
            }
            _ => {}
        }
        self.wrap_pending = false;
    }

    pub(crate) fn erase_line(&mut self, mode: u16) {
        let blank = self.blank();
        let (x, y) = (self.cursor.x, self.cursor.y);
        let row = &mut self.screen_mut()[y];
        let range = match mode {
            0 => x..row.cells.len(),
            1 => 0..x + 1,
            2 => 0..row.cells.len(),
            _ => return,
        };
        if range.start > 0 {
            clear_wide_partner(&mut row.cells, range.start);
        }
        for c in &mut row.cells[range] {
            *c = blank;
        }
        if mode != 1 {
            row.wrapped = false;
        }
        self.wrap_pending = false;
    }

    pub(crate) fn erase_chars(&mut self, n: usize) {
        let blank = self.blank();
        let (x, y, cols) = (self.cursor.x, self.cursor.y, self.cols);
        let row = &mut self.screen_mut()[y];
        let end = (x + n).min(cols);
        clear_wide_partner(&mut row.cells, x);
        if end < cols {
            clear_wide_partner(&mut row.cells, end);
        }
        for c in &mut row.cells[x..end] {
            *c = blank;
        }
        self.wrap_pending = false;
    }

    pub(crate) fn insert_chars(&mut self, n: usize) {
        let blank = self.blank();
        let (x, y, cols) = (self.cursor.x, self.cursor.y, self.cols);
        let row = &mut self.screen_mut()[y];
        let n = n.min(cols - x);
        for _ in 0..n {
            row.cells.insert(x, blank);
        }
        row.cells.truncate(cols);
        self.wrap_pending = false;
    }

    pub(crate) fn delete_chars(&mut self, n: usize) {
        let blank = self.blank();
        let (x, y, cols) = (self.cursor.x, self.cursor.y, self.cols);
        let row = &mut self.screen_mut()[y];
        let n = n.min(cols - x);
        row.cells.drain(x..x + n);
        row.cells.resize(cols, blank);
        self.wrap_pending = false;
    }

    pub(crate) fn insert_lines(&mut self, n: usize) {
        let y = self.cursor.y;
        if y < self.scroll_top || y > self.scroll_bottom {
            return;
        }
        let top = self.scroll_top;
        self.scroll_top = y;
        self.scroll_down(n);
        self.scroll_top = top;
        self.cursor.x = 0;
        self.wrap_pending = false;
    }

    pub(crate) fn delete_lines(&mut self, n: usize) {
        let y = self.cursor.y;
        if y < self.scroll_top || y > self.scroll_bottom {
            return;
        }
        let top = self.scroll_top;
        self.scroll_top = y;
        // deleting lines never feeds scrollback
        self.scroll_up_no_history(n);
        self.scroll_top = top;
        self.cursor.x = 0;
        self.wrap_pending = false;
    }

    fn scroll_up_no_history(&mut self, n: usize) {
        let (top, bottom) = (self.scroll_top, self.scroll_bottom);
        let n = n.min(bottom - top + 1);
        let blank = self.blank();
        for _ in 0..n {
            let mut row = self.screen_mut().remove(top);
            row.reset(blank);
            self.screen_mut().insert(bottom, row);
        }
    }

    pub(crate) fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.rows - 1);
        if top >= bottom {
            return;
        }
        self.scroll_top = top;
        self.scroll_bottom = bottom;
        self.goto(0, 0);
    }

    pub(crate) fn save_cursor(&mut self) {
        let saved = SavedCursor { cursor: self.cursor, origin: self.modes.origin, wrap_pending: self.wrap_pending };
        if self.alt_active {
            self.saved_alt = Some(saved);
        } else {
            self.saved_primary = Some(saved);
        }
    }

    pub(crate) fn restore_cursor(&mut self) {
        let saved = if self.alt_active { self.saved_alt } else { self.saved_primary };
        match saved {
            Some(s) => {
                self.cursor = s.cursor;
                self.cursor.x = self.cursor.x.min(self.cols - 1);
                self.cursor.y = self.cursor.y.min(self.rows - 1);
                self.modes.origin = s.origin;
                self.wrap_pending = s.wrap_pending;
            }
            None => {
                self.cursor = Cursor::default();
                self.modes.origin = false;
                self.wrap_pending = false;
            }
        }
    }

    pub(crate) fn enter_alt_screen(&mut self, save_cursor: bool, clear: bool) {
        if self.alt_active {
            return;
        }
        if save_cursor {
            self.save_cursor();
        }
        self.alt_active = true;
        self.display_offset = 0;
        if clear {
            let blank = self.blank();
            for r in &mut self.alternate {
                r.reset(blank);
            }
        }
    }

    pub(crate) fn leave_alt_screen(&mut self, restore_cursor: bool) {
        if !self.alt_active {
            return;
        }
        self.alt_active = false;
        if restore_cursor {
            self.restore_cursor();
        }
    }

    pub(crate) fn repeat_last(&mut self, n: usize) {
        if let Some(c) = self.last_char {
            for _ in 0..n.min(self.cols * self.rows) {
                self.print(c);
            }
        }
    }

    pub(crate) fn set_title(&mut self, title: String) {
        self.title = title;
        self.title_changed = true;
    }

    pub(crate) fn respond(&mut self, bytes: &[u8]) {
        self.responses.extend_from_slice(bytes);
    }

    pub(crate) fn ring_bell(&mut self) {
        self.bell = true;
    }

    pub(crate) fn report_cursor(&mut self) {
        let y = if self.modes.origin { self.cursor.y - self.scroll_top } else { self.cursor.y };
        let s = format!("\x1b[{};{}R", y + 1, self.cursor.x + 1);
        self.respond(s.as_bytes());
    }

    pub(crate) fn screen_alignment_test(&mut self) {
        let cell = Cell { c: 'E', ..Cell::default() };
        for r in self.screen_mut() {
            r.reset(cell);
        }
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.cursor.x = 0;
        self.cursor.y = 0;
    }

    pub(crate) fn full_reset(&mut self) {
        let (cols, rows, limit) = (self.cols, self.rows, self.scrollback_limit);
        let title = std::mem::take(&mut self.title);
        *self = Terminal::new(cols, rows);
        self.scrollback_limit = limit;
        self.title = title;
    }
}

/// When overwriting part of a wide character, blank the other half.
fn clear_wide_partner(cells: &mut [Cell], x: usize) {
    if x >= cells.len() {
        return;
    }
    if cells[x].has(flags::WIDE_SPACER) && x > 0 {
        cells[x - 1] = Cell { bg: cells[x - 1].bg, ..Cell::default() };
    }
    if cells[x].has(flags::WIDE) && x + 1 < cells.len() {
        cells[x + 1] = Cell { bg: cells[x + 1].bg, ..Cell::default() };
    }
}

/// Crop/pad a screen to `rows` x `cols` without re-wrapping, keeping the
/// cursor row visible.
fn resize_plain(screen: &mut Vec<Row>, cols: usize, rows: usize, cursor_y: &mut usize) {
    while screen.len() > rows {
        if *cursor_y + 1 < screen.len() {
            screen.pop();
        } else {
            screen.remove(0);
            *cursor_y = cursor_y.saturating_sub(1);
        }
    }
    while screen.len() < rows {
        screen.push(Row::new(cols, Cell::default()));
    }
    for r in screen.iter_mut() {
        r.resize(cols);
    }
}

fn blank_screen(cols: usize, rows: usize) -> Vec<Row> {
    (0..rows).map(|_| Row::new(cols, Cell::default())).collect()
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|i| i % 8 == 0 && i != 0).collect()
}

#[cfg(test)]
mod tests;
