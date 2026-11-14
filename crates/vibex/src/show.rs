//! `vibex show FILE...`: images inline (kitty graphics, PNG) and Markdown
//! rendered with colour, in the terminal it's run from. Inside Vibex the
//! shell integration adds `vshow`, which also passes the width.

use std::io::Write;
use std::path::Path;

const USAGE: &str = "usage: vibex show [SIZE] FILE...

  .png                 drawn inline (kitty graphics protocol: Vibex, kitty, WezTerm, Ghostty)
  .md / .markdown      rendered: headings, lists, quotes, code, tables, links
  anything else        printed as text

Image size (the aspect is always kept):
  --width N            N columns wide
  --height N           N rows tall
  --fit                as big as fits the window (width and height)
  --scale 50%          a share of its own size (also 0.5, 200%)
  --px N               N pixels wide
  --actual             its own size, one image pixel per screen pixel
  --center             centred (default: at the left)
  (default: its own size, shrunk to the window's width when wider)";

/// How big to draw an image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub fit: bool,
    pub scale: Option<f32>,
    pub px: Option<u32>,
    pub actual: bool,
    pub center: bool,
}

/// "50%" / "0.5" / "2x" -> 0.5 / 0.5 / 2.0
fn parse_scale(v: &str) -> Option<f32> {
    let v = v.trim();
    let f = if let Some(p) = v.strip_suffix('%') {
        p.trim().parse::<f32>().ok()? / 100.0
    } else if let Some(x) = v.strip_suffix(['x', 'X']) {
        x.trim().parse::<f32>().ok()?
    } else {
        v.parse::<f32>().ok()?
    };
    (f > 0.0 && f <= 16.0).then_some(f)
}

pub fn run(args: Vec<String>) -> i32 {
    // --cols / --rows: the window's size (vshow passes them)
    let (mut cols, mut rows): (Option<usize>, Option<usize>) = (None, None);
    let mut size = Size::default();
    let mut files = Vec::new();
    let mut it = args.into_iter();
    let mut bad: Option<String> = None;
    while let Some(a) = it.next() {
        let mut num = |name: &str| -> Option<usize> {
            let v = it.next();
            let n = v.as_deref().and_then(|v| v.trim().parse::<usize>().ok()).filter(|n| *n > 0);
            if n.is_none() {
                bad = Some(format!("{name} wants a number, got '{}'", v.unwrap_or_default()));
            }
            n
        };
        match a.as_str() {
            "--cols" => cols = num("--cols"),
            "--rows" => rows = num("--rows"),
            "--width" | "-w" => size.width = num("--width"),
            "--height" => size.height = num("--height"),
            "--px" => size.px = num("--px").map(|n| n.min(16_384) as u32),
            "--scale" => {
                let v = it.next().unwrap_or_default();
                size.scale = parse_scale(&v);
                if size.scale.is_none() {
                    bad = Some(format!("--scale wants 50%, 0.5 or 2x, got '{v}'"));
                }
            }
            "--fit" => size.fit = true,
            "--actual" => size.actual = true,
            "--center" | "--centre" => size.center = true,
            "--left" => size.center = false,
            "-h" | "--help" => {
                outln!("{USAGE}");
                return 0;
            }
            _ => files.push(a),
        }
    }
    if let Some(b) = bad {
        eprintln!("vibex show: {b}");
        return 2;
    }
    if files.is_empty() {
        eprintln!("{USAGE}");
        return 2;
    }
    let (cw, ch) = console_size();
    let width = cols.filter(|w| *w >= 20).unwrap_or(cw).min(1000);
    let height = rows.filter(|h| *h >= 3).unwrap_or(ch).min(1000);
    let mut out = std::io::stdout().lock();
    let mut status = 0;
    for f in &files {
        let p = Path::new(f);
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let res = match ext.as_str() {
            "png" => std::fs::read(p).map_err(|e| e.to_string()).and_then(|bytes| image(&mut out, &bytes, width, height, size)),
            "md" | "markdown" => std::fs::read_to_string(p).map_err(|e| e.to_string()).map(|t| {
                let _ = out.write_all(markdown(&t, width).as_bytes());
            }),
            "jpg" | "jpeg" | "jfif" => {
                std::fs::read(p).map_err(|e| e.to_string()).and_then(|b| jpeg_to_png(&b)).and_then(|bytes| image(&mut out, &bytes, width, height, size))
            }
            "gif" | "bmp" | "webp" => Err("PNG and JPEG images only (convert it, or open it with Ctrl+click)".into()),
            _ => std::fs::read_to_string(p).map_err(|e| e.to_string()).map(|t| {
                let _ = out.write_all(t.replace('\n', "\r\n").as_bytes());
            }),
        };
        if let Err(e) = res {
            let _ = out.flush();
            eprintln!("vibex show: {f}: {e}");
            status = 1;
        }
    }
    let _ = out.flush();
    status
}

