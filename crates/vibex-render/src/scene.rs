//! Backend-independent frame description. Panes (terminal grids) and the
//! window chrome (tab bar, dividers, status bar, overlays, palette) are
//! turned into an ordered list of rects and glyphs; the CPU and GPU
//! backends just draw that list.

use crate::font::{FontSet, GlyphKey, Style};
use crate::tabbar::{PxRect, TabBar, TabBarLayout, TabHit};
use crate::theme::{blend, Theme};
use crate::{DrawState, Overlay, PaletteView, StatusLine, StatusSegment, VisibleSelection};
use std::sync::Arc;
use vibex_term::image::ImageData;
use vibex_term::{flags, Cell, Color, CursorShape, Terminal};

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub color: u32,
    pub alpha: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct GlyphItem {
    /// Pen position: cell left edge and baseline.
    pub x: i32,
    pub y: i32,
    pub key: GlyphKey,
    pub color: u32,
}

/// Part of an inline image scaled into a destination rectangle.
#[derive(Clone, Copy, Debug)]
pub struct ImageItem {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Source rectangle in image pixels (x, y, w, h).
    pub src: [f32; 4],
    /// Index into `Scene::images`.
    pub index: u32,
}

/// Integer clip rectangle (x, y, w, h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum Item {
    Rect(Rect),
    Glyph(GlyphItem),
    Image(ImageItem),
    /// Clip following items to a rectangle (None = whole window).
    Clip(Option<Clip>),
    /// Blur everything drawn so far inside the rectangle (Gaussian, about
    /// this many pixels); later items draw sharp on top.
    Blur(Clip, f32),
}

pub struct Scene {
    pub width: usize,
    pub height: usize,
    pub background: u32,
    pub items: Vec<Item>,
    /// Images referenced by `Item::Image`.
    pub images: Vec<Arc<ImageData>>,
}

impl Scene {
    pub fn new(width: usize, height: usize, background: u32) -> Self {
        Self { width, height, background, items: Vec::with_capacity(4096), images: Vec::new() }
    }
    pub(crate) fn image(&mut self, img: &Arc<ImageData>, x: i32, y: i32, w: u32, h: u32, src: [f32; 4]) {
        if w == 0 || h == 0 || src[2] <= 0.0 || src[3] <= 0.0 {
            return;
        }
        let index = match self.images.iter().position(|i| i.uid == img.uid) {
            Some(i) => i,
            None => {
                self.images.push(img.clone());
                self.images.len() - 1
            }
        };
        self.items.push(Item::Image(ImageItem { x, y, w, h, src, index: index as u32 }));
    }
    pub(crate) fn rect(&mut self, x: i32, y: i32, w: usize, h: usize, color: u32) {
        self.rect_a(x, y, w, h, color, 255);
    }
    pub(crate) fn rect_a(&mut self, x: i32, y: i32, w: usize, h: usize, color: u32, alpha: u8) {
        if w > 0 && h > 0 {
            self.items.push(Item::Rect(Rect { x, y, w: w as u32, h: h as u32, color, alpha }));
        }
    }
    pub(crate) fn frect(&mut self, r: PxRect, color: u32, alpha: u8) {
        self.rect_a(r.x.round() as i32, r.y.round() as i32, r.w.round() as usize, r.h.round() as usize, color, alpha);
    }
    pub(crate) fn glyph(&mut self, x: i32, y: i32, key: GlyphKey, color: u32) {
        if key.id != 0 || key.face != 0 {
            self.items.push(Item::Glyph(GlyphItem { x, y, key, color }));
        }
    }
    pub(crate) fn blur(&mut self, r: PxRect, radius: f32) {
        let c = Clip { x: r.x.round() as i32, y: r.y.round() as i32, w: r.w.round().max(0.0) as u32, h: r.h.round().max(0.0) as u32 };
        if c.w > 0 && c.h > 0 && radius > 0.0 {
            self.items.push(Item::Blur(c, radius));
        }
    }
    pub(crate) fn clip(&mut self, r: Option<PxRect>) {
        self.items.push(Item::Clip(r.map(|r| Clip {
            x: r.x.round() as i32,
            y: r.y.round() as i32,
            w: r.w.round().max(0.0) as u32,
            h: r.h.round().max(0.0) as u32,
        })));
    }
}

/// The glass look's frosted sheen: a soft white glow fading down from the
/// top, and a light 1px edge round the window. Drawn first, so text and
/// panes sit on top of it.
pub(crate) fn glass_sheen(scene: &mut Scene) {
    let (w, h) = (scene.width, scene.height);
    let fade = (h as f32 * 0.45) as usize;
    let band = 6;
    let mut y = 0;
    while y < fade {
        let t = 1.0 - y as f32 / fade as f32;
        let a = (t * t * 18.0).round() as u8;
        if a > 0 {
            scene.rect_a(0, y as i32, w, band.min(fade - y), 0xffffff, a);
        }
        y += band;
    }
    // the edge: brighter along the top, like light catching glass
    scene.rect_a(0, 0, w, 1, 0xffffff, 60);
    scene.rect_a(0, h as i32 - 1, w, 1, 0xffffff, 22);
    scene.rect_a(0, 1, 1, h.saturating_sub(2), 0xffffff, 30);
    scene.rect_a(w as i32 - 1, 1, 1, h.saturating_sub(2), 0xffffff, 30);
}

