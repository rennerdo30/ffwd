//! The reflog view (FF-43): every place HEAD has been, newest first,
//! with what moved it. Entries whose commit is on no loaded branch are
//! marked: those are the lost commits a branch here brings back. Read on a
//! worker; only visible rows paint.

use super::*;
use ffwd_core::reflog::Entry;

#[derive(Default)]
pub struct Reflog {
    pub entries: Option<Result<Vec<Entry>, String>>,
    pub sel: usize,
    top: usize,
    hits: Vec<(Rect, usize)>,
}

impl Ui {
    pub(super) fn reflog_area(&self) -> Rect {
        let side = self.lay.side;
        Rect::new(side.right(), self.lay.list.y, self.w as i32 - side.right(), self.lay.status.y - self.lay.list.y)
    }

    pub fn enter_reflog(&mut self) {
        if !self.r.reflog_view {
            self.r.reflog_view = true;
            self.r.changes_view = false;
            self.r.files_view = false;
            self.r.activity_view = false;
            self.focus = Focus::List;
        }
        self.side_select("reflog");
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(t.id);
        crate::work::spawn("reflog", &sink, move |sink| sink(Msg::Reflog(ffwd_core::reflog::read(&g))));
        self.damage.full = true;
    }

    pub(super) fn leave_reflog(&mut self) {
        if self.r.reflog_view {
            self.r.reflog_view = false;
            self.damage.full = true;
        }
    }

    pub(super) fn on_reflog(&mut self, r: Result<Vec<Entry>, String>) {
        let n = r.as_ref().map_or(0, |v| v.len());
        let l = &mut self.r.reflog;
        l.entries = Some(r);
        l.sel = l.sel.min(n.saturating_sub(1));
        let a = self.reflog_area();
        self.damage.rects.push(a);
    }

    fn reflog_rows(&self) -> usize {
        (self.reflog_area().h - self.px(44.0)).max(1) as usize / self.px(30.0).max(1) as usize
    }

    fn reflog_select(&mut self, i: usize) {
        let fit = self.reflog_rows().max(1);
        let n = self.r.reflog.entries.as_ref().and_then(|r| r.as_ref().ok()).map_or(0, |v| v.len());
        let l = &mut self.r.reflog;
        if n == 0 {
            return;
        }
        l.sel = i.min(n - 1);
        if l.sel < l.top {
            l.top = l.sel;
        } else if l.sel >= l.top + fit {
            l.top = l.sel + 1 - fit;
        }
        let a = self.reflog_area();
        self.damage.rects.push(a);
    }

    fn reflog_entry(&self, i: usize) -> Option<Entry> {
        self.r.reflog.entries.as_ref()?.as_ref().ok()?.get(i).cloned()
    }

    /// The menu of reflog entry `i`.
    pub(super) fn reflog_menu(&self, i: usize) -> Vec<menu::MenuItem> {
        let Some(e) = self.reflog_entry(i) else { return Vec::new() };
        menu::reflog_items(self, &e.id.to_string(), self.r.by_id.contains_key(&e.id))
    }

    pub(super) fn reflog_key(&mut self, k: Key) -> bool {
        let fit = self.reflog_rows().max(1);
        let sel = self.r.reflog.sel;
        match k {
            Key::Up => self.reflog_select(sel.saturating_sub(1)),
            Key::Down => self.reflog_select(sel + 1),
            Key::PageUp => self.reflog_select(sel.saturating_sub(fit)),
            Key::PageDown => self.reflog_select(sel + fit),
            Key::Home => self.reflog_select(0),
            Key::End => self.reflog_select(usize::MAX),
            Key::Enter | Key::Ctrl('.') => {
                let items = self.reflog_menu(sel);
                let a = self.reflog_area();
                let y = a.y + self.px(44.0) + (sel.saturating_sub(self.r.reflog.top) as i32 + 1) * self.px(30.0);
                self.open_menu(a.x + self.px(120.0), y, items);
            }
            Key::Esc => {
                self.leave_reflog();
                self.side_select("all");
            }
            _ => return false,
        }
        true
    }

    pub(super) fn reflog_click(&mut self, x: i32, y: i32) -> bool {
        if !self.reflog_area().contains(x, y) {
            return false;
        }
        if let Some(&(_, i)) = self.r.reflog.hits.iter().find(|h| h.0.contains(x, y)) {
            self.reflog_select(i);
        }
        true
    }

    /// Right click on an entry: its menu.
    pub(super) fn reflog_right_click(&mut self, x: i32, y: i32) -> bool {
        let Some(&(_, i)) = self.r.reflog.hits.iter().find(|h| h.0.contains(x, y)) else { return false };
        self.reflog_select(i);
        let items = self.reflog_menu(i);
        self.open_menu(x, y, items);
        true
    }

    pub(super) fn reflog_wheel(&mut self, n: i64) {
        let total = self.r.reflog.entries.as_ref().and_then(|r| r.as_ref().ok()).map_or(0, |v| v.len());
        let l = &mut self.r.reflog;
        l.top = (l.top as i64 + n).clamp(0, total.saturating_sub(1) as i64) as usize;
        let a = self.reflog_area();
        self.damage.rects.push(a);
    }

