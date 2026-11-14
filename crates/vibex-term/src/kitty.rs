//! Kitty graphics protocol (`APC G <keys> ; <base64 payload> ST`).
//!
//! Supported: transmit (`a=t`), transmit + display (`a=T`), put (`a=p`),
//! delete (`a=d`), query (`a=q`); formats RGB (24), RGBA (32) and PNG
//! (100); zlib compression (`o=z`); chunked transfers (`m=1`); source
//! rectangles, cell sizing, z-index, `C=1`. Only direct transmission
//! (`t=d`) is accepted: reading files or shared memory named by a program
//! would let anything running in the terminal make Vibex read local files.
//! Unicode placeholders (`U=1`) are not supported.

use crate::image::{CursorAfter, ImageData, Placement, Source, MAX_PIXELS, MAX_SIDE};
use crate::links::base64_decode;
use crate::Terminal;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// Image data kept for later `a=p` placements.
const STORE_LIMIT: usize = 320 * 1024 * 1024;
/// A chunked transfer may not grow past this much base64.
const MAX_CHUNKED: usize = 200 * 1024 * 1024;

#[derive(Clone, Default, Debug)]
pub(crate) struct Cmd {
    action: u8,
    format: u32,
    medium: u8,
    more: bool,
    id: u32,
    number: u32,
    placement: u32,
    quiet: u8,
    width: u32,
    height: u32,
    zlib: bool,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    cols: u32,
    rows: u32,
    no_move: bool,
    z: i32,
    delete: u8,
    unicode: bool,
}

fn parse_keys(keys: &[u8]) -> Cmd {
    let mut c = Cmd { action: b't', format: 32, medium: b'd', ..Default::default() };
    for kv in keys.split(|&b| b == b',') {
        let mut it = kv.splitn(2, |&b| b == b'=');
        let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
        if k.len() != 1 {
            continue;
        }
        let num = || std::str::from_utf8(v).ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        let unum = || num().clamp(0, u32::MAX as i64) as u32;
        let ch = v.first().copied().unwrap_or(0);
        match k[0] {
            b'a' => c.action = ch,
            b'f' => c.format = unum(),
            b't' => c.medium = ch,
            b'm' => c.more = num() == 1,
            b'i' => c.id = unum(),
            b'I' => c.number = unum(),
            b'p' => c.placement = unum(),
            b'q' => c.quiet = unum().min(2) as u8,
            b's' => c.width = unum(),
            b'v' => c.height = unum(),
            b'o' => c.zlib = ch == b'z',
            b'x' => c.x = unum(),
            b'y' => c.y = unum(),
            b'w' => c.w = unum(),
            b'h' => c.h = unum(),
            b'c' => c.cols = unum().min(1000),
            b'r' => c.rows = unum().min(1000),
            b'C' => c.no_move = num() == 1,
            b'z' => c.z = num().clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            b'd' => c.delete = ch,
            b'U' => c.unicode = num() == 1,
            _ => {}
        }
    }
    c
}

/// Most images kept for later placement (kitty's own limit is similar).
const MAX_STORED: usize = 4096;

/// What an image costs against the store's budget: its pixels plus a fixed
/// overhead for the bookkeeping.
fn cost(img: &ImageData) -> usize {
    img.bytes() + 512
}

/// State kept between APC strings.
#[derive(Default)]
pub(crate) struct KittyState {
    images: HashMap<u32, Arc<ImageData>>,
    /// Image number (`I=`) -> id.
    numbers: HashMap<u32, u32>,
    order: VecDeque<u32>,
    bytes: usize,
    /// A chunked transmission in progress: its first command + payload.
    pending: Option<(Cmd, Vec<u8>)>,
    next_auto_id: u32,
}

impl KittyState {
    fn store(&mut self, id: u32, img: Arc<ImageData>) {
        if let Some(old) = self.images.insert(id, img.clone()) {
            self.bytes -= cost(&old);
            self.order.retain(|&i| i != id);
        }
        self.bytes += cost(&img);
        self.order.push_back(id);
        // a byte budget and a count limit (thousands of 1x1 images cost
        // more in bookkeeping than in pixels)
        while self.bytes > STORE_LIMIT || self.images.len() > MAX_STORED {
            let Some(oldest) = self.order.pop_front() else { break };
            if let Some(old) = self.images.remove(&oldest) {
                self.bytes -= cost(&old);
                self.numbers.retain(|_, v| *v != oldest);
            }
        }
    }

