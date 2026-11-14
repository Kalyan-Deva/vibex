//! Sixel decoder (`DCS P1;P2;P3 q <data> ST`), fed one byte at a time.
//!
//! Supports colour registers (RGB and HLS), repeat, raster attributes and
//! the transparent-background flag (P2 = 1). Pixel aspect ratio is treated
//! as 1:1, like other modern terminals.

use crate::image::{ImageData, MAX_SIDE};
use std::sync::Arc;

/// Sixel images are capped tighter than kitty ones: they arrive as a
/// stream with no declared size.
const MAX_W: usize = 4096;
const MAX_H: usize = 4096;
const REGISTERS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Data,
    Repeat,
    Color,
    Raster,
}

pub(crate) struct Sixel {
    palette: [[u8; 3]; REGISTERS],
    color: usize,
    x: usize,
    y: usize,
    /// Allocated canvas (grows on demand).
    w: usize,
    h: usize,
    pixels: Vec<u32>,
    /// Extent actually drawn.
    max_x: usize,
    max_y: usize,
    raster: Option<(usize, usize)>,
    st: St,
    params: Vec<u32>,
    num: u32,
    num_seen: bool,
    repeat: usize,
}

/// VT340 default colour registers (percent).
const VT340: [[u8; 3]; 16] = [
    [0, 0, 0],
    [20, 20, 80],
    [80, 13, 13],
    [20, 80, 20],
    [80, 20, 80],
    [20, 80, 80],
    [80, 80, 20],
    [53, 53, 53],
    [26, 26, 26],
    [33, 33, 60],
    [60, 26, 26],
    [33, 60, 33],
    [60, 33, 60],
    [33, 60, 60],
    [60, 60, 33],
    [80, 80, 80],
];

fn pct(v: u32) -> u8 {
    ((v.min(100) * 255 + 50) / 100) as u8
}

/// Sixel HLS: hue 0 = blue, 120 = red, 240 = green.
fn hls(h: u32, l: u32, s: u32) -> [u8; 3] {
    let h = ((h + 240) % 360) as f32;
    let l = l.min(100) as f32 / 100.0;
    let s = s.min(100) as f32 / 100.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let f = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    [f(r), f(g), f(b)]
}

impl Sixel {
    pub fn new() -> Self {
        let mut palette = [[0u8; 3]; REGISTERS];
        for (i, c) in VT340.iter().enumerate() {
            palette[i] = [pct(c[0] as u32), pct(c[1] as u32), pct(c[2] as u32)];
        }
        Self {
            palette,
            color: 0,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            pixels: Vec::new(),
            max_x: 0,
            max_y: 0,
            raster: None,
            st: St::Data,
            params: Vec::new(),
            num: 0,
            num_seen: false,
            repeat: 1,
        }
    }

    fn grow(&mut self, need_w: usize, need_h: usize) {
        let need_w = need_w.min(MAX_W);
        let need_h = need_h.min(MAX_H);
        if need_w <= self.w && need_h <= self.h {
            return;
        }
        let nw = need_w.max(self.w).max(if self.w == 0 { need_w } else { (self.w * 2).min(MAX_W) });
        let nh = need_h.max(self.h).max(if self.h == 0 { need_h } else { (self.h * 2).min(MAX_H) });
        let mut px = vec![0u32; nw * nh];
        for row in 0..self.h {
            px[row * nw..row * nw + self.w].copy_from_slice(&self.pixels[row * self.w..(row + 1) * self.w]);
        }
        self.pixels = px;
        self.w = nw;
        self.h = nh;
    }

    fn end_number(&mut self) {
        if self.num_seen || !self.params.is_empty() {
            self.params.push(self.num);
        }
        self.num = 0;
        self.num_seen = false;
    }

    /// Finish a `#`, `"` or `!` introducer once a byte that isn't part of
    /// its parameters arrives.
    fn finish_params(&mut self) {
        self.end_number();
        match self.st {
            St::Color => {
                let p = &self.params;
                if let Some(&reg) = p.first() {
                    let reg = reg as usize % REGISTERS;
                    if p.len() >= 5 {
                        self.palette[reg] = match p[1] {
                            1 => hls(p[2], p[3], p[4]),
                            _ => [pct(p[2]), pct(p[3]), pct(p[4])],
                        };
                    }
                    self.color = reg;
                }
            }
            St::Raster => {
                if self.params.len() >= 4 {
                    let (w, h) = (self.params[2] as usize, self.params[3] as usize);
                    if w > 0 && h > 0 {
                        self.raster = Some((w.min(MAX_W), h.min(MAX_H)));
                        self.grow(w, h);
                    }
                }
            }
            St::Repeat => self.repeat = self.params.first().copied().unwrap_or(1).max(1) as usize,
            St::Data => {}
        }
        self.params.clear();
        self.st = St::Data;
    }

