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
        if w == 0 || h == 0 || src[2] <= 0.0 || src[3] <= 0.0 || !image_ok(img) {
            return;
        }
        let (w, h) = (w.min(MAX_EXTENT), h.min(MAX_EXTENT));
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
            let (w, h) = (w.min(MAX_EXTENT as usize) as u32, h.min(MAX_EXTENT as usize) as u32);
            self.items.push(Item::Rect(Rect { x, y, w, h, color, alpha }));
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
        let c = to_clip(r);
        if c.w > 0 && c.h > 0 && radius > 0.0 {
            self.items.push(Item::Blur(c, radius));
        }
    }
    pub(crate) fn clip(&mut self, r: Option<PxRect>) {
        self.items.push(Item::Clip(r.map(to_clip)));
    }
}

/// Widest / tallest item, so `x + w` stays well inside i32 in the backends.
const MAX_EXTENT: u32 = 1 << 24;

fn to_clip(r: PxRect) -> Clip {
    let ext = |v: f32| v.round().clamp(0.0, MAX_EXTENT as f32) as u32;
    Clip { x: r.x.round() as i32, y: r.y.round() as i32, w: ext(r.w), h: ext(r.h) }
}

/// An image the backends can sample: non-empty, with all its pixels.
pub(crate) fn image_ok(img: &ImageData) -> bool {
    img.width > 0 && img.height > 0 && img.rgba.len() >= img.width as usize * img.height as usize * 4
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
    // a palette colour on the default background: drawn readable
    let mut fg = match (fg_color, cell.bg, &theme.fg_palette) {
        (Color::Indexed(i @ 0..=15), Color::Default, Some(p)) if !cell.has(flags::INVERSE) => p[i as usize],
        _ => theme.resolve(fg_color, theme.foreground),
    };
    let mut bg = theme.resolve(cell.bg, theme.background);
    if cell.has(flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if selected {
        bg = theme.selection;
    }
    if theme.fg_palette.is_some() && !cell.has(flags::HIDDEN) {
        // readable on whatever is behind it: the selection (any text),
        // or a coloured background (palette text; an exact colour a
        // program chose is left alone)
        let palette_fg = !matches!(fg_color, Color::Rgb(..));
        let coloured_bg = !matches!(cell.bg, Color::Default) || cell.has(flags::INVERSE);
        if selected {
            fg = crate::theme::readable_on(fg, bg, crate::theme::MIN_TEXT_CONTRAST + 0.5);
        } else if coloured_bg && palette_fg {
            fg = crate::theme::readable_on(fg, bg, crate::theme::MIN_TEXT_CONTRAST);
        }
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

/// The columns cell `x` belongs to: a wide char and its spacer are one.
fn pair(cells: &[Cell], x: usize) -> (usize, usize) {
    if cells[x].has(flags::WIDE) && x + 1 < cells.len() {
        (x, x + 1)
    } else if cells[x].has(flags::WIDE_SPACER) && x > 0 && cells[x - 1].has(flags::WIDE) {
        (x - 1, x)
    } else {
        (x, x)
    }
}

/// Colours of cell `x`; both halves of a wide char get the same, so a
/// selection or search hit covers the whole glyph, not half of it.
fn colors(theme: &Theme, cells: &[Cell], x: usize, vy: isize, state: &DrawState) -> (u32, u32) {
    let (a, b) = pair(cells, x);
    let either = |r: &VisibleSelection| r.contains(a, vy as usize) || r.contains(b, vy as usize);
    let selected = vy >= 0 && state.selection.as_ref().is_some_and(either);
    let (mut fg, mut bg) = base_colors(theme, &cells[a], selected);
    if !selected && vy >= 0 {
        if let Some(h) = state.highlights.iter().rev().find(|h| either(&h.range)) {
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

/// Colour glyphs are drawn into a two-cell box; one in a single-width cell
/// gets half of it, so it doesn't spill over the next cell.
fn narrow_color(mut key: GlyphKey) -> GlyphKey {
    if key.color && key.split == 0 {
        key.split = 2;
    }
    key
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
    // the colours as drawn: readable text, selection and cursor
    let drawn = theme.for_drawing();
    let theme = &drawn;

    let rows = term.rows() as isize;
    let cols = term.cols();
    let top_line = term.top_visible_line() as isize;
    let total = term.total_lines() as isize;
    // smooth scrolling moves by a row or two; a wild value would only make
    // us walk (and overflow on) rows that can't be seen
    let max_shift = (rows as i32 + 1).saturating_mul(ch as i32);
    let shift = (state.y_shift.round() as i32).clamp(-max_shift, max_shift);
    let extra_above = ((shift.max(0) as usize).div_ceil(ch)) as isize;
    let extra_below = (((-shift).max(0) as usize).div_ceil(ch)) as isize;
    let row_range = -extra_above..rows + extra_below;
    let row_top = |vy: isize| py + (vy as i32) * ch as i32 + shift;

    let offset = term.display_offset();
    let cursor_vy = (term.cursor.y + offset) as isize;
    let show_cursor = term.modes.show_cursor && cursor_vy < rows && state.cursor_visible;
    // the cursor box: on the second half of a wide char it covers the
    // whole char, and on a wide char it's two cells wide
    let (cursor_x, cursor_w) = if show_cursor {
        let cells = &term.visible_row(cursor_vy as usize).cells;
        let x = term.cursor.x;
        let x = if x < cells.len() { pair(cells, x).0 } else { x };
        (x, if cells.get(x).is_some_and(|c| c.has(flags::WIDE)) { cw * 2 } else { cw })
    } else {
        (term.cursor.x, cw)
    };
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
            let (_, bg) = colors(theme, &row.cells, x, vy, state);
            let start = x;
            while x < n && colors(theme, &row.cells, x, vy, state).1 == bg {
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
        scene.rect(px + (cursor_x * cw) as i32, row_top(cursor_vy), cursor_w, ch, theme.cursor);
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
                colors(theme, &row.cells, x, vy, state).0
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
            let narrow = !cell.has(flags::WIDE);

            // a grapheme cluster: shape it as one unit
            if vibex_term::grapheme::is_cluster(cell.c) {
                let text = vibex_term::grapheme::text(cell.c);
                for mut g in fonts.shape_cluster(&text, style) {
                    if narrow && g.key.color {
                        g.key.split = g.key.split.max(1) * 2;
                        g.x /= 2;
                    }
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

            let mut key = fonts.resolve(cell.c, style);
            if narrow {
                key = narrow_color(key);
            }
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
        let w = cursor_w;
        match (state.focused, term.cursor_shape) {
            (true, CursorShape::Bar) => scene.rect(left, top, t, ch, color),
            (true, CursorShape::Underline) => scene.rect(left, top + ch as i32 - t as i32, w, t, color),
            _ => {
                scene.rect(left, top, w, 1, color);
                scene.rect(left, top + ch as i32 - 1, w, 1, color);
                scene.rect(left, top, 1, ch, color);
                scene.rect(left + w as i32 - 1, top, 1, ch, color);
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
        // a pane too narrow for the detail shows only the command
        let detail_w = if detail_w + 2 <= cells { detail_w } else { 0 };
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
            let left = px + (term.cursor.x * cw) as i32;
            let room = term.cols().saturating_sub(term.cursor.x);
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
        let left = px + (term.cursor.x * cw) as i32;
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
            // in f32: a bogus slice can't overflow the i32 maths
            let (xi, yi) = (x0.round() as i32, y0.round() as i32);
            let w = (x1.round() - x0.round()).max(0.0) as u32;
            let h = (y1.round() - y0.round()).max(0.0) as u32;
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
            scene.glyph(x, baseline_y, if w == 1 { narrow_color(key) } else { key }, fg);
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
/// `text` broken into lines of at most `cols` columns: at spaces where it
/// can, inside a word when one is longer than a whole line.
pub(crate) fn wrap_text(text: &str, cols: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let cols = cols.max(1);
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        let mut used = 0;
        for word in para.split(' ') {
            let ww = crate::text_width(word);
            let gap = usize::from(!line.is_empty());
            if used + gap + ww <= cols {
                if gap == 1 {
                    line.push(' ');
                }
                line.push_str(word);
                used += gap + ww;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
                used = 0;
            }
            // a word longer than a line: cut it
            for c in word.chars() {
                let w = c.width().unwrap_or(0);
                if used + w > cols {
                    out.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(c);
                used += w;
            }
        }
        out.push(line);
    }
    out
}

/// A notice in the top-right corner (toasts): a small sheet in the UI font,
/// a stripe in its colour on the left. Returns its bottom edge.
#[allow(clippy::too_many_arguments)]
pub fn overlay(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, y: i32, o: &Overlay, scale: f32) -> i32 {
    let s = scale.max(0.5);
    let k = crate::settings_view::ink(theme);
    let pad = (14.0 * s).round();
    let max_w = (scene.width as f32 - 28.0).min(520.0 * s);
    // a long message wraps to the box instead of running off the window
    let avg = (ui_width(fonts, cw, "abcdefghijklmnopqrstuvwxyz", false) / 26.0).max(1.0);
    let fit_cols = (((max_w - pad * 2.0 - 4.0 * s) / avg) as usize).max(8);
    let lines = wrap_text(&o.text, fit_cols);
    let widest = lines.iter().map(|l| ui_width(fonts, cw, l, false)).fold(0.0, f32::max);
    let line_h = (22.0 * s).round();
    let w = (widest + pad * 2.0 + 4.0 * s).min(max_w).round();
    let h = (lines.len().max(1) as f32 * line_h + 16.0 * s).round();
    let x = (scene.width as f32 - w - 14.0 * s).round();
    let r = PxRect { x, y: y as f32, w, h };
    crate::settings_view::sheet(scene, theme, r, s * 0.8);
    // the notice's colour, as a stripe
    crate::glass::fill(scene, PxRect { x: x + 6.0 * s, y: r.y + 8.0 * s, w: (3.0 * s).round(), h: h - 16.0 * s }, 1.5 * s, o.border, 255);
    for (i, line) in lines.iter().enumerate() {
        let mid = r.y + 8.0 * s + line_h * (i as f32 + 0.5);
        ui_text(scene, fonts, (x + pad + 4.0 * s).round() as i32, mid, cw, ch, line, k.text, false);
    }
    (r.y + h) as i32
}

#[allow(clippy::too_many_arguments)]
pub fn status_bar(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, bar: PxRect, pad_x: i32, s: &StatusLine, glass: bool) {
    let y = bar.y.round() as i32;
    let h = bar.h.round() as i32;
    let bar_bg = s.background.unwrap_or_else(|| blend(theme.background, theme.foreground, 16));
    if let Some(bg) = s.background {
        // a status line with its own colour (tmux's green): solid, edge to edge
        scene.frect(bar, bg, 255);
    } else if glass {
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

    let right_w: usize = s.right.iter().map(|seg| crate::text_width(&seg.text) * cw).sum();
    // the left side stops two cells short of the right side; a segment that
    // doesn't fit is shortened from the front ("…\long\path\end"), so the
    // end of a path stays readable and nothing is drawn over
    let limit = scene.width as i32 - pad_x - right_w as i32 - 2 * cw as i32;
    let mut x = pad_x;
    for seg in &s.left {
        let room = ((limit - x).max(0) as usize) / cw.max(1);
        if room == 0 {
            break;
        }
        if crate::text_width(&seg.text) <= room {
            x = draw(scene, fonts, x, seg);
        } else {
            let short = StatusSegment { text: keep_end(&seg.text, room), ..seg.clone() };
            draw(scene, fonts, x, &short);
            break;
        }
    }
    let mut rx = scene.width as i32 - pad_x - right_w as i32;
    for seg in &s.right {
        rx = draw(scene, fonts, rx, seg);
    }
}

/// `text` cut to `cols` columns by dropping from the front: a leading space
/// is kept, then "…", then as much of the end as fits.
pub(crate) fn keep_end(text: &str, cols: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if crate::text_width(text) <= cols {
        return text.to_string();
    }
    let lead = if text.starts_with(' ') { " " } else { "" };
    let budget = cols.saturating_sub(lead.len() + 1);
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for c in text.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        tail.push(c);
    }
    tail.reverse();
    if cols <= lead.len() {
        return lead[..cols].to_string();
    }
    format!("{lead}…{}", tail.into_iter().collect::<String>())
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
    if b < a {
        // both ends in one pixel: it's covered by the span's width
        scene.rect_a(x0.floor() as i32, y, 1, 1, color, (alpha as f32 * (x1 - x0)) as u8);
        return;
    }
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
            let baseline = (mid_y + fonts.ui_mid_offset()).round() as i32;
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

/// The classic title bar (QTerminal on Kali): a terminal glyph and the
/// window title centred on a dark strip, round window buttons (the close
/// one in the theme's blue), a menu bar, and slim tabs only when there's
/// more than one.
#[allow(clippy::too_many_arguments)]
pub fn classic_bar(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, bar: &TabBar, l: &TabBarLayout, scale: f32, glass: bool) {
    let s = scale.max(0.5);
    let width = scene.width as f32;
    let (th, mh, tabs_h) = crate::tabbar::classic_rows(ch, s, bar.tabs.len() > 1);
    let bar_bg = blend(theme.background, 0x000000, 46);
    let alpha = if glass { 120u8 } else { 255u8 };
    scene.frect(PxRect { x: 0.0, y: 0.0, w: width, h: th + mh }, bar_bg, alpha);
    let text = theme.strong_text();
    let dim = blend(bar_bg, theme.foreground, 175);

    // title row: glyph on the left, the title centred
    let mid = th / 2.0;
    crate::icons::draw(scene, "terminal", 18.0 * s, mid, 15.0 * s, dim);
    let title = if bar.title.is_empty() { "Vibex".to_string() } else { bar.title.clone() };
    let buttons_w = if bar.window_buttons { 3.0 * (16.0 + 8.0) * s + 10.0 * s } else { 0.0 };
    let room = (width - 2.0 * (buttons_w.max(40.0 * s) + 12.0 * s)).max(40.0 * s);
    let title = fit(fonts, cw, &title, false, room);
    let tw = ui_width(fonts, cw, &title, false);
    ui_text(scene, fonts, ((width - tw) / 2.0).round() as i32, mid, cw, ch, &title, text, false);
    if let Some(caption) = &bar.caption {
        let cw_ = ui_width(fonts, cw, caption, false);
        ui_text(scene, fonts, (width - buttons_w - cw_ - 14.0 * s).round() as i32, mid, cw, ch, caption, theme.palette[1], false);
    }

    // round window buttons
    for hit in [TabHit::Minimize, TabHit::Maximize, TabHit::Close] {
        let Some(r) = l.rect(hit) else { continue };
        let d = (16.0 * s).round();
        let c = PxRect { x: (r.x + (r.w - d) / 2.0).round(), y: (mid - d / 2.0).round(), w: d, h: d };
        let hot = bar.hover == Some(hit);
        let (fill, mark) = match hit {
            TabHit::Close => (if hot { blend(theme.palette[4], 0xffffff, 50) } else { theme.palette[4] }, 0x11131a),
            _ => (if hot { blend(bar_bg, theme.foreground, 70) } else { blend(bar_bg, theme.foreground, 34) }, text),
        };
        crate::glass::fill(scene, c, d / 2.0, fill, 255);
        // the glyphs always show (centred), the hovered button brightens
        let kind = match hit {
            TabHit::Close => Glyph::Close,
            TabHit::Minimize => Glyph::Minimize,
            _ if bar.maximized => Glyph::Restore,
            _ => Glyph::Maximize,
        };
        let mark = if hot || hit == TabHit::Close { mark } else { blend(fill, mark, 200) };
        caption_glyph(scene, kind, c, 7.0 * s, 1.0, mark);
    }

    // the menu bar
    let my = th + mh / 2.0;
    for (i, name) in crate::tabbar::MENUS.iter().enumerate() {
        let Some(r) = l.rect(TabHit::Menu(i)) else { continue };
        let open = bar.menu_open == Some(i);
        if open || bar.hover == Some(TabHit::Menu(i)) {
            crate::glass::fill(scene, r, 4.0 * s, theme.foreground, if open { 34 } else { 20 });
        }
        let w = ui_width(fonts, cw, name, false);
        ui_text(scene, fonts, (r.x + (r.w - w) / 2.0).round() as i32, my, cw, ch, name, text, false);
    }

    // tabs, when there's more than one
    if tabs_h > 0.0 {
        let y = th + mh;
        scene.frect(PxRect { x: 0.0, y, w: width, h: tabs_h }, blend(theme.background, 0x000000, 24), alpha);
        for (i, tab) in bar.tabs.iter().enumerate() {
            let Some(r) = l.rect(TabHit::Tab(i)) else { continue };
            let hovered = bar.hover == Some(TabHit::Tab(i)) || bar.hover == Some(TabHit::CloseTab(i));
            if tab.active {
                scene.frect(PxRect { x: r.x, y: r.y, w: r.w, h: r.h }, theme.background, 255);
                scene.frect(PxRect { x: r.x, y: r.y + r.h - (2.0 * s).round(), w: r.w, h: (2.0 * s).round() }, theme.palette[4], 255);
            } else if hovered {
                scene.frect(r, blend(theme.background, theme.foreground, 14), 255);
            }
            if let Some(c) = tab.color {
                scene.frect(PxRect { x: r.x, y: r.y, w: r.w, h: (3.0 * s).round() }, c, 255);
            }
            let fg = if tab.active { text } else { dim };
            let close = l.rect(TabHit::CloseTab(i));
            let right = close.map_or(r.x + r.w - 8.0 * s, |c| c.x) - 4.0 * s;
            let label = fit(fonts, cw, &tab.title, tab.active, (right - r.x - 12.0 * s).max(0.0));
            let lw = ui_width(fonts, cw, &label, tab.active);
            let lx = (r.x + ((right - r.x) - lw).max(12.0 * s) / 2.0).round();
            ui_text(scene, fonts, lx as i32, r.y + r.h / 2.0, cw, ch, &label, fg, tab.active);
            if let Some(c) = close {
                if hovered || tab.active {
                    draw_thin_x(scene, c, (4.0 * s).round(), dim);
                }
            }
            if i + 1 < bar.tabs.len() && !tab.active && !bar.tabs[i + 1].active {
                scene.frect(
                    PxRect { x: (r.x + r.w - 1.0).round(), y: r.y + 7.0 * s, w: 1.0, h: r.h - 14.0 * s },
                    blend(theme.background, theme.foreground, 40),
                    255,
                );
            }
        }
        if let Some(r) = l.rect(TabHit::NewTab) {
            if bar.hover == Some(TabHit::NewTab) {
                scene.frect(r, blend(theme.background, theme.foreground, 18), 255);
            }
            let (cx, cy, k) = (r.x + r.w / 2.0, r.y + r.h / 2.0, (5.0 * s).round());
            scene.frect(PxRect { x: cx - k, y: cy.round(), w: 2.0 * k, h: 1.0 }, dim, 255);
            scene.frect(PxRect { x: cx.round(), y: cy - k, w: 1.0, h: 2.0 * k }, dim, 255);
        }
    }
}

/// The other titled bars: the Windows console, GNOME Terminal's header
/// bar, macOS Terminal and tmux's plain one. Each takes its colours from
/// the system it comes from (light or dark to suit the theme), not from the
/// terminal's palette.
#[allow(clippy::too_many_arguments)]
pub fn titled_bar(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, bar: &TabBar, l: &TabBarLayout, scale: f32) {
    use crate::tabbar::BarKind;
    let s = scale.max(0.5);
    let width = scene.width as f32;
    let (th, _, tabs_h) = crate::tabbar::kind_rows(bar.kind, ch, s, bar.tabs.len() > 1);
    let light = theme.is_light();
    // (bar, title text, dimmer text)
    let (bar_bg, text, dim) = match (bar.kind, light) {
        (BarKind::Gnome, false) => (0x303030, 0xffffff, 0xb0b0b0),
        (BarKind::Gnome, true) => (0xebebeb, 0x2e2e2e, 0x6e6e6e),
        (BarKind::Mac, false) => (0x383838, 0xd0d0d0, 0x9a9a9a),
        (BarKind::Mac, true) => (0xe8e6e8, 0x4d4d4d, 0x8a8a8a),
        (_, false) => (0x202020, 0xffffff, 0xa0a0a0),
        (_, true) => (0xf3f3f3, 0x1a1a1a, 0x6a6a6a),
    };
    scene.frect(PxRect { x: 0.0, y: 0.0, w: width, h: th }, bar_bg, 255);
    // a hairline under the title row
    let line = if light { blend(bar_bg, 0x000000, 26) } else { blend(bar_bg, 0x000000, 90) };
    scene.rect(0, (th - 1.0) as i32, scene.width, 1, line);
    let mid = th / 2.0;
    let title = if bar.title.is_empty() { "Vibex".to_string() } else { bar.title.clone() };

    match bar.kind {
        BarKind::Windows | BarKind::Tmux => {
            // icon and title on the left, as the console window has them
            crate::icons::draw(scene, "terminal", 18.0 * s, mid, 15.0 * s, text);
            let room = (width - 3.0 * (crate::tabbar::BUTTON_W * s) - 52.0 * s).max(40.0 * s);
            let t = fit(fonts, cw, &title, false, room);
            ui_text(scene, fonts, (36.0 * s).round() as i32, mid, cw, ch, &t, text, false);
            windows_buttons(scene, theme, bar, l, s, bar_bg, text);
        }
        BarKind::Gnome => {
            // a centred bold title, flat round buttons, + on the left
            let room = (width - 2.0 * (3.0 * 36.0 * s + 24.0 * s)).max(40.0 * s);
            let t = fit(fonts, cw, &title, true, room);
            let tw = ui_width(fonts, cw, &t, true);
            ui_text(scene, fonts, ((width - tw) / 2.0).round() as i32, mid, cw, ch, &t, text, true);
            let btn = if light { 0xd8d8d8 } else { 0x454545 };
            let btn_hot = if light { 0xc6c6c6 } else { 0x575757 };
            for hit in [TabHit::Minimize, TabHit::Maximize, TabHit::Close] {
                let Some(r) = l.rect(hit) else { continue };
                let d = (24.0 * s).round();
                let c = PxRect { x: (r.x + (r.w - d) / 2.0).round(), y: (mid - d / 2.0).round(), w: d, h: d };
                let hot = bar.hover == Some(hit);
                crate::glass::fill(scene, c, d / 2.0, if hot { btn_hot } else { btn }, 255);
                // Adwaita's symbolic glyphs: 8px, 1.5px lines, centred
                let kind = match hit {
                    TabHit::Minimize => Glyph::Minimize,
                    TabHit::Maximize if bar.maximized => Glyph::Restore,
                    TabHit::Maximize => Glyph::Maximize,
                    _ => Glyph::Close,
                };
                caption_glyph(scene, kind, c, 8.0 * s, (1.5 * s).round().max(1.0), text);
            }
            if let Some(r) = l.rect(TabHit::NewTab) {
                if bar.hover == Some(TabHit::NewTab) {
                    crate::glass::fill(scene, r, 6.0 * s, btn_hot, 255);
                }
                caption_glyph(scene, Glyph::Plus, r, 12.0 * s, (1.5 * s).round().max(1.0), text);
            }
        }
        _ => {
            // macOS: traffic lights on the left, a centred title
            let room = (width - 2.0 * 80.0 * s).max(40.0 * s);
            let t = fit(fonts, cw, &title, false, room);
            let tw = ui_width(fonts, cw, &t, false);
            ui_text(scene, fonts, ((width - tw) / 2.0).round() as i32, mid, cw, ch, &t, text, false);
            let any_hot = matches!(bar.hover, Some(TabHit::Close | TabHit::Minimize | TabHit::Maximize));
            for (hit, fill) in [(TabHit::Close, 0xff5f57), (TabHit::Minimize, 0xfebc2e), (TabHit::Maximize, 0x28c840)] {
                let Some(r) = l.rect(hit) else { continue };
                let d = (12.0 * s).round();
                let c = PxRect { x: (r.x + (r.w - d) / 2.0).round(), y: (mid - d / 2.0).round(), w: d, h: d };
                crate::glass::fill(scene, c, d / 2.0, blend(fill, 0x000000, 40), 255);
                let inner = PxRect { x: c.x + 0.5, y: c.y + 0.5, w: d - 1.0, h: d - 1.0 };
                crate::glass::fill(scene, inner, (d - 1.0) / 2.0, fill, 255);
                // the glyphs show while the pointer is over any of them
                if any_hot {
                    let mark = blend(fill, 0x000000, 150);
                    let kind = match hit {
                        TabHit::Close => Glyph::Close,
                        TabHit::Minimize => Glyph::Minimize,
                        _ => Glyph::Plus,
                    };
                    caption_glyph(scene, kind, c, 6.0 * s, 1.0, mark);
                }
            }
        }
    }

    // the tab strip, when there's more than one tab
    if tabs_h > 0.0 {
        let y = th;
        let strip = if light { blend(bar_bg, 0x000000, 16) } else { blend(bar_bg, 0x000000, 60) };
        scene.frect(PxRect { x: 0.0, y, w: width, h: tabs_h }, strip, 255);
        let active_bg = if bar.kind == BarKind::Mac { bar_bg } else { theme.background };
        let active_text = if bar.kind == BarKind::Mac { text } else { theme.strong_text() };
        for (i, tab) in bar.tabs.iter().enumerate() {
            let Some(r) = l.rect(TabHit::Tab(i)) else { continue };
            let hovered = bar.hover == Some(TabHit::Tab(i)) || bar.hover == Some(TabHit::CloseTab(i));
            if tab.active {
                scene.frect(r, active_bg, 255);
                if bar.kind == BarKind::Gnome {
                    // libadwaita marks the open tab with an accent line
                    scene.frect(PxRect { x: r.x + 8.0 * s, y: r.y + r.h - (3.0 * s).round(), w: r.w - 16.0 * s, h: (3.0 * s).round() }, 0x3584e4, 255);
                }
            } else if hovered {
                scene.frect(r, blend(strip, text, 18), 255);
            }
            if let Some(c) = tab.color {
                scene.frect(PxRect { x: r.x, y: r.y, w: r.w, h: (3.0 * s).round() }, c, 255);
            }
            let fg = if tab.active { active_text } else { dim };
            let close = l.rect(TabHit::CloseTab(i));
            let right = close.map_or(r.x + r.w - 8.0 * s, |c| c.x) - 4.0 * s;
            let label = fit(fonts, cw, &tab.title, tab.active, (right - r.x - 12.0 * s).max(0.0));
            let lw = ui_width(fonts, cw, &label, tab.active);
            let lx = (r.x + ((right - r.x) - lw).max(12.0 * s) / 2.0).round();
            ui_text(scene, fonts, lx as i32, r.y + r.h / 2.0, cw, ch, &label, fg, tab.active);
            if let Some(c) = close {
                if hovered || tab.active {
                    draw_thin_x(scene, c, (4.0 * s).round(), fg);
                }
            }
            // a divider between two tabs that aren't the open one
            if i + 1 < bar.tabs.len() && !tab.active && !bar.tabs[i + 1].active {
                scene.frect(PxRect { x: (r.x + r.w - 1.0).round(), y: r.y + 7.0 * s, w: 1.0, h: r.h - 14.0 * s }, blend(strip, text, 40), 255);
            }
        }
        if let Some(r) = l.rect(TabHit::NewTab).filter(|r| r.y >= y) {
            if bar.hover == Some(TabHit::NewTab) {
                scene.frect(r, blend(strip, text, 18), 255);
            }
            let (cx, cy, k) = (r.x + r.w / 2.0, r.y + r.h / 2.0, (5.0 * s).round());
            scene.frect(PxRect { x: cx - k, y: cy.round(), w: 2.0 * k, h: 1.0 }, dim, 255);
            scene.frect(PxRect { x: cx.round(), y: cy - k, w: 1.0, h: 2.0 * k }, dim, 255);
        }
    }
}

/// Like `fit`, but the ellipsis goes in the middle, so the end (a file name)
/// stays: `C:\Users\me\…\settings.lua`.
pub(crate) fn fit_middle(fonts: &mut FontSet, cw: usize, text: &str, bold: bool, max: f32) -> String {
    if ui_width(fonts, cw, text, bold) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // keep about a third from the start, the rest from the end
    let mut keep = chars.len();
    while keep > 1 {
        keep -= 1;
        let head = (keep / 3).max(1);
        let tail = keep - head;
        let t: String = chars[..head].iter().chain(std::iter::once(&'…')).chain(chars[chars.len() - tail..].iter()).collect();
        if ui_width(fonts, cw, &t, bold) <= max {
            return t;
        }
    }
    String::from("…")
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
    let text_on = theme.strong_text();
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
        // on a coloured tab: black or white, whichever reads on that colour
        let fg = match tab.color {
            Some(c) => Theme::text_on(blend(if tab.active { theme.background } else { bar_bg }, c, if tab.active { 70 } else { 120 })),
            None if tab.active => text_on,
            None => dim,
        };
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
            let hot = bar.hover == Some(TabHit::CloseTab(i));
            if hot {
                let bg = if tab.active { blend(theme.background, theme.foreground, 26) } else { blend(hover_bg, theme.foreground, 18) };
                crate::glass::fill(scene, c, 4.0 * s, bg, 255);
            }
            // quiet until the tab (or the ✕) is under the pointer
            let ink = if hot || hovered || tab.active { fg } else { blend(bar_bg, fg, 150) };
            draw_thin_x(scene, c, close_half, ink);
        }
    }

    for hit in [TabHit::NewTab, TabHit::NewTabMenu] {
        let Some(r) = l.rect(hit) else { continue };
        let hot = bar.hover == Some(hit);
        if hot {
            // a rounded pill, inset from the strip's edges
            let ph = (r.h - 4.0 * s).min(30.0 * s).round();
            let pill = PxRect { x: r.x + 2.0 * s, y: (r.y + (r.h - ph) / 2.0).round(), w: r.w - 4.0 * s, h: ph };
            crate::glass::fill(scene, pill, 6.0 * s, hover_bg, 255);
        }
        let ink = if hot { text_on } else { blend(bar_bg, text_on, 215) };
        if hit == TabHit::NewTab {
            caption_glyph(scene, Glyph::Plus, r, 12.0 * s, s.floor().max(1.0), ink);
        } else {
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            crate::icons::draw(scene, "chevron-down", cx, cy, 15.0 * s, ink);
        }
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

    windows_buttons(scene, theme, bar, l, s, bar_bg, text_on);
}

/// Windows 11's caption buttons: 10px glyphs (scaled), a soft fill on
/// hover, red under the ✕.
pub(crate) fn windows_buttons(scene: &mut Scene, theme: &Theme, bar: &TabBar, l: &TabBarLayout, s: f32, bar_bg: u32, fg: u32) {
    let light = theme.is_light();
    for hit in [TabHit::Minimize, TabHit::Maximize, TabHit::Close] {
        let Some(r) = l.rect(hit) else { continue };
        let hovered = bar.hover == Some(hit);
        let mut mark = blend(bar_bg, fg, 230);
        if hovered {
            if hit == TabHit::Close {
                scene.frect(r, 0xc42b1c, 255);
                mark = 0xffffff;
            } else {
                scene.frect(r, if light { 0x000000 } else { 0xffffff }, if light { 16 } else { 20 });
                mark = fg;
            }
        }
        let kind = match hit {
            TabHit::Minimize => Glyph::Minimize,
            TabHit::Maximize if bar.maximized => Glyph::Restore,
            TabHit::Maximize => Glyph::Maximize,
            _ => Glyph::Close,
        };
        caption_glyph(scene, kind, r, 10.0 * s, s.floor().max(1.0), mark);
    }
}

/// A window-button glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
    Plus,
}

/// `kind` drawn `size` px square, exactly centred in `r` (the glyph box is
/// snapped to whole pixels with the same parity as the button, so it never
/// sits half a pixel off), lines `stroke` px: crisp straight edges, an
/// anti-aliased ✕.
pub(crate) fn caption_glyph(scene: &mut Scene, kind: Glyph, r: PxRect, size: f32, stroke: f32, c: u32) {
    let (rw, rh) = (r.w.round() as i32, r.h.round() as i32);
    let mut n = size.round().max(4.0) as i32;
    if (rw - n) % 2 != 0 {
        n -= 1;
    }
    let t = stroke.round().max(1.0) as i32;
    let x0 = r.x.round() as i32 + (rw - n) / 2;
    // the box's vertical centre on the button's (the bar's parity decides)
    let y0 = r.y.round() as i32 + (rh - n) / 2;
    let rect = |scene: &mut Scene, x: i32, y: i32, w: i32, h: i32| scene.frect(PxRect { x: x as f32, y: y as f32, w: w as f32, h: h as f32 }, c, 255);
    let frame = |scene: &mut Scene, x: i32, y: i32, w: i32, h: i32| {
        rect(scene, x, y, w, t);
        rect(scene, x, y + h - t, w, t);
        rect(scene, x, y, t, h);
        rect(scene, x + w - t, y, t, h);
    };
    match kind {
        Glyph::Minimize => rect(scene, x0, r.y.round() as i32 + (rh - t) / 2, n, t),
        Glyph::Maximize => frame(scene, x0, y0, n, n),
        Glyph::Restore => {
            // the window behind: its top and right edges; the front one whole
            let o = (n as f32 * 0.2).round().max(2.0) as i32;
            rect(scene, x0 + o, y0, n - o, t);
            rect(scene, x0 + n - t, y0, t, n - o);
            frame(scene, x0, y0 + o, n - o, n - o);
        }
        Glyph::Close | Glyph::Plus => {
            let (bx, by, nf, half) = (x0 as f32, y0 as f32, n as f32, t as f32 / 2.0);
            let segs: [((f32, f32), (f32, f32)); 2] = if kind == Glyph::Close {
                [((bx + half, by + half), (bx + nf - half, by + nf - half)), ((bx + nf - half, by + half), (bx + half, by + nf - half))]
            } else {
                let m = nf / 2.0;
                [((bx, by + m), (bx + nf, by + m)), ((bx + m, by), (bx + m, by + nf))]
            };
            for py in (y0 - 1)..=(y0 + n) {
                for px in (x0 - 1)..=(x0 + n) {
                    let p = (px as f32 + 0.5, py as f32 + 0.5);
                    let d = segs.iter().map(|&(a, b)| point_seg(p, a, b)).fold(f32::MAX, f32::min);
                    let cov = (half + 0.5 - d).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        hspan(scene, px as f32, px as f32 + 1.0, py, c, (cov * 255.0).round() as u8);
                    }
                }
            }
        }
    }
}

fn point_seg(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 { ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((wx - t * vx).powi(2) + (wy - t * vy).powi(2)).sqrt()
}

/// A thin ✕ centred in `r`, arms `half` long.
pub(crate) fn draw_thin_x(scene: &mut Scene, r: PxRect, half: f32, c: u32) {
    caption_glyph(scene, Glyph::Close, r, 2.0 * half, 1.0, c);
}

pub(crate) fn outline(scene: &mut Scene, r: PxRect, c: u32) {
    scene.frect(PxRect { h: 1.0, ..r }, c, 255);
    scene.frect(PxRect { y: r.y + r.h - 1.0, h: 1.0, ..r }, c, 255);
    scene.frect(PxRect { w: 1.0, ..r }, c, 255);
    scene.frect(PxRect { x: r.x + r.w - 1.0, w: 1.0, ..r }, c, 255);
}

/// Key chips for a shortcut hint ("Ctrl+Shift+P", "Leader U"), drawn right
/// to left ending at `right`; returns their left edge.
#[allow(clippy::too_many_arguments)]
pub(crate) fn key_chips(scene: &mut Scene, fonts: &mut FontSet, cw: usize, ch: usize, hint: &str, right: f32, mid: f32, s: f32, bg: u32, fg: u32) -> f32 {
    let mut x = right;
    let h = (22.0 * s).round();
    let parts: Vec<&str> = hint.split(['+', ' ']).filter(|p| !p.is_empty()).collect();
    for part in parts.iter().rev() {
        let tw = ui_width(fonts, cw, part, false);
        let w = (tw + 12.0 * s).max(h);
        let r = PxRect { x: (x - w).round(), y: (mid - h / 2.0).round(), w: w.round(), h };
        crate::glass::fill(scene, r, 4.0 * s, bg, 255);
        ui_text(scene, fonts, (r.x + (r.w - tw) / 2.0).round() as i32, mid, cw, ch, part, fg, false);
        x = r.x - 4.0 * s;
    }
    x
}

/// The command palette: a sheet under the tab bar with a search line and
/// the matching commands, their shortcuts as key chips.
#[allow(clippy::too_many_arguments)]
pub fn palette(scene: &mut Scene, fonts: &mut FontSet, theme: &Theme, cw: usize, ch: usize, top: f32, p: &PaletteView, scale: f32) {
    let s = scale.max(0.5);
    let k = crate::settings_view::ink(theme);
    let max_rows = 10usize;
    let w = (640.0 * s).min(scene.width as f32 - 32.0).max((320.0 * s).min(scene.width as f32 - 16.0)).round();
    let x = ((scene.width as f32 - w) / 2.0).round();
    let row_h = (34.0 * s).round();
    let query_h = (48.0 * s).round();
    let shown = p.items.len().min(max_rows).max(1);
    let h = (query_h + 1.0 + 12.0 * s + row_h * shown as f32).round();
    let y = (top + 14.0 * s).round();
    crate::settings_view::sheet(scene, theme, PxRect { x, y, w, h }, s);

    // the search line
    let qmid = y + query_h / 2.0;
    crate::icons::draw(scene, "search", x + 24.0 * s, qmid, 16.0 * s, k.dim);
    let tx = (x + 44.0 * s).round() as i32;
    let room = w - 64.0 * s;
    let end = if p.query.is_empty() {
        // the caret just before the placeholder, not over its first letter
        ui_text(scene, fonts, tx + (4.0 * s).round() as i32, qmid, cw, ch, "Type a command", k.dim, false);
        tx
    } else {
        let q = fit(fonts, cw, &p.query, false, room);
        ui_text(scene, fonts, tx, qmid, cw, ch, &q, k.text, false)
    };
    scene.frect(PxRect { x: end as f32 + 1.0, y: (qmid - 9.0 * s).round(), w: (1.0 * s).round().max(1.0), h: (18.0 * s).round() }, k.accent, 255);
    scene.frect(PxRect { x: x + 1.0, y: y + query_h, w: w - 2.0, h: 1.0 }, k.rule, 255);

    // keep the selection in view
    let first = p.selected.saturating_sub(max_rows - 1);
    let mut ry = y + query_h + 1.0 + 6.0 * s;
    for i in (first..p.items.len()).take(max_rows) {
        let (label, hint) = &p.items[i];
        let r = PxRect { x: x + 6.0 * s, y: ry, w: w - 12.0 * s, h: row_h };
        let selected = i == p.selected;
        if selected {
            crate::glass::fill(scene, r, 6.0 * s, k.selected, 255);
        }
        let mid = r.y + r.h / 2.0;
        let left = if hint.is_empty() {
            r.x + r.w
        } else {
            key_chips(scene, fonts, cw, ch, hint, r.x + r.w - 8.0 * s, mid, s, if selected { k.field_hover } else { k.field }, k.dim)
        };
        let lbl = fit(fonts, cw, label, false, left - r.x - 28.0 * s);
        ui_text(scene, fonts, (r.x + 12.0 * s).round() as i32, mid, cw, ch, &lbl, if selected { k.text } else { blend(k.surface, k.text, 220) }, false);
        ry += row_h;
    }
    if p.items.is_empty() {
        ui_text(scene, fonts, (x + 18.0 * s).round() as i32, ry + row_h / 2.0, cw, ch, "No matching commands", k.dim, false);
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
