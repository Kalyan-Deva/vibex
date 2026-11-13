//! Rendering for Vibex.
//!
//! `scene` turns the terminal grid into an ordered list of rects and glyphs;
//! `gpu` (wgpu, glyph atlas, one instanced draw) or `cpu` (framebuffer
//! fallback) draws it.

mod colr;
pub mod cpu;
mod font;
pub mod glass;
pub mod gpu;
pub mod icons;
mod raster;
mod scene;
pub mod settings_view;
pub mod tabbar;
mod theme;
mod themes_data;

pub use cpu::Frame;
pub use font::{is_emoji, FontPaths, FontSet, GlyphKey, Style};
pub use gpu::GpuRenderer;
pub use scene::{Clip, Item, PaneLayout, Scene};
pub use settings_view::{ConfirmLayout, ConfirmView, Control, MenuIcon, MenuItem, MenuLayout, MenuView, SettingRow, SettingsHit, SettingsLayout, SettingsView};
pub use tabbar::{PxRect, TabBar, TabBarLayout, TabHit, TabIcon, TabInfo};
pub use theme::{blend, Theme};

/// Write a 0RGB pixel buffer as a 24-bit BMP (for headless previews).
pub fn write_bmp(path: &std::path::Path, px: &[u32], w: usize, h: usize) -> std::io::Result<()> {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut b = Vec::with_capacity(size);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(size as u32).to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        for x in 0..w {
            let p = px[y * w + x];
            b.extend_from_slice(&[(p & 0xff) as u8, ((p >> 8) & 0xff) as u8, ((p >> 16) & 0xff) as u8]);
        }
        b.resize(b.len() + row - w * 3, 0);
    }
    std::fs::write(path, b)
}

/// Display width of a string in cells.
pub fn text_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    s.width()
}

use vibex_term::Terminal;

/// Selection in grid coordinates (inclusive), already in visible-row space.
#[derive(Clone, Copy, Debug)]
pub struct VisibleSelection {
    pub start: (usize, usize),
    pub end: (usize, usize),
    /// Rectangular: columns start.0..=end.0 on every row (either order).
    pub block: bool,
}

impl VisibleSelection {
    pub fn contains(&self, x: usize, y: usize) -> bool {
        let (s, e) = (self.start, self.end);
        if self.block {
            let (c0, c1) = (s.0.min(e.0), s.0.max(e.0));
            return y >= s.1 && y <= e.1 && x >= c0 && x <= c1;
        }
        (y > s.1 || (y == s.1 && x >= s.0)) && (y < e.1 || (y == e.1 && x <= e.0))
    }
}

/// A coloured range (search hits).
#[derive(Clone, Copy, Debug)]
pub struct Highlight {
    pub range: VisibleSelection,
    pub bg: u32,
    pub fg: Option<u32>,
}