/// The console window's (columns, rows).
fn console_size() -> (usize, usize) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{GetConsoleScreenBufferInfo, GetStdHandle, CONSOLE_SCREEN_BUFFER_INFO, STD_OUTPUT_HANDLE};
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) != 0 {
            let w = (info.srWindow.Right - info.srWindow.Left + 1) as usize;
            let h = (info.srWindow.Bottom - info.srWindow.Top + 1) as usize;
            if w >= 20 {
                return (w, h.max(3));
            }
        }
    }
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|c| c.parse().ok()).unwrap_or(d);
    (env("COLUMNS", 100), env("LINES", 30))
}

// ------------------------------------------------------------ images

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

/// PNG width and height from its header.
fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 24 || &b[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some((u32::from_be_bytes(b[16..20].try_into().ok()?), u32::from_be_bytes(b[20..24].try_into().ok()?)))
}

/// ~8x16px cells: a fair guess when the terminal's cell size isn't known.
const GUESS_CELL: (f32, f32) = (8.0, 16.0);

/// The terminal's cell size in pixels: Vibex tells its panes
/// (`VIBEX_CELL_PX=9x19`), else a guess.
fn cell_px() -> (f32, f32) {
    std::env::var("VIBEX_CELL_PX").ok().and_then(|v| parse_cell(&v)).unwrap_or(GUESS_CELL)
}

fn parse_cell(v: &str) -> Option<(f32, f32)> {
    let (w, h) = v.trim().split_once('x')?;
    let (w, h) = (w.parse::<f32>().ok()?, h.parse::<f32>().ok()?);
    (w >= 1.0 && h >= 1.0).then_some((w, h))
}

/// Kitty graphics for the default size: at most the terminal's width
/// (the terminal keeps the aspect).
#[cfg(test)]
pub fn image_escape(png: &[u8], max_cols: usize) -> Result<String, String> {
    image_escape_sized(png, max_cols, 1000, Size::default())
}

pub fn image_escape_sized(png: &[u8], max_cols: usize, max_rows: usize, size: Size) -> Result<String, String> {
    image_escape_cell(png, max_cols, max_rows, size, cell_px())
}

