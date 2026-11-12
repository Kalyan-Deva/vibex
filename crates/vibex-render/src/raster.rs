//! Glyph outline rasteriser.
//!
//! Outlines (from ttf-parser) are flattened to line segments and drawn into
//! a signed-area accumulation buffer; a running sum across each row turns
//! that into exact anti-aliased coverage (non-zero winding, approximated by
//! clamping |sum| to 1). This is the classic "accumulation" approach used
//! by font-rs.

use rustybuzz::ttf_parser::{self, GlyphId, OutlineBuilder};

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    acc: Vec<f32>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        // +2 per row of slack: line drawing may touch x = width and width + 1
        Self { width, height, acc: vec![0.0; width * height + width + 4] }
    }

    pub fn line(&mut self, p0: (f32, f32), p1: (f32, f32)) {
        if (p0.1 - p1.1).abs() <= f32::EPSILON {
            return;
        }
        let (dir, p0, p1) = if p0.1 < p1.1 { (1.0, p0, p1) } else { (-1.0, p1, p0) };
        let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
        let mut x = p0.0;
        if p0.1 < 0.0 {
            x -= p0.1 * dxdy;
        }
        let y_start = p0.1.max(0.0) as usize;
        let y_end = (self.height as f32).min(p1.1.ceil()).max(0.0) as usize;
        for y in y_start..y_end {
            let line = y * self.width;
            let dy = ((y + 1) as f32).min(p1.1) - (y as f32).max(p0.1);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let (x0, x1) = (x0.max(0.0), x1.max(0.0));
            let x0floor = x0.floor();
            let x0i = x0floor as usize;
            let x1ceil = x1.ceil();
            let x1i = x1ceil as usize;
            if x1i <= x0i + 1 {
                let xmf = 0.5 * (x + xnext).max(0.0) - x0floor;
                self.add(line + x0i, d - d * xmf);
                self.add(line + x0i + 1, d * xmf);
            } else {
                let s = (x1 - x0).recip();
                let x0f = x0 - x0floor;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = x1 - x1ceil + 1.0;
                let am = 0.5 * s * x1f * x1f;
                self.add(line + x0i, d * a0);
                if x1i == x0i + 2 {
                    self.add(line + x0i + 1, d * (1.0 - a0 - am));
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.add(line + x0i + 1, d * (a1 - a0));
                    for xi in x0i + 2..x1i - 1 {
                        self.add(line + xi, d * s);
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.add(line + x1i - 1, d * (1.0 - a2 - am));
                }
                self.add(line + x1i, d * am);
            }
            x = xnext;
        }
    }

    #[inline]
    fn add(&mut self, i: usize, v: f32) {
        if let Some(a) = self.acc.get_mut(i) {
            *a += v;
        }
    }

    /// Running sum → 8-bit coverage. The sum runs continuously through the
    /// buffer: closed contours contribute zero net area per row, so area
    /// that spills past a row's end is cancelled on the same row.
    pub fn coverage(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width * self.height);
        let mut sum = 0.0f32;
        for i in 0..self.width * self.height {
            sum += self.acc[i];
            out.push((sum.abs().min(1.0) * 255.0 + 0.5) as u8);
        }
        out
    }
}

/// An affine map: (x, y) → (a·x + c·y + e, b·x + d·y + f).
pub type Affine = [f32; 6];

/// Flattens outline curves into canvas lines, mapping font units to pixels.
struct Flatten<'a> {
    canvas: &'a mut Canvas,
    xf: Affine,
    start: (f32, f32),
    last: (f32, f32),
}

impl Flatten<'_> {
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.xf;
        (a * x + c * y + e, b * x + d * y + f)
    }
    fn segments(&self, a: (f32, f32), b: (f32, f32)) -> usize {
        let d = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        (d / 2.0).ceil().clamp(1.0, 32.0) as usize
    }
}

