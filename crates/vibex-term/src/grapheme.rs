//! Grapheme clusters in a `Copy` cell.
//!
//! A cell holds one `char`. When a cluster needs more than one code point
//! (base + combining marks, ZWJ emoji, flags, skin tones, VS16), the full
//! string is interned in a process-wide table and the cell stores a
//! code point from Supplementary Private Use Area-B pointing at it. Real
//! PUA-B characters printed by programs are interned too, so a PUA-B cell
//! always means "look it up". The table only grows with *distinct*
//! clusters, which stays tiny in practice; when full, clusters degrade to
//! their base character.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

const BASE: u32 = 0x10_0000;
const LIMIT: u32 = 0x10_FFFD;

#[derive(Default)]
struct Interner {
    strings: Vec<Box<str>>,
    ids: HashMap<Box<str>, u32>,
}

fn table() -> &'static RwLock<Interner> {
    static T: OnceLock<RwLock<Interner>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// Is `c` an interned cluster handle?
pub fn is_cluster(c: char) -> bool {
    (BASE..=LIMIT).contains(&(c as u32))
}

/// The handle for `s` (interning it on first use).
pub fn intern(s: &str) -> char {
    if let Some(&id) = table().read().unwrap().ids.get(s) {
        return char::from_u32(BASE + id).unwrap();
    }
    let mut t = table().write().unwrap();
    if let Some(&id) = t.ids.get(s) {
        return char::from_u32(BASE + id).unwrap();
    }
    let id = t.strings.len() as u32;
    if BASE + id > LIMIT {
        // table full: keep the base character
        return s.chars().next().unwrap_or(' ');
    }
    t.strings.push(s.into());
    t.ids.insert(s.into(), id);
    char::from_u32(BASE + id).unwrap()
}

/// The text a cell's character stands for.
pub fn text(c: char) -> String {
    if is_cluster(c) {
        if let Some(s) = table().read().unwrap().strings.get((c as u32 - BASE) as usize) {
            return s.to_string();
        }
    }
    c.to_string()
}

/// Append to the text of `c`, returning the new handle.
pub fn append(c: char, extra: char) -> char {
    let mut s = text(c);
    s.push(extra);
    intern(&s)
}

/// The first code point (for classification / search).
pub fn base(c: char) -> char {
    if is_cluster(c) {
        text(c).chars().next().unwrap_or(' ')
    } else {
        c
    }
}

pub const ZWJ: char = '\u{200D}';
pub const VS16: char = '\u{FE0F}';
pub const VS15: char = '\u{FE0E}';

pub fn is_regional_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

pub fn is_emoji_modifier(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// Pictographic bases that take modifiers / ZWJ joins.
pub fn is_pictographic(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x2190..=0x21FF | 0x2300..=0x23FF) || c == '\u{00A9}' || c == '\u{00AE}'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_round_trips() {
        let a = intern("e\u{301}");
        let b = intern("e\u{301}");
        assert_eq!(a, b);
        assert!(is_cluster(a));
        assert_eq!(text(a), "e\u{301}");
        assert_eq!(base(a), 'e');
        assert_eq!(text('x'), "x");
        let c = append(a, '\u{323}');
        assert_eq!(text(c), "e\u{301}\u{323}");
    }
}
