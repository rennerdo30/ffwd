//! Find in the diff (`Ctrl+F` with the diff focused, FF-34): literal,
//! case-insensitive text in the shown diff. Every match is tinted, the
//! current one stronger; `Enter`/`↓` and `↑` step through them and scroll
//! the diff. The query stays when another file or commit is shown, and the
//! new diff is searched at once.
//!
//! The diff is already in memory and capped (20,000 lines of at most
//! 2,000 characters), so the search runs on the UI thread; it is redone
//! only when the query or the diff changes.

use super::*;
use ffwd_core::diff::LineKind;

pub struct DiffFind {
    pub input: TextField,
    /// The field has the keyboard (while the diff is focused).
    pub typing: bool,
    /// Every match in the shown diff: (line, occurrence within the line).
    pub hits: Vec<(u32, u32)>,
    pub cur: usize,
    /// What `hits` was computed for: the diff's identity and the query.
    key: (usize, usize, String),
    /// The field's place in the diff bar, for clicks.
    pub field: Rect,
}

/// Byte offsets of `needle` (already lowercase) in `hay`, ASCII
/// case-insensitive, not overlapping.
pub(super) fn occurrences(hay: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    hay.to_ascii_lowercase().match_indices(needle).map(|(i, _)| i).collect()
}

/// A diff line as painted: tabs widened to the next stop of `tab` columns.
pub(super) fn shown_text(t: &str, tab: usize) -> std::borrow::Cow<'_, str> {
    if !t.contains('\t') {
        return t.into();
    }
    let tab = tab.max(1);
    let mut out = String::with_capacity(t.len() + 8);
    let mut col = 0usize;
    for c in t.chars() {
        if c == '\t' {
            let n = tab - col % tab;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out.into()
}

fn searchable(k: LineKind) -> bool {
    matches!(k, LineKind::Add | LineKind::Del | LineKind::Context)
}

impl Ui {
    /// Gives the diff the keyboard; the old focus's colours repaint.
    fn focus_diff(&mut self) {
        if self.focus != Focus::Diff {
            self.focus = Focus::Diff;
            self.damage.rects.extend([self.lay.details, self.lay.side]);
            self.damage.rows.push(self.r.sel);
        }
    }

    pub(super) fn open_diff_find(&mut self) {
        self.focus_diff();
        match &mut self.diff_find {
            Some(f) => {
                f.typing = true;
                f.input.select_all();
            }
            None => {
                self.diff_find = Some(DiffFind {
                    input: TextField::default(),
                    typing: true,
                    hits: Vec::new(),
                    cur: 0,
                    key: (0, 0, String::new()),
                    field: Rect::default(),
                })
            }
        }
        self.damage.rects.push(self.diff_rect());
    }

    pub(super) fn close_diff_find(&mut self) {
        if self.diff_find.take().is_some() {
            self.damage.rects.push(self.diff_rect());
        }
    }

    /// Searches the shown diff again when the query or the diff changed;
    /// with `jump`, scrolls to the first match from the top of the view.
    pub(super) fn diff_find_refresh(&mut self, jump: bool) {
        let Some(f) = &mut self.diff_find else { return };
        let q = f.input.text().to_ascii_lowercase();
        let diff = self.r.diff.as_ref().filter(|d| !d.binary);
        let key = diff.map_or((0, 0, q.clone()), |d| (d.lines.as_ptr() as usize, d.lines.len(), q.clone()));
        if key == f.key {
            return;
        }
        f.key = key;
        f.hits.clear();
        f.cur = 0;
        let tab = self.diff_tab;
        if let Some(d) = diff {
            for (i, l) in d.lines.iter().enumerate().filter(|(_, l)| searchable(l.kind)) {
                let n = occurrences(&shown_text(&l.text, tab), &q).len();
                f.hits.extend((0..n as u32).map(|k| (i as u32, k)));
            }
        }
        self.damage.rects.push(self.diff_rect());
        if jump && !self.diff_find.as_ref().is_some_and(|f| f.hits.is_empty()) {
            // The first match at or below the top of the view.
            let top = self.diff_top_line();
            if let Some(f) = &mut self.diff_find {
                f.cur = f.hits.iter().position(|&(l, _)| l as usize >= top).unwrap_or(0);
            }
            self.scroll_to_diff_hit();
        }
    }

    /// A new diff arrived: search it (a new diff may reuse the old one's
    /// memory, so the cache key alone can't tell).
    pub(super) fn diff_find_on_new_diff(&mut self) {
        if let Some(f) = &mut self.diff_find {
            f.key = (usize::MAX, 0, String::new());
        }
        self.diff_find_refresh(true);
    }

    /// The diff line in the top row of the view.
    fn diff_top_line(&mut self) -> usize {
        let top = self.r.diff_top;
        let Some(d) = self.r.diff.clone() else { return 0 };
        if !self.diff_split {
            return top;
        }
        let (p, _) = self.diff_meta(&d);
        diff_view::split_rows_of(&d, &p).get(top).and_then(|&(l, r)| l.or(r)).unwrap_or(0)
    }

    /// Scrolls so the current match is in view (a third from the top).
    fn scroll_to_diff_hit(&mut self) {
        let Some(&(line, _)) = self.diff_find.as_ref().and_then(|f| f.hits.get(f.cur)) else { return };
        let line = line as usize;
        let Some(d) = self.r.diff.clone() else { return };
        let row = if self.diff_split {
            let (p, _) = self.diff_meta(&d);
            diff_view::split_rows_of(&d, &p).iter().position(|&(l, r)| l == Some(line) || r == Some(line)).unwrap_or(0)
        } else {
            line
        };
        let fit = ((self.diff_rect().h - self.px(32.0)).max(0) / self.code_lh().max(1)) as usize;
        if row < self.r.diff_top || row >= self.r.diff_top + fit.max(1) {
            self.r.diff_top = row.saturating_sub(fit / 3);
        }
        self.damage.rects.push(self.diff_rect());
    }

    pub(super) fn diff_find_step(&mut self, forward: bool) {
        let Some(f) = &mut self.diff_find else { return };
        let n = f.hits.len();
        if n == 0 {
            return;
        }
        f.cur = if forward { (f.cur + 1) % n } else { (f.cur + n - 1) % n };
        self.scroll_to_diff_hit();
    }

    /// Keys while the find field has the keyboard; false lets the key go on.
    pub(super) fn diff_find_key(&mut self, k: Key) -> bool {
        let Some(s) = &mut self.diff_find else { return false };
        let f = &mut s.input;
        match k {
            Key::Esc => {
                self.close_diff_find();
                return true;
            }
            Key::Enter | Key::Down => {
                self.diff_find_step(true);
                return true;
            }
            Key::Up => {
                self.diff_find_step(false);
                return true;
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
        self.damage.rects.push(self.diff_rect());
        self.diff_find_refresh(true);
        true
    }

    /// The match marks of diff line `i` (painted as `text`): byte range
    /// and colour, the current match stronger.
    pub(super) fn diff_find_marks(&self, i: usize, text: &str) -> Vec<(usize, usize, u32)> {
        let Some(f) = &self.diff_find else { return Vec::new() };
        let q = &f.key.2;
        if q.is_empty() || f.hits.is_empty() {
            return Vec::new();
        }
        let cur = f.hits.get(f.cur).copied();
        occurrences(text, q)
            .into_iter()
            .enumerate()
            .map(|(k, a)| {
                let on = cur == Some((i as u32, k as u32));
                (a, a + q.len(), if on { ffwd_render::mix(BG_PANEL, MOD, 120) } else { tint(MOD) })
            })
            .collect()
    }

    /// The find field and its count in the diff bar, left of the
    /// controls that end at `right`.
    pub(super) fn paint_diff_find(&mut self, cv: &mut Canvas, bar: Rect, right: f32) {
        let Some(mut s) = self.diff_find.take() else { return };
        let ui = self.f.ui;
        let fh = bar.h - self.px(8.0);
        let x0 = bar.x + self.px(10.0);
        let fw = (self.px(260.0)).min((right as i32 - x0 - self.px(110.0)).max(self.px(120.0)));
        let field = Rect::new(x0, bar.y + self.px(4.0), fw, fh);
        s.field = field;
        let typing = s.typing && self.focus == Focus::Diff;
        cv.round_border(field, self.pf(6.0), if typing { ACCENT } else { BORDER }, BG_INPUT);
        let base = (field.y as f32 + field.h as f32 / 2.0 + self.pf(4.0)).round();
        let tx = field.x as f32 + self.pf(8.0);
        let fs = self.pf(12.0);
        let maxw = field.w as f32 - self.pf(16.0);
        if s.input.text().is_empty() {
            self.te.draw(cv, ui, fs, tx, base, "Find in diff", TEXT_4, maxw);
        } else {
            if s.input.all_selected() {
                let w = self.te.measure(ui, fs, s.input.text()).min(maxw);
                cv.fill_round(
                    Rect::new(tx as i32 - 2, field.y + self.px(3.0), w as i32 + 4, field.h - self.px(6.0)),
                    self.pf(3.0),
                    BG_TEXT_SELECT,
                );
            }
            self.te.draw(cv, ui, fs, tx, base, s.input.text(), TEXT, maxw);
        }
        if typing {
            let cx = tx + self.te.measure(ui, fs, &s.input.text()[..s.input.cursor()]);
            cv.fill(
                Rect::new(cx as i32 + 1, field.y + self.px(4.0), self.px(1.5).max(1), field.h - self.px(8.0)),
                ACCENT,
            );
        }
        let count = match (s.input.text().is_empty(), s.hits.len()) {
            (true, _) => String::new(),
            (false, 0) => "No matches".into(),
            (false, n) => format!("{} of {}", fmt::num(s.cur + 1), fmt::num(n)),
        };
        let c = if s.hits.is_empty() { MOD } else { TEXT_2 };
        self.te.draw(cv, ui, self.pf(11.5), field.right() as f32 + self.pf(10.0), base, &count, c, self.pf(100.0));
        self.diff_find = Some(s);
    }

    /// A click on the find field gives it the keyboard.
    pub(super) fn diff_find_click(&mut self, x: i32, y: i32) -> bool {
        let Some(f) = &mut self.diff_find else { return false };
        if !f.field.contains(x, y) {
            return false;
        }
        f.typing = true;
        self.focus_diff();
        self.damage.rects.push(self.diff_rect());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occurrences_are_case_insensitive_and_byte_exact() {
        assert_eq!(occurrences("Foo foo FOO", "foo"), vec![0, 4, 8]);
        assert_eq!(occurrences("ä foo", "foo"), vec![3], "byte offsets past multi-byte text");
        assert_eq!(occurrences("aaaa", "aa"), vec![0, 2], "not overlapping");
        assert!(occurrences("abc", "").is_empty());
        assert_eq!(shown_text("a\tb", 4), "a   b", "to the next stop");
        assert_eq!(shown_text("\tx", 2), "  x");
        assert_eq!(shown_text("abcd\t", 4), "abcd    ");
        assert_eq!(shown_text("no tabs", 8), "no tabs");
    }
}
