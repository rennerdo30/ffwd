//! The Local Changes view (FF-35): unstaged and staged files, the selected
//! file's diff with per-hunk staging, and the commit box. Every Git call
//! runs on a worker (work.rs); the view only shows what is known and marks
//! what is loading.

use super::*;
use crate::text_area::TextArea;
use ffwd_core::changes::{CommitOptions, Entry, Side, WorkStatus};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CFocus {
    Unstaged,
    Staged,
    Subject,
    Desc,
    Diff,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CHit {
    Row(Side, usize),
    StageAll,
    UnstageAll,
    Hunk(usize),
    Subject,
    Desc,
    Amend,
    Commit,
}

pub struct Changes {
    pub status: Option<WorkStatus>,
    pub side: Side,
    pub sel: usize,
    tops: [usize; 2],
    pub diff: Option<Box<FileDiff>>,
    diff_top: usize,
    seq: u64,
    pub subject: TextField,
    pub desc: TextArea,
    pub amend: bool,
    pub focus: CFocus,
    /// An operation is running (its verb, for the commit button).
    pub busy: Option<&'static str>,
    pub hits: Vec<(Rect, CHit)>,
    /// The last error, shown above the commit button.
    pub error: Option<String>,
}

impl Default for Changes {
    fn default() -> Self {
        Changes {
            status: None,
            side: Side::Unstaged,
            sel: 0,
            tops: [0, 0],
            diff: None,
            diff_top: 0,
            seq: 0,
            subject: TextField::default(),
            desc: TextArea::default(),
            amend: false,
            focus: CFocus::Unstaged,
            busy: None,
            hits: Vec::new(),
            error: None,
        }
    }
}

/// Where everything of the view sits.
#[derive(Clone, Copy, Debug, Default)]
pub struct CLayout {
    pub area: Rect,
    pub left: Rect,
    pub heads: [Rect; 2],
    pub lists: [Rect; 2],
    pub commit: Rect,
    pub diff: Rect,
}

fn side_ix(s: Side) -> usize {
    if s == Side::Unstaged { 0 } else { 1 }
}

impl Ui {
    pub(super) fn changes_layout(&self) -> CLayout {
        let side = self.lay.side;
        let area =
            Rect::new(side.right(), self.lay.list.y, self.w as i32 - side.right(), self.lay.status.y - self.lay.list.y);
        let lw = ((area.w as f32 * 0.38) as i32).clamp(self.px(320.0), self.px(560.0)).min(area.w / 2);
        let left = Rect::new(area.x, area.y, lw, area.h);
        let ch = self.px(196.0).min(left.h / 2);
        let commit = Rect::new(left.x, left.bottom() - ch, left.w, ch);
        let head = self.px(32.0);
        let lists_h = (commit.y - left.y - 2 * head).max(0);
        let uh = lists_h * 55 / 100;
        let h0 = Rect::new(left.x, left.y, left.w, head);
        let l0 = Rect::new(left.x, h0.bottom(), left.w, uh);
        let h1 = Rect::new(left.x, l0.bottom(), left.w, head);
        let l1 = Rect::new(left.x, h1.bottom(), left.w, commit.y - h1.bottom());
        CLayout {
            area,
            left,
            heads: [h0, h1],
            lists: [l0, l1],
            commit,
            diff: Rect::new(left.right(), area.y, area.right() - left.right(), area.h),
        }
    }

    fn change_row_h(&self) -> i32 {
        self.px(26.0)
    }

    pub(super) fn git_for_tab(&self) -> Option<Git> {
        let t = self.tabs.get(self.cur_tab)?;
        Some(self.r.git.clone().unwrap_or(Git { root: t.path.clone() }))
    }

    fn tab_sink(&self) -> Option<Sink> {
        self.tabs.get(self.cur_tab).map(|t| (self.sinks)(t.id))
    }

    /// Opens the view (sidebar "Local Changes") and loads the state.
    pub fn enter_changes(&mut self) {
        if !self.r.changes_view {
            self.r.changes_view = true;
            self.r.changes.focus = CFocus::Unstaged;
            self.focus = Focus::List;
        }
        self.side_select("changes");
        self.reload_changes();
        self.damage.full = true;
    }

    pub(super) fn leave_changes(&mut self) {
        if self.r.changes_view {
            self.r.changes_view = false;
            self.damage.full = true;
        }
    }

    pub(super) fn reload_changes(&mut self) {
        if let (Some(g), Some(s)) = (self.git_for_tab(), self.tab_sink()) {
            crate::work::changes_load(g, &s);
        }
    }

    fn side_list(&self, side: Side) -> Vec<Entry> {
        self.r.changes.status.as_ref().map_or_else(Vec::new, |s| s.side(side).into_iter().cloned().collect())
    }

    fn selected_entry(&self) -> Option<Entry> {
        self.side_list(self.r.changes.side).get(self.r.changes.sel).cloned()
    }

    fn request_change_diff(&mut self) {
        let Some(e) = self.selected_entry() else {
            self.r.changes.diff = None;
            return;
        };
        self.r.changes.seq += 1;
        let (seq, side) = (self.r.changes.seq, self.r.changes.side);
        if let Some(w) = &self.r.worker {
            w.change_diff(seq, e, side);
        }
    }

    /// The left pane changed (lists, commit box); a key press repaints
    /// only this, never the diff (rule 2: damage stays small).
    fn changes_dirty(&mut self) {
        let a = self.changes_layout().left;
        self.damage.rects.push(a);
    }

    fn changes_dirty_all(&mut self) {
        let a = self.changes_layout().area;
        self.damage.rects.push(a);
    }

    pub(super) fn on_changes(&mut self, r: Result<WorkStatus, String>) {
        let keep = self.selected_entry().map(|e| e.path);
        match r {
            Ok(st) => {
                let c = &mut self.r.changes;
                c.status = Some(st);
                // Keep the selected file when it is still on the same side,
                // else follow it to the other side, else clamp.
                let find = |side: Side, c: &Changes| {
                    keep.as_ref()
                        .and_then(|p| c.status.as_ref().and_then(|s| s.side(side).iter().position(|e| &e.path == p)))
                };
                if let Some(i) = find(c.side, c) {
                    c.sel = i;
                } else {
                    let n = self.side_list(self.r.changes.side).len();
                    let c = &mut self.r.changes;
                    if n == 0 {
                        let other = if c.side == Side::Unstaged { Side::Staged } else { Side::Unstaged };
                        if c.status.as_ref().is_some_and(|s| !s.side(other).is_empty())
                            && matches!(c.focus, CFocus::Unstaged | CFocus::Staged)
                        {
                            c.side = other;
                            c.focus = if other == Side::Unstaged { CFocus::Unstaged } else { CFocus::Staged };
                        }
                        c.sel = 0;
                    } else {
                        c.sel = c.sel.min(n - 1);
                    }
                }
                self.request_change_diff();
            }
            Err(e) => self.r.changes.error = Some(format!("Could not read the working tree: {e}")),
        }
        self.changes_dirty_all();
    }

    pub(super) fn on_changes_diff(&mut self, seq: u64, r: Result<Box<FileDiff>, String>) {
        if seq != self.r.changes.seq {
            return;
        }
        match r {
            Ok(d) => {
                let same = self.r.changes.diff.as_ref().is_some_and(|o| o.path == d.path);
                if !same {
                    self.r.changes.diff_top = 0;
                }
                self.r.changes.diff = Some(d);
            }
            Err(e) => {
                self.r.changes.diff = None;
                self.r.changes.error = Some(format!("Could not load the diff: {e}"));
            }
        }
        let d = self.changes_layout().diff;
        self.damage.rects.push(d);
    }

    pub(super) fn on_changes_op(&mut self, commit: bool, r: Result<String, String>) {
        self.r.changes.busy = None;
        match r {
            Ok(msg) => {
                self.r.changes.error = None;
                if commit {
                    let c = &mut self.r.changes;
                    c.subject = TextField::default();
                    c.desc = TextArea::default();
                    c.amend = false;
                    c.focus = CFocus::Unstaged;
                    self.set_notice(msg);
                    // History, refs and the sidebar changed.
                    self.reload_current();
                    return;
                }
                self.set_notice(msg);
            }
            Err(e) => {
                self.set_notice(format!("Failed: {e}"));
                self.r.changes.error = Some(e);
            }
        }
        self.changes_dirty();
    }

    pub(super) fn on_last_message(&mut self, m: String) {
        let c = &mut self.r.changes;
        if c.amend && c.subject.text().is_empty() && c.desc.is_empty() {
            let (subject, rest) = m.split_once('\n').unwrap_or((m.as_str(), ""));
            c.subject = TextField::new(subject.trim());
            c.desc = TextArea::new(rest.trim_start_matches('\n'));
            self.changes_dirty();
        }
    }

    fn run_change_op(&mut self, op: crate::work::ChangeOp, verb: &'static str) {
        if self.r.changes.busy.is_some() {
            return self.set_notice("Another change is still running.".into());
        }
        let (Some(g), Some(s)) = (self.git_for_tab(), self.tab_sink()) else { return };
        self.r.changes.busy = Some(verb);
        crate::work::changes_op(g, op, &s);
        self.changes_dirty();
    }

    /// Stages or unstages the selected file.
    fn toggle_selected(&mut self) {
        let Some(e) = self.selected_entry() else { return };
        let p = vec![e.path.to_string()];
        if self.r.changes.side == Side::Unstaged {
            self.run_change_op(crate::work::ChangeOp::Stage(p), "Staging");
        } else {
            self.run_change_op(crate::work::ChangeOp::Unstage(p), "Unstaging");
        }
    }

    pub(super) fn stage_all(&mut self, stage: bool) {
        let side = if stage { Side::Unstaged } else { Side::Staged };
        let paths: Vec<String> =
            self.side_list(side).iter().filter(|e| !e.conflicted).map(|e| e.path.to_string()).collect();
        if paths.is_empty() {
            return;
        }
        if stage {
            self.run_change_op(crate::work::ChangeOp::Stage(paths), "Staging");
        } else {
            self.run_change_op(crate::work::ChangeOp::Unstage(paths), "Unstaging");
        }
    }

    fn toggle_hunk(&mut self, hunk: usize) {
        let Some(e) = self.selected_entry() else { return };
        let side = self.r.changes.side;
        self.run_change_op(crate::work::ChangeOp::Hunk(Box::new(e), side, hunk), "Staging");
    }

    pub(super) fn commit_changes(&mut self) {
        let c = &self.r.changes;
        let staged = self.side_list(Side::Staged).len();
        if staged == 0 && !c.amend {
            return self.set_notice("Nothing is staged: stage files first (Space), or tick Amend.".into());
        }
        if c.subject.text().trim().is_empty() && !c.amend {
            self.r.changes.focus = CFocus::Subject;
            self.changes_dirty();
            return self.set_notice("Write a commit subject first.".into());
        }
        let msg = ffwd_core::changes::message(c.subject.text(), &c.desc.text());
        let opts = CommitOptions { amend: c.amend, ..Default::default() };
        self.run_change_op(crate::work::ChangeOp::Commit(msg, opts), "Committing");
    }

    fn set_amend(&mut self, on: bool) {
        self.r.changes.amend = on;
        if on
            && self.r.changes.subject.text().is_empty()
            && let (Some(g), Some(s)) = (self.git_for_tab(), self.tab_sink())
        {
            crate::work::last_message(g, &s);
        }
        self.changes_dirty();
    }

    fn select_change(&mut self, side: Side, i: usize) {
        let n = self.side_list(side).len();
        if n == 0 {
            return;
        }
        let c = &mut self.r.changes;
        c.side = side;
        c.sel = i.min(n - 1);
        c.focus = if side == Side::Unstaged { CFocus::Unstaged } else { CFocus::Staged };
        // Keep the selection visible.
        let lists = self.changes_layout().lists;
        let fit = (lists[side_ix(side)].h / self.change_row_h().max(1)).max(1) as usize;
        let c = &mut self.r.changes;
        let t = &mut c.tops[side_ix(side)];
        if c.sel < *t {
            *t = c.sel;
        } else if c.sel >= *t + fit {
            *t = c.sel + 1 - fit;
        }
        self.request_change_diff();
        self.changes_dirty();
    }

    /// Keys while the view is shown. Returns false for keys it leaves to
    /// the rest of the UI.
    pub(super) fn changes_key(&mut self, k: Key) -> bool {
        if matches!(k, Key::CtrlEnter) {
            self.commit_changes();
            return true;
        }
        let focus = self.r.changes.focus;
        match focus {
            CFocus::Subject => {
                let f = &mut self.r.changes.subject;
                match k {
                    Key::Enter | Key::Down | Key::Tab => self.r.changes.focus = CFocus::Desc,
                    Key::BackTab | Key::Up => self.r.changes.focus = CFocus::Staged,
                    Key::Esc => self.r.changes.focus = CFocus::Unstaged,
                    Key::Char(c) => f.insert(c.encode_utf8(&mut [0; 4])),
                    Key::Backspace => f.backspace(),
                    Key::CtrlBackspace => f.delete_word(),
                    Key::Delete => f.delete(),
                    Key::Left => f.left(),
                    Key::Right => f.right(),
                    Key::Home => f.home(),
                    Key::End => f.end(),
                    Key::Ctrl('a') => f.select_all(),
                    Key::Ctrl('c') => crate::platform::set_clipboard(f.copy_text()),
                    Key::Ctrl('x') => {
                        let t = f.cut();
                        crate::platform::set_clipboard(&t);
                    }
                    Key::Ctrl('v') => {
                        if let Some(t) = crate::platform::clipboard() {
                            f.insert(t.lines().next().unwrap_or(""));
                        }
                    }
                    _ => return false,
                }
            }
            CFocus::Desc => {
                let t = &mut self.r.changes.desc;
                match k {
                    Key::Enter => t.newline(),
                    Key::Tab => self.r.changes.focus = CFocus::Unstaged,
                    Key::BackTab => self.r.changes.focus = CFocus::Subject,
                    Key::Esc => self.r.changes.focus = CFocus::Unstaged,
                    Key::Up => {
                        if !t.vertical(false) {
                            self.r.changes.focus = CFocus::Subject;
                        }
                    }
                    Key::Down => {
                        t.vertical(true);
                    }
                    Key::Char(c) => t.insert(c.encode_utf8(&mut [0; 4])),
                    Key::Backspace => t.backspace(),
                    Key::CtrlBackspace => t.delete_word(),
                    Key::Delete => t.delete(),
                    Key::Left => t.left(),
                    Key::Right => t.right(),
                    Key::Home => t.home(),
                    Key::End => t.end(),
                    Key::Ctrl('a') => t.select_all(),
                    Key::Ctrl('c') => crate::platform::set_clipboard(&t.text()),
                    Key::Ctrl('v') => {
                        if let Some(p) = crate::platform::clipboard() {
                            t.insert(&p);
                        }
                    }
                    _ => return false,
                }
            }
            CFocus::Diff => match k {
                Key::Up | Key::Char('k') => self.scroll_change_diff(-1),
                Key::Down | Key::Char('j') => self.scroll_change_diff(1),
                Key::PageUp => self.scroll_change_diff(-20),
                Key::PageDown => self.scroll_change_diff(20),
                Key::Tab => self.r.changes.focus = CFocus::Subject,
                Key::BackTab | Key::Esc => {
                    let s = self.r.changes.side;
                    self.r.changes.focus = if s == Side::Unstaged { CFocus::Unstaged } else { CFocus::Staged };
                }
                _ => return false,
            },
            CFocus::Unstaged | CFocus::Staged => {
                let side = self.r.changes.side;
                let sel = self.r.changes.sel;
                let n = self.side_list(side).len();
                match k {
                    Key::Up | Key::Char('k') => {
                        if sel == 0 && side == Side::Staged {
                            let m = self.side_list(Side::Unstaged).len();
                            if m > 0 {
                                self.select_change(Side::Unstaged, m - 1);
                            }
                        } else {
                            self.select_change(side, sel.saturating_sub(1));
                        }
                    }
                    Key::Down | Key::Char('j') => {
                        if sel + 1 >= n && side == Side::Unstaged && !self.side_list(Side::Staged).is_empty() {
                            self.select_change(Side::Staged, 0);
                        } else {
                            self.select_change(side, sel + 1);
                        }
                    }
                    Key::Home => self.select_change(side, 0),
                    Key::End => self.select_change(side, usize::MAX),
                    Key::PageUp => self.select_change(side, sel.saturating_sub(10)),
                    Key::PageDown => self.select_change(side, sel + 10),
                    Key::Char(' ') | Key::Enter => self.toggle_selected(),
                    Key::Right => self.r.changes.focus = CFocus::Diff,
                    Key::Tab => {
                        let other = if side == Side::Unstaged { Side::Staged } else { Side::Unstaged };
                        if side == Side::Unstaged && !self.side_list(other).is_empty() {
                            self.select_change(other, 0);
                        } else {
                            self.r.changes.focus = CFocus::Subject;
                        }
                    }
                    Key::BackTab => self.r.changes.focus = CFocus::Desc,
                    Key::Char(c) if c.is_alphanumeric() => {
                        // Typing starts the commit subject.
                        self.r.changes.focus = CFocus::Subject;
                        self.r.changes.subject.insert(c.encode_utf8(&mut [0; 4]));
                    }
                    _ => return false,
                }
            }
        }
        self.changes_dirty();
        true
    }

    fn scroll_change_diff(&mut self, d: i64) {
        let n = self.r.changes.diff.as_ref().map_or(0, |d| d.lines.len());
        let top = (self.r.changes.diff_top as i64 + d).clamp(0, n.saturating_sub(1) as i64) as usize;
        if top != self.r.changes.diff_top {
            self.r.changes.diff_top = top;
            let r = self.changes_layout().diff;
            self.damage.rects.push(r);
        }
    }

    pub(super) fn changes_wheel(&mut self, x: i32, y: i32, n: i64) {
        let l = self.changes_layout();
        if l.diff.contains(x, y) {
            return self.scroll_change_diff(n);
        }
        for side in [Side::Unstaged, Side::Staged] {
            let i = side_ix(side);
            if l.lists[i].contains(x, y) {
                let len = self.side_list(side).len();
                let fit = (l.lists[i].h / self.change_row_h().max(1)).max(1) as usize;
                let t = &mut self.r.changes.tops[i];
                *t = (*t as i64 + n).clamp(0, len.saturating_sub(fit) as i64) as usize;
                self.changes_dirty();
            }
        }
    }

    pub(super) fn change_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.r.changes_view {
            return None;
        }
        self.r.changes.hits.iter().position(|h| h.0.contains(x, y))
    }

    pub(super) fn change_hit_is_text(&self, i: usize) -> bool {
        matches!(self.r.changes.hits.get(i), Some((_, CHit::Subject | CHit::Desc)))
    }

    /// A click in the view; false when it was outside.
    pub(super) fn changes_click(&mut self, x: i32, y: i32) -> bool {
        let l = self.changes_layout();
        if !l.area.contains(x, y) {
            return false;
        }
        let hit = self.r.changes.hits.iter().find(|h| h.0.contains(x, y)).map(|h| h.1);
        match hit {
            Some(CHit::Row(side, i)) => {
                if self.r.changes.side == side && self.r.changes.sel == i && self.double_click_on(x, y) {
                    self.toggle_selected();
                } else {
                    self.select_change(side, i);
                }
            }
            Some(CHit::StageAll) => self.stage_all(true),
            Some(CHit::UnstageAll) => self.stage_all(false),
            Some(CHit::Hunk(h)) => self.toggle_hunk(h),
            Some(CHit::Subject) => self.r.changes.focus = CFocus::Subject,
            Some(CHit::Desc) => self.r.changes.focus = CFocus::Desc,
            Some(CHit::Amend) => {
                let on = !self.r.changes.amend;
                self.set_amend(on);
            }
            Some(CHit::Commit) => self.commit_changes(),
            None if l.diff.contains(x, y) => self.r.changes.focus = CFocus::Diff,
            None => {}
        }
        self.changes_dirty();
        true
    }

    /// The click that just happened was a double click.
    fn double_click_on(&self, _x: i32, _y: i32) -> bool {
        self.double
    }

    fn change_hot(&self, hits_len: usize) -> bool {
        self.is_hot(Hot::Change(hits_len))
    }

    pub(super) fn paint_changes(&mut self, cv: &mut Canvas) {
        let l = self.changes_layout();
        let mut hits: Vec<(Rect, CHit)> = Vec::new();
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        cv.fill(l.left, BG);
        cv.fill(Rect::new(l.left.right() - 1, l.left.y, 1, l.left.h), BORDER);
        let loading = self.r.changes.status.is_none();
        let rh = self.change_row_h();
        for side in [Side::Unstaged, Side::Staged] {
            let i = side_ix(side);
            let (head, list) = (l.heads[i], l.lists[i]);
            let entries = self.side_list(side);
            cv.fill(head, BG_BAR);
            cv.fill(Rect::new(head.x, head.bottom() - 1, head.w, 1), BORDER_SOFT);
            let base = (head.y as f32 + head.h as f32 / 2.0 + self.pf(4.5)).round();
            let title = if side == Side::Unstaged { "Unstaged" } else { "Staged" };
            let e =
                self.te.draw(cv, semi, self.pf(12.5), head.x as f32 + self.pf(14.0), base, title, TEXT, self.pf(120.0));
            let count = if loading { "…".to_string() } else { entries.len().to_string() };
            self.te.draw(cv, ui, self.pf(12.0), e + self.pf(8.0), base, &count, TEXT_3, self.pf(80.0));
            // Stage all / Unstage all.
            let (label, hit) =
                if side == Side::Unstaged { ("Stage all", CHit::StageAll) } else { ("Unstage all", CHit::UnstageAll) };
            let bw = self.te.measure(semi, self.pf(11.5), label) + self.pf(20.0);
            let b = Rect::new(
                head.right() - self.px(10.0) - bw as i32,
                head.y + self.px(4.0),
                bw as i32,
                head.h - self.px(8.0),
            );
            if !entries.is_empty() {
                let (bg, border) = self.control_colors(Hot::Change(hits.len()), BG_BAR, BORDER);
                cv.round_border(b, self.pf(6.0), border, bg);
                self.te.draw(
                    cv,
                    semi,
                    self.pf(11.5),
                    b.x as f32 + self.pf(10.0),
                    base - self.pf(0.5),
                    label,
                    TEXT_2,
                    bw,
                );
                hits.push((b, hit));
            }
            // Rows.
            cv.fill(list, BG);
            if entries.is_empty() && !loading {
                let t = if side == Side::Unstaged {
                    "No changes in the working tree"
                } else {
                    "Nothing staged · Space stages the selected file"
                };
                self.te.draw(
                    cv,
                    ui,
                    self.pf(12.0),
                    list.x as f32 + self.pf(16.0),
                    list.y as f32 + self.pf(22.0),
                    t,
                    TEXT_4,
                    list.w as f32,
                );
            }
            let fit = (list.h / rh.max(1)).max(0) as usize;
            let top = self.r.changes.tops[i].min(entries.len().saturating_sub(fit.max(1)));
            let old = cv.set_clip(cv.clip().intersect(list));
            for (k, en) in entries.iter().enumerate().skip(top).take(fit + 1) {
                let ry = list.y + (k - top) as i32 * rh;
                let row = Rect::new(list.x + self.px(6.0), ry, list.w - self.px(12.0), rh);
                let selected = self.r.changes.side == side && self.r.changes.sel == k;
                let focused = matches!(
                    (self.r.changes.focus, side),
                    (CFocus::Unstaged, Side::Unstaged) | (CFocus::Staged, Side::Staged)
                );
                if selected {
                    cv.fill_round(row, self.pf(6.0), if focused { BG_SELECT } else { BG_SELECT_DIM });
                } else if self.change_hot(hits.len()) {
                    cv.fill_round(row, self.pf(6.0), BG_HOVER);
                }
                let cy = ry as f32 + rh as f32 / 2.0;
                let b = (cy + self.pf(4.5)).round();
                let letter = en.letter(side);
                let sc = match letter {
                    b'A' => ADD,
                    b'D' => DEL,
                    b'R' | b'C' => REN,
                    b'U' => DEL,
                    _ => MOD,
                };
                let badge = Rect::new(row.x + self.px(8.0), (cy - self.pf(8.0)) as i32, self.px(16.0), self.px(16.0));
                cv.fill_round(badge, self.pf(4.0), crate::ui::tint(sc));
                let ls = (letter as char).to_string();
                let lw = self.te.measure(semi, self.pf(10.5), &ls);
                self.te.draw(
                    cv,
                    semi,
                    self.pf(10.5),
                    badge.x as f32 + (badge.w as f32 - lw) / 2.0,
                    b - self.pf(0.5),
                    &ls,
                    sc,
                    self.pf(16.0),
                );
                let (dir, name) = match en.path.rsplit_once('/') {
                    Some((d, n)) => (d.to_string(), n.to_string()),
                    None => (String::new(), en.path.to_string()),
                };
                let nx = badge.right() as f32 + self.pf(10.0);
                let e2 =
                    self.te.draw(cv, ui, self.pf(12.5), nx, b, &name, TEXT, row.right() as f32 - nx - self.pf(8.0));
                if !dir.is_empty() {
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(11.5),
                        e2 + self.pf(8.0),
                        b,
                        &dir,
                        TEXT_4,
                        row.right() as f32 - e2 - self.pf(16.0),
                    );
                }
                hits.push((row.intersect(list), CHit::Row(side, k)));
            }
            cv.set_clip(old);
        }
        self.paint_commit_box(cv, l.commit, &mut hits);
        self.paint_changes_diff(cv, l.diff, &mut hits);
        self.r.changes.hits = hits;
        let _ = mono;
    }

    fn paint_commit_box(&mut self, cv: &mut Canvas, r: Rect, hits: &mut Vec<(Rect, CHit)>) {
        let (ui, semi) = (self.f.ui, self.f.semi);
        cv.fill(r, BG_PANEL);
        cv.fill(Rect::new(r.x, r.y, r.w - 1, 1), BORDER);
        let pad = self.px(12.0);
        let focus = self.r.changes.focus;
        // Subject.
        let sf = Rect::new(r.x + pad, r.y + pad, r.w - 2 * pad, self.px(32.0));
        let on = focus == CFocus::Subject;
        let border = if on {
            ACCENT
        } else if self.change_hot(hits.len()) {
            BORDER_HOVER
        } else {
            BORDER
        };
        cv.round_border(sf, self.pf(7.0), border, BG_INPUT);
        let sb = (sf.y as f32 + sf.h as f32 / 2.0 + self.pf(4.5)).round();
        let tx = sf.x as f32 + self.pf(10.0);
        let maxw = sf.w as f32 - self.pf(60.0);
        let subject = self.r.changes.subject.clone();
        if subject.text().is_empty() {
            let ph = if self.r.changes.amend { "Subject (amend keeps the last one)" } else { "Commit subject" };
            self.te.draw(cv, ui, self.pf(12.5), tx, sb, ph, TEXT_4, maxw);
        } else {
            if subject.all_selected() {
                let w = self.te.measure(ui, self.pf(12.5), subject.text()).min(maxw);
                cv.fill_round(
                    Rect::new(tx as i32 - 2, sf.y + self.px(6.0), w as i32 + 4, sf.h - self.px(12.0)),
                    self.pf(3.0),
                    BG_TEXT_SELECT,
                );
            }
            self.te.draw(cv, semi, self.pf(12.5), tx, sb, subject.text(), TEXT, maxw);
        }
        if on {
            let cx = tx + self.te.measure(semi, self.pf(12.5), &subject.text()[..subject.cursor()]);
            cv.fill(Rect::new(cx as i32 + 1, sf.y + self.px(7.0), self.px(1.5).max(1), sf.h - self.px(14.0)), ACCENT);
        }
        // Subject length: Git convention is 50, a hard look at 72.
        let n = subject.text().chars().count();
        if n > 0 {
            let s = n.to_string();
            let c = if n > 72 {
                DEL
            } else if n > 50 {
                MOD
            } else {
                TEXT_4
            };
            let w = self.te.measure(ui, self.pf(11.0), &s);
            self.te.draw(cv, ui, self.pf(11.0), sf.right() as f32 - w - self.pf(10.0), sb, &s, c, w + 2.0);
        }
        hits.push((sf, CHit::Subject));
        // Description.
        let df =
            Rect::new(sf.x, sf.bottom() + self.px(8.0), sf.w, r.bottom() - sf.bottom() - self.px(8.0) - self.px(50.0));
        let on = focus == CFocus::Desc;
        let border = if on {
            ACCENT
        } else if self.change_hot(hits.len()) {
            BORDER_HOVER
        } else {
            BORDER
        };
        cv.round_border(df, self.pf(7.0), border, BG_INPUT);
        let desc = self.r.changes.desc.clone();
        let lh = self.pf(18.0);
        let old = cv.set_clip(cv.clip().intersect(df.inset(self.px(2.0), self.px(2.0))));
        if desc.is_empty() && !on {
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                tx,
                df.y as f32 + self.pf(20.0),
                "Description (optional) · Ctrl+Enter commits",
                TEXT_4,
                maxw,
            );
        } else {
            let fit = ((df.h as f32 - self.pf(8.0)) / lh).max(1.0) as usize;
            let (row, col) = desc.cursor();
            let first = row.saturating_sub(fit.saturating_sub(1));
            for (k, line) in desc.lines().iter().enumerate().skip(first).take(fit) {
                let y = df.y as f32 + self.pf(20.0) + (k - first) as f32 * lh;
                self.te.draw(
                    cv,
                    ui,
                    self.pf(12.5),
                    tx,
                    y,
                    line,
                    if desc.all_selected() { ACCENT_FG } else { TEXT_2 },
                    df.w as f32 - self.pf(20.0),
                );
                if on && k == row {
                    let cx = tx + self.te.measure(ui, self.pf(12.5), &line[..col.min(line.len())]);
                    cv.fill(
                        Rect::new(cx as i32 + 1, (y - self.pf(13.0)) as i32, self.px(1.5).max(1), self.px(17.0)),
                        ACCENT,
                    );
                }
            }
        }
        cv.set_clip(old);
        hits.push((df, CHit::Desc));
        // Amend and Commit.
        let by = r.bottom() - self.px(42.0);
        let amend = self.r.changes.amend;
        let ab = Rect::new(sf.x, by, self.px(90.0), self.px(30.0));
        if self.change_hot(hits.len()) {
            cv.fill_round(ab, self.pf(6.0), BG_HOVER);
        }
        let bx = Rect::new(ab.x + self.px(8.0), ab.y + self.px(8.0), self.px(14.0), self.px(14.0));
        if amend {
            cv.fill_round(bx, self.pf(3.0), ACCENT);
            let (x0, y0) = (bx.x as f32, bx.y as f32);
            cv.stroke(
                &[
                    (x0 + self.pf(3.0), y0 + self.pf(7.5)),
                    (x0 + self.pf(6.0), y0 + self.pf(10.5)),
                    (x0 + self.pf(11.0), y0 + self.pf(4.0)),
                ],
                self.pf(1.8),
                ACCENT_TEXT,
            );
        } else {
            cv.round_border(bx, self.pf(3.0), BORDER_HOVER, BG_INPUT);
        }
        self.te.draw(
            cv,
            ui,
            self.pf(12.5),
            bx.right() as f32 + self.pf(8.0),
            (ab.y as f32 + self.pf(20.0)).round(),
            "Amend",
            TEXT_2,
            self.pf(60.0),
        );
        hits.push((ab, CHit::Amend));
        let staged = self.side_list(Side::Staged).len();
        let label = match self.r.changes.busy {
            Some(v) => format!("{v}…"),
            None if amend => "Amend commit".to_string(),
            None if staged == 1 => "Commit 1 file".to_string(),
            None => format!("Commit {staged} files"),
        };
        let can = self.r.changes.busy.is_none() && (staged > 0 || amend);
        let kc = "Ctrl ↵";
        let kw = self.te.measure(ui, self.pf(10.5), kc);
        let w = self.te.measure(semi, self.pf(12.5), &label) + kw + self.pf(40.0);
        let cb = Rect::new(r.right() - pad - w as i32, by, w as i32, self.px(30.0));
        let hot = self.change_hot(hits.len());
        let (bg, border, fg) = if !can {
            (BG_INPUT, BORDER_SOFT, TEXT_4)
        } else if self.is_pressed(Hot::Change(hits.len())) {
            (ACCENT_SOFT, ACCENT, ACCENT_FG)
        } else if hot {
            (ACCENT, ACCENT, ACCENT_TEXT)
        } else {
            (ACCENT_SOFT_HOVER, ACCENT_BORDER, ACCENT_FG)
        };
        cv.round_border(cb, self.pf(7.0), border, bg);
        self.te.draw(
            cv,
            semi,
            self.pf(12.5),
            cb.x as f32 + self.pf(14.0),
            (cb.y as f32 + self.pf(20.0)).round(),
            &label,
            fg,
            w,
        );
        self.te.draw(
            cv,
            ui,
            self.pf(10.5),
            cb.right() as f32 - kw - self.pf(10.0),
            (cb.y as f32 + self.pf(19.5)).round(),
            kc,
            if can && hot { ACCENT_TEXT } else { TEXT_4 },
            kw + 2.0,
        );
        if can {
            hits.push((cb, CHit::Commit));
        }
        if let Some(e) = self.r.changes.error.clone() {
            let ex = ab.right() as f32 + self.pf(8.0);
            self.te.draw(
                cv,
                ui,
                self.pf(11.5),
                ex,
                (by as f32 + self.pf(20.0)).round(),
                &e,
                DEL,
                cb.x as f32 - ex - self.pf(8.0),
            );
        }
    }

    fn paint_changes_diff(&mut self, cv: &mut Canvas, r: Rect, hits: &mut Vec<(Rect, CHit)>) {
        let (ui, semi) = (self.f.ui, self.f.semi);
        cv.fill(r, BG);
        let bar = Rect::new(r.x, r.y, r.w, self.px(32.0));
        cv.fill(bar, BG_BAR);
        cv.fill(Rect::new(bar.x, bar.bottom() - 1, bar.w, 1), BORDER_SOFT);
        let base = (bar.y as f32 + bar.h as f32 / 2.0 + self.pf(4.5)).round();
        let pad = self.pf(16.0);
        let side = self.r.changes.side;
        let sel = self.selected_entry();
        let title = sel.as_ref().map_or_else(String::new, |e| e.path.to_string());
        let focus = self.r.changes.focus == CFocus::Diff;
        self.te.draw(
            cv,
            semi,
            self.pf(12.5),
            r.x as f32 + pad,
            base,
            &title,
            if focus { ACCENT } else { TEXT },
            r.w as f32 - self.pf(200.0),
        );
        let hint = match side {
            Side::Unstaged => "unstaged · click a hunk's button to stage it",
            Side::Staged => "staged · click a hunk's button to unstage it",
        };
        let hw = self.te.measure(ui, self.pf(11.0), hint);
        self.te.draw(cv, ui, self.pf(11.0), r.right() as f32 - hw - pad, base, hint, TEXT_4, hw + 2.0);
        let body = Rect::new(r.x, bar.bottom(), r.w, r.bottom() - bar.bottom());
        let Some(diff) = self.r.changes.diff.clone() else {
            let t = if sel.is_some() { "Loading the diff…" } else { "Select a file to see its changes" };
            self.te.draw(cv, ui, self.pf(12.5), r.x as f32 + pad, body.y as f32 + self.pf(26.0), t, TEXT_4, r.w as f32);
            return;
        };
        if sel.as_ref().is_some_and(|e| e.conflicted) {
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                r.x as f32 + pad,
                body.y as f32 + self.pf(26.0),
                "Conflicted: resolve it in your merge tool, then stage it (the resolver comes with FF-42)",
                MOD,
                r.w as f32,
            );
            return;
        }
        let label = if side == Side::Unstaged { "Stage hunk" } else { "Unstage hunk" };
        let top = self.r.changes.diff_top;
        let buttons = self.paint_diff_body(cv, body, &diff, top, Some(label), hits.len());
        for (h, rect) in buttons {
            hits.push((rect, CHit::Hunk(h)));
        }
    }
}
