#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
    /// The theme's default foreground or background (depends on use).
    #[default]
    Default,
    /// Palette index 0-255.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Cell attribute bit flags.
pub mod flags {
    pub const BOLD: u16 = 1 << 0;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const INVERSE: u16 = 1 << 4;
    pub const HIDDEN: u16 = 1 << 5;
    pub const STRIKE: u16 = 1 << 6;
    pub const BLINK: u16 = 1 << 7;
    pub const DOUBLE_UNDERLINE: u16 = 1 << 8;
    pub const CURLY_UNDERLINE: u16 = 1 << 9;
    /// First half of a double-width character.
    pub const WIDE: u16 = 1 << 10;
    /// Second half (placeholder) of a double-width character.
    pub const WIDE_SPACER: u16 = 1 << 11;
    /// Blank left in the last column when a wide char wrapped to the next
    /// row. Not part of the text (skipped when copying and reflowing).
    pub const WRAP_PAD: u16 = 1 << 12;

    pub const ANY_UNDERLINE: u16 = UNDERLINE | DOUBLE_UNDERLINE | CURLY_UNDERLINE;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: u16,
    /// OSC 8 hyperlink id (0 = none); see `links`.
    pub link: u16,
}

impl Default for Cell {
    fn default() -> Self {
        Self { c: ' ', fg: Color::Default, bg: Color::Default, flags: 0, link: 0 }
    }
}

impl Cell {
    /// A blank cell carrying only the background of `pen` (BCE behaviour).
    pub fn blank(pen: &Cell) -> Self {
        Self { bg: pen.bg, ..Self::default() }
    }

    pub fn has(&self, f: u16) -> bool {
        self.flags & f != 0
    }
}

/// Shell-integration mark on a prompt row (OSC 133).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptMark {
    /// Exit code of the command run from this prompt, once known.
    pub exit: Option<i32>,
    /// Id of the command record (`Terminal::block_at`), 0 = none.
    pub cmd: u32,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The line continues on the next row (soft wrap).
    pub wrapped: bool,
    /// Set on the row where a shell prompt starts.
    pub prompt: Option<PromptMark>,
    /// Parts of inline images drawn on this row.
    pub images: Vec<crate::image::ImageSlice>,
    /// A command's output starts on this row (OSC 133;C).
    pub output: bool,
}

impl Row {
    pub fn new(cols: usize, fill: Cell) -> Self {
        Self { cells: vec![fill; cols], wrapped: false, prompt: None, images: Vec::new(), output: false }
    }

    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::default());
        if let Some(last) = self.cells.last_mut() {
            if last.has(flags::WIDE) {
                *last = Cell::default();
            }
        }
    }

    pub fn reset(&mut self, fill: Cell) {
        self.cells.fill(fill);
        self.wrapped = false;
        self.prompt = None;
        self.images.clear();
        self.output = false;
    }

    /// Text content with trailing blanks trimmed.
    pub fn text(&self) -> String {
        let s: String = self.cells.iter().filter(|c| !c.has(flags::WIDE_SPACER | flags::WRAP_PAD)).map(|c| crate::grapheme::text(c.c)).collect();
        s.trim_end().to_string()
    }
}
