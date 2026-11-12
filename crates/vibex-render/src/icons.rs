//! VibeX Icons, "Circuit": Vibex's own icons.
//!
//! The look: a 24 × 24 grid and a 1.5 stroke like most line icons, but
//! corners are cut at 45° instead of rounded, round things become hexagons
//! and octagons, and every icon carries one cyan element (a node where a
//! trace ends, or the stroke that matters), like a circuit board. At least
//! 2 units of space to the edge, so the set reads as one family. Icons are written as a small
//! subset of SVG (`path`, `circle`, `rect`, and `dot` = a filled circle),
//! which is also what `svg()` writes out, so the same source becomes an
//! icon pack later.
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
    // pages
    ("appearance", "path: M8.25 3.25 H15.75 L20.75 8.25 V15.75 L15.75 20.75 H8.25 L3.25 15.75 V8.25 Z; dot: 8.75 9.75 1.15; dot: 13.75 8.25 1.15; a-dot: 9.75 14.75 1.3; path: M14.25 15.75 L16.5 13.5"),
    ("terminal", "path: M5.5 4.5 H18.5 L20.5 6.5 V17.5 L18.5 19.5 H5.5 L3.5 17.5 V6.5 Z; path: M7.25 9.25 L10 12 L7.25 14.75; a-path: M12.75 14.75 H15.25; a-dot: 16.75 14.75 1.15"),
    ("security", "path: M12 3.25 L19.25 5.75 V12 L12 20.75 L4.75 12 V5.75 Z; a-path: M8.75 12 L11 14.25 L14.25 11; a-dot: 15.4 9.85 1.15"),
    ("profile", "path: M10.5 4.25 H13.5 L15.5 6.25 V9.25 L13.5 11.25 H10.5 L8.5 9.25 V6.25 Z; path: M5.25 20 V17.25 L8 14.5 H16 L18.75 17.25 V20; a-dot: 12 17.5 1.15"),
    ("keyboard", "path: M4.75 6 H19.25 L21.25 8 V16 L19.25 18 H4.75 L2.75 16 V8 Z; dot: 7 10 .95; dot: 10.33 10 .95; dot: 13.67 10 .95; a-dot: 17 10 1.1; path: M8 14.25 h8"),
    ("download", "path: M12 3.75 v9.25; a-path: M8.5 9.75 L12 13.25 L15.5 9.75; path: M4.25 15.25 V18 L6.25 20 H17.75 L19.75 18 V15.25"),
    ("info", "path: M8.25 3.25 H15.75 L20.75 8.25 V15.75 L15.75 20.75 H8.25 L3.25 15.75 V8.25 Z; path: M12 11 v5.25; a-dot: 12 7.9 1.15"),
    ("sliders", "path: M3.5 7.5 H9.75; path: M11 5.5 H15 L16.25 7.5 L15 9.5 H11 L9.75 7.5 Z; a-path: M16.25 7.5 H20.5; path: M3.5 16.5 H5.75; path: M7 14.5 H11 L12.25 16.5 L11 18.5 H7 L5.75 16.5 Z; a-path: M12.25 16.5 H20.5"),
    // actions
    ("plus", "path: M12 5.5 v13; path: M5.5 12 h13"),
    ("close", "path: M6.75 6.75 l10.5 10.5; path: M17.25 6.75 l-10.5 10.5"),
    ("chevron-down", "path: M7 10 l5 5 5-5"),
    ("check", "path: M5.5 12.5 l4 4 9-9"),
    ("copy", "path: M10.25 8.25 H18.75 L20.75 10.25 V18.75 L18.75 20.75 H10.25 L8.25 18.75 V10.25 Z; path: M15.75 8.25 V5.25 L13.75 3.25 H5.25 L3.25 5.25 V13.75 L5.25 15.75 H8.25; a-dot: 14.5 14.5 1.15"),
    ("paste", "path: M7.25 4.75 H16.75 L19.25 7.25 V18.75 L16.75 21.25 H7.25 L4.75 18.75 V7.25 Z; path: M9.25 3 H14.75 V6.25 H9.25 Z; path: M8.75 12 h6.5; a-path: M8.75 15.75 h4"),
    ("split-right", "path: M5.5 4.5 H18.5 L20.5 6.5 V17.5 L18.5 19.5 H5.5 L3.5 17.5 V6.5 Z; a-path: M12 4.5 V10.25; a-path: M12 13.75 V19.5; a-dot: 12 12 1.25"),
    ("split-down", "path: M5.5 4.5 H18.5 L20.5 6.5 V17.5 L18.5 19.5 H5.5 L3.5 17.5 V6.5 Z; a-path: M3.5 12 H10.25; a-path: M13.75 12 H20.5; a-dot: 12 12 1.25"),
    ("search", "path: M8 4.25 H13 L16.25 7.5 V12.5 L13 15.75 H8 L4.75 12.5 V7.5 Z; path: M15.25 14.75 L18.75 18.25; a-path: M7.75 10 H13.25; a-dot: 19.6 19.1 1.2"),
    ("float", "path: M5.5 4.5 H18.5 L20.5 6.5 V17.5 L18.5 19.5 H5.5 L3.5 17.5 V6.5 Z; a-path: M12.25 10.75 H16.5 L17.75 12 V14.75 L16.5 16 H12.25 L11 14.75 V12 Z"),
    ("history", "path: M9.5 3.75 H14.5 L20.25 9.5 V14.5 L14.5 20.25 H9.5 L3.75 14.5 V9.5 Z; path: M12 7.75 V12 L14.75 13.75; a-dot: 12 12 1.1"),
    ("record", "path: M8.25 3.25 H15.75 L20.75 8.25 V15.75 L15.75 20.75 H8.25 L3.25 15.75 V8.25 Z; a-dot: 12 12 3.5"),
    ("note", "path: M6.25 3.25 H14.75 L19.75 8.25 V18.75 L17.75 20.75 H6.25 L4.25 18.75 V5.25 Z; path: M14.75 3.25 V8.25 H19.75; path: M8.25 12.75 h7.5; a-path: M8.25 16.25 h4.5"),
    ("command", "path: M6 3.25 H18 L20.75 6 V18 L18 20.75 H6 L3.25 18 V6 Z; path: M7.5 8.5 h9; path: M7.5 12 h5.5; a-path: M7.5 15.5 h4.5; a-dot: 15.25 15.5 1.1"),
    ("folder", "path: M3.25 6.75 L5.25 4.75 H9.75 L12 7 H18.75 L20.75 9 V17.25 L18.75 19.25 H5.25 L3.25 17.25 Z; a-path: M7 15.25 H11"),
    ("file", "path: M7 3.25 H13.75 L19 8.5 V18.75 L17 20.75 H7 L5 18.75 V5.25 Z; a-path: M13.75 3.25 V8.5 H19"),
    ("link", "path: M10.5 8 H6.25 L3.25 11 V13 L6.25 16 H10.5; path: M13.5 8 H17.75 L20.75 11 V13 L17.75 16 H13.5; a-path: M8.5 12 H15.5"),
    ("key", "path: M5.75 11.5 H8.75 L10.75 13.5 V16.5 L8.75 18.5 H5.75 L3.75 16.5 V13.5 Z; path: M10.75 13.5 L19.25 5; path: M16.5 7.75 L18.75 10; path: M14.25 10 L16 11.75; a-dot: 7.25 15 1.1"),
    ("server", "path: M5.25 4 H18.75 L20.75 6 V9 L18.75 11 H5.25 L3.25 9 V6 Z; path: M5.25 13 H18.75 L20.75 15 V18 L18.75 20 H5.25 L3.25 18 V15 Z; a-dot: 7 7.5 1.1; dot: 7 16.5 1; path: M11 7.5 h5.75; path: M11 16.5 h5.75"),
    ("warning", "path: M10.75 4.25 H13.25 L20.75 17.75 L19.5 19.75 H4.5 L3.25 17.75 Z; path: M12 9.5 v4.25; a-dot: 12 16.5 1.15"),
    ("tab-new", "path: M5.5 4.5 H18.5 L20.5 6.5 V17.5 L18.5 19.5 H5.5 L3.5 17.5 V6.5 Z; a-path: M12 8.75 v6.5; a-path: M8.75 12 h6.5"),
    ("scan", "path: M4 8.5 V6 L6 4 H8.5; path: M15.5 4 H18 L20 6 V8.5; path: M20 15.5 V18 L18 20 H15.5; path: M8.5 20 H6 L4 18 V15.5; a-path: M10.75 9 H13.25 L15 10.75 V13.25 L13.25 15 H10.75 L9 13.25 V10.75 Z"),
    ("target", "path: M8.25 3.25 H15.75 L20.75 8.25 V15.75 L15.75 20.75 H8.25 L3.25 15.75 V8.25 Z; path: M10.25 7.5 H13.75 L16.5 10.25 V13.75 L13.75 16.5 H10.25 L7.5 13.75 V10.25 Z; a-dot: 12 12 1.3"),
    ("trash", "path: M4 6.75 h16; path: M9.25 6.75 V4.75 L10.25 3.75 H13.75 L14.75 4.75 V6.75; path: M6 6.75 L7 19 L8.5 20.5 H15.5 L17 19 L18 6.75; a-path: M10 11 v5.5; path: M14 11 v5.5"),
    ("play", "path: M8 5.25 L9.25 4.5 L19 11.25 V12.75 L9.25 19.5 L8 18.75 Z; a-dot: 11.75 12 1.15"),
    ("eye", "path: M2.75 12 L7 7.25 H17 L21.25 12 L17 16.75 H7 Z; a-path: M10.25 9.25 H13.75 L15.25 12 L13.75 14.75 H10.25 L8.75 12 Z; dot: 12 12 1"),
    ("focus", "path: M4 9 V6 L6 4 H9; path: M15 4 H18 L20 6 V9; path: M20 15 V18 L18 20 H15; path: M9 20 H6 L4 18 V15; a-dot: 12 12 1.3"),
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
