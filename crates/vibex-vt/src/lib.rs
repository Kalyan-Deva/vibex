//! A DEC/ANSI escape-sequence parser.
//!
//! Implements the state machine described by Paul Williams
//! (<https://vt100.net/emu/dec_ansi_parser>) with a UTF-8 decoder in the
//! ground state. C1 controls are not recognised as 8-bit bytes (UTF-8 mode,
//! same as xterm), so bytes >= 0x80 are always text.

mod params;
mod utf8;

pub use params::{Params, ParamsIter, MAX_PARAMS};
use utf8::Utf8Decoder;

/// Receives parsed events.
pub trait Perform {
    /// A printable character in the ground state.
    fn print(&mut self, c: char);
    /// A C0 control (BEL, BS, HT, LF, CR, ...).
    fn execute(&mut self, byte: u8);
    /// `CSI <intermediates/private markers> <params> <action>`.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char);
    /// `ESC <intermediates> <byte>`.
    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8);
    /// `OSC <param>;<param>... (BEL | ST)`.
    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool);
    /// Start of a DCS string.
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    /// A byte of DCS data.
    fn put(&mut self, _byte: u8) {}
    /// End of a DCS string.
    fn unhook(&mut self) {}
    /// `APC <data> ST` (kitty graphics uses `APC G ...`). Data longer than
    /// the parser's limit is dropped.
    fn apc_dispatch(&mut self, _data: &[u8]) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    OscString,
    ApcString,
    SosPmApcString,
}

const MAX_INTERMEDIATES: usize = 2;
const MAX_OSC_LEN: usize = 64 * 1024;
const MAX_OSC_PARAMS: usize = 16;
/// Kitty sends images in 4 KiB chunks, but a single unchunked transmission
/// can be large; cap it so a runaway program can't eat memory.
const MAX_APC_LEN: usize = 16 * 1024 * 1024;