/// Kitty graphics: transmit + display, in 4096-byte chunks. Pixel sizes
/// (`--scale`, `--px`) resize the PNG itself, so it shows at exactly that
/// size; column/row sizes go to the terminal as c= / r= (only one of them,
/// so the terminal keeps the aspect).
///
/// The rows are made first, with plain newlines, and the image goes in
/// them with C=1 (the cursor stays): on Windows the shell's output passes
/// through ConPTY, which never sees an image move the cursor, so without
/// this its cursor and the terminal's part ways and the next prompt lands
/// inside the picture.
fn image_escape_cell(png: &[u8], max_cols: usize, max_rows: usize, size: Size, cell: (f32, f32)) -> Result<String, String> {
    let (w, h) = png_size(png).ok_or("not a PNG file")?;
    let (w, h) = (w.max(1), h.max(1));
    let (cw, ch) = cell;
    let max_c = max_cols.saturating_sub(1).max(1);
    // taller than this and it's shrunk to fit: its rows have to be on screen
    let max_r = max_rows.saturating_sub(2).max(1);
    let mut data: Option<Vec<u8>> = None;
    let mut cols: Option<usize> = None;
    let mut rows: Option<usize> = None;
    // the image's size in cells: width for centring, height for the rows
    let (mut est, tall): (f32, f32);
    let target = size.px.or_else(|| size.scale.map(|s| ((w as f32 * s).round() as u32).max(1)));
    if let Some(tw) = target {
        let tw = tw.clamp(1, 16_384);
        let th = ((h as f64 * tw as f64 / w as f64).round() as u32).clamp(1, 16_384);
        if tw != w {
            data = Some(resize_png(png, tw, th)?);
        }
        (est, tall) = (tw as f32 / cw, th as f32 / ch);
    } else if size.actual {
        (est, tall) = (w as f32 / cw, h as f32 / ch);
    } else if let Some(n) = size.width {
        cols = Some(n.min(1000));
        (est, tall) = (n as f32, n as f32 * cw * h as f32 / w as f32 / ch);
    } else if let Some(n) = size.height {
        rows = Some(n.min(1000));
        (est, tall) = (n as f32 * ch * w as f32 / h as f32 / cw, n as f32);
    } else if size.fit {
        let need_rows = max_c as f32 * cw * h as f32 / w as f32 / ch;
        if need_rows <= max_r as f32 {
            cols = Some(max_c);
            (est, tall) = (max_c as f32, need_rows);
        } else {
            rows = Some(max_r);
            (est, tall) = (max_r as f32 * ch * w as f32 / h as f32 / cw, max_r as f32);
        }
    } else {
        let c = ((w as f32 / cw).ceil() as usize).clamp(1, max_c);
        cols = Some(c);
        (est, tall) = (c as f32, c as f32 * cw * h as f32 / w as f32 / ch);
    }
    let mut n = (tall - 0.001).ceil().max(1.0) as usize;
    // No size asked for: snap to whole rows (sized by rows, the terminal
    // keeps the aspect, so the width moves by a fraction of a column) so
    // there's no nearly empty row between the picture and the prompt.
    let plain = target.is_none() && !size.actual && size.width.is_none() && size.height.is_none() && !size.fit;
    if plain && tall >= 1.0 && (tall - tall.round()).abs() > 0.05 {
        let r = tall.round().max(1.0);
        let wide = est * r / tall;
        if wide.ceil() as usize <= max_c && (r as usize) <= max_r {
            est = wide;
            n = r as usize;
            cols = None;
            rows = Some(n);
        }
    }
    if n > max_r {
        // too tall for the window: as many rows as fit, the aspect kept
        est = est * max_r as f32 / tall;
        n = max_r;
        cols = None;
        rows = Some(max_r);
    }
    // Much bigger than it will be drawn (a phone photo shown at window
    // width): send only the pixels that show. Same size on screen, a
    // fraction of the data through the console host. Best effort: if the
    // PNG can't be re-encoded it goes as it is.
    if data.is_none() && !size.actual {
        let shown_w = (est.ceil() * cw).ceil().max(1.0) as u32;
        if w > shown_w + shown_w / 4 {
            let th = ((h as f64 * shown_w as f64 / w as f64).round() as u32).max(1);
            data = resize_png(png, shown_w, th).ok();
        }
    }
    let data = base64(data.as_deref().unwrap_or(png));
    let chunks: Vec<&str> = data.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    let mut size_keys = String::new();
    if let Some(c) = cols {
        size_keys.push_str(&format!(",c={c}"));
    }
    if let Some(r) = rows {
        size_keys.push_str(&format!(",r={r}"));
    }
    // the rows first (they scroll the screen as needed), then back up
    let mut o = "\r\n".repeat(n);
    o.push_str(&format!("\x1b[{n}A\r"));
    if size.center {
        let pad = (max_cols as f32 - est.ceil()).max(0.0) as usize / 2;
        if pad > 0 {
            o.push_str(&format!("\x1b[{pad}C"));
        }
    }
    for (i, c) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            o.push_str(&format!("\x1b_Ga=T,f=100,t=d,C=1,q=2{size_keys},m={more};{c}\x1b\\"));
        } else {
            o.push_str(&format!("\x1b_Gm={more};{c}\x1b\\"));
        }
    }
    // and below it
    o.push_str(&format!("\x1b[{n}B\r"));
    Ok(o)
}

fn image(out: &mut impl Write, png: &[u8], width: usize, height: usize, size: Size) -> Result<(), String> {
    let esc = image_escape_sized(png, width, height, size)?;
    out.write_all(esc.as_bytes()).map_err(|e| e.to_string())
}

