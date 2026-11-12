//! Saving part of a terminal: a line range (or a selection, or one
//! command's block) as plain text, ANSI, HTML, Markdown, SVG or a PNG in
//! the terminal's own theme and font.
//!
//! Everything goes through `take` first, which copies the cells and masks
//! secrets (the secret guard's patterns, also across soft wraps), so no
//! format can leak a key the screen showed.

use crate::secrets::Guard;
use std::fmt::Write;
use std::time::Duration;
use vibex_render::Theme;
use vibex_term::{flags, grapheme, Cell, Row, Terminal};

/// What a masked secret's cells become.
const MASK: char = '\u{2022}';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Text,
    Ansi,
    Html,
    Markdown,
    Svg,
    Png,
}

impl Format {
    pub const ALL: [Format; 6] = [Format::Png, Format::Text, Format::Ansi, Format::Html, Format::Svg, Format::Markdown];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Text => "txt",
            Format::Ansi => "ans",
            Format::Html => "html",
            Format::Markdown => "md",
            Format::Svg => "svg",
            Format::Png => "png",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG image",
            Format::Text => "Plain text",
            Format::Ansi => "ANSI (keeps colours)",
            Format::Html => "HTML page",
            Format::Svg => "SVG image",
            Format::Markdown => "Markdown code block",
        }
    }

    pub fn parse(s: &str) -> Option<Format> {
        Some(match s.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "txt" | "text" => Format::Text,
            "ans" | "ansi" => Format::Ansi,
            "html" | "htm" => Format::Html,
            "md" | "markdown" => Format::Markdown,
            "svg" => Format::Svg,
            "png" | "image" => Format::Png,
            _ => return None,
        })
    }
}

/// The title bar over a picture of one command.
#[derive(Clone, Debug, Default)]
pub struct Caption {
    pub command: String,
    pub exit: Option<i32>,
    pub duration: Option<Duration>,
}

impl Caption {
    /// "exit 0 · 1.4s"
    pub fn status(&self) -> String {
        let mut parts = Vec::new();
        if let Some(e) = self.exit {
            parts.push(format!("exit {e}"));
        }
        if let Some(d) = self.duration {
            parts.push(short_duration(d));
        }
        parts.join("  \u{00b7}  ")
    }
}

fn short_duration(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 1.0 {
        format!("{}ms", d.as_millis())
    } else if s < 60.0 {
        format!("{s:.1}s")
    } else {
        let m = (s / 60.0).floor() as u64;
        format!("{m}m {:02}s", (s as u64) % 60)
    }
}

/// A copy of some lines, secrets masked.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: Vec<Row>,
    /// Masked spans: (row, first column, last column), inclusive.
    pub masked: Vec<(usize, usize, usize)>,
    pub caption: Option<Caption>,
}

/// Copy lines `from.1..=to.1` (columns `from.0` on the first and up to
/// `to.0` on the last), masking what `guard` matches. `tight` trims the
/// width to the longest line (for pictures of a selection or a command).
pub fn take(term: &Terminal, from: (usize, usize), to: (usize, usize), guard: Option<&Guard>, tight: bool) -> Snapshot {
    let (from, to) = if (from.1, from.0) <= (to.1, to.0) { (from, to) } else { (to, from) };
    let last = to.1.min(term.total_lines().saturating_sub(1));
    let mut rows: Vec<Row> = Vec::new();
    for line in from.1..=last {
        let Some(r) = term.absolute_row(line) else { break };
        let mut r = r.clone();
        if line == from.1 {
            for c in r.cells.iter_mut().take(from.0) {
                *c = Cell::default();
            }
        }
        if line == to.1 {
            for c in r.cells.iter_mut().skip(to.0 + 1) {
                *c = Cell::default();
            }
            r.wrapped = false;
        }
        rows.push(r);
    }
    let mut masked = Vec::new();
    if let Some(g) = guard {
        for m in term.find_regex_wrapped(&g.combined, from.1..last + 1) {
            let Some(row) = rows.get_mut(m.line - from.1) else { continue };
            let end = m.end.min(row.cells.len().saturating_sub(1));
            for c in &mut row.cells[m.start.min(end)..=end] {
                c.c = MASK;
                c.flags &= !(flags::WIDE | flags::WIDE_SPACER | flags::HIDDEN);
            }
            masked.push((m.line - from.1, m.start, end));
        }
    }
    // no blank rows at either end
    let blank = |r: &Row| r.text().is_empty() && r.images.is_empty();
    while rows.last().is_some_and(blank) && rows.len() > 1 {
        rows.pop();
    }
    let lead = rows.iter().take_while(|r| blank(r)).count().min(rows.len().saturating_sub(1));
    if lead > 0 {
        rows.drain(..lead);
        masked = masked.into_iter().filter(|m| m.0 >= lead).map(|(r, a, b)| (r - lead, a, b)).collect();
    }
    let mut cols = term.cols();
    if tight {
        let used = rows.iter().map(used_cols).max().unwrap_or(0);
        cols = (used + 1).clamp(20, term.cols().max(20));
        for r in &mut rows {
            r.cells.truncate(cols);
            if r.cells.len() < cols {
                r.cells.resize(cols, Cell::default());
            }
            // a wrap at the old width isn't one at the new
            if used + 1 < term.cols() {
                r.wrapped = false;
            }
        }
    }
    Snapshot { cols, rows, masked, caption: None }
}

