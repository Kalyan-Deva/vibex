//! Font loading, glyph lookup, shaping (ligatures) and rasterising,
//! including COLR colour glyphs (emoji). Outlines are parsed by ttf-parser
//! and rasterised by `raster` (no third-party rasteriser).

use crate::raster;
use rustybuzz::ttf_parser::GlyphId;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Style {
    pub fn new(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => Style::Regular,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (true, true) => Style::BoldItalic,
        }
    }
    fn is_bold(self) -> bool {
        matches!(self, Style::Bold | Style::BoldItalic)
    }
}

/// Identifies one rasterised glyph.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    pub face: u16,
    pub id: u16,
    /// No real bold face: draw twice, 1px apart.
    pub fake_bold: bool,
    /// Multi-colour (COLR) glyph.
    pub color: bool,
    /// Colour glyph drawn at 1/split of the two-cell width (an emoji
    /// sequence the font doesn't ligate, e.g. some families). 0 = normal.
    pub split: u8,
}

pub enum Bitmap {
    /// 8-bit coverage.
    Mono(Vec<u8>),
    /// Premultiplied RGBA.
    Color(Vec<[u8; 4]>),
}

/// A rasterised glyph. `xmin`/`ymin` are offsets from the pen position on
/// the baseline (ymin = bottom of the bitmap, positive up).
pub struct Glyph {
    pub xmin: i32,
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub bitmap: Bitmap,
    pub fake_bold: bool,
}

impl Glyph {
    fn empty(fake_bold: bool) -> Self {
        Glyph { xmin: 0, ymin: 0, width: 0, height: 0, bitmap: Bitmap::Mono(Vec::new()), fake_bold }
    }
}

/// Font file paths for one family.
#[derive(Clone, Debug, PartialEq)]
pub struct FontPaths {
    pub regular: String,
    pub bold: Option<String>,
    pub italic: Option<String>,
    pub bold_italic: Option<String>,
    /// Tried in order for characters the main font lacks.
    pub fallbacks: Vec<String>,
}

struct Face {
    face: rustybuzz::Face<'static>,
    has_color: bool,
}

impl Face {
    fn gid(&self, c: char) -> u16 {
        self.face.glyph_index(c).map_or(0, |g| g.0)
    }
    fn scale(&self, px: f32) -> f32 {
        px / self.face.units_per_em() as f32
    }
}

fn load_face(path: &str) -> Option<Face> {
    // Faces live for the whole program; leaking keeps `Face<'static>` simple.
    let data: &'static [u8] = Box::leak(std::fs::read(path).ok()?.into_boxed_slice());
    let face = rustybuzz::Face::from_slice(data, 0)?;
    let has_color = face.tables().colr.is_some();
    Some(Face { face, has_color })
}

/// One glyph of a shaped grapheme cluster, offset from the cell's pen
/// position in pixels (y up).
#[derive(Clone, Copy, Debug)]
pub struct ClusterGlyph {
    pub key: GlyphKey,
    pub x: i32,
    pub y: i32,
}

/// A shaped glyph within a run: which char (cluster) it belongs to.
#[derive(Clone, Copy, Debug)]
pub struct Shaped {
    pub cluster: u32,
    pub id: u16,
    pub x_offset: i32,
}

const REGULAR: u16 = 0;
const BOLD: u16 = 1;
const ITALIC: u16 = 2;
const BOLD_ITALIC: u16 = 3;
const FIRST_FALLBACK: u16 = 4;

pub struct FontSet {
    /// Slots 0-3: regular, bold, italic, bold-italic (None if absent);
    /// 4..: fallbacks.
    faces: Vec<Option<Face>>,
    size_px: f32,
    glyphs: HashMap<GlyphKey, Glyph>,
    lookups: HashMap<(char, Style), GlyphKey>,
    shaped: HashMap<(String, Style), Vec<Shaped>>,
    clusters: HashMap<(String, Style), Vec<ClusterGlyph>>,
    pub ligatures: bool,
    pub cell_width: usize,
    pub cell_height: usize,
    /// Distance from the cell top to the baseline.
    pub baseline: i32,
    pub underline_y: i32,
    pub line_thickness: usize,
    /// Proportional UI font (Segoe UI on Windows) for tab titles:
    /// regular and semibold slots in `faces`.
    ui: Option<(u16, u16)>,
}

