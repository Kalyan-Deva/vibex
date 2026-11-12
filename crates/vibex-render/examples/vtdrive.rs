//! Headless test driver: run a program (default `wsl -e vttest`) in Vibex's
//! own ConPTY + terminal core, send scripted keys straight into that PTY
//! (never to the desktop), and render the screen after each step.
//!
//! cargo run --release -p vibex-render --example vtdrive -- <out-dir> "1\r|\r|\r" [program args...]
//!
//! Steps are separated by `|`; `\r`, `\e`, `\n` are escapes. Writes
//! step_NN.bmp and step_NN.txt per step.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use vibex_pty::{Command, Pty, PtyEvent};
use vibex_render::{cpu, DrawState, FontPaths, FontSet, Frame, Renderer, Theme};
use vibex_term::Terminal;
use vibex_vt::Parser;

fn font(name: &str) -> Option<String> {
    let p = std::path::Path::new(r"C:\Windows\Fonts").join(name);
    p.exists().then(|| p.to_string_lossy().into_owned())
}

fn unescape(s: &str) -> String {
    s.replace("\\r", "\r").replace("\\n", "\n").replace("\\e", "\x1b").replace("\\t", "\t")
}

/// Feed output until the program has been quiet for `quiet`. Returns true
/// if it exited.
fn settle(rx: &Receiver<PtyEvent>, pty: &Pty, term: &mut Terminal, parser: &mut Parser, quiet: Duration, max: Duration) -> bool {
    let start = Instant::now();
    loop {
        match rx.recv_timeout(quiet) {
            Ok(PtyEvent::Output(b)) => {
                parser.advance_bytes(term, &b);
                let r = term.take_responses();
                if !r.is_empty() {
                    let _ = pty.write(&r);
                }
            }
            Ok(PtyEvent::Exited(_)) => return true,
            Err(_) => return false,
        }
        if start.elapsed() > max {
            return false;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out_dir = std::path::PathBuf::from(&args[1]);
    std::fs::create_dir_all(&out_dir).unwrap();
    // "-" = no steps (just capture the first screen)
    let steps: Vec<String> = args.get(2).filter(|s| s.as_str() != "-").map(|s| s.split('|').map(unescape).collect()).unwrap_or_default();
    let program: Vec<String> = if args.len() > 3 { args[3..].to_vec() } else { vec!["wsl.exe".into(), "-e".into(), "vttest".into()] };

    let paths = FontPaths {
        regular: font("consola.ttf").expect("consola"),
        bold: font("consolab.ttf"),
        italic: font("consolai.ttf"),
        bold_italic: font("consolaz.ttf"),
        fallbacks: ["seguiemj.ttf", "seguisym.ttf", "msyh.ttc"].iter().filter_map(|f| font(f)).collect(),
    };
    let mut r = Renderer::new(FontSet::load(&paths, 15.0).unwrap(), Theme::default());
    r.padding_x = 4;
    r.padding_y = 4;
    let (cols, rows) = (80, 24);
    let w = cols * r.fonts.cell_width + 8;
    let h = rows * r.fonts.cell_height + 8;

    let (tx, rx) = mpsc::channel();
    let mut cmd = Command::new(program[0].clone());
    cmd.args = program[1..].to_vec();
    // shell integration scripts only run inside Vibex
    cmd.env.push(("TERM_PROGRAM".into(), "Vibex".into()));
    let pty = Pty::spawn(&cmd, cols as u16, rows as u16, tx, || {}).expect("spawn");
    let mut term = Terminal::new(cols, rows);
    term.set_cell_pixels(r.fonts.cell_width as u32, r.fonts.cell_height as u32);
    let mut parser = Parser::new();
    // wait for the program to draw something recognisable (WSL cold starts are slow)
    let ready = std::env::var("VT_READY").unwrap_or_else(|_| "Enter choice".into());
    let start = Instant::now();
    while !term.screen_text().contains(&ready) && start.elapsed() < Duration::from_secs(40) {
        settle(&rx, &pty, &mut term, &mut parser, Duration::from_millis(300), Duration::from_secs(2));
    }
    settle(&rx, &pty, &mut term, &mut parser, Duration::from_millis(500), Duration::from_secs(3));

    let snap = |term: &Terminal, r: &mut Renderer, name: &str| {
        let state = DrawState { focused: true, cursor_visible: true, ..Default::default() };
        let scene = r.build_scene(term, &state, w, h);
        let mut px = vec![0u32; w * h];
        cpu::draw(&scene, &mut r.fonts, &mut Frame { pixels: &mut px, width: w, height: h });
        vibex_render::write_bmp(&out_dir.join(format!("{name}.bmp")), &px, w, h).unwrap();
        std::fs::write(out_dir.join(format!("{name}.txt")), term.screen_text()).unwrap();
    };
    snap(&term, &mut r, "step_00");
    for (i, step) in steps.iter().enumerate() {
        let _ = pty.write(step.as_bytes());
        let exited = settle(&rx, &pty, &mut term, &mut parser, Duration::from_millis(700), Duration::from_secs(20));
        snap(&term, &mut r, &format!("step_{:02}", i + 1));
        if exited {
            println!("program exited after step {}", i + 1);
            break;
        }
    }
    if std::env::var_os("VT_BLOCKS").is_some() {
        // report the last finished command block (shell integration check)
        match term.last_finished_block() {
            Some(b) => println!(
                "block: prompt={} output={} end={} cmd={:?} exit={:?} took={:?}
output: {:?}",
                b.prompt,
                b.output,
                b.end,
                b.command.text,
                b.command.exit,
                b.command.duration,
                term.block_output(&b)
            ),
            None => println!("block: none"),
        }
    }
    pty.kill();
    println!("wrote {} steps to {}", steps.len() + 1, out_dir.display());
}
