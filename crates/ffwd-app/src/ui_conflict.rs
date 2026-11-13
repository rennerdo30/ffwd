//! The conflict resolver (FF-42), inside Local Changes: a conflicted file
//! shows its conflicts with both sides and buttons to choose ours, theirs
//! or both; the file can also take one side whole. A banner names a
//! stopped merge, rebase, cherry-pick or revert, with Continue and Abort.
//! Every Git call runs on a worker.

use super::*;
use ffwd_core::conflict::{Block, Choice, Seg};

pub struct ConflictView {
    pub path: String,
    pub segs: Vec<Seg>,
    pub eol: &'static str,
    pub choices: Vec<Option<Choice>>,
    /// Hand-edited results, per conflict; they win over a choice.
    pub manual: Vec<Option<Vec<String>>>,
    /// The conflict being edited, with its text.
    pub editing: Option<(usize, crate::text_area::TextArea)>,
    /// The conflict the keys act on.
    pub cur: usize,
    pub top: usize,
}

/// One painted row of the conflict view.
#[derive(Clone, Debug)]
enum Row {
    Text(String),
    /// `n` unchanged lines not shown.
    Gap(usize),
    /// Conflict header with its buttons.
    Head(usize),
    Ours(String),
    Theirs(String),
    Result(String),
    /// A line of the conflict being edited (its index in the editor).
    Edit(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KHit {
    Pick(usize, Choice),
    TakeOurs,
    TakeTheirs,
    Save,
    Continue,
    Abort,
    /// Every conflicted file at once.
    AllOurs,
    AllTheirs,
}

const CONTEXT: usize = 3;

fn choose(b: &Block, c: Choice) -> Vec<String> {
    match c {
        Choice::Ours => b.ours.clone(),
        Choice::Theirs => b.theirs.clone(),
        Choice::OursThenTheirs => b.ours.iter().chain(&b.theirs).cloned().collect(),
        Choice::TheirsThenOurs => b.theirs.iter().chain(&b.ours).cloned().collect(),
    }
}

impl ConflictView {
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut k = 0;
        let n = self.segs.len();
        for (si, s) in self.segs.iter().enumerate() {
            match s {
                Seg::Text(t) => {
                    // Context around conflicts; the rest folds into a gap.
                    let head = if si > 0 { CONTEXT } else { 0 };
                    let tail = if si + 1 < n { CONTEXT } else { 0 };
                    if t.len() <= head + tail + 1 {
                        rows.extend(t.iter().map(|l| Row::Text(l.clone())));
                    } else {
                        rows.extend(t[..head].iter().map(|l| Row::Text(l.clone())));
                        rows.push(Row::Gap(t.len() - head - tail));
                        rows.extend(t[t.len() - tail..].iter().map(|l| Row::Text(l.clone())));
                    }
                }
                Seg::Conflict(b) => {
                    rows.push(Row::Head(k));
                    if let Some((e, area)) = &self.editing
                        && *e == k
                    {
                        rows.extend((0..area.lines().len()).map(Row::Edit));
                        k += 1;
                        continue;
                    }
                    if let Some(Some(lines)) = self.manual.get(k) {
                        rows.extend(lines.iter().cloned().map(Row::Result));
                        k += 1;
                        continue;
                    }
                    match self.choices.get(k).copied().flatten() {
                        Some(c) => rows.extend(choose(b, c).into_iter().map(Row::Result)),
                        None => {
                            rows.extend(b.ours.iter().cloned().map(Row::Ours));
                            rows.extend(b.theirs.iter().cloned().map(Row::Theirs));
                        }
                    }
                    k += 1;
                }
            }
        }
        rows
    }

    fn blocks(&self) -> Vec<&Block> {
        self.segs
            .iter()
            .filter_map(|s| match s {
                Seg::Conflict(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    pub fn resolved(&self) -> usize {
        (0..self.choices.len()).filter(|&k| self.is_resolved(k)).count()
    }

    pub fn is_resolved(&self, k: usize) -> bool {
        self.choices.get(k).copied().flatten().is_some() || self.manual.get(k).is_some_and(|m| m.is_some())
    }

    /// The text to start editing conflict `k` with: its current result,
    /// else ours followed by theirs.
    fn edit_start(&self, k: usize) -> Vec<String> {
        if let Some(Some(m)) = self.manual.get(k) {
            return m.clone();
        }
        let b = self.blocks()[k];
        match self.choices.get(k).copied().flatten() {
            Some(c) => choose(b, c),
            None => b.ours.iter().chain(&b.theirs).cloned().collect(),
        }
    }

    /// The file's text with every conflict resolved (hand edits become
    /// plain text, choices are rendered by the core).
    pub fn render(&self) -> String {
        let mut segs = Vec::with_capacity(self.segs.len());
        let mut choices = Vec::new();
        let mut k = 0;
        for s in &self.segs {
            match s {
                Seg::Conflict(_) if self.manual.get(k).is_some_and(|m| m.is_some()) => {
                    segs.push(Seg::Text(self.manual[k].clone().unwrap_or_default()));
                    k += 1;
                }
                Seg::Conflict(_) => {
                    segs.push(s.clone());
                    choices.push(self.choices[k]);
                    k += 1;
                }
                Seg::Text(_) => segs.push(s.clone()),
            }
        }
        ffwd_core::conflict::render(&segs, &choices, self.eol)
    }
}

impl Ui {
    /// The selected file is conflicted: load its conflicts on a worker.
    pub(super) fn request_conflict(&mut self, path: String) {
        self.r.changes.seq += 1;
        let seq = self.r.changes.seq;
        if let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) {
            let sink = (self.sinks)(t.id);
            crate::work::spawn("conflict", &sink, move |sink| {
                let result = ffwd_core::conflict::read(&g, &path).map(|text| {
                    let (segs, eol) = ffwd_core::conflict::parse(&text);
                    (segs, eol)
                });
                sink(Msg::ConflictFile { seq, path, result });
            });
        }
    }

    pub(super) fn on_conflict_file(&mut self, seq: u64, path: String, r: Result<(Vec<Seg>, &'static str), String>) {
        if seq != self.r.changes.seq {
            return;
        }
        match r {
            Ok((segs, eol)) => {
                let n = ffwd_core::conflict::conflict_count(&segs);
                // Keep earlier choices when the same file reloads unchanged.
                let keep = self.r.changes.conflict.take().filter(|c| c.path == path && c.choices.len() == n);
                self.r.changes.conflict = Some(match keep {
                    Some(mut c) => {
                        c.segs = segs;
                        c
                    }
                    None => ConflictView {
                        path,
                        segs,
                        eol,
                        choices: vec![None; n],
                        manual: vec![None; n],
                        editing: None,
                        cur: 0,
                        top: 0,
                    },
                });
            }
            Err(e) => {
                self.r.changes.conflict = None;
                self.r.changes.error = Some(e);
            }
        }
        let d = self.changes_layout().diff;
        self.damage.rects.push(d);
    }

    fn pick(&mut self, k: usize, c: Choice) {
        if let Some(v) = &mut self.r.changes.conflict
            && k < v.choices.len()
        {
            v.choices[k] = if v.choices[k] == Some(c) { None } else { Some(c) };
            // A choice replaces a hand edit.
            v.manual[k] = None;
            // Move on to the next unresolved conflict.
            let n = v.choices.len();
            v.cur = (0..n).map(|i| (k + 1 + i) % n).find(|&i| !v.is_resolved(i)).unwrap_or(k);
        }
        let d = self.changes_layout().diff;
        self.damage.rects.push(d);
    }

    fn save_conflict(&mut self) {
        let Some(v) = &self.r.changes.conflict else { return };
        if v.resolved() < v.choices.len() {
            let left = v.choices.len() - v.resolved();
            return self
                .set_notice(format!("{left} conflict{} still need a choice.", if left == 1 { "" } else { "s" }));
        }
        let text = v.render();
        let path = v.path.clone();
        self.r.changes.conflict = None;
        self.run_change_op(crate::work::ChangeOp::SaveResolved(path, text), "Saving");
    }

    pub(super) fn conflict_key(&mut self, k: Key) -> bool {
        let Some(v) = &mut self.r.changes.conflict else { return false };
        // Editing a conflict by hand: the keys go to the text.
        if let Some((e, area)) = &mut v.editing {
            let e = *e;
            match k {
                Key::Esc => v.editing = None,
                Key::CtrlEnter => {
                    let lines = area.lines().to_vec();
                    v.editing = None;
                    v.manual[e] = Some(lines);
                    v.choices[e] = None;
                    let n = v.choices.len();
                    v.cur = (0..n).map(|i| (e + 1 + i) % n).find(|&i| !v.is_resolved(i)).unwrap_or(e);
                }
                Key::Enter => area.newline(),
                Key::Char(c) => area.insert(c.encode_utf8(&mut [0; 4])),
                Key::Tab => area.insert("    "),
                Key::Backspace => area.backspace(),
                Key::CtrlBackspace => area.delete_word(),
                Key::Delete => area.delete(),
                Key::Left => area.left(),
                Key::Right => area.right(),
                Key::Up => {
                    area.vertical(false);
                }
                Key::Down => {
                    area.vertical(true);
                }
                Key::Home => area.home(),
                Key::End => area.end(),
                Key::Ctrl('a') => area.select_all(),
                Key::Ctrl('v') => {
                    if let Some(t) = crate::platform::clipboard() {
                        area.insert(&t);
                    }
                }
                _ => return true,
            }
            let d = self.changes_layout().diff;
            self.damage.rects.push(d);
            return true;
        }
        let n = v.choices.len();
        let cur = v.cur.min(n.saturating_sub(1));
        match k {
            Key::Char('o') => self.pick(cur, Choice::Ours),
            Key::Char('t') => self.pick(cur, Choice::Theirs),
            Key::Char('b') => self.pick(cur, Choice::OursThenTheirs),
            Key::Char('B') => self.pick(cur, Choice::TheirsThenOurs),
            Key::Char('e') if n > 0 => {
                let start = v.edit_start(cur);
                let mut area = crate::text_area::TextArea::new(&start.join("\n"));
                area.select_all();
                area.right();
                v.editing = Some((cur, area));
            }
            Key::Char('n') | Key::Down if n > 0 => v.cur = (cur + 1) % n,
            Key::Char('p') | Key::Up if n > 0 => v.cur = (cur + n - 1) % n,
            Key::PageDown => v.top += 20,
            Key::PageUp => v.top = v.top.saturating_sub(20),
            Key::Enter => self.save_conflict(),
            _ => return false,
        }
        let d = self.changes_layout().diff;
        self.damage.rects.push(d);
        true
    }

    pub(super) fn conflict_wheel(&mut self, n: i64) {
        if let Some(v) = &mut self.r.changes.conflict {
            v.top = (v.top as i64 + n).max(0) as usize;
            let d = self.changes_layout().diff;
            self.damage.rects.push(d);
        }
    }

    pub(super) fn conflict_click(&mut self, h: KHit) {
        let path = self.r.changes.conflict.as_ref().map(|c| c.path.clone());
        match h {
            KHit::Pick(k, c) => self.pick(k, c),
            KHit::Save => self.save_conflict(),
            KHit::TakeOurs | KHit::TakeTheirs => {
                if let Some(p) = path {
                    self.r.changes.conflict = None;
                    let ours = h == KHit::TakeOurs;
                    self.run_change_op(crate::work::ChangeOp::TakeSide(p, ours), "Resolving");
                }
            }
            KHit::Continue => {
                if let Some(kind) = self.r.changes.in_progress {
                    self.run_change_op(crate::work::ChangeOp::Continue(kind), "Continuing");
                }
            }
            KHit::Abort => self.ask_abort(),
            KHit::AllOurs => self.ask_take_all(true),
            KHit::AllTheirs => self.ask_take_all(false),
        }
    }

    /// Every conflicted file with one side: ask first, naming the files
    /// (the other side's changes in them are dropped; rule 8).
    pub(super) fn ask_take_all(&mut self, ours: bool) {
        let paths: Vec<String> = self
            .r
            .changes
            .status
            .as_ref()
            .map(|s| s.entries.iter().filter(|e| e.conflicted).map(|e| e.path.to_string()).collect())
            .unwrap_or_default();
        if paths.is_empty() {
            return self.set_notice("No file has conflicts.".into());
        }
        let side = if ours { "ours (HEAD)" } else { "theirs (the incoming side)" };
        let other = if ours { "theirs" } else { "ours" };
        let n = paths.len();
        let list: Vec<&str> = paths.iter().take(6).map(String::as_str).collect();
        let more = if n > 6 { format!(" and {} more", n - 6) } else { String::new() };
        self.open_dialog(menu::DialogKind::Ask(menu::Ask {
            title: format!("Take {side} for all {n} conflicted file{}?", if n == 1 { "" } else { "s" }),
            text: format!(
                "{}{more}: each file becomes {side} as a whole; {other} changes in them are dropped and the files are marked resolved. Abort the operation to get everything back.",
                list.join(", ")
            ),
            choices: vec![(
                format!("Take {} for all", if ours { "ours" } else { "theirs" }),
                crate::work::RefOp::TakeSideAll(paths, ours),
                true,
            )],
        }));
    }

    /// Abort throws away the resolutions so far: ask first (rule 8).
    fn ask_abort(&mut self) {
        let Some(kind) = self.r.changes.in_progress else { return };
        self.open_dialog(menu::DialogKind::Ask(menu::Ask {
            title: format!("Abort the {kind}?"),
            text: format!(
                "The repository goes back to how it was before the {kind}. Conflicts you resolved and changes you staged for it are thrown away."
            ),
            choices: vec![(format!("Abort the {kind}"), crate::work::RefOp::Abort(kind), true)],
        }));
    }

    /// The banner for a stopped operation, at the top of the left pane.
    pub(super) fn paint_op_banner(&mut self, cv: &mut Canvas, r: Rect, hits: &mut Vec<(Rect, changes_view::CHit)>) {
        let Some(kind) = self.r.changes.in_progress else { return };
        let (ui, semi) = (self.f.ui, self.f.semi);
        cv.fill(r, TAG_BG);
        cv.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1), BORDER);
        let conflicts = self.r.changes.conflicts;
        // Two rows: the state on top, the buttons below, right-aligned.
        let two = r.h > self.px(50.0);
        let (row1, row2) = if two {
            (Rect::new(r.x, r.y, r.w, self.px(30.0)), Rect::new(r.x, r.y + self.px(28.0), r.w, r.h - self.px(28.0)))
        } else {
            (r, r)
        };
        let base = (row1.y as f32 + row1.h as f32 / 2.0 + self.pf(4.5)).round();
        let mut x = r.right() - self.px(10.0);
        let buttons = [
            ("Abort…", KHit::Abort, true),
            ("All theirs…", KHit::AllTheirs, conflicts > 1),
            ("All ours…", KHit::AllOurs, conflicts > 1),
            ("Continue", KHit::Continue, conflicts == 0),
        ];
        for (label, hit, enabled) in buttons {
            // The all-files buttons only show with several conflicts.
            if matches!(hit, KHit::AllOurs | KHit::AllTheirs) && !enabled {
                continue;
            }
            let w = (self.te.measure(semi, self.pf(11.5), label) + self.pf(22.0)) as i32;
            let b = Rect::new(x - w, row2.y + self.px(5.0), w, row2.h - self.px(10.0));
            let hot = self.is_hot(Hot::Change(hits.len()));
            // Continue is the primary action, Abort the dangerous one; the
            // all-files choices are plain buttons (they ask first anyway).
            let (bg, border, fg) = match (enabled, hit, hot) {
                (false, _, _) => (BG_BAR, BORDER, TEXT_4),
                (true, KHit::Continue, true) => (ACCENT, ACCENT, ACCENT_TEXT),
                (true, KHit::Continue, false) => (ACCENT_SOFT_HOVER, ACCENT_BORDER, ACCENT_FG),
                (true, KHit::Abort, true) => (DEL_BG, DEL, DEL_TEXT),
                (true, KHit::Abort, false) => (BG_BAR, DANGER_BORDER, DEL_TEXT),
                (true, _, true) => (CONTROL_HOVER, BORDER_HOVER, TEXT),
                (true, _, false) => (BG_BAR, BORDER, TEXT_2),
            };
            cv.round_border(b, self.pf(6.0), border, bg);
            self.te.draw(
                cv,
                semi,
                self.pf(11.5),
                b.x as f32 + self.pf(11.0),
                (b.y as f32 + b.h as f32 / 2.0 + self.pf(4.0)).round(),
                label,
                fg,
                w as f32,
            );
            if enabled {
                hits.push((b, changes_view::CHit::Conflict(hit)));
            }
            x = b.x - self.px(6.0);
        }
        // The state, in the space the buttons leave (never under them).
        let tx = r.x as f32 + self.pf(14.0);
        let room = if two { r.w as f32 - self.pf(28.0) } else { x as f32 - self.pf(12.0) - tx };
        let k = title_case(kind);
        let long = if conflicts > 0 {
            format!("{k} stopped · {conflicts} conflicted file{}", if conflicts == 1 { "" } else { "s" })
        } else {
            format!("{k} stopped · all conflicts resolved")
        };
        let short = if conflicts > 0 { format!("{k} stopped · {conflicts}") } else { format!("{k} stopped") };
        let text = [long, short, format!("{k} stopped")]
            .into_iter()
            .find(|t| self.te.measure(semi, self.pf(12.5), t) <= room)
            .unwrap_or_default();
        self.te.draw(cv, semi, self.pf(12.5), tx, base, &text, TAG_FG, room.max(0.0));
        let _ = ui;
    }

    /// The conflict view in the diff pane.
    pub(super) fn paint_conflict(&mut self, cv: &mut Canvas, r: Rect, hits: &mut Vec<(Rect, changes_view::CHit)>) {
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        cv.fill(r, BG);
        let bar = Rect::new(r.x, r.y, r.w, self.px(32.0));
        cv.fill(bar, BG_BAR);
        cv.fill(Rect::new(bar.x, bar.bottom() - 1, bar.w, 1), BORDER_SOFT);
        let base = (bar.y as f32 + bar.h as f32 / 2.0 + self.pf(4.5)).round();
        let Some(v) = self.r.changes.conflict.take() else {
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                r.x as f32 + self.pf(16.0),
                bar.bottom() as f32 + self.pf(26.0),
                "Loading the conflicts…",
                TEXT_4,
                r.w as f32,
            );
            return;
        };
        let n = v.choices.len();
        let title = format!("{} · {} of {n} resolved", v.path, v.resolved());
        self.te.draw(cv, semi, self.pf(12.5), r.x as f32 + self.pf(16.0), base, &title, TEXT, r.w as f32 * 0.45);
        // File buttons, right to left.
        let mut x = r.right() - self.px(10.0);
        let all = v.resolved() == n;
        for (label, hit, primary, enabled) in [
            ("Save and mark resolved (Enter)", KHit::Save, true, all),
            ("Take theirs", KHit::TakeTheirs, false, true),
            ("Take ours", KHit::TakeOurs, false, true),
        ] {
            let w = (self.te.measure(semi, self.pf(11.5), label) + self.pf(20.0)) as i32;
            let b = Rect::new(x - w, bar.y + self.px(4.0), w, bar.h - self.px(8.0));
            let hot = self.is_hot(Hot::Change(hits.len()));
            let (bg, border, fg) = match (enabled, primary, hot) {
                (false, _, _) => (BG_BAR, BORDER_SOFT, TEXT_4),
                (true, true, true) => (ACCENT, ACCENT, ACCENT_TEXT),
                (true, true, false) => (ACCENT_SOFT_HOVER, ACCENT_BORDER, ACCENT_FG),
                (true, false, true) => (CONTROL_HOVER, BORDER_HOVER, TEXT),
                (true, false, false) => (BG_BAR, BORDER, TEXT_2),
            };
            cv.round_border(b, self.pf(6.0), border, bg);
            self.te.draw(cv, semi, self.pf(11.5), b.x as f32 + self.pf(10.0), base - self.pf(0.5), label, fg, w as f32);
            if enabled {
                hits.push((b, changes_view::CHit::Conflict(hit)));
            }
            x = b.x - self.px(6.0);
        }
        // Rows.
        let body = Rect::new(r.x, bar.bottom(), r.w, r.bottom() - bar.bottom());
        let rows = v.rows();
        let lh = self.px(20.0);
        let fit = (body.h / lh).max(1) as usize;
        let blocks = v.blocks();
        // Keep the current conflict's header on screen.
        let mut top = v.top.min(rows.len().saturating_sub(fit));
        if let Some(hr) = rows.iter().position(|row| matches!(row, Row::Head(k) if *k == v.cur))
            && (hr < top || hr >= top + fit)
        {
            top = hr.saturating_sub(2);
        }
        let clip = cv.clip();
        for (j, row) in rows.iter().enumerate().skip(top).take(fit + 1) {
            let y = body.y + (j - top) as i32 * lh;
            let rr = Rect::new(body.x, y, body.w, lh);
            if clip.intersect(rr).is_empty() {
                continue;
            }
            let lb = (y as f32 + lh as f32 / 2.0 + self.pf(4.0)).round();
            // A gutter for the OURS/THEIRS tags, code aligned after it.
            let tx = body.x as f32 + self.pf(66.0);
            match row {
                Row::Text(t) => {
                    self.te.draw(
                        cv,
                        mono,
                        self.pf(12.0),
                        tx,
                        lb,
                        &t.replace('\t', "    "),
                        TEXT_2,
                        body.w as f32 - self.pf(40.0),
                    );
                }
                Row::Gap(n) => {
                    cv.fill(rr, BG_SIDEBAR);
                    let t = format!("… {n} unchanged lines");
                    self.te.draw(cv, ui, self.pf(11.5), tx, lb, &t, TEXT_3, body.w as f32);
                }
                Row::Head(k) => {
                    let b = blocks[*k];
                    let on = *k == v.cur;
                    cv.fill(rr, HUNK_BG);
                    cv.fill(Rect::new(rr.x, rr.y, rr.w, 1), BORDER);
                    if on {
                        cv.fill(Rect::new(rr.x, rr.y, self.px(4.0), rr.h), ACCENT);
                    }
                    let edited = v.manual.get(*k).is_some_and(|m| m.is_some());
                    let editing = v.editing.as_ref().is_some_and(|x| x.0 == *k);
                    let t = if editing {
                        format!("Conflict {} of {n} · editing: Ctrl+Enter keeps, Esc cancels", k + 1)
                    } else if edited {
                        format!("Conflict {} of {n} · edited by hand (e edits again)", k + 1)
                    } else {
                        format!("Conflict {} of {n}", k + 1)
                    };
                    let tc = if on { TEXT } else { TEXT_2 };
                    let e = self.te.draw(cv, semi, self.pf(12.0), tx, lb, &t, tc, self.pf(150.0));
                    let mut bx = e + self.pf(14.0);
                    let chosen = v.choices[*k];
                    for (label, c) in [
                        (format!("Ours · {} (o)", short_label(&b.ours_label)), Choice::Ours),
                        (format!("Theirs · {} (t)", short_label(&b.theirs_label)), Choice::Theirs),
                        ("Both (b)".to_string(), Choice::OursThenTheirs),
                        ("Both, theirs first".to_string(), Choice::TheirsThenOurs),
                    ] {
                        let w = self.te.measure(semi, self.pf(11.5), &label) + self.pf(18.0);
                        let br = Rect::new(bx as i32, y + self.px(1.0), w as i32, lh - self.px(2.0));
                        let sel = chosen == Some(c);
                        let hot = self.is_hot(Hot::Change(hits.len()));
                        // Ours and theirs carry their rows' colours.
                        let side = match c {
                            Choice::Ours => OURS_FG,
                            Choice::Theirs => THEIRS_FG,
                            _ => TEXT_2,
                        };
                        let (bg, border, fg) = match (sel, hot) {
                            (true, _) => (ACCENT, ACCENT, ACCENT_TEXT),
                            (false, true) => (CONTROL_HOVER, BORDER_HOVER, TEXT),
                            _ => (BG_INPUT, BORDER_STRONG, side),
                        };
                        cv.round_border(br, self.pf(5.0), border, bg);
                        self.te.draw(cv, semi, self.pf(11.5), br.x as f32 + self.pf(9.0), lb, &label, fg, w);
                        hits.push((br, changes_view::CHit::Conflict(KHit::Pick(*k, c))));
                        bx += w + self.pf(6.0);
                    }
                }
                Row::Edit(i) => {
                    cv.fill(rr, BG_INPUT);
                    cv.fill(Rect::new(rr.x, rr.y, self.px(3.0), rr.h), ACCENT);
                    if let Some((_, area)) = &v.editing {
                        let line = area.lines().get(*i).cloned().unwrap_or_default();
                        let shown = line.replace('\t', "    ");
                        self.te.draw(cv, mono, self.pf(12.0), tx, lb, &shown, TEXT, body.w as f32 - self.pf(40.0));
                        let (cl, cc) = area.cursor();
                        if cl == *i {
                            let before: String = line.chars().take(cc).collect::<String>().replace('\t', "    ");
                            let cx = tx + self.te.measure(mono, self.pf(12.0), &before);
                            cv.fill(
                                Rect::new(cx as i32, y + self.px(3.0), self.px(1.5).max(1), lh - self.px(6.0)),
                                ACCENT,
                            );
                        }
                    }
                }
                Row::Ours(t) | Row::Theirs(t) | Row::Result(t) => {
                    let (bg, mark, fg) = match row {
                        Row::Ours(..) => (OURS_BG, "OURS", OURS_FG),
                        Row::Theirs(..) => (THEIRS_BG, "THEIRS", THEIRS_FG),
                        _ => (ADD_BG, "", ADD),
                    };
                    cv.fill(rr, bg);
                    cv.fill(Rect::new(rr.x, rr.y, self.px(4.0), rr.h), fg);
                    // The side's name on the first row of its block.
                    let first = j == 0
                        || !matches!(
                            (&rows[j - 1], row),
                            (Row::Ours(..), Row::Ours(..)) | (Row::Theirs(..), Row::Theirs(..))
                        );
                    if !mark.is_empty() && first {
                        let mx = rr.x as f32 + self.pf(12.0);
                        self.te.draw(cv, semi, self.pf(10.5), mx, lb, mark, fg, self.pf(52.0));
                    }
                    self.te.draw(
                        cv,
                        mono,
                        self.pf(12.0),
                        tx,
                        lb,
                        &t.replace('\t', "    "),
                        TEXT,
                        body.w as f32 - self.pf(40.0),
                    );
                }
            }
        }
        let hint = "o ours · t theirs · b both · e edit · ↑↓ conflicts · Enter saves";
        let hw = self.te.measure(ui, self.pf(11.0), hint);
        let hy = r.bottom() as f32 - self.pf(10.0);
        if rows.len() < fit {
            self.te.draw(cv, ui, self.pf(11.0), r.right() as f32 - hw - self.pf(16.0), hy, hint, TEXT_3, hw + 2.0);
        }
        self.r.changes.conflict = Some(ConflictView { top, ..v });
    }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// `HEAD`, a branch, or a long "commit (message)" label, kept short.
fn short_label(l: &str) -> String {
    let l = l.trim();
    if l.chars().count() > 28 { format!("{}…", l.chars().take(27).collect::<String>()) } else { l.to_string() }
}
