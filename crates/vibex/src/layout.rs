//! Split-pane layout: a binary tree of panes. Pure geometry, no terminal
//! state, so it is fully unit-tested.

pub type PaneId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDir {
    /// Side by side (a | b).
    Horizontal,
    /// Stacked (a over b).
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split { dir: SplitDir, ratio: f32, a: Box<Node>, b: Box<Node> },
}

/// A draggable divider between two subtrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divider {
    pub rect: Rect,
    pub dir: SplitDir,
    /// Path to the split node (false = a, true = b at each level).
    pub path: u64,
    pub depth: u8,
    /// The split's full rect (for turning a drag position into a ratio).
    pub parent: Rect,
}

pub const DIVIDER: f32 = 2.0;
const MIN_RATIO: f32 = 0.08;

impl Node {
    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(p) => *p == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// Replace pane `target` with a split of (target, new).
    pub fn split(&mut self, target: PaneId, new: PaneId, dir: SplitDir) -> bool {
        match self {
            Node::Leaf(p) if *p == target => {
                *self = Node::Split { dir, ratio: 0.5, a: Box::new(Node::Leaf(target)), b: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, new, dir) || b.split(target, new, dir),
        }
    }

    /// Remove a pane; its sibling takes the parent's place. Returns false
    /// if `id` is the only pane (caller closes the tab instead).
    pub fn remove(&mut self, id: PaneId) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => {
                if matches!(**a, Node::Leaf(p) if p == id) {
                    *self = (**b).clone();
                    true
                } else if matches!(**b, Node::Leaf(p) if p == id) {
                    *self = (**a).clone();
                    true
                } else {
                    a.remove(id) || b.remove(id)
                }
            }
        }
    }

    /// Lay out panes in `r`. Dividers are DIVIDER px wide between children.
    pub fn layout(&self, r: Rect) -> (Vec<(PaneId, Rect)>, Vec<Divider>) {
        let mut panes = Vec::new();
        let mut divs = Vec::new();
        self.layout_into(r, 0, 0, &mut panes, &mut divs);
        (panes, divs)
    }

    fn layout_into(&self, r: Rect, path: u64, depth: u8, panes: &mut Vec<(PaneId, Rect)>, divs: &mut Vec<Divider>) {
        match self {
            Node::Leaf(id) => panes.push((*id, r)),
            Node::Split { dir, ratio, a, b } => {
                let (ra, rd, rb) = split_rect(r, *dir, *ratio);
                divs.push(Divider { rect: rd, dir: *dir, path, depth, parent: r });
                a.layout_into(ra, path, depth + 1, panes, divs);
                b.layout_into(rb, path | (1 << depth), depth + 1, panes, divs);
            }
        }
    }

    /// Set the ratio of the split at `path`/`depth` from a pointer position.
    pub fn drag_divider(&mut self, d: &Divider, px: f32, py: f32) {
        let ratio = match d.dir {
            SplitDir::Horizontal => (px - d.parent.x) / d.parent.w.max(1.0),
            SplitDir::Vertical => (py - d.parent.y) / d.parent.h.max(1.0),
        }
        .clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        if let Some(Node::Split { ratio: r, .. }) = self.at_path(d.path, d.depth) {
            *r = ratio;
        }
    }

    fn at_path(&mut self, path: u64, depth: u8) -> Option<&mut Node> {
        let mut node = self;
        for level in 0..depth {
            node = match node {
                Node::Split { a, b, .. } => {
                    if path & (1 << level) != 0 {
                        b
                    } else {
                        a
                    }
                }
                Node::Leaf(_) => return None,
            };
        }
        Some(node)
    }

    /// Grow/shrink the pane `id` in `dir` by `delta` (fraction of the
    /// nearest enclosing split along that axis).
    pub fn resize(&mut self, id: PaneId, dir: Direction, delta: f32) -> bool {
        let axis = match dir {
            Direction::Left | Direction::Right => SplitDir::Horizontal,
            Direction::Up | Direction::Down => SplitDir::Vertical,
        };
        let sign = match dir {
            Direction::Right | Direction::Down => 1.0,
            Direction::Left | Direction::Up => -1.0,
        };
        self.resize_inner(id, axis, sign * delta)
    }

    fn resize_inner(&mut self, id: PaneId, axis: SplitDir, delta: f32) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { dir, ratio, a, b } => {
                // deepest matching split wins
                if a.contains(id) && a.resize_inner(id, axis, delta) {
                    return true;
                }
                if b.contains(id) && b.resize_inner(id, axis, delta) {
                    return true;
                }
                if *dir == axis && (a.contains(id) || b.contains(id)) {
                    *ratio = (*ratio + delta).clamp(MIN_RATIO, 1.0 - MIN_RATIO);
                    return true;
                }
                false
            }
        }
    }
}

fn split_rect(r: Rect, dir: SplitDir, ratio: f32) -> (Rect, Rect, Rect) {
    match dir {
        SplitDir::Horizontal => {
            let aw = ((r.w - DIVIDER) * ratio).round().max(1.0);
            let a = Rect { w: aw, ..r };
            let d = Rect { x: r.x + aw, w: DIVIDER, ..r };
            let b = Rect { x: r.x + aw + DIVIDER, w: (r.w - aw - DIVIDER).max(1.0), ..r };
            (a, d, b)
        }
        SplitDir::Vertical => {
            let ah = ((r.h - DIVIDER) * ratio).round().max(1.0);
            let a = Rect { h: ah, ..r };
            let d = Rect { y: r.y + ah, h: DIVIDER, ..r };
            let b = Rect { y: r.y + ah + DIVIDER, h: (r.h - ah - DIVIDER).max(1.0), ..r };
            (a, d, b)
        }
    }
}

