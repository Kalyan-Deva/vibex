//! Screenshots and "save these lines": what the window does with
//! `export` (the right-click and palette items, marks, the format menu).

use super::*;
use crate::export::{self, Caption, Format};

/// Some lines of one pane to save or copy.
#[derive(Clone, Debug)]
pub(crate) struct ExportJob {
    pub pane: PaneId,
    /// (column, absolute line), inclusive.
    pub from: (usize, usize),
    pub to: (usize, usize),
    pub caption: Option<Caption>,
    /// Trim the width to the text (selections, commands).
    pub tight: bool,
    /// Goes in the file name: "screen", "command", "lines".
    pub what: &'static str,
}

/// What to do with an `ExportJob`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ExportOut {
    CopyImage,
    CopyText,
    Save(Format),
    /// Save a PNG and copy it (screenshots).
    Shot,
}

/// A mark set with "Mark start": the pane and a line that survives old
/// lines dropping off the top (absolute line + lines evicted so far).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineMark {
    pub pane: PaneId,
    epoch: u64,
    line: u64,
}

impl Session {
    /// The screen as it is now.
    pub(crate) fn job_screen(&self, id: PaneId) -> Option<ExportJob> {
        let t = &self.panes.get(&id)?.term;
        let top = t.top_visible_line();
        Some(ExportJob { pane: id, from: (0, top), to: (t.cols() - 1, top + t.rows() - 1), caption: None, tight: false, what: "screen" })
    }

    pub(crate) fn job_selection(&self, id: PaneId) -> Option<ExportJob> {
        let p = self.panes.get(&id)?;
        let (from, to) = p.selection?.ordered();
        Some(ExportJob { pane: id, from, to, caption: None, tight: true, what: "selection" })
    }

    /// The command block at `line`, else the last finished one.
    pub(crate) fn job_block(&self, id: PaneId, line: Option<usize>) -> Option<ExportJob> {
        let t = &self.panes.get(&id)?.term;
        let b = line.and_then(|l| t.block_at(l)).or_else(|| t.last_finished_block())?;
        let caption = Caption { command: b.command.text.clone(), exit: b.command.exit, duration: b.command.duration };
        Some(ExportJob { pane: id, from: (0, b.prompt), to: (t.cols() - 1, b.end), caption: Some(caption), tight: true, what: "command" })
    }

    /// Every line, history and screen.
    pub(crate) fn job_all(&self, id: PaneId) -> Option<ExportJob> {
        let t = &self.panes.get(&id)?.term;
        let last = t.total_lines().saturating_sub(1);
        Some(ExportJob { pane: id, from: (0, 0), to: (t.cols() - 1, last), caption: None, tight: false, what: "history" })
    }

    pub(crate) fn job_lines(&self, id: PaneId, a: usize, b: usize) -> Option<ExportJob> {
        let t = &self.panes.get(&id)?.term;
        let last = t.total_lines().saturating_sub(1);
        let (a, b) = (a.min(b).min(last), a.max(b).min(last));
        Some(ExportJob { pane: id, from: (0, a), to: (t.cols() - 1, b), caption: None, tight: false, what: "lines" })
    }

    /// Copy the cells, secrets masked (the guard is always applied to
    /// exports while it is on, even with secrets revealed on screen).
    pub(crate) fn export_snapshot(&self, job: &ExportJob) -> Option<export::Snapshot> {
        let t = &self.panes.get(&job.pane)?.term;
        let guard = self.config.secret_guard.then_some(&self.config.secrets);
        let mut s = export::take(t, job.from, job.to, guard, job.tight);
        s.caption = job.caption.clone();
        Some(s)
    }

    /// Draw a snapshot in this window's theme and font: (pixels, w, h).
    pub(crate) fn render_snapshot(&mut self, s: &export::Snapshot) -> (Vec<u32>, usize, usize) {
        let r = &mut self.renderer;
        let scale = r.ui_scale.max(0.5);
        let (glass, px, py) = (r.glass, r.padding_x, r.padding_y);
        let pad = ((14.0 * scale).round() as usize).max(px);
        r.glass = false;
        r.padding_x = pad;
        r.padding_y = pad;
        let (cw, ch) = (r.fonts.cell_width, r.fonts.cell_height);
        let bar = if s.caption.is_some() { (ch as f32 * 2.0).round() as usize } else { 0 };
        let w = s.cols * cw + 2 * pad;
        let h = s.rows.len() * ch + 2 * pad + bar;
        let term = s.terminal();
        let mut scene = r.begin(w, h);
        let state = DrawState { focused: false, cursor_visible: false, ..Default::default() };
        r.draw_pane(&mut scene, &term, &state, PxRect { x: 0.0, y: bar as f32, w: w as f32, h: (h - bar) as f32 });
        for &(row, a, b) in &s.masked {
            let rect = PxRect { x: (pad + a * cw) as f32, y: (bar + pad + row * ch) as f32, w: ((b + 1 - a) * cw) as f32, h: ch as f32 };
            r.blur_rect(&mut scene, rect, cw as f32 * 0.6);
        }
        if let Some(c) = &s.caption {
            let cmd = if c.command.is_empty() { "command".to_string() } else { c.command.clone() };
            r.draw_caption(&mut scene, PxRect { x: 0.0, y: 0.0, w: w as f32, h: bar as f32 }, &cmd, &c.status(), export::status_color(c.exit));
        }
        let mut pixels = vec![0u32; w * h];
        vibex_render::cpu::draw(&scene, &mut r.fonts, &mut Frame { pixels: &mut pixels, width: w, height: h });
        r.glass = glass;
        r.padding_x = px;
        r.padding_y = py;
        (pixels, w, h)
    }

