use super::*;
use vibex_vt::Parser;

fn term(cols: usize, rows: usize, input: &str) -> Terminal {
    let mut t = Terminal::new(cols, rows);
    let mut p = Parser::new();
    p.advance_bytes(&mut t, input.as_bytes());
    t
}

fn feed(t: &mut Terminal, input: &str) {
    let mut p = Parser::new();
    p.advance_bytes(t, input.as_bytes());
}

#[test]
fn prints_and_newlines() {
    let t = term(10, 3, "ab\r\ncd");
    assert_eq!(t.screen_text(), "ab\ncd\n");
    assert_eq!((t.cursor.x, t.cursor.y), (2, 1));
}

#[test]
fn autowrap_with_pending_wrap() {
    let t = term(4, 3, "abcd");
    // cursor stays on last column until the next print
    assert_eq!((t.cursor.x, t.cursor.y), (3, 0));
    let t = term(4, 3, "abcde");
    assert_eq!(t.screen_text(), "abcd\ne\n");
    assert!(t.screen_row(0).wrapped);
}

#[test]
fn cr_clears_pending_wrap() {
    let t = term(4, 3, "abcd\rX");
    assert_eq!(t.screen_text(), "Xbcd\n\n");
}

#[test]
fn no_autowrap_overwrites_last_column() {
    let t = term(4, 2, "\x1b[?7labcdef");
    assert_eq!(t.screen_text(), "abcf\n");
}

#[test]
fn scrolls_into_scrollback() {
    let t = term(5, 2, "1\r\n2\r\n3\r\n4");
    assert_eq!(t.screen_text(), "3\n4");
    assert_eq!(t.scrollback_len(), 2);
}

#[test]
fn view_scrolling() {
    let mut t = term(5, 2, "1\r\n2\r\n3\r\n4");
    t.scroll_view(1);
    assert_eq!(t.visible_row(0).text(), "2");
    assert_eq!(t.visible_row(1).text(), "3");
    t.scroll_view(10);
    assert_eq!(t.display_offset(), 2);
    assert_eq!(t.visible_row(0).text(), "1");
    t.scroll_view_to_bottom();
    assert_eq!(t.visible_row(0).text(), "3");
}

#[test]
fn view_stays_anchored_on_new_output() {
    let mut t = term(5, 2, "1\r\n2\r\n3");
    t.scroll_view(1);
    assert_eq!(t.visible_row(0).text(), "1");
    feed(&mut t, "\r\n4");
    assert_eq!(t.visible_row(0).text(), "1");
}

#[test]
fn cursor_movement() {
    let t = term(10, 5, "\x1b[3;4HX\x1b[2AY\x1b[10CZ\x1b[1;1H\x1b[BW");
    assert_eq!(t.screen_row(2).text(), "   X");
    assert_eq!(t.screen_row(0).text(), "    Y    Z");
    assert_eq!(t.screen_row(1).text(), "W");
}

#[test]
fn erase_display_and_line() {
    let mut t = term(5, 3, "aaaaabbbbbccc");
    feed(&mut t, "\x1b[2;3H\x1b[K");
    assert_eq!(t.screen_text(), "aaaaa\nbb\nccc");
    feed(&mut t, "\x1b[1K");
    assert_eq!(t.screen_row(1).text(), "");
    feed(&mut t, "\x1b[J");
    assert_eq!(t.screen_text(), "aaaaa\n\n");
    feed(&mut t, "\x1b[2J");
    assert_eq!(t.screen_text(), "\n\n");
}

#[test]
fn erase_uses_background_color() {
    let t = term(4, 1, "\x1b[44m\x1b[2K");
    assert_eq!(t.screen_row(0).cells[3].bg, Color::Indexed(4));
}

#[test]
fn insert_delete_chars() {
    let t = term(6, 1, "abcdef\x1b[1;2H\x1b[2@");
    assert_eq!(t.screen_row(0).text(), "a  bcd");
    let t = term(6, 1, "abcdef\x1b[1;2H\x1b[2P");
    assert_eq!(t.screen_row(0).text(), "adef");
    let t = term(6, 1, "abcdef\x1b[1;2H\x1b[2X");
    assert_eq!(t.screen_row(0).text(), "a  def");
}

