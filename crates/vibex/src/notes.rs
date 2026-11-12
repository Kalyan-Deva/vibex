//! A notes panel beside a pane: plain text you keep while you work (the
//! plan, findings, commands to try). Saved per folder in
//! `~/.config/vibex/notes/`, so the same project shows the same notes.

use std::path::PathBuf;

pub struct Notes {
    pub text: String,
    /// Byte offset of the caret.
    pub cursor: usize,
    pub path: PathBuf,
    /// Keys go here instead of the shell.
    pub focused: bool,
    /// Where it was drawn (set by layout).
    pub rect: vibex_render::PxRect,
    /// What it's for ("~/code/vibex").
    pub title: String,
}

fn dir() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("notes"))
}

/// A file name for a folder: readable tail + a short hash of the whole.
pub fn file_for(folder: &str) -> Option<PathBuf> {
    let tail: String = folder
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("notes")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(40)
        .collect();
    // FNV-1a: stable across runs and versions
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in folder.to_lowercase().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    let tail = if tail.is_empty() { "root".to_string() } else { tail };
    dir().map(|d| d.join(format!("{tail}-{:08x}.md", h as u32)))
}

impl Notes {
    pub fn open(folder: Option<&str>) -> Notes {
        let key = folder.unwrap_or("scratch");
        let path = file_for(key).unwrap_or_else(|| PathBuf::from("vibex-notes.md"));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let title = folder.map_or("scratch".to_string(), |f| {
            let parts: Vec<&str> = f.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
            if parts.len() > 2 {
                format!("…/{}", parts[parts.len() - 2..].join("/"))
            } else {
                f.to_string()
            }
        });
        Notes { cursor: text.len(), text, path, focused: true, rect: Default::default(), title }
    }

    pub fn save(&self) {
        if let Some(d) = self.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&self.path, &self.text);
    }

    pub fn insert(&mut self, s: &str) {
        let s: String = s.chars().filter(|c| *c == '\n' || *c == '\t' || !c.is_control()).collect();
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
        self.save();
    }

    pub fn backspace(&mut self) {
        if let Some((i, _)) = self.text[..self.cursor].char_indices().last() {
            self.text.replace_range(i..self.cursor, "");
            self.cursor = i;
            self.save();
        }
    }

    pub fn delete(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.text.replace_range(self.cursor..self.cursor + c.len_utf8(), "");
            self.save();
        }
    }

    pub fn left(&mut self) {
        if let Some((i, _)) = self.text[..self.cursor].char_indices().last() {
            self.cursor = i;
        }
    }

    pub fn right(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    fn line_start(&self, at: usize) -> usize {
        self.text[..at].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, at: usize) -> usize {
        self.text[at..].find('\n').map_or(self.text.len(), |i| at + i)
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start(self.cursor);
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end(self.cursor);
    }

    /// Up / down a line, keeping the column (in characters).
    pub fn vertical(&mut self, down: bool) {
        let start = self.line_start(self.cursor);
        let col = self.text[start..self.cursor].chars().count();
        let target = if down {
            let end = self.line_end(self.cursor);
            if end >= self.text.len() {
                return;
            }
            end + 1
        } else {
            if start == 0 {
                return;
            }
            self.line_start(start - 1)
        };
        let end = self.line_end(target);
        let mut at = target;
        for (n, (i, c)) in self.text[target..end].char_indices().enumerate() {
            if n == col {
                break;
            }
            at = target + i + c.len_utf8();
        }
        self.cursor = at.min(end);
    }

    /// Wrap to `cols`: each shown line and the byte range it covers, plus
    /// the caret's (line, column).
    pub fn wrap(&self, cols: usize) -> (Vec<String>, (usize, usize)) {
        let cols = cols.max(4);
        let mut lines = Vec::new();
        let mut caret = (0, 0);
        let mut offset = 0;
        for raw in self.text.split('\n') {
            let chars: Vec<(usize, char)> = raw.char_indices().collect();
            let mut i = 0;
            loop {
                let take = (chars.len() - i).min(cols);
                // break at a space if the line goes on
                let mut cut = take;
                if i + take < chars.len() {
                    if let Some(sp) = chars[i..i + take].iter().rposition(|(_, c)| *c == ' ') {
                        if sp > 0 {
                            cut = sp + 1;
                        }
                    }
                }
                let seg: String = chars[i..i + cut].iter().map(|(_, c)| *c).collect();
                let seg_start = offset + chars.get(i).map_or(raw.len(), |(b, _)| *b);
                let seg_end = offset + chars.get(i + cut).map_or(raw.len(), |(b, _)| *b);
                if self.cursor >= seg_start && (self.cursor < seg_end || (self.cursor == seg_end && i + cut >= chars.len())) {
                    caret = (lines.len(), self.text[seg_start..self.cursor].chars().count());
                }
                lines.push(seg);
                i += cut;
                if i >= chars.len() {
                    break;
                }
            }
            offset += raw.len() + 1;
        }
        (lines, caret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(text: &str) -> Notes {
        Notes {
            text: text.into(),
            cursor: text.len(),
            path: std::env::temp_dir().join("vibex-notes-test-ignore.md"),
            focused: true,
            rect: Default::default(),
            title: String::new(),
        }
    }

    #[test]
    fn editing() {
        let mut x = n("ab");
        x.left();
        x.text.insert(x.cursor, 'X');
        x.cursor += 1;
        assert_eq!(x.text, "aXb");
        x.backspace();
        assert_eq!((x.text.as_str(), x.cursor), ("ab", 1));
        x.delete();
        assert_eq!(x.text, "a");
        let mut y = n("first line\nsecond");
        y.vertical(false);
        assert_eq!(&y.text[..y.cursor], "first ");
        y.vertical(true);
        assert_eq!(y.cursor, y.text.len());
        y.home();
        assert_eq!(&y.text[y.cursor..], "second");
        let _ = std::fs::remove_file(&y.path);
    }

    #[test]
    fn wrapping_and_caret() {
        let x = n("one two three four\nfive");
        let (lines, caret) = x.wrap(9);
        assert_eq!(lines, vec!["one two ", "three ", "four", "five"]);
        assert_eq!(caret, (3, 4));
        let mut y = n("abcdefghij");
        y.cursor = 4;
        let (lines, caret) = y.wrap(4);
        assert_eq!(lines, vec!["abcd", "efgh", "ij"]);
        assert_eq!(caret, (1, 0));
        assert_eq!(n("").wrap(10), (vec![String::new()], (0, 0)));
    }

    #[test]
    fn file_names_are_stable_and_safe() {
        let a = file_for("C:\\Users\\me\\code\\vibex").unwrap();
        let b = file_for("c:\\users\\me\\code\\vibex").unwrap();
        assert_eq!(a, b);
        let name = a.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("vibex-") && name.ends_with(".md"), "{name}");
        assert_ne!(file_for("/srv/app").unwrap(), file_for("/home/app").unwrap());
    }
}