    pub fn put(&mut self, b: u8) {
        if self.st != St::Data {
            match b {
                b'0'..=b'9' => {
                    self.num = (self.num * 10 + (b - b'0') as u32).min(1 << 20);
                    self.num_seen = true;
                    return;
                }
                b';' => {
                    // no sixel command takes more than 5; an endless `;;;`
                    // must not grow memory
                    if self.params.len() < 8 {
                        self.params.push(self.num);
                    }
                    self.num = 0;
                    self.num_seen = false;
                    return;
                }
                _ => self.finish_params(),
            }
        }
        match b {
            b'#' => self.st = St::Color,
            b'"' => self.st = St::Raster,
            b'!' => self.st = St::Repeat,
            b'$' => self.x = 0,
            b'-' => {
                self.x = 0;
                self.y += 6;
            }
            0x3f..=0x7e => {
                let bits = b - 0x3f;
                let n = std::mem::replace(&mut self.repeat, 1);
                if bits != 0 && self.y < MAX_H && self.x < MAX_W {
                    let end = (self.x + n).min(MAX_W);
                    self.grow(end, self.y + 6);
                    let [r, g, bl] = self.palette[self.color];
                    let px = 0xff00_0000 | (r as u32) << 16 | (g as u32) << 8 | bl as u32;
                    for i in 0..6 {
                        if bits & (1 << i) != 0 {
                            let yy = self.y + i;
                            if yy >= self.h {
                                break;
                            }
                            let rowp = &mut self.pixels[yy * self.w..yy * self.w + self.w];
                            rowp[self.x..end].fill(px);
                            self.max_y = self.max_y.max(yy + 1);
                        }
                    }
                    self.max_x = self.max_x.max(end);
                }
                self.x += n;
            }
            _ => {}
        }
    }

    /// The finished image, or None if nothing was drawn.
    pub fn finish(mut self) -> Option<Arc<ImageData>> {
        if self.st != St::Data {
            self.finish_params();
        }
        let (w, h) = match self.raster {
            // raster attributes give the intended size; keep drawn pixels
            // that spill past it out (encoders round up to 6-pixel bands)
            Some((rw, rh)) => (rw.max(1), rh.max(1)),
            None => (self.max_x, self.max_y),
        };
        if w == 0 || h == 0 || w as u32 > MAX_SIDE || h as u32 > MAX_SIDE {
            return None;
        }
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h.min(self.h) {
            for x in 0..w.min(self.w) {
                let p = self.pixels[y * self.w + x];
                let o = (y * w + x) * 4;
                rgba[o] = (p >> 16) as u8;
                rgba[o + 1] = (p >> 8) as u8;
                rgba[o + 2] = p as u8;
                rgba[o + 3] = (p >> 24) as u8;
            }
        }
        Some(ImageData::new(w as u32, h as u32, rgba))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(data: &str) -> Arc<ImageData> {
        let mut s = Sixel::new();
        for b in data.bytes() {
            s.put(b);
        }
        s.finish().unwrap()
    }

    fn px(img: &ImageData, x: usize, y: usize) -> [u8; 4] {
        let o = (y * img.width as usize + x) * 4;
        [img.rgba[o], img.rgba[o + 1], img.rgba[o + 2], img.rgba[o + 3]]
    }

    #[test]
    fn colours_repeat_and_bands() {
        // register 1 = pure red (RGB %), 3 columns of full sixels, newline,
        // then register 2 = green (HLS: hue 240 = green, 50% light, 100% sat)
        let img = decode("#1;2;100;0;0!3~-#2;1;240;50;100~");
        assert_eq!((img.width, img.height), (3, 12));
        assert_eq!(px(&img, 2, 5), [255, 0, 0, 255]);
        assert_eq!(px(&img, 0, 6), [0, 255, 0, 255]);
        assert_eq!(px(&img, 1, 6)[3], 0, "undrawn pixels are transparent");
    }

    #[test]
    fn raster_attributes_set_size() {
        let img = decode("\"1;1;5;4#0~~");
        assert_eq!((img.width, img.height), (5, 4));
        // '$' returns to column 0 and overprints
        let img = decode("#1;2;0;0;100@$#2;2;100;100;100A");
        assert_eq!(px(&img, 0, 0), [0, 0, 255, 255]);
        assert_eq!(px(&img, 0, 1), [255, 255, 255, 255]);
    }

    #[test]
    fn huge_repeat_is_capped() {
        let img = decode("!999999~");
        assert_eq!(img.width as usize, MAX_W);
    }
}