/// A glyph of a UI-font text run, at `x` pixels from the run start.
#[derive(Clone, Copy, Debug)]
pub struct UiGlyph {
    pub key: GlyphKey,
    pub x: i32,
}

/// UI text is drawn a bit smaller than the terminal font.
const UI_SCALE: f32 = 0.8;

fn ui_font_paths() -> Option<(String, String)> {
    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let dir = Path::new(&windir).join("Fonts");
        let regular = dir.join("segoeui.ttf");
        let semibold = [dir.join("seguisb.ttf"), dir.join("segoeuib.ttf")].into_iter().find(|p| p.is_file())?;
        if regular.is_file() {
            return Some((regular.to_string_lossy().into_owned(), semibold.to_string_lossy().into_owned()));
        }
    }
    None
}

impl FontSet {
    pub fn load(paths: &FontPaths, size_px: f32) -> Result<Self, String> {
        let regular = load_face(&paths.regular).ok_or_else(|| format!("cannot load font {}", paths.regular))?;
        let opt = |p: &Option<String>| p.as_deref().filter(|p| Path::new(p).exists()).and_then(load_face);
        let mut faces = vec![Some(regular), opt(&paths.bold), opt(&paths.italic), opt(&paths.bold_italic)];
        faces.extend(paths.fallbacks.iter().map(|p| load_face(p)).filter(Option::is_some));
        let mut ui = None;
        if let Some((r, b)) = ui_font_paths() {
            if let (Some(rf), Some(bf)) = (load_face(&r), load_face(&b)) {
                let at = faces.len() as u16;
                faces.push(Some(rf));
                faces.push(Some(bf));
                ui = Some((at, at + 1));
            }
        }
        let mut set = Self {
            faces,
            size_px,
            glyphs: HashMap::new(),
            lookups: HashMap::new(),
            shaped: HashMap::new(),
            clusters: HashMap::new(),
            ligatures: true,
            cell_width: 1,
            cell_height: 1,
            baseline: 1,
            underline_y: 1,
            line_thickness: 1,
            ui,
        };
        set.set_size(size_px);
        Ok(set)
    }

    pub fn size_px(&self) -> f32 {
        self.size_px
    }

    fn regular(&self) -> &Face {
        self.faces[0].as_ref().expect("regular face")
    }

    pub fn set_size(&mut self, size_px: f32) {
        self.size_px = size_px.clamp(6.0, 200.0);
        self.glyphs.clear();
        self.shaped.clear();
        self.clusters.clear();
        let f = self.regular();
        let s = f.scale(self.size_px);
        let ascent = f.face.ascender() as f32 * s;
        let descent = f.face.descender() as f32 * s; // negative
        let gap = f.face.line_gap() as f32 * s;
        let m = f.face.glyph_index('M').unwrap_or(GlyphId(0));
        let upem = f.face.units_per_em() as f32;
        let advance = f.face.glyph_hor_advance(m).map_or(upem / 2.0, |a| a as f32) * s;
        self.cell_width = advance.round().max(1.0) as usize;
        self.cell_height = (ascent - descent + gap).ceil().max(1.0) as usize;
        let extra = self.cell_height as f32 - (ascent - descent);
        self.baseline = (ascent + extra / 2.0).round() as i32;
        self.line_thickness = (self.size_px / 14.0).round().max(1.0) as usize;
        let below = ((-descent) * 0.4).round() as i32;
        self.underline_y = (self.baseline + below.max(1)).min(self.cell_height as i32 - self.line_thickness as i32);
    }