/// Columns in use (trailing default blanks don't count).
fn used_cols(r: &Row) -> usize {
    r.cells.iter().rposition(|c| *c != Cell::default() && !c.has(flags::WRAP_PAD)).map_or(0, |i| i + 1)
}

impl Snapshot {
    pub fn terminal(&self) -> Terminal {
        Terminal::from_rows(self.cols, self.rows.clone())
    }

    /// Plain text: soft-wrapped rows joined, trailing blanks trimmed.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, r) in self.rows.iter().enumerate() {
            let s: String = r.cells.iter().filter(|c| !c.has(flags::WIDE_SPACER | flags::WRAP_PAD)).map(|c| grapheme::text(c.c)).collect();
            if r.wrapped && i + 1 < self.rows.len() {
                out.push_str(&s);
            } else {
                out.push_str(s.trim_end());
                out.push('\n');
            }
        }
        out
    }

    pub fn ansi(&self) -> String {
        let mut s = self.terminal().dump_vt(usize::MAX);
        if let Some(c) = &self.caption {
            s.insert_str(0, &format!("\x1b[1m# {}\x1b[0m  \x1b[2m{}\x1b[0m\r\n", c.command, c.status()));
        }
        s
    }

    pub fn markdown(&self) -> String {
        let text = self.text();
        // a fence longer than any run of backticks inside
        let mut fence = "```".to_string();
        while text.contains(&fence) {
            fence.push('`');
        }
        let mut out = String::new();
        if let Some(c) = &self.caption {
            let _ = writeln!(out, "**`{}`** {}\n", c.command.replace('`', "'"), c.status());
        }
        let _ = write!(out, "{fence}console\n{}{fence}\n", text);
        out
    }

    /// Runs of cells with one look on a row: (first column, cells, text, style).
    fn runs<'a>(&'a self, r: &'a Row) -> Vec<(usize, usize, String, Cell)> {
        let used = used_cols(r);
        let mut out: Vec<(usize, usize, String, Cell)> = Vec::new();
        for (x, c) in r.cells.iter().enumerate().take(used) {
            if c.has(flags::WIDE_SPACER | flags::WRAP_PAD) {
                if let Some(last) = out.last_mut() {
                    last.1 += 1;
                }
                continue;
            }
            match out.last_mut() {
                Some(last) if same_look(&last.3, c) => {
                    last.1 += 1;
                    last.2.push_str(&grapheme::text(c.c));
                }
                _ => out.push((x, 1, grapheme::text(c.c), *c)),
            }
        }
        out
    }

    /// (fg, bg) for a cell, inverse and hidden applied.
    fn colors(theme: &Theme, c: &Cell) -> (u32, u32) {
        let mut fg = theme.resolve(c.fg, theme.foreground);
        let mut bg = theme.resolve(c.bg, theme.background);
        if c.has(flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if c.has(flags::DIM) {
            fg = vibex_render::blend(bg, fg, 150);
        }
        if c.has(flags::HIDDEN) {
            fg = bg;
        }
        (fg, bg)
    }

    pub fn html(&self, theme: &Theme, font: &str) -> String {
        let mut body = String::new();
        for (i, r) in self.rows.iter().enumerate() {
            for (_, _, text, c) in self.runs(r) {
                let (fg, bg) = Self::colors(theme, &c);
                let mut style = String::new();
                if fg != theme.foreground {
                    let _ = write!(style, "color:{};", hex(fg));
                }
                if bg != theme.background {
                    let _ = write!(style, "background:{};", hex(bg));
                }
                if c.has(flags::BOLD) {
                    style.push_str("font-weight:bold;");
                }
                if c.has(flags::ITALIC) {
                    style.push_str("font-style:italic;");
                }
                let mut deco = Vec::new();
                if c.has(flags::ANY_UNDERLINE) {
                    deco.push("underline");
                }
                if c.has(flags::STRIKE) {
                    deco.push("line-through");
                }
                if !deco.is_empty() {
                    let _ = write!(style, "text-decoration:{};", deco.join(" "));
                }
                if c.c == MASK {
                    style.push_str("filter:blur(2px);");
                }
                if style.is_empty() {
                    body.push_str(&esc(&text));
                } else {
                    let _ = write!(body, "<span style=\"{style}\">{}</span>", esc(&text));
                }
            }
            if !(r.wrapped && i + 1 < self.rows.len()) {
                body.push('\n');
            }
        }
        let caption = self.caption.as_ref().map_or(String::new(), |c| {
            format!(
                "<div class=\"bar\"><span class=\"dot\" style=\"background:{}\"></span><b>{}</b><span class=\"st\">{}</span></div>\n",
                hex(status_color(c.exit)),
                esc(&c.command),
                esc(&c.status())
            )
        });
        format!(
            "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{title}</title>\n<style>\nbody{{margin:0;padding:24px;background:{page}}}\n.term{{background:{bg};color:{fg};border-radius:10px;overflow:hidden;box-shadow:0 8px 28px #0006;display:inline-block;min-width:min(100%,40ch)}}\n.bar{{display:flex;gap:10px;align-items:center;padding:9px 14px;background:{bar};border-bottom:1px solid {line};font:13px system-ui,sans-serif}}\n.dot{{width:9px;height:9px;border-radius:50%}}\n.st{{margin-left:auto;opacity:.6}}\npre{{margin:0;padding:12px 16px;font:14px/1.35 {font},Consolas,monospace;white-space:pre}}\n</style></head>\n<body><div class=\"term\">\n{caption}<pre>{body}</pre></div></body></html>\n",
            title = esc(self.caption.as_ref().map_or("Vibex", |c| c.command.as_str())),
            page = hex(vibex_render::blend(theme.background, 0, 160)),
            bg = hex(theme.background),
            fg = hex(theme.foreground),
            bar = hex(vibex_render::blend(theme.background, theme.foreground, 16)),
            line = hex(vibex_render::blend(theme.background, theme.foreground, 46)),
            font = css_font(font),
        )
    }

    /// A vector picture: a rect per background run, a text run per look,
    /// stretched to the cell grid so columns line up in any font.
    pub fn svg(&self, theme: &Theme, font: &str) -> String {
        let (cw, ch, pad) = (8.4f32, 18.0f32, 14.0f32);
        let bar = if self.caption.is_some() { 34.0 } else { 0.0 };
        let w = self.cols as f32 * cw + 2.0 * pad;
        let h = self.rows.len() as f32 * ch + 2.0 * pad + bar;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.1} {h:.1}\" font-family=\"{}, Consolas, monospace\" font-size=\"14\">",
            esc(&css_font(font).replace('\'', ""))
        );
        let _ = writeln!(out, "<rect width=\"100%\" height=\"100%\" rx=\"10\" fill=\"{}\"/>", hex(theme.background));
        if let Some(c) = &self.caption {
            let _ = writeln!(
                out,
                "<path d=\"M0 10a10 10 0 0 1 10-10h{:.1}a10 10 0 0 1 10 10v{:.1}H0z\" fill=\"{}\"/>",
                w - 20.0,
                bar - 10.0,
                hex(vibex_render::blend(theme.background, theme.foreground, 16))
            );
            let _ = writeln!(
                out,
                "<rect y=\"{:.1}\" width=\"{w:.1}\" height=\"1\" fill=\"{}\"/>",
                bar - 1.0,
                hex(vibex_render::blend(theme.background, theme.foreground, 46))
            );
            let _ = writeln!(out, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"4.5\" fill=\"{}\"/>", pad + 4.0, bar / 2.0, hex(status_color(c.exit)));
            let _ = writeln!(
                out,
                "<text x=\"{:.1}\" y=\"{:.1}\" font-family=\"system-ui, sans-serif\" font-size=\"13\" font-weight=\"bold\" fill=\"{}\">{}</text>",
                pad + 18.0,
                bar / 2.0 + 4.5,
                hex(theme.foreground),
                esc(&c.command)
            );
            let _ = writeln!(
                out,
                "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" font-family=\"system-ui, sans-serif\" font-size=\"12\" fill=\"{}\">{}</text>",
                w - pad,
                bar / 2.0 + 4.0,
                hex(vibex_render::blend(theme.background, theme.foreground, 150)),
                esc(&c.status())
            );
        }
        for (y, r) in self.rows.iter().enumerate() {
            let top = pad + bar + y as f32 * ch;
            for (x, n, text, c) in self.runs(r) {
                let (fg, bg) = Self::colors(theme, &c);
                let left = pad + x as f32 * cw;
                if bg != theme.background {
                    let _ = writeln!(out, "<rect x=\"{left:.1}\" y=\"{top:.1}\" width=\"{:.1}\" height=\"{ch:.1}\" fill=\"{}\"/>", n as f32 * cw, hex(bg));
                }
                if text.trim().is_empty() {
                    continue;
                }
                let mut attrs = String::new();
                if c.has(flags::BOLD) {
                    attrs.push_str(" font-weight=\"bold\"");
                }
                if c.has(flags::ITALIC) {
                    attrs.push_str(" font-style=\"italic\"");
                }
                if c.has(flags::ANY_UNDERLINE) {
                    attrs.push_str(" text-decoration=\"underline\"");
                } else if c.has(flags::STRIKE) {
                    attrs.push_str(" text-decoration=\"line-through\"");
                }
                let _ = writeln!(
                    out,
                    "<text x=\"{left:.1}\" y=\"{:.1}\" fill=\"{}\" textLength=\"{:.1}\" lengthAdjust=\"spacingAndGlyphs\" xml:space=\"preserve\"{attrs}>{}</text>",
                    top + ch * 0.74,
                    hex(fg),
                    n as f32 * cw,
                    esc(&text)
                );
            }
        }
        out.push_str("</svg>\n");
        out
    }
}