#[derive(Default)]
pub struct DrawState {
    pub focused: bool,
    /// Cursor is in the "on" phase of blinking.
    pub cursor_visible: bool,
    pub selection: Option<VisibleSelection>,
    /// Later entries win.
    pub highlights: Vec<Highlight>,
    /// Hovered hyperlink (drawn underlined).
    pub link: Option<VisibleSelection>,
    /// Text boxes stacked in the top-right corner (search bar, toasts).
    pub overlays: Vec<Overlay>,
    /// Smooth-scroll offset in pixels (positive = content drawn lower).
    pub y_shift: f32,
    /// Status bar content (single-pane `build_scene` only; multi-pane
    /// callers pass it in `Chrome`).
    pub status: Option<StatusLine>,
    /// Fade this pane (an inactive split).
    pub dim: bool,
    /// Draw shell-integration prompt marks in the left gutter.
    pub marks: bool,
    /// Quick-select badges: (column, visible row, label).
    pub labels: Vec<(usize, usize, String)>,
    /// IME composition text, drawn inline at the cursor.
    pub preedit: Option<String>,
    /// A suggestion drawn dim after the cursor (fish-style).
    pub ghost: Option<String>,
    /// Copy mode's cursor (column, visible row); replaces the terminal's.
    pub copy_cursor: Option<(usize, usize)>,
    /// The command whose output fills the view, pinned to the pane's top
    /// (its prompt has scrolled out of sight).
    pub sticky: Option<StickyHeader>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StickyHeader {
    pub command: String,
    /// "12.3s · exit 1" etc.
    pub detail: String,
    /// Mark colour (green / red / grey).
    pub color: u32,
}

/// The command palette as it should be drawn.
#[derive(Clone, Debug, Default)]
pub struct PaletteView {
    pub query: String,
    /// (label, hint such as the key binding)
    pub items: Vec<(String, String)>,
    pub selected: usize,
}

/// Window-level drawing: everything that isn't a pane.
#[derive(Default)]
pub struct Chrome {
    pub tabbar: Option<(TabBar, TabBarLayout)>,
    pub dividers: Vec<PxRect>,
    pub status: Option<StatusLine>,
    pub overlays: Vec<Overlay>,
    pub palette: Option<PaletteView>,
    /// The Settings dialog: the page, the dialog's rectangle, and the area
    /// dimmed behind it.
    pub settings: Option<(SettingsView, PxRect, PxRect)>,
    /// An open dropdown menu.
    pub menu: Option<MenuView>,
    /// A yes / no question over everything.
    pub confirm: Option<settings_view::ConfirmView>,
}

/// Hit regions from drawing the chrome.
#[derive(Clone, Debug, Default)]
pub struct ChromeLayout {
    pub settings: Option<SettingsLayout>,
    pub menu: Option<MenuLayout>,
    pub confirm: Option<settings_view::ConfirmLayout>,
}

#[derive(Clone, Debug)]
pub struct Overlay {
    pub text: String,
    pub border: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusSegment {
    pub text: String,
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub bold: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusLine {
    pub left: Vec<StatusSegment>,
    pub right: Vec<StatusSegment>,
}

/// Fonts, theme and layout shared by both backends.
pub struct Renderer {
    pub fonts: FontSet,
    pub theme: Theme,
    pub padding_x: usize,
    pub padding_y: usize,
    /// Reserve a status bar at the bottom.
    pub status_bar: bool,
    /// Display scale (1.0 = 100%) for the title bar's geometry.
    pub ui_scale: f32,
    /// Glass look: see-through title and status bars, a faint frosted
    /// sheen and a light edge.
    pub glass: bool,
    /// The classic title bar (title + menu bar), and whether it shows a
    /// tabs row (more than one tab).
    pub classic_bar: bool,
    pub classic_tabs: bool,
}

impl Renderer {
    pub fn new(fonts: FontSet, theme: Theme) -> Self {
        Self { fonts, theme, padding_x: 8, padding_y: 6, status_bar: false, ui_scale: 1.0, glass: false, classic_bar: false, classic_tabs: false }
    }

    /// Height of the status bar in pixels (0 when off).
    pub fn status_height(&self) -> usize {
        if self.status_bar {
            self.fonts.cell_height + 6
        } else {
            0
        }
    }

    pub fn tabbar_height(&self) -> f32 {
        if self.classic_bar {
            tabbar::classic_height(self.fonts.cell_height, self.ui_scale, self.classic_tabs)
        } else {
            tabbar::height(self.fonts.cell_height, self.ui_scale)
        }
    }

    /// The area for panes: below the tab bar (if any), above the status bar.
    pub fn content_rect(&self, width: usize, height: usize, tabbar: bool) -> PxRect {
        let top = if tabbar { self.tabbar_height() } else { 0.0 };
        let bottom = self.status_height() as f32;
        PxRect { x: 0.0, y: top, w: width as f32, h: (height as f32 - top - bottom).max(1.0) }
    }

    /// Grid that fits in a pane rectangle (padding inside).
    pub fn pane_grid(&self, r: PxRect) -> (usize, usize) {
        let w = (r.w as usize).saturating_sub(self.padding_x * 2);
        let h = (r.h as usize).saturating_sub(self.padding_y * 2);
        ((w / self.fonts.cell_width).max(2), (h / self.fonts.cell_height).max(1))
    }

    /// Grid size for a single pane filling a window (no tab bar).
    pub fn grid_size(&self, width: usize, height: usize) -> (usize, usize) {
        self.pane_grid(self.content_rect(width, height, false))
    }

    pub fn tabbar_layout(&self, bar: &TabBar, width: usize) -> TabBarLayout {
        tabbar::layout(bar, width as f32, self.fonts.cell_width, self.fonts.cell_height, self.ui_scale)
    }

    pub fn begin(&self, width: usize, height: usize) -> Scene {
        let mut scene = Scene::new(width, height, self.theme.background);
        if self.glass {
            scene::glass_sheen(&mut scene);
        }
        scene
    }

    pub fn draw_pane(&mut self, scene: &mut Scene, term: &Terminal, state: &DrawState, rect: PxRect) {
        let layout = PaneLayout { cell_w: self.fonts.cell_width, cell_h: self.fonts.cell_height, rect, pad_x: self.padding_x, pad_y: self.padding_y };
        scene::build_pane(scene, &mut self.fonts, &self.theme, &layout, term, state);
    }

    /// Split dividers (drawn before a floating pane so it covers them).
    pub fn draw_dividers(&self, scene: &mut Scene, rects: &[PxRect]) {
        scene::dividers(scene, &self.theme, rects);
    }

    /// The card under a floating pane: shadow, frame and an opaque
    /// background (a floating pane covers what's below it).
    /// The notes panel: a card with a title row, wrapped lines in the
    /// terminal font, and a caret when it has the keyboard.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_notes(&mut self, scene: &mut Scene, r: PxRect, title: &str, lines: &[String], scroll: usize, caret: Option<(usize, usize)>, focused: bool) {
        let s = self.ui_scale.max(0.5);
        let t = self.theme.clone();
        let (cw, ch) = (self.fonts.cell_width, self.fonts.cell_height);
        let base = self.fonts.baseline;
        let bg = blend(t.background, t.foreground, 12);
        scene.frect(r, bg, 255);
        scene.frect(PxRect { w: 1.0, ..r }, blend(t.background, t.foreground, if focused { 120 } else { 50 }), 255);
        let head = (30.0 * s).round();
        let label = if focused { format!("Notes · {title}   Esc: back to the terminal") } else { format!("Notes · {title}   click to write") };
        scene::ui_text(scene, &mut self.fonts, (r.x + 12.0 * s) as i32, r.y + head / 2.0, cw, ch, &label, blend(t.background, t.foreground, 170), false);
        scene.frect(PxRect { x: r.x + 1.0, y: r.y + head, w: r.w - 1.0, h: 1.0 }, blend(t.background, t.foreground, 40), 255);
        let x0 = (r.x + 12.0 * s) as i32;
        let mut y = (r.y + head + 8.0 * s) as i32;
        let rows = ((r.h - head - 8.0 * s) / ch as f32).floor().max(1.0) as usize;
        scene.clip(Some(r));
        for line in lines.iter().skip(scroll).take(rows) {
            scene::text_run(scene, &mut self.fonts, x0, y + base, cw, line, t.foreground, font::Style::Regular);
            y += ch as i32;
        }
        if lines.iter().all(|l| l.is_empty()) && !focused {
            scene::text_run(
                scene,
                &mut self.fonts,
                x0,
                (r.y + head + 8.0 * s) as i32 + base,
                cw,
                "(empty)",
                blend(t.background, t.foreground, 110),
                font::Style::Regular,
            );
        }
        if let (Some((cl, cc)), true) = (caret, focused) {
            if cl >= scroll && cl < scroll + rows {
                let cy = (r.y + head + 8.0 * s) as i32 + ((cl - scroll) * ch) as i32;
                scene.rect(x0 + (cc * cw) as i32, cy, (2.0 * s).round() as usize, ch, t.cursor);
            }
        }
        scene.clip(None);
    }

    /// Blur a region of the picture (secrets in an exported image).
    pub fn blur_rect(&self, scene: &mut Scene, r: PxRect, radius: f32) {
        scene.blur(r, radius);
    }

    /// The title bar over a picture of a command: a status dot (`dot`),
    /// the command in bold and `right` (exit code, time) dimmed at the end.
    pub fn draw_caption(&mut self, scene: &mut Scene, r: PxRect, left: &str, right: &str, dot: u32) {
        let t = self.theme.clone();
        let (cw, ch) = (self.fonts.cell_width, self.fonts.cell_height);
        scene.frect(r, blend(t.background, t.foreground, 16), 255);
        scene.frect(PxRect { x: r.x, y: r.y + r.h - 1.0, w: r.w, h: 1.0 }, blend(t.background, t.foreground, 46), 255);
        let mid = r.y + r.h / 2.0;
        // the status dot, anti-aliased by scanline
        let rad = (r.h * 0.17).max(3.0);
        let cx = r.x + self.padding_x as f32 + rad + 2.0;
        let mut y = (mid - rad).floor();
        while y <= mid + rad {
            let dy = y + 0.5 - mid;
            if dy.abs() < rad {
                let half = (rad * rad - dy * dy).sqrt();
                scene::hspan(scene, cx - half, cx + half, y as i32, dot, 255);
            }
            y += 1.0;
        }
        let x0 = (cx + rad + 10.0) as i32;
        let rw = if right.is_empty() { 0.0 } else { scene::ui_width(&mut self.fonts, cw, right, false) + 16.0 };
        let room = (r.x + r.w - self.padding_x as f32 - rw - x0 as f32).max(20.0);
        let left = scene::fit(&mut self.fonts, cw, left, true, room);
        scene::ui_text(scene, &mut self.fonts, x0, mid, cw, ch, &left, t.foreground, true);
        if !right.is_empty() {
            let x = r.x + r.w - self.padding_x as f32 - rw + 16.0;
            scene::ui_text(scene, &mut self.fonts, x as i32, mid, cw, ch, right, blend(t.background, t.foreground, 150), false);
        }
    }

    /// A coloured frame just inside a pane (SSH hosts marked as special).
    pub fn draw_pane_border(&self, scene: &mut Scene, r: PxRect, color: u32) {
        let t = (2.0 * self.ui_scale.max(0.5)).round();
        scene.frect(PxRect { h: t, ..r }, color, 230);
        scene.frect(PxRect { y: r.y + r.h - t, h: t, ..r }, color, 230);
        scene.frect(PxRect { w: t, ..r }, color, 230);
        scene.frect(PxRect { x: r.x + r.w - t, w: t, ..r }, color, 230);
    }

    pub fn draw_float_frame(&self, scene: &mut Scene, r: PxRect, focused: bool) {
        let s = self.ui_scale.max(0.5);
        let t = &self.theme;
        let pad = 1.0;
        let radius = 8.0 * s;
        settings_view::round_rect(scene, PxRect { x: r.x - 2.0, y: r.y + 3.0 * s, w: r.w + 4.0, h: r.h + 4.0 }, radius, 0x000000, 110);
        let border = if focused { t.cursor } else { blend(t.background, t.foreground, 70) };
        settings_view::round_rect(scene, PxRect { x: r.x - pad, y: r.y - pad, w: r.w + 2.0 * pad, h: r.h + 2.0 * pad }, radius, border, 255);
        settings_view::round_rect(scene, r, radius - 1.0, t.background, 255);
    }

    pub fn draw_chrome(&mut self, scene: &mut Scene, chrome: &Chrome) -> ChromeLayout {
        let (cw, ch) = (self.fonts.cell_width, self.fonts.cell_height);
        let mut out = ChromeLayout::default();
        scene::dividers(scene, &self.theme, &chrome.dividers);
        if let Some((bar, layout)) = &chrome.tabbar {
            if bar.classic {
                scene::classic_bar(scene, &mut self.fonts, &self.theme, cw, ch, bar, layout, self.ui_scale, self.glass);
            } else {
                scene::tab_bar(scene, &mut self.fonts, &self.theme, cw, ch, bar, layout, self.ui_scale, self.glass);
            }
        }
        if self.status_bar {
            if let Some(status) = &chrome.status {
                let h = self.status_height() as f32;
                let bar = PxRect { x: 0.0, y: scene.height as f32 - h, w: scene.width as f32, h };
                scene::status_bar(scene, &mut self.fonts, &self.theme, cw, ch, bar, self.padding_x as i32, status, self.glass);
            }
        }
        if let Some((view, rect, dim)) = &chrome.settings {
            // a dialog over the (still running, blurred) terminal
            scene.blur(*dim, 14.0 * self.ui_scale.max(0.5));
            scene.frect(*dim, 0x000000, 60);
            glass::panel(scene, &self.theme, *rect, self.ui_scale.max(0.5));
            out.settings = Some(settings_view::settings(scene, &mut self.fonts, &self.theme, cw, ch, *rect, view, self.ui_scale));
        }
        let top = chrome.tabbar.as_ref().map_or(0.0, |(_, l)| l.height);
        let mut y = top as i32 + 10;
        for o in &chrome.overlays {
            y = scene::overlay(scene, &mut self.fonts, &self.theme, cw, ch, y, o) + 8;
        }
        if let Some(p) = &chrome.palette {
            scene::palette(scene, &mut self.fonts, &self.theme, cw, ch, top, p);
        }
        if let Some(m) = &chrome.menu {
            out.menu = Some(settings_view::menu(scene, &mut self.fonts, &self.theme, cw, ch, m, self.ui_scale));
        }
        if let Some(c) = &chrome.confirm {
            let area = PxRect { x: 0.0, y: 0.0, w: scene.width as f32, h: scene.height as f32 };
            out.confirm = Some(settings_view::confirm(scene, &mut self.fonts, &self.theme, cw, ch, c, self.ui_scale, area));
        }
        out
    }

    /// One pane filling the window (previews, tests).
    pub fn build_scene(&mut self, term: &Terminal, state: &DrawState, width: usize, height: usize) -> Scene {
        let mut scene = self.begin(width, height);
        let rect = self.content_rect(width, height, false);
        self.draw_pane(&mut scene, term, state, rect);
        let chrome = Chrome { status: state.status.clone(), overlays: state.overlays.clone(), ..Default::default() };
        self.draw_chrome(&mut scene, &chrome);
        scene
    }
}

#[cfg(test)]
mod tests;

/// Fill a rectangle (for the app's own backgrounds).
pub fn fill_rect(scene: &mut Scene, r: PxRect, color: u32, alpha: u8) {
    scene.frect(r, color, alpha);
}
