//! The Settings page and popup menus, drawn in the same style as the
//! title bar. The app describes what to show (`SettingsView`, `MenuView`);
//! drawing returns hit regions for the mouse.

use crate::font::FontSet;
use crate::scene::{fit, hspan, line, outline, round_top_inset, ui_text, ui_width, Scene};
use crate::tabbar::PxRect;
use crate::theme::{blend, Theme};

/// The control on the right of a settings row.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// A section heading (no card).
    Header,
    /// Plain text on the right.
    Info(String),
    Toggle(bool),
    /// ‹ value › with previous / next.
    Choice(String),
    /// 0.0–1.0 and the text shown next to it.
    Slider {
        value: f32,
        text: String,
    },
    /// A one-line text field.
    Text {
        value: String,
        editing: bool,
        placeholder: String,
    },
    /// Key chips ("Ctrl", "Shift", "T"); `capturing` waits for new keys.
    Keys {
        chips: Vec<Vec<String>>,
        capturing: bool,
    },
    Button(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingRow {
    pub label: String,
    /// Smaller text under the label.
    pub detail: String,
    /// Shown in the warning colour instead of `detail` (e.g. a key clash).
    pub warning: Option<String>,
    pub control: Control,
    /// Colour swatches drawn before the control (themes).
    pub swatches: Vec<u32>,
    /// Overridden in settings.lua: offer a reset.
    pub changed: bool,
}

impl SettingRow {
    pub fn new(label: impl Into<String>, control: Control) -> Self {
        Self { label: label.into(), detail: String::new(), warning: None, control, swatches: Vec::new(), changed: false }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = d.into();
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsView {
    pub pages: Vec<String>,
    pub page: usize,
    pub title: String,
    pub rows: Vec<SettingRow>,
    /// Keyboard focus.
    pub focus: Option<usize>,
    pub hover: Option<SettingsHit>,
    /// Scrolled this many pixels down.
    pub scroll: f32,
    /// A search box above the rows (Shortcuts page): its text.
    pub filter: Option<String>,
    /// Bottom-of-sidebar button.
    pub footer_button: Option<String>,
    /// A short note at the bottom of the page.
    pub note: String,
    /// Drawn as a dialog: a title bar with ✕ above the page.
    pub dialog: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsHit {
    Page(usize),
    Row(usize),
    /// The control itself (toggle, button, text field, keys).
    Control(usize),
    Prev(usize),
    Next(usize),
    /// A slider's track (the app maps the x position to a value).
    Slider(usize),
    Reset(usize),
    Filter,
    Footer,
    /// The dialog's ✕.
    Close,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsLayout {
    pub regions: Vec<(PxRect, SettingsHit)>,
    /// Total height of the rows, to clamp scrolling.
    pub content_height: f32,
    pub view_height: f32,
}

impl SettingsLayout {
    /// Topmost region under (x, y), with its rectangle.
    pub fn hit(&self, x: f32, y: f32) -> Option<(SettingsHit, PxRect)> {
        // controls are pushed after their rows: search backwards
        self.regions.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(r, h)| (*h, *r))
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.view_height).max(0.0)
    }
}

/// A rectangle with all four corners rounded.
pub fn round_rect(scene: &mut Scene, r: PxRect, radius: f32, color: u32, alpha: u8) {
    let radius = radius.min(r.h / 2.0).min(r.w / 2.0);
    let (y0, h) = (r.y.round() as i32, r.h.round() as i32);
    for row in 0..h {
        let from_top = row as f32;
        let from_bottom = (h - 1 - row) as f32;
        let inset = round_top_inset(radius, from_top.min(from_bottom));
        hspan(scene, r.x + inset, r.x + r.w - inset, y0 + row, color, alpha);
    }
}

fn round_outline(scene: &mut Scene, r: PxRect, radius: f32, color: u32, fill: u32) {
    round_rect(scene, r, radius, color, 255);
    round_rect(scene, PxRect { x: r.x + 1.0, y: r.y + 1.0, w: r.w - 2.0, h: r.h - 2.0 }, (radius - 1.0).max(0.0), fill, 255);
}

struct Ink {
    /// Section headings: the text, warmed with the accent.
    heading: u32,
    text: u32,
    dim: u32,
    card: u32,
    card_hover: u32,
    line: u32,
    accent: u32,
    on_accent: u32,
    warn: u32,
}

fn ink(theme: &Theme) -> Ink {
    let bg = theme.background;
    let (c0, _) = crate::glass::accents(theme);
    let text = theme.strong_text();
    Ink {
        heading: blend(text, c0, 150),
        text,
        dim: if crate::glass::tint(theme) == 0xffffff { blend(bg, 0xdfe6ee, 150) } else { blend(bg, theme.foreground, 150) },
        card: blend(bg, theme.foreground, 14),
        card_hover: blend(bg, theme.foreground, 24),
        line: blend(bg, theme.foreground, 60),
        accent: c0,
        on_accent: Theme::text_on(c0),
        warn: theme.palette[1],
    }
}

/// Draw the Settings page into `area` (below the tab bar).
#[allow(clippy::too_many_arguments)]
pub fn settings(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, area: PxRect, v: &SettingsView, scale: f32) -> SettingsLayout {
    let s = scale.max(0.5);
    let k = ink(theme);
    let mut regions = Vec::new();
    scene.clip(Some(area));
    let mut area = area;
    if v.dialog {
        // title bar: "Settings" and ✕
        let hh = (56.0 * s).round();
        let tint = crate::glass::tint(theme);
        // a little gradient gem before the title
        let gem = PxRect { x: area.x + 22.0 * s, y: area.y + hh / 2.0 - 7.0 * s, w: 14.0 * s, h: 14.0 * s };
        crate::glass::pill(scene, theme, gem, 5.0 * s, 255, 255);
        let end = ui_text(scene, fonts, (gem.x + gem.w + 12.0 * s).round() as i32, area.y + hh / 2.0, cw, ch, "Settings", k.text, true);
        ui_text(scene, fonts, end + (12.0 * s) as i32, area.y + hh / 2.0, cw, ch, "make it yours", k.dim, false);
        let cw_btn = (34.0 * s).round();
        let close = PxRect { x: area.x + area.w - cw_btn - 14.0 * s, y: area.y + (hh - cw_btn) / 2.0, w: cw_btn, h: cw_btn };
        let hot = v.hover == Some(SettingsHit::Close);
        crate::glass::fill(scene, close, cw_btn / 2.0, if hot { 0xe5484d } else { tint }, if hot { 230 } else { 14 });
        crate::icons::draw(scene, "close", close.x + close.w / 2.0, close.y + close.h / 2.0, 16.0 * s, if hot { 0xffffff } else { k.text });
        regions.push((close, SettingsHit::Close));
        scene.frect(PxRect { x: area.x + 16.0 * s, y: area.y + hh, w: area.w - 32.0 * s, h: 1.0 }, tint, 22);
        area = PxRect { y: area.y + hh + 1.0, h: area.h - hh - 1.0, ..area };
    }

    // sidebar
    let side_w = (220.0 * s).round().min(area.w * 0.35);
    let item_h = (40.0 * s).round();
    let tint = crate::glass::tint(theme);
    let mut y = area.y + 14.0 * s;
    for (i, name) in v.pages.iter().enumerate() {
        let r = PxRect { x: area.x + 12.0 * s, y, w: side_w - 22.0 * s, h: item_h };
        let selected = i == v.page;
        let hovered = v.hover == Some(SettingsHit::Page(i));
        if selected {
            crate::glass::pill(scene, theme, r, item_h / 2.0, 120, 40);
            crate::glass::ring(scene, r, item_h / 2.0, tint, 40);
        } else if hovered {
            crate::glass::fill(scene, r, item_h / 2.0, tint, 14);
        }
        // the selected page: white on the dark glass, near-black on light
        let on_pill = if theme.is_light() { 0x0d1b2a } else { 0xffffff };
        let fg = if selected {
            on_pill
        } else if hovered {
            k.text
        } else {
            k.dim
        };
        crate::icons::draw_in(
            scene,
            page_icon(name),
            r.x + 22.0 * s,
            r.y + r.h / 2.0,
            18.0 * s,
            fg,
            if selected { on_pill } else { crate::glass::accents(theme).0 },
        );
        ui_text(scene, fonts, (r.x + 42.0 * s).round() as i32, r.y + r.h / 2.0, cw, ch, name, fg, selected);
        regions.push((r, SettingsHit::Page(i)));
        y += item_h + 4.0 * s;
    }
    if let Some(label) = &v.footer_button {
        let r = PxRect { x: area.x + 14.0 * s, y: area.y + area.h - item_h - 14.0 * s, w: side_w - 26.0 * s, h: item_h };
        button(scene, fonts, cw, ch, &k, r, label, v.hover == Some(SettingsHit::Footer), s);
        let _ = theme;
        regions.push((r, SettingsHit::Footer));
    }
    scene.frect(PxRect { x: area.x + side_w, y: area.y + 14.0 * s, w: 1.0, h: area.h - 28.0 * s }, tint, 20);

    // page
    let px = area.x + side_w + 28.0 * s;
    let pw = (area.x + area.w - px - 28.0 * s).min(900.0 * s).max(100.0);
    let page_clip = PxRect { x: area.x + side_w + 1.0, y: area.y, w: area.w - side_w - 1.0, h: area.h };
    scene.clip(Some(page_clip));
    let mut y = area.y + 18.0 * s - v.scroll;
    let start_y = y;
    ui_text(scene, fonts, px.round() as i32, y + 14.0 * s, cw, ch, &v.title, k.text, true);
    y += 40.0 * s;

    if let Some(f) = &v.filter {
        let r = PxRect { x: px, y, w: pw.min(420.0 * s), h: (34.0 * s).round() };
        round_outline(scene, r, 5.0 * s, if v.focus.is_none() { k.accent } else { k.line }, k.card);
        let (text, fg) = if f.is_empty() { ("Search shortcuts", k.dim) } else { (f.as_str(), k.text) };
        let end = ui_text(scene, fonts, (r.x + 12.0 * s).round() as i32, r.y + r.h / 2.0, cw, ch, text, fg, false);
        if v.focus.is_none() {
            let cx = if f.is_empty() { (r.x + 12.0 * s).round() } else { end as f32 + 1.0 };
            scene.frect(PxRect { x: cx, y: r.y + 8.0 * s, w: 1.0, h: r.h - 16.0 * s }, k.text, 255);
        }
        regions.push((r, SettingsHit::Filter));
        y += r.h + 14.0 * s;
    }

    let row_h = (62.0 * s).round();
    let pad = 16.0 * s;
    for (i, row) in v.rows.iter().enumerate() {
        if row.control == Control::Header {
            y += 12.0 * s;
            let end = ui_text(scene, fonts, px.round() as i32, y + 12.0 * s, cw, ch, &row.label, k.heading, true);
            // a soft line running off to the right
            let lx = end as f32 + 12.0 * s;
            if lx < px + pw {
                crate::glass::gradient(scene, PxRect { x: lx, y: (y + 12.0 * s).round(), w: px + pw - lx, h: 1.0 }, 0.0, tint, tint, 30, 0);
            }
            y += 32.0 * s;
            continue;
        }
        let r = PxRect { x: px, y, w: pw, h: row_h };
        let visible = r.y + r.h > area.y && r.y < area.y + area.h;
        if visible {
            let hovered = matches!(v.hover, Some(h) if hit_row(h) == Some(i));
            crate::glass::card(scene, theme, r, 12.0 * s, hovered);
            if v.focus == Some(i) {
                // focus ring
                let ring = PxRect { x: r.x - 2.0, y: r.y - 2.0, w: r.w + 4.0, h: r.h + 4.0 };
                crate::glass::ring(scene, ring, 14.0 * s, k.accent, 220);
            }
            regions.push((r, SettingsHit::Row(i)));
            let (label_y, detail_y) =
                if row.detail.is_empty() && row.warning.is_none() { (r.y + r.h / 2.0, 0.0) } else { (r.y + r.h * 0.36, r.y + r.h * 0.68) };

            // control area from the right
            let right = r.x + r.w - pad;
            let control_left = draw_control(scene, fonts, theme, cw, ch, &k, &mut regions, i, &row.control, right, r, v.hover, s);
            let mut left_limit = control_left - 12.0 * s;
            if !row.swatches.is_empty() {
                let sw = (14.0 * s).round();
                let total = row.swatches.len() as f32 * (sw + 3.0 * s);
                let mut sx = left_limit - total;
                for c in &row.swatches {
                    let dot = PxRect { x: sx.round(), y: (r.y + (r.h - sw) / 2.0).round(), w: sw, h: sw };
                    crate::glass::fill(scene, dot, sw / 2.0, *c, 255);
                    crate::glass::ring(scene, dot, sw / 2.0, tint, 60);
                    sx += sw + 3.0 * s;
                }
                left_limit -= total + 10.0 * s;
            }
            if row.changed {
                let t = "Reset";
                let w = ui_width(fonts, cw, t, false) + 12.0 * s;
                let rr = PxRect { x: left_limit - w, y: r.y + (r.h - 26.0 * s) / 2.0, w, h: 26.0 * s };
                if v.hover == Some(SettingsHit::Reset(i)) {
                    crate::glass::fill(scene, rr, rr.h / 2.0, tint, 20);
                }
                ui_text(scene, fonts, (rr.x + 6.0 * s).round() as i32, rr.y + rr.h / 2.0, cw, ch, t, k.accent, false);
                regions.push((rr, SettingsHit::Reset(i)));
                left_limit = rr.x - 8.0 * s;
            }
            let label_max = (left_limit - (r.x + pad)).max(20.0);
            let label = fit(fonts, cw, &row.label, false, label_max);
            ui_text(scene, fonts, (r.x + pad).round() as i32, label_y, cw, ch, &label, k.text, false);
            if let Some(w) = &row.warning {
                let t = fit(fonts, cw, w, false, label_max);
                ui_text(scene, fonts, (r.x + pad).round() as i32, detail_y, cw, ch, &t, k.warn, false);
            } else if !row.detail.is_empty() {
                let t = fit(fonts, cw, &row.detail, false, label_max);
                ui_text(scene, fonts, (r.x + pad).round() as i32, detail_y, cw, ch, &t, k.dim, false);
            }
        }
        y += row_h + 8.0 * s;
    }
    if !v.note.is_empty() {
        y += 8.0 * s;
        let note = fit(fonts, cw, &v.note, false, pw);
        ui_text(scene, fonts, px.round() as i32, y + 10.0 * s, cw, ch, &note, k.dim, false);
        y += 30.0 * s;
    }
    scene.clip(None);
    SettingsLayout { regions, content_height: y - start_y + 24.0 * s, view_height: area.h }
}

fn hit_row(h: SettingsHit) -> Option<usize> {
    match h {
        SettingsHit::Row(i) | SettingsHit::Control(i) | SettingsHit::Prev(i) | SettingsHit::Next(i) | SettingsHit::Slider(i) | SettingsHit::Reset(i) => Some(i),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn button(scene: &mut Scene, fonts: &mut FontSet, cw: usize, ch: usize, k: &Ink, r: PxRect, label: &str, hovered: bool, s: f32) {
    let t = if k.text & 0x80_0000 != 0 { 0xffffff } else { 0x000000 };
    crate::glass::fill(scene, r, r.h / 2.0, t, if hovered { 30 } else { 16 });
    crate::glass::ring(scene, r, r.h / 2.0, t, if hovered { 60 } else { 34 });
    let _ = s;
    let w = ui_width(fonts, cw, label, false);
    ui_text(scene, fonts, (r.x + (r.w - w) / 2.0).round() as i32, r.y + r.h / 2.0, cw, ch, label, k.text, false);
}

/// Draw `c` right-aligned at `right` inside row `r`; returns its left edge.
#[allow(clippy::too_many_arguments)]
fn draw_control(
    scene: &mut Scene,
    fonts: &mut FontSet,
    theme: &Theme,
    cw: usize,
    ch: usize,
    k: &Ink,
    regions: &mut Vec<(PxRect, SettingsHit)>,
    i: usize,
    c: &Control,
    right: f32,
    r: PxRect,
    hover: Option<SettingsHit>,
    s: f32,
) -> f32 {
    let mid = r.y + r.h / 2.0;
    match c {
        Control::Header => right,
        Control::Info(t) => {
            // a path keeps its start and its file name: C:\Users\…\vibex.lua
            let t = if t.contains(['\\', '/']) { crate::scene::fit_middle(fonts, cw, t, false, r.w * 0.55) } else { fit(fonts, cw, t, false, r.w * 0.55) };
            let w = ui_width(fonts, cw, &t, false);
            ui_text(scene, fonts, (right - w).round() as i32, mid, cw, ch, &t, k.dim, false);
            right - w
        }
        Control::Toggle(on) => {
            let (w, h) = ((44.0 * s).round(), (24.0 * s).round());
            let t = PxRect { x: right - w, y: (mid - h / 2.0).round(), w, h };
            let tint = crate::glass::tint(theme);
            if *on {
                crate::glass::pill(scene, theme, t, h / 2.0, 255, 255);
                let d = h - 6.0 * s;
                let knob = PxRect { x: t.x + w - d - 3.0 * s, y: t.y + 3.0 * s, w: d, h: d };
                crate::glass::fill(scene, PxRect { y: knob.y + 1.0 * s, ..knob }, d / 2.0, 0x000000, 50);
                crate::glass::fill(scene, knob, d / 2.0, 0xffffff, 255);
            } else {
                crate::glass::fill(scene, t, h / 2.0, tint, 18);
                crate::glass::ring(scene, t, h / 2.0, tint, 70);
                let d = h - 8.0 * s;
                crate::glass::fill(scene, PxRect { x: t.x + 4.0 * s, y: t.y + 4.0 * s, w: d, h: d }, d / 2.0, k.dim, 255);
            }
            let word = if *on { "On" } else { "Off" };
            let ww = ui_width(fonts, cw, word, false);
            ui_text(scene, fonts, (t.x - 10.0 * s - ww).round() as i32, mid, cw, ch, word, k.dim, false);
            regions.push((PxRect { x: t.x - 16.0 * s - ww, y: r.y, w: w + ww + 16.0 * s, h: r.h }, SettingsHit::Control(i)));
            t.x - 10.0 * s - ww
        }
        Control::Choice(value) => {
            // a dropdown: the value, and a chevron; the whole box opens it
            let w = (230.0 * s).round();
            let h = (32.0 * s).round();
            let b = PxRect { x: right - w, y: (mid - h / 2.0).round(), w, h };
            let hovered = hover == Some(SettingsHit::Control(i));
            crate::glass::card(scene, theme, b, 10.0 * s, hovered);
            crate::icons::draw(scene, "chevron-down", b.x + w - 18.0 * s, mid, 16.0 * s, k.text);
            regions.push((b, SettingsHit::Control(i)));
            let t = fit(fonts, cw, value, false, w - 44.0 * s);
            ui_text(scene, fonts, (b.x + 12.0 * s).round() as i32, mid, cw, ch, &t, k.text, false);
            b.x
        }
        Control::Slider { value, text } => {
            let track_w = (180.0 * s).round();
            let tw = ui_width(fonts, cw, text, false).max(36.0 * s);
            let tx = right - tw;
            ui_text(scene, fonts, tx.round() as i32, mid, cw, ch, text, k.text, false);
            let x0 = tx - 14.0 * s - track_w;
            let th = (6.0 * s).round().max(3.0);
            let track = PxRect { x: x0, y: (mid - th / 2.0).round(), w: track_w, h: th };
            crate::glass::fill(scene, track, th / 2.0, crate::glass::tint(theme), 30);
            let v = value.clamp(0.0, 1.0);
            if v > 0.0 {
                crate::glass::pill(scene, theme, PxRect { w: (track_w * v).max(th), ..track }, th / 2.0, 255, 255);
            }
            let knob = (18.0 * s).round();
            let kx = x0 + track_w * v - knob / 2.0;
            let kr = PxRect { x: kx, y: (mid - knob / 2.0).round(), w: knob, h: knob };
            crate::glass::fill(scene, PxRect { y: kr.y + 1.5 * s, ..kr }, knob / 2.0, 0x000000, 60);
            crate::glass::fill(scene, kr, knob / 2.0, 0xffffff, 255);
            crate::glass::fill(
                scene,
                PxRect { x: kx + knob * 0.32, y: kr.y + knob * 0.32, w: knob * 0.36, h: knob * 0.36 },
                knob * 0.18,
                crate::glass::accents(theme).1,
                255,
            );
            regions.push((PxRect { x: x0 - knob / 2.0, y: r.y, w: track_w + knob, h: r.h }, SettingsHit::Slider(i)));
            x0 - knob / 2.0
        }
        Control::Text { value, editing, placeholder } => {
            let w = (300.0 * s).round().min(r.w * 0.5);
            let h = (32.0 * s).round();
            let b = PxRect { x: right - w, y: (mid - h / 2.0).round(), w, h };
            crate::glass::card(scene, theme, b, 10.0 * s, *editing);
            if *editing {
                crate::glass::ring(scene, b, 10.0 * s, k.accent, 230);
            }
            let (t, fg) = if value.is_empty() { (placeholder.as_str(), k.dim) } else { (value.as_str(), k.text) };
            // show the end of long text while editing
            let max = w - 20.0 * s;
            let shown = if *editing && ui_width(fonts, cw, t, false) > max {
                let chars: Vec<char> = t.chars().collect();
                let mut n = 0;
                while n < chars.len() && ui_width(fonts, cw, &chars[n..].iter().collect::<String>(), false) > max {
                    n += 1;
                }
                chars[n..].iter().collect()
            } else {
                fit(fonts, cw, t, false, max)
            };
            let end = ui_text(scene, fonts, (b.x + 10.0 * s).round() as i32, mid, cw, ch, &shown, fg, false);
            if *editing {
                let cx = if value.is_empty() { b.x + 10.0 * s } else { end as f32 + 1.0 };
                scene.frect(PxRect { x: cx.round(), y: b.y + 7.0 * s, w: 1.0, h: h - 14.0 * s }, k.text, 255);
            }
            regions.push((b, SettingsHit::Control(i)));
            b.x
        }
        Control::Keys { chips, capturing } => {
            let h = (26.0 * s).round();
            let gap = 4.0 * s;
            let mut x = right;
            if *capturing {
                let t = "Press keys…  Esc cancels · Backspace removes";
                let tw = ui_width(fonts, cw, t, false);
                let b = PxRect { x: right - tw - 20.0 * s, y: (mid - h / 2.0).round(), w: tw + 20.0 * s, h };
                round_outline(scene, b, 4.0 * s, k.accent, k.card);
                ui_text(scene, fonts, (b.x + 10.0 * s).round() as i32, mid, cw, ch, t, k.accent, false);
                regions.push((b, SettingsHit::Control(i)));
                return b.x;
            }
            if chips.is_empty() {
                let t = "Not set";
                let tw = ui_width(fonts, cw, t, false);
                ui_text(scene, fonts, (right - tw).round() as i32, mid, cw, ch, t, k.dim, false);
                regions.push((PxRect { x: right - tw - 8.0, y: r.y, w: tw + 8.0, h: r.h }, SettingsHit::Control(i)));
                return right - tw;
            }
            // several bindings: draw right to left, "or" between
            for (bi, combo) in chips.iter().enumerate().rev() {
                for part in combo.iter().rev() {
                    let tw = ui_width(fonts, cw, part, false).max(10.0 * s);
                    let b = PxRect { x: x - tw - 16.0 * s, y: (mid - h / 2.0).round(), w: tw + 16.0 * s, h };
                    round_outline(scene, b, 4.0 * s, k.line, blend(theme.background, theme.foreground, 30));
                    let pw = ui_width(fonts, cw, part, false);
                    ui_text(scene, fonts, (b.x + (b.w - pw) / 2.0).round() as i32, mid, cw, ch, part, k.text, false);
                    x = b.x - gap;
                }
                if bi > 0 {
                    let t = "or";
                    let tw = ui_width(fonts, cw, t, false);
                    ui_text(scene, fonts, (x - tw - 2.0 * s).round() as i32, mid, cw, ch, t, k.dim, false);
                    x -= tw + 8.0 * s;
                }
            }
            regions.push((PxRect { x, y: r.y, w: right - x, h: r.h }, SettingsHit::Control(i)));
            x
        }
        Control::Button(label) => {
            let w = ui_width(fonts, cw, label, false) + 28.0 * s;
            let h = (32.0 * s).round();
            let b = PxRect { x: right - w, y: (mid - h / 2.0).round(), w, h };
            button(scene, fonts, cw, ch, k, b, label, hover == Some(SettingsHit::Control(i)), s);
            regions.push((b, SettingsHit::Control(i)));
            b.x
        }
    }
}

// ------------------------------------------------------------------ confirm

/// A question in the middle of the window: title, detail lines (shown in
/// the terminal font, e.g. the command), and two buttons.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfirmView {
    pub title: String,
    pub lines: Vec<String>,
    pub yes: String,
    pub no: String,
    /// The yes button is the dangerous kind (red).
    pub danger: bool,
    /// Which button has the keyboard (true = yes).
    pub yes_focused: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConfirmLayout {
    pub frame: PxRect,
    pub yes: PxRect,
    pub no: PxRect,
}

#[allow(clippy::too_many_arguments)]
pub fn confirm(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, v: &ConfirmView, scale: f32, area: PxRect) -> ConfirmLayout {
    let s = scale.max(0.5);
    let k = ink(theme);
    let pad = (22.0 * s).round();
    let line_h = ch as f32 + 2.0;
    let shown: Vec<String> = v.lines.iter().take(8).map(|l| l.chars().take(90).collect()).collect();
    let text_w = shown.iter().map(|l| crate::text_width(l) * cw).max().unwrap_or(0) as f32;
    let title_w = ui_width(fonts, cw, &v.title, true);
    let w = (text_w.max(title_w) + pad * 2.0).clamp(360.0 * s, area.w - 32.0).round();
    let btn_h = (32.0 * s).round();
    let h = (pad + 26.0 * s + if shown.is_empty() { 0.0 } else { shown.len() as f32 * line_h + 14.0 * s } + 16.0 * s + btn_h + pad).round();
    let x = (area.x + (area.w - w) / 2.0).round();
    let y = (area.y + (area.h - h) / 2.5).max(area.y + 8.0).round();
    let frame = PxRect { x, y, w, h };
    scene.frect(area, 0x000000, 90);
    round_rect(scene, PxRect { x: x + 2.0, y: y + 5.0 * s, w, h }, 10.0 * s, 0x000000, 110);
    round_rect(scene, PxRect { x: x - 1.0, y: y - 1.0, w: w + 2.0, h: h + 2.0 }, 10.0 * s, k.line, 255);
    round_rect(scene, frame, 9.0 * s, theme.background, 255);
    let mut cy = y + pad + 10.0 * s;
    ui_text(scene, fonts, (x + pad) as i32, cy, cw, ch, &v.title, k.text, true);
    cy += 22.0 * s;
    if !shown.is_empty() {
        let box_r = PxRect { x: x + pad - 8.0 * s, y: cy - 2.0, w: w - 2.0 * pad + 16.0 * s, h: shown.len() as f32 * line_h + 10.0 * s };
        round_rect(scene, box_r, 6.0 * s, k.card, 255);
        let mut ly = cy + 4.0 * s;
        for l in &shown {
            let base = ly as i32 + fonts.baseline;
            crate::scene::text_run(scene, fonts, (x + pad) as i32, base, cw, l, theme.foreground, crate::font::Style::Regular);
            ly += line_h;
        }
        cy += shown.len() as f32 * line_h + 14.0 * s;
    }
    let by = (cy + 12.0 * s).round();
    let bw = |label: &str, fonts: &mut FontSet| (ui_width(fonts, cw, label, false) + 36.0 * s).max(96.0 * s).round();
    let (yw, nw) = (bw(&v.yes, fonts), bw(&v.no, fonts));
    let yes = PxRect { x: x + w - pad - yw, y: by, w: yw, h: btn_h };
    let no = PxRect { x: yes.x - 10.0 * s - nw, y: by, w: nw, h: btn_h };
    let yes_bg = if v.danger { 0xc42b1c } else { k.accent };
    round_rect(scene, yes, 6.0 * s, yes_bg, 255);
    round_rect(scene, no, 6.0 * s, k.card_hover, 255);
    let focus = if v.yes_focused { yes } else { no };
    let ring = PxRect { x: focus.x - 2.0, y: focus.y - 2.0, w: focus.w + 4.0, h: focus.h + 4.0 };
    for r in [
        PxRect { h: 1.0, ..ring },
        PxRect { y: ring.y + ring.h - 1.0, h: 1.0, ..ring },
        PxRect { w: 1.0, ..ring },
        PxRect { x: ring.x + ring.w - 1.0, w: 1.0, ..ring },
    ] {
        scene.frect(r, k.text, 200);
    }
    let center = |fonts: &mut FontSet, scene: &mut Scene, r: PxRect, label: &str, fg: u32| {
        let tw = ui_width(fonts, cw, label, false);
        ui_text(scene, fonts, (r.x + (r.w - tw) / 2.0).round() as i32, r.y + r.h / 2.0, cw, ch, label, fg, false);
    };
    center(fonts, scene, yes, &v.yes, if v.danger { 0xffffff } else { k.on_accent });
    center(fonts, scene, no, &v.no, k.text);
    ConfirmLayout { frame, yes, no }
}

// ------------------------------------------------------------------ menus

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub hint: String,
    /// Draw a separator above this item.
    pub separator: bool,
    /// Draw the settings gear instead of the terminal icon (None = no icon).
    pub icon: Option<MenuIcon>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuIcon {
    Terminal,
    Settings,
    /// A VibeX icon by name (icons.rs).
    Named(&'static str),
    /// A theme's colours: its background with three of its colours on it.
    Swatch([u32; 4]),
}

/// The icon for a Settings page.
fn page_icon(page: &str) -> &'static str {
    match page {
        "Appearance" => "appearance",
        "Terminal" => "terminal",
        "Security" => "security",
        "Profiles" => "profile",
        "Shortcuts" => "keyboard",
        "Updates" => "download",
        _ => "info",
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuView {
    pub items: Vec<MenuItem>,
    /// Top-left corner.
    pub x: f32,
    pub y: f32,
    /// Highlighted item (hover or keyboard).
    pub selected: Option<usize>,
    /// First item shown when they don't all fit.
    pub first: usize,
    /// At least this wide (a dropdown matches its box).
    pub min_width: f32,
}

#[derive(Clone, Debug, Default)]
pub struct MenuLayout {
    pub frame: PxRect,
    pub items: Vec<PxRect>,
    /// The first item actually shown (kept so the selection stays visible).
    pub first: usize,
}

impl MenuLayout {
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.items.iter().position(|r| r.contains(x, y))
    }
}

/// A gear icon centred on (cx, cy).
pub(crate) fn gear_icon(scene: &mut Scene, cx: f32, cy: f32, s: f32, c: u32) {
    let r_out = 7.0 * s;
    let r_in = 4.5 * s;
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::PI / 4.0;
        let (dx, dy) = (a.cos(), a.sin());
        line(scene, cx + dx * r_in, cy + dy * r_in, cx + dx * r_out, cy + dy * r_out, c);
    }
    let steps = 28;
    for i in 0..steps {
        let a0 = i as f32 / steps as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / steps as f32 * std::f32::consts::TAU;
        line(scene, cx + a0.cos() * r_in, cy + a0.sin() * r_in, cx + a1.cos() * r_in, cy + a1.sin() * r_in, c);
        let rr = 1.8 * s;
        line(scene, cx + a0.cos() * rr, cy + a0.sin() * rr, cx + a1.cos() * rr, cy + a1.sin() * rr, c);
    }
}

/// A dropdown menu (the ⌄ next to the + button).
#[allow(clippy::too_many_arguments)]
pub fn menu(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, m: &MenuView, scale: f32) -> MenuLayout {
    let s = scale.max(0.5);
    let k = ink(theme);
    let item_h = (34.0 * s).round();
    let sep_h = (9.0 * s).round();
    let pad = 6.0 * s;
    let label_w = m.items.iter().map(|i| ui_width(fonts, cw, &i.label, false)).fold(0.0, f32::max);
    let hint_w = m.items.iter().map(|i| ui_width(fonts, cw, &i.hint, false)).fold(0.0, f32::max);
    let w = (44.0 * s + label_w + 32.0 * s + hint_w + 16.0 * s).max(220.0 * s).max(m.min_width).min(scene.width as f32 - 8.0).round();
    // only as many rows as fit; the window follows the selection
    let fit_rows = (((scene.height as f32 - 16.0) - pad * 2.0) / (item_h + 1.0)).floor().max(3.0) as usize;
    // long lists (fonts) show 12 at a time
    let rows = m.items.len().min(fit_rows).min(if m.items.len() > 16 { 12 } else { usize::MAX });
    let mut first = m.first.min(m.items.len().saturating_sub(rows));
    if let Some(sel) = m.selected {
        if sel < first {
            first = sel;
        } else if sel >= first + rows {
            first = sel + 1 - rows;
        }
    }
    let shown = first..(first + rows).min(m.items.len());
    let h = pad * 2.0 + m.items[shown.clone()].iter().enumerate().map(|(n, i)| item_h + if i.separator && n > 0 { sep_h } else { 0.0 }).sum::<f32>();
    let x = m.x.min(scene.width as f32 - w - 4.0).max(4.0).round();
    // below the pointer when it fits, else above it, else pinned in
    let y = if m.y + h > scene.height as f32 - 4.0 { (m.y - h).max(4.0) } else { m.y }.round();
    let frame = PxRect { x, y, w, h };
    // warm, milky, opaque (menus sit on sharp text, so no see-through)
    let tint = crate::glass::tint(theme);
    let bg = if tint == 0xffffff { blend(theme.background, 0x05070b, 90) } else { blend(theme.background, theme.foreground, 18) };
    let radius = 12.0 * s;
    // soft shadow, body, a light edge, a hint of peach from the top-left
    for (g, a) in [(10.0, 14u8), (4.0, 34)] {
        let g = g * s;
        crate::glass::fill(scene, PxRect { x: x - g, y: y - g + 6.0 * s, w: w + 2.0 * g, h: h + 2.0 * g }, radius + g, 0x000000, a);
    }
    crate::glass::fill(scene, frame, radius, bg, 255);
    let (a0, a1) = crate::glass::accents(theme);
    crate::glass::gradient(scene, frame, radius, a1, a0, 18, 0);
    crate::glass::ring(scene, frame, radius, tint, 46);
    let mut items = vec![PxRect::default(); m.items.len()];
    let mut iy = y + pad;
    for (i, it) in m.items.iter().enumerate() {
        if !shown.contains(&i) {
            continue;
        }
        if it.separator && i != first {
            scene.frect(PxRect { x: x + 10.0 * s, y: (iy + sep_h / 2.0).round(), w: w - 20.0 * s, h: 1.0 }, k.line, 255);
            iy += sep_h;
        }
        let r = PxRect { x: x + pad, y: iy, w: w - pad * 2.0, h: item_h };
        if m.selected == Some(i) {
            crate::glass::pill(scene, theme, r, 8.0 * s, 70, 25);
        }
        let mid = r.y + r.h / 2.0;
        match it.icon {
            Some(MenuIcon::Terminal) => crate::icons::draw(scene, "terminal", r.x + 19.0 * s, mid, 17.0 * s, k.text),
            Some(MenuIcon::Settings) => crate::icons::draw(scene, "sliders", r.x + 19.0 * s, mid, 17.0 * s, k.text),
            Some(MenuIcon::Named(n)) => crate::icons::draw(scene, n, r.x + 19.0 * s, mid, 17.0 * s, k.dim),
            Some(MenuIcon::Swatch([bg, a, b, c])) => {
                let (w, h) = ((24.0 * s).round(), (16.0 * s).round());
                let chip = PxRect { x: (r.x + 7.0 * s).round(), y: (mid - h / 2.0).round(), w, h };
                crate::glass::fill(scene, chip, 4.0 * s, bg, 255);
                crate::glass::ring(scene, chip, 4.0 * s, k.line, 255);
                let d = (5.0 * s).round();
                for (n, col) in [a, b, c].into_iter().enumerate() {
                    let dot = PxRect { x: (chip.x + 4.0 * s + n as f32 * (d + 1.5 * s)).round(), y: (mid - d / 2.0).round(), w: d, h: d };
                    crate::glass::fill(scene, dot, d / 2.0, col, 255);
                }
            }
            None => {}
        }
        ui_text(scene, fonts, (r.x + 38.0 * s).round() as i32, mid, cw, ch, &it.label, k.text, false);
        if it.hint == "\u{2713}" {
            // the current choice in a dropdown
            crate::icons::draw(scene, "check", r.x + r.w - 18.0 * s, mid, 16.0 * s, k.accent);
        } else if !it.hint.is_empty() {
            let hw = ui_width(fonts, cw, &it.hint, false);
            ui_text(scene, fonts, (r.x + r.w - 10.0 * s - hw).round() as i32, mid, cw, ch, &it.hint, k.dim, false);
        }
        items[i] = r;
        iy += item_h;
    }
    // more above / below
    if first > 0 {
        scene.frect(PxRect { x: x + w / 2.0 - 10.0 * s, y: y + 2.0, w: 20.0 * s, h: 2.0 }, k.dim, 255);
    }
    if shown.end < m.items.len() {
        scene.frect(PxRect { x: x + w / 2.0 - 10.0 * s, y: y + h - 4.0, w: 20.0 * s, h: 2.0 }, k.dim, 255);
    }
    let _ = outline;
    MenuLayout { frame, items, first }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_hits() {
        let l = SettingsLayout {
            regions: vec![
                (PxRect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 }, SettingsHit::Row(0)),
                (PxRect { x: 60.0, y: 10.0, w: 30.0, h: 30.0 }, SettingsHit::Control(0)),
            ],
            content_height: 500.0,
            view_height: 300.0,
        };
        assert_eq!(l.hit(70.0, 20.0).unwrap().0, SettingsHit::Control(0), "controls win over their row");
        assert_eq!(l.hit(10.0, 20.0).unwrap().0, SettingsHit::Row(0));
        assert!(l.hit(10.0, 60.0).is_none());
        assert_eq!(l.max_scroll(), 200.0);
    }
}
