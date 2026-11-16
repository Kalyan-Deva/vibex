use super::*;
use vibex_term::Terminal;
use vibex_vt::Parser;

/// Readability of every built-in theme (run with --nocapture for the table).
/// WCAG contrast: 4.5 is "readable text", 3.0 large text / UI parts.
#[test]
fn theme_readability_report() {
    use crate::theme::contrast;
    let names = ["blk", "red", "grn", "yel", "blu", "mag", "cyn", "wht", "BLK", "RED", "GRN", "YEL", "BLU", "MAG", "CYN", "WHT"];
    println!("{:<20} {:>5} {:>5} {:>5} {:>5}  weakest ANSI on bg (excl. black/white family)", "theme", "text", "sel", "curs", "c.txt");
    let mut problems = Vec::new();
    for id in Theme::ids() {
        let raw = Theme::builtin(id).unwrap();
        let t = raw.for_drawing();
        // what's drawn: text colours as adjusted for the default background
        let shown = t.fg_palette.unwrap();
        let fg_bg = contrast(t.foreground, t.background);
        if contrast(t.foreground, t.selection) < crate::theme::MIN_SELECTION_CONTRAST - 0.05 {
            problems.push(format!("{id}: selected text {:.1}", contrast(t.foreground, t.selection)));
        }
        if contrast(t.cursor, t.background) < crate::theme::MIN_CURSOR_CONTRAST - 0.05 {
            problems.push(format!("{id}: cursor {:.1}", contrast(t.cursor, t.background)));
        }
        if contrast(t.cursor_text, t.cursor) < 3.0 {
            problems.push(format!("{id}: text under the cursor {:.1}", contrast(t.cursor_text, t.cursor)));
        }
        if fg_bg < 4.0 {
            problems.push(format!("{id}: text {fg_bg:.1}"));
        }
        for (i, c) in shown.iter().enumerate() {
            if contrast(*c, t.background) < crate::theme::MIN_TEXT_CONTRAST - 0.05 {
                problems.push(format!("{id}: ANSI {i} {:.1}", contrast(*c, t.background)));
            }
        }
        // the scheme's own palette is untouched
        assert_eq!(t.palette, raw.palette, "{id}");
        let t = Theme { palette: shown, ..t };
        let sel = contrast(t.foreground, t.selection);
        let cur = contrast(t.cursor, t.background);
        let cur_txt = contrast(t.cursor_text, t.cursor);
        // colours that are text on the default background: 1-6, 9-14, and
        // 8 (bright black: comments, hints) - plus white on light themes,
        // black on dark ones
        let mut weak: Vec<(f32, &str)> = (1..16)
            .filter(|i| *i != 7 && *i != 15 || t.is_light())
            .filter(|i| *i != 0 || !t.is_light())
            .map(|i| (contrast(t.palette[i], t.background), names[i]))
            .collect();
        weak.sort_by(|a, b| a.0.total_cmp(&b.0));
        let w: Vec<String> = weak.iter().take(3).map(|(c, n)| format!("{n} {c:.1}")).collect();
        println!("{:<20} {:>5.1} {:>5.1} {:>5.1} {:>5.1}  {}", id, fg_bg, sel, cur, cur_txt, w.join(", "));
    }
    assert!(problems.is_empty(), "hard to read:\n{}", problems.join("\n"));
}

#[test]
fn readable_adjustments_are_small_and_optional() {
    use crate::theme::contrast;
    // a colour that already reads is left exactly as it is
    let t = Theme::builtin("dracula").unwrap().for_drawing();
    let p = t.fg_palette.unwrap();
    assert_eq!(p[1], t.palette[1]);
    // Solarized Dark's bright black is its background colour: drawn visible
    let s = Theme::builtin("solarized-dark").unwrap();
    assert_eq!(s.palette[8], s.background);
    let d = s.for_drawing();
    assert!(contrast(d.fg_palette.unwrap()[8], d.background) >= 1.95);
    // Ayu Dark's selection hid the text; the toned-down one keeps its blue
    let a = Theme::builtin("ayu-dark").unwrap();
    let ad = a.for_drawing();
    assert_ne!(ad.selection, a.selection);
    assert!((ad.selection & 0xff) > ((ad.selection >> 16) & 0xff), "still blue: {:06x}", ad.selection);
    // turned off: everything exactly as the scheme says
    let off = Theme { readable: false, ..Theme::builtin("ayu-dark").unwrap() }.for_drawing();
    assert!(off.fg_palette.is_none());
    assert_eq!((off.selection, off.cursor, off.cursor_text), (a.selection, a.cursor, a.cursor_text));
}

#[test]
fn status_text_is_shortened_from_the_front() {
    use crate::scene::keep_end;
    assert_eq!(keep_end(r" C:\Users\me\stress\imgs", 12), r" …tress\imgs");
    assert_eq!(keep_end(" short", 12), " short");
    assert_eq!(keep_end("abcdef", 4), "…def");
    assert_eq!(keep_end(" 日本語のフォルダ", 8), " …ォルダ");
    assert!(crate::text_width(&keep_end(" 日本語のフォルダ", 7)) <= 7);
}