fn base_colors(theme: &Theme, cell: &Cell, selected: bool) -> (u32, u32) {
    let mut fg_color = cell.fg;
    if theme.bold_is_bright && cell.has(flags::BOLD) {
        if let Color::Indexed(i @ 0..=7) = fg_color {
            fg_color = Color::Indexed(i + 8);
        }
    }
    let mut fg = theme.resolve(fg_color, theme.foreground);
    let mut bg = theme.resolve(cell.bg, theme.background);
    if cell.has(flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if selected {
        bg = theme.selection;
    }
    if cell.has(flags::DIM) {
        fg = blend(bg, fg, 150);
    }
    if cell.has(flags::HIDDEN) {
        fg = bg;
    }
    (fg, bg)
}

fn in_range(r: &Option<VisibleSelection>, x: usize, vy: isize) -> bool {
    vy >= 0 && r.is_some_and(|s| s.contains(x, vy as usize))
}

fn colors(theme: &Theme, cell: &Cell, x: usize, vy: isize, state: &DrawState) -> (u32, u32) {
    let selected = in_range(&state.selection, x, vy);
    let (mut fg, mut bg) = base_colors(theme, cell, selected);
    if !selected && vy >= 0 {
        if let Some(h) = state.highlights.iter().rev().find(|h| h.range.contains(x, vy as usize)) {
            bg = h.bg;
            if let Some(f) = h.fg {
                fg = f;
            }
        }
    }
    (fg, bg)
}

fn is_box_char(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F)
}

/// Where and how a pane is drawn.
pub struct PaneLayout {
    pub cell_w: usize,
    pub cell_h: usize,
    /// The pane's full rectangle (padding is inside it).
    pub rect: PxRect,
    pub pad_x: usize,
    pub pad_y: usize,
}

