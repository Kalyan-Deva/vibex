//! Copy mode: move a cursor over the screen and history with vi keys,
//! select (character, line or block) and yank, without touching the mouse.
//!
//! Motions: h j k l / arrows, w b e, 0 ^ $, gg G, H M L, Ctrl+u/d (half
//! page), Ctrl+b/f / PageUp/PageDown, counts (`5j`). Selection: v, V,
//! Ctrl+v. y / Enter copies, Esc clears the selection (or leaves), q
//! leaves, / and ? search, n / N jump between hits.

use vibex_term::{flags, Terminal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelMode {
    None,
    Char,
    Line,
    Block,
}

/// A key as copy mode sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CKey {
    Char(char),
    Ctrl(char),
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Esc,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Moved,
    Copy(String),
    Exit,
    /// Open the search bar (`true` = search backwards / older).
    Search,
    /// Jump to the next (`false`) or previous (`true`) search hit.
    SearchStep(bool),
}

pub struct CopyMode {
    /// (column, absolute line).
    pub cursor: (usize, usize),
    pub anchor: (usize, usize),
    pub mode: SelMode,
    count: usize,
    pending_g: bool,
    /// Column to return to when moving through shorter lines.
    want_col: usize,
}

fn class(c: char) -> u8 {
    if c == ' ' || c == '\t' {
        0
    } else if c.is_alphanumeric() || c == '_' {
        2
    } else {
        1
    }
}

impl CopyMode {
    /// Start on the terminal's cursor.
    pub fn new(term: &Terminal) -> Self {
        let line = term.total_lines() - term.rows() + term.cursor.y.min(term.rows() - 1);
        let cursor = (term.cursor.x.min(term.cols() - 1), line);
        Self { cursor, anchor: cursor, mode: SelMode::None, count: 0, pending_g: false, want_col: cursor.0 }
    }

    fn char_at(term: &Terminal, col: usize, line: usize) -> char {
        term.absolute_row(line).and_then(|r| r.cells.get(col)).map_or(' ', |c| if c.has(flags::WIDE_SPACER) { '\0' } else { vibex_term::grapheme::base(c.c) })
    }

    fn last_nonblank(term: &Terminal, line: usize) -> usize {
        let Some(row) = term.absolute_row(line) else { return 0 };
        row.cells.iter().rposition(|c| c.c != ' ' && !c.has(flags::WIDE_SPACER | flags::WRAP_PAD)).unwrap_or(0)
    }

    fn first_nonblank(term: &Terminal, line: usize) -> usize {
        let Some(row) = term.absolute_row(line) else { return 0 };
        row.cells.iter().position(|c| c.c != ' ').unwrap_or(0)
    }

    /// Step one cell forward/back across line ends; None at the buffer end.
    fn step(term: &Terminal, (x, l): (usize, usize), fwd: bool) -> Option<(usize, usize)> {
        let cols = term.cols();
        if fwd {
            if x + 1 < cols {
                Some((x + 1, l))
            } else if l + 1 < term.total_lines() {
                Some((0, l + 1))
            } else {
                None
            }
        } else if x > 0 {
            Some((x - 1, l))
        } else if l > 0 {
            Some((cols - 1, l - 1))
        } else {
            None
        }
    }

    fn cls(term: &Terminal, p: (usize, usize)) -> u8 {
        // the end of a row counts as a blank so words don't run across lines
        if p.0 + 1 >= term.cols() && !term.absolute_row(p.1).is_some_and(|r| r.wrapped) && Self::char_at(term, p.0, p.1) == ' ' {
            return 0;
        }
        match Self::char_at(term, p.0, p.1) {
            '\0' => 255,
            c => class(c),
        }
    }

    fn word_forward(term: &Terminal, mut p: (usize, usize)) -> (usize, usize) {
        let start = Self::cls(term, p);
        // skip the rest of this word, then blanks
        while let Some(n) = Self::step(term, p, true) {
            let c = Self::cls(term, n);
            p = n;
            if c != start && c != 255 {
                break;
            }
        }
        while Self::cls(term, p) == 0 {
            match Self::step(term, p, true) {
                Some(n) => p = n,
                None => break,
            }
        }
        p
    }

    fn word_back(term: &Terminal, mut p: (usize, usize)) -> (usize, usize) {
        // skip blanks before, then to the start of that word
        while let Some(n) = Self::step(term, p, false) {
            p = n;
            if Self::cls(term, p) != 0 {
                break;
            }
        }
        let c = Self::cls(term, p);
        while let Some(n) = Self::step(term, p, false) {
            let nc = Self::cls(term, n);
            if nc != c && nc != 255 {
                break;
            }
            p = n;
        }
        p
    }

