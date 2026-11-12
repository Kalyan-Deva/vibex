//! CPU backend: draws a `Scene` into a 0RGB framebuffer (softbuffer).
//! Used when no GPU adapter is available, and for headless previews.

use crate::font::{Bitmap, FontSet};
use crate::scene::{Item, Scene};
use crate::theme::blend;

pub struct Frame<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
}

/// Active clip as (x0, y0, x1, y1), exclusive.
type Bounds = (i32, i32, i32, i32);

impl Frame<'_> {
    fn fill(&mut self, clip: Bounds, x: i32, y: i32, w: u32, h: u32, color: u32, alpha: u8) {
        let x0 = x.max(clip.0).max(0) as usize;
        let y0 = y.max(clip.1).max(0) as usize;
        let x1 = ((x + w as i32).min(clip.2).max(0) as usize).min(self.width);
        let y1 = ((y + h as i32).min(clip.3).max(0) as usize).min(self.height);
        if x0 >= x1 {
            return;
        }
        for row in y0..y1 {
            let line = &mut self.pixels[row * self.width + x0..row * self.width + x1];
            if alpha == 255 {
                line.fill(color);
            } else {
                for p in line {
                    *p = blend(*p, color, alpha);
                }
            }
        }
    }

    /// Scale the source rectangle into the destination (nearest sample),
    /// blending straight alpha over what's there.
    fn image(&mut self, clip: Bounds, img: &vibex_term::image::ImageData, it: &crate::scene::ImageItem) {
        let x0 = it.x.max(clip.0).max(0);
        let y0 = it.y.max(clip.1).max(0);
        let x1 = (it.x + it.w as i32).min(clip.2).min(self.width as i32);
        let y1 = (it.y + it.h as i32).min(clip.3).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (iw, ih) = (img.width as i32, img.height as i32);
        let [sx, sy, sw, sh] = it.src;
        let fx = sw / it.w as f32;
        let fy = sh / it.h as f32;
        for y in y0..y1 {
            let v = ((sy + (y - it.y) as f32 * fy + fy * 0.5) as i32).clamp(0, ih - 1);
            let row = &mut self.pixels[y as usize * self.width..(y as usize + 1) * self.width];
            for x in x0..x1 {
                let u = ((sx + (x - it.x) as f32 * fx + fx * 0.5) as i32).clamp(0, iw - 1);
                let o = (v * iw + u) as usize * 4;
                let a = img.rgba[o + 3];
                if a == 0 {
                    continue;
                }
                let src = (img.rgba[o] as u32) << 16 | (img.rgba[o + 1] as u32) << 8 | img.rgba[o + 2] as u32;
                let p = &mut row[x as usize];
                *p = if a == 255 { src } else { blend(*p, src, a) };
            }
        }
    }

    /// Three box blurs in a row (close to a Gaussian of `radius`).
    fn blur(&mut self, c: crate::scene::Clip, radius: f32) {
        let x0 = c.x.max(0) as usize;
        let y0 = c.y.max(0) as usize;
        let x1 = ((c.x + c.w as i32).max(0) as usize).min(self.width);
        let y1 = ((c.y + c.h as i32).max(0) as usize).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let r = ((radius * 0.9).round() as usize).max(1);
        let (w, h, stride) = (x1 - x0, y1 - y0, self.width);
        let mut line = Vec::new();
        for _ in 0..3 {
            for y in y0..y1 {
                let start = y * stride + x0;
                box_line(&mut self.pixels[start..start + w], 1, w, r, &mut line);
            }
            for x in x0..x1 {
                let start = y0 * stride + x;
                box_line(&mut self.pixels[start..], stride, h, r, &mut line);
            }
        }
    }

    #[inline]
    fn px(&mut self, clip: Bounds, x: i32, y: i32) -> Option<&mut u32> {
        if x < clip.0.max(0) || y < clip.1.max(0) || x >= clip.2 || y >= clip.3 {
            return None;
        }
        if x as usize >= self.width || y as usize >= self.height {
            return None;
        }
        Some(&mut self.pixels[y as usize * self.width + x as usize])
    }
}

/// Box-blur `n` pixels spaced `step` apart, edges clamped.
fn box_line(px: &mut [u32], step: usize, n: usize, r: usize, tmp: &mut Vec<u32>) {
    tmp.clear();
    tmp.extend((0..n).map(|i| px[i * step]));
    let at = |i: isize| tmp[i.clamp(0, n as isize - 1) as usize];
    let ch = |p: u32, s: u32| (p >> s) & 0xff;
    let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
    for i in -(r as isize)..=(r as isize) {
        let p = at(i);
        sr += ch(p, 16);
        sg += ch(p, 8);
        sb += ch(p, 0);
    }
    let d = (2 * r + 1) as u32;
    for i in 0..n {
        px[i * step] = (sr / d) << 16 | (sg / d) << 8 | (sb / d);
        let (add, sub) = (at(i as isize + r as isize + 1), at(i as isize - r as isize));
        sr = sr + ch(add, 16) - ch(sub, 16);
        sg = sg + ch(add, 8) - ch(sub, 8);
        sb = sb + ch(add, 0) - ch(sub, 0);
    }
}

pub fn draw(scene: &Scene, fonts: &mut FontSet, frame: &mut Frame) {
    frame.pixels.fill(scene.background);
    let full: Bounds = (0, 0, frame.width as i32, frame.height as i32);
    let mut clip = full;
    for item in &scene.items {
        match *item {
            Item::Clip(c) => {
                clip = c.map_or(full, |c| (c.x, c.y, c.x + c.w as i32, c.y + c.h as i32));
            }
            Item::Rect(r) => frame.fill(clip, r.x, r.y, r.w, r.h, r.color, r.alpha),
            Item::Blur(c, radius) => frame.blur(c, radius),
            Item::Image(im) => frame.image(clip, &scene.images[im.index as usize], &im),
            Item::Glyph(g) => {
                let glyph = fonts.glyph(g.key);
                let gx = g.x + glyph.xmin;
                let gy = g.y - glyph.ymin - glyph.height as i32;
                let passes = if glyph.fake_bold { 2 } else { 1 };
                for pass in 0..passes {
                    for r in 0..glyph.height {
                        for c in 0..glyph.width {
                            let i = r * glyph.width + c;
                            let Some(p) = frame.px(clip, gx + c as i32 + pass, gy + r as i32) else { continue };
                            match &glyph.bitmap {
                                Bitmap::Mono(cov) => *p = blend(*p, g.color, cov[i]),
                                Bitmap::Color(rgba) => {
                                    let [cr, cg, cb, a] = rgba[i];
                                    if a == 0 {
                                        continue;
                                    }
                                    // premultiplied src-over
                                    let ia = 255 - a as u32;
                                    let ch = |src: u8, shift: u32| (src as u32 + ((*p >> shift) & 0xff) * ia / 255).min(255) << shift;
                                    *p = ch(cr, 16) | ch(cg, 8) | ch(cb, 0);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