    pub(super) fn reflog_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.r.reflog.hits.iter().position(|h| h.0.contains(x, y))
    }

    pub(super) fn reflog_hit_rect(&self, i: usize) -> Rect {
        self.r.reflog.hits.get(i).map_or(self.reflog_area(), |h| h.0)
    }

    pub(super) fn paint_reflog(&mut self, cv: &mut Canvas) {
        let area = self.reflog_area();
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        cv.fill(area, BG());
        let head = Rect::new(area.x, area.y, area.w, self.px(44.0));
        cv.fill(Rect::new(head.x, head.bottom() - 1, head.w, 1), BORDER_SOFT());
        let x0 = area.x as f32 + self.pf(18.0);
        let b = (head.y as f32 + self.pf(27.0)).round();
        let e = self.te.draw(cv, semi, self.pf(14.0), x0, b, "Reflog", TEXT(), self.pf(200.0));
        let hint = "Where HEAD has been. Commits marked \u{2018}lost\u{2019} are on no branch: a branch here brings one back. Enter or right click for actions.";
        self.te.draw(
            cv,
            ui,
            self.pf(12.0),
            e + self.pf(14.0),
            b,
            hint,
            TEXT_3(),
            area.right() as f32 - e - self.pf(40.0),
        );
        let mut hits = Vec::new();
        let list = Rect::new(area.x, head.bottom(), area.w, area.h - head.h);
        match self.r.reflog.entries.clone() {
            None => {
                self.te.draw(
                    cv,
                    ui,
                    self.pf(12.5),
                    x0,
                    list.y as f32 + self.pf(26.0),
                    "Reading the reflog…",
                    TEXT_3(),
                    list.w as f32,
                );
            }
            Some(Err(e)) => {
                self.te.draw(cv, ui, self.pf(12.5), x0, list.y as f32 + self.pf(26.0), &e, DEL(), list.w as f32);
            }
            Some(Ok(entries)) => {
                if entries.is_empty() {
                    let t = "The reflog is empty.";
                    self.te.draw(cv, ui, self.pf(12.5), x0, list.y as f32 + self.pf(26.0), t, TEXT_3(), list.w as f32);
                }
                let rh = self.px(30.0);
                let fit = (list.h / rh.max(1)) as usize + 1;
                let top = self.r.reflog.top.min(entries.len().saturating_sub(1));
                let old = cv.set_clip(cv.clip().intersect(list));
                for (k, en) in entries.iter().enumerate().skip(top).take(fit) {
                    let y = list.y + (k - top) as i32 * rh;
                    let row =
                        Rect::new(list.x + self.px(8.0), y + self.px(2.0), list.w - self.px(16.0), rh - self.px(4.0));
                    if k == self.r.reflog.sel {
                        cv.fill_round(row, self.pf(6.0), BG_SELECT());
                    } else if self.is_hot(Hot::Reflog(hits.len())) {
                        cv.fill_round(row, self.pf(6.0), BG_HOVER());
                    }
                    let rb = (row.y as f32 + row.h as f32 / 2.0 + self.pf(4.5)).round();
                    let rx = row.x as f32 + self.pf(10.0);
                    self.te.draw(cv, mono, self.pf(11.5), rx, rb, &en.selector, TEXT_4(), self.pf(90.0));
                    self.te.draw(
                        cv,
                        mono,
                        self.pf(11.5),
                        rx + self.pf(96.0),
                        rb,
                        &en.id.short(8),
                        TEXT_3(),
                        self.pf(70.0),
                    );
                    let lost = !self.r.by_id.contains_key(&en.id) && self.r.log_done;
                    let mut tx = rx + self.pf(172.0);
                    if lost {
                        let lw = self.te.measure(semi, self.pf(10.5), "lost") + self.pf(12.0);
                        let chip = Rect::new(tx as i32, (rb - self.pf(13.5)) as i32, lw as i32, self.px(18.0));
                        cv.fill_round(chip, self.pf(5.0), DEL_BG());
                        self.te.draw(
                            cv,
                            semi,
                            self.pf(10.5),
                            tx + self.pf(6.0),
                            rb - self.pf(1.0),
                            "lost",
                            DEL_TEXT(),
                            lw,
                        );
                        tx += lw + self.pf(8.0);
                    }
                    let when = fmt::relative(en.time, self.now, self.r.tz);
                    let ww = self.te.measure(ui, self.pf(12.0), &when);
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(12.0),
                        row.right() as f32 - ww - self.pf(10.0),
                        rb,
                        &when,
                        TEXT_3(),
                        ww + 2.0,
                    );
                    let right = row.right() as f32 - ww - self.pf(24.0);
                    self.te.draw(cv, ui, self.pf(13.0), tx, rb, &en.message, TEXT(), right - tx);
                    hits.push((row, k));
                }
                cv.set_clip(old);
            }
        }
        self.r.reflog.hits = hits;
    }
}
