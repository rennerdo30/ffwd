//! Interactive rebase (FF-37): the plan for the commits from the chosen
//! one up to HEAD, newest first like the history. Each commit is picked,
//! reworded, squashed, fixed up or dropped, and moved with Shift+K/J or
//! its arrows; Ctrl+Enter runs it (ffwd-core rebase.rs). The plan is read
//! on a worker; nothing changes until the user starts it.

use super::*;
use ffwd_core::rebase::{Action, Step};

pub struct Plan {
    /// The commit the plan rebases onto (`None`: from the root).
    pub base: Option<String>,
    /// Oldest first, as Git takes them.
    pub steps: Vec<Step>,
    /// Selected row, counted from the newest (row 0 is the top).
    pub sel: usize,
    top: usize,
    /// Editing the selected commit's message.
    pub editing: Option<TextField>,
    hits: Vec<(Rect, RHit)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RHit {
    Row(usize),
    Action(usize),
    Up(usize),
    Down(usize),
    Start,
    Cancel,
}

/// Loading, or the plan.
pub enum RebaseState {
    Loading,
    Ready(Box<Plan>),
}

impl Plan {
    fn idx(&self, row: usize) -> usize {
        self.steps.len() - 1 - row
    }

    fn set(&mut self, a: Action) {
        let i = self.idx(self.sel);
        self.steps[i].action = a;
    }

    /// Moves the selected commit one row up (newer) or down (older).
    fn shift(&mut self, newer: bool) {
        let i = self.idx(self.sel);
        let j = if newer { i + 1 } else { i.wrapping_sub(1) };
        if j < self.steps.len() {
            self.steps.swap(i, j);
            self.sel = self.steps.len() - 1 - j;
        }
    }
}

fn cycle(a: &Action, subject: &str) -> Action {
    match a {
        Action::Pick => Action::Reword(subject.to_string()),
        Action::Reword(_) => Action::Squash,
        Action::Squash => Action::Fixup,
        Action::Fixup => Action::Drop,
        Action::Drop => Action::Pick,
    }
}

impl Ui {
    pub(super) fn rebase_area(&self) -> Rect {
        let side = self.lay.side;
        Rect::new(side.right(), self.lay.list.y, self.w as i32 - side.right(), self.lay.status.y - self.lay.list.y)
    }

    /// Opens the plan for the commits from `from` up to HEAD.
    pub(super) fn open_rebase(&mut self, from: String) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        if self.r.refs.head_branch.is_none() {
            return self.set_notice("Check out a branch first: an interactive rebase rewrites a branch.".into());
        }
        let sink = (self.sinks)(t.id);
        self.r.rebase = Some(RebaseState::Loading);
        self.damage.full = true;
        crate::work::spawn("rebase-plan", &sink, move |sink| {
            sink(Msg::RebasePlan(ffwd_core::rebase::plan(&g, &from).map_err(|e| e.to_string())));
        });
    }

    /// Snapshot mode: the plan from history row `row`.
    pub fn snapshot_rebase(&mut self, row: usize) {
        if let Some(c) = self.r.commits.get(row) {
            let sha = c.id.to_string();
            self.open_rebase(sha);
        }
    }

    /// Snapshot mode: a few choices so every kind of row shows.
    pub fn snapshot_rebase_demo(&mut self) {
        if let Some(p) = self.plan_mut() {
            let n = p.steps.len();
            let acts = [Action::Reword(String::new()), Action::Squash, Action::Fixup, Action::Drop];
            for (row, a) in acts.into_iter().enumerate().take(n.saturating_sub(1)) {
                let i = n - 1 - row;
                p.steps[i].action = match a {
                    Action::Reword(_) => Action::Reword(format!("{} (reworded)", p.steps[i].subject)),
                    a => a,
                };
            }
        }
        self.damage.full = true;
    }

    pub(super) fn on_rebase_plan(&mut self, r: Result<(Option<String>, Vec<Step>), String>) {
        if self.r.rebase.is_none() {
            return;
        }
        match r {
            Ok((base, steps)) if !steps.is_empty() => {
                self.r.rebase = Some(RebaseState::Ready(Box::new(Plan {
                    base,
                    steps,
                    sel: 0,
                    top: 0,
                    editing: None,
                    hits: Vec::new(),
                })));
            }
            Ok(_) => {
                self.r.rebase = None;
                self.set_notice("Nothing to rebase there.".into());
            }
            Err(e) => {
                self.r.rebase = None;
                self.set_notice(format!("Cannot rebase from there: {e}"));
            }
        }
        self.damage.full = true;
    }

