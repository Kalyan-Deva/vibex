//! Text queries over the buffer: word/line bounds, URL detection, search.
//! All positions are absolute `(col, line)` (line 0 = oldest history row).

use crate::{flags, Row, Terminal};

/// A detected hyperlink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub start: (usize, usize),
    pub end: (usize, usize),
}

/// A file reference in output (compiler errors, grep, stack traces).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRef {
    pub path: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub start: (usize, usize),
    pub end: (usize, usize),
}

/// "src/main.rs:12:5" / "a.cpp(12,5):" / "C:\x\y.py:3" / "./run.sh" ->
/// (path, line, col). Needs a directory separator or a file extension.
pub fn parse_file_ref(token: &str) -> Option<(String, Option<u32>, Option<u32>)> {
    let t = token.trim_end_matches([',', '.', ':', ';', ')']).trim_start_matches(['(', '@']);
    let num = |s: &str| s.parse::<u32>().ok().filter(|n| *n > 0);
    // MSVC: path(line,col) or path(line)
    let (path, line, col) = if let Some(open) = t.rfind('(').filter(|_| token.contains(')')) {
        let inner = token[open + 1..].split(')').next().unwrap_or("");
        let mut it = inner.split(',');
        (t[..open].to_string(), it.next().and_then(num), it.next().and_then(num))
    } else {
        // path:line:col, path:line; don't split a drive letter (C:\...)
        let mut parts: Vec<&str> = t.split(':').collect();
        let mut nums: Vec<u32> = Vec::new();
        while parts.len() > 1 {
            match parts.last().and_then(|p| num(p)) {
                Some(n) if nums.len() < 2 => {
                    nums.insert(0, n);
                    parts.pop();
                }
                _ => break,
            }
        }
        (parts.join(":"), nums.first().copied(), nums.get(1).copied())
    };
    if path.is_empty() || path.contains("://") || path.len() > 400 {
        return None;
    }
    let file = path.rsplit(['/', '\\']).next().unwrap_or(&path);
    let has_sep = path.contains('/') || path.contains('\\');
    let has_ext = file.rfind('.').is_some_and(|d| d > 0 && d + 1 < file.len() && file[d + 1..].chars().all(|c| c.is_ascii_alphanumeric()));
    if !(has_sep || has_ext) || file.is_empty() {
        return None;
    }
    // "1.2.3" or "3.14" are versions / numbers, not files
    if path.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    Some((path, line, col))
}

/// A search hit on one line (`start..=end` columns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Something quick-select can copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Url,
    Path,
    Hash,
    Ip,
    Uuid,
}

/// A quick-select candidate on one line (`start..=end` columns).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub kind: TargetKind,
}

