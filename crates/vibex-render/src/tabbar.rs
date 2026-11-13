//! Tab bar geometry, shared by drawing and mouse hit-testing. When Vibex
//! draws its own title bar, this strip is it: tabs, a new-tab button, a
//! drag area and the window buttons.

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PxRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PxRect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabHit {
    Tab(usize),
    CloseTab(usize),
    NewTab,
    /// The ⌄ next to +: pick a profile.
    NewTabMenu,
    Minimize,
    Maximize,
    Close,
    /// Empty bar: drag moves the window, double-click maximizes.
    Drag,
    /// The classic bar's menu (Session, Actions, Edit, View, Help) by index.
    Menu(usize),
}

/// The classic bar's menus.
pub const MENUS: [&str; 5] = ["Session", "Actions", "Edit", "View", "Help"];

#[derive(Clone, Debug)]
pub struct TabInfo {
    pub title: String,
    pub active: bool,
    /// The tab's shell exited with an error / has activity: small dot.
    pub alert: bool,
    /// User-chosen accent colour.
    pub color: Option<u32>,
    /// Being renamed: draw a caret after the title.
    pub editing: bool,
    pub icon: TabIcon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TabIcon {
    #[default]
    Terminal,
    Settings,
}

#[derive(Clone, Debug)]
pub struct TabBar {
    pub tabs: Vec<TabInfo>,
    /// Draw minimize / maximize / close (custom title bar).
    pub window_buttons: bool,
    pub maximized: bool,
    pub hover: Option<TabHit>,
    /// Text shown in the drag area (e.g. the active title when there is
    /// only one tab).
    pub caption: Option<String>,
    /// The classic look: a centred title, round window buttons and a menu
    /// bar, with tabs below only when there's more than one.
    pub classic: bool,
    /// The window's title (the classic bar shows it).
    pub title: String,
    /// The classic bar's menu that's open.
    pub menu_open: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct TabBarLayout {
    pub height: f32,
    pub regions: Vec<(PxRect, TabHit)>,
}

/// Window button width at 100% scaling (Windows' own caption buttons).
pub const BUTTON_W: f32 = 46.0;
const BAR_H: f32 = 35.0;
const TAB_TOP: f32 = 5.0;
const TAB_W: f32 = 240.0;
const MIN_TAB: f32 = 72.0;
const NEW_TAB_W: f32 = 34.0;
const MENU_W: f32 = 28.0;
const DRAG_MIN: f32 = 48.0;

/// Bar height for a display `scale` (1.0 = 100%). Never shorter than the
/// text needs.
pub fn height(cell_h: usize, scale: f32) -> f32 {
    (BAR_H * scale).round().max((cell_h + 12) as f32)
}

/// Lay the bar out across `width` pixels, Windows Terminal style: equal
/// width tabs from the left, then + and the new-tab menu button, a drag
/// area, and the window buttons on the right.
pub fn layout(bar: &TabBar, width: f32, cell_w: usize, cell_h: usize, scale: f32) -> TabBarLayout {
    if bar.classic {
        return classic_layout(bar, width, cell_w, cell_h, scale);
    }
    let _ = cell_w;
    let s = scale.max(0.5);
    let h = height(cell_h, s);
    let mut regions = Vec::new();
    let button_w = (BUTTON_W * s).round();
    let buttons = if bar.window_buttons { button_w * 3.0 } else { 0.0 };
    let left = (8.0 * s).round();
    let extras = ((NEW_TAB_W + MENU_W + 6.0) * s).round();
    let avail = (width - buttons - extras - DRAG_MIN * s - left).max(0.0);
    let n = bar.tabs.len().max(1) as f32;
    // equal-width tabs that shrink (titles truncate) instead of overflowing
    let tab_w = (TAB_W * s).min(avail / n).max((MIN_TAB * s).min(avail / n)).floor().max(4.0);
    let top = (TAB_TOP * s).round();
    let tab_h = h - top;
    let mut x = left;
    for i in 0..bar.tabs.len() {
        let r = PxRect { x, y: top, w: tab_w, h: tab_h };
        let cw = (28.0 * s).round();
        regions.push((PxRect { x: r.x + r.w - cw - (4.0 * s).round(), y: r.y + 4.0 * s, w: cw, h: r.h - 8.0 * s }, TabHit::CloseTab(i)));
        regions.push((r, TabHit::Tab(i)));
        x += tab_w;
    }
    x += (6.0 * s).round();
    let bh = tab_h - 8.0 * s;
    let by = top + 3.0 * s;
    regions.push((PxRect { x, y: by, w: (NEW_TAB_W * s).round(), h: bh }, TabHit::NewTab));
    x += (NEW_TAB_W * s).round();
    regions.push((PxRect { x, y: by, w: (MENU_W * s).round(), h: bh }, TabHit::NewTabMenu));
    if bar.window_buttons {
        let bx = width - buttons;
        for (i, hit) in [TabHit::Minimize, TabHit::Maximize, TabHit::Close].into_iter().enumerate() {
            regions.push((PxRect { x: bx + button_w * i as f32, y: 0.0, w: button_w, h }, hit));
        }
    }
    TabBarLayout { height: h, regions }
}

// the classic bar, at 100%
const TITLE_H: f32 = 30.0;
const MENU_H: f32 = 26.0;
const TABROW_H: f32 = 30.0;
const DOT: f32 = 16.0;
const DOT_GAP: f32 = 8.0;

/// The classic bar's three rows at `scale`: title, menus, tabs (0 when
/// there's one tab).
pub fn classic_rows(cell_h: usize, scale: f32, tabs_row: bool) -> (f32, f32, f32) {
    let s = scale.max(0.5);
    let title = (TITLE_H * s).round().max((cell_h + 8) as f32);
    let menu = (MENU_H * s).round().max((cell_h + 4) as f32);
    let tabs = if tabs_row { (TABROW_H * s).round().max((cell_h + 8) as f32) } else { 0.0 };
    (title, menu, tabs)
}

/// The classic bar's height.
pub fn classic_height(cell_h: usize, scale: f32, tabs_row: bool) -> f32 {
    let (a, b, c) = classic_rows(cell_h, scale, tabs_row);
    a + b + c
}

/// Rough width of a menu label in the UI font (layout has no fonts).
fn label_w(text: &str, s: f32) -> f32 {
    (text.chars().count() as f32 * 6.4 * s + 9.0 * s).round()
}

fn classic_layout(bar: &TabBar, width: f32, _cell_w: usize, cell_h: usize, scale: f32) -> TabBarLayout {
    let s = scale.max(0.5);
    let (th, mh, tabs_h) = classic_rows(cell_h, s, bar.tabs.len() > 1);
    let mut regions = Vec::new();
    // window buttons: dots on the right of the title row
    if bar.window_buttons {
        let d = (DOT * s).round();
        let gap = (DOT_GAP * s).round();
        let hit = (d + gap).round();
        let mut x = width - (10.0 * s).round() - d;
        for h in [TabHit::Close, TabHit::Maximize, TabHit::Minimize] {
            let cx = x + d / 2.0;
            regions.push((PxRect { x: (cx - hit / 2.0).round(), y: 0.0, w: hit, h: th }, h));
            x -= d + gap;
        }
    }
    // the menu bar
    let mut x = (3.0 * s).round();
    for (i, m) in MENUS.iter().enumerate() {
        let w = label_w(m, s);
        regions.push((PxRect { x, y: th + (2.0 * s).round(), w, h: mh - (4.0 * s).round() }, TabHit::Menu(i)));
        x += w;
    }
    // tabs, when there's more than one: equal widths across the window
    if tabs_h > 0.0 {
        let y = th + mh;
        let n = bar.tabs.len() as f32;
        let avail = width - (8.0 * s).round() * 2.0 - (NEW_TAB_W * s).round();
        let tab_w = (avail / n).min(TAB_W * s * 1.2).floor().max(40.0 * s);
        let mut x = (8.0 * s).round();
        for i in 0..bar.tabs.len() {
            let r = PxRect { x, y, w: tab_w, h: tabs_h };
            let cw = (24.0 * s).round();
            regions.push((PxRect { x: r.x + r.w - cw - (4.0 * s).round(), y: r.y + 4.0 * s, w: cw, h: r.h - 8.0 * s }, TabHit::CloseTab(i)));
            regions.push((r, TabHit::Tab(i)));
            x += tab_w;
        }
        regions.push((PxRect { x: x + (4.0 * s).round(), y: y + 3.0 * s, w: (NEW_TAB_W * s).round(), h: tabs_h - 6.0 * s }, TabHit::NewTab));
    }
    TabBarLayout { height: th + mh + tabs_h, regions }
}

impl TabBarLayout {
    /// What is under (x, y)? None below the bar.
    pub fn hit(&self, x: f32, y: f32) -> Option<TabHit> {
        if y < 0.0 || y >= self.height {
            return None;
        }
        Some(self.regions.iter().find(|(r, _)| r.contains(x, y)).map_or(TabHit::Drag, |(_, h)| *h))
    }

