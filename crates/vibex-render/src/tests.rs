use super::*;
use vibex_term::Terminal;
use vibex_vt::Parser;

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
