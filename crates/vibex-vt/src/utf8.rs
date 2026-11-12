/// Incremental UTF-8 decoder. Invalid input becomes U+FFFD.
#[derive(Default)]
pub(crate) struct Utf8Decoder {
    cp: u32,
    need: u8,
    min: u32,
}

const REPL: char = char::REPLACEMENT_CHARACTER;

impl Utf8Decoder {
    /// Drop any partial sequence. Returns true if one was pending.
    pub(crate) fn reset(&mut self) -> bool {
        let pending = self.need != 0;
        self.need = 0;
        self.cp = 0;
        pending
    }

    pub(crate) fn feed(&mut self, b: u8, out: &mut impl FnMut(char)) {
        if self.need == 0 {
            self.start(b, out);
            return;
        }
        if b & 0xc0 == 0x80 {
            self.cp = (self.cp << 6) | (b & 0x3f) as u32;
            self.need -= 1;
            if self.need == 0 {
                let c = if self.cp < self.min { None } else { char::from_u32(self.cp) };
                out(c.unwrap_or(REPL));
            }
        } else {
            // truncated sequence: emit replacement and reprocess this byte
            self.need = 0;
            out(REPL);
            self.start(b, out);
        }
    }

    fn start(&mut self, b: u8, out: &mut impl FnMut(char)) {
        match b {
            0x00..=0x7f => out(b as char),
            0xc2..=0xdf => self.begin(b & 0x1f, 1, 0x80),
            0xe0..=0xef => self.begin(b & 0x0f, 2, 0x800),
            0xf0..=0xf4 => self.begin(b & 0x07, 3, 0x10000),
            _ => out(REPL),
        }
    }

    fn begin(&mut self, bits: u8, need: u8, min: u32) {
        self.cp = bits as u32;
        self.need = need;
        self.min = min;
    }
}