    pub fn rect(&self, hit: TabHit) -> Option<PxRect> {
        self.regions.iter().find(|(_, h)| *h == hit).map(|(r, _)| *r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(n: usize) -> TabBar {
        TabBar {
            tabs: (0..n)
                .map(|i| TabInfo { title: format!("tab {i}"), active: i == 0, alert: false, color: None, editing: false, icon: TabIcon::Terminal })
                .collect(),
            window_buttons: true,
            maximized: false,
            hover: None,
            caption: None,
            classic: false,
            title: String::new(),
            menu_open: None,
        }
    }

    #[test]
    fn classic_bar() {
        let mut b = bar(1);
        b.classic = true;
        let l = layout(&b, 1000.0, 9, 19, 1.0);
        // title + menus, no tabs row with one tab
        assert_eq!(l.height, 56.0);
        assert!(l.rect(TabHit::Tab(0)).is_none());
        assert_eq!(l.hit(1000.0 - 18.0, 15.0), Some(TabHit::Close));
        assert_eq!(l.hit(500.0, 15.0), Some(TabHit::Drag));
        let session = l.rect(TabHit::Menu(0)).unwrap();
        assert_eq!(l.hit(session.x + 4.0, session.y + 4.0), Some(TabHit::Menu(0)));
        assert!(l.rect(TabHit::Menu(4)).unwrap().x > session.x);
        // a second tab brings the tabs row
        let mut b = bar(2);
        b.classic = true;
        let l = layout(&b, 1000.0, 9, 19, 1.0);
        assert_eq!(l.height, 86.0);
        let t1 = l.rect(TabHit::Tab(1)).unwrap();
        assert!(t1.y >= 56.0);
        assert_eq!(classic_height(19, 1.0, true), 86.0);
    }

    #[test]
    fn hit_testing() {
        let l = layout(&bar(2), 1000.0, 9, 19, 1.0);
        assert_eq!(l.height, 35.0);
        let t0 = l.rect(TabHit::Tab(0)).unwrap();
        assert_eq!(l.hit(t0.x + 5.0, 15.0), Some(TabHit::Tab(0)));
        let c0 = l.rect(TabHit::CloseTab(0)).unwrap();
        assert_eq!(l.hit(c0.x + 2.0, 15.0), Some(TabHit::CloseTab(0)));
        assert_eq!(l.hit(999.0, 5.0), Some(TabHit::Close));
        assert_eq!(l.hit(1000.0 - BUTTON_W * 1.5, 5.0), Some(TabHit::Maximize));
        assert_eq!(l.hit(1000.0 - BUTTON_W * 2.5, 5.0), Some(TabHit::Minimize));
        assert_eq!(l.hit(700.0, 15.0), Some(TabHit::Drag));
        assert_eq!(l.hit(10.0, 40.0), None);
        let nt = l.rect(TabHit::NewTab).unwrap();
        assert!(nt.x > l.rect(TabHit::Tab(1)).unwrap().x);
    }

    #[test]
    fn tabs_shrink_to_fit() {
        let l = layout(&bar(30), 900.0, 9, 19, 1.25);
        let nt = l.rect(TabHit::NewTab).unwrap();
        assert!(nt.x + nt.w <= 900.0 - BUTTON_W * 1.25 * 3.0, "{nt:?}");
    }

    #[test]
    fn no_window_buttons() {
        let mut b = bar(1);
        b.window_buttons = false;
        let l = layout(&b, 800.0, 9, 19, 1.0);
        assert_eq!(l.hit(799.0, 5.0), Some(TabHit::Drag));
    }
}