/// Spans (char indices, end exclusive) of http(s) URLs in `text`.
fn url_spans(text: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest: String = text[i..text.len().min(i + 8)].iter().collect();
        let scheme = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else {
            i += 1;
            continue;
        };
        let mut end = i + scheme;
        while end < text.len() && is_url_char(text[end]) {
            end += 1;
        }
        while end > i + scheme {
            let c = text[end - 1];
            let s: String = text[i..end].iter().collect();
            let unbalanced = |open: char, close: char| c == close && s.matches(open).count() < s.matches(close).count();
            if ".,;:!?".contains(c) || unbalanced('(', ')') || unbalanced('[', ']') {
                end -= 1;
            } else {
                break;
            }
        }
        if end > i + scheme {
            out.push((i, end));
        }
        i = end.max(i + 1);
    }
    out
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What kind of quick-select target a token is, if any.
pub fn classify(t: &str) -> Option<TargetKind> {
    if t.len() < 3 {
        return None;
    }
    let b = t.as_bytes();
    // UUID 8-4-4-4-12
    if t.len() == 36 && [8, 13, 18, 23].iter().all(|&i| b[i] == b'-') && is_hex(&t.replace('-', "")) {
        return Some(TargetKind::Uuid);
    }
    // IPv4 with optional :port
    let (host, port) = t.split_once(':').map_or((t, None), |(h, p)| (h, Some(p)));
    let octets: Vec<&str> = host.split('.').collect();
    if octets.len() == 4
        && octets.iter().all(|o| !o.is_empty() && o.len() <= 3 && o.bytes().all(|c| c.is_ascii_digit()) && o.parse::<u16>().is_ok_and(|v| v <= 255))
        && port.is_none_or(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
    {
        return Some(TargetKind::Ip);
    }
    // git hash / sha: 7-40 or 64 hex with both letters and digits
    if (7..=40).contains(&t.len()) || t.len() == 64 {
        let has_digit = b.iter().any(|c| c.is_ascii_digit());
        let has_alpha = b.iter().any(|c| matches!(c, b'a'..=b'f' | b'A'..=b'F'));
        if is_hex(t) && has_digit && has_alpha {
            return Some(TargetKind::Hash);
        }
    }
    // paths
    let starts_like_path = t.starts_with("~/")
        || t.starts_with("./")
        || t.starts_with("../")
        || t.starts_with(".\\")
        || t.starts_with("..\\")
        || t.starts_with("\\\\")
        || (t.starts_with('/') && t.len() > 1 && b[1] != b'/')
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'));
    if starts_like_path {
        return Some(TargetKind::Path);
    }
    let seps = t.matches(['/', '\\']).count();
    if seps >= 1 && !t.contains("://") {
        let segments: Vec<&str> = t.split(['/', '\\']).collect();
        let solid = segments.iter().all(|s| !s.is_empty() && s.chars().any(|c| c.is_alphanumeric()));
        let last_has_ext = segments.last().is_some_and(|s| s.contains('.'));
        if solid && (seps >= 2 || last_has_ext) {
            return Some(TargetKind::Path);
        }
    }
    None
}

/// Characters that split quick-select tokens.
const TOKEN_DELIMITERS: &str = " \t\"'`()[]{}<>|,;=";

/// Characters that end a word on double-click.
const WORD_DELIMITERS: &str = " \t\"'`()[]{}<>|,;│";

fn char_class(c: char) -> u8 {
    if c == ' ' || c == '\t' {
        0
    } else if WORD_DELIMITERS.contains(c) {
        1
    } else {
        2
    }
}

/// Chars of a row with the column each starts at (wide spacers and wrap
/// padding skipped).
fn row_chars(row: &Row) -> impl Iterator<Item = (char, usize)> + '_ {
    row.cells.iter().enumerate().filter(|(_, c)| !c.has(flags::WIDE_SPACER | flags::WRAP_PAD)).map(|(x, c)| (crate::grapheme::base(c.c), x))
}

impl Terminal {
    /// Like `find_regex_in`, but a soft-wrapped line is searched as one
    /// (a match can cross rows); each match comes back as one piece per
    /// row it covers. Starts at the beginning of the logical line that
    /// `lines.start` is in.
    pub fn find_regex_wrapped(&self, re: &regex::Regex, lines: std::ops::Range<usize>) -> Vec<Match> {
        let mut out = Vec::new();
        let end = lines.end.min(self.total_lines());
        // back up to where the line holding `lines.start` begins
        let mut line = lines.start;
        while line > 0 && line + 64 > lines.start && self.absolute_row(line - 1).is_some_and(|r| r.wrapped) {
            line -= 1;
        }
        let mut hay = String::new();
        let mut at: Vec<(usize, usize, usize)> = Vec::new(); // (byte, line, column)
        while line < end {
            hay.clear();
            at.clear();
            // gather one logical line
            loop {
                let Some(row) = self.absolute_row(line) else { break };
                for (c, x) in row_chars(row) {
                    at.push((hay.len(), line, x));
                    hay.push(c);
                }
                line += 1;
                if !row.wrapped || line >= self.total_lines() || line - lines.start > 400 {
                    break;
                }
            }
            if hay.trim().is_empty() {
                continue;
            }
            let pos = |byte: usize| match at.binary_search_by_key(&byte, |&(b, _, _)| b) {
                Ok(i) => at[i],
                Err(i) => at[i.saturating_sub(1)],
            };
            for m in re.find_iter(&hay) {
                if m.is_empty() || out.len() >= 10_000 {
                    continue;
                }
                let (_, l0, c0) = pos(m.start());
                let last = hay[..m.end()].char_indices().last().map_or(m.start(), |(i, _)| i);
                let (_, l1, c1) = pos(last);
                for l in l0..=l1 {
                    let start = if l == l0 { c0 } else { 0 };
                    let stop = if l == l1 {
                        let wide = self.absolute_row(l).and_then(|r| r.cells.get(c1)).is_some_and(|c| c.has(flags::WIDE)) as usize;
                        c1 + wide
                    } else {
                        self.absolute_row(l).map_or(0, |r| r.cells.len().saturating_sub(1))
                    };
                    if l >= lines.start && l < end {
                        out.push(Match { line: l, start, end: stop });
                    }
                }
            }
        }
        out
    }
}

