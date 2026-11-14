use super::*;
use vibex_term::Terminal;
use vibex_vt::Parser;

#[test]
fn status_text_is_shortened_from_the_front() {
    use crate::scene::keep_end;
    assert_eq!(keep_end(r" C:\Users\kalya\stress\imgs", 12), r" …tress\imgs");
    assert_eq!(keep_end(" short", 12), " short");
    assert_eq!(keep_end("abcdef", 4), "…def");
    assert_eq!(keep_end(" 日本語のフォルダ", 8), " …ォルダ");
    assert!(crate::text_width(&keep_end(" 日本語のフォルダ", 7)) <= 7);
}

fn font(name: &str) -> Option<String> {
    let p = std::path::Path::new(&std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts").join(name);
    p.exists().then(|| p.to_string_lossy().into_owned())
}

fn renderer() -> Option<Renderer> {
    let paths = FontPaths {
        regular: font("consola.ttf")?,
        bold: font("consolab.ttf"),
        italic: font("consolai.ttf"),
        bold_italic: font("consolaz.ttf"),
        fallbacks: ["seguiemj.ttf", "seguisym.ttf"].iter().filter_map(|f| font(f)).collect(),
    };
    Some(Renderer::new(FontSet::load(&paths, 16.0).ok()?, Theme::default()))
}

fn term(cols: usize, rows: usize, input: &str) -> Terminal {
    let mut t = Terminal::new(cols, rows);
    Parser::new().advance_bytes(&mut t, input.as_bytes());
    t
}

fn state() -> DrawState {
    DrawState { focused: true, cursor_visible: false, ..Default::default() }
}

fn glyphs(s: &Scene) -> Vec<GlyphKey> {
    s.items.iter().filter_map(|i| if let Item::Glyph(g) = i { Some(g.key) } else { None }).collect()
}

fn rects(s: &Scene) -> usize {
    s.items.iter().filter(|i| matches!(i, Item::Rect(_))).count()
}

#[test]
fn text_becomes_glyphs_spaces_dont() {
    let Some(mut r) = renderer() else { return };
    let t = term(10, 1, "a b");
    let s = r.build_scene(&t, &state(), 200, 40);
    assert_eq!(glyphs(&s).len(), 2);
    assert_eq!(rects(&s), 0);
}

#[test]
fn background_runs_are_merged() {
    let Some(mut r) = renderer() else { return };
    let t = term(10, 1, "\x1b[44mabcd\x1b[0m");
    let s = r.build_scene(&t, &state(), 200, 40);
    assert_eq!(rects(&s), 1);
}

#[test]
fn box_drawing_is_geometry() {
    let Some(mut r) = renderer() else { return };
    let t = term(10, 1, "┌─┐");
    let s = r.build_scene(&t, &state(), 200, 40);
    assert!(glyphs(&s).is_empty());
    assert!(rects(&s) >= 4);
}

#[test]
fn shaped_runs_keep_one_glyph_per_cell_without_ligature_font() {
    let Some(mut r) = renderer() else { return };
    let t = term(20, 1, "a->b != c");
    let s = r.build_scene(&t, &state(), 400, 40);
    assert_eq!(glyphs(&s).len(), 7);
}

#[test]
fn emoji_uses_colour_face() {
    let Some(mut r) = renderer() else { return };
    if font("seguiemj.ttf").is_none() {
        return;
    }
    let key = r.fonts.resolve('🦀', Style::Regular);
    assert!(key.color, "crab should come from the colour emoji face: {key:?}");
    let (cw, chh) = (r.fonts.cell_width, r.fonts.cell_height);
    let g = r.fonts.glyph(key);
    assert!(matches!(g.bitmap, font::Bitmap::Color(_)));
    eprintln!("emoji {}x{} vs cell {cw}x{chh}", g.width, g.height);
    assert!(g.width > 0 && g.width <= cw * 2 + 2, "emoji {}x{} vs cell {cw}", g.width, g.height);
    // fills most of the two-cell box
    assert!(g.width >= cw * 2 - 4 || g.height >= chh - 3);
}

#[test]
fn smooth_scroll_draws_extra_row() {
    let Some(mut r) = renderer() else { return };
    let mut t = term(5, 2, "1\r\n2\r\n3\r\n4");
    t.scroll_view(1);
    let still = r.build_scene(&t, &state(), 100, 60);
    let shifted = DrawState { y_shift: -(r.fonts.cell_height as f32), ..state() };
    let moving = r.build_scene(&t, &shifted, 100, 60);
    assert_eq!(glyphs(&still).len(), 2);
    assert_eq!(glyphs(&moving).len(), 3);
}

#[test]
fn panes_are_clipped_and_offset() {
    let Some(mut r) = renderer() else { return };
    let t = term(5, 1, "ab");
    let mut scene = r.begin(400, 200);
    let rect = PxRect { x: 200.0, y: 50.0, w: 150.0, h: 100.0 };
    r.draw_pane(&mut scene, &t, &state(), rect);
    assert!(matches!(scene.items.first(), Some(Item::Clip(Some(c))) if c.x == 200 && c.y == 50));
    assert!(matches!(scene.items.last(), Some(Item::Clip(None))));
    let xs: Vec<i32> = scene.items.iter().filter_map(|i| if let Item::Glyph(g) = i { Some(g.x) } else { None }).collect();
    assert!(xs.iter().all(|&x| x >= 200), "{xs:?}");
}

#[test]
fn prompt_marks_drawn_in_gutter() {
    let Some(mut r) = renderer() else { return };
    let t = term(10, 2, "\x1b]133;A\x07$ false\r\n\x1b]133;D;1\x07");
    let with = r.build_scene(&t, &DrawState { marks: true, ..state() }, 200, 60);
    let without = r.build_scene(&t, &state(), 200, 60);
    assert_eq!(rects(&with), rects(&without) + 1);
}

#[test]
fn chrome_draws_tabbar_and_palette() {
    let Some(mut r) = renderer() else { return };
    let bar = TabBar {
        tabs: vec![TabInfo { title: "pwsh".into(), active: true, alert: false, color: Some(0x57ab5a), editing: true, icon: crate::TabIcon::Terminal }],
        window_buttons: true,
        maximized: false,
        hover: None,
        caption: None,
        classic: false,
        title: String::new(),
        menu_open: None,
    };
    let layout = r.tabbar_layout(&bar, 800);
    let mut scene = r.begin(800, 400);
    let chrome = Chrome {
        tabbar: Some((bar, layout)),
        palette: Some(PaletteView { query: "spl".into(), items: vec![("Split right".into(), "".into())], selected: 0 }),
        ..Default::default()
    };
    r.draw_chrome(&mut scene, &chrome);
    assert!(glyphs(&scene).len() > 10);
}

#[test]
fn reverse_screen_paints_light_background() {
    let Some(mut r) = renderer() else { return };
    let normal = term(5, 1, "ab");
    let reversed = term(5, 1, "\x1b[?5hab");
    assert!(reversed.modes.reverse_screen);
    let a = r.build_scene(&normal, &state(), 100, 40);
    let b = r.build_scene(&reversed, &state(), 100, 40);
    let fg = Theme::default().foreground;
    let filled = b.items.iter().any(|i| matches!(i, Item::Rect(rc) if rc.color == fg && rc.w >= 100));
    assert!(filled, "pane should be painted with the old foreground colour");
    assert_eq!(rects(&b), rects(&a) + 1);
}

#[test]
fn clusters_shape_as_one_unit() {
    let Some(mut r) = renderer() else { return };
    if font("seguiemj.ttf").is_none() {
        return;
    }
    // a ZWJ sequence the font ligates: one colour glyph filling the cell
    let coder = r.fonts.shape_cluster("\u{1F469}\u{200D}\u{1F4BB}", Style::Regular);
    assert_eq!(coder.len(), 1, "{coder:?}");
    assert!(coder[0].key.color && coder[0].key.split == 0);
    // one it doesn't (Segoe has no three-person family): all three people,
    // sharing the two-cell box
    let fam = r.fonts.shape_cluster("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", Style::Regular);
    assert_eq!(fam.len(), 3, "{fam:?}");
    assert!(fam.iter().all(|g| g.key.color && g.key.split == 3));
    assert!(fam[2].x > fam[1].x && fam[1].x > fam[0].x);
    // e + combining acute composes to the font's precomposed é
    let e = r.fonts.shape_cluster("e\u{301}", Style::Regular);
    assert_eq!(e.len(), 1, "{e:?}");
    // q + acute has no precomposed form: base glyph + mark glyph
    let q = r.fonts.shape_cluster("q\u{301}", Style::Regular);
    assert_eq!(q.len(), 2, "{q:?}");
    assert!(!q[1].key.color);
    // drawn in a scene
    let t = term(10, 1, "e\u{301} \u{1F44D}\u{1F3FD}");
    let s = r.build_scene(&t, &state(), 200, 40);
    let keys = glyphs(&s);
    assert!(keys.iter().any(|k| k.color), "skin-tone thumbs up in colour");
}

#[test]
fn ime_preedit_is_drawn_at_the_cursor() {
    let Some(mut r) = renderer() else { return };
    let t = term(20, 1, "ab");
    let plain = r.build_scene(&t, &state(), 300, 40);
    // (the test font set has no CJK face, so count with Latin text)
    let comp = r.build_scene(&t, &DrawState { preedit: Some("nih".into()), ..state() }, 300, 40);
    assert_eq!(glyphs(&comp).len(), glyphs(&plain).len() + 3);
    assert_eq!(rects(&comp), rects(&plain) + 2, "background + underline");
}

#[test]
fn cursor_block_is_drawn() {
    let Some(mut r) = renderer() else { return };
    let t = term(5, 1, "ab");
    let s = r.build_scene(&t, &DrawState { cursor_visible: true, ..state() }, 100, 40);
    assert_eq!(rects(&s), 1);
}

#[test]
fn block_selection_is_rectangular() {
    let s = crate::VisibleSelection { start: (5, 1), end: (2, 3), block: true };
    assert!(s.contains(2, 1) && s.contains(5, 3) && s.contains(3, 2));
    assert!(!s.contains(6, 2) && !s.contains(1, 2) && !s.contains(3, 0));
    let s = crate::VisibleSelection { start: (5, 1), end: (2, 3), block: false };
    assert!(s.contains(9, 2), "a stream selection covers whole middle rows");
}

fn rect_items(s: &Scene) -> Vec<crate::scene::Rect> {
    s.items.iter().filter_map(|i| if let Item::Rect(r) = i { Some(*r) } else { None }).collect()
}

#[test]
fn font_caches_stay_bounded() {
    let Some(mut r) = renderer() else { return };
    // a long session of arbitrary text: every char and run is new
    for c in ('\u{4E00}'..'\u{9FFF}').chain('\u{AC00}'..'\u{D7A3}').chain('\u{20000}'..'\u{2FFFF}') {
        r.fonts.resolve(c, Style::Regular);
    }
    for i in 0..20_000u32 {
        r.fonts.shape(&format!("x->{i}!=y"), Style::Regular);
        r.fonts.shape_cluster(&format!("e\u{301}{}", char::from_u32(0x100 + i % 0x500).unwrap_or('a')), Style::Regular);
    }
    for id in 0..9000u16 {
        r.fonts.glyph(GlyphKey { face: 0, id, fake_bold: false, color: false, split: 0 });
    }
    let (glyphs, lookups, shaped, clusters) = r.fonts.cache_sizes();
    assert!(lookups <= 65536, "{lookups}");
    assert!(shaped <= 8192, "{shaped}");
    assert!(clusters <= 4096, "{clusters}");
    assert!(glyphs <= 8192, "{glyphs}");
}

#[test]
fn reloaded_fonts_have_a_new_generation() {
    // the GPU atlas is keyed by (face slot, glyph id): a different font at
    // the same size must reset it
    let (Some(a), Some(b)) = (renderer(), renderer()) else { return };
    assert_ne!(a.fonts.generation(), b.fonts.generation());
}

#[test]
fn sticky_header_in_a_narrow_pane() {
    let Some(mut r) = renderer() else { return };
    let t = term(10, 2, "ab");
    let sticky = crate::StickyHeader { command: "cargo build --release".into(), detail: "12.3s · exit 1".into(), color: 0xff0000 };
    let st = DrawState { sticky: Some(sticky), ..state() };
    // narrower than the detail: used to underflow (cells - detail_w)
    let mut scene = r.begin(60, 60);
    r.draw_pane(&mut scene, &t, &st, PxRect { x: 0.0, y: 0.0, w: 40.0, h: 60.0 });
    let mut scene = r.begin(400, 60);
    r.draw_pane(&mut scene, &t, &st, PxRect { x: 0.0, y: 0.0, w: 400.0, h: 60.0 });
    assert!(glyphs(&scene).len() > 10);
}

#[test]
fn wide_char_selection_covers_both_halves() {
    let Some(mut r) = renderer() else { return };
    let cw = r.fonts.cell_width as u32;
    let t = term(10, 1, "日本");
    assert!(t.visible_row(0).cells[1].has(vibex_term::flags::WIDE_SPACER));
    // selecting just the second half of 日 highlights all of it
    let sel = crate::VisibleSelection { start: (1, 0), end: (1, 0), block: false };
    let s = r.build_scene(&t, &DrawState { selection: Some(sel), ..state() }, 200, 40);
    let bg: Vec<_> = rect_items(&s).into_iter().filter(|rc| rc.color == r.theme.selection).collect();
    assert_eq!(bg.len(), 1, "{bg:?}");
    assert_eq!(bg[0].w, 2 * cw);
    assert_eq!(bg[0].x, r.padding_x as i32);
}

#[test]
fn cursor_on_wide_char_second_half() {
    let Some(mut r) = renderer() else { return };
    let cw = r.fonts.cell_width as u32;
    // the cursor moved onto the spacer of 日
    let t = term(10, 1, "日本\x1b[2G");
    assert_eq!(t.cursor.x, 1);
    let s = r.build_scene(&t, &DrawState { cursor_visible: true, ..state() }, 200, 40);
    let cur: Vec<_> = rect_items(&s).into_iter().filter(|rc| rc.color == r.theme.cursor).collect();
    assert_eq!(cur.len(), 1, "{cur:?}");
    assert_eq!((cur[0].x, cur[0].w), (r.padding_x as i32, 2 * cw), "covers the whole char");
    // the hollow (unfocused) cursor is two cells wide too
    let s = r.build_scene(&t, &DrawState { cursor_visible: true, focused: false, ..state() }, 200, 40);
    let top = rect_items(&s).into_iter().find(|rc| rc.color == r.theme.cursor && rc.h == 1).unwrap();
    assert_eq!(top.w, 2 * cw);
}

#[test]
fn colour_emoji_in_a_narrow_cell_fits_it() {
    let Some(mut r) = renderer() else { return };
    // U+1F321 (thermometer) is one cell wide but is drawn from the colour face
    let key = r.fonts.resolve('\u{1F321}', Style::Regular);
    if !key.color {
        return;
    }
    let t = term(10, 1, "\u{1F321}x");
    assert!(!t.visible_row(0).cells[0].has(vibex_term::flags::WIDE));
    let s = r.build_scene(&t, &state(), 200, 40);
    let k = glyphs(&s).into_iter().find(|k| k.color).unwrap();
    assert_eq!(k.split, 2);
    let cw = r.fonts.cell_width;
    let g = r.fonts.glyph(k);
    assert!(g.width <= cw && g.xmin >= 0 && g.xmin as usize + g.width <= cw, "{}+{} vs {cw}", g.xmin, g.width);
}

#[test]
fn huge_scroll_offset_does_not_overflow() {
    let Some(mut r) = renderer() else { return };
    let t = term(5, 2, "ab");
    for y_shift in [f32::MAX, f32::MIN, f32::NAN, 1e9] {
        r.build_scene(&t, &DrawState { y_shift, ..state() }, 100, 40);
    }
}

#[test]
fn bad_images_are_skipped() {
    use vibex_term::image::ImageData;
    let mut s = Scene::new(10, 10, 0);
    let empty = std::sync::Arc::new(ImageData { uid: u64::MAX, width: 0, height: 0, rgba: Vec::new() });
    let short = std::sync::Arc::new(ImageData { uid: u64::MAX - 1, width: 4, height: 4, rgba: vec![255; 8] });
    s.image(&empty, 0, 0, 5, 5, [0.0, 0.0, 1.0, 1.0]);
    s.image(&short, 0, 0, 5, 5, [0.0, 0.0, 4.0, 4.0]);
    assert!(s.items.is_empty());
    let ok = ImageData::new(2, 2, vec![255; 16]);
    s.image(&ok, 0, 0, 5, 5, [0.0, 0.0, 2.0, 2.0]);
    assert_eq!(s.items.len(), 1);
}

#[test]
fn cpu_draw_survives_odd_frames() {
    let Some(mut r) = renderer() else { return };
    let t = term(10, 2, "hello 日本 \u{1F980}");
    let scene = r.build_scene(&t, &state(), 300, 60);
    // zero-size window
    let mut px: Vec<u32> = Vec::new();
    cpu::draw(&scene, &mut r.fonts, &mut Frame { pixels: &mut px, width: 0, height: 0 });
    // a buffer smaller than the size it claims, and a window narrower than a cell
    let mut px = vec![0u32; 300 * 10];
    cpu::draw(&scene, &mut r.fonts, &mut Frame { pixels: &mut px, width: 300, height: 60 });
    let mut px = vec![0u32; 3 * 60];
    cpu::draw(&scene, &mut r.fonts, &mut Frame { pixels: &mut px, width: 3, height: 60 });
    // items far outside the window, with huge sizes
    let mut s = Scene::new(20, 20, 0);
    s.rect(i32::MAX - 2, i32::MAX - 2, usize::MAX, usize::MAX, 0xffffff);
    s.rect(-5, -5, usize::MAX, 10, 0xffffff);
    s.clip(Some(PxRect { x: 1e30, y: -1e30, w: f32::MAX, h: f32::MAX }));
    s.rect(0, 0, 10, 10, 0xffffff);
    s.blur(PxRect { x: -1e30, y: 0.0, w: f32::MAX, h: 10.0 }, 4.0);
    let mut px = vec![0u32; 400];
    cpu::draw(&s, &mut r.fonts, &mut Frame { pixels: &mut px, width: 20, height: 20 });
    assert_eq!(px[0], 0xffffff, "the rect from -5,-5 still covers the top-left");
}

#[test]
fn hspan_inside_one_pixel_uses_its_width() {
    let mut s = Scene::new(10, 1, 0);
    crate::scene::hspan(&mut s, 3.2, 3.5, 0, 0xffffff, 200);
    let r = rect_items(&s);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].x, 3);
    assert!((r[0].alpha as i32 - 60).abs() <= 1, "{}", r[0].alpha);
}