/// Draw one terminal into its rectangle (clipped).
pub fn build_pane(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, layout: &PaneLayout, term: &Terminal, state: &DrawState) {
    let (cw, ch) = (layout.cell_w, layout.cell_h);
    let px = layout.rect.x.round() as i32 + layout.pad_x as i32;
    let py = layout.rect.y.round() as i32 + layout.pad_y as i32;
    scene.clip(Some(layout.rect));

    // DECSCNM: swap the default colours for this pane and paint its
    // (normally transparent) background
    let reversed;
    let theme = if term.modes.reverse_screen {
        reversed = theme.reversed();
        scene.frect(layout.rect, reversed.background, 255);
        &reversed
    } else {
        theme
    };

    let rows = term.rows() as isize;
    let cols = term.cols();
    let top_line = term.top_visible_line() as isize;
    let total = term.total_lines() as isize;
    let shift = state.y_shift.round() as i32;
    let extra_above = ((shift.max(0) as usize).div_ceil(ch)) as isize;
    let extra_below = (((-shift).max(0) as usize).div_ceil(ch)) as isize;
    let row_range = -extra_above..rows + extra_below;
    let row_top = |vy: isize| py + (vy as i32) * ch as i32 + shift;

    let offset = term.display_offset();
    let cursor_vy = (term.cursor.y + offset) as isize;
    let show_cursor = term.modes.show_cursor && cursor_vy < rows && state.cursor_visible;
    let cursor_x = term.cursor.x;
    let block_cursor = show_cursor && state.focused && term.cursor_shape == CursorShape::Block;

    let row_at = |vy: isize| {
        let line = top_line + vy;
        (line >= 0 && line < total).then(|| term.absolute_row(line as usize)).flatten()
    };

    // pass 1: backgrounds (+ prompt marks in the left gutter)
    for vy in row_range.clone() {
        let Some(row) = row_at(vy) else { continue };
        let top = row_top(vy);
        let mut x = 0;
        let n = cols.min(row.cells.len());
        while x < n {
            let (_, bg) = colors(theme, &row.cells[x], x, vy, state);
            let start = x;
            while x < n && colors(theme, &row.cells[x], x, vy, state).1 == bg {
                x += 1;
            }
            if bg != theme.background {
                scene.rect(px + (start * cw) as i32, top, (x - start) * cw, ch, bg);
            }
        }
        if state.marks && layout.pad_x >= 4 {
            if let Some(mark) = row.prompt {
                let color = match mark.exit {
                    Some(0) => theme.palette[2],
                    Some(_) => theme.palette[1],
                    None => blend(theme.background, theme.foreground, 90),
                };
                let gx = layout.rect.x.round() as i32 + (layout.pad_x as i32 - 3) / 2;
                scene.rect(gx, top + 2, 3, ch.saturating_sub(4), color);
            }
        }
    }

    draw_images(scene, row_range.clone(), &row_at, &row_top, px, cw, ch, true);

    if block_cursor {
        let wide = term.visible_row(cursor_vy as usize).cells.get(cursor_x).is_some_and(|c| c.has(flags::WIDE));
        let w = if wide { cw * 2 } else { cw };
        scene.rect(px + (cursor_x * cw) as i32, row_top(cursor_vy), w, ch, theme.cursor);
    }

    // pass 2: glyphs and decorations
    let row_range_images = row_range.clone();
    let baseline = fonts.baseline;
    let thick = fonts.line_thickness;
    let underline_y = fonts.underline_y;
    let ligatures = fonts.ligatures;
    for vy in row_range {
        let Some(row) = row_at(vy) else { continue };
        let top = row_top(vy);
        let n = cols.min(row.cells.len());
        let fg_at = |x: usize| {
            if block_cursor && vy == cursor_vy && x == cursor_x {
                theme.cursor_text
            } else {
                colors(theme, &row.cells[x], x, vy, state).0
            }
        };
        let left = |x: usize| px + (x * cw) as i32;
        let style_of = |c: &Cell| Style::new(c.has(flags::BOLD), c.has(flags::ITALIC));

        let mut x = 0;
        while x < n {
            let cell = &row.cells[x];
            if cell.has(flags::WIDE_SPACER | flags::WRAP_PAD | flags::HIDDEN) || cell.c == ' ' {
                x += 1;
                continue;
            }
            let fg = fg_at(x);
            let cell_w = if cell.has(flags::WIDE) { cw * 2 } else { cw };
            if is_box_char(cell.c) && box_char(scene, cell.c, left(x), top, cell_w, ch, fg, thick) {
                x += 1;
                continue;
            }
            let style = style_of(cell);

            // a grapheme cluster: shape it as one unit
            if vibex_term::grapheme::is_cluster(cell.c) {
                let text = vibex_term::grapheme::text(cell.c);
                for g in fonts.shape_cluster(&text, style) {
                    scene.glyph(left(x) + g.x, top + baseline - g.y, g.key, fg);
                }
                x += 1;
                continue;
            }

            // ligature run: same style + colour, ASCII, covered by the run face
            if ligatures && cell.c.is_ascii_graphic() && fonts.in_run_face(cell.c, style) {
                let mut end = x + 1;
                while end < n {
                    let c2 = &row.cells[end];
                    if c2.c == ' '
                        || !c2.c.is_ascii_graphic()
                        || c2.has(flags::HIDDEN)
                        || style_of(c2) != style
                        || fg_at(end) != fg
                        || is_box_char(c2.c)
                        || !fonts.in_run_face(c2.c, style)
                    {
                        break;
                    }
                    end += 1;
                }
                let text: String = row.cells[x..end].iter().map(|c| c.c).collect();
                if end - x >= 2 && text.bytes().any(|b| b.is_ascii_punctuation()) {
                    let (face, fake_bold, shaped) = fonts.shape(&text, style);
                    let shaped = shaped.to_vec();
                    for g in shaped {
                        let cx = x + g.cluster as usize;
                        let key = GlyphKey { face, id: g.id, fake_bold, color: false, split: 0 };
                        scene.glyph(left(cx) + g.x_offset, top + baseline, key, fg);
                    }
                } else {
                    for (i, c) in text.chars().enumerate() {
                        let key = fonts.resolve(c, style);
                        scene.glyph(left(x + i), top + baseline, key, fg);
                    }
                }
                x = end;
                continue;
            }

            let key = fonts.resolve(cell.c, style);
            scene.glyph(left(x), top + baseline, key, fg);
            x += 1;
        }

        // decorations
        for x in 0..n {
            let cell = &row.cells[x];
            if cell.has(flags::WIDE_SPACER) {
                continue;
            }
            let linked = in_range(&state.link, x, vy);
            if !(cell.has(flags::ANY_UNDERLINE | flags::STRIKE) || linked) {
                continue;
            }
            let fg = fg_at(x);
            let l = left(x);
            let cell_w = if cell.has(flags::WIDE) { cw * 2 } else { cw };
            if cell.has(flags::UNDERLINE) || linked {
                scene.rect(l, top + underline_y, cell_w, thick, fg);
            } else if cell.has(flags::DOUBLE_UNDERLINE) {
                scene.rect(l, top + underline_y - thick as i32, cell_w, thick, fg);
                scene.rect(l, top + underline_y + thick as i32, cell_w, thick, fg);
            } else if cell.has(flags::CURLY_UNDERLINE) {
                for i in 0..cell_w as i32 {
                    let phase = ((l + i) as f32 / 2.0).sin();
                    let yy = top + underline_y + (phase * thick as f32).round() as i32;
                    scene.rect(l + i, yy, 1, thick, fg);
                }
            }
            if cell.has(flags::STRIKE) {
                scene.rect(l, top + ch as i32 / 2, cell_w, thick, fg);
            }
        }
    }

    draw_images(scene, row_range_images, &row_at, &row_top, px, cw, ch, false);

    // non-block / unfocused cursor
    if show_cursor && !block_cursor {
        let left = px + (cursor_x * cw) as i32;
        let top = row_top(cursor_vy);
        let color = theme.cursor;
        let t = thick.max(2);
        match (state.focused, term.cursor_shape) {
            (true, CursorShape::Bar) => scene.rect(left, top, t, ch, color),
            (true, CursorShape::Underline) => scene.rect(left, top + ch as i32 - t as i32, cw, t, color),
            _ => {
                scene.rect(left, top, cw, 1, color);
                scene.rect(left, top + ch as i32 - 1, cw, 1, color);
                scene.rect(left, top, 1, ch, color);
                scene.rect(left + cw as i32 - 1, top, 1, ch, color);
            }
        }
    }

    // the command whose output is on screen, pinned over the first row
    if let Some(st) = &state.sticky {
        let top = layout.rect.y.round() as i32;
        let h = ch + layout.pad_y;
        let bg = blend(theme.background, theme.foreground, 22);
        scene.rect(layout.rect.x.round() as i32, top, layout.rect.w.round() as usize, h, bg);
        scene.rect(layout.rect.x.round() as i32, top + h as i32, layout.rect.w.round() as usize, 1, blend(bg, theme.foreground, 40));
        scene.rect(px - (layout.pad_x as i32).min(6) + 1, top + 3, 3, h.saturating_sub(6), st.color);
        let cells = ((layout.rect.w as usize).saturating_sub(layout.pad_x * 2) / cw).max(4);
        let detail_w = crate::text_width(&st.detail);
        let cmd = truncate(&format!("❯ {}", st.command), cells.saturating_sub(detail_w + 2));
        let base = top + layout.pad_y as i32 / 2 + baseline;
        text_run(scene, fonts, px, base, cw, &cmd, theme.foreground, Style::Bold);
        if detail_w > 0 {
            let dx = px + ((cells - detail_w) * cw) as i32;
            text_run(scene, fonts, dx, base, cw, &st.detail, blend(bg, theme.foreground, 150), Style::Regular);
        }
    }

    // copy mode cursor: a hollow box that doesn't hide the text under it
    if let Some((cx, cy)) = state.copy_cursor {
        let (l, t) = (px + (cx * cw) as i32, row_top(cy as isize));
        let b = thick.max(2);
        scene.rect(l, t, cw, b, theme.cursor);
        scene.rect(l, t + ch as i32 - b as i32, cw, b, theme.cursor);
        scene.rect(l, t, b, ch, theme.cursor);
        scene.rect(l + cw as i32 - b as i32, t, b, ch, theme.cursor);
    }

    if state.dim {
        scene.frect(layout.rect, theme.background, 70);
    }

    // a suggestion: dim, after the cursor, cut at the right edge
    if let Some(text) = state.ghost.as_deref().filter(|t| !t.is_empty()) {
        if state.preedit.is_none() {
            let top = row_top(cursor_vy);
            let left = px + (cursor_x * cw) as i32;
            let room = term.cols().saturating_sub(cursor_x);
            let mut shown = String::new();
            let mut width = 0;
            for ch in text.chars() {
                let w = crate::text_width(&ch.to_string()).max(1);
                if width + w > room {
                    break;
                }
                width += w;
                shown.push(ch);
            }
            text_run(scene, fonts, left, top + baseline, cw, &shown, blend(theme.background, theme.foreground, 105), Style::Regular);
        }
    }

    // IME composition: inline at the cursor, underlined, over the text below
    if let Some(text) = state.preedit.as_deref().filter(|t| !t.is_empty()) {
        let top = row_top(cursor_vy);
        let left = px + (cursor_x * cw) as i32;
        let w = crate::text_width(text).max(1) * cw;
        scene.rect(left, top, w, ch, blend(theme.background, theme.foreground, 28));
        text_run(scene, fonts, left, top + baseline, cw, text, theme.foreground, Style::Regular);
        scene.rect(left, top + underline_y, w, thick.max(1), theme.cursor);
    }

    // quick-select badges over the start of each target
    for (col, row, label) in &state.labels {
        let top = row_top(*row as isize);
        let left = px + (*col * cw) as i32;
        let w = crate::text_width(label).max(1) * cw;
        scene.rect(left, top, w, ch, theme.cursor);
        text_run(scene, fonts, left, top + baseline, cw, label, theme.cursor_text, Style::Bold);
    }
    scene.clip(None);
}