    pub(super) fn close_rebase(&mut self) {
        self.r.rebase = None;
        self.damage.full = true;
    }

    fn plan_mut(&mut self) -> Option<&mut Plan> {
        match &mut self.r.rebase {
            Some(RebaseState::Ready(p)) => Some(p),
            _ => None,
        }
    }

    fn start_rebase(&mut self) {
        let Some(p) = self.plan_mut() else { return };
        if let Some(why) = ffwd_core::rebase::check(&p.steps) {
            return self.set_notice(why);
        }
        let (base, steps) = (p.base.clone(), p.steps.clone());
        self.close_rebase();
        self.run_refop(crate::work::RefOp::RebaseInteractive(base, steps));
    }

    pub(super) fn rebase_key(&mut self, k: Key) {
        let page = (self.rebase_area().h / self.px(36.0).max(1)).max(1) as usize;
        let Some(p) = self.plan_mut() else {
            if matches!(k, Key::Esc) {
                self.close_rebase();
            }
            return;
        };
        if let Some(f) = &mut p.editing {
            match k {
                Key::Esc => p.editing = None,
                Key::Enter | Key::CtrlEnter => {
                    let m = f.text().trim().to_string();
                    p.editing = None;
                    if !m.is_empty() {
                        p.set(Action::Reword(m));
                    }
                }
                Key::Char(c) => f.insert(c.encode_utf8(&mut [0; 4])),
                Key::Backspace => f.backspace(),
                Key::CtrlBackspace => f.delete_word(),
                Key::Delete => f.delete(),
                Key::Left => f.left(),
                Key::Right => f.right(),
                Key::Home => f.home(),
                Key::End => f.end(),
                Key::Ctrl('a') => f.select_all(),
                Key::Ctrl('v') => {
                    if let Some(t) = crate::platform::clipboard() {
                        f.insert(t.lines().next().unwrap_or(""));
                    }
                }
                _ => {}
            }
            let r = self.rebase_area();
            self.damage.rects.push(r);
            return;
        }
        let n = p.steps.len();
        match k {
            Key::Esc => return self.close_rebase(),
            Key::CtrlEnter => return self.start_rebase(),
            Key::Up => p.sel = p.sel.saturating_sub(1),
            Key::Down => p.sel = (p.sel + 1).min(n - 1),
            Key::PageUp => p.sel = p.sel.saturating_sub(page),
            Key::PageDown => p.sel = (p.sel + page).min(n - 1),
            Key::Home => p.sel = 0,
            Key::End => p.sel = n - 1,
            Key::Char('K') => p.shift(true),
            Key::Char('J') => p.shift(false),
            Key::Char('p') => p.set(Action::Pick),
            Key::Char('s') => p.set(Action::Squash),
            Key::Char('f') => p.set(Action::Fixup),
            Key::Char('d') | Key::Delete => p.set(Action::Drop),
            Key::Char('r') | Key::Enter => {
                let i = p.idx(p.sel);
                let cur = match &p.steps[i].action {
                    Action::Reword(m) => m.clone(),
                    _ => p.steps[i].subject.clone(),
                };
                let mut f = TextField::new(&cur);
                f.end();
                p.editing = Some(f);
            }
            _ => {}
        }
        // Keep the selection on screen.
        let fit = ((self.rebase_area().h - self.px(130.0)) / self.px(36.0).max(1)).max(1) as usize;
        if let Some(p) = self.plan_mut() {
            if p.sel < p.top {
                p.top = p.sel;
            } else if p.sel >= p.top + fit {
                p.top = p.sel + 1 - fit;
            }
        }
        let r = self.rebase_area();
        self.damage.rects.push(r);
    }

