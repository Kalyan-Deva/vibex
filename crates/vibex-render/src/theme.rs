use vibex_term::Color;

/// Colours as 0x00RRGGBB.
#[derive(Clone, Debug)]
pub struct Theme {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub cursor_text: u32,
    pub selection: u32,
    pub search_match: u32,
    /// ANSI 0-15.
    pub palette: [u32; 16],
    /// Draw bold text with bright colours (classic xterm behaviour).
    pub bold_is_bright: bool,
}

impl Default for Theme {
    /// Tokyo Night, as in Windows Terminal's scheme of that name.
    fn default() -> Self {
        Theme::builtin("tokyo-night").expect("built-in")
    }
}

impl Theme {
    /// "Vibex Night": a warm dark theme (the default before Tokyo Night).
    fn vibex_night() -> Self {
        Self {
            foreground: 0xe6e1d8,
            background: 0x14131a,
            cursor: 0xf38020,
            cursor_text: 0x14131a,
            selection: 0x3a3550,
            search_match: 0x5c4320,
            palette: [
                0x1d1c24, // black
                0xe5534b, // red
                0x57ab5a, // green
                0xe0a84a, // yellow
                0x539bf5, // blue
                0xb083f0, // magenta
                0x39c5cf, // cyan
                0xcdd0d6, // white
                0x545d68, // bright black
                0xff7b72, // bright red
                0x7ee787, // bright green
                0xf6c15b, // bright yellow
                0x79c0ff, // bright blue
                0xd2a8ff, // bright magenta
                0x56d4dd, // bright cyan
                0xf0f3f6, // bright white
            ],
            bold_is_bright: true,
        }
    }

    pub const BUILTIN: &'static [&'static str] = &["tokyo-night", "vibex-night", "vibex-day", "vibex-ember"];

    /// A built-in theme by name.
    pub fn builtin(name: &str) -> Option<Theme> {
        match name {
            "vibex-night" => Some(Theme::vibex_night()),
            // light, paper-like
            "vibex-day" => Some(Theme {
                foreground: 0x2b2a33,
                background: 0xf6f3ec,
                cursor: 0xd9661f,
                cursor_text: 0xf6f3ec,
                selection: 0xd8d2e8,
                search_match: 0xf2d9a4,
                palette: [
                    0x3b3a44, 0xc0392b, 0x2e7d32, 0x9a6a00, 0x1f5fbf, 0x8e44ad, 0x00838f, 0xb8b3a8, 0x6e6a78, 0xe74c3c, 0x43a047, 0xb7860b, 0x2f74d0, 0xa55bc4,
                    0x0097a7, 0x2b2a33,
                ],
                bold_is_bright: false,
            }),
            // warm dark, orange accents
            "vibex-ember" => Some(Theme {
                foreground: 0xf1e3d3,
                background: 0x1a1412,
                cursor: 0xff8a3d,
                cursor_text: 0x1a1412,
                selection: 0x4a2f22,
                search_match: 0x6b3d14,
                palette: [
                    0x2a201c, 0xe0533d, 0x9fbf5a, 0xf2a541, 0x6c9bd2, 0xc67fb4, 0x63b8a8, 0xd9c8b4, 0x5c4a40, 0xff7259, 0xb8d977, 0xffc266, 0x8db7e8, 0xe09ccc,
                    0x82d4c4, 0xfff4e6,
                ],
                bold_is_bright: true,
            }),
            // Tokyo Night (as in Windows Terminal's scheme of that name)
            "tokyo-night" => Some(Theme {
                foreground: 0xc0caf5,
                background: 0x1a1b26,
                cursor: 0x7aa2f7,
                cursor_text: 0x1a1b26,
                selection: 0x283457,
                search_match: 0x3d4a75,
                palette: [
                    0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, 0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0x9d7cd8,
                    0x7dcfff, 0xc0caf5,
                ],
                bold_is_bright: true,
            }),
            _ => None,
        }
    }

    /// The same theme with default foreground and background swapped
    /// (DECSCNM reverse-video screen).
    pub fn reversed(&self) -> Theme {
        let mut t = self.clone();
        std::mem::swap(&mut t.foreground, &mut t.background);
        t.cursor_text = t.background;
        t
    }

    pub fn indexed(&self, i: u8) -> u32 {
        match i {
            0..=15 => self.palette[i as usize],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v as u32 * 40 };
                let r = level(i / 36);
                let g = level((i / 6) % 6);
                let b = level(i % 6);
                (r << 16) | (g << 8) | b
            }
            232..=255 => {
                let v = 8 + (i - 232) as u32 * 10;
                (v << 16) | (v << 8) | v
            }
        }
    }

    pub fn resolve(&self, c: Color, default: u32) -> u32 {
        match c {
            Color::Default => default,
            Color::Indexed(i) => self.indexed(i),
            Color::Rgb(r, g, b) => ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
        }
    }
}

#[inline]
pub fn blend(bg: u32, fg: u32, alpha: u8) -> u32 {
    if alpha == 255 {
        return fg;
    }
    if alpha == 0 {
        return bg;
    }
    let a = alpha as u32;
    let ia = 255 - a;
    let ch = |shift: u32| ((((fg >> shift) & 0xff) * a + ((bg >> shift) & 0xff) * ia + 127) / 255) << shift;
    ch(16) | ch(8) | ch(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grays() {
        let t = Theme::default();
        assert_eq!(t.indexed(16), 0x000000);
        assert_eq!(t.indexed(231), 0xffffff);
        assert_eq!(t.indexed(196), 0xff0000);
        assert_eq!(t.indexed(232), 0x080808);
        assert_eq!(t.indexed(255), 0xeeeeee);
    }

    #[test]
    fn default_is_tokyo_night() {
        assert_eq!(Theme::default().background, 0x1a1b26);
        assert_eq!(Theme::default().cursor, 0x7aa2f7);
        assert_eq!(Theme::builtin("vibex-night").unwrap().background, 0x14131a);
    }

    #[test]
    fn builtins_exist() {
        for name in Theme::BUILTIN {
            assert!(Theme::builtin(name).is_some(), "{name}");
        }
        assert!(Theme::builtin("nope").is_none());
    }

    #[test]
    fn blending() {
        assert_eq!(blend(0x000000, 0xffffff, 255), 0xffffff);
        assert_eq!(blend(0x000000, 0xffffff, 0), 0x000000);
        assert_eq!(blend(0x000000, 0xfefefe, 128), 0x7f7f7f);
    }
}