    /// Changes whenever cached glyph rasters are dropped (GPU atlas resets).
    pub fn generation(&self) -> (u32, usize) {
        (self.size_px.to_bits(), self.cell_width)
    }

    fn styled_slot(&self, style: Style) -> Option<u16> {
        let slot = match style {
            Style::Regular => REGULAR,
            Style::Bold => BOLD,
            Style::Italic => ITALIC,
            Style::BoldItalic => {
                if self.faces[BOLD_ITALIC as usize].is_some() {
                    BOLD_ITALIC
                } else if self.faces[BOLD as usize].is_some() {
                    BOLD
                } else {
                    ITALIC
                }
            }
        };
        self.faces[slot as usize].is_some().then_some(slot)
    }

    /// The face used for shaping runs of this style, and whether bold must
    /// be synthesised.
    fn run_face(&self, style: Style) -> (u16, bool) {
        match self.styled_slot(style) {
            Some(slot) => (slot, style.is_bold() && slot != BOLD && slot != BOLD_ITALIC),
            None => (REGULAR, style.is_bold()),
        }
    }

    /// Glyph for one character, walking the fallback chain.
    pub fn resolve(&mut self, c: char, style: Style) -> GlyphKey {
        if let Some(k) = self.lookups.get(&(c, style)) {
            return *k;
        }
        let key = self.resolve_uncached(c, style);
        self.lookups.insert((c, style), key);
        key
    }

    fn resolve_uncached(&self, c: char, style: Style) -> GlyphKey {
        let make = |face: u16, id: u16, fake_bold: bool, color: bool| GlyphKey { face, id, fake_bold, color, split: 0 };
        // emoji presentation: prefer a colour face that has it
        if is_emoji(c) {
            for (i, f) in self.faces.iter().enumerate().skip(FIRST_FALLBACK as usize) {
                if let Some(f) = f.as_ref().filter(|f| f.has_color) {
                    let id = f.gid(c);
                    if id != 0 && f.face.is_color_glyph(GlyphId(id)) {
                        return make(i as u16, id, false, true);
                    }
                }
            }
        }
        let (slot, fake) = self.run_face(style);
        let id = self.faces[slot as usize].as_ref().unwrap().gid(c);
        if id != 0 {
            return make(slot, id, fake, false);
        }
        for (i, f) in self.faces.iter().enumerate() {
            let Some(f) = f else { continue };
            let id = f.gid(c);
            if id != 0 {
                let color = f.has_color && f.face.is_color_glyph(GlyphId(id));
                let real_bold = i as u16 == BOLD || i as u16 == BOLD_ITALIC;
                return make(i as u16, id, style.is_bold() && !real_bold && !color, color);
            }
        }
        make(REGULAR, 0, false, false)
    }

    /// Whether `c` is drawn from the run face for `style` (so it can take
    /// part in a shaped ligature run).
    pub fn in_run_face(&mut self, c: char, style: Style) -> bool {
        let k = self.resolve(c, style);
        !k.color && k.face == self.run_face(style).0
    }

    /// Shape a run (all characters in the run face) with the font's
    /// default OpenType features (calt, liga). Cached.
    pub fn shape(&mut self, text: &str, style: Style) -> (u16, bool, &[Shaped]) {
        let (slot, fake) = self.run_face(style);
        let key = (text.to_string(), style);
        if !self.shaped.contains_key(&key) {
            if self.shaped.len() > 8192 {
                self.shaped.clear();
            }
            let f = self.faces[slot as usize].as_ref().unwrap();
            let mut buf = rustybuzz::UnicodeBuffer::new();
            buf.push_str(text);
            let out = rustybuzz::shape(&f.face, &[], buf);
            let scale = f.scale(self.size_px);
            let shaped = out
                .glyph_infos()
                .iter()
                .zip(out.glyph_positions())
                .map(|(info, pos)| Shaped { cluster: info.cluster, id: info.glyph_id as u16, x_offset: (pos.x_offset as f32 * scale).round() as i32 })
                .collect();
            self.shaped.insert(key.clone(), shaped);
        }
        (slot, fake, &self.shaped[&key])
    }