/// Decode a PNG to RGBA8: (pixels, width, height).
fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut dec = png::Decoder::new(bytes);
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width, info.height);
    let n = (w * h) as usize;
    let src = &buf[..info.buffer_size()];
    let line = info.line_size;
    let mut out = Vec::with_capacity(n * 4);
    for y in 0..h as usize {
        let row = &src[y * line..(y + 1) * line];
        for x in 0..w as usize {
            let px = match info.color_type {
                png::ColorType::Rgba => [row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]],
                png::ColorType::Rgb => [row[x * 3], row[x * 3 + 1], row[x * 3 + 2], 255],
                png::ColorType::GrayscaleAlpha => [row[x * 2], row[x * 2], row[x * 2], row[x * 2 + 1]],
                png::ColorType::Grayscale => [row[x], row[x], row[x], 255],
                png::ColorType::Indexed => return Err("unexpected palette image".into()),
            };
            out.extend_from_slice(&px);
        }
    }
    Ok((out, w, h))
}

/// A box filter: each output pixel averages the source pixels it covers
/// (nearest-pixel when growing), alpha-weighted.
fn resize_rgba(src: &[u8], sw: u32, sh: u32, tw: u32, th: u32) -> Vec<u8> {
    let (sw, sh, tw, th) = (sw as usize, sh as usize, tw as usize, th as usize);
    let mut out = vec![0u8; tw * th * 4];
    for ty in 0..th {
        let y0 = ty * sh / th;
        let y1 = ((ty + 1) * sh / th).max(y0 + 1).min(sh);
        for tx in 0..tw {
            let x0 = tx * sw / tw;
            let x1 = ((tx + 1) * sw / tw).max(x0 + 1).min(sw);
            let mut acc = [0u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * sw + x) * 4;
                    let a = u64::from(src[i + 3]);
                    acc[0] += u64::from(src[i]) * a;
                    acc[1] += u64::from(src[i + 1]) * a;
                    acc[2] += u64::from(src[i + 2]) * a;
                    acc[3] += a;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            let o = (ty * tw + tx) * 4;
            if acc[3] > 0 {
                for k in 0..3 {
                    out[o + k] = (acc[k] / acc[3]) as u8;
                }
            }
            out[o + 3] = (acc[3] / n) as u8;
        }
    }
    out
}

fn resize_png(bytes: &[u8], tw: u32, th: u32) -> Result<Vec<u8>, String> {
    let (px, w, h) = decode_png(bytes)?;
    encode_png(&resize_rgba(&px, w, h, tw, th), tw, th)
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        // fast: the picture goes straight to the terminal, not to disk
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// The biggest picture side decoded (photos beyond this are refused
/// rather than eating memory).
const MAX_DECODE_SIDE: u16 = 16_384;

/// A JPEG as a PNG, so it goes through the same path (kitty graphics only
/// carries PNG or raw pixels).
fn jpeg_to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut dec = jpeg_decoder::Decoder::new(bytes);
    dec.read_info().map_err(|e| format!("not a JPEG file ({e})"))?;
    let info = dec.info().ok_or("not a JPEG file")?;
    if info.width == 0 || info.height == 0 || info.width > MAX_DECODE_SIDE || info.height > MAX_DECODE_SIDE {
        return Err(format!("picture too big ({}x{})", info.width, info.height));
    }
    let px = dec.decode().map_err(|e| format!("can't read the JPEG ({e})"))?;
    let (w, h) = (info.width as u32, info.height as u32);
    let n = w as usize * h as usize;
    let mut rgba = Vec::with_capacity(n * 4);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => px.iter().for_each(|&l| rgba.extend_from_slice(&[l, l, l, 255])),
        jpeg_decoder::PixelFormat::L16 => px.chunks_exact(2).for_each(|l| rgba.extend_from_slice(&[l[0], l[0], l[0], 255])),
        jpeg_decoder::PixelFormat::RGB24 => px.chunks_exact(3).for_each(|p| rgba.extend_from_slice(&[p[0], p[1], p[2], 255])),
        jpeg_decoder::PixelFormat::CMYK32 => px.chunks_exact(4).for_each(|p| {
            // Adobe JPEGs store CMYK inverted
            let k = p[3] as u32;
            let ch = |c: u8| (c as u32 * k / 255) as u8;
            rgba.extend_from_slice(&[ch(p[0]), ch(p[1]), ch(p[2]), 255])
        }),
    }
    if rgba.len() != n * 4 {
        return Err("can't read the JPEG (short data)".into());
    }
    encode_png(&rgba, w, h)
}

// ------------------------------------------------------------ markdown

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const UNDER: &str = "\x1b[4m";

