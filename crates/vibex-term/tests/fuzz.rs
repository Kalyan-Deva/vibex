//! Random and hostile input, random resizes and every reader: nothing may
//! panic, and the cursor stays on the screen. Deterministic (a fixed seed per
//! run); `VIBEX_FUZZ_ITERS=20000 cargo test --release -p vibex-term --test fuzz`
//! for a long run, `VIBEX_FUZZ_SEED=N` to replay one.

use vibex_term::Terminal;
use vibex_vt::Parser;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// Pieces a shell or a hostile program might send.
const PIECES: &[&str] = &[
    "hello",
    " ",
    "\r\n",
    "\n",
    "\r",
    "\t",
    "\x08",
    "\x07",
    "\x1b",
    "\x1b[",
    "\x00",
    "\x7f",
    "日本語",
    "👨‍👩‍👧",
    "🇮🇳",
    "e\u{301}",
    "\u{200d}",
    "ｗｉｄｅ",
    "\u{fe0f}",
    "\u{10ffff}",
    "\x1bc",
    "\x1b7",
    "\x1b8",
    "\x1bD",
    "\x1bM",
    "\x1bE",
    "\x1b#8",
    "\x1b(0lqk\x1b(B",
    "\x1b=",
    "\x1b>",
    "\x1b[?1049h",
    "\x1b[?1049l",
    "\x1b[?47h",
    "\x1b[?1047l",
    "\x1b[?6h",
    "\x1b[?6l",
    "\x1b[?7l",
    "\x1b[?7h",
    "\x1b[?69h",
    "\x1b[?2004h",
    "\x1b[?1000h\x1b[?1006h",
    "\x1b[?25l",
    "\x1b[?25h",
    "\x1b[H",
    "\x1b[999;999H",
    "\x1b[0;0H",
    "\x1b[65535;65535H",
    "\x1b[99999999999999999999H",
    "\x1b[5A",
    "\x1b[999B",
    "\x1b[999C",
    "\x1b[999D",
    "\x1b[0G",
    "\x1b[999d",
    "\x1b[3E",
    "\x1b[3F",
    "\x1b[J",
    "\x1b[1J",
    "\x1b[2J",
    "\x1b[3J",
    "\x1b[K",
    "\x1b[1K",
    "\x1b[2K",
    "\x1b[999L",
    "\x1b[999M",
    "\x1b[999P",
    "\x1b[999@",
    "\x1b[999X",
    "\x1b[65535b",
    "\x1b[999S",
    "\x1b[999T",
    "\x1b[5;3r",
    "\x1b[3;5r",
    "\x1b[0;0r",
    "\x1b[999;1r",
    "\x1b[r",
    "\x1b[2;999s",
    "\x1b[5;2s",
    "\x1b[?69h\x1b[10;5s",
    "\x1b[4h",
    "\x1b[4l",
    "\x1b[20h",
    "\x1b[3g",
    "\x1b[999I",
    "\x1b[999Z",
    "\x1bH",
    "\x1b[1;31;48;5;200;38;2;1;2;3m",
    "\x1b[38:2::1:2:3m",
    "\x1b[58:5:9m",
    "\x1b[4:3m",
    "\x1b[0m",
    "\x1b[;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;m",
    "\x1b[1;2;3;4;5;6;7;8;9;10;11;12;13;14;15;16;17;18;19;20;21;22;23;24;25;26;27;28;29;30;31;32;33m",
    "\x1b[6n",
    "\x1b[c",
    "\x1b[>c",
    "\x1b[5n",
    "\x1b[?u",
    "\x1b[>1u",
    "\x1b[<u",
    "\x1b[=1;1u",
    "\x1b[16t",
    "\x1b[14t",
    "\x1b[18t",
    "\x1b[22;0t",
    "\x1b[23;0t",
    "\x1b[ q",
    "\x1b[5 q",
    "\x1b[!p",
    "\x1b[?1$p",
    "\x1bP$qm\x1b\\",
    "\x1bP+q544e\x1b\\",
    "\x1b]0;title\x07",
    "\x1b]2;\x1b\\",
    "\x1b]8;;https://x.y\x1b\\link\x1b]8;;\x1b\\",
    "\x1b]8;id=1;file:///C:/a\x07",
    "\x1b]52;c;aGVsbG8=\x07",
    "\x1b]52;c;?\x07",
    "\x1b]9;hi\x07",
    "\x1b]9;9;\"C:\\x\"\x07",
    "\x1b]9;4;1;50\x07",
    "\x1b]777;notify;a;b\x07",
    "\x1b]7;file://h/tmp\x07",
    "\x1b]4;1;rgb:ff/00/00\x07",
    "\x1b]10;?\x07",
    "\x1b]11;#123456\x07",
    "\x1b]104\x07",
    "\x1b]1337;SetMark\x07",
    "\x1b]133;A\x07",
    "\x1b]133;B\x07",
    "\x1b]133;C\x07",
    "\x1b]133;D;0\x07",
    "\x1b]133;D;1\x07",
    "\x1b]133;D\x07",
    "\x1b]633;E;ls\\x3b -la\x07",
    "\x1b]633;E;\\\x07",
    "\x1b]633;E;\\x\x07",
    "\x1b]133;A\x07PS> \x1b]133;B\x07git push",
    "\x1b]133;A\x07$ \x1b]133;B\x07",
    "\x1bPq#0;2;100;0;0#0!20~-!20~\x1b\\",
    "\x1bPq\"1;1;4;4#1~~~~\x1b\\",
    "\x1bPq!99999~\x1b\\",
    "\x1bP0;1;0q#0;2;0;0;0\x1b\\",
    "\x1b_Ga=T,f=100,t=d,q=2,c=4,m=0;PNG\x1b\\",
    "\x1b_Ga=T,f=100,t=d,C=1,q=2,r=3,m=0;PNG\x1b\\",
    "\x1b_Ga=T,f=100,t=d,q=2,m=1;iVBOR\x1b\\",
    "\x1b_Gm=0;w0KGgo\x1b\\",
    "\x1b_Ga=d\x1b\\",
    "\x1b_Ga=d,d=A\x1b\\",
    "\x1b_Ga=p,i=1,c=999,r=999\x1b\\",
    "\x1b_Ga=T,f=32,s=1,v=1,t=d;AAAA\x1b\\",
    "\x1b_Ga=T,f=24,s=99999,v=99999;AA\x1b\\",
    "\x1b_Gbogus\x1b\\",
    "\x1b_G\x1b\\",
    "\x1b[?2026h",
    "\x1b[?2026l",
];