impl OutlineBuilder for Flatten<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.start = p;
        self.last = p;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.canvas.line(self.last, p);
        self.last = p;
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (c, p) = (self.map(x1, y1), self.map(x, y));
        let p0 = self.last;
        let n = self.segments(p0, p);
        let mut prev = p0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let q = (mt * mt * p0.0 + 2.0 * mt * t * c.0 + t * t * p.0, mt * mt * p0.1 + 2.0 * mt * t * c.1 + t * t * p.1);
            self.canvas.line(prev, q);
            prev = q;
        }
        self.last = p;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1, c2, p) = (self.map(x1, y1), self.map(x2, y2), self.map(x, y));
        let p0 = self.last;
        let n = self.segments(p0, p) * 2;
        let mut prev = p0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let (a, b, cc, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
            let q = (a * p0.0 + b * c1.0 + cc * c2.0 + d * p.0, a * p0.1 + b * c1.1 + cc * c2.1 + d * p.1);
            self.canvas.line(prev, q);
            prev = q;
        }
        self.last = p;
    }
    fn close(&mut self) {
        if self.last != self.start {
            self.canvas.line(self.last, self.start);
        }
        self.last = self.start;
    }
}

/// Pixel bounds of a set of glyphs at `scale`: (x0, y0, x1, y1) where y is
/// up (font orientation).
pub fn pixel_bounds(face: &ttf_parser::Face, ids: &[u16], scale: f32) -> Option<(i32, i32, i32, i32)> {
    let mut b: Option<(i32, i32, i32, i32)> = None;
    for &id in ids {
        let Some(r) = face.glyph_bounding_box(GlyphId(id)) else { continue };
        let g = (
            (r.x_min as f32 * scale).floor() as i32,
            (r.y_min as f32 * scale).floor() as i32,
            (r.x_max as f32 * scale).ceil() as i32,
            (r.y_max as f32 * scale).ceil() as i32,
        );
        b = Some(match b {
            None => g,
            Some(p) => (p.0.min(g.0), p.1.min(g.1), p.2.max(g.2), p.3.max(g.3)),
        });
    }
    b.filter(|b| b.2 > b.0 && b.3 > b.1)
}

/// Rasterise one glyph into a canvas whose bottom-left is at pixel
/// (x0, y0) in font orientation and whose size is `w` x `h`.
pub fn render_into(face: &ttf_parser::Face, id: u16, scale: f32, x0: i32, y1: i32, w: usize, h: usize) -> Vec<u8> {
    // px = x·scale − x0, py = y1 − y·scale (font y is up, pixels go down)
    render_affine(face, id, [scale, 0.0, 0.0, -scale, -(x0 as f32), y1 as f32], w, h)
}

/// Rasterise a glyph outline through an arbitrary font-units → pixels map.
pub fn render_affine(face: &ttf_parser::Face, id: u16, xf: Affine, w: usize, h: usize) -> Vec<u8> {
    let mut canvas = Canvas::new(w, h);
    let mut f = Flatten { canvas: &mut canvas, xf, start: (0.0, 0.0), last: (0.0, 0.0) };
    face.outline_glyph(GlyphId(id), &mut f);
    canvas.coverage()
}

/// Fill a closed polygon given in pixel coordinates.
pub fn fill_polygon(points: &[(f32, f32)], w: usize, h: usize) -> Vec<u8> {
    let mut canvas = Canvas::new(w, h);
    for i in 0..points.len() {
        canvas.line(points[i], points[(i + 1) % points.len()]);
    }
    canvas.coverage()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_square_is_solid() {
        let mut c = Canvas::new(4, 4);
        // clockwise-ish square covering the whole canvas
        c.line((0.0, 0.0), (0.0, 4.0));
        c.line((0.0, 4.0), (4.0, 4.0));
        c.line((4.0, 4.0), (4.0, 0.0));
        c.line((4.0, 0.0), (0.0, 0.0));
        assert!(c.coverage().iter().all(|&v| v == 255));
    }

    #[test]
    fn half_pixel_edge_is_half_covered() {
        let mut c = Canvas::new(2, 1);
        c.line((0.0, 0.0), (0.0, 1.0));
        c.line((0.5, 1.0), (0.5, 0.0));
        let cov = c.coverage();
        assert!((cov[0] as i32 - 128).abs() <= 1, "{cov:?}");
        assert_eq!(cov[1], 0);
    }

    #[test]
    fn diagonal_triangle_area() {
        // right triangle with legs 4 → area 8 of 16 pixels
        let mut c = Canvas::new(4, 4);
        c.line((0.0, 0.0), (0.0, 4.0));
        c.line((0.0, 4.0), (4.0, 4.0));
        c.line((4.0, 4.0), (0.0, 0.0));
        let total: f32 = c.coverage().iter().map(|&v| v as f32 / 255.0).sum();
        assert!((total - 8.0).abs() < 0.1, "{total}");
    }
}