    /// Carry out a job. `to` overrides where a saved file goes. Returns
    /// what to tell the user.
    pub(crate) fn run_export(&mut self, job: &ExportJob, out: ExportOut, to: Option<PathBuf>) -> Result<String, String> {
        let s = self.export_snapshot(job).ok_or("that pane is gone")?;
        let masked =
            if s.masked.is_empty() { String::new() } else { format!(" ({} secret{} masked)", s.masked.len(), if s.masked.len() == 1 { "" } else { "s" }) };
        let lines = s.rows.len();
        let save = |bytes: &[u8], f: Format, to: Option<PathBuf>| -> Result<PathBuf, String> {
            let path = to.unwrap_or_else(|| export::new_path(job.what, f));
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(path)
        };
        match out {
            ExportOut::CopyText => {
                let text = s.text();
                if let Some(cb) = self.clipboard.as_mut() {
                    cb.set_text(text.trim_end().to_string()).map_err(|e| e.to_string())?;
                }
                Ok(format!("copied {lines} lines as text{masked}"))
            }
            ExportOut::CopyImage => {
                let (px, w, h) = self.render_snapshot(&s);
                export::copy_image(&px, w, h)?;
                Ok(format!("copied a {w}\u{d7}{h} picture{masked}"))
            }
            ExportOut::Shot => {
                let (px, w, h) = self.render_snapshot(&s);
                let path = save(&export::png(&px, w, h)?, Format::Png, to)?;
                let copied = export::copy_image(&px, w, h).is_ok();
                Ok(format!("saved{} {}{masked}", if copied { " and copied" } else { "" }, path.display()))
            }
            ExportOut::Save(f) => {
                let theme = self.renderer.theme.clone();
                let font = font_family(&self.config.fonts.regular);
                let bytes = match f {
                    Format::Text => s.text().into_bytes(),
                    Format::Ansi => s.ansi().into_bytes(),
                    Format::Markdown => s.markdown().into_bytes(),
                    Format::Html => s.html(&theme, &font).into_bytes(),
                    Format::Svg => s.svg(&theme, &font).into_bytes(),
                    Format::Png => {
                        let (px, w, h) = self.render_snapshot(&s);
                        export::png(&px, w, h)?
                    }
                };
                let path = save(&bytes, f, to)?;
                Ok(format!("saved {lines} lines: {}{masked}", path.display()))
            }
        }
    }

    pub(crate) fn do_export(&mut self, job: Option<ExportJob>, out: ExportOut) {
        let Some(job) = job else {
            self.toast("nothing to save there", 2.0);
            return;
        };
        match self.run_export(&job, out, None) {
            Ok(msg) => self.toast(msg, 4.0),
            Err(e) => self.toast(format!("couldn't save: {e}"), 4.0),
        }
    }

    /// The format menu for a job, at the pointer.
    pub(crate) fn export_menu(&mut self, job: ExportJob) {
        let n = job.to.1 + 1 - job.from.1.min(job.to.1);
        let mut items: Vec<(MenuItem, MenuCmd)> = Vec::new();
        let it =
            |label: String, hint: &str, sep: bool, out: ExportOut| (MenuItem { label, hint: hint.into(), separator: sep, icon: None }, MenuCmd::Export(out));
        items.push(it("Copy as image".into(), "", false, ExportOut::CopyImage));
        items.push(it("Copy as text".into(), "", false, ExportOut::CopyText));
        for (i, f) in Format::ALL.into_iter().enumerate() {
            items.push(it(format!("Save as {}", f.label()), &format!(".{}", f.ext()), i == 0, ExportOut::Save(f)));
        }
        items.push((
            MenuItem { label: "Open the screenshots folder".into(), hint: String::new(), separator: true, icon: None },
            MenuCmd::OpenLink(export::folder().display().to_string()),
        ));
        with_icons(&mut items);
        self.export_job = Some(job);
        let (x, y) = (self.mouse.x as f32 + 2.0, self.mouse.y as f32 + 2.0);
        self.menu = Some(Menu { items, typing: false, first: 0, min_width: 0.0, selected: Some(0), x, y });
        self.palette = None;
        self.toast(format!("{n} lines"), 1.2);
        self.window.request_redraw();
    }

