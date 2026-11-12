//! Mouse reporting to applications (xterm X10/normal and SGR 1006 encodings).

use winit::keyboard::ModifiersState;

pub const LEFT: u8 = 0;
pub const MIDDLE: u8 = 1;
pub const RIGHT: u8 = 2;
/// Motion with no button held.
pub const NONE: u8 = 3;
pub const WHEEL_UP: u8 = 64;
pub const WHEEL_DOWN: u8 = 65;

/// Encode one mouse event. `col`/`row` are 0-based visible cells.
pub fn encode(button: u8, pressed: bool, motion: bool, mods: ModifiersState, col: usize, row: usize, sgr: bool) -> Option<Vec<u8>> {
    let mut code = button;
    if motion {
        code += 32;
    }
    if mods.shift_key() {
        code += 4;
    }
    if mods.alt_key() {
        code += 8;
    }
    if mods.control_key() {
        code += 16;
    }
    let (x, y) = (col + 1, row + 1);
    if sgr {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    // legacy encoding: releases are button 3, coordinates capped at 223
    if !pressed {
        code = (code & !3) | 3;
    }
    if x > 223 || y > 223 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + code, 32 + x as u8, 32 + y as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_press_release() {
        let m = ModifiersState::empty();
        assert_eq!(encode(LEFT, true, false, m, 0, 0, true).unwrap(), b"\x1b[<0;1;1M");
        assert_eq!(encode(LEFT, false, false, m, 9, 4, true).unwrap(), b"\x1b[<0;10;5m");
        assert_eq!(encode(WHEEL_UP, true, false, m, 0, 0, true).unwrap(), b"\x1b[<64;1;1M");
    }

    #[test]
    fn sgr_motion_with_modifiers() {
        let m = ModifiersState::CONTROL;
        assert_eq!(encode(LEFT, true, true, m, 2, 3, true).unwrap(), b"\x1b[<48;3;4M");
    }

    #[test]
    fn legacy_encoding() {
        let m = ModifiersState::empty();
        assert_eq!(encode(RIGHT, true, false, m, 0, 0, false).unwrap(), vec![0x1b, b'[', b'M', 34, 33, 33]);
        // release -> button 3
        assert_eq!(encode(RIGHT, false, false, m, 0, 0, false).unwrap(), vec![0x1b, b'[', b'M', 35, 33, 33]);
        assert!(encode(LEFT, true, false, m, 300, 0, false).is_none());
    }
}
