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
    SkipHooks,
    /// The Unstaged list's options (`…`): show or hide untracked files.
    ListOptions,
    /// Push after the commit (FF-35; remembered as `commit.push`).
    PushAfter,
    StageLines,
    /// Unified or side by side (FF-35; the same choice as the history's).
    Layout,
    Commit,
    /// AI draft: a commit message from the staged changes (FF-50).
    Ai,
    /// A button of the AI pane.
    AiButton(ai_view::AiHit),
    /// The conflict resolver and the stopped-operation banner.
    Conflict(conflict_view::KHit),
}

pub struct Changes {
    pub status: Option<WorkStatus>,
    /// Files per side (unstaged, staged), counted once per status.
    counts: [usize; 2],
    /// Conflicted files, counted once per status.
    pub conflicts: usize,
    /// Untracked files left out of the list (`changes.hide_untracked`).
    pub hidden_untracked: usize,
    /// Ignored paths, merged into the list while `changes.show_ignored`.
    ignored: Vec<Entry>,
    pub side: Side,
    pub sel: usize,
    tops: [usize; 2],
    pub diff: Option<Box<FileDiff>>,
    /// A stopped merge, rebase, cherry-pick or revert.
    pub in_progress: Option<&'static str>,
    /// The selected conflicted file's conflicts.
    pub conflict: Option<conflict_view::ConflictView>,
    diff_top: usize,
    pub(super) seq: u64,
    pub subject: TextField,
    pub desc: TextArea,
    pub amend: bool,
    pub focus: CFocus,
    /// The AI commit message pane, shown in place of the diff.
    pub ai: Option<ai_view::AiPane>,
    /// An operation is running (its verb, for the commit button).
    pub busy: Option<&'static str>,
    pub hits: Vec<(Rect, CHit)>,
    /// The last error, shown above the commit button.
    pub error: Option<String>,
    pub skip_hooks: bool,
    /// Line cursor in the diff (an index into the diff's lines).
    pub line: usize,
    /// Lines picked for staging (indexes into the diff's lines).
    pub picked: std::collections::BTreeSet<usize>,
    /// Discards that Ctrl+Z can undo, newest last.
    pub undo: Vec<ffwd_core::changes::Backup>,
    /// A discard waiting for its confirming second Delete.
    armed: Option<(std::time::Instant, Vec<String>)>,
    /// Which recent message Ctrl+R showed last.
    recent_ix: usize,
}

