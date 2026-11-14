//! Inline images (sixel and the kitty graphics protocol).
//!
//! An image placement is cut into one `ImageSlice` per text row it covers,
//! and each slice is stored on its `Row`. That way images scroll, move into
//! history, get re-wrapped and are evicted together with the text around
//! them, with no separate coordinate bookkeeping.

use crate::{Row, Terminal};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Largest image side we accept, in pixels.
pub const MAX_SIDE: u32 = 8192;
/// Largest image area we accept (128 MiB of RGBA).
pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;

/// Image bytes placed on rows (screen + history) before the oldest
/// placements are taken down.
pub const PLACED_LIMIT: usize = 512 * 1024 * 1024;
/// Most placements kept on rows.
pub const MAX_PLACEMENTS: usize = 8192;

static NEXT_UID: AtomicU64 = AtomicU64::new(1);

/// Decoded pixels: RGBA8, straight (not premultiplied) alpha.
pub struct ImageData {
    /// Unique per decoded image; renderers cache textures by it.
    pub uid: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl ImageData {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Arc<Self> {
        debug_assert_eq!(rgba.len(), width as usize * height as usize * 4);
        Arc::new(Self { uid: NEXT_UID.fetch_add(1, Ordering::Relaxed), width, height, rgba })
    }

    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }
}

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImageData#{}({}x{})", self.uid, self.width, self.height)
    }
}

/// Who placed an image (decides which deletes apply to it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Sixel,
    /// Kitty: image id, image number, placement id (0 = none given).
    Kitty {
        id: u32,
        number: u32,
        placement: u32,
    },
}

/// The part of one placement that falls on one row.
#[derive(Clone, Debug)]
pub struct ImageSlice {
    pub image: Arc<ImageData>,
    /// Unique per placement (all slices of a placement share it).
    pub pid: u64,
    pub source: Source,
    /// First column covered.
    pub col: usize,
    /// Destination on this row, in cell units relative to (col, row top):
    /// x, y, w, h.
    pub dest: [f32; 4],
    /// Source rectangle in image pixels: x, y, w, h.
    pub src: [f32; 4],
    /// Below text when negative.
    pub z: i32,
}

impl ImageSlice {
    /// Columns this slice touches.
    pub fn cols(&self) -> usize {
        (self.dest[0] + self.dest[2]).ceil().max(1.0) as usize
    }

    pub fn covers_col(&self, x: usize) -> bool {
        x >= self.col && x < self.col + self.cols()
    }
}

/// What to place and how big.
pub(crate) struct Placement {
    pub image: Arc<ImageData>,
    pub source: Source,
    /// Source rectangle in pixels.
    pub src: [f32; 4],
    /// Size in cells; 0 = derive from the pixel size and the cell size.
    pub cols: u32,
    pub rows: u32,
    pub z: i32,
    pub cursor: CursorAfter,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorAfter {
    /// Kitty: just right of the image, on its last row.
    RightOfLastRow,
    /// Sixel (VT340): start column, on the row below the image.
    BelowImage,
    /// Kitty `C=1`.
    Stay,
}

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

impl Terminal {
    /// Cell size in pixels, used to size images and to answer `CSI 16 t`.
    pub fn set_cell_pixels(&mut self, w: u32, h: u32) {
        self.cell_px = (w.max(1), h.max(1));
    }

    pub fn cell_pixels(&self) -> (u32, u32) {
        self.cell_px
    }

    /// Place an image at the cursor, scrolling as needed. Returns the
    /// placement id.
    pub(crate) fn place_image(&mut self, p: Placement) -> u64 {
        let (cw, ch) = (self.cell_px.0 as f32, self.cell_px.1 as f32);
        let [_, _, sw, sh] = p.src;
        // only one of columns / rows given: the other keeps the aspect
        let (sw1, sh1) = (sw.max(1.0), sh.max(1.0));
        let (w_cells, h_cells) = match (p.cols, p.rows) {
            (0, 0) => ((sw / cw).max(1.0 / cw), (sh / ch).max(1.0 / ch)),
            (c, 0) => (c as f32, (c as f32 * cw * sh1 / sw1 / ch).max(1.0 / ch)),
            (0, r) => ((r as f32 * ch * sw1 / sh1 / cw).max(1.0 / cw), r as f32),
            (c, r) => (c as f32, r as f32),
        };
        // never more rows than the scrollback could hold anyway
        let rows = (h_cells.ceil() as usize).clamp(1, 4096);
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
        let (x0, start_y) = (self.cursor.x, self.cursor.y);
        let sixel = p.source == Source::Sixel;
        self.wrap_pending = false;

        let stay = p.cursor == CursorAfter::Stay;
        for r in 0..rows {
            let y = if stay {
                // C=1: no scrolling; whatever falls off the bottom is clipped
                if start_y + r >= self.rows {
                    break;
                }
                start_y + r
            } else {
                if r > 0 {
                    self.linefeed_inner();
                }
                self.cursor.y
            };
            let frac0 = r as f32 / h_cells;
            let frac1 = ((r + 1) as f32 / h_cells).min(1.0);
            let slice = ImageSlice {
                image: p.image.clone(),
                pid,
                source: p.source,
                col: x0,
                dest: [0.0, 0.0, w_cells, (h_cells - r as f32).min(1.0)],
                src: [p.src[0], p.src[1] + sh * frac0, sw, sh * (frac1 - frac0)],
                z: p.z,
            };
            let cols = self.cols;
            let blank = self.blank();
            let row = &mut self.screen_mut()[y];
            if sixel {
                // sixel replaces what was there: text and older sixels
                let end = (x0 + slice.cols()).min(cols);
                for c in &mut row.cells[x0.min(end)..end] {
                    *c = blank;
                }
                row.images.retain(|s| !(s.source == Source::Sixel && s.col >= x0 && s.col + s.cols() <= x0 + slice.cols()));
            }
            if row.images.len() < 64 {
                row.images.push(slice);
            }
        }

        match p.cursor {
            CursorAfter::Stay => {}
            CursorAfter::RightOfLastRow => {
                let x = x0 + w_cells.ceil() as usize;
                if x >= self.cols {
                    self.cursor.x = self.cols - 1;
                    self.wrap_pending = true;
                } else {
                    self.cursor.x = x;
                }
            }
            CursorAfter::BelowImage => {
                self.linefeed_inner();
                self.cursor.x = x0;
            }
        }
        self.last_cell = None;
        self.track_placement(pid, p.image.bytes());
        pid
    }