/// Image slices on the visible rows: those below the text (`z < 0`) or
/// above it.
#[allow(clippy::too_many_arguments)]
fn draw_images<'a>(
    scene: &mut Scene,
    rows: std::ops::Range<isize>,
    row_at: &impl Fn(isize) -> Option<&'a vibex_term::Row>,
    row_top: &impl Fn(isize) -> i32,
    px: i32,
    cw: usize,
    ch: usize,
    below: bool,
) {
    let (cwf, chf) = (cw as f32, ch as f32);
    for vy in rows {
        let Some(row) = row_at(vy) else { continue };
        let top = row_top(vy);
        for s in row.images.iter().filter(|s| (s.z < 0) == below) {
            let x0 = px as f32 + (s.col as f32 + s.dest[0]) * cwf;
            let y0 = top as f32 + s.dest[1] * chf;
            let x1 = x0 + s.dest[2] * cwf;
            let y1 = y0 + s.dest[3] * chf;
            let (xi, yi) = (x0.round() as i32, y0.round() as i32);
            let w = (x1.round() as i32 - xi).max(0) as u32;
            let h = (y1.round() as i32 - yi).max(0) as u32;
            scene.image(&s.image, xi, yi, w, h, s.src);
        }
    }
}

// ------------------------------------------------------------------ chrome

/// Draw `text` starting at pixel `x`, returning the x after it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn text_run(scene: &mut Scene, fonts: &mut FontSet, x: i32, baseline_y: i32, cw: usize, text: &str, fg: u32, style: Style) -> i32 {
    use unicode_width::UnicodeWidthChar;
    let mut x = x;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if w == 0 {
            continue;
        }
        if c != ' ' {
            let key = fonts.resolve(c, style);
            scene.glyph(x, baseline_y, key, fg);
        }
        x += (w * cw) as i32;
    }
    x
}

/// Shorten `s` to `max` cells with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if crate::text_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// A bordered box in the top-right corner, one line per `\n`. Returns its
/// bottom edge.
#[allow(clippy::too_many_arguments)]
pub fn overlay(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, y: i32, o: &Overlay) -> i32 {
    let lines: Vec<&str> = o.text.lines().collect();
    let widest = lines.iter().map(|l| crate::text_width(l)).max().unwrap_or(0);
    let pad = cw as i32;
    let max_w = scene.width.saturating_sub(28);
    let w = (widest * cw + pad as usize * 2).min(max_w);
    let h = lines.len().max(1) * ch + 10;
    let x = scene.width as i32 - w as i32 - 14;
    scene.rect(x - 1, y - 1, w + 2, h + 2, o.border);
    scene.rect(x, y, w, h, blend(theme.background, theme.foreground, 22));
    let baseline = fonts.baseline;
    for (i, line) in lines.iter().enumerate() {
        let ly = y + 5 + (i * ch) as i32 + baseline;
        text_run(scene, fonts, x + pad, ly, cw, line, theme.foreground, Style::Regular);
    }
    y + h as i32
}