pub struct Parser {
    state: State,
    intermediates: [u8; MAX_INTERMEDIATES],
    inter_len: usize,
    ignoring: bool,
    params: Params,
    param: u32,
    param_seen: bool,
    next_is_sub: bool,
    osc: Vec<u8>,
    osc_splits: Vec<usize>,
    apc: Vec<u8>,
    apc_overflow: bool,
    utf8: Utf8Decoder,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            intermediates: [0; MAX_INTERMEDIATES],
            inter_len: 0,
            ignoring: false,
            params: Params::default(),
            param: 0,
            param_seen: false,
            next_is_sub: false,
            osc: Vec::new(),
            osc_splits: Vec::new(),
            apc: Vec::new(),
            apc_overflow: false,
            utf8: Utf8Decoder::default(),
        }
    }

    /// Feed a slice of bytes.
    pub fn advance_bytes<P: Perform>(&mut self, p: &mut P, bytes: &[u8]) {
        for &b in bytes {
            self.advance(p, b);
        }
    }

    /// Feed one byte.
    pub fn advance<P: Perform>(&mut self, p: &mut P, byte: u8) {
        // Ground-state text is the hot path.
        if self.state == State::Ground && byte >= 0x20 && byte != 0x7f {
            self.utf8.feed(byte, &mut |c| p.print(c));
            return;
        }

        // "Anywhere" transitions.
        match byte {
            0x18 | 0x1a => {
                self.flush_utf8(p);
                self.leave_string(p, false);
                p.execute(byte);
                self.state = State::Ground;
                return;
            }
            0x1b => {
                self.flush_utf8(p);
                self.leave_string(p, false);
                self.clear();
                self.state = State::Escape;
                return;
            }
            _ => {}
        }

        match self.state {
            State::Ground => {
                if byte < 0x20 {
                    self.flush_utf8(p);
                    p.execute(byte);
                }
                // 0x7f (DEL) ignored
            }
            State::Escape => match byte {
                0x00..=0x1f => p.execute(byte),
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::EscapeIntermediate;
                }
                b'[' => {
                    self.clear();
                    self.state = State::CsiEntry;
                }
                b']' => {
                    self.osc.clear();
                    self.osc_splits.clear();
                    self.state = State::OscString;
                }
                b'P' => {
                    self.clear();
                    self.state = State::DcsEntry;
                }
                b'_' => {
                    self.apc.clear();
                    self.apc_overflow = false;
                    self.state = State::ApcString;
                }
                b'X' | b'^' => self.state = State::SosPmApcString,
                0x30..=0x7e => {
                    p.esc_dispatch(self.inter(), self.ignoring, byte);
                    self.state = State::Ground;
                }
                _ => {}
            },
            State::EscapeIntermediate => match byte {
                0x00..=0x1f => p.execute(byte),
                0x20..=0x2f => self.collect(byte),
                0x30..=0x7e => {
                    p.esc_dispatch(self.inter(), self.ignoring, byte);
                    self.state = State::Ground;
                }
                _ => {}
            },
            State::CsiEntry => match byte {
                0x00..=0x1f => p.execute(byte),
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::CsiIntermediate;
                }
                b'0'..=b'9' | b';' | b':' => {
                    self.param_byte(byte);
                    self.state = State::CsiParam;
                }
                0x3c..=0x3f => {
                    self.collect(byte);
                    self.state = State::CsiParam;
                }
                0x40..=0x7e => self.csi_dispatch(p, byte),
                _ => {}
            },
            State::CsiParam => match byte {
                0x00..=0x1f => p.execute(byte),
                b'0'..=b'9' | b';' | b':' => self.param_byte(byte),
                0x3c..=0x3f => self.state = State::CsiIgnore,
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::CsiIntermediate;
                }
                0x40..=0x7e => self.csi_dispatch(p, byte),
                _ => {}
            },
            State::CsiIntermediate => match byte {
                0x00..=0x1f => p.execute(byte),
                0x20..=0x2f => self.collect(byte),
                0x30..=0x3f => self.state = State::CsiIgnore,
                0x40..=0x7e => self.csi_dispatch(p, byte),
                _ => {}
            },
            State::CsiIgnore => match byte {
                0x00..=0x1f => p.execute(byte),
                0x40..=0x7e => self.state = State::Ground,
                _ => {}
            },
            State::DcsEntry => match byte {
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::DcsIntermediate;
                }
                b'0'..=b'9' | b';' | b':' => {
                    self.param_byte(byte);
                    self.state = State::DcsParam;
                }
                0x3c..=0x3f => {
                    self.collect(byte);
                    self.state = State::DcsParam;
                }
                0x40..=0x7e => self.dcs_hook(p, byte),
                _ => {}
            },
            State::DcsParam => match byte {
                b'0'..=b'9' | b';' | b':' => self.param_byte(byte),
                0x3c..=0x3f => self.state = State::DcsIgnore,
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::DcsIntermediate;
                }
                0x40..=0x7e => self.dcs_hook(p, byte),
                _ => {}
            },
            State::DcsIntermediate => match byte {
                0x20..=0x2f => self.collect(byte),
                0x30..=0x3f => self.state = State::DcsIgnore,
                0x40..=0x7e => self.dcs_hook(p, byte),
                _ => {}
            },
            State::DcsPassthrough => {
                if byte != 0x7f {
                    p.put(byte);
                }
            }
            State::DcsIgnore => {}
            State::OscString => match byte {
                0x07 => {
                    self.osc_dispatch(p, true);
                    self.state = State::Ground;
                }
                0x00..=0x1f => {}
                b';' if self.osc_splits.len() < MAX_OSC_PARAMS - 1 => {
                    self.osc_splits.push(self.osc.len());
                }
                _ => {
                    if self.osc.len() < MAX_OSC_LEN {
                        self.osc.push(byte);
                    }
                }
            },
            State::ApcString => match byte {
                0x00..=0x1f => {}
                _ => {
                    if self.apc.len() < MAX_APC_LEN {
                        self.apc.push(byte);
                    } else {
                        self.apc_overflow = true;
                    }
                }
            },
            State::SosPmApcString => {}
        }
    }

    /// Emit a replacement char if a UTF-8 sequence was cut short by a control.
    fn flush_utf8<P: Perform>(&mut self, p: &mut P) {
        if self.utf8.reset() {
            p.print(char::REPLACEMENT_CHARACTER);
        }
    }

    /// Called when ESC/CAN/SUB interrupts a string state.
    fn leave_string<P: Perform>(&mut self, p: &mut P, bell: bool) {
        match self.state {
            State::OscString => self.osc_dispatch(p, bell),
            State::DcsPassthrough => p.unhook(),
            State::ApcString => {
                if !self.apc_overflow {
                    p.apc_dispatch(&self.apc);
                }
                // don't keep a multi-megabyte buffer around
                if self.apc.capacity() > 64 * 1024 {
                    self.apc = Vec::new();
                } else {
                    self.apc.clear();
                }
            }
            _ => {}
        }
    }

    fn clear(&mut self) {
        self.inter_len = 0;
        self.ignoring = false;
        self.params.clear();
        self.param = 0;
        self.param_seen = false;
        self.next_is_sub = false;
    }

    fn inter(&self) -> &[u8] {
        &self.intermediates[..self.inter_len]
    }

    fn collect(&mut self, byte: u8) {
        if self.inter_len == MAX_INTERMEDIATES {
            self.ignoring = true;
        } else {
            self.intermediates[self.inter_len] = byte;
            self.inter_len += 1;
        }
    }

    fn param_byte(&mut self, byte: u8) {
        self.param_seen = true;
        match byte {
            b';' | b':' => {
                self.push_param();
                self.next_is_sub = byte == b':';
            }
            d => {
                self.param = (self.param * 10 + (d - b'0') as u32).min(u16::MAX as u32);
            }
        }
    }

    fn push_param(&mut self) {
        if !self.params.push(self.param as u16, self.next_is_sub) {
            self.ignoring = true;
        }
        self.param = 0;
    }

    fn finish_params(&mut self, seen_any: bool) {
        if seen_any {
            self.push_param();
        }
    }

    fn csi_dispatch<P: Perform>(&mut self, p: &mut P, byte: u8) {
        self.finish_params(self.param_seen);
        p.csi_dispatch(&self.params, self.inter(), self.ignoring, byte as char);
        self.state = State::Ground;
    }

    fn dcs_hook<P: Perform>(&mut self, p: &mut P, byte: u8) {
        self.finish_params(self.param_seen);
        p.hook(&self.params, self.inter(), self.ignoring, byte as char);
        self.state = State::DcsPassthrough;
    }

    fn osc_dispatch<P: Perform>(&mut self, p: &mut P, bell: bool) {
        let mut parts: [&[u8]; MAX_OSC_PARAMS] = [&[]; MAX_OSC_PARAMS];
        let mut n = 0;
        let mut start = 0;
        for &split in &self.osc_splits {
            parts[n] = &self.osc[start..split];
            n += 1;
            start = split;
        }
        parts[n] = &self.osc[start..];
        n += 1;
        p.osc_dispatch(&parts[..n], bell);
    }
}

#[cfg(test)]
mod tests;
