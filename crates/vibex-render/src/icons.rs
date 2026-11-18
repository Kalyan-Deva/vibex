//! Vibex's icons: Lucide (https://lucide.dev, ISC licence, see
//! THIRD_PARTY_NOTICES.md), the 24 × 24 line icons, drawn with a 1.5 stroke
//! (Lucide's own is 2) so they sit as lightly as the UI text. Sources are a
//! small subset of SVG (`path`, `circle`, `rect`, and `dot` = a filled
//! circle; an `a-` prefix draws in the accent colour), converted from
//! Lucide's SVG files; `svg()` writes them back out.
//!
//! Drawing: paths are flattened to line segments and each pixel's coverage
//! comes from its distance to the nearest segment (anti-aliased, round caps
//! and joins for free). Results are cached per icon, size and colour.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vibex_term::image::ImageData;

/// Stroke width in grid units.
pub const STROKE: f32 = 1.5;

/// (name, shapes). Shapes are separated by `;`, each `kind: args`.
pub const ICONS: &[(&str, &str)] = &[
    ("appearance", "path: M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z; circle: 13.5 6.5 0.5; circle: 17.5 10.5 0.5; circle: 6.5 12.5 0.5; circle: 8.5 7.5 0.5"),  // lucide: palette
    ("terminal", "path: m7 11 2-2-2-2; path: M11 13h4; rect: 3 3 18 18 2"),  // lucide: square-terminal
    ("security", "path: M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z; path: m9 12 2 2 4-4"),  // lucide: shield-check
    ("profile", "path: M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2; circle: 12 7 4"),  // lucide: user
    ("keyboard", "path: M10 8h.01; path: M12 12h.01; path: M14 8h.01; path: M16 12h.01; path: M18 8h.01; path: M6 8h.01; path: M7 16h10; path: M8 12h.01; rect: 2 4 20 16 2"),  // lucide: keyboard
    ("download", "path: M12 15V3; path: M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4; path: m7 10 5 5 5-5"),  // lucide: download
    ("info", "circle: 12 12 10; path: M12 16v-4; path: M12 8h.01"),  // lucide: info
    ("sliders", "path: M10 5H3; path: M12 19H3; path: M14 3v4; path: M16 17v4; path: M21 12h-9; path: M21 19h-5; path: M21 5h-7; path: M8 10v4; path: M8 12H3"),  // lucide: sliders-horizontal
    ("plus", "path: M5 12h14; path: M12 5v14"),  // lucide: plus
    ("close", "path: M18 6 6 18; path: m6 6 12 12"),  // lucide: x
    ("chevron-down", "path: m6 9 6 6 6-6"),  // lucide: chevron-down
    ("check", "path: M20 6 9 17l-5-5"),  // lucide: check
    ("copy", "rect: 8 8 14 14 2; path: M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"),  // lucide: copy
    ("paste", "path: M11 14h10; path: M16 4h2a2 2 0 0 1 2 2v1.344; path: m17 18 4-4-4-4; path: M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 1.793-1.113; rect: 8 2 8 4 1"),  // lucide: clipboard-paste
    ("split-right", "rect: 3 3 18 18 2; path: M12 3v18"),  // lucide: columns-2
    ("split-down", "rect: 3 3 18 18 2; path: M3 12h18"),  // lucide: rows-2
    ("search", "path: m21 21-4.34-4.34; circle: 11 11 8"),  // lucide: search
    ("float", "path: M21 9V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v10c0 1.1.9 2 2 2h4; rect: 12 13 10 7 2"),  // lucide: picture-in-picture-2
    ("history", "path: M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8; path: M3 3v5h5; path: M12 7v5l4 2"),  // lucide: rotate-ccw-clock
    ("record", "circle: 12 12 1; circle: 12 12 10"),  // lucide: circle-dot
    ("note", "path: M13.4 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-7.4; path: M2 6h4; path: M2 10h4; path: M2 14h4; path: M2 18h4; path: M21.378 5.626a1 1 0 1 0-3.004-3.004l-5.01 5.012a2 2 0 0 0-.506.854l-.837 2.87a.5.5 0 0 0 .62.62l2.87-.837a2 2 0 0 0 .854-.506z"),  // lucide: notebook-pen
    ("command", "path: M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"),  // lucide: command
    ("folder", "path: M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"),  // lucide: folder
    ("file", "path: M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z; path: M14 2v5a1 1 0 0 0 1 1h5"),  // lucide: file
    ("link", "path: M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71; path: M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"),  // lucide: link
    ("key", "path: M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z; circle: 16.5 7.5 0.5"),  // lucide: key-round
    ("server", "rect: 2 2 20 8 2; rect: 2 14 20 8 2; path: M6 6 L6.01 6; path: M6 18 L6.01 18"),  // lucide: server
    ("warning", "path: m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3; path: M12 9v4; path: M12 17h.01"),  // lucide: triangle-exclamation-point
    ("tab-new", "rect: 3 3 18 18 2; path: M8 12h8; path: M12 8v8"),  // lucide: square-plus
    ("scan", "path: M3 7V5a2 2 0 0 1 2-2h2; path: M17 3h2a2 2 0 0 1 2 2v2; path: M21 17v2a2 2 0 0 1-2 2h-2; path: M7 21H5a2 2 0 0 1-2-2v-2"),  // lucide: scan
    ("target", "circle: 12 12 10; path: M22 12 L18 12; path: M6 12 L2 12; path: M12 6 L12 2; path: M12 22 L12 18"),  // lucide: crosshair
    ("trash", "path: M10 11v6; path: M14 11v6; path: M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6; path: M3 6h18; path: M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"),  // lucide: trash
    ("play", "path: M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"),  // lucide: play
    ("eye", "path: M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0; circle: 12 12 3"),  // lucide: eye
    ("focus", "circle: 12 12 3; path: M3 7V5a2 2 0 0 1 2-2h2; path: M17 3h2a2 2 0 0 1 2 2v2; path: M21 17v2a2 2 0 0 1-2 2h-2; path: M7 21H5a2 2 0 0 1-2-2v-2"),  // lucide: focus
];