/// The pane to move to from `from` in `dir`: among panes whose rect lies in
/// that direction and overlaps on the other axis, the closest one.
pub fn neighbor(panes: &[(PaneId, Rect)], from: PaneId, dir: Direction) -> Option<PaneId> {
    let (_, r) = panes.iter().find(|(id, _)| *id == from)?;
    let (cx, cy) = r.center();
    let eps = DIVIDER + 1.0;
    panes
        .iter()
        .filter(|(id, _)| *id != from)
        .filter(|(_, o)| match dir {
            Direction::Left => o.x + o.w <= r.x + eps && overlaps(o.y, o.h, r.y, r.h),
            Direction::Right => o.x >= r.x + r.w - eps && overlaps(o.y, o.h, r.y, r.h),
            Direction::Up => o.y + o.h <= r.y + eps && overlaps(o.x, o.w, r.x, r.w),
            Direction::Down => o.y >= r.y + r.h - eps && overlaps(o.x, o.w, r.x, r.w),
        })
        .min_by(|(_, a), (_, b)| {
            let d = |o: &Rect| {
                let (ox, oy) = o.center();
                (ox - cx).powi(2) + (oy - cy).powi(2)
            };
            d(a).total_cmp(&d(b))
        })
        .map(|(id, _)| *id)
}

fn overlaps(a: f32, al: f32, b: f32, bl: f32) -> bool {
    a < b + bl && b < a + al
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect { x: 0.0, y: 0.0, w: 802.0, h: 602.0 };

    #[test]
    fn split_and_layout() {
        let mut t = Node::Leaf(1);
        assert!(t.split(1, 2, SplitDir::Horizontal));
        let (panes, divs) = t.layout(R);
        assert_eq!(panes.len(), 2);
        assert_eq!(divs.len(), 1);
        let (a, b) = (panes[0].1, panes[1].1);
        assert_eq!(a.w + DIVIDER + b.w, R.w);
        assert_eq!(a.h, R.h);
        assert_eq!(b.x, a.w + DIVIDER);
    }

    #[test]
    fn nested_splits() {
        let mut t = Node::Leaf(1);
        t.split(1, 2, SplitDir::Horizontal);
        t.split(2, 3, SplitDir::Vertical);
        assert_eq!(t.panes(), vec![1, 2, 3]);
        let (panes, divs) = t.layout(R);
        assert_eq!(divs.len(), 2);
        let r2 = panes.iter().find(|p| p.0 == 2).unwrap().1;
        let r3 = panes.iter().find(|p| p.0 == 3).unwrap().1;
        assert_eq!(r2.x, r3.x);
        assert!(r3.y > r2.y);
    }

    #[test]
    fn remove_collapses() {
        let mut t = Node::Leaf(1);
        t.split(1, 2, SplitDir::Horizontal);
        t.split(2, 3, SplitDir::Vertical);
        assert!(t.remove(2));
        assert_eq!(t.panes(), vec![1, 3]);
        assert!(t.remove(1));
        assert_eq!(t, Node::Leaf(3));
        assert!(!t.remove(3), "last pane can't be removed");
    }

    #[test]
    fn neighbors() {
        // 1 | (2 / 3)
        let mut t = Node::Leaf(1);
        t.split(1, 2, SplitDir::Horizontal);
        t.split(2, 3, SplitDir::Vertical);
        let (panes, _) = t.layout(R);
        assert!(matches!(neighbor(&panes, 1, Direction::Right), Some(2) | Some(3)));
        assert_eq!(neighbor(&panes, 2, Direction::Down), Some(3));
        assert_eq!(neighbor(&panes, 3, Direction::Up), Some(2));
        assert_eq!(neighbor(&panes, 3, Direction::Left), Some(1));
        assert_eq!(neighbor(&panes, 1, Direction::Left), None);
    }

    #[test]
    fn drag_and_resize() {
        let mut t = Node::Leaf(1);
        t.split(1, 2, SplitDir::Horizontal);
        let (_, divs) = t.layout(R);
        t.drag_divider(&divs[0], 200.0, 10.0);
        let (panes, _) = t.layout(R);
        assert!((panes[0].1.w - 200.0).abs() < 2.0, "{:?}", panes[0].1);
        assert!(t.resize(1, Direction::Right, 0.1));
        let (panes2, _) = t.layout(R);
        assert!(panes2[0].1.w > panes[0].1.w);
        // no vertical split to resize
        assert!(!t.resize(1, Direction::Down, 0.1));
        // clamped
        t.drag_divider(&divs[0], -50.0, 0.0);
        let (p3, _) = t.layout(R);
        assert!(p3[0].1.w >= R.w * 0.07);
    }

    #[test]
    fn drag_nested_divider_by_path() {
        let mut t = Node::Leaf(1);
        t.split(1, 2, SplitDir::Horizontal);
        t.split(2, 3, SplitDir::Vertical);
        let (_, divs) = t.layout(R);
        let inner = divs.iter().find(|d| d.dir == SplitDir::Vertical).copied().unwrap();
        t.drag_divider(&inner, 0.0, 450.0);
        let (panes, _) = t.layout(R);
        let r2 = panes.iter().find(|p| p.0 == 2).unwrap().1;
        assert!((r2.h - 450.0).abs() < 3.0, "{r2:?}");
    }
}