fn hostile(r: &mut Rng) -> String {
    let mut s = String::new();
    for _ in 0..1 + r.below(40) {
        match r.below(10) {
            0 => {
                // random bytes, made valid UTF-8 the lossy way
                let n = 1 + r.below(24);
                let b: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
                s.push_str(&String::from_utf8_lossy(&b));
            }
            1 => {
                // a CSI with random params and final byte
                s.push_str("\x1b[");
                if r.below(3) == 0 {
                    s.push(['?', '>', '<', '=', '!'][r.below(5)]);
                }
                for i in 0..r.below(6) {
                    if i > 0 {
                        s.push(if r.below(5) == 0 { ':' } else { ';' });
                    }
                    let v = match r.below(4) {
                        0 => 0,
                        1 => r.below(10),
                        2 => r.below(1000),
                        _ => r.next() as usize % 100_000,
                    };
                    s.push_str(&v.to_string());
                }
                s.push((0x40 + r.below(0x3f) as u8) as char);
            }
            2 => s.push_str(&"x".repeat(r.below(300))),
            3 => {
                // a long string sequence (OSC / DCS / APC), maybe unterminated
                let open = ["\x1b]0;", "\x1b]8;;", "\x1bP", "\x1b_G", "\x1b]52;c;", "\x1b]633;E;"][r.below(6)];
                s.push_str(open);
                s.push_str(&"A".repeat(r.below(5000)));
                if r.below(4) != 0 {
                    s.push_str(if r.below(2) == 0 { "\x07" } else { "\x1b\\" });
                }
            }
            _ => s.push_str(&r.pick(PIECES).replace("PNG", PNG)),
        }
    }
    s
}