    fn remove(&mut self, id: u32) {
        if let Some(old) = self.images.remove(&id) {
            self.bytes -= cost(&old);
            self.order.retain(|&i| i != id);
            self.numbers.retain(|_, v| *v != id);
        }
    }
}

fn decode(cmd: &Cmd, payload: &[u8]) -> Result<Arc<ImageData>, &'static str> {
    if cmd.medium != b'd' {
        return Err("EINVAL:only direct transmission (t=d) is supported");
    }
    let raw = base64_decode(std::str::from_utf8(payload).map_err(|_| "EINVAL:bad base64")?).ok_or("EINVAL:bad base64")?;
    let data = if cmd.zlib {
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&raw, (MAX_PIXELS * 4) as usize).map_err(|_| "EINVAL:bad zlib data")?
    } else {
        raw
    };
    match cmd.format {
        24 | 32 => {
            let (w, h) = (cmd.width, cmd.height);
            if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
                return Err("EINVAL:bad image size");
            }
            let n = w as usize * h as usize;
            if cmd.format == 32 {
                if data.len() != n * 4 {
                    return Err("ENODATA:wrong amount of data");
                }
                Ok(ImageData::new(w, h, data))
            } else {
                if data.len() != n * 3 {
                    return Err("ENODATA:wrong amount of data");
                }
                let mut rgba = Vec::with_capacity(n * 4);
                for p in data.chunks_exact(3) {
                    rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
                Ok(ImageData::new(w, h, rgba))
            }
        }
        100 => decode_png(&data),
        _ => Err("EINVAL:unsupported format"),
    }
}

pub(crate) fn decode_png(data: &[u8]) -> Result<Arc<ImageData>, &'static str> {
    let mut dec = png::Decoder::new_with_limits(std::io::Cursor::new(data), png::Limits { bytes: (MAX_PIXELS * 4) as usize });
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|_| "EBADPNG:bad png")?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
        return Err("EINVAL:bad image size");
    }
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|_| "EBADPNG:bad png")?;
    buf.truncate(info.buffer_size());
    let n = w as usize * h as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("EBADPNG:unexpanded palette"),
    };
    if rgba.len() != n * 4 {
        return Err("EBADPNG:bad png");
    }
    Ok(ImageData::new(w, h, rgba))
}

impl Terminal {
    pub(crate) fn kitty_apc(&mut self, data: &[u8]) {
        let Some(body) = data.strip_prefix(b"G") else { return };
        let (keys, payload) = match body.iter().position(|&b| b == b';') {
            Some(i) => (&body[..i], &body[i + 1..]),
            None => (body, &[][..]),
        };
        let cmd = parse_keys(keys);

        // continuation of a chunked transfer: only m (and q) matter
        if let Some((first, mut buf)) = self.kitty.pending.take() {
            if buf.len() + payload.len() > MAX_CHUNKED {
                self.kitty_reply(&first, Err("EFBIG:image too large"));
                return;
            }
            buf.extend_from_slice(payload);
            if cmd.more {
                self.kitty.pending = Some((first, buf));
            } else {
                self.kitty_run(first, &buf);
            }
            return;
        }
        if cmd.more && matches!(cmd.action, b't' | b'T' | b'q') {
            self.kitty.pending = Some((cmd, payload.to_vec()));
            return;
        }
        self.kitty_run(cmd, payload);
    }