    pub(super) fn rebase_click(&mut self, x: i32, y: i32) {
        let hit = match &self.r.rebase {
            Some(RebaseState::Ready(p)) => p.hits.iter().find(|h| h.0.contains(x, y)).map(|h| h.1),
            _ => None,
        };
        let Some(p) = self.plan_mut() else { return };
        p.editing = None;
        match hit {
            Some(RHit::Row(i)) => p.sel = i,
            Some(RHit::Action(i)) => {
                p.sel = i;
                let k = p.idx(i);
                p.steps[k].action = cycle(&p.steps[k].action, &p.steps[k].subject);
            }
            Some(RHit::Up(i)) => {
                p.sel = i;
                p.shift(true);
            }
            Some(RHit::Down(i)) => {
                p.sel = i;
                p.shift(false);
            }
            Some(RHit::Start) => return self.start_rebase(),
            Some(RHit::Cancel) => return self.close_rebase(),
            None => {}
        }
        let r = self.rebase_area();
        self.damage.rects.push(r);
    }

    pub(super) fn rebase_wheel(&mut self, n: i64) {
        let Some(p) = self.plan_mut() else { return };
        p.top = (p.top as i64 + n).clamp(0, p.steps.len().saturating_sub(1) as i64) as usize;
        let r = self.rebase_area();
        self.damage.rects.push(r);
    }

