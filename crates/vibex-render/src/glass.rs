//! The glassy, cozy look for Vibex's own UI (Settings, dialogs): soft
//! colour glows behind a frosted panel, translucent cards, gradient pills,
//! and small line icons. Everything here is made of rects and spans, so it
//! draws the same on the CPU and GPU backends.

use crate::scene::{hspan, round_top_inset, Scene};
use crate::tabbar::PxRect;
use crate::theme::{blend, Theme};

/// White on dark themes, black on light ones: what "glass" is tinted with.
pub fn tint(theme: &Theme) -> u32 {
    let bg = theme.background;
    let luma = ((bg >> 16) & 0xff) * 299 + ((bg >> 8) & 0xff) * 587 + (bg & 0xff) * 114;
    if luma < 128_000 {
        0xffffff
    } else {
        0x000000
    }
}

/// Cool accents on dark glass: electric cyan to deep blue (gradients,
/// headings, focus).
pub const CYAN: u32 = 0x38d9f5;
pub const BLUE: u32 = 0x4f6ef7;
/// The third glow: a dim teal.
pub const TEAL: u32 = 0x1fb8a6;

/// The two colours the gradients run between.
pub fn accents(_theme: &Theme) -> (u32, u32) {
    (CYAN, BLUE)
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    blend(a, b, (t.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Inset of a rounded shape's edge on row `row` (0 = top) of height `h`.
fn inset(radius: f32, row: i32, h: i32) -> f32 {
    let from_edge = (row as f32).min((h - 1 - row) as f32);
    round_top_inset(radius, from_edge)
}

/// A filled rounded rectangle at `alpha`.
pub fn fill(scene: &mut Scene, r: PxRect, radius: f32, color: u32, alpha: u8) {
    let radius = radius.min(r.h / 2.0).min(r.w / 2.0);
    let (y0, h) = (r.y.round() as i32, r.h.round() as i32);
    for row in 0..h {
        let i = inset(radius, row, h);
        hspan(scene, r.x + i, r.x + r.w - i, y0 + row, color, alpha);
    }
}

/// A rounded rectangle with a left-to-right gradient.
pub fn gradient(scene: &mut Scene, r: PxRect, radius: f32, c0: u32, c1: u32, a0: u8, a1: u8) {
    let radius = radius.min(r.h / 2.0).min(r.w / 2.0);
    let (y0, h) = (r.y.round() as i32, r.h.round() as i32);
    let steps = ((r.w / 4.0).ceil() as usize).clamp(2, 96);
    // segment edges on whole pixels, so neighbours never overlap (no seams);
    // only the shape's own ends are anti-aliased
    let edge = |k: usize| (r.x + r.w * k as f32 / steps as f32).round();
    for row in 0..h {
        let i = inset(radius, row, h);
        let (x0, x1) = (r.x + i, r.x + r.w - i);
        for k in 0..steps {
            let sx0 = if k == 0 { x0 } else { edge(k).max(x0) };
            let sx1 = if k + 1 == steps { x1 } else { edge(k + 1).min(x1) };
            if sx1 > sx0 {
                let t = (k as f32 + 0.5) / steps as f32;
                let a = (a0 as f32 + (a1 as f32 - a0 as f32) * t) as u8;
                hspan(scene, sx0, sx1, y0 + row, mix(c0, c1, t), a);
            }
        }
    }
}

/// A 1px rounded outline at `alpha` (sides only where the fill would go).
pub fn ring(scene: &mut Scene, r: PxRect, radius: f32, color: u32, alpha: u8) {
    let radius = radius.min(r.h / 2.0).min(r.w / 2.0);
    let (y0, h) = (r.y.round() as i32, r.h.round() as i32);
    for row in 0..h {
        let i = inset(radius, row, h);
        let (x0, x1) = (r.x + i, r.x + r.w - i);
        if row == 0 || row == h - 1 {
            hspan(scene, x0, x1, y0 + row, color, alpha);
            continue;
        }
        // the edge is 1px wide plus however far the curve moves this row
        let prev = inset(radius, row - 1, h).max(inset(radius, row + 1, h));
        let w = (prev - i).max(0.0) + 1.0;
        hspan(scene, x0, x0 + w, y0 + row, color, alpha);
        hspan(scene, x1 - w, x1, y0 + row, color, alpha);
    }
}

/// A soft round glow: rings of falling alpha, `r` the radius.
pub fn glow(scene: &mut Scene, cx: f32, cy: f32, r: f32, color: u32, peak: u8) {
    // many faint discs stacked: each step adds a little, so the edge fades
    // smoothly from nothing to `peak` at the centre
    let rings = 40;
    let step = (peak as f32 / rings as f32).max(1.0);
    for k in 0..rings {
        let rr = r * (1.0 - k as f32 / rings as f32).sqrt();
        if rr < 1.0 {
            continue;
        }
        fill(scene, PxRect { x: cx - rr, y: cy - rr, w: rr * 2.0, h: rr * 2.0 }, rr, color, step.round() as u8);
    }
}

/// The frosted panel a dialog sits on: a soft drop shadow, colour glows
/// behind the glass, the glass, a sheen along the top and a light edge.
pub fn panel(scene: &mut Scene, theme: &Theme, r: PxRect, s: f32) {
    let radius = 16.0 * s;
    let t = tint(theme);
    // shadow: a few wider, fainter layers
    for (grow, a) in [(18.0, 10u8), (10.0, 16), (4.0, 28)] {
        let g = grow * s;
        fill(scene, PxRect { x: r.x - g, y: r.y - g + 8.0 * s, w: r.w + 2.0 * g, h: r.h + 2.0 * g }, radius + g, 0x000000, a);
    }
    // the colours the glass picks up
    let (c0, c1) = accents(theme);
    scene.clip(Some(r));
    glow(scene, r.x + r.w * 0.04, r.y + r.h * 0.00, r.w.max(r.h) * 0.40, c1, 150);
    glow(scene, r.x + r.w * 0.98, r.y + r.h * 1.00, r.w.max(r.h) * 0.42, c0, 120);
    glow(scene, r.x + r.w * 0.70, r.y + r.h * 0.25, r.w.max(r.h) * 0.24, TEAL, 50);
    scene.clip(None);
    // smoked glass: darker than the theme's background, mostly opaque, so
    // the glows are a faint light behind it
    let smoke = if t == 0xffffff { blend(theme.background, 0x05070b, 120) } else { blend(theme.background, 0xffffff, 120) };
    fill(scene, r, radius, smoke, 214);
    // sheen fading down from the top
    let sheen_h = (r.h * 0.22).min(140.0 * s);
    let bands = 10;
    for b in 0..bands {
        let y = r.y + sheen_h * b as f32 / bands as f32;
        let a = (8.0 * (1.0 - b as f32 / bands as f32)) as u8;
        let band = PxRect { x: r.x, y, w: r.w, h: (sheen_h / bands as f32).ceil() };
        if b == 0 {
            // keep the rounded corners
            fill(scene, PxRect { h: band.h + radius, ..band }, radius, t, a / 2);
        } else {
            scene.frect(PxRect { x: r.x + 1.0, ..band }, t, a);
        }
    }
    ring(scene, r, radius, t, 34);
    // the top edge catches the light
    hspan(scene, r.x + radius, r.x + r.w - radius, r.y.round() as i32, t, 60);
}

/// A translucent card (rows, boxes): just lifted off the glass.
pub fn card(scene: &mut Scene, theme: &Theme, r: PxRect, radius: f32, hovered: bool) {
    let t = tint(theme);
    fill(scene, r, radius, t, if hovered { 16 } else { 8 });
    ring(scene, r, radius, t, if hovered { 40 } else { 20 });
}

/// A gradient pill (toggle on, slider fill, selected page).
pub fn pill(scene: &mut Scene, theme: &Theme, r: PxRect, radius: f32, a0: u8, a1: u8) {
    let (c0, c1) = accents(theme);
    gradient(scene, r, radius, c0, c1, a0, a1);
}