    fn word_end(term: &Terminal, mut p: (usize, usize)) -> (usize, usize) {
        if let Some(n) = Self::step(term, p, true) {
            p = n;
        }
        while Self::cls(term, p) == 0 {
            match Self::step(term, p, true) {
                Some(n) => p = n,
                None => return p,
            }
        }
        let c = Self::cls(term, p);
        while let Some(n) = Self::step(term, p, true) {
            let nc = Self::cls(term, n);
            if nc != c && nc != 255 {
                break;
            }
            p = n;
        }
        p
    }

    fn set_line(&mut self, term: &Terminal, line: usize) {
        let line = line.min(term.total_lines().saturating_sub(1));
        let col = self.want_col.min(Self::last_nonblank(term, line)).min(term.cols() - 1);
        self.cursor = (col, line);
    }

    pub fn key(&mut self, term: &Terminal, key: CKey) -> Outcome {
        let rows = term.rows();
        let total = term.total_lines();
        if let CKey::Char(d @ '0'..='9') = key {
            if d != '0' || self.count > 0 {
                self.count = (self.count * 10 + (d as usize - '0' as usize)).min(100_000);
                return Outcome::Nothing;
            }
        }
        let n = std::mem::take(&mut self.count).max(1);
        if self.pending_g {
            self.pending_g = false;
            if key == CKey::Char('g') {
                self.want_col = 0;
                self.cursor = (0, 0);
                return Outcome::Moved;
            }
        }
        let (x, l) = self.cursor;
        let top = term.top_visible_line();
        match key {
            CKey::Char('h') | CKey::Left => {
                self.cursor.0 = x.saturating_sub(n);
                if self.cursor.0 > 0 && Self::char_at(term, self.cursor.0, l) == '\0' {
                    self.cursor.0 -= 1;
                }
                self.want_col = self.cursor.0;
            }
            CKey::Char('l') | CKey::Right => {
                let mut nx = (x + n).min(term.cols() - 1);
                if Self::char_at(term, nx, l) == '\0' {
                    nx = (nx + 1).min(term.cols() - 1);
                }
                self.cursor.0 = nx;
                self.want_col = nx;
            }
            CKey::Char('j') | CKey::Down => self.set_line(term, l + n),
            CKey::Char('k') | CKey::Up => self.set_line(term, l.saturating_sub(n)),
            CKey::Char('w') => {
                for _ in 0..n {
                    self.cursor = Self::word_forward(term, self.cursor);
                }
                self.want_col = self.cursor.0;
            }
            CKey::Char('b') => {
                for _ in 0..n {
                    self.cursor = Self::word_back(term, self.cursor);
                }
                self.want_col = self.cursor.0;
            }
            CKey::Char('e') => {
                for _ in 0..n {
                    self.cursor = Self::word_end(term, self.cursor);
                }
                self.want_col = self.cursor.0;
            }
            CKey::Char('0') | CKey::Home => {
                self.cursor.0 = 0;
                self.want_col = 0;
            }
            CKey::Char('^') => {
                self.cursor.0 = Self::first_nonblank(term, l);
                self.want_col = self.cursor.0;
            }
            CKey::Char('$') | CKey::End => {
                self.cursor.0 = Self::last_nonblank(term, l);
                self.want_col = usize::MAX;
            }
            CKey::Char('g') => {
                self.pending_g = true;
                return Outcome::Nothing;
            }
            CKey::Char('G') => {
                self.want_col = 0;
                self.set_line(term, total - 1);
            }
            CKey::Char('H') => self.set_line(term, top),
            CKey::Char('M') => self.set_line(term, top + rows / 2),
            CKey::Char('L') => self.set_line(term, top + rows - 1),
            CKey::Ctrl('u') => self.set_line(term, l.saturating_sub(rows / 2 * n)),
            CKey::Ctrl('d') => self.set_line(term, l + rows / 2 * n),
            CKey::Ctrl('b') | CKey::PageUp => self.set_line(term, l.saturating_sub(rows * n)),
            CKey::Ctrl('f') | CKey::PageDown => self.set_line(term, l + rows * n),
            CKey::Char('v') => self.toggle(SelMode::Char),
            CKey::Char('V') => self.toggle(SelMode::Line),
            CKey::Ctrl('v') => self.toggle(SelMode::Block),
            CKey::Char('o') => std::mem::swap(&mut self.cursor, &mut self.anchor),
            CKey::Char('y') | CKey::Enter => {
                return match self.text(term) {
                    Some(t) => Outcome::Copy(t),
                    None if key == CKey::Enter => Outcome::Exit,
                    None => Outcome::Nothing,
                };
            }
            CKey::Esc => {
                if self.mode == SelMode::None {
                    return Outcome::Exit;
                }
                self.mode = SelMode::None;
            }
            CKey::Char('q') | CKey::Ctrl('c') => return Outcome::Exit,
            CKey::Char('/') | CKey::Char('?') => return Outcome::Search,
            CKey::Char('n') => return Outcome::SearchStep(true),
            CKey::Char('N') => return Outcome::SearchStep(false),
            _ => return Outcome::Nothing,
        }
        Outcome::Moved
    }

