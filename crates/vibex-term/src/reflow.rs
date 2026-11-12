//! Re-wrapping history and the primary screen when the width changes.

use crate::image::ImageSlice;
use crate::{flags, Cell, PromptMark, Row, Terminal};

fn is_blank(c: &Cell) -> bool {
    *c == Cell::default()
}

fn finish(out: &mut Vec<Row>, mut cells: Vec<Cell>, wrapped: bool, cols: usize, prompt: Option<PromptMark>) {
    cells.resize(cols, Cell::default());
    out.push(Row { cells, wrapped, prompt, images: Vec::new(), output: false });
}

impl Terminal {
    /// Re-wrap scrollback + primary screen to `cols` x `rows`.
    /// `cursor` is the primary cursor `(x, y)`; returns its new position.
    pub(crate) fn reflow_primary(&mut self, cols: usize, rows: usize, cursor: (usize, usize)) -> (usize, usize) {
        let cursor_line = self.scrollback.len() + cursor.1;
        // restored history ends here: that row starts the live session
        let fence_row = (self.history_fence > 0).then_some(self.history_fence);
        let mut fence_logical: Option<usize> = None;
        // every line number changes
        self.epoch += 1;
        let all: Vec<Row> = self.scrollback.drain(..).chain(self.primary.drain(..)).collect();
        let n = all.len();

        // 1. Join soft-wrapped rows into logical lines.
        let mut logical: Vec<Vec<Cell>> = Vec::new();
        let mut marks: Vec<Option<PromptMark>> = Vec::new();
        // images per logical line, by row offset within that line
        let mut line_images: Vec<Vec<(usize, ImageSlice)>> = Vec::new();
        let mut line_output: Vec<bool> = Vec::new();
        let mut buf_output = false;
        let mut buf_images: Vec<(usize, ImageSlice)> = Vec::new();
        let mut row_in_line = 0;
        let mut line_mark: Option<PromptMark> = None;
        let mut cursor_at: Option<(usize, usize)> = None; // (logical line, cell offset)
        let mut buf: Vec<Cell> = Vec::new();
        for (i, mut row) in all.into_iter().enumerate() {
            if Some(i) == fence_row && buf.is_empty() && fence_logical.is_none() {
                fence_logical = Some(logical.len());
            }
            if line_mark.is_none() {
                line_mark = row.prompt;
            }
            buf_images.extend(row.images.drain(..).map(|s| (row_in_line, s)));
            buf_output |= row.output;
            row_in_line += 1;
            for (x, cell) in row.cells.iter().enumerate() {
                if i == cursor_line && x == cursor.0 {
                    cursor_at = Some((logical.len(), buf.len()));
                }
                if !cell.has(flags::WRAP_PAD) {
                    buf.push(*cell);
                }
            }
            // pending-wrap cursor sits just past the end of its row
            if i == cursor_line && cursor.0 >= row.cells.len() {
                cursor_at = Some((logical.len(), buf.len()));
            }
            if !row.wrapped || i + 1 == n {
                let mut end = buf.len();
                while end > 0 && is_blank(&buf[end - 1]) {
                    end -= 1;
                }
                if let Some((li, off)) = cursor_at {
                    if li == logical.len() {
                        end = end.max(off);
                    }
                }
                buf.truncate(end);
                logical.push(std::mem::take(&mut buf));
                marks.push(line_mark.take());
                line_images.push(std::mem::take(&mut buf_images));
                line_output.push(std::mem::take(&mut buf_output));
                row_in_line = 0;
            }
        }

        // 2. Wrap each logical line at the new width.
        let mut out: Vec<Row> = Vec::new();
        let mut new_cursor = (0, 0);
        let mut fence_out: Option<usize> = None;
        for (li, cells) in logical.iter().enumerate() {
            let cursor_off = cursor_at.filter(|c| c.0 == li).map(|c| c.1);
            let first_row = out.len();
            if Some(li) == fence_logical {
                fence_out = Some(first_row);
            }
            // the mark goes on the first row of the re-wrapped line
            let mut mark = marks[li];
            let mut row: Vec<Cell> = Vec::with_capacity(cols);
            let mut placed = false;
            let mut i = 0;
            while i < cells.len() {
                let c = cells[i];
                let has_spacer = c.has(flags::WIDE) && cells.get(i + 1).is_some_and(|s| s.has(flags::WIDE_SPACER));
                let width = if c.has(flags::WIDE) { 2 } else { 1 };
                if row.len() + width > cols {
                    if width == 2 && row.len() < cols {
                        row.push(Cell { flags: flags::WRAP_PAD, ..Cell::default() });
                    }
                    finish(&mut out, std::mem::take(&mut row), true, cols, mark.take());
                }
                if cursor_off == Some(i) {
                    new_cursor = (row.len(), out.len());
                    placed = true;
                } else if has_spacer && cursor_off == Some(i + 1) {
                    new_cursor = (row.len() + 1, out.len());
                    placed = true;
                }
                row.push(c);
                if width == 2 {
                    let spacer = if has_spacer { cells[i + 1] } else { Cell { c: ' ', flags: flags::WIDE_SPACER, ..c } };
                    row.push(spacer);
                    i += if has_spacer { 2 } else { 1 };
                } else {
                    i += 1;
                }
            }
            if cursor_off.is_some() && !placed {
                // cursor sits just after the text
                new_cursor = (row.len().min(cols - 1), out.len());
            }
            finish(&mut out, row, false, cols, mark.take());
            if line_output[li] {
                out[first_row].output = true;
            }
            // images keep their row offset within the line, clamped to it
            let last_row = out.len() - 1;
            for (off, s) in line_images[li].drain(..) {
                out[(first_row + off).min(last_row)].images.push(s);
            }
        }

        // 3. Drop the blank rows below the cursor, then fill the screen.
        while out.len() > new_cursor.1 + 1 && out.last().is_some_and(|r| !r.wrapped && r.images.is_empty() && r.cells.iter().all(is_blank)) {
            out.pop();
        }
        while out.len() < rows {
            out.push(Row::new(cols, Cell::default()));
        }
        let mut start = out.len() - rows;
        if new_cursor.1 < start {
            start = new_cursor.1;
            out.truncate(start + rows);
        }
        // never show restored history on screen: the screen starts at the
        // fence at the earliest
        let fence_at = fence_out.or(fence_logical.map(|_| out.len()));
        if let Some(f) = fence_at {
            if start < f && new_cursor.1 >= f {
                start = f;
                while out.len() < start + rows {
                    out.push(Row::new(cols, Cell::default()));
                }
                out.truncate(start + rows);
            }
        }
        self.primary = out.split_off(start);
        self.scrollback = out.into();
        if fence_row.is_some() {
            self.history_fence = fence_at.map_or(0, |f| f.min(self.scrollback.len()));
        }
        while self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
        self.display_offset = 0;
        (new_cursor.0.min(cols - 1), new_cursor.1 - start)
    }
}
