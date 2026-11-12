//! Headless throughput benchmark: parse 1M lines of output into a
//! 160x48 terminal, then time scene building for a full screen of
//! coloured text. Run: cargo run --release -p vibex-render --example throughput

use std::time::Instant;
use vibex_render::{DrawState, FontPaths, FontSet, Renderer, Theme};
use vibex_term::Terminal;
use vibex_vt::Parser;

fn font(name: &str) -> Option<String> {
    let p = std::path::Path::new(r"C:\Windows\Fonts").join(name);
    p.exists().then(|| p.to_string_lossy().into_owned())
}

fn main() {
    // 1. parse + terminal state
    let mut out = Vec::new();
    for i in 1..=1_000_000u32 {
        out.extend_from_slice(format!("{i}\r\n").as_bytes());
    }
    let mut colored = Vec::new();
    for i in 0..200_000u32 {
        colored.extend_from_slice(format!("\x1b[3{}mline {i} \x1b[1mbold\x1b[0m text -> here\r\n", i % 8).as_bytes());
    }
    let mut term = Terminal::new(160, 48);
    let mut parser = Parser::new();
    let t = Instant::now();
    parser.advance_bytes(&mut term, &out);
    let plain = t.elapsed();
    let t = Instant::now();
    parser.advance_bytes(&mut term, &colored);
    let color = t.elapsed();
    println!(
        "parse: 1M plain lines ({:.1} MB) in {:.0} ms = {:.0} MB/s",
        out.len() as f64 / 1e6,
        plain.as_secs_f64() * 1e3,
        out.len() as f64 / 1e6 / plain.as_secs_f64()
    );
    println!(
        "parse: 200k SGR lines ({:.1} MB) in {:.0} ms = {:.0} MB/s",
        colored.len() as f64 / 1e6,
        color.as_secs_f64() * 1e3,
        colored.len() as f64 / 1e6 / color.as_secs_f64()
    );

    // 2. scene building (what each frame costs on the CPU before the GPU draws it)
    let paths = FontPaths {
        regular: font("consola.ttf").expect("consola.ttf"),
        bold: font("consolab.ttf"),
        italic: font("consolai.ttf"),
        bold_italic: font("consolaz.ttf"),
        fallbacks: vec![],
    };
    let mut r = Renderer::new(FontSet::load(&paths, 22.0).unwrap(), Theme::default());
    let state = DrawState { focused: true, cursor_visible: true, ..Default::default() };
    let _ = r.build_scene(&term, &state, 1920, 1080); // warm glyph cache
    let frames = 500;
    let t = Instant::now();
    let mut items = 0;
    for _ in 0..frames {
        items = r.build_scene(&term, &state, 1920, 1080).items.len();
    }
    let per = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
    println!("scene: 160x48 full screen, {items} items, {per:.3} ms/frame ({:.0} fps ceiling)", 1000.0 / per);
}
