//! Command blocks: a prompt, the command typed at it and its output, from
//! shell-integration marks (OSC 133 A/B/C/D, plus VS Code's OSC 633;E for
//! the exact command text).
//!
//! A prompt row carries a `PromptMark` with a command id; the id maps to
//! what's known about that command (text, how long it ran, exit code). A
//! block runs from its prompt row to the row before the next prompt.

use crate::{PromptMark, Terminal};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandInfo {
    /// What was run ("" if unknown).
    pub text: String,
    pub started: Option<Instant>,
    pub duration: Option<Duration>,
    pub exit: Option<i32>,
}

/// A command that just finished (for "long command done" notifications).
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub text: String,
    pub exit: Option<i32>,
    pub duration: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Absolute lines: the prompt row, the first output row, the last row.
    pub prompt: usize,
    pub output: usize,
    pub end: usize,
    pub command: CommandInfo,
}

#[derive(Default)]
pub(crate) struct BlockState {
    next_id: u32,
    pub(crate) commands: BTreeMap<u32, CommandInfo>,
    /// Where typing began (OSC 133;B): absolute line, column.
    input_at: Option<(usize, usize)>,
    /// Exact command text from OSC 633;E.
    pending_text: Option<String>,
    /// The command currently running (between C and D).
    running: Option<u32>,
    finished: Vec<Finished>,
}

const MAX_COMMANDS: usize = 2000;