/// Everything the app reads from a terminal, at assorted positions.
fn read_all(t: &mut Terminal, r: &mut Rng) {
    let (cols, rows) = (t.cols(), t.rows());
    assert!(t.cursor.x <= cols && t.cursor.y < rows, "cursor {:?} off a {cols}x{rows} screen", (t.cursor.x, t.cursor.y));
    let total = t.total_lines();
    let _ = t.screen_text();
    let _ = t.prompt_input();
    let _ = t.prompt_lines();
    let _ = t.running_command();
    let _ = t.last_finished_block();
    let _ = t.quick_targets();
    let _ = t.dump_vt(50);
    let _ = t.find("x");
    let _ = t.take_responses();
    let _ = t.take_finished();
    let _ = t.take_notifications();
    let _ = t.take_clipboard_writes();
    let _ = t.take_title_change().map(str::to_owned);
    for _ in 0..3 {
        let line = r.below(total + 3);
        let col = r.below(cols + 3);
        let _ = t.absolute_row(line);
        let _ = t.block_at(line);
        let _ = t.word_at(col, line);
        let _ = t.url_at(col, line);
        let _ = t.hyperlink_at(col, line);
        let _ = t.file_ref_at(col, line);
        let _ = t.logical_line_bounds(line.min(total.saturating_sub(1)));
        let _ = t.line_selection(line.min(total.saturating_sub(1)));
        let a = (r.below(cols + 2), r.below(total + 2));
        let b = (r.below(cols + 2), r.below(total + 2));
        let _ = t.text_range(a, b);
        let _ = t.text_block(a, b);
    }
    for i in 0..rows {
        let _ = t.visible_row(i);
        let _ = t.screen_row(i);
    }
    if let Some(b) = t.last_finished_block() {
        let _ = t.block_output(&b);
    }
    let re = regex::Regex::new("[A-Za-z]{3,}").unwrap();
    let _ = t.find_regex_wrapped(&re, 0..total);
}

fn run(seed: u64, steps: usize) {
    let mut r = Rng(seed | 1);
    let mut t = Terminal::new(1 + r.below(120), 1 + r.below(40));
    t.set_scrollback_limit(r.below(300));
    t.set_cell_pixels(1 + r.below(20) as u32, 1 + r.below(40) as u32);
    let mut p = Parser::new();
    for _ in 0..steps {
        match r.below(12) {
            0 => t.resize(1 + r.below(150), 1 + r.below(50)),
            1 => t.scroll_view(r.below(200) as isize - 100),
            2 => t.scroll_view_to_bottom(),
            3 => t.scroll_to_line(r.below(t.total_lines() + 5)),
            4 if r.below(20) == 0 => t.clear_scrollback(),
            5 if r.below(30) == 0 => t.seal_history(),
            _ => {
                let s = hostile(&mut r);
                // split at random byte offsets: sequences arrive in pieces
                let b = s.as_bytes();
                let mut i = 0;
                let t0 = std::time::Instant::now();
                while i < b.len() {
                    let n = 1 + r.below(b.len() - i);
                    p.advance_bytes(&mut t, &b[i..i + n]);
                    i += n;
                }
                if std::env::var_os("VIBEX_FUZZ_SLOW").is_some() && t0.elapsed().as_millis() > 20 {
                    let shown: String = s.escape_debug().to_string().chars().take(300).collect();
                    eprintln!("SLOW feed {:?} ({}x{}, {} bytes): {shown}", t0.elapsed(), t.cols(), t.rows(), b.len());
                }
            }
        }
        let t0 = std::time::Instant::now();
        read_all(&mut t, &mut r);
        if std::env::var_os("VIBEX_FUZZ_SLOW").is_some() && t0.elapsed().as_millis() > 20 {
            eprintln!("SLOW read {:?} ({}x{}, {} lines)", t0.elapsed(), t.cols(), t.rows(), t.total_lines());
        }
    }
}

#[test]
fn hostile_input_never_panics() {
    let iters: u64 = std::env::var("VIBEX_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(25);
    if let Some(seed) = std::env::var("VIBEX_FUZZ_SEED").ok().and_then(|v| v.parse().ok()) {
        run(seed, 200);
        return;
    }
    for seed in 1..=iters {
        let res = std::panic::catch_unwind(|| run(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), 200));
        if res.is_err() {
            panic!("seed {} panicked: replay with VIBEX_FUZZ_SEED={}", seed, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        }
    }
}