#[test]
fn scroll_region_and_lines() {
    // region rows 2-3; LF at bottom of region scrolls only the region
    let t = term(3, 4, "1\r\n2\r\n3\r\n4\x1b[2;3r\x1b[3;1H\nX");
    assert_eq!(t.screen_text(), "1\n3\nX\n4");
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn insert_and_delete_lines() {
    let t = term(3, 4, "1\r\n2\r\n3\r\n4\x1b[2;1H\x1b[L");
    assert_eq!(t.screen_text(), "1\n\n2\n3");
    let t = term(3, 4, "1\r\n2\r\n3\r\n4\x1b[2;1H\x1b[M");
    assert_eq!(t.screen_text(), "1\n3\n4\n");
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn reverse_index_scrolls_down() {
    let t = term(3, 3, "1\r\n2\r\n3\x1b[H\x1bM");
    assert_eq!(t.screen_text(), "\n1\n2");
}

#[test]
fn sgr_colors_and_attributes() {
    let t = term(10, 1, "\x1b[1;31;44mA\x1b[38;5;200mB\x1b[38;2;1;2;3mC\x1b[0mD");
    let c = &t.screen_row(0).cells;
    assert!(c[0].has(flags::BOLD));
    assert_eq!(c[0].fg, Color::Indexed(1));
    assert_eq!(c[0].bg, Color::Indexed(4));
    assert_eq!(c[1].fg, Color::Indexed(200));
    assert_eq!(c[2].fg, Color::Rgb(1, 2, 3));
    assert_eq!(c[3], Cell { c: 'D', ..Cell::default() });
}

#[test]
fn sgr_colon_forms() {
    let t = term(10, 1, "\x1b[38:2::9:8:7mA\x1b[48:5:42mB\x1b[4:3mC\x1b[95mD");
    let c = &t.screen_row(0).cells;
    assert_eq!(c[0].fg, Color::Rgb(9, 8, 7));
    assert_eq!(c[1].bg, Color::Indexed(42));
    assert!(c[2].has(flags::CURLY_UNDERLINE));
    assert_eq!(c[3].fg, Color::Indexed(13));
}

#[test]
fn alt_screen_1049() {
    let mut t = term(5, 2, "main\x1b[?1049hALT");
    assert!(t.is_alt_screen());
    assert_eq!(t.screen_text(), "    A\nLT");
    feed(&mut t, "\x1b[?1049l");
    assert!(!t.is_alt_screen());
    assert_eq!(t.screen_text(), "main\n");
    assert_eq!((t.cursor.x, t.cursor.y), (4, 0));
}

#[test]
fn alt_screen_does_not_feed_scrollback() {
    let t = term(3, 2, "\x1b[?1049h1\r\n2\r\n3\r\n4");
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn wide_chars() {
    let t = term(5, 2, "a日b");
    let c = &t.screen_row(0).cells;
    assert!(c[1].has(flags::WIDE));
    assert!(c[2].has(flags::WIDE_SPACER));
    assert_eq!(c[3].c, 'b');
    assert_eq!(t.screen_row(0).text(), "a日b");
}

#[test]
fn wide_char_wraps_at_last_column() {
    let t = term(4, 2, "abc日");
    assert_eq!(t.screen_text(), "abc\n日");
}

#[test]
fn overwriting_half_of_wide_char_clears_it() {
    let t = term(4, 1, "日\x1b[1;2Hx");
    assert_eq!(t.screen_row(0).text(), " x");
}

#[test]
fn tabs() {
    let t = term(20, 1, "a\tb");
    assert_eq!(t.screen_row(0).cells[8].c, 'b');
    let t = term(20, 1, "\x1b[3g\x1b[1;4H\x1bH\x1b[1;1H\tX");
    assert_eq!(t.screen_row(0).cells[3].c, 'X');
}

#[test]
fn save_restore_cursor() {
    let t = term(10, 3, "\x1b[2;3H\x1b7\x1b[H\x1b8X");
    assert_eq!(t.screen_row(1).cells[2].c, 'X');
}

#[test]
fn device_reports() {
    let mut t = term(10, 5, "\x1b[3;4H\x1b[6n\x1b[c\x1b[5n");
    assert_eq!(t.take_responses(), b"\x1b[3;4R\x1b[?62;4;22c\x1b[0n");
}

#[test]
fn osc_title() {
    let mut t = term(10, 1, "\x1b]0;hello; world\x07");
    assert_eq!(t.take_title_change(), Some("hello; world"));
    assert_eq!(t.take_title_change(), None);
}

#[test]
fn modes() {
    let t = term(10, 1, "\x1b[?1h\x1b[?25l\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[5 q");
    assert!(t.modes.app_cursor);
    assert!(!t.modes.show_cursor);
    assert!(t.modes.bracketed_paste);
    assert_eq!(t.modes.mouse_report, MouseMode::Drag);
    assert!(t.modes.mouse_sgr);
    assert_eq!(t.cursor_shape, CursorShape::Bar);
}

#[test]
fn origin_mode() {
    let t = term(5, 5, "\x1b[2;4r\x1b[?6h\x1b[1;1HX\x1b[9;1HY");
    assert_eq!(t.screen_row(1).cells[0].c, 'X');
    assert_eq!(t.screen_row(3).cells[0].c, 'Y');
}

#[test]
fn repeat_last_char() {
    let t = term(10, 1, "a\x1b[3b");
    assert_eq!(t.screen_row(0).text(), "aaaa");
}

#[test]
fn resize_shrink_keeps_cursor_line() {
    let mut t = term(5, 4, "1\r\n2\r\n3\r\n4");
    t.resize(5, 2);
    assert_eq!(t.screen_text(), "3\n4");
    assert_eq!(t.cursor.y, 1);
    assert_eq!(t.scrollback_len(), 2);
}

#[test]
fn resize_grow_pulls_from_scrollback() {
    let mut t = term(5, 2, "1\r\n2\r\n3\r\n4");
    t.resize(5, 4);
    assert_eq!(t.screen_text(), "1\n2\n3\n4");
    assert_eq!(t.cursor.y, 3);
}

#[test]
fn resize_shrink_removes_blank_lines_below_cursor_first() {
    let mut t = term(5, 4, "1");
    t.resize(5, 2);
    assert_eq!(t.screen_text(), "1\n");
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn resize_columns() {
    let mut t = term(5, 1, "abcde");
    t.resize(3, 1);
    // reflowed: "abc" moves to history, "de" stays with the cursor
    assert_eq!(t.screen_row(0).text(), "de");
    assert_eq!(t.absolute_row(0).unwrap().text(), "abc");
    assert_eq!(t.cursor.x, 2);
    t.resize(6, 1);
    assert_eq!(t.screen_row(0).cells.len(), 6);
    assert_eq!(t.screen_row(0).text(), "abcde");
}

#[test]
fn full_reset() {
    let t = term(5, 2, "abc\x1b[31m\x1bc");
    assert_eq!(t.screen_text(), "\n");
    assert_eq!(t.cursor.pen, Cell::default());
}

#[test]
fn text_range_joins_wrapped_lines() {
    let t = term(4, 3, "abcdef\r\nxy  ");
    // line 0 wraps into line 1
    assert_eq!(t.text_range((0, 0), (3, 2)), "abcdef\nxy");
    assert_eq!(t.text_range((1, 0), (1, 1)), "bcdef");
}

#[test]
fn absolute_rows_span_scrollback() {
    let mut t = term(5, 2, "1\r\n2\r\n3\r\n4");
    assert_eq!(t.total_lines(), 4);
    assert_eq!(t.absolute_row(0).unwrap().text(), "1");
    assert_eq!(t.top_visible_line(), 2);
    t.scroll_view(1);
    assert_eq!(t.top_visible_line(), 1);
    assert_eq!(t.text_range((0, 0), (4, 3)), "1\n2\n3\n4");
}

#[test]
fn reflow_narrower_rewraps_history() {
    let mut t = term(10, 3, "abcdefgh\r\nxy");
    t.resize(4, 3);
    // "abcdefgh" -> "abcd" + "efgh", "xy" on the cursor line
    assert_eq!(t.screen_text(), "abcd\nefgh\nxy");
    assert!(t.screen_row(0).wrapped);
    assert_eq!((t.cursor.x, t.cursor.y), (2, 2));
}

#[test]
fn reflow_wider_unwraps() {
    let mut t = term(4, 3, "abcdefgh\r\nxy");
    assert_eq!(t.screen_text(), "abcd\nefgh\nxy");
    t.resize(10, 3);
    assert_eq!(t.screen_text(), "abcdefgh\nxy\n");
    assert_eq!((t.cursor.x, t.cursor.y), (2, 1));
}

#[test]
fn reflow_round_trip_with_scrollback() {
    let mut t = term(6, 2, "one two three\r\nfour");
    let before = t.text_range((0, 0), (5, t.total_lines() - 1));
    t.resize(3, 2);
    t.resize(6, 2);
    let after = t.text_range((0, 0), (5, t.total_lines() - 1));
    assert_eq!(before, after);
}

#[test]
fn reflow_wide_chars_use_pad() {
    let mut t = term(6, 2, "ab日本");
    t.resize(3, 2);
    // "ab" + pad (日 can't fit in 1 col), then 日 + 本 split
    assert!(t.screen_row(0).cells[2].has(flags::WRAP_PAD));
    t.resize(6, 2);
    assert_eq!(t.screen_row(0).text(), "ab日本");
}

#[test]
fn reflow_keeps_cursor_mid_line() {
    let mut t = term(10, 2, "hello\x1b[1;3H");
    t.resize(4, 2);
    assert_eq!((t.cursor.x, t.cursor.y), (2, 0));
}

#[test]
fn reflow_preserves_alt_screen_and_primary() {
    let mut t = term(8, 2, "primary!\x1b[?1049hALT");
    t.resize(4, 2);
    assert!(t.is_alt_screen());
    feed(&mut t, "\x1b[?1049l");
    assert!(t.text_range((0, 0), (3, t.total_lines() - 1)).contains("primary!"));
}

#[test]
fn word_and_line_selection() {
    let t = term(20, 2, "git commit -m fix");
    assert_eq!(t.word_at(5, 0), ((4, 0), (9, 0)));
    assert_eq!(t.word_at(12, 0), ((11, 0), (12, 0)));
    assert_eq!(t.line_selection(0), ((0, 0), (19, 0)));
}

#[test]
fn url_detection() {
    let t = term(40, 2, "see https://pqdoctor.com/docs). ok");
    let link = t.url_at(10, 0).unwrap();
    assert_eq!(link.url, "https://pqdoctor.com/docs");
    assert_eq!(link.start, (4, 0));
    assert_eq!(link.end, (28, 0));
    assert!(t.url_at(1, 0).is_none());
    assert!(t.url_at(33, 0).is_none());
}

#[test]
fn url_across_soft_wrap() {
    let t = term(10, 3, "https://example.com/x");
    let link = t.url_at(2, 1).unwrap();
    assert_eq!(link.url, "https://example.com/x");
    assert_eq!(link.end, (0, 2));
}

#[test]
fn url_keeps_balanced_parens() {
    let t = term(60, 1, "https://en.wikipedia.org/wiki/Rust_(language)");
    assert_eq!(t.url_at(0, 0).unwrap().url, "https://en.wikipedia.org/wiki/Rust_(language)");
}

#[test]
fn search_case_insensitive() {
    let t = term(20, 3, "Error one\r\nok\r\nerror two ERROR");
    let m = t.find("error");
    assert_eq!(m.len(), 3);
    assert_eq!(m[0], Match { line: 0, start: 0, end: 4 });
    assert_eq!(m[2], Match { line: 2, start: 10, end: 14 });
    assert!(t.find("").is_empty());
}

#[test]
fn scroll_to_line_brings_history_into_view() {
    let mut t = term(5, 2, "1\r\n2\r\n3\r\n4\r\n5\r\n6");
    t.scroll_to_line(0);
    assert_eq!(t.top_visible_line(), 0);
    t.scroll_to_line(5);
    assert!(t.top_visible_line() + t.rows() > 5);
}

#[test]
fn osc133_marks_prompts_and_exit_codes() {
    let mut t = term(20, 6, "\x1b]133;A\x07PS> ls\r\n");
    feed(&mut t, "out\r\n\x1b]133;D;0\x07\x1b]133;A\x07PS> bad\r\n");
    feed(&mut t, "err\r\n\x1b]133;D;1\x07\x1b]133;A\x07PS> ");
    assert_eq!(t.prompt_lines(), vec![0, 2, 4]);
    assert_eq!(t.absolute_row(0).unwrap().prompt.map(|m| m.exit), Some(Some(0)));
    assert_eq!(t.absolute_row(2).unwrap().prompt.map(|m| m.exit), Some(Some(1)));
    assert_eq!(t.absolute_row(4).unwrap().prompt.map(|m| m.exit), Some(None));
    assert_eq!(t.last_exit(), Some(1));
}

#[test]
fn prompt_marks_survive_scrolling_and_reflow() {
    let mut t = term(10, 2, "\x1b]133;A\x07PS> a long command\r\n1\r\n2\r\n3");
    assert_eq!(t.prompt_lines(), vec![0]);
    t.resize(5, 2);
    assert_eq!(t.prompt_lines(), vec![0]);
    t.resize(30, 2);
    assert_eq!(t.prompt_lines(), vec![0]);
}

#[test]
fn erase_clears_prompt_marks() {
    let t = term(10, 2, "\x1b]133;A\x07PS>\x1b[2J");
    assert!(t.prompt_lines().is_empty());
}

#[test]
fn osc7_and_osc9_cwd() {
    let t = term(10, 1, "\x1b]7;file://box/C:/Users/me%20x/code\x07");
    assert_eq!(t.cwd(), Some(r"C:\Users\me x\code"));
    let t = term(10, 1, "\x1b]7;file://box/home/me\x1b\\");
    assert_eq!(t.cwd(), Some("/home/me"));
    let t = term(10, 1, "\x1b]9;9;\"D:\\work\"\x07");
    assert_eq!(t.cwd(), Some(r"D:\work"));
}

#[test]
fn quick_select_classification() {
    use crate::select::classify;
    assert_eq!(classify("3f2a91c"), Some(TargetKind::Hash));
    assert_eq!(classify("deadbeefcafe1234deadbeefcafe1234deadbeef"), Some(TargetKind::Hash));
    assert_eq!(classify("abcdefg"), None, "not hex");
    assert_eq!(classify("1234567"), None, "digits only");
    assert_eq!(classify("192.168.1.10"), Some(TargetKind::Ip));
    assert_eq!(classify("10.0.0.1:8080"), Some(TargetKind::Ip));
    assert_eq!(classify("300.1.1.1"), None);
    assert_eq!(classify("123e4567-e89b-12d3-a456-426614174000"), Some(TargetKind::Uuid));
    assert_eq!(classify(r"C:\Users\me\notes.txt"), Some(TargetKind::Path));
    assert_eq!(classify("~/code/vibex"), Some(TargetKind::Path));
    assert_eq!(classify("./run.sh"), Some(TargetKind::Path));
    assert_eq!(classify("/etc/hosts"), Some(TargetKind::Path));
    assert_eq!(classify("src/main.rs:42"), Some(TargetKind::Path));
    assert_eq!(classify("crates/vibex/src"), Some(TargetKind::Path));
    assert_eq!(classify("and/or"), None, "prose");
    assert_eq!(classify("//comment"), None);
    assert_eq!(classify("hello"), None);
}

#[test]
fn quick_select_finds_targets_on_screen() {
    let t = term(
        80,
        4,
        "commit 3f2a91c fixed src/main.rs:42 (see https://pqdoctor.com/docs).\r\n\
         server 10.0.0.1:8080 id=123e4567-e89b-12d3-a456-426614174000\r\n\
         just words here",
    );
    let found: Vec<(String, TargetKind)> = t.quick_targets().into_iter().map(|t| (t.text, t.kind)).collect();
    assert_eq!(
        found,
        vec![
            ("3f2a91c".into(), TargetKind::Hash),
            ("src/main.rs:42".into(), TargetKind::Path),
            ("https://pqdoctor.com/docs".into(), TargetKind::Url),
            ("10.0.0.1:8080".into(), TargetKind::Ip),
            ("123e4567-e89b-12d3-a456-426614174000".into(), TargetKind::Uuid),
        ]
    );
    let first = &t.quick_targets()[0];
    assert_eq!((first.line, first.start, first.end), (0, 7, 13));
}

#[test]
fn kitty_keyboard_flags() {
    let mut t = term(10, 2, "\x1b[?u");
    assert_eq!(t.take_responses(), b"\x1b[?0u");
    feed(&mut t, "\x1b[>1u");
    assert_eq!(t.kitty_flags(), 1);
    feed(&mut t, "\x1b[>11u");
    assert_eq!(t.kitty_flags(), 11);
    feed(&mut t, "\x1b[=4;2u");
    assert_eq!(t.kitty_flags(), 15);
    feed(&mut t, "\x1b[=8;3u");
    assert_eq!(t.kitty_flags(), 7);
    feed(&mut t, "\x1b[<u");
    assert_eq!(t.kitty_flags(), 1);
    feed(&mut t, "\x1b[?u");
    assert_eq!(t.take_responses(), b"\x1b[?1u");
    // the alternate screen has its own stack
    feed(&mut t, "\x1b[?1049h");
    assert_eq!(t.kitty_flags(), 0);
    feed(&mut t, "\x1b[>3u");
    assert_eq!(t.kitty_flags(), 3);
    feed(&mut t, "\x1b[?1049l");
    assert_eq!(t.kitty_flags(), 1);
    feed(&mut t, "\x1b[<5u");
    assert_eq!(t.kitty_flags(), 0);
    // plain CSI u still restores the cursor
    let t = term(10, 3, "\x1b[2;3H\x1b[s\x1b[H\x1b[uX");
    assert_eq!(t.screen_row(1).cells[2].c, 'X');
}

#[test]
fn combining_marks_join_the_previous_cell() {
    let t = term(10, 1, "e\u{301}x");
    let c = &t.screen_row(0).cells;
    assert!(grapheme::is_cluster(c[0].c));
    assert_eq!(grapheme::text(c[0].c), "e\u{301}");
    assert_eq!(c[1].c, 'x');
    assert_eq!(t.cursor.x, 2);
    assert_eq!(t.screen_row(0).text(), "e\u{301}x");
}

#[test]
fn zwj_family_is_one_wide_cell() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let t = term(10, 1, &format!("{family}!"));
    let c = &t.screen_row(0).cells;
    assert_eq!(grapheme::text(c[0].c), family);
    assert!(c[0].has(flags::WIDE));
    assert!(c[1].has(flags::WIDE_SPACER));
    assert_eq!(c[2].c, '!');
    assert_eq!(t.cursor.x, 3);
}

#[test]
fn skin_tone_and_flags_join() {
    let t = term(10, 1, "\u{1F44D}\u{1F3FD}\u{1F1EE}\u{1F1F3}");
    let c = &t.screen_row(0).cells;
    assert_eq!(grapheme::text(c[0].c), "\u{1F44D}\u{1F3FD}");
    assert_eq!(grapheme::text(c[2].c), "\u{1F1EE}\u{1F1F3}", "flag = two regional indicators");
    assert!(c[2].has(flags::WIDE));
    assert_eq!(t.cursor.x, 4);
    // a third indicator starts a new flag
    let t = term(10, 1, "\u{1F1EE}\u{1F1F3}\u{1F1FA}");
    assert_eq!(grapheme::text(t.screen_row(0).cells[2].c), "\u{1F1FA}");
}

#[test]
fn vs16_widens_a_text_symbol() {
    let t = term(10, 1, "\u{2764}\u{FE0F}x");
    let c = &t.screen_row(0).cells;
    assert!(c[0].has(flags::WIDE));
    assert_eq!(grapheme::text(c[0].c), "\u{2764}\u{FE0F}");
    assert_eq!(c[2].c, 'x');
}

#[test]
fn marks_after_cursor_moves_do_not_join() {
    // moving the cursor between base and mark: the mark has nothing to attach to
    let t = term(10, 1, "a\x1b[5G\u{301}b");
    assert_eq!(t.screen_row(0).cells[0].c, 'a');
    assert_eq!(t.screen_row(0).cells[4].c, 'b');
}

#[test]
fn regex_find_and_block_text() {
    let t = term(20, 3, "err: 404\r\nok 200\r\nerr: 500 x");
    let re = regex::Regex::new(r"err: (\d+)").unwrap();
    let m = t.find_regex(&re);
    assert_eq!(m.len(), 2);
    assert_eq!((m[0].start, m[0].end), (0, 7));
    assert_eq!(m[1].line, 2);
    // unicode before the match keeps columns right
    let t = term(20, 1, "日本 abc");
    let m = t.find_regex(&regex::Regex::new("b").unwrap());
    assert_eq!((m[0].start, m[0].end), (6, 6));
    let t = term(10, 3, "abcdef\r\nghijkl\r\nmnopqr");
    assert_eq!(t.text_block((1, 0), (3, 2)), "bcd\nhij\nnop");
}

#[test]
fn generation_skips_queries() {
    let mut t = term(10, 2, "hi");
    let g = t.generation();
    let mut p = vibex_vt::Parser::new();
    p.advance_bytes(&mut t, b"\x1b[6n\x1b[c\x1b[?u\x1b[16t");
    assert_eq!(t.generation(), g, "pure queries don't need a redraw");
    let _ = t.take_responses();
    p.advance_bytes(&mut t, b"x");
    assert_ne!(t.generation(), g);
}

#[test]
fn file_refs() {
    use crate::select::parse_file_ref;
    assert_eq!(parse_file_ref("src/main.rs:12:5"), Some(("src/main.rs".into(), Some(12), Some(5))));
    assert_eq!(parse_file_ref("crates/x/lib.rs:7:"), Some(("crates/x/lib.rs".into(), Some(7), None)));
    assert_eq!(parse_file_ref(r"C:\Users\k\a.py:3"), Some((r"C:\Users\k\a.py".into(), Some(3), None)));
    assert_eq!(parse_file_ref("foo.cpp(12,5):"), Some(("foo.cpp".into(), Some(12), Some(5))));
    assert_eq!(parse_file_ref("./run.sh"), Some(("./run.sh".into(), None, None)));
    assert_eq!(parse_file_ref("1.2.3"), None);
    assert_eq!(parse_file_ref("https://x.y/a.rs"), None);
    assert_eq!(parse_file_ref("hello"), None);
    let t = term(60, 2, "error at --> src/lib.rs:42:9 here");
    let f = t.file_ref_at(17, 0).unwrap();
    assert_eq!((f.path.as_str(), f.line, f.col, f.start.0, f.end.0), ("src/lib.rs", Some(42), Some(9), 13, 27));
    assert!(t.file_ref_at(2, 0).is_none(), "'error' isn't a file");
}

#[test]
fn eviction_counter() {
    let mut t = term(10, 2, "");
    t.set_scrollback_limit(3);
    let mut p = vibex_vt::Parser::new();
    for i in 0..10 {
        p.advance_bytes(&mut t, format!("{i}\r\n").as_bytes());
    }
    let (_, ev) = t.line_base();
    // 10 lines + the cursor line; 2 on screen, 3 kept: 6 dropped
    assert_eq!(ev, 6);
    assert_eq!(t.absolute_row(0).unwrap().text(), "6");
}

#[test]
fn osc8_hyperlinks() {
    let t = term(30, 2, "see \x1b]8;;https://pqdoctor.com\x1b\\docs here\x1b]8;;\x1b\\ ok");
    let l = t.hyperlink_at(6, 0).unwrap();
    assert_eq!(l.url, "https://pqdoctor.com");
    assert_eq!(l.start, (4, 0));
    assert_eq!(l.end, (12, 0), "\"docs here\" is 9 cells");
    assert!(t.hyperlink_at(2, 0).is_none());
    assert!(t.hyperlink_at(14, 0).is_none(), "link ended");
}

#[test]
fn osc8_link_survives_sgr_reset() {
    let t = term(20, 1, "]8;;https://a.b[1mab[0mcd]8;;e");
    assert_eq!(t.hyperlink_at(3, 0).unwrap().end, (3, 0));
    assert!(t.hyperlink_at(4, 0).is_none());
}

#[test]
fn osc8_link_spans_soft_wrap() {
    let t = term(6, 3, "\x1b]8;;https://x.y\x07abcdefgh\x1b]8;;\x07");
    let l = t.hyperlink_at(1, 1).unwrap();
    assert_eq!((l.start, l.end), ((0, 0), (1, 1)));
}

#[test]
fn osc52_clipboard() {
    let mut t = term(10, 1, "\x1b]52;c;aGVsbG8gdmliZXg=\x07\x1b]52;c;?\x07");
    assert_eq!(t.take_clipboard_writes(), vec!["hello vibex".to_string()]);
    assert_eq!(t.take_clipboard_reads(), 1);
    t.reply_clipboard("hi");
    assert_eq!(t.take_responses(), b"\x1b]52;c;aGk=\x07");
}

#[test]
fn notifications() {
    let mut t = term(10, 1, "\x1b]9;build done\x07\x1b]777;notify;Tests;all 170 passed\x07\x1b]9;4;1;50\x07");
    assert_eq!(t.take_notifications(), vec![(String::new(), "build done".to_string()), ("Tests".to_string(), "all 170 passed".to_string())]);
    // 9;9 is still the working directory, not a message
    let mut t = term(10, 1, "\x1b]9;9;\"C:\\\"\x07");
    assert!(t.take_notifications().is_empty());
}

#[test]
fn decaln() {
    let t = term(3, 2, "\x1b#8");
    assert_eq!(t.screen_text(), "EEE\nEEE");
}

#[test]
fn prompt_input_reads_what_is_typed() {
    let mut t = term(40, 5, "\x1b]133;A\x07PS> \x1b]133;B\x07git pu");
    let i = t.prompt_input().unwrap();
    assert_eq!(i.text, "git pu");
    assert!(i.at_end);
    // cursor moved back: typed text stops there, and something follows it
    feed(&mut t, "\x1b[3D");
    let i = t.prompt_input().unwrap();
    assert_eq!(i.text, "git");
    assert!(!i.at_end);
    // a trailing space counts
    let t = term(40, 5, "\x1b]133;A\x07> \x1b]133;B\x07cargo ");
    assert_eq!(t.prompt_input().unwrap().text, "cargo ");
    // once the command runs there's no prompt input
    let mut t = term(40, 5, "\x1b]133;A\x07> \x1b]133;B\x07ls\r\n\x1b]133;C\x07");
    assert!(t.prompt_input().is_none());
    feed(&mut t, "out\r\n\x1b]133;D;0\x07\x1b]133;A\x07> \x1b]133;B\x07");
    assert_eq!(t.prompt_input().unwrap().text, "");
    // no shell integration: nothing
    assert!(term(40, 5, "plain> ls").prompt_input().is_none());
}

#[test]
fn regex_across_soft_wraps() {
    // 10 columns: the token wraps onto the next row
    let t = term(10, 4, "key ghp_ABCDEFGHIJ ok");
    let re = regex::Regex::new(r"ghp_[A-Z]+").unwrap();
    assert!(t.find_regex_in(&re, 0..4).iter().all(|m| m.end - m.start < 13), "per-row search can't see it whole");
    let m = t.find_regex_wrapped(&re, 0..4);
    assert_eq!(m.len(), 2, "{m:?}");
    assert_eq!((m[0].line, m[0].start, m[0].end), (0, 4, 9));
    assert_eq!((m[1].line, m[1].start, m[1].end), (1, 0, 7));
    // starting the range on the second row still finds the piece there
    let m = t.find_regex_wrapped(&re, 1..4);
    assert_eq!(m.len(), 1);
    assert_eq!((m[0].line, m[0].start), (1, 0));
}

#[test]
fn sealed_history_stays_off_screen() {
    // an earlier session, restored then sealed
    let mut t = term(20, 5, "old 1\r\nold 2\r\nold 3\r\n-- restored --\r\n");
    t.seal_history();
    assert_eq!(t.cursor.y, 0, "new session starts top-left");
    let screen = |t: &Terminal| (0..t.rows()).map(|y| t.absolute_row(t.scrollback_len() + y).unwrap().text()).collect::<Vec<_>>();
    assert!(screen(&t).iter().all(|l| l.is_empty()), "{:?}", screen(&t));
    feed(&mut t, "PS> ");
    // the window grows: nothing old comes back down
    t.resize(20, 12);
    assert_eq!(screen(&t)[0], "PS>", "{:?}", screen(&t));
    assert!(!screen(&t).iter().any(|l| l.starts_with("old")));
    // a width change re-wraps everything: still not on screen
    t.resize(14, 12);
    assert_eq!(screen(&t)[0], "PS>", "{:?}", screen(&t));
    assert!(!screen(&t).iter().any(|l| l.starts_with("old") || l.contains("restored")));
    // but it's all there in history
    let hist: Vec<String> = (0..t.scrollback_len()).map(|l| t.absolute_row(l).unwrap().text()).collect();
    assert!(hist.iter().any(|l| l == "old 1") && hist.iter().any(|l| l == "-- restored --"), "{hist:?}");
    // the new session's own lines still come back when it grows
    let mut u = term(20, 3, "");
    u.seal_history();
    feed(&mut u, "a\r\nb\r\nc\r\nd\r\ne");
    u.resize(20, 3);
    u.resize(20, 6);
    let s = screen(&u);
    assert_eq!(s[0], "a", "{s:?}");
}