    fn toggle(&mut self, mode: SelMode) {
        if self.mode == mode {
            self.mode = SelMode::None;
        } else {
            if self.mode == SelMode::None {
                self.anchor = self.cursor;
            }
            self.mode = mode;
        }
    }

    /// Selected region as (start, end, block), in absolute coordinates.
    pub fn region(&self, term: &Terminal) -> Option<((usize, usize), (usize, usize), bool)> {
        let (a, c) = (self.anchor, self.cursor);
        let (s, e) = if (a.1, a.0) <= (c.1, c.0) { (a, c) } else { (c, a) };
        match self.mode {
            SelMode::None => None,
            SelMode::Char => Some((s, e, false)),
            SelMode::Line => Some(((0, s.1), (term.cols() - 1, e.1), false)),
            SelMode::Block => {
                let (l0, l1) = (a.1.min(c.1), a.1.max(c.1));
                Some(((a.0.min(c.0), l0), (a.0.max(c.0), l1), true))
            }
        }
    }

    pub fn text(&self, term: &Terminal) -> Option<String> {
        let (s, e, block) = self.region(term)?;
        Some(if block { term.text_block(s, e) } else { term.text_range(s, e) })
    }

    pub fn label(&self) -> String {
        let mode = match self.mode {
            SelMode::None => "COPY",
            SelMode::Char => "COPY · VISUAL",
            SelMode::Line => "COPY · VISUAL LINE",
            SelMode::Block => "COPY · VISUAL BLOCK",
        };
        format!("{mode}   v/V/^V select · y copy · / search · q quit")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(text: &str) -> Terminal {
        let mut t = Terminal::new(20, 4);
        vibex_vt::Parser::new().advance_bytes(&mut t, text.as_bytes());
        t
    }

    fn keys(cm: &mut CopyMode, t: &Terminal, s: &str) -> Outcome {
        let mut last = Outcome::Nothing;
        for c in s.chars() {
            last = cm.key(t, CKey::Char(c));
        }
        last
    }

    #[test]
    fn starts_on_cursor_and_moves() {
        let t = term("hello world\r\nfoo-bar baz");
        let mut cm = CopyMode::new(&t);
        assert_eq!(cm.cursor, (11, 1));
        keys(&mut cm, &t, "0");
        assert_eq!(cm.cursor, (0, 1));
        keys(&mut cm, &t, "w");
        assert_eq!(cm.cursor, (3, 1), "punctuation is its own word");
        keys(&mut cm, &t, "ww");
        assert_eq!(cm.cursor, (8, 1));
        keys(&mut cm, &t, "b");
        assert_eq!(cm.cursor, (4, 1));
        keys(&mut cm, &t, "k$");
        assert_eq!(cm.cursor, (10, 0));
        keys(&mut cm, &t, "gg");
        assert_eq!(cm.cursor, (0, 0));
        keys(&mut cm, &t, "2l");
        assert_eq!(cm.cursor, (2, 0));
        keys(&mut cm, &t, "e");
        assert_eq!(cm.cursor, (4, 0));
    }

    #[test]
    fn select_and_yank() {
        let t = term("hello world\r\nfoo-bar baz\r\nthird");
        let mut cm = CopyMode::new(&t);
        keys(&mut cm, &t, "gg");
        assert_eq!(keys(&mut cm, &t, "vey"), Outcome::Copy("hello".into()));
        let mut cm = CopyMode::new(&t);
        keys(&mut cm, &t, "ggjVj");
        assert_eq!(cm.text(&t).unwrap(), "foo-bar baz\nthird");
        let mut cm = CopyMode::new(&t);
        keys(&mut cm, &t, "gg");
        cm.key(&t, CKey::Ctrl('v'));
        keys(&mut cm, &t, "jll");
        assert_eq!(cm.text(&t).unwrap(), "hel\nfoo");
        assert_eq!(cm.key(&t, CKey::Esc), Outcome::Moved, "first Esc drops the selection");
        assert_eq!(cm.key(&t, CKey::Esc), Outcome::Exit);
    }
}