    /// Shape one grapheme cluster (base + marks, ZWJ emoji, flag, ...) as a
    /// unit. Glyph positions are relative to the cell's pen position
    /// (x right, y up). Cached.
    pub fn shape_cluster(&mut self, text: &str, style: Style) -> Vec<ClusterGlyph> {
        let key = (text.to_string(), style);
        if let Some(v) = self.clusters.get(&key) {
            return v.clone();
        }
        let joiners = |c: &char| matches!(*c, '\u{200D}' | '\u{FE0F}' | '\u{FE0E}');
        let chars: Vec<char> = text.chars().filter(|c| !joiners(c)).collect();
        let emoji = text.contains('\u{FE0F}') || text.contains('\u{200D}') || chars.iter().any(|&c| is_emoji(c) || ('\u{1F3FB}'..='\u{1F3FF}').contains(&c));
        let covers = |f: &Face| chars.iter().all(|&c| f.gid(c) != 0);
        let (slot, fake) = self.run_face(style);
        let mut order: Vec<usize> = Vec::new();
        if emoji && !text.contains('\u{FE0E}') {
            order.extend((0..self.faces.len()).filter(|&i| self.faces[i].as_ref().is_some_and(|f| f.has_color)));
        }
        order.push(slot as usize);
        order.extend(0..self.faces.len());
        let pick = order
            .iter()
            .copied()
            .find(|&i| self.faces[i].as_ref().is_some_and(&covers))
            .or_else(|| {
                let first = *chars.first()?;
                order.iter().copied().find(|&i| self.faces[i].as_ref().is_some_and(|f| f.gid(first) != 0))
            })
            .unwrap_or(slot as usize);
        let f = self.faces[pick].as_ref().unwrap();
        let mut buf = rustybuzz::UnicodeBuffer::new();
        buf.push_str(text);
        let out = rustybuzz::shape(&f.face, &[], buf);
        let scale = f.scale(self.size_px);
        let mut pen = 0.0f32;
        let mut glyphs = Vec::new();
        for (info, pos) in out.glyph_infos().iter().zip(out.glyph_positions()) {
            let id = info.glyph_id as u16;
            let x = ((pen + pos.x_offset as f32) * scale).round() as i32;
            let y = (pos.y_offset as f32 * scale).round() as i32;
            pen += pos.x_advance as f32;
            if id == 0 {
                continue;
            }
            let color = f.has_color && f.face.is_color_glyph(GlyphId(id));
            glyphs.push(ClusterGlyph { key: GlyphKey { face: pick as u16, id, fake_bold: fake && pick == slot as usize && !color, color, split: 0 }, x, y });
        }
        // Colour emoji: a ligated sequence is one glyph filling the 2-cell
        // box. If the font doesn't ligate it (e.g. Segoe UI Emoji has no
        // three-person family), share the box between the pieces.
        let color: Vec<ClusterGlyph> = glyphs.iter().copied().filter(|g| g.key.color).collect();
        if !color.is_empty() {
            let n = color.len().min(8) as u8;
            let slot_w = (2 * self.cell_width) as i32 / n as i32;
            glyphs = color
                .into_iter()
                .take(n as usize)
                .enumerate()
                .map(|(i, mut g)| {
                    g.key.split = if n > 1 { n } else { 0 };
                    g.x = if n > 1 { i as i32 * slot_w } else { 0 };
                    g.y = 0;
                    g
                })
                .collect();
        }
        if self.clusters.len() > 4096 {
            self.clusters.clear();
        }
        self.clusters.insert(key, glyphs.clone());
        glyphs
    }

    pub fn glyph(&mut self, key: GlyphKey) -> &Glyph {
        if !self.glyphs.contains_key(&key) {
            let g = self.rasterize(key);
            self.glyphs.insert(key, g);
        }
        &self.glyphs[&key]
    }

