//! Screen reader support (UI Automation on Windows, via AccessKit).
//!
//! The window exposes one `Terminal` node per visible pane, and each
//! visible row as a text run with per-character positions, so Narrator /
//! NVDA can read by line, word and character and follow the caret (the
//! terminal cursor). Panes are polite live regions: new output is
//! announced. The tree is only built while a screen reader is listening.

use accesskit::{Live, Node, NodeId, Rect, Role, TextPosition, TextSelection, TreeId, TreeInfo, TreeUpdate};
use vibex_term::{flags, Terminal};

const WINDOW: NodeId = NodeId(1);

fn pane_node(pane: u32) -> NodeId {
    NodeId((pane as u64) << 20)
}

fn row_node(pane: u32, row: usize) -> NodeId {
    NodeId(((pane as u64) << 20) | (row as u64 + 1))
}

/// One pane as the tree builder needs it.
pub struct PaneView<'a> {
    pub id: u32,
    pub term: &'a Terminal,
    pub title: String,
    /// Content origin (inside padding), in window pixels.
    pub origin: (f64, f64),
    pub active: bool,
}

/// A row's text as characters (grapheme clusters) with their columns and
/// widths in cells. Trailing blanks are dropped, except up to `keep`
/// (the cursor column) so the caret has somewhere to sit.
pub fn row_chars(term: &Terminal, line: usize, keep: Option<usize>) -> Vec<(String, usize, usize)> {
    let Some(row) = term.absolute_row(line) else { return Vec::new() };
    let mut out = Vec::new();
    for (x, c) in row.cells.iter().enumerate() {
        if c.has(flags::WIDE_SPACER | flags::WRAP_PAD) {
            continue;
        }
        let w = if c.has(flags::WIDE) { 2 } else { 1 };
        out.push((vibex_term::grapheme::text(c.c), x, w));
    }
    let last_text = out.iter().rposition(|(t, _, _)| t != " ");
    let keep_idx = keep.and_then(|k| out.iter().position(|(_, x, _)| *x >= k));
    let end = match (last_text, keep_idx) {
        (Some(a), Some(b)) => a.max(b) + 1,
        (Some(a), None) => a + 1,
        (None, Some(b)) => b + 1,
        (None, None) => 0,
    };
    out.truncate(end);
    out
}

fn word_starts(chars: &[(String, usize, usize)]) -> Vec<u8> {
    let mut starts = Vec::new();
    let mut prev_space = true;
    for (i, (t, _, _)) in chars.iter().enumerate() {
        let space = t.trim().is_empty();
        if !space && prev_space && i < 256 {
            starts.push(i as u8);
        }
        prev_space = space;
    }
    starts
}