fn is_url_char(c: char) -> bool {
    c.is_ascii_graphic() && !matches!(c, '<' | '>' | '"' | '\'' | '`' | '{' | '}' | '|' | '\\' | '^')
}

impl Terminal {
    /// Scroll the view so `line` is visible (roughly centred).
    pub fn scroll_to_line(&mut self, line: usize) {
        if self.alt_active {
            return;
        }
        let top = self.top_visible_line();
        if line >= top && line < top + self.rows {
            return;
        }
        let sb = self.scrollback.len();
        let target_top = line.saturating_sub(self.rows / 2);
        self.display_offset = sb.saturating_sub(target_top).min(sb);
    }

    /// Scroll the view so absolute `line` is the top row (clamped).
    pub fn set_view_top(&mut self, line: usize) {
        if self.alt_active {
            return;
        }
        let sb = self.scrollback.len();
        self.display_offset = sb.saturating_sub(line).min(sb);
    }

    /// First and last absolute rows of the logical (soft-wrapped) line
    /// containing `line`.
    pub fn logical_line_bounds(&self, line: usize) -> (usize, usize) {
        let mut first = line;
        while first > 0 && self.absolute_row(first - 1).is_some_and(|r| r.wrapped) {
            first -= 1;
        }
        let mut last = line;
        while self.absolute_row(last).is_some_and(|r| r.wrapped) && last + 1 < self.total_lines() {
            last += 1;
        }
        (first, last)
    }

    /// Word under `(col, line)` for double-click selection.
    pub fn word_at(&self, col: usize, line: usize) -> ((usize, usize), (usize, usize)) {
        let Some(row) = self.absolute_row(line) else { return ((col, line), (col, line)) };
        let mut col = col.min(row.cells.len().saturating_sub(1));
        if col > 0 && row.cells[col].has(flags::WIDE_SPACER) {
            col -= 1;
        }
        let class = char_class(row.cells[col].c);
        let same = |x: usize| {
            let c = &row.cells[x];
            c.has(flags::WIDE_SPACER) || char_class(c.c) == class
        };
        let mut start = col;
        while start > 0 && same(start - 1) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < row.cells.len() && same(end + 1) {
            end += 1;
        }
        ((start, line), (end, line))
    }

    /// Whole logical line for triple-click selection.
    pub fn line_selection(&self, line: usize) -> ((usize, usize), (usize, usize)) {
        let (first, last) = self.logical_line_bounds(line);
        ((0, first), (self.cols.saturating_sub(1), last))
    }