fn same_look(a: &Cell, b: &Cell) -> bool {
    let keep = !(flags::WIDE | flags::WIDE_SPACER | flags::WRAP_PAD);
    a.fg == b.fg && a.bg == b.bg && a.flags & keep == b.flags & keep && (a.c == MASK) == (b.c == MASK)
}

/// Green for exit 0, red for a failure, grey when unknown.
pub fn status_color(exit: Option<i32>) -> u32 {
    match exit {
        Some(0) => 0x3fcf8e,
        Some(_) => 0xf0616d,
        None => 0x8a93a6,
    }
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xff_ffff)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn css_font(font: &str) -> String {
    let f = font.trim();
    if f.is_empty() {
        "'Cascadia Mono'".into()
    } else {
        format!("'{}'", f.replace(['\'', '"', ';', '<', '>', '{', '}'], ""))
    }
}

/// `px` (0x00RRGGBB, `w` wide) as a PNG.
pub fn png(px: &[u32], w: usize, h: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        let mut data = Vec::with_capacity(w * h * 3);
        for &p in px {
            data.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
        wr.write_image_data(&data).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Where saved pictures and files go: Pictures\Vibex.
pub fn folder() -> std::path::PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(std::path::PathBuf::from).unwrap_or_default();
    home.join("Pictures").join("Vibex")
}

/// A fresh file name there: vibex-2026-11-12-140233.png (or -2, -3 …).
pub fn new_path(what: &str, f: Format) -> std::path::PathBuf {
    let dir = folder();
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // "2026-11-12 14:02:33Z" -> "2026-11-12-140233"
    let stamp: String = crate::audit::iso(secs).replace(' ', "-").chars().filter(|c| *c != ':' && *c != 'Z').collect();
    let base = format!("vibex-{what}-{stamp}");
    let mut p = dir.join(format!("{base}.{}", f.ext()));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{base}-{n}.{}", f.ext()));
        n += 1;
    }
    p
}