/// Inline styles: **bold**, *italic*, `code`, [text](url) as a real link.
fn inline(s: &str) -> String {
    let mut o = String::new();
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let (mut bold, mut ital) = (false, false);
    let style = |o: &mut String, bold: bool, ital: bool| {
        o.push_str(RESET);
        if bold {
            o.push_str(BOLD);
        }
        if ital {
            o.push_str(ITALIC);
        }
    };
    while i < c.len() {
        match c[i] {
            '\\' if i + 1 < c.len() => {
                o.push(c[i + 1]);
                i += 2;
            }
            '`' => {
                if let Some(end) = c[i + 1..].iter().position(|&x| x == '`') {
                    let code: String = c[i + 1..i + 1 + end].iter().collect();
                    o.push_str("\x1b[38;5;180m");
                    o.push_str(&code);
                    style(&mut o, bold, ital);
                    i += end + 2;
                } else {
                    o.push('`');
                    i += 1;
                }
            }
            '*' | '_' if i + 1 < c.len() && c[i + 1] == c[i] => {
                bold = !bold;
                style(&mut o, bold, ital);
                i += 2;
            }
            '*' | '_'
                if (c[i] == '*' || i == 0 || !c[i - 1].is_alphanumeric()) && i + 1 < c.len() && !c[i + 1].is_whitespace()
                    || ital && (c[i] == '*' || c[i] == '_') =>
            {
                ital = !ital;
                style(&mut o, bold, ital);
                i += 1;
            }
            '[' => {
                // [text](url)
                let rest: String = c[i..].iter().collect();
                if let Some(close) = rest.find("](") {
                    if let Some(end) = rest[close + 2..].find(')') {
                        let text = &rest[1..close];
                        let url = &rest[close + 2..close + 2 + end];
                        o.push_str(&format!("\x1b]8;;{url}\x1b\\\x1b[4;38;5;111m{}\x1b]8;;\x1b\\", inline(text)));
                        style(&mut o, bold, ital);
                        i += rest[..close + 2 + end + 1].chars().count();
                        continue;
                    }
                }
                o.push('[');
                i += 1;
            }
            ch => {
                o.push(ch);
                i += 1;
            }
        }
    }
    o.push_str(RESET);
    o
}

