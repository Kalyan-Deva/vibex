//! Keyboard → VT byte encoding (xterm conventions).

use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn mod_param(m: ModifiersState) -> u8 {
    1 + m.shift_key() as u8 + 2 * m.alt_key() as u8 + 4 * m.control_key() as u8
}

fn csi_letter(letter: char, m: u8, app_cursor: bool) -> Vec<u8> {
    if m == 1 {
        if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    } else {
        format!("\x1b[1;{m}{letter}").into_bytes()
    }
}

fn tilde(code: u8, m: u8) -> Vec<u8> {
    if m == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{m}~").into_bytes()
    }
}

fn ss3(letter: char, m: u8) -> Vec<u8> {
    if m == 1 {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[1;{m}{letter}").into_bytes()
    }
}

fn with_alt(alt: bool, mut bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Control byte for Ctrl+<c>, if any.
pub fn ctrl_byte(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '/' | '7' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

pub fn encode(event: &KeyEvent, mods: ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
    let m = mod_param(mods);
    let (ctrl, alt, shift) = (mods.control_key(), mods.alt_key(), mods.shift_key());

    if let Key::Named(named) = &event.logical_key {
        let bytes = match named {
            NamedKey::Enter => with_alt(alt, b"\r".to_vec()),
            NamedKey::Backspace => with_alt(alt, vec![if ctrl { 0x08 } else { 0x7f }]),
            NamedKey::Tab => {
                if shift {
                    b"\x1b[Z".to_vec()
                } else {
                    with_alt(alt, b"\t".to_vec())
                }
            }
            NamedKey::Escape => b"\x1b".to_vec(),
            NamedKey::Space => with_alt(alt, vec![if ctrl { 0 } else { b' ' }]),
            NamedKey::ArrowUp => csi_letter('A', m, app_cursor),
            NamedKey::ArrowDown => csi_letter('B', m, app_cursor),
            NamedKey::ArrowRight => csi_letter('C', m, app_cursor),
            NamedKey::ArrowLeft => csi_letter('D', m, app_cursor),
            NamedKey::Home => csi_letter('H', m, app_cursor),
            NamedKey::End => csi_letter('F', m, app_cursor),
            NamedKey::Insert => tilde(2, m),
            NamedKey::Delete => tilde(3, m),
            NamedKey::PageUp => tilde(5, m),
            NamedKey::PageDown => tilde(6, m),
            NamedKey::F1 => ss3('P', m),
            NamedKey::F2 => ss3('Q', m),
            NamedKey::F3 => ss3('R', m),
            NamedKey::F4 => ss3('S', m),
            NamedKey::F5 => tilde(15, m),
            NamedKey::F6 => tilde(17, m),
            NamedKey::F7 => tilde(18, m),
            NamedKey::F8 => tilde(19, m),
            NamedKey::F9 => tilde(20, m),
            NamedKey::F10 => tilde(21, m),
            NamedKey::F11 => tilde(23, m),
            NamedKey::F12 => tilde(24, m),
            _ => return None,
        };
        return Some(bytes);
    }

    let text = event.text.as_ref().map(|t| t.as_str());

    if ctrl {
        let base = match &event.logical_key {
            Key::Character(s) => s.chars().next(),
            _ => None,
        };
        // AltGr arrives as Ctrl+Alt: if it produced printable text, send that.
        if alt {
            if let Some(t) = text.filter(|t| t.chars().all(|c| !c.is_control())) {
                return Some(t.as_bytes().to_vec());
            }
        }
        if let Some(b) = base.and_then(ctrl_byte) {
            return Some(with_alt(alt, vec![b]));
        }
    }

    let t = match (text, &event.logical_key) {
        (Some(t), _) if !t.is_empty() => t.to_string(),
        (_, Key::Character(s)) => s.to_string(),
        _ => return None,
    };
    Some(with_alt(alt && !ctrl, t.into_bytes()))
}

/// The kitty keyboard protocol
/// (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>).
pub mod kitty {
    #[allow(dead_code)] // the encoder treats every non-zero flag set as disambiguating
    pub const DISAMBIGUATE: u8 = 1;
    pub const REPORT_EVENTS: u8 = 2;
    pub const ALTERNATES: u8 = 4;
    pub const ALL_KEYS: u8 = 8;
    pub const TEXT: u8 = 16;

    pub const PRESS: u8 = 1;
    pub const REPEAT: u8 = 2;
    pub const RELEASE: u8 = 3;

    /// Modifier bits as the protocol counts them (value sent is 1 + bits).
    pub const SHIFT: u8 = 1;
    pub const ALT: u8 = 2;
    pub const CTRL: u8 = 4;
    pub const SUPER: u8 = 8;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum KKey {
        /// A text-producing key, by its unshifted character.
        Char(char),
        /// Keys sent as `CSI code u`: Escape 27, Enter 13, Tab 9, Backspace 127.
        Code(u32),
        /// Keys sent as `CSI 1;m X`: arrows, Home/End, F1/F2/F4.
        Letter(char),
        /// Keys sent as `CSI n;m ~`: Insert, Delete, PageUp/Down, F3, F5-F12.
        Tilde(u32),
        /// A modifier key itself (only reported with ALL_KEYS).
        Modifier(u32),
    }

    fn mods_part(mods: u8, event: u8) -> String {
        let m = 1 + mods as u32;
        if event > PRESS {
            format!("{m}:{event}")
        } else {
            m.to_string()
        }
    }

    fn csi_u(code: u32, alternate: Option<u32>, mods: u8, event: u8, text: Option<&str>) -> Vec<u8> {
        let mut s = format!("\x1b[{code}");
        if let Some(a) = alternate {
            s.push_str(&format!(":{a}"));
        }
        if mods != 0 || event > PRESS || text.is_some() {
            s.push(';');
            s.push_str(&mods_part(mods, event));
        }
        if let Some(t) = text {
            let cps: Vec<String> = t.chars().map(|c| (c as u32).to_string()).collect();
            s.push(';');
            s.push_str(&cps.join(":"));
        }
        s.push('u');
        s.into_bytes()
    }

    /// Encode a key event. `None` = use the legacy encoding; `Some(empty)`
    /// = send nothing (e.g. an unreported release).
    pub fn encode(key: KKey, mods: u8, event: u8, shifted: Option<char>, text: Option<&str>, flags: u8) -> Option<Vec<u8>> {
        let all = flags & ALL_KEYS != 0;
        let events = flags & REPORT_EVENTS != 0;
        if event == RELEASE && !events {
            return Some(Vec::new());
        }
        let ev = if events { event } else { PRESS };
        match key {
            KKey::Modifier(code) => {
                if all {
                    Some(csi_u(code, None, mods, ev, None))
                } else {
                    Some(Vec::new())
                }
            }
            KKey::Code(code) => {
                let legacy_text_key = matches!(code, 13 | 9 | 127);
                if legacy_text_key && !all {
                    if event == RELEASE {
                        return Some(Vec::new());
                    }
                    if mods == 0 {
                        return None; // \r, \t, DEL as usual
                    }
                }
                Some(csi_u(code, None, mods, ev, None))
            }
            KKey::Letter(l) => {
                Some(if mods == 0 && ev == PRESS { format!("\x1b[{l}").into_bytes() } else { format!("\x1b[1;{}{l}", mods_part(mods, ev)).into_bytes() })
            }
            KKey::Tilde(n) => {
                Some(if mods == 0 && ev == PRESS { format!("\x1b[{n}~").into_bytes() } else { format!("\x1b[{n};{}~", mods_part(mods, ev)).into_bytes() })
            }
            KKey::Char(c) => {
                let base = c.to_lowercase().next().unwrap_or(c) as u32;
                let alternate = (flags & ALTERNATES != 0 && mods & SHIFT != 0).then_some(shifted).flatten().map(|s| s as u32).filter(|&s| s != base);
                if all {
                    let text =
                        (flags & TEXT != 0 && event != RELEASE).then_some(text).flatten().filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control()));
                    return Some(csi_u(base, alternate, mods, ev, text));
                }
                if event == RELEASE {
                    return Some(Vec::new()); // text key releases need ALL_KEYS
                }
                if mods & (CTRL | ALT | SUPER) != 0 {
                    Some(csi_u(base, alternate, mods, ev, None))
                } else {
                    None // plain or shifted text: send the text itself
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn e(key: KKey, mods: u8, flags: u8) -> Option<String> {
            encode(key, mods, PRESS, None, None, flags).map(|b| String::from_utf8(b).unwrap())
        }

        #[test]
        fn disambiguate() {
            let f = DISAMBIGUATE;
            assert_eq!(e(KKey::Code(27), 0, f).as_deref(), Some("\x1b[27u"));
            assert_eq!(e(KKey::Char('c'), CTRL, f).as_deref(), Some("\x1b[99;5u"));
            assert_eq!(e(KKey::Char('i'), CTRL, f).as_deref(), Some("\x1b[105;5u"), "Ctrl+I is not Tab");
            assert_eq!(e(KKey::Char('a'), ALT, f).as_deref(), Some("\x1b[97;3u"));
            assert_eq!(e(KKey::Char('a'), 0, f), None, "plain text stays text");
            assert_eq!(e(KKey::Char('a'), SHIFT, f), None);
            assert_eq!(e(KKey::Code(13), 0, f), None, "Enter stays \\r");
            assert_eq!(e(KKey::Code(13), SHIFT, f).as_deref(), Some("\x1b[13;2u"));
            assert_eq!(e(KKey::Letter('A'), 0, f).as_deref(), Some("\x1b[A"));
            assert_eq!(e(KKey::Letter('C'), CTRL | SHIFT, f).as_deref(), Some("\x1b[1;6C"));
            assert_eq!(e(KKey::Tilde(3), 0, f).as_deref(), Some("\x1b[3~"));
            assert_eq!(e(KKey::Tilde(5), CTRL, f).as_deref(), Some("\x1b[5;5~"));
        }

        #[test]
        fn releases_and_repeats() {
            let f = DISAMBIGUATE | REPORT_EVENTS;
            let r = |k, m| encode(k, m, RELEASE, None, None, f).map(|b| String::from_utf8(b).unwrap());
            assert_eq!(r(KKey::Code(27), 0).as_deref(), Some("\x1b[27;1:3u"));
            assert_eq!(r(KKey::Letter('A'), 0).as_deref(), Some("\x1b[1;1:3A"));
            assert_eq!(r(KKey::Char('a'), 0).as_deref(), Some(""), "text releases need ALL_KEYS");
            let rep = encode(KKey::Char('x'), CTRL, REPEAT, None, None, f).unwrap();
            assert_eq!(rep, b"\x1b[120;5:2u");
            // without REPORT_EVENTS, releases are swallowed
            assert_eq!(encode(KKey::Code(27), 0, RELEASE, None, None, DISAMBIGUATE).unwrap(), b"");
        }

        #[test]
        fn all_keys_alternates_and_text() {
            let f = ALL_KEYS | ALTERNATES | TEXT;
            let s = encode(KKey::Char('a'), SHIFT, PRESS, Some('A'), Some("A"), f).unwrap();
            assert_eq!(s, b"\x1b[97:65;2;65u");
            assert_eq!(encode(KKey::Char('a'), 0, PRESS, None, Some("a"), ALL_KEYS).unwrap(), b"\x1b[97u");
            assert_eq!(encode(KKey::Code(13), 0, PRESS, None, None, ALL_KEYS).unwrap(), b"\x1b[13u");
            assert_eq!(encode(KKey::Modifier(57441), SHIFT, PRESS, None, None, ALL_KEYS).unwrap(), b"\x1b[57441;2u");
            assert_eq!(encode(KKey::Modifier(57441), SHIFT, PRESS, None, None, DISAMBIGUATE).unwrap(), b"");
        }
    }
}

/// Map a winit key event onto the kitty encoder.
pub fn encode_kitty(event: &KeyEvent, mods: ModifiersState, flags: u8) -> Option<Vec<u8>> {
    use kitty::KKey;
    use winit::keyboard::{KeyCode, PhysicalKey};
    use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

    let base = event.key_without_modifiers();
    let key = match &base {
        Key::Named(n) => match n {
            NamedKey::Escape => KKey::Code(27),
            NamedKey::Enter => KKey::Code(13),
            NamedKey::Tab => KKey::Code(9),
            NamedKey::Backspace => KKey::Code(127),
            NamedKey::Space => KKey::Char(' '),
            NamedKey::Insert => KKey::Tilde(2),
            NamedKey::Delete => KKey::Tilde(3),
            NamedKey::PageUp => KKey::Tilde(5),
            NamedKey::PageDown => KKey::Tilde(6),
            NamedKey::ArrowUp => KKey::Letter('A'),
            NamedKey::ArrowDown => KKey::Letter('B'),
            NamedKey::ArrowRight => KKey::Letter('C'),
            NamedKey::ArrowLeft => KKey::Letter('D'),
            NamedKey::Home => KKey::Letter('H'),
            NamedKey::End => KKey::Letter('F'),
            NamedKey::F1 => KKey::Letter('P'),
            NamedKey::F2 => KKey::Letter('Q'),
            NamedKey::F3 => KKey::Tilde(13),
            NamedKey::F4 => KKey::Letter('S'),
            NamedKey::F5 => KKey::Tilde(15),
            NamedKey::F6 => KKey::Tilde(17),
            NamedKey::F7 => KKey::Tilde(18),
            NamedKey::F8 => KKey::Tilde(19),
            NamedKey::F9 => KKey::Tilde(20),
            NamedKey::F10 => KKey::Tilde(21),
            NamedKey::F11 => KKey::Tilde(23),
            NamedKey::F12 => KKey::Tilde(24),
            NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super => {
                let code = match event.physical_key {
                    PhysicalKey::Code(KeyCode::ShiftLeft) => 57441,
                    PhysicalKey::Code(KeyCode::ControlLeft) => 57442,
                    PhysicalKey::Code(KeyCode::AltLeft) => 57443,
                    PhysicalKey::Code(KeyCode::SuperLeft) => 57444,
                    PhysicalKey::Code(KeyCode::ShiftRight) => 57447,
                    PhysicalKey::Code(KeyCode::ControlRight) => 57448,
                    PhysicalKey::Code(KeyCode::AltRight) => 57449,
                    PhysicalKey::Code(KeyCode::SuperRight) => 57450,
                    _ => return None,
                };
                KKey::Modifier(code)
            }
            _ => return None,
        },
        Key::Character(s) => KKey::Char(s.chars().next()?),
        _ => return None,
    };
    let mut m = 0;
    if mods.shift_key() {
        m |= kitty::SHIFT;
    }
    if mods.alt_key() {
        m |= kitty::ALT;
    }
    if mods.control_key() {
        m |= kitty::CTRL;
    }
    if mods.super_key() {
        m |= kitty::SUPER;
    }
    let ev = match (event.state, event.repeat) {
        (winit::event::ElementState::Released, _) => kitty::RELEASE,
        (_, true) => kitty::REPEAT,
        _ => kitty::PRESS,
    };
    let shifted = match &event.logical_key {
        Key::Character(s) => s.chars().next(),
        _ => None,
    };
    kitty::encode(key, m, ev, shifted, event.text.as_ref().map(|t| t.as_str()), flags)
}

/// Prepare clipboard text for the PTY: normalise newlines to CR and wrap in
/// bracketed-paste markers when the application asked for them.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // strip any embedded end marker so pasted text can't escape the bracket
        let safe = normalized.replace("\x1b[201~", "");
        format!("\x1b[200~{safe}\x1b[201~").into_bytes()
    } else {
        normalized.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_bytes() {
        assert_eq!(ctrl_byte('c'), Some(3));
        assert_eq!(ctrl_byte('C'), Some(3));
        assert_eq!(ctrl_byte('['), Some(0x1b));
        assert_eq!(ctrl_byte(' '), Some(0));
        assert_eq!(ctrl_byte('é'), None);
    }

    #[test]
    fn modifier_sequences() {
        assert_eq!(csi_letter('A', 1, false), b"\x1b[A");
        assert_eq!(csi_letter('A', 1, true), b"\x1bOA");
        assert_eq!(csi_letter('C', 5, true), b"\x1b[1;5C");
        assert_eq!(tilde(3, 1), b"\x1b[3~");
        assert_eq!(tilde(5, 2), b"\x1b[5;2~");
        assert_eq!(ss3('P', 1), b"\x1bOP");
    }

    #[test]
    fn paste() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("x", true), b"\x1b[200~x\x1b[201~");
        assert_eq!(paste_bytes("x\x1b[201~rm", true), b"\x1b[200~xrm\x1b[201~");
    }
}
