//! Session recording, asciinema style: a pane's output saved as an
//! asciicast v2 file (`~/.config/vibex/recordings/*.cast`), played back
//! in a tab, or turned into a GIF (`vibex cast2gif`).
//!
//! The format: a JSON header line, then one `[seconds, "o", "text"]` line
//! per chunk of output and `[seconds, "r", "COLSxROWS"]` on resize.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// "2026-11-12_143000-build" from a recording's path.
pub fn short_name(p: &str) -> String {
    Path::new(p).file_stem().map_or(p.to_string(), |s| s.to_string_lossy().into_owned())
}

pub fn dir() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("recordings"))
}

// ------------------------------------------------------------ recording

pub struct Recorder {
    out: BufWriter<std::fs::File>,
    start: Instant,
    pub path: PathBuf,
    /// The start of a UTF-8 character split across reads.
    carry: Vec<u8>,
    /// Blank out secrets before they're written.
    pub guard: Option<crate::secrets::Guard>,
    /// Started by engagement mode (stops with it).
    pub engagement: bool,
    /// Written to disk at most a second behind (so a crash keeps it).
    last_flush: Instant,
}

impl Recorder {
    pub fn start(cols: usize, rows: usize, title: &str) -> Result<Recorder, String> {
        let dir = dir().ok_or("no config folder")?;
        Recorder::start_in(&dir, cols, rows, title)
    }

    pub fn start_in(dir: &Path, cols: usize, rows: usize, title: &str) -> Result<Recorder, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let now = crate::history::now();
        let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(40).collect();
        let path = dir.join(format!("{}-{}.cast", stamp(now), safe.trim_matches('-')));
        let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut out = BufWriter::new(file);
        writeln!(
            out,
            "{{\"version\":2,\"width\":{cols},\"height\":{rows},\"timestamp\":{now},\"title\":{},\"env\":{{\"TERM\":\"xterm-256color\"}}}}",
            json_string(title)
        )
        .map_err(|e| e.to_string())?;
        Ok(Recorder { out, start: Instant::now(), path, carry: Vec::new(), guard: None, engagement: false, last_flush: Instant::now() })
    }

    fn t(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn output(&mut self, bytes: &[u8]) {
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(bytes);
        // keep an unfinished UTF-8 sequence for the next read
        let cut = match std::str::from_utf8(&data) {
            Ok(_) => data.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => data.len(),
        };
        self.carry = data.split_off(cut);
        if data.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&data);
        let text = match &self.guard {
            Some(g) => std::borrow::Cow::Owned(g.redact(&text)),
            None => text,
        };
        let _ = writeln!(self.out, "[{:.6}, \"o\", {}]", self.t(), json_string(&text));
        if self.last_flush.elapsed().as_secs_f32() >= 1.0 {
            let _ = self.out.flush();
            self.last_flush = Instant::now();
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let _ = writeln!(self.out, "[{:.6}, \"r\", \"{cols}x{rows}\"]", self.t());
    }

    pub fn finish(mut self) -> PathBuf {
        let _ = self.out.flush();
        self.path
    }
}

/// "2026-11-12_1430" (UTC) for file names.
fn stamp(secs: u64) -> String {
    let days = secs / 86_400;
    let (y, m, d) = civil(days as i64);
    let rem = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02}_{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 to (year, month, day).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (era * 400 + yoe + i64::from(m <= 2), m, d)
}