    fn kitty_run(&mut self, mut cmd: Cmd, payload: &[u8]) {
        match cmd.action {
            b'q' => {
                let r = decode(&cmd, payload).map(|_| ());
                self.kitty_reply(&cmd, r);
            }
            b't' | b'T' => {
                let img = match decode(&cmd, payload) {
                    Ok(i) => i,
                    Err(e) => return self.kitty_reply(&cmd, Err(e)),
                };
                if cmd.id == 0 && cmd.number != 0 {
                    // the terminal picks the id and reports it
                    self.kitty.next_auto_id = self.kitty.next_auto_id.wrapping_add(1).max(1 << 31);
                    cmd.id = self.kitty.next_auto_id;
                    self.kitty.numbers.insert(cmd.number, cmd.id);
                }
                if cmd.id != 0 {
                    self.kitty.store(cmd.id, img.clone());
                }
                if cmd.action == b'T' {
                    if cmd.unicode {
                        return self.kitty_reply(&cmd, Err("EINVAL:unicode placeholders are not supported"));
                    }
                    self.kitty_place(&cmd, img);
                }
                self.kitty_reply(&cmd, Ok(()));
            }
            b'p' => {
                let id = if cmd.id != 0 { cmd.id } else { self.kitty.numbers.get(&cmd.number).copied().unwrap_or(0) };
                cmd.id = id;
                match self.kitty.images.get(&id).cloned() {
                    Some(img) if !cmd.unicode => {
                        self.kitty_place(&cmd, img);
                        self.kitty_reply(&cmd, Ok(()));
                    }
                    Some(_) => self.kitty_reply(&cmd, Err("EINVAL:unicode placeholders are not supported")),
                    None => self.kitty_reply(&cmd, Err("ENOENT:no such image")),
                }
            }
            b'd' => self.kitty_delete(&cmd),
            _ => {}
        }
    }

    fn kitty_place(&mut self, cmd: &Cmd, img: Arc<ImageData>) {
        let (iw, ih) = (img.width, img.height);
        let x = cmd.x.min(iw.saturating_sub(1));
        let y = cmd.y.min(ih.saturating_sub(1));
        let w = if cmd.w == 0 { iw - x } else { cmd.w.min(iw - x) };
        let h = if cmd.h == 0 { ih - y } else { cmd.h.min(ih - y) };
        // a placement with the same image + placement id replaces the old one
        if cmd.id != 0 && cmd.placement != 0 {
            let (id, pl) = (cmd.id, cmd.placement);
            self.remove_images(true, |s| matches!(s.source, Source::Kitty { id: i, placement: p, .. } if i == id && p == pl));
        }
        self.place_image(Placement {
            image: img,
            source: Source::Kitty { id: cmd.id, number: cmd.number, placement: cmd.placement },
            src: [x as f32, y as f32, w as f32, h as f32],
            cols: cmd.cols,
            rows: cmd.rows,
            z: cmd.z,
            cursor: if cmd.no_move { CursorAfter::Stay } else { CursorAfter::RightOfLastRow },
        });
    }

    fn kitty_delete(&mut self, cmd: &Cmd) {
        let is_kitty = |s: &crate::image::ImageSlice| matches!(s.source, Source::Kitty { .. });
        let free = cmd.delete.is_ascii_uppercase();
        let (cx, cy) = (self.cursor.x, self.cursor.y);
        match cmd.delete.to_ascii_lowercase() {
            0 | b'a' => {
                let mut ids = Vec::new();
                self.remove_images(false, |s| {
                    if let Source::Kitty { id, .. } = s.source {
                        ids.push(id);
                        true
                    } else {
                        false
                    }
                });
                if free {
                    for id in ids {
                        self.kitty.remove(id);
                    }
                }
            }
            b'i' | b'n' => {
                let id = if cmd.delete.eq_ignore_ascii_case(&b'n') { self.kitty.numbers.get(&cmd.number).copied().unwrap_or(0) } else { cmd.id };
                if id == 0 {
                    return;
                }
                let pl = cmd.placement;
                self.remove_images(true, |s| matches!(s.source, Source::Kitty { id: i, placement: p, .. } if i == id && (pl == 0 || p == pl)));
                if free {
                    self.kitty.remove(id);
                }
            }
            b'c' | b'p' | b'x' | b'y' | b'z' => {
                let (px, py) = match cmd.delete.to_ascii_lowercase() {
                    b'c' => (Some(cx), Some(cy)),
                    b'p' => (Some(cmd.x.saturating_sub(1) as usize), Some(cmd.y.saturating_sub(1) as usize)),
                    b'x' => (Some(cmd.x.saturating_sub(1) as usize), None),
                    b'y' => (None, Some(cmd.y.saturating_sub(1) as usize)),
                    _ => (None, None),
                };
                let rows: Vec<usize> = match py {
                    Some(y) => vec![y],
                    None => (0..self.rows).collect(),
                };
                let mut pids = Vec::new();
                for y in rows {
                    pids.extend(self.placements_on_row(y, |s| {
                        is_kitty(s) && px.is_none_or(|x| s.covers_col(x)) && (!cmd.delete.eq_ignore_ascii_case(&b'z') || s.z == cmd.z)
                    }));
                }
                pids.sort_unstable();
                pids.dedup();
                self.remove_placements(&pids);
            }
            _ => {}
        }
    }