/// Visible width (escape sequences don't count).
fn visible(s: &str) -> usize {
    let mut n = 0;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            match it.peek() {
                Some('[') => {
                    for d in it.by_ref() {
                        if d.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC ... ESC \
                    while let Some(d) = it.next() {
                        if d == '\x1b' {
                            it.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else {
            n += 1;
        }
    }
    n
}

/// Word-wrap plain text to `width` after an indent.
fn wrap(text: &str, width: usize, first: &str, rest: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = first.to_string();
    let mut cur_w = visible(first);
    let mut empty = true;
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let w = visible(word);
        if !empty && cur_w + 1 + w > width {
            lines.push(std::mem::replace(&mut cur, rest.to_string()));
            cur_w = visible(rest);
            empty = true;
        }
        if !empty {
            cur.push(' ');
            cur_w += 1;
        }
        cur.push_str(word);
        cur_w += w;
        empty = false;
    }
    lines.push(cur);
    lines
}

pub fn markdown(text: &str, width: usize) -> String {
    let width = width.max(20);
    let mut out: Vec<String> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut code: Option<(String, Vec<String>)> = None;
    let mut table: Vec<Vec<String>> = Vec::new();
    let flush_para = |para: &mut Vec<String>, out: &mut Vec<String>| {
        if !para.is_empty() {
            let t = inline(&para.join(" "));
            out.extend(wrap(&t, width, "", ""));
            out.push(String::new());
            para.clear();
        }
    };
    let flush_table = |table: &mut Vec<Vec<String>>, out: &mut Vec<String>| {
        if table.is_empty() {
            return;
        }
        let n = table.iter().map(Vec::len).max().unwrap_or(0);
        let mut w = vec![0usize; n];
        for row in table.iter() {
            for (i, cell) in row.iter().enumerate() {
                w[i] = w[i].max(visible(&inline(cell)));
            }
        }
        for (r, row) in table.iter().enumerate() {
            let mut line = String::from("  ");
            for (i, width) in w.iter().enumerate() {
                let cell = inline(row.get(i).map_or("", |c| c.as_str()));
                let pad = width - visible(&cell);
                if r == 0 {
                    line.push_str(BOLD);
                }
                line.push_str(&cell);
                line.push_str(RESET);
                line.push_str(&" ".repeat(pad));
                if i + 1 < n {
                    line.push_str(&format!("{DIM} │ {RESET}"));
                }
            }
            out.push(line);
            if r == 0 {
                let rule: Vec<String> = w.iter().map(|w| "─".repeat(*w)).collect();
                out.push(format!("  {DIM}{}{RESET}", rule.join("─┼─")));
            }
        }
        out.push(String::new());
        table.clear();
    };
    for raw in text.lines() {
        let line = raw.trim_end();
        // fenced code
        if let Some((lang, body)) = code.as_mut() {
            if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                let inner = width.saturating_sub(4);
                let label = if lang.is_empty() { String::new() } else { format!(" {lang} ") };
                out.push(format!("  {DIM}┌{label}{}{RESET}", "─".repeat(inner.saturating_sub(label.chars().count()))));
                for l in body.iter() {
                    let shown: String = l.chars().take(inner.saturating_sub(1)).collect();
                    out.push(format!("  {DIM}│{RESET} \x1b[38;5;180m{shown}{RESET}"));
                }
                out.push(format!("  {DIM}└{}{RESET}", "─".repeat(inner)));
                out.push(String::new());
                code = None;
            } else {
                body.push(line.replace('\t', "    "));
            }
            continue;
        }
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            flush_para(&mut para, &mut out);
            flush_table(&mut table, &mut out);
            code = Some((t.trim_start_matches(['`', '~']).trim().to_string(), Vec::new()));
            continue;
        }
        // tables
        if t.starts_with('|') && t.ends_with('|') && t.len() > 1 {
            flush_para(&mut para, &mut out);
            let cells: Vec<String> = t[1..t.len() - 1].split('|').map(|c| c.trim().to_string()).collect();
            if cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':')) {
                continue; // the |---|---| line
            }
            table.push(cells);
            continue;
        }
        flush_table(&mut table, &mut out);
        if t.is_empty() {
            flush_para(&mut para, &mut out);
            continue;
        }
        // headings
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush_para(&mut para, &mut out);
            let title = inline(t[hashes..].trim());
            match hashes {
                1 => {
                    out.push(format!("{BOLD}\x1b[38;5;209m{title}{RESET}"));
                    out.push(format!("{DIM}{}{RESET}", "═".repeat(visible(&title).min(width))));
                }
                2 => {
                    out.push(format!("{BOLD}\x1b[38;5;215m{title}{RESET}"));
                    out.push(format!("{DIM}{}{RESET}", "─".repeat(visible(&title).min(width))));
                }
                _ => out.push(format!("{BOLD}{UNDER}{title}{RESET}")),
            }
            out.push(String::new());
            continue;
        }
        // rules
        if t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_')) {
            flush_para(&mut para, &mut out);
            out.push(format!("{DIM}{}{RESET}", "─".repeat(width.min(80))));
            out.push(String::new());
            continue;
        }
        // quotes
        if let Some(q) = t.strip_prefix('>') {
            flush_para(&mut para, &mut out);
            let body = inline(q.trim());
            for l in wrap(&body, width, &format!("{DIM}▌{RESET} {ITALIC}"), &format!("{DIM}▌{RESET} {ITALIC}")) {
                out.push(format!("{l}{RESET}"));
            }
            continue;
        }
        // lists (with nesting by indent)
        let indent = (line.len() - t.len()) / 2;
        let pad = "  ".repeat(indent);
        let bullet = |b: &str| format!("{pad}{b}");
        if let Some(item) = t.strip_prefix("- [ ] ").or_else(|| t.strip_prefix("* [ ] ")) {
            flush_para(&mut para, &mut out);
            out.extend(wrap(&inline(item), width, &bullet("☐ "), &bullet("  ")));
            continue;
        }
        if let Some(item) = t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("* [x] ")).or_else(|| t.strip_prefix("- [X] ")) {
            flush_para(&mut para, &mut out);
            out.extend(wrap(&format!("{DIM}{}", inline(item)), width, &bullet("\x1b[32m☑\x1b[0m "), &bullet("  ")));
            continue;
        }
        if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
            flush_para(&mut para, &mut out);
            let b = if indent % 2 == 0 { "• " } else { "◦ " };
            out.extend(wrap(&inline(item), width, &bullet(&format!("\x1b[38;5;209m{b}\x1b[0m")), &bullet("  ")));
            continue;
        }
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && digits < 4 && t[digits..].starts_with(". ") {
            flush_para(&mut para, &mut out);
            let n = &t[..digits];
            out.extend(wrap(&inline(&t[digits + 2..]), width, &bullet(&format!("\x1b[38;5;209m{n}.\x1b[0m ")), &bullet(&" ".repeat(digits + 2))));
            continue;
        }
        // images: ![alt](file.png) → a note (the file can be shown too)
        if let Some(rest) = t.strip_prefix("![") {
            if let Some(close) = rest.find("](") {
                flush_para(&mut para, &mut out);
                let alt = &rest[..close];
                let src = rest[close + 2..].trim_end_matches(')');
                out.push(format!("{DIM}[image: {} — {src}]{RESET}", if alt.is_empty() { "untitled" } else { alt }));
                continue;
            }
        }
        para.push(t.to_string());
    }
    flush_para(&mut para, &mut out);
    flush_table(&mut table, &mut out);
    if let Some((_, body)) = code {
        // an unclosed fence: still show it
        for l in body {
            out.push(format!("  {DIM}│{RESET} {l}"));
        }
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let mut s = out.join("\r\n");
    s.push_str("\r\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> String {
        // strip escapes for checking the text
        let mut o = String::new();
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\x1b' {
                match it.peek() {
                    Some('[') => {
                        for d in it.by_ref() {
                            if d.is_ascii_alphabetic() {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        while let Some(d) = it.next() {
                            if d == '\x1b' {
                                it.next();
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                o.push(c);
            }
        }
        o
    }

    #[test]
    fn renders_markdown() {
        let md = "# Title\n\nSome **bold** and *italic* and `code` and a [link](https://x.y).\n\n- one\n- two\n  - nested\n1. first\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 22 |\n";
        let out = markdown(md, 60);
        let p = plain(&out);
        assert!(p.contains("Title\r\n═════"), "{p}");
        assert!(p.contains("Some bold and italic and code and a link."), "{p}");
        assert!(out.contains("\x1b]8;;https://x.y\x1b\\"), "real link");
        assert!(p.contains("• one") && p.contains("◦ nested") && p.contains("1. first"), "{p}");
        assert!(p.contains("▌ quoted"));
        assert!(p.contains("┌ rust ") && p.contains("│ fn main() {}"), "{p}");
        assert!(p.contains("a │ b ") && p.contains("1 │ 22"), "{p}");
    }

    #[test]
    fn wraps_to_width() {
        let out = plain(&markdown(&"word ".repeat(40), 30));
        assert!(out.lines().all(|l| l.trim_end().chars().count() <= 30), "{out}");
        assert!(out.lines().count() >= 6);
    }

    #[test]
    fn sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        let cell = (8.0, 16.0);
        let head = |s: Size| image_escape_cell(&png, 100, 40, s, cell).unwrap().split(";").next().unwrap().to_string();
        assert!(head(Size { width: Some(30), ..Default::default() }).ends_with("q=2,c=30,m=0"));
        assert!(head(Size { height: Some(10), ..Default::default() }).ends_with("q=2,r=10,m=0"));
        // 640x480 at 99 columns would want ~37 rows: fits in 38
        assert!(head(Size { fit: true, ..Default::default() }).ends_with("c=99,m=0"));
        let tall = |s: Size| image_escape_cell(&png, 100, 20, s, cell).unwrap();
        assert!(tall(Size { fit: true, ..Default::default() }).contains(",r=18,"));
        assert!(head(Size { actual: true, ..Default::default() }).ends_with("q=2,m=0"));
        let centred = image_escape_cell(&png, 100, 40, Size { width: Some(40), center: true, ..Default::default() }, cell).unwrap();
        assert!(centred.contains("\x1b[30C\x1b_G"), "{centred}");
        assert_eq!(parse_scale("50%"), Some(0.5));
        assert_eq!(parse_scale("2x"), Some(2.0));
        assert_eq!(parse_scale("0"), None);
    }

    #[test]
    fn makes_the_rows_first() {
        // 640x480 at 30 columns of 8x16 cells: 30*8*480/640/16 = 11.25 rows -> 12
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        let esc = image_escape_cell(&png, 100, 40, Size { width: Some(30), ..Default::default() }, (8.0, 16.0)).unwrap();
        assert!(esc.starts_with(&format!("{}\x1b[12A\r\x1b_G", "\r\n".repeat(12))), "{esc}");
        assert!(esc.ends_with("\x1b[12B\r"));
        // taller than the window: shrunk to its rows (40 - 2)
        let esc = image_escape_cell(&png, 100, 40, Size { width: Some(120), ..Default::default() }, (8.0, 16.0)).unwrap();
        assert!(esc.contains(",r=38,") && !esc.contains(",c=") && esc.ends_with("\x1b[38B\r"), "{esc}");
        // no size asked: a 256x256 picture on 11x26 cells is 10.15 rows at
        // 24 columns; it's snapped to 10 whole rows (no near-empty 11th)
        let mut sq = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        sq.extend_from_slice(&256u32.to_be_bytes());
        sq.extend_from_slice(&256u32.to_be_bytes());
        let esc = image_escape_cell(&sq, 120, 40, Size::default(), (11.0, 26.0)).unwrap();
        assert!(esc.contains(",r=10,") && !esc.contains(",c=") && esc.ends_with("\x1b[10B\r"), "{esc}");
        // an asked-for width is kept as it is
        let esc = image_escape_cell(&sq, 120, 40, Size { width: Some(24), ..Default::default() }, (11.0, 26.0)).unwrap();
        assert!(esc.contains(",c=24,") && esc.ends_with("\x1b[11B\r"), "{esc}");
        assert_eq!(parse_cell("9x19"), Some((9.0, 19.0)));
        assert_eq!(parse_cell("x"), None);
    }

    #[test]
    fn jpeg_pictures_show_too() {
        let png = jpeg_to_png(include_bytes!("testdata/tiny.jpg")).unwrap();
        assert_eq!(png_size(&png), Some((16, 8)));
        let (px, _, _) = decode_png(&png).unwrap();
        // left column dark red-ish, right column bright: a gradient survived
        assert!(px[0] < 40 && px[15 * 4] > 200, "{} {}", px[0], px[15 * 4]);
        assert!(jpeg_to_png(b"not a jpeg").is_err());
    }

    #[test]
    fn big_pictures_are_sent_at_their_shown_size() {
        let px = vec![200u8; 400 * 100 * 4];
        let big = encode_png(&px, 400, 100).unwrap();
        // 10 columns of 8 px: 80 px wide is all that shows
        let esc = image_escape_cell(&big, 100, 40, Size { width: Some(10), ..Default::default() }, (8.0, 16.0)).unwrap();
        let b64: String = esc.split(';').nth(1).unwrap().chars().take_while(|c| *c != '\x1b').collect();
        let sent = base64_decode_for_test(&b64);
        assert_eq!(png_size(&sent), Some((80, 20)));
        // --actual keeps every pixel
        let esc = image_escape_cell(&big, 100, 40, Size { actual: true, ..Default::default() }, (8.0, 16.0)).unwrap();
        assert!(esc.len() > base64(&big).len());
    }

    fn base64_decode_for_test(s: &str) -> Vec<u8> {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0);
        for c in s.bytes().filter(|c| *c != b'=') {
            acc = acc << 6 | T.iter().position(|t| *t == c).unwrap() as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    }

    #[test]
    fn scales_pixels() {
        // a real 4x2 PNG, halved to 2x1
        let mut px = Vec::new();
        for i in 0..8u8 {
            px.extend_from_slice(&[i * 30, 0, 0, 255]);
        }
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 4, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&px).unwrap();
        }
        let small = resize_png(&png, 2, 1).unwrap();
        assert_eq!(png_size(&small), Some((2, 1)));
        let (out, _, _) = decode_png(&small).unwrap();
        // the left pixel averages columns 0-1 of both rows: (0+30+120+150)/4
        assert_eq!(out[0], 75);
        let esc = image_escape_sized(&png, 100, 40, Size { scale: Some(0.5), ..Default::default() }).unwrap();
        assert!(esc.contains("\x1b_Ga=T,f=100,t=d,C=1,q=2,m=0;"));
    }

    #[test]
    fn base64_and_png() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((640, 480)));
        let esc = image_escape(&png, 100).unwrap();
        assert!(esc.contains("\x1b_Ga=T,f=100,t=d,C=1,q=2,c=80,m=0;"), "{esc}");
        assert!(image_escape(b"nope", 80).is_err());
    }
}