#[allow(clippy::too_many_arguments)]
pub fn status_bar(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, bar: PxRect, pad_x: i32, s: &StatusLine, glass: bool) {
    let y = bar.y.round() as i32;
    let h = bar.h.round() as i32;
    let bar_bg = blend(theme.background, theme.foreground, 16);
    if glass {
        scene.frect(bar, 0x000000, 50);
        scene.rect_a(0, y, scene.width, 1, 0xffffff, 26);
    } else {
        scene.frect(bar, bar_bg, 255);
        scene.rect(0, y, scene.width, 1, blend(theme.background, theme.foreground, 40));
    }
    let top = y + (h - ch as i32) / 2;
    let baseline = top + fonts.baseline;

    let draw = |scene: &mut Scene, fonts: &mut FontSet, x: i32, seg: &StatusSegment| -> i32 {
        let w = crate::text_width(&seg.text) * cw;
        if let Some(bg) = seg.bg {
            scene.rect(x, top, w, ch, bg);
        }
        let fg = seg.fg.unwrap_or(theme.foreground);
        text_run(scene, fonts, x, baseline, cw, &seg.text, fg, Style::new(seg.bold, false))
    };

    let mut x = pad_x;
    for seg in &s.left {
        x = draw(scene, fonts, x, seg);
    }
    let right_w: usize = s.right.iter().map(|seg| crate::text_width(&seg.text) * cw).sum();
    let mut rx = scene.width as i32 - pad_x - right_w as i32;
    for seg in &s.right {
        rx = draw(scene, fonts, rx, seg);
    }
}

pub fn dividers(scene: &mut Scene, theme: &Theme, rects: &[PxRect]) {
    let c = blend(theme.background, theme.foreground, 46);
    for r in rects {
        scene.frect(*r, c, 255);
    }
}

/// Rows of a rectangle whose top corners are rounded with `radius`: for
/// each pixel row, the inset from each side (fractional, for edge alpha).
pub(crate) fn round_top_inset(radius: f32, row: f32) -> f32 {
    if radius <= 0.0 || row >= radius {
        return 0.0;
    }
    let dy = radius - row - 0.5;
    radius - (radius * radius - dy * dy).max(0.0).sqrt()
}

/// Fill [x0, x1) on pixel row `y`, with partial alpha on fractional ends.
pub(crate) fn hspan(scene: &mut Scene, x0: f32, x1: f32, y: i32, color: u32, alpha: u8) {
    if x1 <= x0 {
        return;
    }
    let (a, b) = (x0.ceil(), x1.floor());
    if b > a {
        scene.rect_a(a as i32, y, (b - a) as usize, 1, color, alpha);
    }
    let lead = a - x0;
    if lead > 0.01 {
        scene.rect_a(x0.floor() as i32, y, 1, 1, color, (alpha as f32 * lead) as u8);
    }
    let tail = x1 - b;
    if tail > 0.01 && b >= a {
        scene.rect_a(b as i32, y, 1, 1, color, (alpha as f32 * tail) as u8);
    }
}

/// A rectangle with rounded top corners.
pub(crate) fn round_top_rect(scene: &mut Scene, r: PxRect, radius: f32, color: u32, alpha: u8) {
    let (x, y0, h) = (r.x, r.y.round() as i32, r.h.round() as i32);
    for row in 0..h {
        let inset = round_top_inset(radius, row as f32);
        if inset == 0.0 && row as f32 >= radius {
            scene.rect_a(x.round() as i32, y0 + row, r.w.round() as usize, (h - row) as usize, color, alpha);
            break;
        }
        hspan(scene, x + inset, x + r.w - inset, y0 + row, color, alpha);
    }
}

/// A 1px line from (x0, y0) to (x1, y1), lightly anti-aliased.
pub(crate) fn line(scene: &mut Scene, x0: f32, y0: f32, x1: f32, y1: f32, c: u32) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
        let (fx, fy) = (x.floor(), y.floor());
        let (ax, ay) = (x - fx, y - fy);
        let a = |w: f32| (255.0 * w).round().clamp(0.0, 255.0) as u8;
        scene.rect_a(fx as i32, fy as i32, 1, 1, c, a((1.0 - ax) * (1.0 - ay) + 0.35));
        if ax > 0.1 {
            scene.rect_a(fx as i32 + 1, fy as i32, 1, 1, c, a(ax * (1.0 - ay)));
        }
        if ay > 0.1 {
            scene.rect_a(fx as i32, fy as i32 + 1, 1, 1, c, a((1.0 - ax) * ay));
        }
    }
}