/// The whole tree for a window.
pub fn tree(window_title: &str, panes: &[PaneView], cell: (f64, f64)) -> TreeUpdate {
    let (cw, ch) = cell;
    let mut nodes = Vec::new();
    let mut focus = WINDOW;
    let mut window = Node::new(Role::Window);
    window.set_label(window_title.to_string());
    let mut children = Vec::new();

    for p in panes {
        let term = p.term;
        let (ox, oy) = p.origin;
        let rows = term.rows();
        let top = term.top_visible_line();
        let cursor_row = (term.cursor.y + term.display_offset()) as isize;
        let cursor_visible = cursor_row >= 0 && (cursor_row as usize) < rows;

        let mut node = Node::new(Role::Terminal);
        node.set_label(p.title.clone());
        node.set_live(Live::Polite);
        node.set_bounds(Rect { x0: ox, y0: oy, x1: ox + term.cols() as f64 * cw, y1: oy + rows as f64 * ch });
        let mut row_ids = Vec::with_capacity(rows);
        let mut caret = None;
        for vy in 0..rows {
            let keep = (cursor_visible && vy == cursor_row as usize).then_some(term.cursor.x);
            let mut chars = row_chars(term, top + vy, keep);
            // hard line ends read as line breaks; soft wraps flow on
            let wrapped = term.absolute_row(top + vy).is_some_and(|r| r.wrapped);
            if !wrapped && vy + 1 < rows {
                let end = chars.last().map_or(0, |(_, x, w)| x + w);
                chars.push((
                    "
"
                    .to_string(),
                    end,
                    0,
                ));
            }
            let id = row_node(p.id, vy);
            let mut run = Node::new(Role::TextRun);
            let value: String = chars.iter().map(|(t, _, _)| t.as_str()).collect();
            run.set_character_lengths(chars.iter().map(|(t, _, _)| t.len().min(255) as u8).collect::<Vec<u8>>());
            run.set_character_positions(chars.iter().map(|(_, x, _)| (*x as f64 * cw) as f32).collect::<Vec<f32>>());
            run.set_character_widths(chars.iter().map(|(_, _, w)| (*w as f64 * cw) as f32).collect::<Vec<f32>>());
            run.set_word_starts(word_starts(&chars));
            run.set_value(value);
            let y = oy + vy as f64 * ch;
            run.set_bounds(Rect { x0: ox, y0: y, x1: ox + term.cols() as f64 * cw, y1: y + ch });
            if let Some(k) = keep {
                let idx = chars.iter().position(|(_, x, _)| *x >= k).unwrap_or(chars.len());
                caret = Some(TextPosition { node: id, character_index: idx });
            }
            nodes.push((id, run));
            row_ids.push(id);
        }
        if let Some(pos) = caret {
            node.set_text_selection(TextSelection { anchor: pos, focus: pos });
        }
        node.set_children(row_ids);
        let pid = pane_node(p.id);
        if p.active {
            focus = pid;
        }
        nodes.push((pid, node));
        children.push(pid);
    }
    window.set_children(children);
    nodes.push((WINDOW, window));
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo { root: WINDOW, toolkit_name: Some("Vibex".into()), toolkit_version: Some(env!("CARGO_PKG_VERSION").into()) }),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// A page of labelled rows (the Settings tab) as a list.
pub fn list_tree(title: &str, rows: &[String], focus: Option<usize>) -> TreeUpdate {
    let list_id = NodeId(2);
    let mut nodes = Vec::new();
    let mut window = Node::new(Role::Window);
    window.set_label(title.to_string());
    window.set_children(vec![list_id]);
    let mut list = Node::new(Role::List);
    list.set_label(title.to_string());
    let ids: Vec<NodeId> = (0..rows.len()).map(|i| NodeId(100 + i as u64)).collect();
    list.set_children(ids.clone());
    for (id, text) in ids.iter().zip(rows) {
        let mut item = Node::new(Role::ListItem);
        item.set_label(text.clone());
        item.add_action(accesskit::Action::Focus);
        nodes.push((*id, item));
    }
    nodes.push((list_id, list));
    nodes.push((WINDOW, window));
    let focus = focus.and_then(|f| ids.get(f).copied()).unwrap_or(list_id);
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo { root: WINDOW, toolkit_name: Some("Vibex".into()), toolkit_version: Some(env!("CARGO_PKG_VERSION").into()) }),
        tree_id: TreeId::ROOT,
        focus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_caret() {
        let mut t = Terminal::new(12, 3);
        vibex_vt::Parser::new().advance_bytes(&mut t, "ls 日本\r\n$ ".as_bytes());
        let chars = row_chars(&t, 0, None);
        let text: Vec<&str> = chars.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(text, ["l", "s", " ", "日", "本"]);
        assert_eq!(chars[4].1, 5, "second wide char starts at column 5");
        let upd = tree("Vibex", &[PaneView { id: 1, term: &t, title: "pwsh".into(), origin: (10.0, 8.0), active: true }], (8.0, 16.0));
        assert_eq!(upd.focus, pane_node(1));
        let (_, term_node) = upd.nodes.iter().find(|(id, _)| *id == pane_node(1)).unwrap();
        let sel = term_node.text_selection().unwrap();
        // cursor after "$ " on row 1 -> index 2 of that run
        assert_eq!(sel.focus, TextPosition { node: row_node(1, 1), character_index: 2 });
        let (_, run) = upd.nodes.iter().find(|(id, _)| *id == row_node(1, 0)).unwrap();
        assert_eq!(
            run.value(),
            Some(
                "ls 日本
"
            )
        );
        assert_eq!(run.character_lengths(), &[1, 1, 1, 3, 3, 1]);
        assert_eq!(run.word_starts(), &[0, 3]);
    }
}