    fn kitty_reply(&mut self, cmd: &Cmd, r: Result<(), &str>) {
        if cmd.id == 0 && cmd.number == 0 {
            return;
        }
        match (&r, cmd.quiet) {
            (_, 2) | (Ok(()), 1) => return,
            _ => {}
        }
        let mut s = String::from("\x1b_G");
        if cmd.id != 0 {
            s.push_str(&format!("i={}", cmd.id));
        }
        if cmd.number != 0 {
            if cmd.id != 0 {
                s.push(',');
            }
            s.push_str(&format!("I={}", cmd.number));
        }
        if cmd.placement != 0 {
            s.push_str(&format!(",p={}", cmd.placement));
        }
        s.push(';');
        s.push_str(match r {
            Ok(()) => "OK",
            Err(e) => e,
        });
        s.push_str("\x1b\\");
        self.respond(s.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use crate::links::base64_encode;
    use crate::Terminal;

    fn feed(t: &mut Terminal, s: &str) {
        let mut p = vibex_vt::Parser::new();
        p.advance_bytes(t, s.as_bytes());
    }

    fn rgba(w: usize, h: usize) -> String {
        base64_encode(&vec![200u8; w * h * 4])
    }

    #[test]
    fn transmit_and_display() {
        let mut t = Terminal::new(40, 10);
        t.set_cell_pixels(10, 20);
        feed(&mut t, &format!("\x1b_Ga=T,f=32,s=20,v=40,i=7;{}\x1b\\", rgba(20, 40)));
        assert_eq!(t.screen_image_count(), 2);
        assert_eq!(t.take_responses(), b"\x1b_Gi=7;OK\x1b\\");
        assert_eq!((t.cursor.x, t.cursor.y), (2, 1));
        // put it again elsewhere, then delete by id
        feed(&mut t, "\r\n\x1b_Ga=p,i=7,q=1\x1b\\");
        assert_eq!(t.screen_image_count(), 4);
        assert!(t.take_responses().is_empty(), "q=1 hides OK");
        feed(&mut t, "\x1b_Ga=d,d=i,i=7\x1b\\");
        assert_eq!(t.screen_image_count(), 0);
    }

    #[test]
    fn chunked_png_and_errors() {
        // a 1x1 red PNG
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 1, 1);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&[255, 0, 0]).unwrap();
        }
        let b64 = base64_encode(&png);
        let (a, b) = b64.split_at(8);
        let mut t = Terminal::new(20, 5);
        feed(&mut t, &format!("\x1b_Ga=T,f=100,i=3,m=1;{a}\x1b\\\x1b_Gm=0;{b}\x1b\\"));
        assert_eq!(t.take_responses(), b"\x1b_Gi=3;OK\x1b\\");
        assert_eq!(t.screen_image_count(), 1);
        let img = &t.screen_row(0).images[0].image;
        assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]);

        // a=q answers without storing; files are refused
        feed(&mut t, &format!("\x1b_Ga=q,i=31,s=1,v=1,f=24;{}\x1b\\", base64_encode(&[1, 2, 3])));
        assert_eq!(t.take_responses(), b"\x1b_Gi=31;OK\x1b\\");
        feed(&mut t, &format!("\x1b_Ga=T,t=f,i=4;{}\x1b\\", base64_encode(b"/etc/passwd")));
        let r = String::from_utf8(t.take_responses()).unwrap();
        assert!(r.starts_with("\x1b_Gi=4;EINVAL"), "{r}");
        feed(&mut t, "\x1b_Ga=p,i=99\x1b\\");
        assert!(String::from_utf8(t.take_responses()).unwrap().contains("ENOENT"));
    }

    #[test]
    fn delete_at_cursor_removes_whole_placement() {
        let mut t = Terminal::new(40, 10);
        t.set_cell_pixels(10, 10);
        feed(&mut t, &format!("\x1b_Ga=T,f=32,s=10,v=30,C=1;{}\x1b\\", rgba(10, 30)));
        assert_eq!(t.screen_image_count(), 3);
        assert_eq!((t.cursor.x, t.cursor.y), (0, 0), "C=1 keeps the cursor");
        feed(&mut t, "\x1b_Ga=d,d=c\x1b\\");
        assert_eq!(t.screen_image_count(), 0);
    }
}