/// Text in the UI font (Segoe UI), else the terminal font. Returns the x
/// after it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ui_text(scene: &mut Scene, fonts: &mut FontSet, x: i32, mid_y: f32, cw: usize, ch: usize, text: &str, fg: u32, bold: bool) -> i32 {
    match fonts.ui_run(text, bold) {
        Some((glyphs, w)) => {
            // centre the x-height-ish band on mid_y
            let baseline = (mid_y + fonts.size_px() * 0.8 * 0.36).round() as i32;
            for g in glyphs {
                scene.glyph(x + g.x, baseline, g.key, fg);
            }
            x + w
        }
        None => {
            let baseline = (mid_y - ch as f32 / 2.0).round() as i32 + fonts.baseline;
            text_run(scene, fonts, x, baseline, cw, text, fg, if bold { Style::Bold } else { Style::Regular })
        }
    }
}

/// Width of `text` as `ui_text` would draw it.
pub(crate) fn ui_width(fonts: &mut FontSet, cw: usize, text: &str, bold: bool) -> f32 {
    fonts.ui_run(text, bold).map_or((crate::text_width(text) * cw) as f32, |(_, w)| w as f32)
}

/// Cut `text` (with an ellipsis) to fit `max` pixels.
pub(crate) fn fit(fonts: &mut FontSet, cw: usize, text: &str, bold: bool, max: f32) -> String {
    if ui_width(fonts, cw, text, bold) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 0 {
        n -= 1;
        let t: String = chars[..n].iter().collect::<String>() + "…";
        if ui_width(fonts, cw, &t, bold) <= max {
            return t;
        }
    }
    String::new()
}

/// The title bar, drawn like Windows Terminal's: a dark strip; the active
/// tab is cut out of it with rounded top corners so it flows into the
/// terminal below; icon + title + ✕ on each tab; + and ⌄; window buttons.
#[allow(clippy::too_many_arguments)]
pub fn tab_bar(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, bar: &TabBar, l: &TabBarLayout, scale: f32, glass: bool) {
    let s = scale.max(0.5);
    let width = scene.width as f32;
    let bar_bg = blend(theme.background, 0x000000, 70);
    // glass: a light smoky strip the backdrop shows through
    let bar_alpha = if glass { 90u8 } else { 235u8 };
    let text_on = blend(theme.foreground, 0xffffff, 140);
    let dim = blend(bar_bg, theme.foreground, 165);
    let hover_bg = blend(bar_bg, theme.foreground, 18);
    let radius = (8.0 * s).round();

    // the strip, with the active tab's shape left open
    let active = bar.tabs.iter().position(|t| t.active).and_then(|i| l.rect(TabHit::Tab(i)));
    match active {
        Some(t) => {
            let top = t.y.round() as i32;
            scene.rect_a(0, 0, width as usize, top as usize, bar_bg, bar_alpha);
            for row in top..l.height.round() as i32 {
                let inset = round_top_inset(radius, (row - top) as f32);
                hspan(scene, 0.0, t.x + inset, row, bar_bg, bar_alpha);
                hspan(scene, t.x + t.w - inset, width, row, bar_bg, bar_alpha);
            }
        }
        None => scene.frect(PxRect { x: 0.0, y: 0.0, w: width, h: l.height }, bar_bg, bar_alpha),
    }

    let close_half = (5.0 * s).round();
    for (i, tab) in bar.tabs.iter().enumerate() {
        let Some(r) = l.rect(TabHit::Tab(i)) else { continue };
        let hovered = bar.hover == Some(TabHit::Tab(i)) || bar.hover == Some(TabHit::CloseTab(i));
        if !tab.active && hovered {
            round_top_rect(scene, PxRect { x: r.x + 2.0, w: r.w - 4.0, h: r.h - 3.0 * s, ..r }, radius, hover_bg, 255);
        }
        if let Some(c) = tab.color {
            // a tab colour tints the whole tab, like Windows Terminal
            round_top_rect(scene, r, radius, c, if tab.active { 70 } else { 120 });
        }
        if !tab.active && !hovered && i + 1 < bar.tabs.len() && !bar.tabs[i + 1].active {
            // thin divider between two inactive tabs
            let x = (r.x + r.w - 1.0).round();
            scene.frect(PxRect { x, y: r.y + 8.0 * s, w: 1.0, h: r.h - 16.0 * s }, blend(bar_bg, theme.foreground, 40), 255);
        }
        let fg = if tab.active || tab.color.is_some() { text_on } else { dim };
        let mid = r.y + r.h / 2.0;
        let icon_x = r.x + 12.0 * s;
        match tab.icon {
            crate::tabbar::TabIcon::Terminal => crate::icons::draw(scene, "terminal", icon_x + 8.0 * s, mid, 17.0 * s, fg),
            crate::tabbar::TabIcon::Settings => crate::settings_view::gear_icon(scene, icon_x + 8.0 * s, mid, s, fg),
        }
        let text_x = icon_x + 16.0 * s + 10.0 * s;
        let close = l.rect(TabHit::CloseTab(i));
        let right = close.map_or(r.x + r.w - 8.0 * s, |c| c.x) - 6.0 * s;
        let title = if tab.editing { format!("{}▏", tab.title) } else { tab.title.clone() };
        let title = fit(fonts, cw, &title, tab.active, (right - text_x).max(0.0));
        let end = ui_text(scene, fonts, text_x.round() as i32, mid, cw, ch, &title, fg, tab.active);
        if tab.alert {
            let d = (5.0 * s).round();
            scene.frect(PxRect { x: end as f32 + 5.0 * s, y: (mid - d / 2.0).round(), w: d, h: d }, theme.palette[1], 255);
        }
        if let Some(c) = close {
            if bar.hover == Some(TabHit::CloseTab(i)) {
                let bg = if tab.active { blend(theme.background, theme.foreground, 26) } else { blend(hover_bg, theme.foreground, 18) };
                scene.frect(c, bg, 255);
            }
            draw_thin_x(scene, c, close_half, fg);
        }
    }

    for hit in [TabHit::NewTab, TabHit::NewTabMenu] {
        let Some(r) = l.rect(hit) else { continue };
        if bar.hover == Some(hit) {
            round_top_rect(scene, r, (4.0 * s).round(), hover_bg, 255);
            round_top_rect(scene, PxRect { y: r.y + r.h - 1.0, h: 1.0, ..r }, 0.0, hover_bg, 255);
        }
        let (cx, cy) = ((r.x + r.w / 2.0).round(), (r.y + r.h / 2.0).round());
        let name = if hit == TabHit::NewTab { "plus" } else { "chevron-down" };
        crate::icons::draw(scene, name, cx, cy, if hit == TabHit::NewTab { 18.0 } else { 15.0 } * s, text_on);
    }

    if let Some(caption) = &bar.caption {
        if let Some(nt) = l.rect(TabHit::NewTabMenu) {
            let x = nt.x + nt.w + 12.0 * s;
            let right = l.rect(TabHit::Minimize).map_or(width, |m| m.x) - 12.0 * s;
            let t = fit(fonts, cw, caption, false, right - x);
            if !t.is_empty() {
                ui_text(scene, fonts, x.round() as i32, l.height / 2.0, cw, ch, &t, dim, false);
            }
        }
    }

    // window buttons: thin 10px glyphs like Windows' own
    let icon = (10.0 * s).round();
    for hit in [TabHit::Minimize, TabHit::Maximize, TabHit::Close] {
        let Some(r) = l.rect(hit) else { continue };
        let hovered = bar.hover == Some(hit);
        let mut fg = text_on;
        if hovered {
            if hit == TabHit::Close {
                scene.frect(r, 0xc42b1c, 255);
                fg = 0xffffff;
            } else {
                scene.frect(r, hover_bg, 255);
            }
        }
        let (cx, cy) = ((r.x + r.w / 2.0).round(), (r.y + r.h / 2.0).round());
        let half = (icon / 2.0).round();
        match hit {
            TabHit::Minimize => scene.frect(PxRect { x: cx - half, y: cy, w: icon, h: 1.0 }, fg, 255),
            TabHit::Maximize if bar.maximized => {
                let sq = icon - (2.0 * s).round();
                let off = (2.0 * s).round();
                outline(scene, PxRect { x: cx - half + off, y: cy - half, w: sq, h: sq }, fg);
                scene.frect(PxRect { x: cx - half, y: cy - half + off, w: sq, h: sq }, if hovered { hover_bg } else { bar_bg }, 255);
                outline(scene, PxRect { x: cx - half, y: cy - half + off, w: sq, h: sq }, fg);
            }
            TabHit::Maximize => outline(scene, PxRect { x: cx - half, y: cy - half, w: icon, h: icon }, fg),
            TabHit::Close => draw_thin_x(scene, r, half, fg),
            _ => {}
        }
    }
}