impl Default for Changes {
    fn default() -> Self {
        Changes {
            ai: None,
            status: None,
            counts: [0, 0],
            conflicts: 0,
            hidden_untracked: 0,
            ignored: Vec::new(),
            side: Side::Unstaged,
            sel: 0,
            tops: [0, 0],
            diff: None,
            in_progress: None,
            conflict: None,
            diff_top: 0,
            seq: 0,
            subject: TextField::default(),
            desc: TextArea::default(),
            amend: false,
            focus: CFocus::Unstaged,
            busy: None,
            hits: Vec::new(),
            error: None,
            skip_hooks: false,
            line: 0,
            picked: std::collections::BTreeSet::new(),
            undo: Vec::new(),
            armed: None,
            recent_ix: 0,
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
    /// The stopped-operation banner (zero height when none).
    pub banner: Rect,
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
        let ch = self.px(172.0).min(left.h / 2);
        let commit = Rect::new(left.x, left.bottom() - ch, left.w, ch);
        let head = self.px(32.0);
        // The stopped-operation banner: two rows (state, then buttons)
        // when the column is too narrow for both in one (FF-57).
        let two_rows = lw < self.px(if self.r.changes.conflicts > 1 { 590.0 } else { 400.0 });
        let bh = match (self.r.changes.in_progress.is_some(), two_rows) {
            (false, _) => 0,
            (true, false) => self.px(36.0),
            (true, true) => self.px(66.0),
        };
        let banner = Rect::new(left.x, left.y, left.w, bh);
        let lists_h = (commit.y - left.y - bh - 2 * head).max(0);
        // Each list gets a share by how many files it shows, at least
        // about three rows; an empty list doesn't hold half the column.
        let rh = self.change_row_h();
        let need = |n: usize| (n.max(1) as i32 * rh + self.px(12.0)) as i64;
        let (n0, n1) = (need(self.side_len(Side::Unstaged)), need(self.side_len(Side::Staged)));
        let min = self.px(96.0).min(lists_h / 2);
        let uh = ((lists_h as i64 * n0 / (n0 + n1).max(1)) as i32).clamp(min, (lists_h - min).max(min));
        let h0 = Rect::new(left.x, left.y + bh, left.w, head);
        let l0 = Rect::new(left.x, h0.bottom(), left.w, uh);
        let h1 = Rect::new(left.x, l0.bottom(), left.w, head);
        let l1 = Rect::new(left.x, h1.bottom(), left.w, commit.y - h1.bottom());
        CLayout {
            area,
            left,
            heads: [h0, h1],
            lists: [l0, l1],
            commit,
            banner,
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
            self.r.files_view = false;
            self.r.activity_view = false;
            self.r.reflog_view = false;
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

    /// Whether untracked files are left out of Local Changes.
    pub(super) fn hide_untracked(&self) -> bool {
        self.settings.get("changes.hide_untracked") == Some("true")
    }

    pub(super) fn set_hide_untracked(&mut self, hide: bool) {
        self.settings.set_string("changes.hide_untracked", if hide { "true" } else { "false" });
        self.reload_changes();
        self.changes_dirty_all();
    }

    pub(super) fn reload_changes(&mut self) {
        if let (Some(g), Some(s)) = (self.git_for_tab(), self.tab_sink()) {
            if self.show_ignored() {
                crate::work::ignored_load(g.clone(), &s);
            }
            crate::work::changes_load(g, &s);
        }
    }

    /// Whether ignored files show in Local Changes (`changes.show_ignored`).
    pub(super) fn show_ignored(&self) -> bool {
        self.settings.get("changes.show_ignored") == Some("true")
    }

    pub(super) fn set_show_ignored(&mut self, show: bool) {
        self.settings.set_string("changes.show_ignored", if show { "true" } else { "false" });
        if !show {
            self.r.changes.ignored.clear();
        }
        self.reload_changes();
        self.changes_dirty_all();
    }

    /// Ignored paths arrived: kept, and the list reloads to merge them.
    pub(super) fn on_ignored(&mut self, r: Result<Vec<String>, String>) {
        match r {
            Ok(paths) => {
                self.r.changes.ignored = paths.iter().map(|p| Entry::ignored_entry(p)).collect();
                if let (Some(g), Some(s)) = (self.git_for_tab(), self.tab_sink()) {
                    crate::work::changes_load(g, &s);
                }
            }
            Err(e) => self.set_notice(format!("Could not list ignored files: {e}")),
        }
    }

    /// How many files a side has (cached; no walk over the status).
    pub(super) fn side_len(&self, side: Side) -> usize {
        if self.r.changes.status.is_none() { 0 } else { self.r.changes.counts[side_ix(side)] }
    }

    /// Rows `top..top + n` of a side, cloned (only what is shown).
    pub(super) fn side_rows(&self, side: Side, top: usize, n: usize) -> Vec<Entry> {
        self.r.changes.status.as_ref().map_or_else(Vec::new, |s| {
            s.entries.iter().filter(|e| in_side(e, side)).skip(top).take(n).cloned().collect()
        })
    }

    fn side_list(&self, side: Side) -> Vec<Entry> {
        self.r.changes.status.as_ref().map_or_else(Vec::new, |s| s.side(side).into_iter().cloned().collect())
    }

    pub(super) fn selected_entry(&self) -> Option<Entry> {
        self.side_rows(self.r.changes.side, self.r.changes.sel, 1).pop()
    }

    fn request_change_diff(&mut self) {
        let Some(e) = self.selected_entry() else {
            self.r.changes.diff = None;
            return;
        };
        if e.conflicted {
            self.r.changes.diff = None;
            return self.request_conflict(e.path.to_string());
        }
        self.r.changes.conflict = None;
        self.r.changes.seq += 1;
        let (seq, side) = (self.r.changes.seq, self.r.changes.side);
        if let Some(w) = &self.r.worker {
            w.change_diff(seq, e, side);
        }
    }

    /// The left pane changed (lists, commit box); a key press repaints
    /// only this, never the diff (rule 2: damage stays small).
    pub(super) fn changes_dirty(&mut self) {
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
            Ok(mut st) => {
                // Hidden untracked files (remembered; FF-35) never reach the list.
                let before = st.entries.len();
                if self.hide_untracked() {
                    st.entries.retain(|e| !e.untracked());
                }
                if self.show_ignored() {
                    st.entries.extend(self.r.changes.ignored.iter().cloned());
                }
                let c = &mut self.r.changes;
                c.hidden_untracked = before.saturating_sub(st.entries.iter().filter(|e| !e.ignored()).count());
                c.counts = [Side::Unstaged, Side::Staged].map(|s| st.entries.iter().filter(|e| in_side(e, s)).count());
                c.conflicts = st.entries.iter().filter(|e| e.conflicted).count();
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
                    let n = self.side_len(self.r.changes.side);
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
                    self.r.changes.line = 0;
                }
                // Line numbers shift after any staging: picks start over.
                self.r.changes.picked.clear();
                // A new diff starts at the left edge.
                self.code_left = 0.0;
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

    /// Whether a commit is followed by a push (`commit.push`).
    pub(super) fn push_after_commit(&self) -> bool {
        self.settings.get("commit.push") == Some("true")
    }

    pub(super) fn on_changes_op(&mut self, commit: bool, r: Result<String, String>) {
        self.r.changes.busy = None;
        match r {
            Ok(msg) => {
                self.r.changes.error = None;
                if commit {
                    self.remember_message();
                    let c = &mut self.r.changes;
                    c.skip_hooks = false;
                    c.subject = TextField::default();
                    c.desc = TextArea::default();
                    c.amend = false;
                    c.focus = CFocus::Unstaged;
                    self.set_notice(msg);
                    // History, refs and the sidebar changed.
                    self.reload_current();
                    if self.push_after_commit() {
                        // Commit and push: the push runs like the toolbar's.
                        self.start_op(ffwd_core::ops::Op::Push);
                    }
                    return;
                }
                if msg.starts_with("Discarded") && !self.r.changes.undo.is_empty() {
                    // The click twin of Ctrl+Z (FF-57).
                    self.set_notice_with(msg, super::NoticeAct::Undo);
                } else {
                    self.set_notice(msg);
                }
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

    pub(super) fn run_change_op(&mut self, op: crate::work::ChangeOp, verb: &'static str) {
        if self.r.changes.busy.is_some() {
            return self.set_notice("Another change is still running.".into());
        }
        let (Some(g), Some(s)) = (self.git_for_tab(), self.tab_sink()) else { return };
        self.r.changes.busy = Some(verb);
        crate::work::changes_op(g, op, &s);
        self.changes_dirty();
    }

    /// Stages or unstages the selected file.
    /// The file menu's Stage / Unstage.
    pub(super) fn toggle_selected_change(&mut self) {
        self.toggle_selected();
    }

    fn toggle_selected(&mut self) {
        let Some(e) = self.selected_entry() else { return };
        if e.ignored() {
            return self.set_notice(format!("{} is ignored: it isn't part of the repository.", e.path));
        }
        if e.conflicted {
            // Staging it now would mark the markers as the resolution.
            self.r.changes.focus = CFocus::Diff;
            self.changes_dirty_all();
            return self.set_notice(
                "Resolve the conflicts first (o, t, b on each), or take one side for the whole file.".into(),
            );
        }
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
        let staged = self.side_len(Side::Staged);
        if staged == 0 && !c.amend {
            return self.set_notice("Nothing is staged: stage files first (Space), or tick Amend.".into());
        }
        if c.subject.text().trim().is_empty() && !c.amend {
            self.r.changes.focus = CFocus::Subject;
            self.changes_dirty();
            return self.set_notice("Write a commit subject first.".into());
        }
        let msg = ffwd_core::changes::message(c.subject.text(), &c.desc.text());
        let opts = CommitOptions { amend: c.amend, no_verify: c.skip_hooks, ..Default::default() };
        self.run_change_op(crate::work::ChangeOp::Commit(msg, opts), "Committing");
    }

    /// Keeps the committed message for Ctrl+R (the newest 20).
    fn remember_message(&mut self) {
        let c = &self.r.changes;
        let m = ffwd_core::changes::message(c.subject.text(), &c.desc.text()).trim_end().to_string();
        if m.is_empty() {
            return;
        }
        let mut recent = self.state.get_list("commit.recent");
        // The state file stores lists with `|` between entries.
        let enc = m.replace('|', "\u{2}").replace('\n', "\u{1}");
        recent.retain(|r| *r != enc);
        recent.insert(0, enc);
        recent.truncate(20);
        self.state.set_list("commit.recent", &recent);
    }

    /// Ctrl+R: the next recent message into the subject and description.
    fn recall_message(&mut self) {
        let recent = self.state.get_list("commit.recent");
        if recent.is_empty() {
            return self.set_notice("No recent commit messages yet.".into());
        }
        let c = &mut self.r.changes;
        let m = recent[c.recent_ix % recent.len()].replace('\u{1}', "\n").replace('\u{2}', "|");
        c.recent_ix += 1;
        let (subject, rest) = m.split_once('\n').unwrap_or((m.as_str(), ""));
        c.subject = TextField::new(subject.trim());
        c.desc = TextArea::new(rest.trim_start_matches('\n'));
        c.focus = CFocus::Subject;
        let n = recent.len();
        self.set_notice(format!(
            "Recent message {} of {n} (Ctrl+R for the next)",
            (self.r.changes.recent_ix - 1) % n + 1
        ));
    }

    /// Delete on a file: the first press names what will be lost, the
    /// second (within 4 s) discards it; Ctrl+Z puts it back (rule 8).
    pub(super) fn discard_selected(&mut self) {
        if self.r.changes.side != Side::Unstaged {
            return self.set_notice("Discard works on unstaged changes; unstage first.".into());
        }
        let Some(e) = self.selected_entry() else { return };
        if e.ignored() {
            return self.set_notice(format!("{} is ignored: it isn't part of the repository.", e.path));
        }
        let path = e.path.to_string();
        let armed = self.r.changes.armed.take();
        if let Some((t, paths)) = armed
            && t.elapsed().as_secs() < 4
            && paths == [path.clone()]
        {
            return self.run_change_op(crate::work::ChangeOp::Discard(vec![e]), "Discarding");
        }
        let what = if e.untracked() { "delete the new file" } else { "throw away the unstaged changes of" };
        self.r.changes.armed = Some((std::time::Instant::now(), vec![path.clone()]));
        self.set_notice_with(
            format!("Discard? This will {what} {path}; Undo (Ctrl+Z) brings it back."),
            super::NoticeAct::Discard,
        );
    }

    /// The status bar's Discard button: confirms the armed discard.
    pub(super) fn discard_now(&mut self) {
        let Some((_, paths)) = self.r.changes.armed.take() else { return };
        let list = self.side_list(Side::Unstaged);
        let entries: Vec<_> = paths.iter().filter_map(|p| list.iter().find(|e| *e.path == **p).cloned()).collect();
        if !entries.is_empty() {
            self.run_change_op(crate::work::ChangeOp::Discard(entries), "Discarding");
        }
    }

    pub(super) fn undo_discard_now(&mut self) {
        self.undo_discard();
    }

    fn undo_discard(&mut self) {
        match self.r.changes.undo.pop() {
            Some(b) => self.run_change_op(crate::work::ChangeOp::Restore(b), "Restoring"),
            None => self.set_notice("Nothing to undo.".into()),
        }
    }

    pub(super) fn ignore_selected(&mut self) {
        let Some(e) = self.selected_entry() else { return };
        if !e.untracked() {
            return self.set_notice("Only untracked files can be ignored; tracked files stay tracked.".into());
        }
        let p = format!("/{}", e.path);
        self.run_change_op(crate::work::ChangeOp::Ignore(p), "Ignoring");
    }

    /// The picked lines, grouped by hunk, as body-line indexes.
    fn picked_by_hunk(&self) -> Vec<(usize, Vec<usize>)> {
        let Some(d) = &self.r.changes.diff else { return Vec::new() };
        let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
        let mut hunk: Option<usize> = None;
        let mut body = 0;
        for (i, l) in d.lines.iter().enumerate() {
            if l.kind == LineKind::Hunk {
                hunk = Some(hunk.map_or(0, |h| h + 1));
                body = 0;
                continue;
            }
            if let Some(h) = hunk
                && self.r.changes.picked.contains(&i)
            {
                match out.last_mut() {
                    Some((lh, v)) if *lh == h => v.push(body),
                    _ => out.push((h, vec![body])),
                }
            }
            body += 1;
        }
        out
    }

    fn stage_picked(&mut self) {
        let picks = self.picked_by_hunk();
        let Some(e) = self.selected_entry() else { return };
        if picks.is_empty() {
            return self.set_notice("Pick lines first: Space on a + or − line (or click it).".into());
        }
        let side = self.r.changes.side;
        self.r.changes.picked.clear();
        self.run_change_op(crate::work::ChangeOp::Lines(Box::new(e), side, picks), "Staging");
    }

    /// The Local Changes diff's top line.
    pub(super) fn changes_diff_top(&self) -> usize {
        self.r.changes.diff_top
    }

    /// The line of the Local Changes diff under (x, y), if that pane
    /// shows a diff there (not the AI pane or the conflict resolver).
    pub(super) fn changes_diff_line_at(&mut self, x: i32, y: i32) -> Option<usize> {
        if !self.r.changes_view || self.r.changes.ai.is_some() || self.selected_entry().is_some_and(|e| e.conflicted) {
            return None;
        }
        let r = self.changes_layout().diff;
        let body_y = r.y + self.px(32.0);
        if !r.contains(x, y) || y < body_y {
            return None;
        }
        let d = self.r.changes.diff.clone()?;
        let body = Rect::new(r.x, body_y, r.w, r.bottom() - body_y);
        if self.changes_split() {
            return self.split_line_at(&d, self.r.changes.diff_top, body, x, y);
        }
        self.diff_line_at_y(&d, self.r.changes.diff_top, body, y)
    }

    /// Local Changes shows its diff side by side (FF-35): the diff
    /// layout choice, for text diffs.
    pub(super) fn changes_split(&self) -> bool {
        self.diff_split && !self.r.changes.diff.as_ref().is_some_and(|d| d.binary)
    }

    /// Switches the layout, keeping the line cursor's place on screen
    /// (the diff's top is a line unified, a row side by side).
    pub(super) fn set_changes_split(&mut self, on: bool) {
        let d = self.r.changes.diff.clone();
        let rows = d.as_ref().map(|d| {
            let (p, _) = self.diff_meta(d);
            Self::split_rows(d, &p)
        });
        let c = &mut self.r.changes;
        if let Some(rows) = rows.filter(|r| !r.is_empty()) {
            c.diff_top = if on {
                Self::split_row_of(&rows, c.diff_top)
            } else {
                rows.get(c.diff_top).and_then(|&(l, r)| l.or(r)).unwrap_or(0)
            };
        }
        self.apply_diff_bar(if on { diff_view::DiffBar::Split } else { diff_view::DiffBar::Unified });
        self.damage.full = true;
    }

    fn toggle_pick(&mut self, i: usize) {
        let Some(d) = &self.r.changes.diff else { return };
        if !matches!(d.lines.get(i).map(|l| l.kind), Some(LineKind::Add | LineKind::Del)) {
            return;
        }
        let p = &mut self.r.changes.picked;
        if !p.remove(&i) {
            p.insert(i);
        }
        let r = self.changes_layout().diff;
        self.damage.rects.push(r);
    }

    /// Moves the diff's line cursor, keeping it on screen.
    fn move_line(&mut self, d: i64) {
        let n = self.r.changes.diff.as_ref().map_or(0, |d| d.lines.len());
        if n == 0 {
            return;
        }
        let c = &mut self.r.changes;
        c.line = (c.line as i64 + d).clamp(0, n as i64 - 1) as usize;
        let body_h = self.changes_layout().diff.h - self.px(32.0);
        let fit = (body_h / self.code_lh().max(1)).max(1) as usize;
        // Side by side, the top is a row: follow the cursor's row.
        let at = match self.r.changes.diff.clone().filter(|_| self.changes_split()) {
            Some(d) => {
                let (p, _) = self.diff_meta(&d);
                Self::split_row_of(&Self::split_rows(&d, &p), self.r.changes.line)
            }
            None => self.r.changes.line,
        };
        let c = &mut self.r.changes;
        if at < c.diff_top {
            c.diff_top = at;
        } else if at >= c.diff_top + fit {
            c.diff_top = at + 1 - fit;
        }
        let r = self.changes_layout().diff;
        self.damage.rects.push(r);
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

    pub(super) fn select_change(&mut self, side: Side, i: usize) {
        let n = self.side_len(side);
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
        if self.ai_key(k) {
            return true;
        }
        // A conflict being edited by hand takes every key (FF-42).
        if self.r.changes.conflict.as_ref().is_some_and(|c| c.editing.is_some()) {
            return self.conflict_key(k);
        }
        match k {
            Key::CtrlEnter => {
                self.commit_changes();
                return true;
            }
            Key::Ctrl('z') if !matches!(self.r.changes.focus, CFocus::Subject | CFocus::Desc) => {
                self.undo_discard();
                return true;
            }
            Key::Ctrl('r') => {
                self.recall_message();
                self.changes_dirty();
                return true;
            }
            _ => {}
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
            CFocus::Diff if self.r.changes.conflict.is_some() && self.conflict_key(k) => {}
            CFocus::Diff => match k {
                Key::Up | Key::Char('k') => self.move_line(-1),
                Key::Down | Key::Char('j') => self.move_line(1),
                Key::PageUp => self.move_line(-20),
                Key::PageDown => self.move_line(20),
                Key::Char(' ') => {
                    let l = self.r.changes.line;
                    self.toggle_pick(l);
                    self.move_line(1);
                }
                Key::Char('s') | Key::Enter => self.stage_picked(),
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
                let n = self.side_len(side);
                match k {
                    Key::Up | Key::Char('k') => {
                        if sel == 0 && side == Side::Staged {
                            let m = self.side_len(Side::Unstaged);
                            if m > 0 {
                                self.select_change(Side::Unstaged, m - 1);
                            }
                        } else {
                            self.select_change(side, sel.saturating_sub(1));
                        }
                    }
                    Key::Down | Key::Char('j') => {
                        if sel + 1 >= n && side == Side::Unstaged && self.side_len(Side::Staged) > 0 {
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
                        if side == Side::Unstaged && self.side_len(other) > 0 {
                            self.select_change(other, 0);
                        } else {
                            self.r.changes.focus = CFocus::Subject;
                        }
                    }
                    Key::BackTab => self.r.changes.focus = CFocus::Desc,
                    Key::Delete => self.discard_selected(),
                    Key::Char('i') if self.selected_entry().is_some_and(|e| e.untracked()) => self.ignore_selected(),
                    // A conflicted file: the resolver's keys work from the list too.
                    Key::Char('o' | 't' | 'b' | 'B' | 'e')
                        if self.r.changes.conflict.is_some() && self.selected_entry().is_some_and(|e| e.conflicted) =>
                    {
                        self.r.changes.focus = CFocus::Diff;
                        self.conflict_key(k);
                    }
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
        let n = match self.r.changes.diff.clone() {
            Some(diff) if self.changes_split() => {
                let (p, _) = self.diff_meta(&diff);
                Self::split_rows(&diff, &p).len()
            }
            Some(diff) => diff.lines.len(),
            None => 0,
        };
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
            if self.r.changes.ai.is_some() {
                return self.ai_wheel(n);
            }
            if self.r.changes.conflict.is_some() {
                return self.conflict_wheel(n);
            }
            return self.scroll_change_diff(n);
        }
        for side in [Side::Unstaged, Side::Staged] {
            let i = side_ix(side);
            if l.lists[i].contains(x, y) {
                let len = self.side_len(side);
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
            Some(CHit::Layout) => self.set_changes_split(!self.diff_split),
            Some(CHit::Subject) => self.r.changes.focus = CFocus::Subject,
            Some(CHit::Desc) => self.r.changes.focus = CFocus::Desc,
            Some(CHit::SkipHooks) => self.r.changes.skip_hooks = !self.r.changes.skip_hooks,
            Some(CHit::ListOptions) => {
                let hide = self.hide_untracked();
                let (b, _) = *self
                    .r
                    .changes
                    .hits
                    .iter()
                    .find(|h| h.1 == CHit::ListOptions)
                    .unwrap_or(&(Rect::default(), CHit::ListOptions));
                let shown = self.show_ignored();
                let items = vec![
                    super::menu::MenuItem {
                        label: if hide { "Show untracked files".into() } else { "✓ Show untracked files".into() },
                        act: Some(super::menu::MenuAct::HideUntracked(!hide)),
                        danger: false,
                    },
                    super::menu::MenuItem {
                        label: if shown { "✓ Show ignored files".into() } else { "Show ignored files".into() },
                        act: Some(super::menu::MenuAct::ShowIgnored(!shown)),
                        danger: false,
                    },
                ];
                self.open_menu(b.x, b.bottom(), items);
                return true;
            }
            Some(CHit::PushAfter) => {
                let on = !self.push_after_commit();
                self.settings.set_string("commit.push", if on { "true" } else { "false" });
            }
            Some(CHit::Ai) => self.ai_draft(),
            Some(CHit::AiButton(h)) => self.ai_button(h),
            Some(CHit::StageLines) => self.stage_picked(),
            Some(CHit::Amend) => {
                let on = !self.r.changes.amend;
                self.set_amend(on);
            }
            Some(CHit::Commit) => self.commit_changes(),
            Some(CHit::Conflict(h)) => self.conflict_click(h),
            None if l.diff.contains(x, y) => {
                self.r.changes.focus = CFocus::Diff;
                if let Some(i) = self.changes_diff_line_at(x, y) {
                    self.r.changes.line = i;
                    self.toggle_pick(i);
                }
            }
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
        if l.banner.h > 0 {
            self.paint_op_banner(cv, l.banner, &mut hits);
        }
        cv.fill(Rect::new(l.left.right() - 1, l.left.y, 1, l.left.h), BORDER);
        let loading = self.r.changes.status.is_none();
        let rh = self.change_row_h();
        for side in [Side::Unstaged, Side::Staged] {
            let i = side_ix(side);
            let (head, list) = (l.heads[i], l.lists[i]);
            let n_side = self.side_len(side);
            cv.fill(head, BG_BAR);
            cv.fill(Rect::new(head.x, head.bottom() - 1, head.w, 1), BORDER_SOFT);
            let base = (head.y as f32 + head.h as f32 / 2.0 + self.pf(4.5)).round();
            let title = if side == Side::Unstaged { "Unstaged" } else { "Staged" };
            let e =
                self.te.draw(cv, semi, self.pf(12.5), head.x as f32 + self.pf(14.0), base, title, TEXT, self.pf(120.0));
            let count = if loading { "…".to_string() } else { n_side.to_string() };
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
            if side == Side::Unstaged {
                let ow = self.px(26.0);
                let ob = Rect::new(b.x - self.px(6.0) - ow, b.y, ow, b.h);
                let (bg, border) = self.control_colors(Hot::Change(hits.len()), BG_BAR, BORDER);
                cv.round_border(ob, self.pf(6.0), border, bg);
                let (cx, cy) = (ob.x as f32 + ob.w as f32 / 2.0, ob.y as f32 + ob.h as f32 / 2.0);
                for k in [-1.0f32, 0.0, 1.0] {
                    cv.fill_circle(cx + k * self.pf(4.5), cy, self.pf(1.4), TEXT_2);
                }
                hits.push((ob, CHit::ListOptions));
                let hidden = self.r.changes.hidden_untracked;
                if hidden > 0 {
                    let t = format!("{hidden} untracked hidden");
                    let tw = self.te.measure(ui, self.pf(11.0), &t);
                    self.te.draw(cv, ui, self.pf(11.0), ob.x as f32 - tw - self.pf(10.0), base, &t, TEXT_4, tw + 2.0);
                }
            }
            if n_side > 0 {
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
            if n_side == 0 && !loading {
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
                    TEXT_3,
                    list.w as f32,
                );
            }
            let fit = (list.h / rh.max(1)).max(0) as usize;
            let top = self.r.changes.tops[i].min(n_side.saturating_sub(fit.max(1)));
            let old = cv.set_clip(cv.clip().intersect(list));
            let shown = self.side_rows(side, top, fit + 1);
            for (k, en) in shown.iter().enumerate().map(|(j, e)| (j + top, e)) {
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
                    b'I' => TEXT_4,
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
            let ph = if self.r.changes.amend {
                "Subject (amend keeps the last one)"
            } else {
                "Commit subject · Ctrl+R recent messages"
            };
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
        let hot = self.change_hot(hits.len());
        let ab = self.checkbox(cv, sf.x, by, "Amend", amend, hot);
        hits.push((ab, CHit::Amend));
        let hot = self.change_hot(hits.len());
        let skip = self.r.changes.skip_hooks;
        let hb = self.checkbox(cv, ab.right() + self.px(4.0), by, "Skip hooks", skip, hot);
        hits.push((hb, CHit::SkipHooks));
        // Push after committing ("commit and push").
        let hot = self.change_hot(hits.len());
        let push = self.push_after_commit();
        let hb = self.checkbox(cv, hb.right() + self.px(4.0), by, "Push", push, hot);
        hits.push((hb, CHit::PushAfter));
        // AI draft: a message from the staged changes.
        let hot = self.change_hot(hits.len());
        // The commit button's width first: "AI draft" shortens to "AI"
        // rather than run into it.
        let staged_n = self.side_len(Side::Staged);
        let commit_label = match self.r.changes.busy {
            Some(v) => format!("{v}…"),
            None if amend && push => "Amend and push".to_string(),
            None if amend => "Amend commit".to_string(),
            None if push => "Commit and push".to_string(),
            None if staged_n == 1 => "Commit 1 file".to_string(),
            None => format!("Commit {staged_n} files"),
        };
        let commit_w = self.te.measure(semi, self.pf(12.5), &commit_label)
            + self.te.measure(ui, self.pf(10.5), "Ctrl ↵")
            + self.pf(40.0);
        let commit_x = r.right() as f32 - pad as f32 - commit_w;
        let full = self.te.measure(semi, self.pf(12.0), "AI draft") + self.pf(20.0);
        let label = if hb.right() as f32 + self.pf(6.0) + full + self.pf(8.0) <= commit_x { "AI draft" } else { "AI" };
        let aw = self.te.measure(semi, self.pf(12.0), label) + self.pf(20.0);
        let air = Rect::new(hb.right() + self.px(6.0), by, aw as i32, self.px(30.0));
        let (bg, border) = if hot { (CONTROL_HOVER, ACCENT_BORDER) } else { (BG_BAR, BORDER) };
        cv.round_border(air, self.pf(7.0), border, bg);
        self.te.draw(
            cv,
            semi,
            self.pf(12.0),
            air.x as f32 + self.pf(10.0),
            (air.y as f32 + self.pf(20.0)).round(),
            label,
            ACCENT_FG,
            aw,
        );
        hits.push((air, CHit::Ai));
        let ab = air;
        let staged = self.side_len(Side::Staged);
        let label = match self.r.changes.busy {
            Some(v) => format!("{v}…"),
            None if amend && push => "Amend and push".to_string(),
            None if amend => "Amend commit".to_string(),
            None if push => "Commit and push".to_string(),
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
            if can && hot {
                ACCENT_TEXT
            } else if can {
                TAG_FG
            } else {
                TEXT_4
            },
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

    /// A checkbox with its label; returns its whole hit area.
    fn checkbox(&mut self, cv: &mut Canvas, x: i32, y: i32, label: &str, on: bool, hot: bool) -> Rect {
        let ui = self.f.ui;
        let tw = self.te.measure(ui, self.pf(12.5), label);
        let r = Rect::new(x, y, self.px(34.0) + tw as i32, self.px(30.0));
        if hot {
            cv.fill_round(r, self.pf(6.0), BG_HOVER);
        }
        let bx = Rect::new(r.x + self.px(8.0), r.y + self.px(8.0), self.px(14.0), self.px(14.0));
        if on {
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
        let b = (r.y as f32 + self.pf(20.0)).round();
        self.te.draw(cv, ui, self.pf(12.5), bx.right() as f32 + self.pf(8.0), b, label, TEXT_2, tw + 2.0);
        r
    }

    fn paint_changes_diff(&mut self, cv: &mut Canvas, r: Rect, hits: &mut Vec<(Rect, CHit)>) {
        if self.r.changes.ai.is_some() {
            return self.paint_ai(cv, r, hits);
        }
        if self.selected_entry().is_some_and(|e| e.conflicted) {
            return self.paint_conflict(cv, r, hits);
        }
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
        // Unified or side by side, clickable (FF-35).
        let lt = if self.diff_split { "Unified" } else { "Side by side" };
        let lw = self.te.measure(semi, self.pf(11.5), lt) + self.pf(16.0);
        let layout_btn =
            Rect::new(r.right() - self.px(10.0) - lw as i32, bar.y + self.px(4.0), lw as i32, bar.h - self.px(8.0));
        let hot = Hot::Change(hits.len());
        let (lbg, lborder) = self.control_colors(hot, BG_BAR, BORDER);
        cv.round_border(layout_btn, self.pf(5.0), lborder, lbg);
        self.te.draw(cv, semi, self.pf(11.5), layout_btn.x as f32 + self.pf(8.0), base, lt, TEXT_2, lw);
        hits.push((layout_btn, CHit::Layout));
        let hint = match side {
            Side::Unstaged => "unstaged · Space or click picks lines · S stages them",
            Side::Staged => "staged · Space or click picks lines · S unstages them",
        };
        let hw = self.te.measure(ui, self.pf(11.0), hint);
        let hint_right = layout_btn.x as f32 - self.pf(12.0);
        if hint_right - hw > r.x as f32 + self.pf(260.0) {
            self.te.draw(cv, ui, self.pf(11.0), hint_right - hw, base, hint, TEXT_4, hw + 2.0);
        }
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
        let split = self.changes_split();
        let buttons = if split {
            self.paint_diff_split(cv, body, &diff, top, Some(label), hits.len())
        } else {
            self.paint_diff_body(cv, body, &diff, top, Some(label), hits.len())
        };
        // Picked lines and the line cursor, over the painted lines (side
        // by side: at the left edge of their half).
        let lh = self.code_lh();
        let focus = self.r.changes.focus == CFocus::Diff;
        let picked: Vec<usize> = self.r.changes.picked.iter().copied().collect();
        for i in picked.iter().copied().chain(focus.then_some(self.r.changes.line)) {
            let at = if split {
                self.split_line_rect(&diff, top, body, i).map(|r| (r.x, r.y, r.h))
            } else {
                self.diff_line_y(&diff, top, body, i).map(|(y, h)| (body.x, y, h))
            };
            let Some((x, y, h)) = at else { continue };
            let c = if picked.contains(&i) { ACCENT } else { BORDER_HOVER };
            cv.fill(Rect::new(x, y, self.px(3.0), h), c);
            if picked.contains(&i) && !split {
                cv.fill_circle(x as f32 + self.pf(12.0), y as f32 + lh as f32 / 2.0, self.pf(3.0), ACCENT);
            }
        }
        if !picked.is_empty() {
            let n = picked.len();
            let verb = if side == Side::Unstaged { "Stage" } else { "Unstage" };
            let label = format!("{verb} {n} line{} (S)", if n == 1 { "" } else { "s" });
            let w = self.te.measure(semi, self.pf(11.5), &label) + self.pf(22.0);
            let b = Rect::new(r.right() - self.px(16.0) - w as i32, r.y + self.px(4.0), w as i32, self.px(24.0));
            let hot = Hot::Change(hits.len() + buttons.len());
            // Over the bar's hint text.
            cv.fill(Rect::new(b.x - self.px(10.0), r.y, r.right() - b.x + self.px(10.0), self.px(31.0)), BG_BAR);
            let (bg, border) = if self.is_hot(hot) { (ACCENT, ACCENT) } else { (ACCENT_SOFT_HOVER, ACCENT_BORDER) };
            cv.round_border(b, self.pf(6.0), border, bg);
            let fg = if self.is_hot(hot) { ACCENT_TEXT } else { ACCENT_FG };
            self.te.draw(
                cv,
                semi,
                self.pf(11.5),
                b.x as f32 + self.pf(11.0),
                (b.y as f32 + self.pf(16.5)).round(),
                &label,
                fg,
                w,
            );
            for (h, rect) in &buttons {
                hits.push((*rect, CHit::Hunk(*h)));
            }
            hits.push((b, CHit::StageLines));
            return;
        }
        for (h, rect) in buttons {
            hits.push((rect, CHit::Hunk(h)));
        }
    }
}

/// Whether `e` shows in `side`'s list.
fn in_side(e: &Entry, side: Side) -> bool {
    if side == Side::Staged { e.in_staged() } else { e.in_unstaged() }
}
