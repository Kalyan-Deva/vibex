/// Maximum number of numeric parameters (including sub-parameters) kept.
pub const MAX_PARAMS: usize = 32;

/// CSI/DCS parameters. Values separated by `;` are separate parameters;
/// values separated by `:` are sub-parameters of the preceding one
/// (e.g. `38:2::255:0:0`).
#[derive(Clone, Debug)]
pub struct Params {
    values: [u16; MAX_PARAMS],
    is_sub: [bool; MAX_PARAMS],
    len: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self { values: [0; MAX_PARAMS], is_sub: [false; MAX_PARAMS], len: 0 }
    }
}

impl Params {
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Returns false if full.
    pub(crate) fn push(&mut self, value: u16, sub: bool) -> bool {
        if self.len == MAX_PARAMS {
            return false;
        }
        self.values[self.len] = value;
        self.is_sub[self.len] = sub && self.len > 0;
        self.len += 1;
        true
    }

    /// Total number of values (parameters + sub-parameters).
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Iterate parameter groups: each item is a parameter followed by its
    /// sub-parameters.
    pub fn iter(&self) -> ParamsIter<'_> {
        ParamsIter { params: self, pos: 0 }
    }

    /// First value of the `i`th parameter group, or `default` when absent
    /// or zero (VT semantics: 0 means "default").
    pub fn get_or(&self, i: usize, default: u16) -> u16 {
        match self.iter().nth(i).and_then(|g| g.first().copied()) {
            Some(0) | None => default,
            Some(v) => v,
        }
    }

    /// First value of the `i`th group, with zero preserved.
    pub fn raw(&self, i: usize) -> Option<u16> {
        self.iter().nth(i).and_then(|g| g.first().copied())
    }
}

pub struct ParamsIter<'a> {
    params: &'a Params,
    pos: usize,
}

impl<'a> Iterator for ParamsIter<'a> {
    type Item = &'a [u16];

    fn next(&mut self) -> Option<&'a [u16]> {
        let p = self.params;
        if self.pos >= p.len {
            return None;
        }
        let start = self.pos;
        let mut end = start + 1;
        while end < p.len && p.is_sub[end] {
            end += 1;
        }
        self.pos = end;
        Some(&p.values[start..end])
    }
}