pub fn json_string(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// ------------------------------------------------------------ reading

#[derive(Clone, Debug, PartialEq)]
pub struct Cast {
    pub width: usize,
    pub height: usize,
    /// (seconds, output text); resizes are dropped.
    pub events: Vec<(f64, String)>,
}

/// Read a JSON string starting at `s[0] == '"'`; returns it and the rest.
fn read_string(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut it = s.strip_prefix('"')?.char_indices();
    while let Some((i, c)) = it.next() {
        match c {
            '"' => return Some((out, &s[1 + i + 1..])),
            '\\' => {
                let (_, e) = it.next()?;
                match e {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'u' => {
                        let hex: String = (0..4).filter_map(|_| it.next().map(|(_, c)| c)).collect();
                        let mut cp = u32::from_str_radix(&hex, 16).ok()?;
                        if (0xd800..0xdc00).contains(&cp) {
                            // a surrogate pair: 😀
                            let next: String = (0..6).filter_map(|_| it.next().map(|(_, c)| c)).collect();
                            let lo = next.strip_prefix("\\u").and_then(|h| u32::from_str_radix(h, 16).ok())?;
                            cp = 0x10000 + ((cp - 0xd800) << 10) + (lo.wrapping_sub(0xdc00) & 0x3ff);
                        }
                        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                    }
                    other => out.push(other),
                }
            }
            c => out.push(c),
        }
    }
    None
}

fn header_number(h: &str, key: &str) -> Option<usize> {
    let at = h.find(&format!("\"{key}\""))?;
    let rest = h[at + key.len() + 2..].trim_start().strip_prefix(':')?.trim_start();
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

pub fn parse(text: &str) -> Result<Cast, String> {
    let mut lines = text.lines();
    let header = lines.next().ok_or("empty file")?;
    if !header.trim_start().starts_with('{') {
        return Err("not an asciicast file".into());
    }
    let width = header_number(header, "width").ok_or("header has no width")?;
    let height = header_number(header, "height").ok_or("header has no height")?;
    let mut events = Vec::new();
    for line in lines {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else { continue };
        let (t, rest) = rest.split_once(',').ok_or("bad event")?;
        let t: f64 = t.trim().parse().map_err(|_| "bad time")?;
        let rest = rest.trim_start();
        let Some((kind, rest)) = read_string(rest) else { continue };
        if kind != "o" {
            continue;
        }
        let rest = rest.trim_start().strip_prefix(',').ok_or("bad event")?.trim_start();
        let Some((data, _)) = read_string(rest) else { continue };
        events.push((t, data));
    }
    Ok(Cast { width, height, events })
}

pub fn load(path: &Path) -> Result<Cast, String> {
    parse(&std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
}

/// Gaps over `idle` seconds shrink to `idle` (like `asciinema -i`), then
/// everything is sped up by `speed`.
pub fn retime(events: &mut [(f64, String)], idle: f64, speed: f64) {
    let mut last = 0.0;
    let mut shift = 0.0;
    for (t, _) in events.iter_mut() {
        let gap = *t - last;
        last = *t;
        if gap > idle {
            shift += gap - idle;
        }
        *t = (*t - shift).max(0.0) / speed.max(0.01);
    }
}

/// The newest recordings first: (path, file name).
pub fn list() -> Vec<PathBuf> {
    let Some(d) = dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(d) else { return Vec::new() };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "cast"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, p)| p).take(30).collect()
}

// ------------------------------------------------------------ playback

/// Feeds a recording into a pane's terminal on its own clock.
pub struct Player {
    pub events: Vec<(f64, String)>,
    pub start: Instant,
    pub next: usize,
}

impl Player {
    pub fn new(mut cast: Cast) -> Player {
        retime(&mut cast.events, 2.0, 1.0);
        Player { events: cast.events, start: Instant::now(), next: 0 }
    }

    /// Text due by now, and when the next chunk is due (None: finished).
    pub fn due(&mut self) -> (String, Option<Instant>) {
        let t = self.start.elapsed().as_secs_f64();
        let mut out = String::new();
        while let Some((at, data)) = self.events.get(self.next) {
            if *at > t {
                return (out, Some(self.start + std::time::Duration::from_secs_f64(*at)));
            }
            out.push_str(data);
            self.next += 1;
        }
        (out, None)
    }
}

// ------------------------------------------------------------ GIF

/// A GIF89a encoder: one global 256-colour palette, LZW by hand, each
/// frame only the rectangle that changed.
pub struct Gif {
    out: Vec<u8>,
    w: usize,
    h: usize,
    palette: Vec<u32>,
    lookup: std::collections::HashMap<u32, u8>,
    prev: Option<Vec<u8>>,
}

impl Gif {
    pub fn new(w: usize, h: usize, palette: Vec<u32>) -> Gif {
        let mut out = Vec::new();
        out.extend_from_slice(b"GIF89a");
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(h as u16).to_le_bytes());
        out.extend_from_slice(&[0xf7, 0, 0]); // global table, 256 entries
        for i in 0..256 {
            let c = palette.get(i).copied().unwrap_or(0);
            out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
        }
        // loop forever
        out.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00");
        Gif { out, w, h, palette, lookup: Default::default(), prev: None }
    }

    fn index(&mut self, c: u32) -> u8 {
        if let Some(i) = self.lookup.get(&c) {
            return *i;
        }
        let (r, g, b) = ((c >> 16 & 0xff) as i32, (c >> 8 & 0xff) as i32, (c & 0xff) as i32);
        let best = self
            .palette
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| {
                let (pr, pg, pb) = ((*p >> 16 & 0xff) as i32, (*p >> 8 & 0xff) as i32, (*p & 0xff) as i32);
                (r - pr).pow(2) * 3 + (g - pg).pow(2) * 4 + (b - pb).pow(2) * 2
            })
            .map_or(0, |(i, _)| i as u8);
        self.lookup.insert(c, best);
        best
    }

    /// Add a frame shown for `delay_cs` hundredths of a second. Returns
    /// false (and adds nothing) if it's the same as the last one.
    pub fn frame(&mut self, pixels: &[u32], delay_cs: u16) -> bool {
        let idx: Vec<u8> = pixels.iter().map(|&c| self.index(c & 0xff_ffff)).collect();
        // the rectangle that changed
        let (mut x0, mut y0, mut x1, mut y1) = (0, 0, self.w, self.h);
        if let Some(prev) = &self.prev {
            let (mut minx, mut miny, mut maxx, mut maxy) = (usize::MAX, usize::MAX, 0, 0);
            for y in 0..self.h {
                let row = y * self.w;
                for x in 0..self.w {
                    if prev[row + x] != idx[row + x] {
                        minx = minx.min(x);
                        maxx = maxx.max(x);
                        miny = miny.min(y);
                        maxy = maxy.max(y);
                    }
                }
            }
            if minx == usize::MAX {
                return false;
            }
            (x0, y0, x1, y1) = (minx, miny, maxx + 1, maxy + 1);
        }
        let o = &mut self.out;
        // graphic control: leave the frame in place, the delay
        o.extend_from_slice(&[0x21, 0xf9, 0x04, 0x04]);
        o.extend_from_slice(&delay_cs.to_le_bytes());
        o.extend_from_slice(&[0, 0]);
        // image descriptor
        o.push(0x2c);
        for v in [x0, y0, x1 - x0, y1 - y0] {
            o.extend_from_slice(&(v as u16).to_le_bytes());
        }
        o.push(0);
        let mut sub = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 {
            sub.extend_from_slice(&idx[y * self.w + x0..y * self.w + x1]);
        }
        lzw(&sub, o);
        self.prev = Some(idx);
        true
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.out.push(0x3b);
        self.out
    }
}