/// Put a picture on the clipboard, both as a bitmap and as PNG (which
/// browsers and chat apps prefer).
#[cfg(windows)]
pub fn copy_image(px: &[u32], w: usize, h: usize) -> Result<(), String> {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    const CF_DIB: u32 = 8;
    // a DIB: BITMAPINFOHEADER then bottom-up BGRX rows
    let mut dib = Vec::with_capacity(40 + w * h * 4);
    for v in [40u32, w as u32, h as u32] {
        dib.extend_from_slice(&v.to_le_bytes());
    }
    dib.extend_from_slice(&1u16.to_le_bytes()); // planes
    dib.extend_from_slice(&32u16.to_le_bytes()); // bits
    dib.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    dib.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    for _ in 0..4 {
        dib.extend_from_slice(&0u32.to_le_bytes());
    }
    for y in (0..h).rev() {
        for &p in &px[y * w..(y + 1) * w] {
            dib.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8, 255]);
        }
    }
    let png = png(px, w, h)?;
    unsafe fn put(format: u32, data: &[u8]) -> bool {
        let h = GlobalAlloc(GMEM_MOVEABLE, data.len());
        if h.is_null() {
            return false;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return false;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        GlobalUnlock(h);
        if SetClipboardData(format, h).is_null() {
            GlobalFree(h);
            return false;
        }
        true
    }
    unsafe {
        let mut tries = 0;
        while OpenClipboard(std::ptr::null_mut()) == 0 {
            tries += 1;
            if tries > 10 {
                return Err("the clipboard is busy".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        EmptyClipboard();
        let ok = put(CF_DIB, &dib);
        let name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
        let fmt = RegisterClipboardFormatW(name.as_ptr());
        if fmt != 0 {
            put(fmt, &png);
        }
        CloseClipboard();
        if ok {
            Ok(())
        } else {
            Err("couldn't put the picture on the clipboard".into())
        }
    }
}

#[cfg(not(windows))]
pub fn copy_image(_px: &[u32], _w: usize, _h: usize) -> Result<(), String> {
    Err("copying pictures is Windows-only for now".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(text: &str) -> Terminal {
        let mut t = Terminal::new(40, 6);
        let mut p = vibex_vt::Parser::new();
        p.advance_bytes(&mut t, text.as_bytes());
        t
    }

    #[test]
    fn text_and_range() {
        let t = term("one\r\n\x1b[31mtwo\x1b[0m red\r\nthree\r\n");
        let s = take(&t, (0, 1), (39, 2), None, false);
        assert_eq!(s.text(), "two red\nthree\n");
        let part = take(&t, (1, 0), (2, 1), None, false);
        assert_eq!(part.text(), " ne\ntwo\n");
    }

    #[test]
    fn masks_secrets_everywhere() {
        // put together at run time: no scanner-bait in the source
        let key = ["gh", "p_", "abcdefghijklmnopqrstuvwxyz", "0123456789"].concat();
        let t = term(&format!("export T={key}\r\n"));
        let g = Guard::default();
        let s = take(&t, (0, 0), (39, 1), Some(&g), false);
        assert!(!s.masked.is_empty());
        for out in [s.text(), s.ansi(), s.markdown(), s.html(&Theme::default(), "x"), s.svg(&Theme::default(), "x")] {
            assert!(!out.contains("abcdefghijklmnop"), "leaked: {out}");
        }
        assert!(s.text().contains("export T="));
    }

    #[test]
    fn html_and_svg_carry_colour() {
        let t = term("\x1b[38;2;255;0;0mred <b>\x1b[0m\r\n");
        let s = take(&t, (0, 0), (39, 0), None, true);
        let h = s.html(&Theme::default(), "Cascadia Mono");
        assert!(h.contains("color:#ff0000") && h.contains("red &lt;b&gt;"));
        let v = s.svg(&Theme::default(), "Cascadia Mono");
        assert!(v.contains("fill=\"#ff0000\"") && v.starts_with("<svg"));
        assert_eq!(s.cols, 20);
    }

    #[test]
    fn markdown_fence_outgrows_backticks() {
        let t = term("a ``` b\r\n");
        let s = take(&t, (0, 0), (39, 0), None, false);
        assert!(s.markdown().starts_with("````console\n"));
    }

    #[test]
    fn png_header() {
        let p = png(&[0xff0000, 0x00ff00, 0x0000ff, 0], 2, 2).unwrap();
        assert_eq!(&p[1..4], b"PNG");
    }
}