    fn is_ui_face(&self, face: u16) -> bool {
        self.ui.is_some_and(|(r, b)| face == r || face == b)
    }

    pub fn has_ui_font(&self) -> bool {
        self.ui.is_some()
    }

    /// Lay out `text` in the UI font (None if there is none). Returns the
    /// glyphs and the run's width in pixels.
    pub fn ui_run(&mut self, text: &str, semibold: bool) -> Option<(Vec<UiGlyph>, i32)> {
        let (r, b) = self.ui?;
        let slot = if semibold { b } else { r };
        let f = self.faces[slot as usize].as_ref()?;
        let scale = f.scale(self.size_px * UI_SCALE);
        let mut x = 0.0f32;
        let mut out = Vec::new();
        let mut missing = Vec::new();
        for c in text.chars() {
            let id = f.gid(c);
            if id == 0 && !c.is_whitespace() {
                missing.push((c, x));
                x += self.cell_width as f32;
                continue;
            }
            out.push(UiGlyph { key: GlyphKey { face: slot, id, fake_bold: false, color: false, split: 0 }, x: x.round() as i32 });
            x += f.face.glyph_hor_advance(GlyphId(id)).unwrap_or(0) as f32 * scale;
        }
        // characters Segoe lacks come from the terminal fonts
        for (c, at) in missing {
            let key = self.resolve(c, Style::Regular);
            out.push(UiGlyph { key, x: at.round() as i32 });
        }
        Some((out, x.round() as i32))
    }

    fn rasterize(&self, key: GlyphKey) -> Glyph {
        let f = self.faces.get(key.face as usize).and_then(|f| f.as_ref()).unwrap_or_else(|| self.regular());
        if key.color {
            if let Some(g) = self.rasterize_color(f, key.id, key.split.max(1)) {
                return g;
            }
        }
        let px = if self.is_ui_face(key.face) { self.size_px * UI_SCALE } else { self.size_px };
        let scale = f.scale(px);
        let Some((x0, y0, x1, y1)) = raster::pixel_bounds(&f.face, &[key.id], scale) else {
            return Glyph::empty(key.fake_bold);
        };
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let coverage = raster::render_into(&f.face, key.id, scale, x0, y1, w, h);
        Glyph { xmin: x0, ymin: y0, width: w, height: h, bitmap: Bitmap::Mono(coverage), fake_bold: key.fake_bold }
    }

    /// Render a COLR glyph (v0 layers or v1 paint graph) so its ink fits two
    /// cells wide and one cell tall, centred.
    fn rasterize_color(&self, f: &Face, id: u16, split: u8) -> Option<Glyph> {
        let slot = (2 * self.cell_width) as f32 / split as f32;
        let box_w = slot - if split > 1 { 0.5 } else { 2.0 };
        let box_h = (self.cell_height as f32 - 1.0).min(if split > 1 { slot * 1.4 } else { f32::MAX });
        let bmp = crate::colr::render(&f.face, id, box_w, box_h)?;
        let (w, h) = (bmp.width, bmp.height);
        // centre in its slot of the 2-cell box; ymin is relative to the baseline
        let xmin = (slot as i32 - w as i32) / 2;
        let top = (self.cell_height as i32 - h as i32) / 2;
        let ymin = self.baseline - top - h as i32;
        Some(Glyph { xmin, ymin, width: w, height: h, bitmap: Bitmap::Color(bmp.rgba), fake_bold: false })
    }
}

/// Characters drawn as colour emoji (the double-width emoji blocks; the
/// older 0x2600 symbols stay monochrome so they keep their 1-cell width).
pub fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F300..=0x1F5FF | 0x1F600..=0x1F64F | 0x1F680..=0x1F6FF |
        0x1F900..=0x1F9FF | 0x1FA70..=0x1FAFF | 0x1F1E6..=0x1F1FF)
}