/// A thin ✕ centred in `r`, arms `half` long.
pub(crate) fn draw_thin_x(scene: &mut Scene, r: PxRect, half: f32, c: u32) {
    let (cx, cy) = ((r.x + r.w / 2.0).round(), (r.y + r.h / 2.0).round());
    line(scene, cx - half, cy - half, cx + half, cy + half, c);
    line(scene, cx - half, cy + half, cx + half, cy - half, c);
}

pub(crate) fn outline(scene: &mut Scene, r: PxRect, c: u32) {
    scene.frect(PxRect { h: 1.0, ..r }, c, 255);
    scene.frect(PxRect { y: r.y + r.h - 1.0, h: 1.0, ..r }, c, 255);
    scene.frect(PxRect { w: 1.0, ..r }, c, 255);
    scene.frect(PxRect { x: r.x + r.w - 1.0, w: 1.0, ..r }, c, 255);
}

/// The command palette: a centred box with a query line and a list.
pub fn palette(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, top: f32, p: &PaletteView) {
    let max_rows = 12usize;
    let w = ((64 * cw) as f32).min(scene.width as f32 - 40.0).max(cw as f32 * 20.0);
    let x = ((scene.width as f32 - w) / 2.0).round();
    let row_h = ch as f32 + 6.0;
    let shown = p.items.len().min(max_rows);
    let h = row_h * (shown as f32 + 1.0) + 14.0;
    let y = top + 12.0;
    let bg = blend(theme.background, theme.foreground, 20);
    // shadow + frame
    scene.frect(PxRect { x: x + 3.0, y: y + 4.0, w, h }, 0x000000, 90);
    scene.frect(PxRect { x: x - 1.0, y: y - 1.0, w: w + 2.0, h: h + 2.0 }, theme.cursor, 255);
    scene.frect(PxRect { x, y, w, h }, bg, 255);

    let cells = ((w - cw as f32 * 3.0) / cw as f32) as usize;
    let fb = fonts.baseline;
    let base = |ry: f32| (ry + (row_h - ch as f32) / 2.0).round() as i32 + fb;
    // query line
    let qy = y + 6.0;
    let prompt_end = text_run(scene, fonts, (x + cw as f32).round() as i32, base(qy), cw, "❯ ", theme.cursor, Style::Bold);
    let q = if p.query.is_empty() { "type a command…".to_string() } else { format!("{}_", p.query) };
    let qfg = if p.query.is_empty() { blend(bg, theme.foreground, 120) } else { theme.foreground };
    text_run(scene, fonts, prompt_end, base(qy), cw, &truncate(&q, cells.saturating_sub(2)), qfg, Style::Regular);
    scene.frect(PxRect { x: x + 6.0, y: qy + row_h + 1.0, w: w - 12.0, h: 1.0 }, blend(bg, theme.foreground, 40), 255);

    // keep the selection in view
    let first = p.selected.saturating_sub(max_rows - 1);
    for (row, i) in (first..p.items.len()).take(max_rows).enumerate() {
        let (label, hint) = &p.items[i];
        let ry = qy + row_h * (row as f32 + 1.0) + 4.0;
        let selected = i == p.selected;
        if selected {
            scene.frect(PxRect { x: x + 4.0, y: ry, w: w - 8.0, h: row_h }, theme.selection, 255);
        }
        let hint_w = crate::text_width(hint);
        let label_cells = cells.saturating_sub(hint_w + 2);
        text_run(scene, fonts, (x + cw as f32).round() as i32, base(ry), cw, &truncate(label, label_cells), theme.foreground, Style::Regular);
        if !hint.is_empty() {
            let hx = (x + w - cw as f32 * (hint_w as f32 + 1.0)).round() as i32;
            text_run(scene, fonts, hx, base(ry), cw, hint, blend(bg, theme.foreground, 130), Style::Regular);
        }
    }
    if p.items.is_empty() {
        let ry = qy + row_h + 4.0;
        text_run(scene, fonts, (x + cw as f32).round() as i32, base(ry), cw, "no matching commands", blend(bg, theme.foreground, 120), Style::Regular);
    }
}