// ------------------------------------------------------------ geometry

#[derive(Clone, Copy, Debug, PartialEq)]
struct P(f32, f32);

#[derive(Clone, Debug, Default)]
pub struct Shapes {
    /// Stroked line segments.
    segs: Vec<(P, P)>,
    /// Filled circles: centre, radius.
    dots: Vec<(P, f32)>,
    /// The same, in the accent colour.
    a_segs: Vec<(P, P)>,
    a_dots: Vec<(P, f32)>,
    /// Translucent filled shapes (closed polygons): ink, accent.
    fills: Vec<Vec<P>>,
    a_fills: Vec<Vec<P>>,
}

fn nums(s: &str) -> Vec<f32> {
    // "1.6-.86" → 1.6, -0.86 ; ".39-2.34" ; "1e-3"
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' {
            let start = i;
            let mut seen_dot = c == '.';
            let mut seen_e = false;
            i += 1;
            while i < b.len() {
                let d = b[i] as char;
                if d.is_ascii_digit() {
                    i += 1;
                } else if d == '.' && !seen_dot && !seen_e {
                    seen_dot = true;
                    i += 1;
                } else if (d == 'e' || d == 'E') && !seen_e {
                    seen_e = true;
                    i += 1;
                    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            if let Ok(v) = s[start..i].parse() {
                out.push(v);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn cubic(out: &mut Vec<P>, p0: P, p1: P, p2: P, p3: P) {
    let n = 16;
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push(P(a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1));
    }
}

/// SVG's endpoint arc, flattened.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<P>, p0: P, rx: f32, ry: f32, rot: f32, large: bool, sweep: bool, p1: P) {
    if (p0.0 - p1.0).abs() < 1e-6 && (p0.1 - p1.1).abs() < 1e-6 {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-6 || ry < 1e-6 {
        out.push(p1);
        return;
    }
    let phi = rot.to_radians();
    let (cs, sn) = (phi.cos(), phi.sin());
    let (dx, dy) = ((p0.0 - p1.0) / 2.0, (p0.1 - p1.1) / 2.0);
    let x1 = cs * dx + sn * dy;
    let y1 = -sn * dx + cs * dy;
    let lam = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut co = (num / den).sqrt();
    if large == sweep {
        co = -co;
    }
    let cxp = co * rx * y1 / ry;
    let cyp = -co * ry * x1 / rx;
    let cx = cs * cxp - sn * cyp + (p0.0 + p1.0) / 2.0;
    let cy = sn * cxp + cs * cyp + (p0.1 + p1.1) / 2.0;
    let ang = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let d = (ux * vx + uy * vy) / ((ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt());
        let a = d.clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            -a
        } else {
            a
        }
    };
    let t1 = ang(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut dt = ang((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && dt > 0.0 {
        dt -= std::f32::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f32::consts::TAU;
    }
    let n = ((dt.abs() / std::f32::consts::TAU) * 64.0).ceil().max(4.0) as usize;
    for k in 1..=n {
        let t = t1 + dt * k as f32 / n as f32;
        let (x, y) = (rx * t.cos(), ry * t.sin());
        out.push(P(cs * x - sn * y + cx, sn * x + cs * y + cy));
    }
}

/// Flatten an SVG path `d` into polylines.
fn path(d: &str) -> Vec<Vec<P>> {
    let mut lines: Vec<Vec<P>> = Vec::new();
    let mut cur = P(0.0, 0.0);
    let mut start = cur;
    let mut last_ctrl: Option<P> = None;
    // split into (command, numbers)
    let mut cmds: Vec<(char, Vec<f32>)> = Vec::new();
    let mut buf = String::new();
    let mut cmd: Option<char> = None;
    for c in d.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_alphabetic() || c == '\0' {
            if let Some(k) = cmd {
                cmds.push((k, nums(&buf)));
            }
            buf.clear();
            cmd = (c != '\0').then_some(c);
        } else {
            buf.push(c);
        }
    }
    let mut poly: Vec<P> = Vec::new();
    let flush = |poly: &mut Vec<P>, lines: &mut Vec<Vec<P>>| {
        if poly.len() > 1 {
            lines.push(std::mem::take(poly));
        } else {
            poly.clear();
        }
    };
    for (k, v) in cmds {
        let rel = k.is_ascii_lowercase();
        let at = |cur: P, x: f32, y: f32| if rel { P(cur.0 + x, cur.1 + y) } else { P(x, y) };
        match k.to_ascii_uppercase() {
            'M' => {
                flush(&mut poly, &mut lines);
                for (i, c) in v.chunks(2).enumerate() {
                    if c.len() < 2 {
                        break;
                    }
                    cur = at(cur, c[0], c[1]);
                    if i == 0 {
                        start = cur;
                        poly.push(cur);
                    } else {
                        poly.push(cur);
                    }
                }
                last_ctrl = None;
            }
            'L' => {
                for c in v.chunks(2).filter(|c| c.len() == 2) {
                    cur = at(cur, c[0], c[1]);
                    poly.push(cur);
                }
                last_ctrl = None;
            }
            'H' => {
                for x in &v {
                    cur = P(if rel { cur.0 + x } else { *x }, cur.1);
                    poly.push(cur);
                }
                last_ctrl = None;
            }
            'V' => {
                for y in &v {
                    cur = P(cur.0, if rel { cur.1 + y } else { *y });
                    poly.push(cur);
                }
                last_ctrl = None;
            }
            'C' => {
                for c in v.chunks(6).filter(|c| c.len() == 6) {
                    let p1 = at(cur, c[0], c[1]);
                    let p2 = at(cur, c[2], c[3]);
                    let p3 = at(cur, c[4], c[5]);
                    cubic(&mut poly, cur, p1, p2, p3);
                    last_ctrl = Some(p2);
                    cur = p3;
                }
            }
            'S' => {
                for c in v.chunks(4).filter(|c| c.len() == 4) {
                    let p1 = last_ctrl.map_or(cur, |lc| P(2.0 * cur.0 - lc.0, 2.0 * cur.1 - lc.1));
                    let p2 = at(cur, c[0], c[1]);
                    let p3 = at(cur, c[2], c[3]);
                    cubic(&mut poly, cur, p1, p2, p3);
                    last_ctrl = Some(p2);
                    cur = p3;
                }
            }
            'A' => {
                for c in v.chunks(7).filter(|c| c.len() == 7) {
                    let p1 = at(cur, c[5], c[6]);
                    arc(&mut poly, cur, c[0], c[1], c[2], c[3] != 0.0, c[4] != 0.0, p1);
                    cur = p1;
                }
                last_ctrl = None;
            }
            'Z' => {
                poly.push(start);
                cur = start;
                flush(&mut poly, &mut lines);
                poly.push(cur);
                last_ctrl = None;
            }
            _ => {}
        }
    }
    flush(&mut poly, &mut lines);
    lines
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> String {
    let r = r.min(w / 2.0).min(h / 2.0);
    format!(
        "M{} {} H{} A{r} {r} 0 0 1 {} {} V{} A{r} {r} 0 0 1 {} {} H{} A{r} {r} 0 0 1 {} {} V{} A{r} {r} 0 0 1 {} {} Z",
        x + r,
        y,
        x + w - r,
        x + w,
        y + r,
        y + h - r,
        x + w - r,
        y + h,
        x + r,
        x,
        y + h - r,
        y + r,
        x + r,
        y
    )
}

pub fn shapes(src: &str) -> Shapes {
    let mut out = Shapes::default();
    for part in src.split(';') {
        let Some((kind, args)) = part.split_once(':') else { continue };
        let n = nums(args);
        let kind = kind.trim();
        let (accent, kind) = match kind.strip_prefix("a-") {
            Some(k) => (true, k),
            None => (false, kind),
        };
        // translucent fills: polygons, not strokes
        if let Some(fk) = kind.strip_prefix("fill") {
            let d = match fk {
                "" => args.trim().to_string(),
                "-rect" if n.len() >= 4 => rounded_rect(n[0], n[1], n[2], n[3], n.get(4).copied().unwrap_or(0.0)),
                "-circle" if n.len() >= 3 => {
                    let (cx, cy, r) = (n[0], n[1], n[2]);
                    format!("M{} {cy} A{r} {r} 0 1 1 {} {cy} A{r} {r} 0 1 1 {} {cy} Z", cx - r, cx + r, cx - r)
                }
                _ => continue,
            };
            let polys = path(&d);
            if accent {
                out.a_fills.extend(polys)
            } else {
                out.fills.extend(polys)
            }
            continue;
        }
        let d = match kind {
            "path" => args.trim().to_string(),
            "circle" if n.len() >= 3 => {
                let (cx, cy, r) = (n[0], n[1], n[2]);
                format!("M{} {cy} A{r} {r} 0 1 1 {} {cy} A{r} {r} 0 1 1 {} {cy} Z", cx - r, cx + r, cx - r)
            }
            "rect" if n.len() >= 4 => rounded_rect(n[0], n[1], n[2], n[3], n.get(4).copied().unwrap_or(0.0)),
            "dot" if n.len() >= 3 => {
                if accent {
                    out.a_dots.push((P(n[0], n[1]), n[2]))
                } else {
                    out.dots.push((P(n[0], n[1]), n[2]))
                }
                continue;
            }
            _ => continue,
        };
        for line in path(&d) {
            for w in line.windows(2) {
                if accent {
                    out.a_segs.push((w[0], w[1]))
                } else {
                    out.segs.push((w[0], w[1]))
                }
            }
        }
    }
    out
}

pub fn source(name: &str) -> Option<&'static str> {
    ICONS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

// ------------------------------------------------------------ drawing

fn seg_dist(p: P, a: P, b: P) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 { ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    (dx * dx + dy * dy).sqrt()
}

/// Coverage (0–255) of `name` at `size` px square.
pub fn coverage(name: &str, size: u32) -> Option<Vec<u8>> {
    let sh = shapes(source(name)?);
    let s = size as f32 / 24.0;
    // keep the line at least ~1.3 px so small icons stay solid
    let half = (STROKE * s).max(1.3) / 2.0;
    let mut out = vec![0u8; (size * size) as usize];
    for py in 0..size {
        for px in 0..size {
            let p = P((px as f32 + 0.5) / s, (py as f32 + 0.5) / s);
            let mut cov = 0.0f32;
            // only segments near this pixel matter
            let reach = (half + 1.0) / s;
            for (a, b) in &sh.segs {
                if p.0 < a.0.min(b.0) - reach || p.0 > a.0.max(b.0) + reach || p.1 < a.1.min(b.1) - reach || p.1 > a.1.max(b.1) + reach {
                    continue;
                }
                let d = seg_dist(p, *a, *b) * s;
                cov = cov.max((half + 0.5 - d).clamp(0.0, 1.0));
                if cov >= 1.0 {
                    break;
                }
            }
            for (c, r) in &sh.dots {
                let d = (((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt() - r) * s;
                cov = cov.max((0.5 - d).clamp(0.0, 1.0));
            }
            cov = cov.max(stroke_cov(p, s, half, &sh.a_segs, &sh.a_dots));
            out[(py * size + px) as usize] = (cov * 255.0).round() as u8;
        }
    }
    Some(out)
}

fn stroke_cov(p: P, s: f32, half: f32, segs: &[(P, P)], dots: &[(P, f32)]) -> f32 {
    let reach = (half + 1.0) / s;
    let mut cov = 0.0f32;
    for (a, b) in segs {
        if p.0 < a.0.min(b.0) - reach || p.0 > a.0.max(b.0) + reach || p.1 < a.1.min(b.1) - reach || p.1 > a.1.max(b.1) + reach {
            continue;
        }
        cov = cov.max((half + 0.5 - seg_dist(p, *a, *b) * s).clamp(0.0, 1.0));
        if cov >= 1.0 {
            return 1.0;
        }
    }
    for (c, r) in dots {
        let d = (((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt() - r) * s;
        cov = cov.max((0.5 - d).clamp(0.0, 1.0));
    }
    cov
}

/// Even-odd point in polygon.
fn inside(p: P, poly: &[P]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            c = !c;
        }
    }
    c
}

/// Fill coverage with 4×4 samples per pixel.
fn fill_cov(px: u32, py: u32, s: f32, polys: &[Vec<P>]) -> f32 {
    if polys.is_empty() {
        return 0.0;
    }
    let mut hits = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let p = P((px as f32 + (sx as f32 + 0.5) / 4.0) / s, (py as f32 + (sy as f32 + 0.5) / 4.0) / s);
            if polys.iter().any(|poly| inside(p, poly)) {
                hits += 1;
            }
        }
    }
    hits as f32 / 16.0
}

/// An icon source drawn in two colours: straight RGBA, `size` px square.
/// Layers, bottom up: ink fill (18%), accent fill (30%), ink strokes,
/// accent strokes.
pub fn render(src: &str, size: u32, ink: u32, accent: u32) -> Vec<u8> {
    let sh = shapes(src);
    let s = size as f32 / 24.0;
    let half = (STROKE * s).max(1.3) / 2.0;
    let rgb = |c: u32| [((c >> 16) & 0xff) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, (c & 0xff) as f32 / 255.0];
    let (ik, ac) = (rgb(ink), rgb(accent));
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let p = P((px as f32 + 0.5) / s, (py as f32 + 0.5) / s);
            // (colour, alpha) layers, composited "over" in straight alpha
            let layers = [
                (ik, fill_cov(px, py, s, &sh.fills) * 0.18),
                (ac, fill_cov(px, py, s, &sh.a_fills) * 0.30),
                (ik, stroke_cov(p, s, half, &sh.segs, &sh.dots)),
                (ac, stroke_cov(p, s, half, &sh.a_segs, &sh.a_dots)),
            ];
            let (mut c, mut a) = ([0.0f32; 3], 0.0f32);
            for (lc, la) in layers {
                if la <= 0.0 {
                    continue;
                }
                let na = la + a * (1.0 - la);
                for k in 0..3 {
                    c[k] = (lc[k] * la + c[k] * a * (1.0 - la)) / na.max(1e-6);
                }
                a = na;
            }
            out.extend_from_slice(&[(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (a * 255.0).round() as u8]);
        }
    }
    out
}

/// A row of icon sources on `paper`, `cell` px each (0RGB pixels).
pub fn sheet_of(list: &[(&str, &str)], cell: u32, ink: u32, accent: u32, paper: u32) -> (Vec<u32>, usize, usize) {
    let pad = cell / 2;
    let n = list.len() as u32;
    let (w, h) = ((n * (cell + pad) + pad) as usize, (cell + 2 * pad) as usize);
    let mut px = vec![paper; w * h];
    for (i, (_, src)) in list.iter().enumerate() {
        let rgba = render(src, cell, ink, accent);
        let (ox, oy) = ((i as u32 * (cell + pad) + pad) as usize, pad as usize);
        for y in 0..cell as usize {
            for x in 0..cell as usize {
                let o = (y * cell as usize + x) * 4;
                let a = rgba[o + 3];
                if a > 0 {
                    let c = (rgba[o] as u32) << 16 | (rgba[o + 1] as u32) << 8 | rgba[o + 2] as u32;
                    let d = &mut px[(oy + y) * w + ox + x];
                    *d = crate::theme::blend(*d, c, a);
                }
            }
        }
    }
    (px, w, h)
}

type Cache = Mutex<HashMap<(String, u32, u64), Arc<ImageData>>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The cyan the accent parts are drawn in.
pub const ACCENT: u32 = 0x38d9f5;

/// The icon as an image: lines in `color`, accent parts in `accent`.
pub fn image(name: &str, size: u32, color: u32, accent: u32) -> Option<Arc<ImageData>> {
    let size = size.clamp(8, 256);
    let key = (name.to_string(), size, ((color & 0xff_ffff) as u64) << 24 | (accent & 0xff_ffff) as u64);
    if let Some(img) = cache().lock().ok()?.get(&key) {
        return Some(img.clone());
    }
    let rgba = render(source(name)?, size, color, accent);
    let img = ImageData::new(size, size, rgba);
    if let Ok(mut c) = cache().lock() {
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, img.clone());
    }
    Some(img)
}

/// Draw `name` centred at (cx, cy), `size` px square, with the cyan accent.
pub fn draw(scene: &mut crate::scene::Scene, name: &str, cx: f32, cy: f32, size: f32, color: u32) {
    draw_in(scene, name, cx, cy, size, color, ACCENT);
}

/// The same with the accent in `accent` (e.g. white on a cyan highlight).
pub fn draw_in(scene: &mut crate::scene::Scene, name: &str, cx: f32, cy: f32, size: f32, color: u32, accent: u32) {
    let px = size.round().max(8.0) as u32;
    if let Some(img) = image(name, px, color, accent) {
        let (x, y) = ((cx - px as f32 / 2.0).round() as i32, (cy - px as f32 / 2.0).round() as i32);
        scene.image(&img, x, y, px, px, [0.0, 0.0, px as f32, px as f32]);
    }
}

/// The icon as an SVG file (for the pack, and for checking the drawing).
pub fn svg(name: &str) -> Option<String> {
    let src = source(name)?;
    let mut body = String::new();
    for part in src.split(';') {
        let Some((kind, args)) = part.split_once(':') else { continue };
        let n = nums(args);
        let kind = kind.trim();
        let (accent, kind) = match kind.strip_prefix("a-") {
            Some(k) => (" class=\"accent\"", k),
            None => ("", kind),
        };
        match kind {
            "path" => body.push_str(&format!("  <path{accent} d=\"{}\"/>\n", args.trim())),
            "circle" if n.len() >= 3 => body.push_str(&format!("  <circle{accent} cx=\"{}\" cy=\"{}\" r=\"{}\"/>\n", n[0], n[1], n[2])),
            "rect" if n.len() >= 4 => body.push_str(&format!(
                "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\"/>\n",
                n[0],
                n[1],
                n[2],
                n[3],
                n.get(4).copied().unwrap_or(0.0)
            )),
            "dot" if n.len() >= 3 => {
                let cls = if accent.is_empty() { "" } else { " class=\"accent-fill\"" };
                body.push_str(&format!("  <circle{cls} cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"currentColor\" stroke=\"none\"/>\n", n[0], n[1], n[2]))
            }
            _ => {}
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"{STROKE}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n  <style>.accent{{stroke:#38d9f5}}.accent-fill{{fill:#38d9f5}}</style>\n{body}</svg>\n"
    ))
}

/// Every icon on a grid (light background, dark ink), `cell` px each, for
/// checking them side by side. Returns (pixels 0RGB, width, height).
pub fn sheet(cell: u32, cols: u32, ink: u32, paper: u32) -> (Vec<u32>, usize, usize) {
    let n = ICONS.len() as u32;
    let rows = n.div_ceil(cols);
    let pad = cell / 3;
    let (w, h) = ((cols * (cell + pad) + pad) as usize, (rows * (cell + pad) + pad) as usize);
    let mut px = vec![paper; w * h];
    for (i, (name, _)) in ICONS.iter().enumerate() {
        let (cx, cy) = ((i as u32 % cols) * (cell + pad) + pad, (i as u32 / cols) * (cell + pad) + pad);
        let Some(src) = source(name) else { continue };
        let rgba = render(src, cell, ink, ACCENT);
        for y in 0..cell {
            for x in 0..cell {
                let o4 = ((y * cell + x) * 4) as usize;
                let a = rgba[o4 + 3];
                if a > 0 {
                    let c = (rgba[o4] as u32) << 16 | (rgba[o4 + 1] as u32) << 8 | rgba[o4 + 2] as u32;
                    let o = (cy + y) as usize * w + (cx + x) as usize;
                    px[o] = crate::theme::blend(paper, c, a);
                }
            }
        }
    }
    (px, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_like_svg_writes_them() {
        assert_eq!(nums("1.6-.86 1.27-1.7"), vec![1.6, -0.86, 1.27, -1.7]);
        assert_eq!(nums(".39-2.34.5"), vec![0.39, -2.34, 0.5]);
        assert_eq!(nums("1e-3,2"), vec![0.001, 2.0]);
    }

    #[test]
    fn every_icon_draws_inside_its_grid() {
        for (name, src) in ICONS {
            let sh = shapes(src);
            assert!(!sh.segs.is_empty() || !sh.a_segs.is_empty() || !sh.dots.is_empty(), "{name} is empty");
            for (a, b) in sh.segs.iter().chain(&sh.a_segs) {
                for p in [a, b] {
                    assert!(p.0 >= 1.9 && p.0 <= 22.1 && p.1 >= 1.9 && p.1 <= 22.1, "{name} leaves the 2-unit margin at {p:?}");
                }
            }
            let cov = coverage(name, 24).unwrap();
            assert!(cov.iter().any(|&a| a > 180), "{name} has no solid stroke");
            assert!(svg(name).unwrap().contains("viewBox=\"0 0 24 24\""));
        }
    }

    #[test]
    fn arcs_close_circles() {
        let sh = shapes("circle: 12 12 8");
        // every point on the outline is 8 from the centre
        for (a, _) in &sh.segs {
            let r = ((a.0 - 12.0).powi(2) + (a.1 - 12.0).powi(2)).sqrt();
            assert!((r - 8.0).abs() < 0.05, "{r}");
        }
        let names: std::collections::HashSet<_> = ICONS.iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), ICONS.len(), "duplicate names");
    }
}