/// Every theme drawn by the real (CPU) renderer, one BMP each, for looking
/// at: `VIBEX_THEME_GALLERY=dir cargo test -p vibex-render theme_gallery -- --ignored`
#[test]
#[ignore]
fn theme_gallery() {
    let Some(dir) = std::env::var_os("VIBEX_THEME_GALLERY") else { return };
    let Some(mut r) = renderer() else { return };
    std::fs::create_dir_all(&dir).unwrap();
    for id in Theme::ids() {
        r.theme = Theme::builtin(id).unwrap();
        let mut text = format!("\x1b[1m{}\x1b[0m\r\n", Theme::title(id));
        for (base, label) in [(30, "normal"), (90, "bright")] {
            text.push_str(&format!("{label:<7}"));
            for (i, n) in ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"].iter().enumerate() {
                text.push_str(&format!("\x1b[{}m{n}\x1b[0m ", base + i));
            }
            text.push_str("\r\n");
        }
        text.push_str("on bg  ");
        for i in 0..8 {
            text.push_str(&format!("\x1b[4{i}m \x1b[3{}m{i} \x1b[0m", if i == 0 { 7 } else { 0 }));
        }
        text.push_str("\r\n\x1b[1mbold\x1b[0m \x1b[2mdim (hints, comments)\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[7m inverse \x1b[0m\r\n");
        text.push_str("selected: plain text \x1b[31mred \x1b[32mgreen \x1b[34mblue\x1b[0m, and \x1b[90mbright black\x1b[0m\r\n");
        text.push_str("\x1b[90m# a comment in bright black\x1b[0m\r\n");
        text.push_str("\x1b[32m~/code\x1b[0m \x1b[34m❯\x1b[0m git status");
        let t = term(70, 9, &text);
        let st = DrawState {
            focused: true,
            cursor_visible: true,
            selection: Some(VisibleSelection { start: (0, 5), end: (52, 5), block: false }),
            ..Default::default()
        };
        let (w, h) = (700, 9 * r.fonts.cell_height + 16);
        let scene = r.build_scene(&t, &st, w, h);
        let mut px = vec![0u32; w * h];
        let mut frame = crate::cpu::Frame { pixels: &mut px, width: w, height: h };
        crate::cpu::draw(&scene, &mut r.fonts, &mut frame);
        crate::write_bmp(&std::path::Path::new(&dir).join(format!("{id}.bmp")), &px, w, h).unwrap();
    }
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

/// Every title bar, with one tab and with three, in a light and a dark
/// theme, hovering each button in turn, maximized or not, wide and very
/// narrow: draws without panicking, inside the window, and the pane area
/// starts below it.
#[test]
fn every_title_bar_draws() {
    use crate::BarKind;
    let Some(mut r) = renderer() else { return };
    for kind in [BarKind::Tabs, BarKind::Kali, BarKind::Windows, BarKind::Gnome, BarKind::Mac, BarKind::Tmux] {
        for theme in ["tokyo-night", "terminal-basic"] {
            r.theme = Theme::builtin(theme).unwrap();
            r.bar_kind = kind;
            for n in [1usize, 3] {
                r.classic_tabs = n > 1;
                for (width, maximized) in [(900usize, false), (900, true), (180, false)] {
                    for hover in [None, Some(TabHit::Close), Some(TabHit::Minimize), Some(TabHit::Maximize), Some(TabHit::NewTab), Some(TabHit::Tab(1))] {
                        let bar = TabBar {
                            tabs: (0..n)
                                .map(|i| TabInfo {
                                    title: format!("tab {i} with a long title"),
                                    active: i == 0,
                                    alert: i == 2,
                                    color: None,
                                    editing: false,
                                    icon: crate::TabIcon::Terminal,
                                })
                                .collect(),
                            window_buttons: true,
                            maximized,
                            hover,
                            caption: None,
                            kind,
                            title: "me@host: ~\\Documents\\a long folder name".into(),
                            menu_open: None,
                        };
                        let layout = r.tabbar_layout(&bar, width);
                        assert_eq!(layout.height, r.tabbar_height(), "{kind:?} {n} tabs");
                        let mut scene = r.begin(width, 300);
                        let chrome = Chrome { tabbar: Some((bar, layout)), ..Default::default() };
                        r.draw_chrome(&mut scene, &chrome);
                        let content = r.content_rect(width, 300, true);
                        assert!(content.y >= r.tabbar_height() - 0.5, "{kind:?}: panes start below the bar");
                    }
                }
            }
        }
    }
}

#[test]
fn long_notices_wrap() {
    use crate::scene::wrap_text;
    let t = "style: GNOME Terminal (Ubuntu). Its font, Ubuntu Mono, isn't installed (design.ubuntu.com/font); using yours meanwhile.";
    let lines = wrap_text(t, 40);
    assert!(lines.iter().all(|l| crate::text_width(l) <= 40), "{lines:?}");
    assert_eq!(lines.join(" "), t, "nothing lost or doubled");
    // a word longer than a line is cut, not left to overflow
    let lines = wrap_text("C:\\a\\very\\long\\path\\with\\no\\spaces\\at\\all", 10);
    assert!(lines.iter().all(|l| crate::text_width(l) <= 10), "{lines:?}");
    // existing line breaks stay
    assert_eq!(wrap_text("one\ntwo", 40), ["one", "two"]);
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
        kind: crate::BarKind::Tabs,
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