/// Box-drawing and block characters as rectangles so lines join perfectly
/// between cells. Returns false if `c` isn't handled.
#[allow(clippy::too_many_arguments)]
fn box_char(s: &mut Scene, c: char, x: i32, y: i32, w: usize, h: usize, color: u32, thick: usize) -> bool {
    let (wi, hi) = (w as i32, h as i32);
    let t = thick as i32;
    let heavy = (t * 2).max(2);

    // (left, right, up, down) arm weights: 0 none, 1 light, 2 heavy
    let arms: Option<(u8, u8, u8, u8)> = match c {
        '─' => Some((1, 1, 0, 0)),
        '━' => Some((2, 2, 0, 0)),
        '│' => Some((0, 0, 1, 1)),
        '┃' => Some((0, 0, 2, 2)),
        '┌' | '╭' => Some((0, 1, 0, 1)),
        '┐' | '╮' => Some((1, 0, 0, 1)),
        '└' | '╰' => Some((0, 1, 1, 0)),
        '┘' | '╯' => Some((1, 0, 1, 0)),
        '├' => Some((0, 1, 1, 1)),
        '┤' => Some((1, 0, 1, 1)),
        '┬' => Some((1, 1, 0, 1)),
        '┴' => Some((1, 1, 1, 0)),
        '┼' => Some((1, 1, 1, 1)),
        '┏' => Some((0, 2, 0, 2)),
        '┓' => Some((2, 0, 0, 2)),
        '┗' => Some((0, 2, 2, 0)),
        '┛' => Some((2, 0, 2, 0)),
        '┣' => Some((0, 2, 2, 2)),
        '┫' => Some((2, 0, 2, 2)),
        '┳' => Some((2, 2, 0, 2)),
        '┻' => Some((2, 2, 2, 0)),
        '╋' => Some((2, 2, 2, 2)),
        '╴' => Some((1, 0, 0, 0)),
        '╶' => Some((0, 1, 0, 0)),
        '╵' => Some((0, 0, 1, 0)),
        '╷' => Some((0, 0, 0, 1)),
        _ => None,
    };
    if let Some((l, r, u, d)) = arms {
        let wt = |k: u8| if k == 2 { heavy } else { t };
        let mid_x = x + wi / 2;
        let mid_y = y + hi / 2;
        if l > 0 {
            let k = wt(l);
            s.rect(x, mid_y - k / 2, (wi / 2 + k / 2 + k % 2) as usize, k as usize, color);
        }
        if r > 0 {
            let k = wt(r);
            s.rect(mid_x - k / 2, mid_y - k / 2, (wi - wi / 2 + k / 2) as usize, k as usize, color);
        }
        if u > 0 {
            let k = wt(u);
            s.rect(mid_x - k / 2, y, k as usize, (hi / 2 + k / 2 + k % 2) as usize, color);
        }
        if d > 0 {
            let k = wt(d);
            s.rect(mid_x - k / 2, mid_y - k / 2, k as usize, (hi - hi / 2 + k / 2) as usize, color);
        }
        return true;
    }

    match c {
        '█' => s.rect(x, y, w, h, color),
        '▀' => s.rect(x, y, w, h / 2, color),
        '▄' => s.rect(x, y + hi / 2, w, h - h / 2, color),
        '▌' => s.rect(x, y, w / 2, h, color),
        '▐' => s.rect(x + wi / 2, y, w - w / 2, h, color),
        '▁'..='▇' => {
            let eighths = c as i32 - '▁' as i32 + 1;
            let bh = hi * eighths / 8;
            s.rect(x, y + hi - bh, w, bh as usize, color);
        }
        '░' => s.rect_a(x, y, w, h, color, 64),
        '▒' => s.rect_a(x, y, w, h, color, 128),
        '▓' => s.rect_a(x, y, w, h, color, 192),
        _ => return false,
    }
    true
}