    /// Count a placement against the budget for images on rows; past it,
    /// the oldest placements are taken down (a program placing image after
    /// image would otherwise keep every one alive in the scrollback).
    fn track_placement(&mut self, pid: u64, bytes: usize) {
        self.placed.push_back((pid, bytes + 512));
        self.placed_bytes += bytes + 512;
        while self.placed_bytes > PLACED_LIMIT || self.placed.len() > MAX_PLACEMENTS {
            let Some((old, b)) = self.placed.pop_front() else { break };
            self.placed_bytes -= b;
            self.remove_placements(&[old]);
        }
    }

    /// Every row that can hold images: history + both screens.
    fn all_rows_mut(&mut self) -> impl Iterator<Item = &mut Row> {
        self.scrollback.iter_mut().chain(self.primary.iter_mut()).chain(self.alternate.iter_mut())
    }

    /// Remove every slice of the placements matching `pred`.
    pub(crate) fn remove_images(&mut self, everywhere: bool, mut pred: impl FnMut(&ImageSlice) -> bool) {
        if everywhere {
            for row in self.all_rows_mut() {
                row.images.retain(|s| !pred(s));
            }
        } else {
            for row in self.screen_mut().iter_mut() {
                row.images.retain(|s| !pred(s));
            }
        }
    }

    /// Placement ids of slices on screen row `y` (0-based) that `hit` picks.
    pub(crate) fn placements_on_row(&self, y: usize, mut hit: impl FnMut(&ImageSlice) -> bool) -> Vec<u64> {
        self.screen().get(y).map_or(Vec::new(), |r| r.images.iter().filter(|s| hit(s)).map(|s| s.pid).collect())
    }

    /// Remove whole placements by id (all their rows).
    pub(crate) fn remove_placements(&mut self, pids: &[u64]) {
        if !pids.is_empty() {
            self.remove_images(true, |s| pids.contains(&s.pid));
        }
    }

    /// Total number of image slices on the live screen (tests, debugging).
    pub fn screen_image_count(&self) -> usize {
        self.screen().iter().map(|r| r.images.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_cover_rows() {
        let mut t = Terminal::new(20, 6);
        t.set_cell_pixels(10, 20);
        let img = ImageData::new(30, 50, vec![255; 30 * 50 * 4]);
        t.place_image(Placement {
            image: img,
            source: Source::Kitty { id: 1, number: 0, placement: 0 },
            src: [0.0, 0.0, 30.0, 50.0],
            cols: 0,
            rows: 0,
            z: 0,
            cursor: CursorAfter::RightOfLastRow,
        });
        // 50px / 20px rows = 2.5 rows -> 3 slices
        assert_eq!(t.screen_image_count(), 3);
        let last = &t.screen_row(2).images[0];
        assert_eq!(last.dest[3], 0.5);
        assert_eq!(last.src[1], 40.0);
        assert_eq!((t.cursor.x, t.cursor.y), (3, 2));
    }

    #[test]
    fn images_scroll_into_history() {
        let mut t = Terminal::new(10, 3);
        t.set_cell_pixels(10, 10);
        let img = ImageData::new(10, 40, vec![0; 10 * 40 * 4]);
        t.place_image(Placement { image: img, source: Source::Sixel, src: [0.0, 0.0, 10.0, 40.0], cols: 0, rows: 0, z: 0, cursor: CursorAfter::BelowImage });
        // 4 rows + cursor below on a 3-row screen: 2 rows went into history
        assert_eq!(t.screen_image_count(), 2);
        assert_eq!(t.absolute_row(0).unwrap().images.len(), 1);
        assert_eq!(t.cursor.y, 2);
    }
}
