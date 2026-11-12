use super::*;

#[derive(Default, Debug, PartialEq)]
struct Rec {
    out: Vec<String>,
}

impl Perform for Rec {
    fn print(&mut self, c: char) {
        self.out.push(format!("p:{c}"));
    }
    fn execute(&mut self, b: u8) {
        self.out.push(format!("x:{b:02x}"));
    }
    fn csi_dispatch(&mut self, params: &Params, inter: &[u8], ignore: bool, action: char) {
        let groups: Vec<String> = params.iter().map(|g| g.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(":")).collect();
        self.out.push(format!("csi:{}{}{}{}", String::from_utf8_lossy(inter), groups.join(";"), if ignore { "!" } else { "" }, action));
    }
    fn esc_dispatch(&mut self, inter: &[u8], _ignore: bool, b: u8) {
        self.out.push(format!("esc:{}{}", String::from_utf8_lossy(inter), b as char));
    }
    fn osc_dispatch(&mut self, params: &[&[u8]], bell: bool) {
        let p: Vec<_> = params.iter().map(|p| String::from_utf8_lossy(p).into_owned()).collect();
        self.out.push(format!("osc:{}{}", p.join("|"), if bell { "(bel)" } else { "" }));
    }
    fn hook(&mut self, _p: &Params, _i: &[u8], _ig: bool, action: char) {
        self.out.push(format!("hook:{action}"));
    }
    fn put(&mut self, b: u8) {
        self.out.push(format!("put:{}", b as char));
    }
    fn unhook(&mut self) {
        self.out.push("unhook".into());
    }
    fn apc_dispatch(&mut self, data: &[u8]) {
        self.out.push(format!("apc:{}", String::from_utf8_lossy(data)));
    }
}

#[test]
fn apc_string() {
    assert_eq!(parse(b"\x1b_Ga=T,f=24;AAAA\x1b\\x"), ["apc:Ga=T,f=24;AAAA", "esc:\\", "p:x"]);
    // SOS / PM are still swallowed
    assert_eq!(parse(b"\x1b^secret\x1b\\y"), ["esc:\\", "p:y"]);
}

fn parse(input: &[u8]) -> Vec<String> {
    let mut p = Parser::new();
    let mut r = Rec::default();
    p.advance_bytes(&mut r, input);
    r.out
}

#[test]
fn plain_text_and_controls() {
    assert_eq!(parse(b"hi\r\n"), ["p:h", "p:i", "x:0d", "x:0a"]);
}

#[test]
fn utf8_multibyte() {
    assert_eq!(parse("é→🦀".as_bytes()), ["p:é", "p:→", "p:🦀"]);
}

#[test]
fn utf8_invalid_and_truncated() {
    assert_eq!(parse(&[0xff, b'a']), ["p:\u{fffd}", "p:a"]);
    assert_eq!(parse(&[0xe2, 0x86, b'a']), ["p:\u{fffd}", "p:a"]);
    // overlong encoding of '/'
    assert_eq!(parse(&[0xe0, 0x80, 0xaf]), ["p:\u{fffd}"]);
}

#[test]
fn csi_basic() {
    assert_eq!(parse(b"\x1b[H"), ["csi:H"]);
    assert_eq!(parse(b"\x1b[12;34H"), ["csi:12;34H"]);
    assert_eq!(parse(b"\x1b[;5H"), ["csi:0;5H"]);
    assert_eq!(parse(b"\x1b[1;31m"), ["csi:1;31m"]);
}

#[test]
fn csi_private_and_intermediate() {
    assert_eq!(parse(b"\x1b[?1049h"), ["csi:?1049h"]);
    assert_eq!(parse(b"\x1b[>c"), ["csi:>c"]);
    assert_eq!(parse(b"\x1b[2 q"), ["csi: 2q"]);
}

#[test]
fn csi_subparams() {
    assert_eq!(parse(b"\x1b[38:2::255:0:0m"), ["csi:38:2:0:255:0:0m"]);
    assert_eq!(parse(b"\x1b[4:3;1m"), ["csi:4:3;1m"]);
}

#[test]
fn csi_control_inside_executes() {
    assert_eq!(parse(b"\x1b[1\n;2H"), ["x:0a", "csi:1;2H"]);
}

#[test]
fn csi_invalid_goes_to_ignore() {
    assert_eq!(parse(b"\x1b[1?2hX"), ["p:X"]);
}

#[test]
fn csi_too_many_params_sets_ignore() {
    let mut s = b"\x1b[".to_vec();
    for _ in 0..40 {
        s.extend_from_slice(b"1;");
    }
    s.push(b'm');
    let out = parse(&s);
    assert_eq!(out.len(), 1);
    assert!(out[0].ends_with("!m"));
}

#[test]
fn esc_sequences() {
    assert_eq!(parse(b"\x1b7\x1b8\x1bM"), ["esc:7", "esc:8", "esc:M"]);
    assert_eq!(parse(b"\x1b(B"), ["esc:(B"]);
    assert_eq!(parse(b"\x1b#8"), ["esc:#8"]);
}

#[test]
fn osc_bel_and_st() {
    assert_eq!(parse(b"\x1b]0;my title\x07"), ["osc:0|my title(bel)"]);
    assert_eq!(parse(b"\x1b]2;t\x1b\\"), ["osc:2|t", "esc:\\"]);
    assert_eq!(parse("\x1b]0;π\x07".as_bytes()), ["osc:0|π(bel)"]);
}

#[test]
fn osc_multiple_params() {
    assert_eq!(parse(b"\x1b]8;;https://x.y\x07"), ["osc:8||https://x.y(bel)"]);
}

#[test]
fn dcs_passthrough() {
    assert_eq!(parse(b"\x1bPqAB\x1b\\"), ["hook:q", "put:A", "put:B", "unhook", "esc:\\"]);
}

#[test]
fn can_aborts_sequence() {
    assert_eq!(parse(b"\x1b[12\x18A"), ["x:18", "p:A"]);
}

#[test]
fn esc_interrupts_utf8() {
    assert_eq!(parse(&[0xe2, 0x1b, b'7']), ["p:\u{fffd}", "esc:7"]);
}

#[test]
fn params_get_or() {
    let mut p = Params::default();
    p.push(0, false);
    p.push(7, false);
    assert_eq!(p.get_or(0, 1), 1);
    assert_eq!(p.get_or(1, 1), 7);
    assert_eq!(p.get_or(2, 1), 1);
    assert_eq!(p.raw(0), Some(0));
}
