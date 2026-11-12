//! Write a terminal's history back out as escape sequences, so it can be
//! replayed into a new terminal after a restart (restored scrollback).

use crate::{flags, grapheme, Cell, Color, Terminal};
use std::fmt::Write;

fn sgr(c: &Cell) -> String {
    let mut p: Vec<String> = vec!["0".into()];
    let f = c.flags;
    for (bit, code) in [
        (flags::BOLD, "1"),
        (flags::DIM, "2"),
        (flags::ITALIC, "3"),
        (flags::UNDERLINE, "4"),
        (flags::BLINK, "5"),
        (flags::INVERSE, "7"),
        (flags::HIDDEN, "8"),
        (flags::STRIKE, "9"),
        (flags::DOUBLE_UNDERLINE, "21"),
    ] {
        if f & bit != 0 {
            p.push(code.into());
        }
    }
    if f & flags::CURLY_UNDERLINE != 0 {
        p.push("4:3".into());
    }
    let color = |c: Color, base: u8| -> Option<String> {
        match c {
            Color::Default => None,
            Color::Indexed(i) if i < 8 => Some(format!("{}", base + i)),
            Color::Indexed(i) if i < 16 => Some(format!("{}", base + 60 + i - 8)),
            Color::Indexed(i) => Some(format!("{};5;{i}", base + 8)),
            Color::Rgb(r, g, b) => Some(format!("{};2;{r};{g};{b}", base + 8)),
        }
    };
    p.extend(color(c.fg, 30));
    p.extend(color(c.bg, 40));
    format!("\x1b[{}m", p.join(";"))
}

fn same_style(a: &Cell, b: &Cell) -> bool {
    a.fg == b.fg
        && a.bg == b.bg
        && a.flags & !(flags::WIDE | flags::WIDE_SPACER | flags::WRAP_PAD) == b.flags & !(flags::WIDE | flags::WIDE_SPACER | flags::WRAP_PAD)
}

impl Terminal {
    /// The last `max_lines` of history and screen (up to the cursor) as
    /// text with SGR colours. Images and hyperlinks are left out.
    pub fn dump_vt(&self, max_lines: usize) -> String {
        if self.alt_active {
            return String::new();
        }
        let last = self.scrollback.len() + self.cursor.y;
        // drop trailing blank rows (e.g. a fresh prompt line)
        let mut end = last;
        while end > 0 && self.absolute_row(end).is_some_and(|r| r.text().is_empty()) {
            end -= 1;
        }
        let start = (end + 1).saturating_sub(max_lines);
        let mut out = String::new();
        let mut cur: Option<Cell> = None;
        for line in start..=end {
            let Some(row) = self.absolute_row(line) else { break };
            // trailing default blanks aren't worth writing
            let used = row.cells.iter().rposition(|c| *c != Cell::default() && !c.has(flags::WRAP_PAD)).map_or(0, |i| i + 1);
            for c in &row.cells[..used] {
                if c.has(flags::WIDE_SPACER | flags::WRAP_PAD) {
                    continue;
                }
                if cur.is_none_or(|p| !same_style(&p, c)) {
                    out.push_str(&sgr(c));
                    cur = Some(*c);
                }
                let _ = write!(out, "{}", grapheme::text(c.c));
            }
            if !row.wrapped || line == end {
                out.push_str("\x1b[0m\r\n");
                cur = None;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::Terminal;

    #[test]
    fn round_trip_keeps_text_and_colours() {
        let mut a = Terminal::new(20, 5);
        vibex_vt::Parser::new().advance_bytes(&mut a, "plain\r\n\x1b[1;31mbold red\x1b[0m ok\r\n\x1b[38;2;1;2;3m日本\x1b[0m\r\n".as_bytes());
        let dump = a.dump_vt(100);
        let mut b = Terminal::new(20, 5);
        vibex_vt::Parser::new().advance_bytes(&mut b, dump.as_bytes());
        assert_eq!(b.screen_row(0).text(), "plain");
        assert_eq!(b.screen_row(1).text(), "bold red ok");
        assert_eq!(b.screen_row(2).text(), "日本");
        assert_eq!(a.screen_row(1).cells[0], b.screen_row(1).cells[0], "bold red kept");
        assert_eq!(a.screen_row(2).cells[0].fg, b.screen_row(2).cells[0].fg, "truecolor kept");
        // limits to the last lines
        assert_eq!(a.dump_vt(1).matches("\r\n").count(), 1);
    }
}