/// VS Code's escaping in OSC 633;E: `\\` and `\xHH`.
pub(crate) fn unescape_633(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            if b[i + 1] == b'\\' {
                out.push(b'\\');
                i += 2;
                continue;
            }
            if b[i + 1] == b'x' && i + 3 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                    out.push(v);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What's typed at the shell prompt so far (needs OSC 133;B).
#[derive(Clone, Debug, PartialEq)]
pub struct PromptInput {
    /// From where typing began up to the cursor.
    pub text: String,
    /// Nothing after the cursor on its line (so a suggestion can follow).
    pub at_end: bool,
}

impl Terminal {
    /// The prompt's input, while one is being typed (not while a command
    /// runs, nor in full-screen apps).
    pub fn prompt_input(&self) -> Option<PromptInput> {
        if self.alt_active || self.blocks.running.is_some() || self.display_offset() != 0 {
            return None;
        }
        let (l, c) = self.blocks.input_at?;
        let (cx, cl) = (self.cursor.x, self.cursor_line());
        if cl < l || (cl == l && cx < c) || cl - l > 8 {
            return None;
        }
        let blank = |cell: &crate::Cell| crate::grapheme::text(cell.c).trim().is_empty();
        let mut text = String::new();
        for line in l..=cl {
            let row = self.absolute_row(line)?;
            let from = if line == l { c } else { 0 };
            let to = if line == cl { cx } else { row.cells.len() };
            for cell in row.cells.get(from..to.min(row.cells.len())).unwrap_or(&[]) {
                if !cell.has(crate::flags::WIDE_SPACER | crate::flags::WRAP_PAD) {
                    text.push_str(&crate::grapheme::text(cell.c));
                }
            }
            if line < cl && !row.wrapped {
                // multi-line input: leave it alone
                return None;
            }
        }
        let row = self.absolute_row(cl)?;
        let at_end = !row.wrapped && row.cells.get(cx..).unwrap_or(&[]).iter().all(blank);
        Some(PromptInput { text, at_end })
    }

    fn cursor_line(&self) -> usize {
        self.scrollback.len() + self.cursor.y
    }

    /// The prompt mark nearest above `line` (with its absolute line).
    fn prompt_above(&self, line: usize, limit: usize) -> Option<(usize, PromptMark)> {
        (line.saturating_sub(limit)..=line).rev().find_map(|l| self.absolute_row(l).and_then(|r| r.prompt).map(|m| (l, m)))
    }

    /// OSC 133;A.
    pub(crate) fn mark_prompt(&mut self) {
        if self.alt_active {
            return;
        }
        let b = &mut self.blocks;
        b.next_id = b.next_id.wrapping_add(1).max(1);
        let id = b.next_id;
        b.commands.insert(id, CommandInfo::default());
        while b.commands.len() > MAX_COMMANDS {
            let first = *b.commands.keys().next().unwrap();
            b.commands.remove(&first);
        }
        b.input_at = None;
        let y = self.cursor.y;
        self.primary[y].prompt = Some(PromptMark { exit: None, cmd: id });
    }

    /// OSC 133;B: the prompt ends, typing starts here.
    pub(crate) fn mark_input(&mut self) {
        if !self.alt_active {
            self.blocks.input_at = Some((self.cursor_line(), self.cursor.x));
        }
    }

    /// OSC 633;E: the exact command line.
    pub(crate) fn set_command_text(&mut self, text: String) {
        self.blocks.pending_text = Some(text);
    }

    /// OSC 133;C: the command runs; its output starts on this row.
    pub(crate) fn mark_output(&mut self) {
        if self.alt_active {
            return;
        }
        let line = self.cursor_line();
        let y = self.cursor.y;
        self.primary[y].output = true;
        let Some((_, mark)) = self.prompt_above(line, 500) else { return };
        let text = match self.blocks.pending_text.take() {
            Some(t) => t,
            // no exact text: read what was typed between B and here
            None => match self.blocks.input_at.take() {
                Some((l, c)) if l <= line => {
                    let end = if self.cursor.x == 0 && line > l { (self.cols - 1, line - 1) } else { (self.cursor.x, line) };
                    self.text_range((c, l), end).trim().replace('\n', " ")
                }
                _ => String::new(),
            },
        };
        if let Some(info) = self.blocks.commands.get_mut(&mark.cmd) {
            info.text = text;
            info.started = Some(Instant::now());
        }
        self.blocks.running = Some(mark.cmd);
    }

    /// OSC 133;D: the last command finished; attach its exit code to the
    /// most recent prompt above the cursor.
    pub(crate) fn finish_command(&mut self, exit: Option<i32>) {
        self.last_exit = exit;
        if self.alt_active {
            return;
        }
        let start = self.cursor_line();
        let found = (start.saturating_sub(5000)..=start).rev().find(|&l| self.absolute_row(l).and_then(|r| r.prompt).is_some_and(|m| m.exit.is_none()));
        let Some(line) = found else { return };
        let exit = exit.or(Some(0));
        let sb = self.scrollback.len();
        let row = if line < sb { self.scrollback.get_mut(line) } else { self.primary.get_mut(line - sb) };
        let Some(mark) = row.and_then(|r| r.prompt.as_mut()) else { return };
        mark.exit = exit;
        let id = mark.cmd;
        let running = self.blocks.running.take();
        if let Some(info) = self.blocks.commands.get_mut(&id) {
            info.exit = exit;
            info.duration = info.started.map(|s| s.elapsed());
            if running == Some(id) || info.started.is_some() {
                self.blocks.finished.push(Finished { text: info.text.clone(), exit, duration: info.duration });
                if self.blocks.finished.len() > 32 {
                    self.blocks.finished.remove(0);
                }
            }
        }
    }

    /// Commands that finished since the last call.
    pub fn take_finished(&mut self) -> Vec<Finished> {
        std::mem::take(&mut self.blocks.finished)
    }

    /// The command running right now, and since when.
    pub fn running_command(&self) -> Option<&CommandInfo> {
        self.blocks.running.and_then(|id| self.blocks.commands.get(&id))
    }

    /// The block containing absolute `line`.
    pub fn block_at(&self, line: usize) -> Option<Block> {
        if self.alt_active {
            return None;
        }
        let total = self.total_lines();
        let (prompt, mark) = self.prompt_above(line.min(total.saturating_sub(1)), total)?;
        let next = (prompt + 1..total).find(|&l| self.absolute_row(l).is_some_and(|r| r.prompt.is_some()));
        let mut end = next.map_or(total - 1, |n| n - 1);
        // the live block ends at the cursor, not the blank rows below it
        if next.is_none() {
            end = end.min(self.cursor_line());
        }
        while end > prompt && self.absolute_row(end).is_some_and(|r| r.text().is_empty()) {
            end -= 1;
        }
        let output = (prompt..=end).find(|&l| self.absolute_row(l).is_some_and(|r| r.output)).unwrap_or((prompt + 1).min(end.max(prompt + 1)));
        let command = self.blocks.commands.get(&mark.cmd).cloned().unwrap_or_default();
        Some(Block { prompt, output, end: end.max(prompt), command })
    }

    /// The most recent block whose command has finished.
    pub fn last_finished_block(&self) -> Option<Block> {
        let total = self.total_lines();
        let line = (0..total).rev().find(|&l| self.absolute_row(l).and_then(|r| r.prompt).is_some_and(|m| m.exit.is_some()))?;
        self.block_at(line)
    }

    /// The output of a block as plain text.
    pub fn block_output(&self, b: &Block) -> String {
        if b.output > b.end {
            return String::new();
        }
        self.text_range((0, b.output), (self.cols - 1, b.end)).trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(t: &mut Terminal, s: &str) {
        vibex_vt::Parser::new().advance_bytes(t, s.as_bytes());
    }

    #[test]
    fn blocks_from_marks() {
        let mut t = Terminal::new(40, 10);
        // prompt, typed command, output, finish, next prompt
        feed(&mut t, "\x1b]133;A\x07PS> \x1b]133;B\x07echo hi\r\n\x1b]133;C\x07hi\r\nthere\r\n\x1b]133;D;0\x07");
        feed(&mut t, "\x1b]133;A\x07PS> \x1b]133;B\x07");
        let fin = t.take_finished();
        assert_eq!(fin.len(), 1);
        assert_eq!(fin[0].text, "echo hi");
        assert_eq!(fin[0].exit, Some(0));
        assert!(fin[0].duration.is_some());
        let b = t.last_finished_block().unwrap();
        assert_eq!((b.prompt, b.output, b.end), (0, 1, 2));
        assert_eq!(t.block_output(&b), "hi\nthere");
        assert_eq!(b.command.text, "echo hi");
        // the live block is the new prompt
        assert_eq!(t.block_at(3).unwrap().prompt, 3);
    }

    #[test]
    fn exact_text_from_633() {
        let mut t = Terminal::new(40, 6);
        feed(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07gi\x08\x08git log; x\r\n");
        feed(&mut t, "\x1b]633;E;git log\\x3b x\x07\x1b]133;C\x07out\r\n\x1b]133;D;1\x07");
        let fin = t.take_finished();
        assert_eq!(fin[0].text, "git log; x");
        assert_eq!(fin[0].exit, Some(1));
    }

    #[test]
    fn unescape() {
        assert_eq!(unescape_633(r"a\x3bb\\c"), r"a;b\c");
        assert_eq!(unescape_633(r"bad\xZZ"), r"bad\xZZ");
    }
}