    /// An `http(s)://` URL under `(col, line)`, following soft wraps.
    pub fn url_at(&self, col: usize, line: usize) -> Option<Link> {
        // an explicit OSC 8 link wins over a URL spotted in the text
        if let Some(l) = self.hyperlink_at(col, line) {
            return Some(l);
        }
        let (first, last) = self.logical_line_bounds(line);
        let mut chars: Vec<(char, usize, usize)> = Vec::new();
        for l in first..=last {
            let row = self.absolute_row(l)?;
            chars.extend(row_chars(row).map(|(c, x)| (c, x, l)));
        }
        let text: Vec<char> = chars.iter().map(|c| c.0).collect();
        // index of the char under the mouse (a wide char covers two columns)
        let hit = chars.iter().position(|&(_, x, l)| {
            let wide = self.absolute_row(l).is_some_and(|r| r.cells[x].has(flags::WIDE));
            l == line && (x == col || (wide && x + 1 == col))
        })?;

        let mut i = 0;
        while i < text.len() {
            let rest: String = text[i..text.len().min(i + 8)].iter().collect();
            let scheme = if rest.starts_with("https://") {
                8
            } else if rest.starts_with("http://") {
                7
            } else {
                i += 1;
                continue;
            };
            let mut end = i + scheme;
            while end < text.len() && is_url_char(text[end]) {
                end += 1;
            }
            // trim trailing punctuation and unbalanced closing brackets
            while end > i + scheme {
                let c = text[end - 1];
                let s: String = text[i..end].iter().collect();
                let unbalanced = |open: char, close: char| c == close && s.matches(open).count() < s.matches(close).count();
                if ".,;:!?".contains(c) || unbalanced('(', ')') || unbalanced('[', ']') {
                    end -= 1;
                } else {
                    break;
                }
            }
            if end > i + scheme && hit >= i && hit < end {
                let url: String = text[i..end].iter().collect();
                let s = chars[i];
                let e = chars[end - 1];
                let wide_end = self.absolute_row(e.2).and_then(|r| r.cells.get(e.1)).is_some_and(|c| c.has(flags::WIDE));
                return Some(Link { url, start: (s.1, s.2), end: (e.1 + wide_end as usize, e.2) });
            }
            i = end.max(i + 1);
        }
        None
    }

    /// URLs, paths, hashes, IPs and UUIDs on the visible screen, top to
    /// bottom.
    pub fn quick_targets(&self) -> Vec<Target> {
        let top = self.top_visible_line();
        let mut out = Vec::new();
        for line in top..top + self.rows {
            let Some(row) = self.absolute_row(line) else { break };
            let chars: Vec<(char, usize)> = row_chars(row).collect();
            let text: Vec<char> = chars.iter().map(|c| c.0).collect();
            let urls = url_spans(&text);
            let mut push = |s: usize, e: usize, kind: TargetKind| {
                let last = chars[e - 1].1;
                let wide = row.cells[last].has(flags::WIDE) as usize;
                out.push(Target { line, start: chars[s].1, end: last + wide, text: text[s..e].iter().collect(), kind });
            };
            let mut i = 0;
            while i < text.len() {
                if let Some(&(s, e)) = urls.iter().find(|(s, _)| *s == i) {
                    push(s, e, TargetKind::Url);
                    i = e;
                    continue;
                }
                if TOKEN_DELIMITERS.contains(text[i]) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < text.len() && !TOKEN_DELIMITERS.contains(text[i]) && !urls.iter().any(|(s, _)| *s == i) {
                    i += 1;
                }
                // trim punctuation that usually isn't part of the thing
                let mut s = start;
                let mut e = i;
                while e > s && ".,:;!?".contains(text[e - 1]) {
                    e -= 1;
                }
                while s < e && ":".contains(text[s]) {
                    s += 1;
                }
                if e > s {
                    let tok: String = text[s..e].iter().collect();
                    if let Some(kind) = classify(&tok) {
                        push(s, e, kind);
                    }
                }
            }
        }
        out
    }

    /// Case-insensitive search over history + screen, oldest first.
    /// Regex matches on every line (each row is searched on its own).
    /// Empty matches are skipped.
    pub fn find_regex(&self, re: &regex::Regex) -> Vec<Match> {
        self.find_regex_in(re, 0..self.total_lines())
    }

