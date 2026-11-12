//! COLR colour glyphs (v0 layers and v1 paint graphs).
//!
//! ttf-parser walks the paint graph and calls a `Painter`. We first record
//! those calls into a display list with every transform resolved (so the
//! canvas can be sized to the real ink), then render the list with:
//! a clip-mask stack (glyph outlines and clip boxes), compositing layers
//! (Porter-Duff + Plus/Screen/Multiply), and exact linear, two-point
//! conical radial and sweep gradients with pad/repeat/reflect. Each pixel
//! is mapped back into the gradient's own space through the inverse
//! transform, so rotated/skewed gradients are right.

use crate::raster::{self, Affine};
use rustybuzz::ttf_parser::{self, colr, GlyphId};

const IDENTITY: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a ∘ b`: apply `b` first, then `a`.
fn compose(a: Affine, b: Affine) -> Affine {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

fn invert(m: Affine) -> Option<Affine> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-9 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}

fn apply(m: Affine, x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// Premultiplied RGBA in 0..1.
type Rgba = [f32; 4];

fn premul(c: ttf_parser::RgbaColor) -> Rgba {
    let a = c.alpha as f32 / 255.0;
    [c.red as f32 / 255.0 * a, c.green as f32 / 255.0 * a, c.blue as f32 / 255.0 * a, a]
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Extend {
    Pad,
    Repeat,
    Reflect,
}

impl From<colr::GradientExtend> for Extend {
    fn from(e: colr::GradientExtend) -> Self {
        match e {
            colr::GradientExtend::Pad => Extend::Pad,
            colr::GradientExtend::Repeat => Extend::Repeat,
            colr::GradientExtend::Reflect => Extend::Reflect,
        }
    }
}

#[derive(Clone, Debug)]
struct ColorLine {
    stops: Vec<(f32, Rgba)>,
    extend: Extend,
}

impl ColorLine {
    fn new(mut stops: Vec<(f32, Rgba)>, extend: Extend) -> Self {
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        ColorLine { stops, extend }
    }

    fn at(&self, t: f32) -> Rgba {
        let s = &self.stops;
        if s.is_empty() {
            return [0.0; 4];
        }
        if s.len() == 1 {
            return s[0].1;
        }
        let (first, last) = (s[0].0, s[s.len() - 1].0);
        let span = (last - first).max(1e-6);
        // extend modes work over the stops' own range
        let u = (t - first) / span;
        let u = match self.extend {
            Extend::Pad => u.clamp(0.0, 1.0),
            Extend::Repeat => u - u.floor(),
            Extend::Reflect => {
                let m = u.rem_euclid(2.0);
                if m > 1.0 {
                    2.0 - m
                } else {
                    m
                }
            }
        };
        let t = first + u * span;
        if t <= first {
            return s[0].1;
        }
        for w in s.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t <= b.0 {
                let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
                return [0, 1, 2, 3].map(|i| a.1[i] + (b.1[i] - a.1[i]) * k);
            }
        }
        s[s.len() - 1].1
    }
}

#[derive(Clone, Debug)]
enum Fill {
    Solid(Rgba),
    Linear { p0: (f32, f32), p3: (f32, f32), line: ColorLine },
    Radial { c0: (f32, f32), r0: f32, c1: (f32, f32), r1: f32, line: ColorLine },
    Sweep { c: (f32, f32), start: f32, end: f32, line: ColorLine },
}

impl Fill {
    /// Colour at a point in the paint's own coordinate space.
    fn at(&self, x: f32, y: f32) -> Option<Rgba> {
        match self {
            Fill::Solid(c) => Some(*c),
            Fill::Linear { p0, p3, line } => {
                let (dx, dy) = (p3.0 - p0.0, p3.1 - p0.1);
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-9 {
                    return Some(line.at(0.0));
                }
                Some(line.at(((x - p0.0) * dx + (y - p0.1) * dy) / len2))
            }
            Fill::Radial { c0, r0, c1, r1, line } => {
                // largest t with |p − c(t)| = r(t), r(t) ≥ 0
                let (cdx, cdy) = (c1.0 - c0.0, c1.1 - c0.1);
                let (pdx, pdy) = (x - c0.0, y - c0.1);
                let dr = r1 - r0;
                let a = cdx * cdx + cdy * cdy - dr * dr;
                let b = pdx * cdx + pdy * cdy + r0 * dr;
                let c = pdx * pdx + pdy * pdy - r0 * r0;
                let t = if a.abs() < 1e-6 {
                    if b.abs() < 1e-9 {
                        return None;
                    }
                    c / (2.0 * b)
                } else {
                    let disc = b * b - a * c;
                    if disc < 0.0 {
                        return None;
                    }
                    let sq = disc.sqrt();
                    let (t1, t2) = ((b + sq) / a, (b - sq) / a);
                    let ok = |t: f32| r0 + t * dr >= 0.0;
                    match (ok(t1.max(t2)), ok(t1.min(t2))) {
                        (true, _) => t1.max(t2),
                        (false, true) => t1.min(t2),
                        _ => return None,
                    }
                };
                Some(line.at(t))
            }
            Fill::Sweep { c, start, end, line } => {
                // angles in units of 180°, counter-clockwise in font space
                let mut ang = (y - c.1).atan2(x - c.0).to_degrees() / 180.0;
                if ang < 0.0 {
                    ang += 2.0;
                }
                let span = end - start;
                if span.abs() < 1e-6 {
                    return Some(line.at(0.0));
                }
                Some(line.at((ang - start) / span))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Clear,
    Src,
    Dst,
    SrcOver,
    DstOver,
    SrcIn,
    DstIn,
    SrcOut,
    DstOut,
    SrcAtop,
    DstAtop,
    Xor,
    Plus,
    Screen,
    Multiply,
}

fn mode_of(m: colr::CompositeMode) -> Mode {
    use colr::CompositeMode as C;
    match m {
        C::Clear => Mode::Clear,
        C::Source => Mode::Src,
        C::Destination => Mode::Dst,
        C::SourceOver => Mode::SrcOver,
        C::DestinationOver => Mode::DstOver,
        C::SourceIn => Mode::SrcIn,
        C::DestinationIn => Mode::DstIn,
        C::SourceOut => Mode::SrcOut,
        C::DestinationOut => Mode::DstOut,
        C::SourceAtop => Mode::SrcAtop,
        C::DestinationAtop => Mode::DstAtop,
        C::Xor => Mode::Xor,
        C::Plus => Mode::Plus,
        C::Screen => Mode::Screen,
        C::Multiply => Mode::Multiply,
        // the blend modes we don't model fall back to normal painting
        _ => Mode::SrcOver,
    }
}

fn composite(mode: Mode, s: Rgba, d: Rgba) -> Rgba {
    let (sa, da) = (s[3], d[3]);
    let pd = |fs: f32, fd: f32| [0, 1, 2, 3].map(|i| s[i] * fs + d[i] * fd);
    match mode {
        Mode::Clear => [0.0; 4],
        Mode::Src => s,
        Mode::Dst => d,
        Mode::SrcOver => pd(1.0, 1.0 - sa),
        Mode::DstOver => pd(1.0 - da, 1.0),
        Mode::SrcIn => pd(da, 0.0),
        Mode::DstIn => pd(0.0, sa),
        Mode::SrcOut => pd(1.0 - da, 0.0),
        Mode::DstOut => pd(0.0, 1.0 - sa),
        Mode::SrcAtop => pd(da, 1.0 - sa),
        Mode::DstAtop => pd(1.0 - da, sa),
        Mode::Xor => pd(1.0 - da, 1.0 - sa),
        Mode::Plus => [0, 1, 2, 3].map(|i| (s[i] + d[i]).min(1.0)),
        Mode::Screen => [0, 1, 2, 3].map(|i| s[i] + d[i] - s[i] * d[i]),
        Mode::Multiply => {
            let c = |i: usize| s[i] * d[i] + s[i] * (1.0 - da) + d[i] * (1.0 - sa);
            [c(0), c(1), c(2), sa + da - sa * da]
        }
    }
}

/// One recorded painter call, with its transform resolved.
#[derive(Clone, Debug)]
enum Op {
    /// v0: fill this glyph directly (no clip push).
    FillGlyph {
        gid: u16,
        xf: Affine,
        fill: Fill,
        fill_xf: Affine,
    },
    ClipGlyph {
        gid: u16,
        xf: Affine,
    },
    ClipBox {
        corners: [(f32, f32); 4],
    },
    PopClip,
    /// Fill the current clip region.
    Paint {
        fill: Fill,
        xf: Affine,
    },
    PushLayer(Mode),
    PopLayer,
}

#[derive(Default)]
struct Recorder {
    xf: Vec<Affine>,
    pending: Option<(u16, Affine)>,
    ops: Vec<Op>,
}

impl Recorder {
    fn cur(&self) -> Affine {
        self.xf.last().copied().unwrap_or(IDENTITY)
    }
}

fn line_from(stops: impl Iterator<Item = colr::ColorStop>, extend: colr::GradientExtend) -> ColorLine {
    ColorLine::new(stops.map(|s| (s.stop_offset, premul(s.color))).collect(), extend.into())
}

impl<'a> colr::Painter<'a> for Recorder {
    fn outline_glyph(&mut self, glyph_id: GlyphId) {
        self.pending = Some((glyph_id.0, self.cur()));
    }
    fn paint(&mut self, paint: colr::Paint<'a>) {
        use colr::Paint;
        let fill = match paint {
            Paint::Solid(c) => Fill::Solid(premul(c)),
            Paint::LinearGradient(g) => {
                // p3 = p0 + projection of (p1 − p0) onto the normal of (p2 − p0)
                let (p0, p1, p2) = ((g.x0, g.y0), (g.x1, g.y1), (g.x2, g.y2));
                let n = (-(p2.1 - p0.1), p2.0 - p0.0);
                let nn = n.0 * n.0 + n.1 * n.1;
                let p3 = if nn > 1e-9 {
                    let k = ((p1.0 - p0.0) * n.0 + (p1.1 - p0.1) * n.1) / nn;
                    (p0.0 + n.0 * k, p0.1 + n.1 * k)
                } else {
                    p1
                };
                Fill::Linear { p0, p3, line: line_from(g.stops(0, &[]), g.extend) }
            }
            Paint::RadialGradient(g) => Fill::Radial { c0: (g.x0, g.y0), r0: g.r0, c1: (g.x1, g.y1), r1: g.r1, line: line_from(g.stops(0, &[]), g.extend) },
            Paint::SweepGradient(g) => {
                Fill::Sweep { c: (g.center_x, g.center_y), start: g.start_angle, end: g.end_angle, line: line_from(g.stops(0, &[]), g.extend) }
            }
        };
        let xf = self.cur();
        match self.pending.take() {
            // COLRv0: outline_glyph followed directly by paint
            Some((gid, gxf)) => self.ops.push(Op::FillGlyph { gid, xf: gxf, fill, fill_xf: xf }),
            None => self.ops.push(Op::Paint { fill, xf }),
        }
    }
    fn push_clip(&mut self) {
        if let Some((gid, xf)) = self.pending.take() {
            self.ops.push(Op::ClipGlyph { gid, xf });
        }
    }
    fn push_clip_box(&mut self, b: colr::ClipBox) {
        let xf = self.cur();
        let corners = [apply(xf, b.x_min, b.y_min), apply(xf, b.x_max, b.y_min), apply(xf, b.x_max, b.y_max), apply(xf, b.x_min, b.y_max)];
        self.ops.push(Op::ClipBox { corners });
    }
    fn pop_clip(&mut self) {
        self.ops.push(Op::PopClip);
    }
    fn push_layer(&mut self, mode: colr::CompositeMode) {
        self.ops.push(Op::PushLayer(mode_of(mode)));
    }
    fn pop_layer(&mut self) {
        self.ops.push(Op::PopLayer);
    }
    fn push_transform(&mut self, t: ttf_parser::Transform) {
        let m = [t.a, t.b, t.c, t.d, t.e, t.f];
        let next = compose(self.cur(), m);
        self.xf.push(next);
    }
    fn pop_transform(&mut self) {
        self.xf.pop();
    }
}

/// A rendered colour glyph: premultiplied RGBA and its size.
pub struct ColorBitmap {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<[u8; 4]>,
}

/// Font-space bounds of all the outlines the glyph uses.
fn bounds(face: &ttf_parser::Face, ops: &[Op]) -> Option<(f32, f32, f32, f32)> {
    let mut b: Option<(f32, f32, f32, f32)> = None;
    let mut add = |x: f32, y: f32| {
        b = Some(match b {
            None => (x, y, x, y),
            Some((a, c, d, e)) => (a.min(x), c.min(y), d.max(x), e.max(y)),
        });
    };
    for op in ops {
        let (gid, xf) = match op {
            Op::FillGlyph { gid, xf, .. } | Op::ClipGlyph { gid, xf } => (*gid, *xf),
            _ => continue,
        };
        if let Some(r) = face.glyph_bounding_box(GlyphId(gid)) {
            for (x, y) in
                [(r.x_min as f32, r.y_min as f32), (r.x_max as f32, r.y_min as f32), (r.x_max as f32, r.y_max as f32), (r.x_min as f32, r.y_max as f32)]
            {
                let (tx, ty) = apply(xf, x, y);
                add(tx, ty);
            }
        }
    }
    b.filter(|b| b.2 > b.0 && b.3 > b.1)
}

/// Render colour glyph `id` so its ink fits in `box_w` × `box_h` pixels.
pub fn render(face: &ttf_parser::Face, id: u16, box_w: f32, box_h: f32) -> Option<ColorBitmap> {
    let mut rec = Recorder::default();
    face.paint_color_glyph(GlyphId(id), 0, ttf_parser::RgbaColor::new(255, 255, 255, 255), &mut rec)?;
    if rec.ops.is_empty() {
        return None;
    }
    let (x0, y0, x1, y1) = bounds(face, &rec.ops)?;
    let scale = (box_w / (x1 - x0)).min(box_h / (y1 - y0));
    let w = ((x1 - x0) * scale).ceil().max(1.0) as usize;
    let h = ((y1 - y0) * scale).ceil().max(1.0) as usize;
    // font units → pixels (y flipped)
    let base: Affine = [scale, 0.0, 0.0, -scale, -x0 * scale, y1 * scale];
    let n = w * h;

    let mut layers: Vec<(Vec<Rgba>, Mode)> = vec![(vec![[0.0; 4]; n], Mode::SrcOver)];
    let mut clips: Vec<Vec<f32>> = Vec::new();
    let to_mask = |cov: Vec<u8>| cov.into_iter().map(|c| c as f32 / 255.0).collect::<Vec<f32>>();

    let fill_into = |layer: &mut Vec<Rgba>, mask: &[f32], fill: &Fill, paint_xf: Affine| {
        let Some(inv) = invert(compose(base, paint_xf)) else { return };
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let m = mask[i];
                if m <= 0.0 {
                    continue;
                }
                let (fx, fy) = apply(inv, x as f32 + 0.5, y as f32 + 0.5);
                let Some(c) = fill.at(fx, fy) else { continue };
                let s = c.map(|v| v * m);
                layer[i] = composite(Mode::SrcOver, s, layer[i]);
            }
        }
    };

    for op in &rec.ops {
        match op {
            Op::ClipGlyph { gid, xf } => {
                let mut m = to_mask(raster::render_affine(face, *gid, compose(base, *xf), w, h));
                if let Some(parent) = clips.last() {
                    for (a, b) in m.iter_mut().zip(parent) {
                        *a *= b;
                    }
                }
                clips.push(m);
            }
            Op::ClipBox { corners } => {
                let pts: Vec<(f32, f32)> = corners.iter().map(|&(x, y)| apply(base, x, y)).collect();
                let mut m = to_mask(raster::fill_polygon(&pts, w, h));
                if let Some(parent) = clips.last() {
                    for (a, b) in m.iter_mut().zip(parent) {
                        *a *= b;
                    }
                }
                clips.push(m);
            }
            Op::PopClip => {
                clips.pop();
            }
            Op::FillGlyph { gid, xf, fill, fill_xf } => {
                let mut m = to_mask(raster::render_affine(face, *gid, compose(base, *xf), w, h));
                if let Some(parent) = clips.last() {
                    for (a, b) in m.iter_mut().zip(parent) {
                        *a *= b;
                    }
                }
                let layer = &mut layers.last_mut().unwrap().0;
                fill_into(layer, &m, fill, *fill_xf);
            }
            Op::Paint { fill, xf } => {
                let full;
                let mask: &[f32] = match clips.last() {
                    Some(m) => m,
                    None => {
                        full = vec![1.0; n];
                        &full
                    }
                };
                let layer = &mut layers.last_mut().unwrap().0;
                fill_into(layer, mask, fill, *xf);
            }
            Op::PushLayer(mode) => layers.push((vec![[0.0; 4]; n], *mode)),
            Op::PopLayer => {
                if layers.len() > 1 {
                    let (src, mode) = layers.pop().unwrap();
                    let dst = &mut layers.last_mut().unwrap().0;
                    for (d, s) in dst.iter_mut().zip(src) {
                        *d = composite(mode, s, *d);
                    }
                }
            }
        }
    }

    let out = &layers[0].0;
    let rgba = out.iter().map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect();
    Some(ColorBitmap { width: w, height: h, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_math() {
        let t: Affine = [2.0, 0.0, 0.0, 3.0, 5.0, 7.0];
        let r: Affine = [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]; // 90°
        let m = compose(t, r);
        assert_eq!(apply(m, 1.0, 0.0), (5.0, 10.0));
        let inv = invert(m).unwrap();
        let (x, y) = apply(inv, 5.0, 10.0);
        assert!((x - 1.0).abs() < 1e-5 && y.abs() < 1e-5);
    }

    #[test]
    fn color_line_extend_modes() {
        let red = [1.0, 0.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        let mut l = ColorLine::new(vec![(0.0, red), (1.0, blue)], Extend::Pad);
        assert_eq!(l.at(-1.0), red);
        assert_eq!(l.at(2.0), blue);
        assert!((l.at(0.5)[0] - 0.5).abs() < 1e-5);
        l.extend = Extend::Repeat;
        assert!((l.at(1.25)[0] - 0.75).abs() < 1e-5);
        l.extend = Extend::Reflect;
        assert!((l.at(1.25)[0] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn gradients_evaluate() {
        let line = ColorLine::new(vec![(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])], Extend::Pad);
        let lin = Fill::Linear { p0: (0.0, 0.0), p3: (10.0, 0.0), line: line.clone() };
        assert!((lin.at(5.0, 99.0).unwrap()[0] - 0.5).abs() < 1e-5, "perpendicular offset doesn't matter");
        let rad = Fill::Radial { c0: (0.0, 0.0), r0: 0.0, c1: (0.0, 0.0), r1: 10.0, line: line.clone() };
        assert!((rad.at(5.0, 0.0).unwrap()[0] - 0.5).abs() < 1e-4);
        let sweep = Fill::Sweep { c: (0.0, 0.0), start: 0.0, end: 2.0, line };
        // straight up = 90° = 0.5 in 180° units → a quarter of the way round
        assert!((sweep.at(0.0, 1.0).unwrap()[0] - 0.25).abs() < 1e-4);
    }

    #[test]
    fn porter_duff() {
        let s = [0.5, 0.0, 0.0, 0.5];
        let d = [0.0, 0.0, 1.0, 1.0];
        let o = composite(Mode::SrcOver, s, d);
        assert!((o[2] - 0.5).abs() < 1e-6 && (o[3] - 1.0).abs() < 1e-6);
        assert_eq!(composite(Mode::DstIn, s, d), [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(composite(Mode::Clear, s, d), [0.0; 4]);
    }
}