/// GIF LZW with 8-bit pixels, written as data sub-blocks.
fn lzw(data: &[u8], out: &mut Vec<u8>) {
    const CLEAR: u16 = 256;
    const END: u16 = 257;
    out.push(8);
    let mut bits = Vec::<u8>::new();
    let (mut acc, mut nacc) = (0u32, 0u32);
    let mut emit = |code: u16, size: u32, bits: &mut Vec<u8>| {
        acc |= u32::from(code) << nacc;
        nacc += size;
        while nacc >= 8 {
            bits.push(acc as u8);
            acc >>= 8;
            nacc -= 8;
        }
    };
    let mut dict: std::collections::HashMap<(u16, u8), u16> = std::collections::HashMap::new();
    let mut size = 9u32;
    let mut next = 258u16;
    emit(CLEAR, size, &mut bits);
    let mut cur: Option<u16> = None;
    for &k in data {
        let Some(c) = cur else {
            cur = Some(u16::from(k));
            continue;
        };
        if let Some(&code) = dict.get(&(c, k)) {
            cur = Some(code);
            continue;
        }
        emit(c, size, &mut bits);
        if next < 4096 {
            dict.insert((c, k), next);
            next += 1;
            if u32::from(next) > (1 << size) && size < 12 {
                size += 1;
            }
        } else {
            emit(CLEAR, size, &mut bits);
            dict.clear();
            size = 9;
            next = 258;
        }
        cur = Some(u16::from(k));
    }
    if let Some(c) = cur {
        emit(c, size, &mut bits);
    }
    emit(END, size, &mut bits);
    if nacc > 0 {
        bits.push(acc as u8);
    }
    for chunk in bits.chunks(255) {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
    }
    out.push(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cast_round_trip() {
        let text = "{\"version\":2,\"width\":80,\"height\":24,\"timestamp\":1}\n\
                    [0.5, \"o\", \"hi \\u001b[31mred\\u001b[0m \\\"q\\\" \\ud83d\\ude00\\r\\n\"]\n\
                    [1.0, \"r\", \"100x30\"]\n\
                    [2.25, \"o\", \"bye\"]\n";
        let c = parse(text).unwrap();
        assert_eq!((c.width, c.height), (80, 24));
        assert_eq!(c.events.len(), 2);
        assert_eq!(c.events[0].1, "hi \u{1b}[31mred\u{1b}[0m \"q\" 😀\r\n");
        assert_eq!(c.events[1], (2.25, "bye".to_string()));
        // and what we write reads back
        let line = format!("[0.1, \"o\", {}]", json_string("a\u{1b}[0m\"\\\n😀"));
        let back = parse(&format!("{{\"width\":1,\"height\":1}}\n{line}\n")).unwrap();
        assert_eq!(back.events[0].1, "a\u{1b}[0m\"\\\n😀");
    }

    #[test]
    fn idle_gaps_shrink() {
        let mut ev = vec![(1.0, "a".to_string()), (10.0, "b".into()), (10.5, "c".into())];
        retime(&mut ev, 2.0, 1.0);
        let t: Vec<f64> = ev.iter().map(|e| e.0).collect();
        assert_eq!(t, vec![1.0, 3.0, 3.5]);
        retime(&mut ev, 100.0, 2.0);
        assert_eq!(ev[2].0, 1.75);
    }

    #[test]
    fn dates() {
        assert_eq!(stamp(0), "1970-01-01_000000");
        assert_eq!(stamp(1_794_460_894), "2026-11-12_052134");
    }

    /// Decode our own LZW output (a tiny reference decoder).
    fn unlzw(mut d: &[u8]) -> Vec<u8> {
        assert_eq!(d[0], 8);
        d = &d[1..];
        let mut bytes = Vec::new();
        while d[0] != 0 {
            let n = d[0] as usize;
            bytes.extend_from_slice(&d[1..=n]);
            d = &d[n + 1..];
        }
        let (mut pos, mut size) = (0usize, 9usize);
        let read = |pos: &mut usize, size: usize| {
            let mut v = 0u32;
            for i in 0..size {
                let b = *pos + i;
                v |= u32::from(bytes[b / 8] >> (b % 8) & 1) << i;
            }
            *pos += size;
            v as u16
        };
        let mut dict: Vec<Vec<u8>> = Vec::new();
        let reset = |dict: &mut Vec<Vec<u8>>| {
            dict.clear();
            for i in 0..256 {
                dict.push(vec![i as u8]);
            }
            dict.push(vec![]);
            dict.push(vec![]);
        };
        reset(&mut dict);
        let mut out = Vec::new();
        let mut prev: Option<Vec<u8>> = None;
        loop {
            let code = read(&mut pos, size);
            if code == 256 {
                reset(&mut dict);
                size = 9;
                prev = None;
                continue;
            }
            if code == 257 {
                break;
            }
            let entry = if (code as usize) < dict.len() {
                dict[code as usize].clone()
            } else {
                let p = prev.clone().unwrap();
                let mut e = p.clone();
                e.push(p[0]);
                e
            };
            out.extend_from_slice(&entry);
            if let Some(p) = prev {
                let mut n = p;
                n.push(entry[0]);
                dict.push(n);
                if dict.len() + 1 > (1 << size) && size < 12 {
                    size += 1;
                }
            }
            prev = Some(entry);
        }
        out
    }

    #[test]
    fn lzw_round_trips() {
        for data in [
            vec![0u8; 10],
            (0..5000u32).map(|i| (i * 7 % 13) as u8).collect::<Vec<_>>(),
            (0..70_000u32).map(|i| (i.wrapping_mul(2654435761u32) >> 24) as u8).collect::<Vec<_>>(),
        ] {
            let mut enc = Vec::new();
            lzw(&data, &mut enc);
            assert_eq!(enc.last(), Some(&0));
            assert_eq!(unlzw(&enc), data);
        }
    }

    #[test]
    fn gif_frames_only_when_changed() {
        let pal = vec![0x000000, 0xffffff];
        let mut g = Gif::new(4, 2, pal);
        let a = vec![0u32; 8];
        let mut b = a.clone();
        b[5] = 0xffffff;
        assert!(g.frame(&a, 10));
        assert!(!g.frame(&a, 10));
        assert!(g.frame(&b, 10));
        let bytes = g.finish();
        assert!(bytes.starts_with(b"GIF89a"));
        assert_eq!(bytes.last(), Some(&0x3b));
    }
}