    /// Regex matches on the absolute lines in `lines`.
    pub fn find_regex_in(&self, re: &regex::Regex, lines: std::ops::Range<usize>) -> Vec<Match> {
        let mut out = Vec::new();
        let mut hay = String::new();
        let mut cols: Vec<(usize, usize)> = Vec::new(); // (byte offset, column)
        for line in lines.start..lines.end.min(self.total_lines()) {
            let Some(row) = self.absolute_row(line) else { break };
            hay.clear();
            cols.clear();
            for (c, x) in row_chars(row) {
                cols.push((hay.len(), x));
                hay.push(c);
            }
            let col_at = |byte: usize| match cols.binary_search_by_key(&byte, |&(b, _)| b) {
                Ok(i) => cols[i].1,
                Err(i) => cols[i.saturating_sub(1)].1,
            };
            for m in re.find_iter(&hay) {
                if m.is_empty() || out.len() >= 100_000 {
                    continue;
                }
                let start = col_at(m.start());
                // last char of the match
                let last_byte = hay[..m.end()].char_indices().last().map_or(m.start(), |(i, _)| i);
                let last = col_at(last_byte);
                let wide = row.cells.get(last).is_some_and(|c| c.has(flags::WIDE)) as usize;
                out.push(Match { line, start, end: last + wide });
            }
        }
        out
    }

    /// Text of the rectangle spanned by two (col, line) corners, one line
    /// per row, trailing blanks trimmed.
    pub fn text_block(&self, a: (usize, usize), b: (usize, usize)) -> String {
        let (c0, c1) = (a.0.min(b.0), a.0.max(b.0));
        let (l0, l1) = (a.1.min(b.1), a.1.max(b.1));
        let mut lines = Vec::new();
        for line in l0..=l1 {
            let Some(row) = self.absolute_row(line) else { break };
            let to = (c1 + 1).min(row.cells.len());
            let s: String = row
                .cells
                .get(c0.min(to)..to)
                .unwrap_or(&[])
                .iter()
                .filter(|c| !c.has(flags::WIDE_SPACER | flags::WRAP_PAD))
                .map(|c| crate::grapheme::text(c.c))
                .collect();
            lines.push(s.trim_end().to_string());
        }
        lines.join("\n")
    }

    /// A `path:line[:col]` (or `path(line,col)`, MSVC style) under
    /// (col, line), for opening in an editor. The path isn't checked here.
    pub fn file_ref_at(&self, col: usize, line: usize) -> Option<FileRef> {
        let row = self.absolute_row(line)?;
        let chars: Vec<(char, usize)> = row_chars(row).collect();
        let i = chars.iter().position(|&(_, x)| x >= col).filter(|&i| chars[i].1 == col || i > 0)?;
        let stop = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '|' | '[' | ']' | '{' | '}');
        if stop(chars[i].0) {
            return None;
        }
        let mut s = i;
        while s > 0 && !stop(chars[s - 1].0) {
            s -= 1;
        }
        let mut e = i;
        while e + 1 < chars.len() && !stop(chars[e + 1].0) {
            e += 1;
        }
        let token: String = chars[s..=e].iter().map(|c| c.0).collect();
        let parsed = parse_file_ref(&token)?;
        Some(FileRef { path: parsed.0, line: parsed.1, col: parsed.2, start: (chars[s].1, line), end: (chars[e].1, line) })
    }

    pub fn find(&self, query: &str) -> Vec<Match> {
        let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
        let mut out = Vec::new();
        if needle.is_empty() {
            return out;
        }
        for line in 0..self.total_lines() {
            let Some(row) = self.absolute_row(line) else { break };
            let hay: Vec<(char, usize)> = row_chars(row).map(|(c, x)| (c.to_lowercase().next().unwrap_or(c), x)).collect();
            if hay.len() < needle.len() {
                continue;
            }
            let mut i = 0;
            while i + needle.len() <= hay.len() {
                if hay[i..i + needle.len()].iter().zip(&needle).all(|(h, n)| h.0 == *n) {
                    let last = hay[i + needle.len() - 1].1;
                    let wide = row.cells[last].has(flags::WIDE) as usize;
                    out.push(Match { line, start: hay[i].1, end: last + wide });
                    i += needle.len();
                } else {
                    i += 1;
                }
            }
        }
        out
    }
}
