//! Headless preview: render sample text with a given font family to a BMP,
//! once with ligatures off and once on. No window, no input.
//!
//! cargo run --release -p vibex-render --example preview -- <font-dir> <prefix> <out.bmp>
//! e.g. <font-dir>=...\ttf\static  <prefix>=CascadiaCode

use vibex_render::{cpu, DrawState, FontPaths, FontSet, Frame, Renderer, Theme};
use vibex_term::Terminal;
use vibex_vt::Parser;

const SAMPLE: &str = "\x1b[38;2;243;128;32m// ligatures\x1b[0m\r\n\
fn main() -> Result<(), Error> {\r\n\
    let ok = a != b && c >= d || e <= f;\r\n\
    x => y; i++; p |> q; a == b === c;\r\n\
    <!-- html --> /* c */ www ::: ... <=> ~>\r\n\
\x1b[1mbold -> != =>\x1b[0m  \x1b[3mitalic -> != =>\x1b[0m\r\n\
\x1b[31mred->\x1b[32mgreen\x1b[0m (colour change splits the run)";

fn render(paths: &FontPaths, ligatures: bool, cols: usize, rows: usize) -> (Vec<u32>, usize, usize) {
    let mut fonts = FontSet::load(paths, 26.0).expect("font");
    fonts.ligatures = ligatures;
    let mut r = Renderer::new(fonts, Theme::default());
    let mut t = Terminal::new(cols, rows);
    Parser::new().advance_bytes(&mut t, SAMPLE.as_bytes());
    let w = cols * r.fonts.cell_width + r.padding_x * 2;
    let h = rows * r.fonts.cell_height + r.padding_y * 2;
    let state = DrawState { focused: true, cursor_visible: false, ..Default::default() };
    let scene = r.build_scene(&t, &state, w, h);
    let mut px = vec![0u32; w * h];
    cpu::draw(&scene, &mut r.fonts, &mut Frame { pixels: &mut px, width: w, height: h });
    (px, w, h)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (dir, prefix, out) = (&args[1], &args[2], &args[3]);
    let f = |style: &str| {
        let p = format!("{dir}\\{prefix}-{style}.ttf");
        std::path::Path::new(&p).exists().then_some(p)
    };
    let paths =
        FontPaths { regular: f("Regular").expect("regular face"), bold: f("Bold"), italic: f("Italic"), bold_italic: f("BoldItalic"), fallbacks: vec![] };
    let (cols, rows) = (52, 7);
    let (off, w, h) = render(&paths, false, cols, rows);
    let (on, _, _) = render(&paths, true, cols, rows);
    // stack: ligatures off on top, on below, with a divider
    let mut px = off;
    px.extend(std::iter::repeat(0xf38020).take(w * 3));
    px.extend(on);
    vibex_render::write_bmp(std::path::Path::new(out), &px, w, h * 2 + 3).expect("write bmp");
    println!("wrote {out} ({w}x{})", h * 2 + 3);
}
