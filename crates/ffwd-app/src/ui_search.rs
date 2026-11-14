//! The find bar over the commit list (`Ctrl+F`, FF-34): search by message,
//! author, SHA, content or path. Results stream in from a worker; commits
//! that don't match are dimmed, matches are marked on the scroll track,
//! and `Enter`/`↓`/`↑` jump between them.

use super::*;
use ffwd_core::search::Field;
use std::collections::{HashMap, HashSet};

/// The search modes, in the order of the chips.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Git(Field),
    /// A SHA prefix, matched in the loaded history (no git call).
    Sha,
}

const MODES: [Mode; 5] =
    [Mode::Git(Field::Message), Mode::Git(Field::Author), Mode::Sha, Mode::Git(Field::Content), Mode::Git(Field::Path)];

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Git(f) => f.label(),
            Mode::Sha => "SHA",
        }
    }

    /// Content and path searches read every commit's diff: they start on
    /// Enter, not on every key.
    fn on_enter(self) -> bool {
        matches!(self, Mode::Git(Field::Content | Field::Path))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FHit {
    Field,
    Mode(usize),
    Prev,
    Next,
    Close,
}

pub struct Search {
    pub input: TextField,
    mode: usize,
    pub matches: HashSet<Oid>,
    /// Content and path searches: the files that matched, per commit.
    pub files: HashMap<Oid, Vec<String>>,
    /// Matching rows of the loaded history, ascending.
    pub rows: Vec<usize>,
    pub done: bool,
    /// Typed but not searched yet (content and path wait for Enter).
    pending: bool,
    seq: u64,
    /// The bar has the keyboard.
    pub typing: bool,
    hits: Vec<(Rect, FHit)>,
    /// Browsing recent queries: the index into them (0 = newest).
    recall: Option<usize>,
}

/// Recent queries kept in the session state (FF-34).
const RECENT_KEY: &str = "search.recent";
const RECENT_MAX: usize = 10;

impl Ui {
    pub(super) fn search_active(&self) -> bool {
        self.r.search.as_ref().is_some_and(|s| !s.input.text().is_empty() && !s.pending)
    }

    pub(super) fn open_search(&mut self) {
        if self.r.changes_view || self.tabs.is_empty() {
            return;
        }
        match &mut self.r.search {
            Some(s) => {
                s.typing = true;
                s.input.select_all();
            }
            None => {
                self.r.search = Some(Search {
                    input: TextField::default(),
                    mode: 0,
                    matches: HashSet::new(),
                    files: HashMap::new(),
                    rows: Vec::new(),
                    done: true,
                    pending: false,
                    seq: 0,
                    typing: true,
                    hits: Vec::new(),
                    recall: None,
                })
            }
        }
        self.damage.rects.push(self.lay.list);
    }

    pub(super) fn close_search(&mut self) {
        if self.r.search.take().is_some() {
            self.search_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.damage.rects.extend([self.lay.list, self.lay.details]);
        }
    }

    pub(super) fn restart_search(&mut self) {
        let seq = self.search_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let Some(s) = &mut self.r.search else { return };
        s.matches.clear();
        s.files.clear();
        s.rows.clear();
        s.pending = false;
        s.seq = seq;
        let q = s.input.text().trim().to_string();
        let mode = MODES[s.mode];
        s.done = q.is_empty() || mode == Mode::Sha;
        self.damage.rects.extend([self.lay.list, self.lay.details]);
        if q.is_empty() {
            return;
        }
        match mode {
            Mode::Sha => {
                let q = q.to_ascii_lowercase();
                let rows: Vec<usize> = self
                    .r
                    .commits
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.id.to_string().starts_with(&q))
                    .map(|(i, _)| i)
                    .collect();
                let matches = rows.iter().map(|&i| self.r.commits[i].id).collect();
                if let Some(s) = &mut self.r.search {
                    s.matches = matches;
                    s.rows = rows;
                }
                self.jump_match(true, true);
            }
            Mode::Git(field) => {
                let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
                let sink = (self.sinks)(t.id);
                let cur = self.search_seq.clone();
                // The same history the list shows (FF-33 scope).
                let head_only = t.head_only;
                crate::work::spawn("search", &sink, move |sink| {
                    ffwd_core::search::stream(&g, field, &q, head_only, seq, &cur, |ids, done| {
                        sink(Msg::SearchBatch { seq, ids, done })
                    });
                });
            }
        }
    }

    pub(super) fn on_search_batch(&mut self, seq: u64, ids: Vec<ffwd_core::search::Match>, done: bool) {
        let Some(s) = &mut self.r.search else { return };
        if s.seq != seq {
            return;
        }
        let first = s.rows.is_empty();
        let shown = self.r.details.as_ref().map(|d| d.id);
        let mut touches_shown = false;
        for m in ids {
            let id = m.id;
            if !m.files.is_empty() {
                touches_shown |= shown == Some(id);
                s.files.insert(id, m.files);
            }
            if s.matches.insert(id)
                && let Some(&row) = self.r.by_id.get(&id)
            {
                s.rows.push(row as usize);
            }
        }
        if touches_shown {
            // The shown commit's matching files are marked now.
            self.damage.rects.push(self.lay.details);
        }
        s.rows.sort_unstable();
        s.done = done;
        if first && !s.rows.is_empty() {
            self.jump_match(true, true);
        }
        self.damage_visible_rows();
        self.damage.rects.extend([self.lay.scrollbar, self.lay.list_head]);
    }

    /// The files of `id` that matched a content or path search.
    pub(super) fn match_files(&self, id: Oid) -> Option<&[String]> {
        self.r.search.as_ref().filter(|s| !s.pending)?.files.get(&id).map(Vec::as_slice)
    }

    /// The file to show first in the selected commit: the first one that
    /// matched the search, else the first file.
    pub(super) fn first_file(&self) -> usize {
        let Some(d) = &self.r.details else { return 0 };
        let Some(m) = self.match_files(d.id) else { return 0 };
        let m: HashSet<&str> = m.iter().map(String::as_str).collect();
        d.files.iter().position(|f| m.contains(&*f.path)).unwrap_or(0)
    }

    /// New history rows arrived: matches among them become rows.
    pub(super) fn search_new_rows(&mut self, from: usize) {
        let Some(s) = &mut self.r.search else { return };
        if s.matches.is_empty() {
            return;
        }
        let before = s.rows.len();
        for (i, c) in self.r.commits.iter().enumerate().skip(from) {
            if s.matches.contains(&c.id) {
                s.rows.push(i);
            }
        }
        if s.rows.len() != before {
            s.rows.sort_unstable();
            self.damage.rects.extend([self.lay.scrollbar, self.lay.list_head]);
        }
    }

    /// Selects the next (or previous) match after the selection; `inclusive`
    /// accepts the selected row itself.
    pub(super) fn jump_match(&mut self, forward: bool, inclusive: bool) {
        let Some(s) = &self.r.search else { return };
        let sel = self.r.sel;
        let row = if forward {
            s.rows.iter().copied().find(|&r| if inclusive { r >= sel } else { r > sel }).or(s.rows.first().copied())
        } else {
            s.rows.iter().rev().copied().find(|&r| r < sel).or(s.rows.last().copied())
        };
        if let Some(r) = row {
            self.select(r);
            self.damage.rects.push(self.lay.list_head);
        }
    }

    /// Keys while the bar has the keyboard; false leaves a key to the rest.
    pub(super) fn search_key(&mut self, k: Key) -> bool {
        let Some(s) = &mut self.r.search else { return false };
        let instant = !MODES[s.mode].on_enter();
        let f = &mut s.input;
        let edited = match k {
            Key::Esc => {
                let q = f.text().trim().to_string();
                self.remember_query(&q);
                self.close_search();
                return true;
            }
            Key::Up | Key::Down if f.text().is_empty() || s.recall.is_some() => {
                // An empty field (or one being browsed) walks recent queries.
                let recent = self.state.get_list(RECENT_KEY);
                let s = self.r.search.as_mut().unwrap_or_else(|| unreachable!());
                let next = match (k, s.recall) {
                    (Key::Up, None) => Some(0),
                    (Key::Up, Some(i)) => Some((i + 1).min(recent.len().saturating_sub(1))),
                    (_, Some(0)) | (_, None) => None,
                    (_, Some(i)) => Some(i - 1),
                };
                match next.and_then(|i| recent.get(i).map(|q| (i, q.clone()))) {
                    Some((i, q)) => {
                        s.input = TextField::new(&q);
                        s.input.end();
                        s.recall = Some(i);
                    }
                    None => {
                        s.input = TextField::default();
                        s.recall = None;
                    }
                }
                true
            }
            Key::Enter => {
                let q = f.text().trim().to_string();
                self.remember_query(&q);
                let s = self.r.search.as_mut().unwrap_or_else(|| unreachable!());
                let f = &s.input;
                if s.pending || (!instant && s.matches.is_empty() && s.done && !f.text().is_empty()) {
                    self.restart_search();
                } else {
                    self.jump_match(true, false);
                }
                return true;
            }
            Key::Down => {
                self.jump_match(true, false);
                return true;
            }
            Key::Up => {
                self.jump_match(false, false);
                return true;
            }
            Key::Tab | Key::BackTab => {
                let n = MODES.len();
                s.mode = if matches!(k, Key::Tab) { (s.mode + 1) % n } else { (s.mode + n - 1) % n };
                true
            }
            Key::Char(c) => {
                f.insert(c.encode_utf8(&mut [0; 4]));
                s.recall = None;
                true
            }
            Key::Backspace => {
                f.backspace();
                true
            }
            Key::CtrlBackspace => {
                f.delete_word();
                true
            }
            Key::Delete => {
                f.delete();
                true
            }
            Key::Left | Key::Right | Key::Home | Key::End | Key::Ctrl('a') => {
                match k {
                    Key::Left => f.left(),
                    Key::Right => f.right(),
                    Key::Home => f.home(),
                    Key::End => f.end(),
                    _ => f.select_all(),
                }
                self.damage.rects.push(self.lay.list_head);
                return true;
            }
            Key::Ctrl('c') => {
                crate::platform::set_clipboard(f.copy_text());
                return true;
            }
            Key::Ctrl('v') => {
                if let Some(t) = crate::platform::clipboard() {
                    f.insert(t.lines().next().unwrap_or(""));
                }
                true
            }
            Key::Ctrl('x') => {
                let t = f.cut();
                crate::platform::set_clipboard(&t);
                true
            }
            _ => return false,
        };
        if edited {
            let s = self.r.search.as_mut().unwrap_or_else(|| unreachable!());
            if MODES[s.mode].on_enter() && !s.input.text().is_empty() {
                // Waits for Enter; old results go.
                s.pending = true;
                s.matches.clear();
                s.files.clear();
                s.rows.clear();
                self.search_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.damage.rects.extend([self.lay.list, self.lay.details]);
            } else {
                self.restart_search();
            }
        }
        true
    }

    /// Keeps `q` as the newest recent query (session state).
    fn remember_query(&mut self, q: &str) {
        if q.is_empty() {
            return;
        }
        let mut recent = self.state.get_list(RECENT_KEY);
        recent.retain(|x| x != q);
        recent.insert(0, q.to_string());
        recent.truncate(RECENT_MAX);
        if self.state.get_list(RECENT_KEY) != recent {
            self.state.set_list(RECENT_KEY, &recent);
        }
    }

    /// A click in the list header while the bar is open.
    pub(super) fn search_click(&mut self, x: i32, y: i32) -> bool {
        let Some(s) = &self.r.search else { return false };
        if !self.lay.list_head.contains(x, y) {
            if let Some(s) = &mut self.r.search {
                s.typing = false;
                self.damage.rects.push(self.lay.list_head);
            }
            return false;
        }
        let hit = s.hits.iter().find(|h| h.0.contains(x, y)).map(|h| h.1);
        match hit {
            Some(FHit::Mode(i)) => {
                if let Some(s) = &mut self.r.search {
                    s.mode = i;
                    s.typing = true;
                }
                self.restart_search();
            }
            Some(FHit::Prev) => self.jump_match(false, false),
            Some(FHit::Next) => self.jump_match(true, false),
            Some(FHit::Close) => self.close_search(),
            _ => {
                if let Some(s) = &mut self.r.search {
                    s.typing = true;
                }
                self.damage.rects.push(self.lay.list_head);
            }
        }
        true
    }

    pub(super) fn search_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.r.search.as_ref()?.hits.iter().position(|h| h.0.contains(x, y))
    }

    pub(super) fn search_hit_is_field(&self, i: usize) -> bool {
        self.r.search.as_ref().is_some_and(|s| matches!(s.hits.get(i), Some((_, FHit::Field))))
    }

    /// Is this row dimmed (a search is active and it doesn't match)?
    pub(super) fn row_dimmed(&self, row: usize) -> bool {
        self.search_active()
            && self
                .r
                .search
                .as_ref()
                .is_some_and(|s| self.r.commits.get(row).is_some_and(|c| !s.matches.contains(&c.id)))
    }

    /// The find bar, in place of the list header.
    pub(super) fn paint_search_head(&mut self, cv: &mut Canvas, head: Rect) {
        let Some(mut s) = self.r.search.take() else { return };
        let (ui, semi) = (self.f.ui, self.f.semi);
        cv.fill(head, BG_BAR());
        cv.fill(Rect::new(head.x, head.bottom() - 1, head.w, 1), BORDER_SOFT());
        let mut hits = Vec::new();
        let fy = head.y + self.px(3.0);
        let fh = head.h - self.px(6.0);
        let fw = (head.w as f32 * 0.36) as i32;
        let field = Rect::new(head.x + self.px(10.0), fy, fw.max(self.px(220.0)), fh);
        let border = if s.typing {
            ACCENT()
        } else if self.is_hot(Hot::Find(hits.len())) {
            BORDER_HOVER()
        } else {
            BORDER()
        };
        cv.round_border(field, self.pf(6.0), border, BG_INPUT());
        let base = (field.y as f32 + field.h as f32 / 2.0 + self.pf(4.5)).round();
        let tx = field.x as f32 + self.pf(10.0);
        let mode = MODES[s.mode];
        if s.input.text().is_empty() {
            let ph = match mode {
                Mode::Sha => "SHA prefix",
                Mode::Git(Field::Content) => "Text added or removed · Enter",
                Mode::Git(Field::Path) => "Part of a file path · Enter",
                Mode::Git(Field::Author) => "Author name or e-mail",
                Mode::Git(Field::Message) => "Search commit messages",
            };
            self.te.draw(cv, ui, self.pf(12.5), tx, base, ph, TEXT_4(), field.w as f32 - self.pf(20.0));
        } else {
            if s.input.all_selected() {
                let w = self.te.measure(ui, self.pf(12.5), s.input.text());
                cv.fill_round(
                    Rect::new(tx as i32 - 2, field.y + self.px(4.0), w as i32 + 4, field.h - self.px(8.0)),
                    self.pf(3.0),
                    BG_TEXT_SELECT(),
                );
            }
            self.te.draw(cv, ui, self.pf(12.5), tx, base, s.input.text(), TEXT(), field.w as f32 - self.pf(20.0));
        }
        if s.typing {
            let cx = tx + self.te.measure(ui, self.pf(12.5), &s.input.text()[..s.input.cursor()]);
            cv.fill(
                Rect::new(cx as i32 + 1, field.y + self.px(5.0), self.px(1.5).max(1), field.h - self.px(10.0)),
                ACCENT(),
            );
        }
        hits.push((field, FHit::Field));
        // Count and prev/next.
        let mut x = field.right() as f32 + self.pf(12.0);
        let count = if s.input.text().is_empty() {
            String::new()
        } else if s.pending {
            "Enter searches".to_string()
        } else {
            let pos = s.rows.iter().position(|&r| r == self.r.sel).map(|p| p + 1);
            let n = s.rows.len();
            let more = if s.done { "" } else { " · searching…" };
            match (pos, n) {
                (_, 0) if s.done => "No matches".to_string(),
                (Some(p), _) => format!("{p} of {n}{more}"),
                (None, _) => format!("{n} matches{more}"),
            }
        };
        let cw = self.te.measure(ui, self.pf(12.0), &count);
        self.te.draw(
            cv,
            ui,
            self.pf(12.0),
            x,
            base,
            &count,
            if count == "No matches" { MOD() } else { TEXT_2() },
            cw + 2.0,
        );
        x += cw.max(self.pf(90.0)) + self.pf(8.0);
        for (glyph, hit) in [("↑", FHit::Prev), ("↓", FHit::Next)] {
            let b = Rect::new(x as i32, fy, self.px(26.0), fh);
            let (bg, bd) = self.control_colors(Hot::Find(hits.len()), BG_BAR(), BORDER());
            cv.round_border(b, self.pf(5.0), bd, bg);
            let gw = self.te.measure(semi, self.pf(12.0), glyph);
            self.te.draw(
                cv,
                semi,
                self.pf(12.0),
                b.x as f32 + (b.w as f32 - gw) / 2.0,
                base,
                glyph,
                TEXT_2(),
                gw + 2.0,
            );
            hits.push((b, hit));
            x = b.right() as f32 + self.pf(4.0);
        }
        // Mode chips.
        x += self.pf(12.0);
        for (i, m) in MODES.iter().enumerate() {
            let w = self.te.measure(semi, self.pf(11.5), m.label()) + self.pf(18.0);
            let b = Rect::new(x as i32, fy, w as i32, fh);
            if i == s.mode {
                cv.fill_round(b, self.pf(5.0), BG_CHOSEN());
            } else if self.is_hot(Hot::Find(hits.len())) {
                cv.fill_round(b, self.pf(5.0), CONTROL_HOVER());
            }
            self.te.draw(
                cv,
                semi,
                self.pf(11.5),
                b.x as f32 + self.pf(9.0),
                base,
                m.label(),
                if i == s.mode { TEXT() } else { TEXT_3() },
                w,
            );
            hits.push((b, FHit::Mode(i)));
            x = b.right() as f32 + self.pf(2.0);
        }
        // Close.
        let cb = Rect::new(head.right() - self.px(34.0), fy, self.px(26.0), fh);
        if self.is_hot(Hot::Find(hits.len())) {
            cv.fill_round(cb, self.pf(5.0), CONTROL_HOVER());
        }
        let (cx, cy, k) = (cb.x as f32 + cb.w as f32 / 2.0, cb.y as f32 + cb.h as f32 / 2.0, self.pf(4.0));
        cv.stroke(&[(cx - k, cy - k), (cx + k, cy + k)], self.pf(1.4), TEXT_3());
        cv.stroke(&[(cx - k, cy + k), (cx + k, cy - k)], self.pf(1.4), TEXT_3());
        hits.push((cb, FHit::Close));
        let hint = "Tab: next field · ↑↓: matches · Esc";
        let hw = self.te.measure(ui, self.pf(11.0), hint);
        if x + hw + self.pf(20.0) < cb.x as f32 {
            self.te.draw(cv, ui, self.pf(11.0), cb.x as f32 - hw - self.pf(12.0), base, hint, TEXT_4(), hw + 2.0);
        }
        s.hits = hits;
        self.r.search = Some(s);
    }

    /// Match marks on the scroll track.
    pub(super) fn paint_search_marks(&mut self, cv: &mut Canvas, sb: Rect) {
        let Some(s) = &self.r.search else { return };
        let n = self.r.commits.len().max(1);
        let mut last = i32::MIN;
        for &row in &s.rows {
            let y = sb.y + (sb.h as f32 * row as f32 / n as f32) as i32;
            if y == last {
                continue; // one mark per pixel row is enough
            }
            last = y;
            cv.fill(Rect::new(sb.x + self.px(2.0), y, sb.w - self.px(4.0), self.px(2.0).max(1)), MOD());
        }
    }
}
