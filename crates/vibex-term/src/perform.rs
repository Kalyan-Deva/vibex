//! Maps parsed VT events onto terminal operations.

use crate::{flags, Cell, Color, CursorShape, MouseMode, Terminal};
use vibex_vt::{Params, Perform};

impl Perform for Terminal {
    fn print(&mut self, c: char) {
        self.touch();
        Terminal::print(self, c);
    }

    fn execute(&mut self, byte: u8) {
        self.touch();
        match byte {
            0x07 => self.ring_bell(),
            0x08 => self.backspace(),
            0x09 => self.tab(1),
            0x0a..=0x0c => self.linefeed(),
            0x0d => self.carriage_return(),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, inter: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        // reports and queries answer without changing the screen
        let query = matches!((inter, action), ([], 'c') | ([b'>'], 'c') | ([], 'n') | ([b'?'], 'n') | ([], 't') | ([b'?'], 'u') | ([b'?'], 'S'))
            || inter.last() == Some(&b'$');
        if !query {
            self.touch();
        }
        let n = |i: usize| params.get_or(i, 1) as usize;
        match (inter, action) {
            ([], '@') => self.insert_chars(n(0)),
            ([], 'A') => self.move_up(n(0)),
            ([], 'B') | ([], 'e') => self.move_down(n(0)),
            ([], 'C') | ([], 'a') => self.move_right(n(0)),
            ([], 'D') => self.move_left(n(0)),
            ([], 'E') => {
                self.move_down(n(0));
                self.carriage_return();
            }
            ([], 'F') => {
                self.move_up(n(0));
                self.carriage_return();
            }
            ([], 'G') | ([], '`') => self.goto_col(n(0) - 1),
            ([], 'H') | ([], 'f') => self.goto(n(1) - 1, n(0) - 1),
            ([], 'I') => self.tab(n(0)),
            ([], 'J') | ([b'?'], 'J') => self.erase_display(params.raw(0).unwrap_or(0)),
            ([], 'K') | ([b'?'], 'K') => self.erase_line(params.raw(0).unwrap_or(0)),
            ([], 'L') => self.insert_lines(n(0)),
            ([], 'M') => self.delete_lines(n(0)),
            ([], 'P') => self.delete_chars(n(0)),
            ([], 'S') => self.scroll_up(n(0)),
            ([], 'T') => self.scroll_down(n(0)),
            ([], 'X') => self.erase_chars(n(0)),
            ([], 'Z') => self.back_tab(n(0)),
            ([], 'b') => self.repeat_last(n(0)),
            ([], 'c') if params.get_or(0, 0) == 0 => {
                // DA1: VT220-class with sixel graphics and ANSI colour
                self.respond(b"\x1b[?62;4;22c");
            }
            // XTWINOPS size reports (programs size sixel/kitty images by them)
            ([], 't') => {
                let (cw, ch) = self.cell_pixels();
                let (cols, rows) = (self.cols() as u32, self.rows() as u32);
                let reply = match params.raw(0).unwrap_or(0) {
                    14 => format!("\x1b[4;{};{}t", rows * ch, cols * cw),
                    16 => format!("\x1b[6;{ch};{cw}t"),
                    18 => format!("\x1b[8;{rows};{cols}t"),
                    _ => return,
                };
                self.respond(reply.as_bytes());
            }
            // XTSMGRAPHICS: sixel colour registers / maximum geometry
            ([b'?'], 'S') => {
                let item = params.raw(0).unwrap_or(0);
                let action = params.raw(1).unwrap_or(0);
                let reply = match (item, action) {
                    (1, 1 | 4) => "\x1b[?1;0;256S".to_string(),
                    (2, 1 | 4) => "\x1b[?2;0;4096;4096S".to_string(),
                    (1 | 2, _) => format!("\x1b[?{item};3;0S"),
                    _ => format!("\x1b[?{item};1;0S"),
                };
                self.respond(reply.as_bytes());
            }
            ([b'>'], 'c') => self.respond(b"\x1b[>0;10;1c"),
            ([], 'd') => self.goto_row(n(0) - 1),
            ([], 'g') => self.clear_tabs(params.raw(0).unwrap_or(0)),
            ([], 'h') => self.set_ansi_modes(params, true),
            ([], 'l') => self.set_ansi_modes(params, false),
            ([b'?'], 'h') => self.set_private_modes(params, true),
            ([b'?'], 'l') => self.set_private_modes(params, false),
            ([], 'm') => self.sgr(params),
            ([], 'n') => match params.raw(0).unwrap_or(0) {
                5 => self.respond(b"\x1b[0n"),
                6 => self.report_cursor(),
                _ => {}
            },
            ([], 'r') => {
                let top = params.get_or(0, 1) as usize - 1;
                let bottom = params.get_or(1, self.rows() as u16) as usize - 1;
                self.set_scroll_region(top, bottom);
            }
            ([], 's') => self.save_cursor(),
            ([], 'u') => self.restore_cursor(),
            // kitty keyboard protocol
            ([b'?'], 'u') => self.kitty_query(),
            ([b'>'], 'u') => self.kitty_push(params.raw(0).unwrap_or(0) as u8),
            ([b'<'], 'u') => self.kitty_pop(params.get_or(0, 1) as usize),
            ([b'='], 'u') => self.kitty_set(params.raw(0).unwrap_or(0) as u8, params.get_or(1, 1)),
            ([b' '], 'q') => {
                let (shape, blink) = match params.raw(0).unwrap_or(0) {
                    0 | 1 => (CursorShape::Block, true),
                    2 => (CursorShape::Block, false),
                    3 => (CursorShape::Underline, true),
                    4 => (CursorShape::Underline, false),
                    5 => (CursorShape::Bar, true),
                    6 => (CursorShape::Bar, false),
                    _ => return,
                };
                self.cursor_shape = shape;
                self.modes.cursor_blink = blink;
            }
            ([b'!'], 'p') => {
                // DECSTR soft reset
                self.modes = Default::default();
                self.cursor.pen = Default::default();
                let rows = self.rows();
                self.set_scroll_region(0, rows - 1);
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, inter: &[u8], ignore: bool, action: char) {
        self.sixel = None;
        if !ignore && inter.is_empty() && action == 'q' {
            self.sixel = Some(crate::sixel::Sixel::new());
        }
    }

    fn put(&mut self, byte: u8) {
        if let Some(s) = self.sixel.as_mut() {
            s.put(byte);
        }
    }

    fn unhook(&mut self) {
        self.touch();
        if let Some(img) = self.sixel.take().and_then(|s| s.finish()) {
            let (w, h) = (img.width as f32, img.height as f32);
            self.place_image(crate::image::Placement {
                image: img,
                source: crate::image::Source::Sixel,
                src: [0.0, 0.0, w, h],
                cols: 0,
                rows: 0,
                z: 0,
                cursor: crate::image::CursorAfter::BelowImage,
            });
        }
    }

    fn apc_dispatch(&mut self, data: &[u8]) {
        self.touch();
        self.kitty_apc(data);
    }

    fn esc_dispatch(&mut self, inter: &[u8], ignore: bool, byte: u8) {
        self.touch();
        if ignore {
            return;
        }
        match (inter, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.carriage_return();
                self.linefeed();
            }
            ([], b'H') => self.set_tab(),
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.full_reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([b'#'], b'8') => self.screen_alignment_test(),
            // charset designation (ESC ( B etc.): UTF-8 only, ignored
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell: bool) {
        self.touch();
        let Some(kind) = params.first() else { return };
        match *kind {
            b"0" | b"2" => {
                if params.len() > 1 {
                    // the title itself may contain ';'
                    let title = params[1..].join(&b';');
                    self.set_title(String::from_utf8_lossy(&title).into_owned());
                }
            }
            // OSC 7 ; file://host/path
            b"7" => {
                if let Some(p) = params.get(1).and_then(|u| parse_file_url(&String::from_utf8_lossy(u))) {
                    self.set_cwd(p);
                }
            }
            // OSC 9 ; 9 ; "C:\path"   (ConEmu / Windows Terminal style)
            b"9" if params.get(1) == Some(&&b"9"[..]) => {
                if let Some(p) = params.get(2) {
                    let p = String::from_utf8_lossy(p).trim_matches('"').to_string();
                    if !p.is_empty() {
                        self.set_cwd(p);
                    }
                }
            }
            // OSC 9 ; message   (iTerm2 / Windows Terminal notification);
            // numeric subcommands (9;4 progress etc.) are not messages
            b"9" => {
                if let Some(first) = params.get(1) {
                    if !first.iter().all(|b| b.is_ascii_digit()) {
                        let body = String::from_utf8_lossy(&params[1..].join(&b';')).into_owned();
                        self.notify(String::new(), body);
                    }
                }
            }
            // OSC 777 ; notify ; title ; body   (urxvt / foot)
            b"777" if params.get(1) == Some(&&b"notify"[..]) => {
                let title = params.get(2).map(|t| String::from_utf8_lossy(t).into_owned()).unwrap_or_default();
                let body = if params.len() > 3 { String::from_utf8_lossy(&params[3..].join(&b';')).into_owned() } else { String::new() };
                self.notify(title, body);
            }
            // OSC 8 ; params ; URI   (hyperlink; empty URI ends it)
            b"8" => {
                let uri = if params.len() > 2 { String::from_utf8_lossy(&params[2..].join(&b';')).into_owned() } else { String::new() };
                self.set_hyperlink(&uri);
            }
            // OSC 52 ; selection ; base64 | ?
            b"52" => {
                if let Some(data) = params.get(2) {
                    self.osc52(&String::from_utf8_lossy(data));
                }
            }
            // OSC 133 ; A|B|C|D[;exit]   (FinalTerm shell integration)
            // VS Code's OSC 633;E;<command line>: the exact command text
            b"633" if params.get(1) == Some(&&b"E"[..]) => {
                let raw = String::from_utf8_lossy(&params[2.min(params.len())..].join(&b';')).into_owned();
                self.set_command_text(crate::blocks::unescape_633(&raw));
            }
            b"133" => match params.get(1).and_then(|p| p.first()) {
                Some(b'A') => self.mark_prompt(),
                Some(b'B') => self.mark_input(),
                Some(b'C') => self.mark_output(),
                Some(b'D') => {
                    let code = params.get(2).and_then(|c| std::str::from_utf8(c).ok()).and_then(|c| c.parse().ok());
                    self.finish_command(code);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// `file://host/C:/Users/x` → `C:\Users\x`; `file://host/home/x` → `/home/x`.
pub(crate) fn parse_file_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let decoded = percent_decode(path);
    let b = decoded.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        Some(decoded[1..].replace('/', "\\"))
    } else {
        Some(decoded)
    }
}

fn percent_decode(s: &str) -> String {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Terminal {
    fn set_ansi_modes(&mut self, params: &Params, on: bool) {
        for g in params.iter() {
            match g[0] {
                4 => self.modes.insert = on,
                20 => self.modes.newline = on,
                _ => {}
            }
        }
    }

    fn set_private_modes(&mut self, params: &Params, on: bool) {
        for g in params.iter() {
            match g[0] {
                1 => self.modes.app_cursor = on,
                5 => self.modes.reverse_screen = on,
                6 => {
                    self.modes.origin = on;
                    self.goto(0, 0);
                }
                7 => self.modes.autowrap = on,
                12 => self.modes.cursor_blink = on,
                25 => self.modes.show_cursor = on,
                47 | 1047 => {
                    if on {
                        self.enter_alt_screen(false, g[0] == 1047);
                    } else {
                        self.leave_alt_screen(false);
                    }
                }
                1048 => {
                    if on {
                        self.save_cursor()
                    } else {
                        self.restore_cursor()
                    }
                }
                1049 => {
                    if on {
                        self.enter_alt_screen(true, true);
                    } else {
                        self.leave_alt_screen(true);
                    }
                }
                1000 => self.modes.mouse_report = if on { MouseMode::Click } else { MouseMode::Off },
                1002 => self.modes.mouse_report = if on { MouseMode::Drag } else { MouseMode::Off },
                1003 => self.modes.mouse_report = if on { MouseMode::Motion } else { MouseMode::Off },
                1004 => self.modes.focus_events = on,
                1006 => self.modes.mouse_sgr = on,
                2004 => self.modes.bracketed_paste = on,
                _ => {}
            }
        }
    }

    fn sgr(&mut self, params: &Params) {
        let pen = &mut self.cursor.pen;
        // SGR reset clears attributes, not an open OSC 8 link
        if params.is_empty() {
            *pen = Cell { link: pen.link, ..Default::default() };
            return;
        }
        let mut it = params.iter().peekable();
        while let Some(g) = it.next() {
            match g[0] {
                0 => *pen = Cell { link: pen.link, ..Default::default() },
                1 => pen.flags |= flags::BOLD,
                2 => pen.flags |= flags::DIM,
                3 => pen.flags |= flags::ITALIC,
                4 => {
                    pen.flags &= !flags::ANY_UNDERLINE;
                    match g.get(1).copied().unwrap_or(1) {
                        0 => {}
                        2 => pen.flags |= flags::DOUBLE_UNDERLINE,
                        3 => pen.flags |= flags::CURLY_UNDERLINE,
                        _ => pen.flags |= flags::UNDERLINE,
                    }
                }
                5 | 6 => pen.flags |= flags::BLINK,
                7 => pen.flags |= flags::INVERSE,
                8 => pen.flags |= flags::HIDDEN,
                9 => pen.flags |= flags::STRIKE,
                21 => {
                    pen.flags &= !flags::ANY_UNDERLINE;
                    pen.flags |= flags::DOUBLE_UNDERLINE;
                }
                22 => pen.flags &= !(flags::BOLD | flags::DIM),
                23 => pen.flags &= !flags::ITALIC,
                24 => pen.flags &= !flags::ANY_UNDERLINE,
                25 => pen.flags &= !flags::BLINK,
                27 => pen.flags &= !flags::INVERSE,
                28 => pen.flags &= !flags::HIDDEN,
                29 => pen.flags &= !flags::STRIKE,
                v @ 30..=37 => pen.fg = Color::Indexed((v - 30) as u8),
                38 => {
                    if let Some(c) = extended_color(g, &mut it) {
                        pen.fg = c;
                    }
                }
                39 => pen.fg = Color::Default,
                v @ 40..=47 => pen.bg = Color::Indexed((v - 40) as u8),
                48 => {
                    if let Some(c) = extended_color(g, &mut it) {
                        pen.bg = c;
                    }
                }
                49 => pen.bg = Color::Default,
                v @ 90..=97 => pen.fg = Color::Indexed((v - 90 + 8) as u8),
                v @ 100..=107 => pen.bg = Color::Indexed((v - 100 + 8) as u8),
                _ => {}
            }
        }
    }
}

/// Parse `38;5;n`, `38;2;r;g;b` and the colon forms `38:5:n`,
/// `38:2::r:g:b` / `38:2:r:g:b`.
fn extended_color<'a, I>(group: &[u16], rest: &mut std::iter::Peekable<I>) -> Option<Color>
where
    I: Iterator<Item = &'a [u16]>,
{
    if group.len() > 1 {
        // colon form
        return match group[1] {
            5 => group.get(2).map(|&i| Color::Indexed(i.min(255) as u8)),
            2 => {
                let v = &group[2..];
                let rgb = if v.len() >= 4 {
                    &v[1..4]
                } else if v.len() == 3 {
                    v
                } else {
                    return None;
                };
                Some(Color::Rgb(rgb[0].min(255) as u8, rgb[1].min(255) as u8, rgb[2].min(255) as u8))
            }
            _ => None,
        };
    }
    let mut next = || rest.next().map(|g| g[0]);
    match next()? {
        5 => next().map(|i| Color::Indexed(i.min(255) as u8)),
        2 => {
            let r = next()?;
            let g = next()?;
            let b = next()?;
            Some(Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8))
        }
        _ => None,
    }
}
