//! The activity log (FF-48): every operation the user started (fetch,
//! pull, push, branch and history operations, stashes, remotes, Local
//! Changes actions) with its result and the exact git commands it ran,
//! their exit codes, times and output. App-wide and capped; it lives in
//! memory only, so nothing about the user's repositories is written out.

use super::*;
use ffwd_core::git::Ran;
use std::collections::VecDeque;

/// Entries kept (the oldest go first).
pub const MAX_ENTRIES: usize = 500;

#[derive(Clone, Debug)]
pub struct Entry {
    /// Unix seconds.
    pub at: i64,
    pub repo: String,
    pub title: String,
    pub ok: bool,
    pub summary: String,
    pub ms: u32,
    pub ran: Vec<Ran>,
}

impl Entry {
    pub fn new(git: &Git, title: impl Into<String>, ok: bool, summary: String, ms: u32, ran: Vec<Ran>) -> Entry {
        let repo = git.root.file_name().map_or_else(|| git.root.display().to_string(), |n| n.to_string_lossy().into());
        Entry { at: now_secs(), repo, title: title.into(), ok, summary, ms, ran }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AHit {
    Row(usize),
}

#[derive(Default)]
pub struct Activity {
    pub entries: VecDeque<Entry>,
    /// Failures since the view was last open (the sidebar badge).
    pub unseen_failures: u32,
    /// Selected entry, counted from the newest (0 = newest).
    pub sel: usize,
    top: usize,
    /// First visible line of the detail pane.
    dtop: usize,
    /// The detail lines of the selected entry: (kind, text).
    lines: Vec<(LineKind, String)>,
    hits: Vec<(Rect, AHit)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineKind {
    Command,
    Meta,
    Output,
    Error,
    Blank,
}

impl Activity {
    pub fn push(&mut self, e: Entry, viewing: bool) {
        if !e.ok && !viewing {
            self.unseen_failures += 1;
        }
        self.entries.push_back(e);
        while self.entries.len() > MAX_ENTRIES {
            self.entries.pop_front();
        }
        // Keep the selection on the same entry while new ones arrive,
        // except when the newest was selected.
        if self.sel > 0 {
            self.sel = (self.sel + 1).min(self.entries.len().saturating_sub(1));
        }
        self.rebuild();
    }

    /// The entry `i` places from the newest.
    pub fn get(&self, i: usize) -> Option<&Entry> {
        self.entries.len().checked_sub(i + 1).and_then(|k| self.entries.get(k))
    }

    fn rebuild(&mut self) {
        self.lines.clear();
        self.dtop = 0;
        let Some(e) = self.get(self.sel).cloned() else { return };
        for r in &e.ran {
            self.lines.push((LineKind::Command, format!("$ git {}", r.args)));
            let code = r.code.map_or("failed or ended early".to_string(), |c| format!("exit {c}"));
            self.lines.push((LineKind::Meta, format!("{code} · {} ms", r.ms)));
            let bad = r.code != Some(0);
            for l in r.stderr.split(['\n', '\r']).map(str::trim_end).filter(|l| !l.trim().is_empty()) {
                let kind = if bad || l.starts_with("fatal:") || l.starts_with("error:") {
                    LineKind::Error
                } else {
                    LineKind::Output
                };
                self.lines.push((kind, l.chars().take(400).collect()));
            }
            self.lines.push((LineKind::Blank, String::new()));
        }
        if e.ran.is_empty() {
            self.lines.push((LineKind::Meta, "No git command ran.".into()));
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ALayout {
    pub area: Rect,
    pub list: Rect,
    pub head: Rect,
    pub detail: Rect,
}

impl Ui {
    pub(super) fn activity_layout(&self) -> ALayout {
        let side = self.lay.side;
        let area =
            Rect::new(side.right(), self.lay.list.y, self.w as i32 - side.right(), self.lay.status.y - self.lay.list.y);
        let lw = ((area.w as f32 * 0.38) as i32).clamp(self.px(300.0), self.px(560.0)).min(area.w / 2);
        let hh = self.px(64.0);
        ALayout {
            area,
            list: Rect::new(area.x, area.y, lw, area.h),
            head: Rect::new(area.x + lw, area.y, area.w - lw, hh),
            detail: Rect::new(area.x + lw, area.y + hh, area.w - lw, area.h - hh),
        }
    }

    pub fn enter_activity(&mut self) {
        if !self.r.activity_view {
            self.r.activity_view = true;
            self.r.changes_view = false;
            self.r.files_view = false;
            self.r.reflog_view = false;
            self.focus = Focus::List;
        }
        self.activity.unseen_failures = 0;
        self.rebuild_side();
        self.activity.sel = 0;
        self.activity.top = 0;
        self.activity.rebuild();
        self.side_select("activity");
        self.damage.full = true;
    }

    /// Snapshot mode: two operations that fail and change nothing.
    pub fn snapshot_activity(&mut self) {
        self.run_refop(crate::work::RefOp::Checkout("ffwd-snapshot-no-such-branch".into()));
        self.run_refop(crate::work::RefOp::DeleteTag("ffwd-snapshot-no-such-tag".into()));
    }

    pub(super) fn leave_activity(&mut self) {
        if self.r.activity_view {
            self.r.activity_view = false;
            self.damage.full = true;
        }
    }

    pub(super) fn on_activity(&mut self, e: Entry) {
        let viewing = self.r.activity_view;
        let had_badge = self.activity.unseen_failures;
        self.activity.push(e, viewing);
        if viewing {
            let a = self.activity_layout().area;
            self.damage.rects.push(a);
        } else if self.activity.unseen_failures != had_badge {
            self.rebuild_side();
            self.damage.rects.push(self.lay.side);
        }
    }

    fn activity_select(&mut self, i: usize) {
        let fit = (self.activity_layout().list.h / self.px(52.0).max(1)).max(1) as usize;
        let a = &mut self.activity;
        if a.entries.is_empty() {
            return;
        }
        a.sel = i.min(a.entries.len() - 1);
        if a.sel < a.top {
            a.top = a.sel;
        } else if a.sel >= a.top + fit {
            a.top = a.sel + 1 - fit;
        }
        a.rebuild();
        let r = self.activity_layout().area;
        self.damage.rects.push(r);
    }

    pub(super) fn activity_key(&mut self, k: Key) -> bool {
        let fit = (self.activity_layout().list.h / self.px(52.0).max(1)).max(1) as usize;
        let page = (self.activity_layout().detail.h / self.px(19.0).max(1)).max(1) as usize;
        let sel = self.activity.sel;
        match k {
            Key::Up => self.activity_select(sel.saturating_sub(1)),
            Key::Down => self.activity_select(sel + 1),
            Key::PageUp => self.activity_select(sel.saturating_sub(fit)),
            Key::PageDown => self.activity_select(sel + fit),
            Key::Home => self.activity_select(0),
            Key::End => self.activity_select(usize::MAX),
            Key::Char('j') => self.activity_scroll(1),
            Key::Char('k') => self.activity_scroll(-1),
            Key::Char(' ') => self.activity_scroll(page as i64),
            Key::Ctrl('c') => {
                if let Some(e) = self.activity.get(sel) {
                    let mut t = format!("{} · {} · {}\n", e.title, e.repo, e.summary);
                    for (_, l) in &self.activity.lines {
                        t.push_str(l);
                        t.push('\n');
                    }
                    crate::platform::set_clipboard(&t);
                    self.set_notice("Copied the entry with its commands and output".into());
                }
            }
            Key::Esc => {
                self.leave_activity();
                self.side_select("all");
            }
            _ => return false,
        }
        true
    }

    fn activity_scroll(&mut self, n: i64) {
        let a = &mut self.activity;
        a.dtop = (a.dtop as i64 + n).clamp(0, a.lines.len().saturating_sub(1) as i64) as usize;
        let r = self.activity_layout().detail;
        self.damage.rects.push(r);
    }

    pub(super) fn activity_click(&mut self, x: i32, y: i32) -> bool {
        if !self.activity_layout().area.contains(x, y) {
            return false;
        }
        if let Some(AHit::Row(i)) = self.activity.hits.iter().find(|h| h.0.contains(x, y)).map(|h| h.1) {
            self.activity_select(i);
        }
        true
    }

    pub(super) fn activity_wheel(&mut self, x: i32, y: i32, n: i64) {
        let l = self.activity_layout();
        if l.list.contains(x, y) {
            let a = &mut self.activity;
            a.top = (a.top as i64 + n).clamp(0, a.entries.len().saturating_sub(1) as i64) as usize;
            self.damage.rects.push(l.list);
        } else {
            self.activity_scroll(n);
        }
    }

    pub(super) fn activity_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.r.activity_view {
            return None;
        }
        self.activity.hits.iter().position(|h| h.0.contains(x, y))
    }

    pub(super) fn activity_hit_rect(&self, i: usize) -> Rect {
        self.activity.hits.get(i).map_or(self.activity_layout().list, |h| h.0)
    }

    pub(super) fn paint_activity(&mut self, cv: &mut Canvas) {
        let l = self.activity_layout();
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let mut hits = Vec::new();
        cv.fill(l.list, BG_SIDEBAR());
        cv.fill(Rect::new(l.list.right() - 1, l.area.y, 1, l.area.h), BORDER());
        if self.activity.entries.is_empty() {
            let t = "Nothing has run yet. Fetch, pull, push, branch, stash and commit actions appear here with the exact git commands.";
            let x = l.list.x as f32 + self.pf(18.0);
            let mut y = l.list.y as f32 + self.pf(30.0);
            let size = self.pf(12.5);
            let maxw = l.list.w as f32 - self.pf(36.0);
            for line in wrap(&mut self.te, ui, size, t, maxw) {
                self.te.draw(cv, ui, size, x, y, &line, TEXT_3(), l.list.w as f32);
                y += self.pf(19.0);
            }
        }
        let rh = self.px(52.0);
        let fit = (l.list.h / rh.max(1)) as usize + 1;
        let n = self.activity.entries.len();
        let top = self.activity.top.min(n.saturating_sub(1));
        let old = cv.set_clip(cv.clip().intersect(l.list));
        for k in 0..fit {
            let i = top + k;
            let Some(e) = self.activity.get(i).cloned() else { break };
            let y = l.list.y + k as i32 * rh;
            let row = Rect::new(l.list.x + self.px(6.0), y + self.px(3.0), l.list.w - self.px(12.0), rh - self.px(6.0));
            if i == self.activity.sel {
                cv.fill_round(row, self.pf(7.0), BG_SELECT());
            } else if self.is_hot(Hot::Activity(hits.len())) {
                cv.fill_round(row, self.pf(7.0), BG_HOVER());
            }
            let cx = row.x as f32 + self.pf(16.0);
            let cy = row.y as f32 + self.pf(16.0);
            if e.ok {
                self.check(cv, cx, cy, ADD());
            } else {
                let s = self.pf(4.0);
                cv.stroke(&[(cx - s, cy - s), (cx + s, cy + s)], self.pf(1.6), DEL());
                cv.stroke(&[(cx - s, cy + s), (cx + s, cy - s)], self.pf(1.6), DEL());
            }
            let tx = cx + self.pf(16.0);
            let right = row.right() as f32 - self.pf(10.0);
            let when = clock(e.at, self.r.tz);
            let meta = format!("{when} · {}", fmt_ms(e.ms));
            let mw = self.te.measure(ui, self.pf(11.0), &meta);
            let b1 = (row.y as f32 + self.pf(20.0)).round();
            self.te.draw(cv, ui, self.pf(11.0), right - mw, b1, &meta, TEXT_4(), mw + 2.0);
            let title = format!("{} · {}", e.title, e.repo);
            self.te.draw(cv, semi, self.pf(12.5), tx, b1, &title, TEXT(), right - mw - tx - self.pf(10.0));
            let b2 = (row.y as f32 + self.pf(38.0)).round();
            let c = if e.ok { TEXT_3() } else { DEL() };
            self.te.draw(cv, ui, self.pf(11.5), tx, b2, &e.summary, c, right - tx);
            hits.push((row, AHit::Row(i)));
        }
        cv.set_clip(old);
        // The selected entry.
        cv.fill(l.head, BG_BAR());
        cv.fill(Rect::new(l.head.x, l.head.bottom() - 1, l.head.w, 1), BORDER_SOFT());
        cv.fill(l.detail, BG());
        if let Some(e) = self.activity.get(self.activity.sel).cloned() {
            let x = l.head.x as f32 + self.pf(18.0);
            let w = l.head.w as f32 - self.pf(36.0);
            let t = format!("{} · {}", e.title, e.repo);
            self.te.draw(cv, semi, self.pf(14.0), x, l.head.y as f32 + self.pf(26.0), &t, TEXT(), w);
            let c = if e.ok { TEXT_2() } else { DEL() };
            let s = format!("{} · {} · {}", if e.ok { "Done" } else { "Failed" }, clock(e.at, self.r.tz), e.summary);
            self.te.draw(cv, ui, self.pf(12.0), x, l.head.y as f32 + self.pf(47.0), &s, c, w);
            let lh = self.px(19.0);
            let fit = (l.detail.h / lh) as usize + 1;
            let old = cv.set_clip(cv.clip().intersect(l.detail));
            let lines: Vec<(LineKind, String)> =
                self.activity.lines.iter().skip(self.activity.dtop).take(fit).cloned().collect();
            for (k, (kind, text)) in lines.into_iter().enumerate() {
                let b = (l.detail.y as f32 + self.pf(16.0) + k as f32 * lh as f32).round() + self.pf(4.0);
                let (font, color) = match kind {
                    LineKind::Command => (mono, ACCENT()),
                    LineKind::Meta => (ui, TEXT_4()),
                    LineKind::Output => (mono, TEXT_2()),
                    LineKind::Error => (mono, DEL()),
                    LineKind::Blank => continue,
                };
                self.te.draw(cv, font, self.pf(11.5), x, b, &text, color, w);
            }
            cv.set_clip(old);
            let hint = "↑↓ entries · j k space scroll · Ctrl+C copies · Esc back";
            let hw = self.te.measure(ui, self.pf(11.0), hint);
            let hx = l.detail.right() as f32 - hw - self.pf(16.0);
            self.te.draw(cv, ui, self.pf(11.0), hx, l.detail.bottom() as f32 - self.pf(10.0), hint, TEXT_4(), hw + 2.0);
        }
        self.activity.hits = hits;
    }
}

/// `HH:MM:SS` in the repository's time zone offset.
fn clock(t: i64, tz: i64) -> String {
    let s = (t + tz).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn fmt_ms(ms: u32) -> String {
    if ms < 1000 { format!("{ms} ms") } else { format!("{:.1} s", ms as f64 / 1000.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(ok: bool) -> Entry {
        Entry { at: 0, repo: "r".into(), title: "T".into(), ok, summary: String::new(), ms: 0, ran: Vec::new() }
    }

    #[test]
    fn capped_newest_first_and_counts_unseen_failures() {
        let mut a = Activity::default();
        for i in 0..MAX_ENTRIES + 20 {
            a.push(e(i % 2 == 0), false);
        }
        assert_eq!(a.entries.len(), MAX_ENTRIES);
        assert_eq!(a.unseen_failures as usize, (MAX_ENTRIES + 20) / 2);
        a.push(e(false), true);
        assert_eq!(a.unseen_failures as usize, (MAX_ENTRIES + 20) / 2, "failures seen in the view do not count");
        assert!(!a.get(0).unwrap().ok, "0 is the newest");
    }

    #[test]
    fn clock_and_durations() {
        assert_eq!(clock(3_725, 0), "01:02:05");
        assert_eq!(clock(0, -3600), "23:00:00");
        assert_eq!(fmt_ms(250), "250 ms");
        assert_eq!(fmt_ms(1500), "1.5 s");
    }
}