    pub(super) fn rebase_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        match &self.r.rebase {
            Some(RebaseState::Ready(p)) => p.hits.iter().position(|h| h.0.contains(x, y)),
            _ => None,
        }
    }

    pub(super) fn rebase_hit_rect(&self, i: usize) -> Rect {
        match &self.r.rebase {
            Some(RebaseState::Ready(p)) => p.hits.get(i).map_or(self.rebase_area(), |h| h.0.inset(-2, -2)),
            _ => self.rebase_area(),
        }
    }

    pub(super) fn paint_rebase(&mut self, cv: &mut Canvas) {
        let area = self.rebase_area();
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        cv.fill(area, BG());
        let x0 = area.x as f32 + self.pf(24.0);
        let Some(RebaseState::Ready(plan)) = &self.r.rebase else {
            self.te.draw(
                cv,
                ui,
                self.pf(13.0),
                x0,
                area.y as f32 + self.pf(34.0),
                "Reading the commits…",
                TEXT_3(),
                area.w as f32,
            );
            return;
        };
        let n = plan.steps.len();
        let base = plan.base.as_deref().map_or("the root".to_string(), |b| b[..b.len().min(8)].to_string());
        let branch = self.r.refs.head_branch.as_deref().unwrap_or("HEAD").to_string();
        let title = format!("Interactive rebase of {branch}: {n} commit{} onto {base}", if n == 1 { "" } else { "s" });
        self.te.draw(
            cv,
            semi,
            self.pf(15.5),
            x0,
            area.y as f32 + self.pf(32.0),
            &title,
            TEXT(),
            area.w as f32 - self.pf(48.0),
        );
        let hint = "p pick · r reword · s squash · f fixup · d drop · Shift+K/J move · Ctrl+Enter start · Esc cancel";
        self.te.draw(
            cv,
            ui,
            self.pf(12.0),
            x0,
            area.y as f32 + self.pf(54.0),
            hint,
            TEXT_3(),
            area.w as f32 - self.pf(48.0),
        );
        let list =
            Rect::new(area.x + self.px(16.0), area.y + self.px(72.0), area.w - self.px(32.0), area.h - self.px(132.0));
        let rh = self.px(36.0);
        let fit = (list.h / rh.max(1)) as usize + 1;
        let mut hits: Vec<(Rect, RHit)> = Vec::new();
        let steps = plan.steps.clone();
        let (sel, top) = (plan.sel, plan.top.min(n.saturating_sub(1)));
        let editing = plan.editing.clone();
        let old = cv.set_clip(cv.clip().intersect(list));
        for row in top..(top + fit).min(n) {
            let s = &steps[n - 1 - row];
            let y = list.y + (row - top) as i32 * rh;
            let r = Rect::new(list.x, y + self.px(2.0), list.w, rh - self.px(4.0));
            let hot = |ui: &Self, k: usize| ui.is_hot(Hot::Rebase(k));
            if row == sel {
                cv.fill_round(r, self.pf(7.0), BG_SELECT());
            } else if hot(self, hits.len()) {
                cv.fill_round(r, self.pf(7.0), BG_HOVER());
            }
            hits.push((r, RHit::Row(row)));
            let b = (r.y as f32 + r.h as f32 / 2.0 + self.pf(4.5)).round();
            // The action as a pill (click cycles it).
            let (label, fg, bg) = match &s.action {
                Action::Pick => ("pick", TEXT_2(), BG_BAR()),
                Action::Reword(_) => ("reword", ACCENT_TEXT(), ACCENT()),
                Action::Squash => ("squash", TAG_FG(), TAG_BG()),
                Action::Fixup => ("fixup", TAG_FG(), TAG_BG()),
                Action::Drop => ("drop", DEL_TEXT(), DEL_BG()),
            };
            let pill = Rect::new(r.x + self.px(10.0), r.y + self.px(6.0), self.px(68.0), r.h - self.px(12.0));
            let pbg = if hot(self, hits.len()) { CONTROL_HOVER() } else { bg };
            cv.fill_round(pill, self.pf(6.0), pbg);
            let lw = self.te.measure(semi, self.pf(11.5), label);
            self.te.draw(
                cv,
                semi,
                self.pf(11.5),
                pill.x as f32 + (pill.w as f32 - lw) / 2.0,
                b,
                label,
                fg,
                pill.w as f32,
            );
            hits.push((pill, RHit::Action(row)));
            let sx = pill.right() as f32 + self.pf(14.0);
            self.te.draw(cv, mono, self.pf(11.5), sx, b, &s.sha[..s.sha.len().min(8)], TEXT_3(), self.pf(70.0));
            let tx = sx + self.pf(78.0);
            let right = r.right() as f32 - self.pf(70.0);
            let dropped = s.action == Action::Drop;
            if row == sel
                && let Some(f) = &editing
            {
                let fr =
                    Rect::new(tx as i32 - self.px(6.0), r.y + self.px(4.0), (right - tx) as i32, r.h - self.px(8.0));
                cv.round_border(fr, self.pf(6.0), ACCENT(), BG_INPUT());
                self.te.draw(cv, ui, self.pf(13.0), tx, b, f.text(), TEXT(), right - tx - self.pf(8.0));
                let cx = tx + self.te.measure(ui, self.pf(13.0), &f.text()[..f.cursor()]);
                cv.fill(
                    Rect::new(cx as i32 + 1, fr.y + self.px(6.0), self.px(1.5).max(1), fr.h - self.px(12.0)),
                    ACCENT(),
                );
            } else {
                let (text, color) = match &s.action {
                    Action::Reword(m) => (m.clone(), TEXT()),
                    _ if dropped => (s.subject.clone(), TEXT_4()),
                    _ => (s.subject.clone(), TEXT()),
                };
                let e = self.te.draw(cv, ui, self.pf(13.0), tx, b, &text, color, right - tx - self.pf(8.0));
                if dropped {
                    cv.fill(Rect::new(tx as i32, (b - self.pf(4.5)) as i32, (e - tx) as i32, 1), TEXT_4());
                }
                let note = match &s.action {
                    Action::Squash => Some("↓ into the commit below, both messages"),
                    Action::Fixup => Some("↓ into the commit below, its message"),
                    Action::Reword(_) => Some("new message"),
                    _ => None,
                };
                if let Some(nt) = note {
                    let nx = (e + self.pf(14.0)).min(right - self.pf(10.0));
                    self.te.draw(cv, ui, self.pf(11.0), nx, b, nt, TEXT_4(), right - nx);
                }
            }
            // Move arrows on the selected row.
            if row == sel {
                for (k, up) in [(0, true), (1, false)] {
                    let ar = Rect::new(
                        r.right() - self.px(64.0) + k * self.px(28.0),
                        r.y + self.px(6.0),
                        self.px(24.0),
                        r.h - self.px(12.0),
                    );
                    let (bg, _) = self.control_colors(Hot::Rebase(hits.len()), BG_BAR(), BORDER());
                    cv.fill_round(ar, self.pf(5.0), bg);
                    let (cx, cy, s) = (ar.x as f32 + ar.w as f32 / 2.0, ar.y as f32 + ar.h as f32 / 2.0, self.pf(4.0));
                    let pts = if up {
                        [(cx - s, cy + s / 2.0), (cx, cy - s / 2.0), (cx + s, cy + s / 2.0)]
                    } else {
                        [(cx - s, cy - s / 2.0), (cx, cy + s / 2.0), (cx + s, cy - s / 2.0)]
                    };
                    cv.stroke(&pts, self.pf(1.5), TEXT_2());
                    hits.push((ar, if up { RHit::Up(row) } else { RHit::Down(row) }));
                }
            }
        }
        cv.set_clip(old);
        // Footer: what is wrong (if anything), Cancel and Start.
        let fy = area.bottom() - self.px(52.0);
        cv.fill(Rect::new(area.x, fy - self.px(8.0), area.w, 1), BORDER_SOFT());
        let problem = ffwd_core::rebase::check(&steps);
        let bh = self.px(32.0);
        let mut bx = area.right() - self.px(24.0);
        for (label, hit, primary) in [("Start rebase", RHit::Start, true), ("Cancel", RHit::Cancel, false)] {
            let w = (self.te.measure(semi, self.pf(12.5), label) + self.pf(28.0)) as i32;
            let r = Rect::new(bx - w, fy, w, bh);
            let h = Hot::Rebase(hits.len());
            if primary {
                let enabled = problem.is_none();
                let bg = if !enabled {
                    BG_BAR()
                } else if self.is_hot(h) {
                    ACCENT_FG()
                } else {
                    ACCENT()
                };
                cv.fill_round(r, self.pf(7.0), bg);
                let c = if enabled { ACCENT_TEXT() } else { TEXT_4() };
                self.te.draw(
                    cv,
                    semi,
                    self.pf(12.5),
                    r.x as f32 + self.pf(14.0),
                    (fy as f32 + self.pf(21.0)).round(),
                    label,
                    c,
                    w as f32,
                );
            } else {
                let (bg, border) = self.control_colors(h, BG_BAR(), BORDER());
                cv.round_border(r, self.pf(7.0), border, bg);
                self.te.draw(
                    cv,
                    semi,
                    self.pf(12.5),
                    r.x as f32 + self.pf(14.0),
                    (fy as f32 + self.pf(21.0)).round(),
                    label,
                    TEXT(),
                    w as f32,
                );
            }
            hits.push((r, hit));
            bx = r.x - self.px(8.0);
        }
        let msg = problem.unwrap_or_else(|| {
            "ffwd keeps the current tip as a backup ref; conflicts stop the rebase in Local Changes.".to_string()
        });
        let c = if ffwd_core::rebase::check(&steps).is_some() { DEL() } else { TEXT_3() };
        self.te.draw(
            cv,
            ui,
            self.pf(12.0),
            x0,
            (fy as f32 + self.pf(21.0)).round(),
            &msg,
            c,
            (bx - area.x) as f32 - self.pf(40.0),
        );
        if let Some(RebaseState::Ready(p)) = &mut self.r.rebase {
            p.hits = hits;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(n: usize) -> Plan {
        let steps =
            (0..n).map(|i| Step { sha: format!("{i:040}"), subject: format!("c{i}"), action: Action::Pick }).collect();
        Plan { base: None, steps, sel: 0, top: 0, editing: None, hits: Vec::new() }
    }

    #[test]
    fn rows_are_newest_first_and_moves_follow_the_selection() {
        let mut p = plan(3); // steps oldest first: c0 c1 c2; rows: c2 c1 c0
        assert_eq!(p.steps[p.idx(0)].subject, "c2");
        p.sel = 2; // c0
        p.shift(true);
        assert_eq!(p.steps.iter().map(|s| s.subject.as_str()).collect::<Vec<_>>(), ["c1", "c0", "c2"]);
        assert_eq!(p.steps[p.idx(p.sel)].subject, "c0", "the selection moves with the commit");
        p.sel = 0;
        p.shift(true); // the newest cannot go higher
        assert_eq!(p.steps[2].subject, "c2");
        assert_eq!(cycle(&Action::Pick, "s"), Action::Reword("s".into()));
        assert_eq!(cycle(&Action::Drop, "s"), Action::Pick);
    }
}