    /// "Save…" with nothing chosen yet: which lines.
    pub(crate) fn save_range(&mut self) {
        let id = self.active_id();
        if let Some(job) = self.job_selection(id) {
            return self.export_menu(job);
        }
        let mut items: Vec<(MenuItem, MenuCmd)> = Vec::new();
        let pick = |label: &str, job: Option<ExportJob>| {
            job.map(|j| (MenuItem { label: label.into(), hint: String::new(), separator: false, icon: None }, MenuCmd::ExportPick(j)))
        };
        let line = self.pointer_line(id);
        items.extend(pick("This command and its output", self.job_block(id, line)));
        items.extend(pick("The screen", self.job_screen(id)));
        items.extend(pick("Everything (history and screen)", self.job_all(id)));
        items.push((
            MenuItem { label: "From a line to another…".into(), hint: hint_for(&self.config, &Action::Mark), separator: true, icon: None },
            MenuCmd::Act(Action::MarkStart),
        ));
        with_icons(&mut items);
        let (x, y) = (self.mouse.x as f32 + 2.0, self.mouse.y as f32 + 2.0);
        self.menu = Some(Menu { items, typing: false, first: 0, min_width: 0.0, selected: Some(0), x, y });
        self.palette = None;
        self.window.request_redraw();
    }

    /// The line under the pointer, when it is over pane `id`.
    pub(crate) fn pointer_line(&self, id: PaneId) -> Option<usize> {
        (self.pane_at(self.mouse.x, self.mouse.y) == Some(id)).then(|| self.abs_cell_in(id).1)
    }

    fn mark_line(&self, m: &LineMark) -> Option<usize> {
        let t = &self.panes.get(&m.pane)?.term;
        let (epoch, evicted) = t.line_base();
        if epoch != m.epoch || m.line < evicted {
            return None;
        }
        Some((m.line - evicted) as usize)
    }

    /// Where the mark is now (None once its lines were re-wrapped or dropped).
    pub(crate) fn mark_now(&self) -> Option<(PaneId, usize)> {
        let m = self.mark?;
        self.mark_line(&m).map(|l| (m.pane, l))
    }

    /// Mark the start at `line` (the pointer's, else the cursor's).
    pub(crate) fn mark_start(&mut self, line: Option<usize>) {
        let id = self.active_id();
        let t = &self.panes[&id].term;
        let line = line.or_else(|| self.pointer_line(id)).unwrap_or(t.total_lines() - t.rows() + t.cursor.y);
        let (epoch, evicted) = t.line_base();
        self.mark = Some(LineMark { pane: id, epoch, line: line as u64 + evicted });
        let key = hint_for(&self.config, &Action::Mark);
        let how = if key.is_empty() { "right-click where it ends".to_string() } else { format!("right-click where it ends, or {key} there") };
        self.toast(format!("start marked: scroll, then {how}"), 4.0);
        self.window.request_redraw();
    }

    /// Mark the end: everything from the start to `line` goes to the format menu.
    pub(crate) fn mark_end(&mut self, line: Option<usize>) {
        let Some((id, start)) = self.mark_now() else {
            self.mark = None;
            self.toast("mark a start first (Mark start)", 2.5);
            return;
        };
        let t = &self.panes[&id].term;
        let end = line.or_else(|| self.pointer_line(id)).unwrap_or(t.total_lines() - t.rows() + t.cursor.y);
        self.mark = None;
        if let Some(job) = self.job_lines(id, start, end) {
            self.export_menu(job);
        }
    }

    /// The rows between the mark and the pointer, tinted while choosing.
    pub(crate) fn mark_highlight(&self, id: PaneId, theme: &vibex_render::Theme) -> Option<vibex_render::Highlight> {
        let (pane, start) = self.mark_now()?;
        if pane != id {
            return None;
        }
        let t = &self.panes[&id].term;
        let top = t.top_visible_line();
        let hover = self.pointer_line(id).unwrap_or(start);
        let (a, b) = (start.min(hover), start.max(hover));
        let rows = t.rows();
        if b < top || a >= top + rows {
            return None;
        }
        let (a, b) = (a.max(top) - top, b.min(top + rows - 1) - top);
        Some(vibex_render::Highlight {
            range: vibex_render::VisibleSelection { start: (0, a), end: (t.cols() - 1, b), block: false },
            bg: vibex_render::blend(theme.background, 0x38d9f5, 46),
            fg: None,
        })
    }
}

/// The shortcut bound to `a` ("Alt+Shift+M"), or "".
pub(crate) fn hint_for(cfg: &Config, a: &Action) -> String {
    cfg.bindings.iter().find(|b| &b.action == a).map(|b| b.describe()).unwrap_or_default()
}

/// "C:\\Fonts\\CascadiaMono.ttf" -> "CascadiaMono" (a guess at the family
/// for HTML/SVG; the viewer falls back to Consolas).
fn font_family(path: &str) -> String {
    let stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path);
    let stem = stem.split(['-', '_']).next().unwrap_or(stem);
    // split CamelCase: CascadiaMono -> Cascadia Mono
    let mut out = String::new();
    for (i, c) in stem.chars().enumerate() {
        if i > 0 && c.is_uppercase() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push(c);
    }
    out
}
