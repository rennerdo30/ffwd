//! The diff viewer's options and painters (FF-41): unified or side by
//! side, context size, whitespace, syntax colours and word highlights.
//! Only the lines on screen are tokenized; the line pairing is computed
//! once per diff and cached.

use super::menu::{MenuAct, MenuItem};
use super::*;
use crate::highlight::{self, Kind, Lang};
use ffwd_core::diff::DiffOpts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffBar {
    Unified,
    Split,
    Context(u32),
    Whitespace,
    /// The view menu (`…`): whitespace characters, tab width.
    More,
    /// Previous and next change (`p`, `n`), find in the diff (`Ctrl+F`).
    Prev,
    Next,
    Find,
    /// The large view on or off (Space).
    Max,
}

/// The tab widths the view menu offers.
pub const TAB_WIDTHS: [u8; 3] = [2, 4, 8];

/// Code text sizes the view menu steps through (points; 12 is the default).
pub const CODE_SIZES: [u8; 6] = [10, 11, 12, 13, 14, 16];

/// The code text size from settings (`diff.font_size`).
pub fn code_size_from(settings: &crate::settings::Settings) -> u8 {
    settings.get("diff.font_size").and_then(|t| t.parse::<u8>().ok()).filter(|t| CODE_SIZES.contains(t)).unwrap_or(12)
}

/// Whether long diff lines wrap (`diff.wrap`; unified view).
pub fn wrap_from(settings: &crate::settings::Settings) -> bool {
    settings.get("diff.wrap") == Some("true")
}

/// Whether whitespace characters are shown, and the tab width.
pub fn view_from(settings: &crate::settings::Settings) -> (bool, u8) {
    let ws = settings.get("diff.show_whitespace") == Some("true");
    let tab = settings.get("diff.tab_width").and_then(|t| t.parse::<u8>().ok()).filter(|t| (1..=16).contains(t));
    (ws, tab.unwrap_or(4))
}

/// The context sizes the bar offers; `u32::MAX` is the whole file.
const CONTEXTS: [(u32, &str); 3] = [(3, "3 lines"), (10, "10 lines"), (u32::MAX, "Whole file")];

pub fn opts_from(settings: &crate::settings::Settings) -> (bool, DiffOpts) {
    let split = settings.get("diff.layout") == Some("split");
    let context = match settings.get("diff.context") {
        Some("all") => u32::MAX,
        Some(n) => n.parse().unwrap_or(3).clamp(0, 10_000),
        None => 3,
    };
    let ignore_whitespace = settings.get("diff.ignore_whitespace") == Some("true");
    (split, DiffOpts { context, ignore_whitespace })
}

impl Ui {
    /// The line pairing of `d` and, per line, whether it starts inside a
    /// block comment; computed once per diff and cached.
    pub(super) fn diff_meta(&mut self, d: &FileDiff) -> (std::rc::Rc<Vec<u32>>, std::rc::Rc<Vec<bool>>) {
        let key = (d.lines.as_ptr() as usize, d.lines.len());
        if let Some((k, p, b)) = &self.pairs_cache
            && *k == key
        {
            return (p.clone(), b.clone());
        }
        let pairs = std::rc::Rc::new(ffwd_core::diff::pair_lines(d));
        let mut blocks = Vec::with_capacity(d.lines.len());
        let lang = highlight::lang_for(&d.path);
        let mut st = false;
        for line in &d.lines {
            blocks.push(st);
            match (lang, line.kind) {
                (_, LineKind::Hunk) => st = false,
                (Some(l), LineKind::Add | LineKind::Del | LineKind::Context) => {
                    st = highlight::ends_in_block(l, &line.text, st)
                }
                _ => {}
            }
        }
        let blocks = std::rc::Rc::new(blocks);
        self.pairs_cache = Some((key, pairs.clone(), blocks.clone()));
        (pairs, blocks)
    }

    /// Draws one line of code: backgrounds under parts of it (`marks`:
    /// byte ranges and colours, such as the changed words and find
    /// matches), then the text in syntax colours.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_code(
        &mut self,
        cv: &mut Canvas,
        x: f32,
        base: f32,
        row: Rect,
        text: &str,
        fg: u32,
        maxw: f32,
        lang: Option<&Lang>,
        in_block: bool,
        marks: &[(usize, usize, u32)],
    ) {
        let mono = self.f.mono;
        let fs = self.code_fs();
        for &(a, b, c) in marks {
            if b <= a || b > text.len() || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
                continue;
            }
            let x0 = x + self.te.measure(mono, fs, &text[..a]);
            if x0 >= x + maxw {
                continue;
            }
            let x1 = (x + self.te.measure(mono, fs, &text[..b])).min(x + maxw);
            let r = Rect::new(x0 as i32, row.y + self.px(2.0), (x1 - x0).ceil().max(2.0) as i32, row.h - self.px(4.0));
            cv.fill_round(r, self.pf(3.0), c);
        }
        let Some(l) = lang else {
            self.te.draw(cv, mono, fs, x, base, text, fg, maxw);
            return;
        };
        let mut pen = x;
        let end = x + maxw;
        for (a, b, k) in highlight::spans(l, text, in_block) {
            if pen >= end {
                break;
            }
            let c = match k {
                Kind::Plain => fg,
                Kind::Keyword => SYN_KEYWORD(),
                Kind::Type => SYN_TYPE(),
                Kind::String => SYN_STRING(),
                Kind::Number => SYN_NUMBER(),
                Kind::Comment => SYN_COMMENT(),
            };
            pen = self.te.draw(cv, mono, fs, pen, base, &text[a..b], c, end - pen);
        }
    }

    /// The word highlight of line `i`: its changed range against its pair.
    pub(super) fn word_range(d: &FileDiff, pairs: &[u32], i: usize) -> Option<(usize, usize, u32)> {
        let p = *pairs.get(i)? as usize;
        let line = d.lines.get(i)?;
        let other = d.lines.get(p)?;
        let (a, b) = ffwd_core::diff::changed_range(&line.text, &other.text);
        let c = if line.kind == LineKind::Add { ADD_WORD() } else { DEL_WORD() };
        // Only a small change inside a line gets a word highlight; when
        // most of the line differs, the line colour says it all.
        let len = line.text.trim().len().max(1);
        (b > a && (b - a) * 10 <= len * 6).then_some((a, b, c))
    }

    /// Side-by-side rows: (left line, right line); a hunk header has the
    /// same line on both sides.
    pub(super) fn split_rows(d: &FileDiff, pairs: &[u32]) -> Vec<(Option<usize>, Option<usize>)> {
        let mut rows = Vec::with_capacity(d.lines.len());
        let mut i = 0;
        while i < d.lines.len() {
            match d.lines[i].kind {
                LineKind::Del => {
                    let p = pairs[i];
                    rows.push((Some(i), (p != u32::MAX).then_some(p as usize)));
                }
                LineKind::Add if pairs[i] != u32::MAX => {} // drawn with its deletion
                LineKind::Add => rows.push((None, Some(i))),
                LineKind::Meta => {}
                _ => rows.push((Some(i), Some(i))),
            }
            i += 1;
        }
        rows
    }

    /// Side by side from row `top`. With `hunk_label` each hunk gets a
    /// stage or unstage button (Local Changes); returns (hunk, rect).
    pub(super) fn paint_diff_split(
        &mut self,
        cv: &mut Canvas,
        body: Rect,
        diff: &FileDiff,
        top: usize,
        hunk_label: Option<&str>,
        hit_base: usize,
    ) -> Vec<(usize, Rect)> {
        let (mono, semi) = (self.f.mono, self.f.semi);
        let lh = self.code_lh();
        let (pairs, blocks) = self.diff_meta(diff);
        let rows = Self::split_rows(diff, &pairs);
        let mut buttons = Vec::new();
        // Hunks above the first painted row count too.
        let mut hunk =
            rows.iter().take(top).filter(|(l, _)| l.is_some_and(|i| diff.lines[i].kind == LineKind::Hunk)).count();
        let lang = highlight::lang_for(&diff.path);
        let half = body.w / 2;
        let num_w = self.code_px(40.0);
        let clip = cv.clip();
        let fit = (body.h / lh + 1) as usize;
        // Divider between the sides.
        cv.fill(Rect::new(body.x + half, body.y, 1, body.h), BORDER());
        for (k, &(l, r)) in rows.iter().enumerate().skip(top).take(fit) {
            let y = body.y + (k - top) as i32 * lh;
            let row = Rect::new(body.x, y, body.w, lh);
            if clip.intersect(row).is_empty() {
                continue;
            }
            let lb = (y as f32 + lh as f32 / 2.0 + self.code_px(4.0)).round();
            if let Some(i) = l
                && diff.lines[i].kind == LineKind::Hunk
            {
                cv.fill(row, HUNK_BG());
                hunk += 1;
                let mut text_w = body.w as f32 - self.pf(20.0);
                if let Some(label) = hunk_label {
                    let w = self.te.measure(semi, self.pf(11.0), label) + self.pf(18.0);
                    let b = Rect::new(
                        body.right() - self.px(16.0) - w as i32,
                        y + self.px(2.0),
                        w as i32,
                        lh - self.px(4.0),
                    );
                    let hot = Hot::Change(hit_base + buttons.len());
                    let (bg, border) = self.control_colors(hot, BG_BAR(), HUNK_BORDER());
                    cv.round_border(b, self.pf(5.0), border, bg);
                    self.te.draw(cv, semi, self.pf(11.0), b.x as f32 + self.pf(9.0), lb, label, HUNK_TEXT(), w);
                    buttons.push((hunk - 1, b));
                    text_w -= w + self.pf(12.0);
                }
                let t = diff.lines[i].text.clone();
                self.te.draw(cv, mono, self.code_px(11.5), body.x as f32 + self.pf(10.0), lb, &t, HUNK_TEXT(), text_w);
                continue;
            }
            for (side, li) in [(0, l), (1, r)] {
                let x0 = body.x + side * half + side;
                let cell = Rect::new(x0, y, half - 1, lh);
                let Some(i) = li else {
                    // Nothing on this side: a quiet hatch-free fill.
                    cv.fill(cell, BG_SIDEBAR());
                    continue;
                };
                let line = &diff.lines[i];
                let (bg, fg, gut) = match (line.kind, side) {
                    (LineKind::Del, _) => (DEL_BG(), DEL_TEXT(), DIFF_DEL_LINE()),
                    (LineKind::Add, _) => (ADD_BG(), ADD_TEXT(), DIFF_ADD_LINE()),
                    _ => (BG(), TEXT_2(), BG_SIDEBAR()),
                };
                cv.fill(cell, bg);
                cv.fill(Rect::new(x0, y, num_w as i32, lh), gut);
                let n = if side == 0 { line.old } else { line.new };
                if n > 0 {
                    let s = n.to_string();
                    let w = self.te.measure(mono, self.code_px(11.0), &s);
                    let nc = match line.kind {
                        LineKind::Add => ADD_GUTTER(),
                        LineKind::Del => DEL_GUTTER(),
                        _ => TEXT_4(),
                    };
                    self.te.draw(cv, mono, self.code_px(11.0), x0 as f32 + num_w - w - self.pf(6.0), lb, &s, nc, num_w);
                }
                let text = super::diff_find::shown_text(&line.text, self.diff_tab).into_owned();
                let mut marks: Vec<_> =
                    Self::word_range(diff, &pairs, i).filter(|_| !line.text.contains('\t')).into_iter().collect();
                marks.extend(self.diff_find_marks(i, &text));
                let tx = x0 as f32 + num_w + self.pf(8.0);
                let st = blocks.get(i).copied().unwrap_or(false);
                // Code scrolls sideways inside its half; numbers stay.
                let left = self.code_left;
                let code = Rect::new(tx as i32, cell.y, (cell.right() as f32 - tx) as i32, cell.h);
                let old = cv.set_clip(clip.intersect(code));
                let maxw = (cell.right() as f32) - tx - self.pf(8.0);
                self.draw_code(cv, tx - left, lb, cell, &text, fg, maxw + left, lang, st, &marks);
                if self.diff_show_ws {
                    let raw = line.text.clone();
                    self.draw_whitespace(cv, tx - left, lb, &raw, maxw + left);
                }
                cv.set_clip(old);
            }
        }
        buttons
    }

    /// The split row that shows line `i` (either side).
    pub(super) fn split_row_of(rows: &[(Option<usize>, Option<usize>)], i: usize) -> usize {
        rows.iter().position(|&(l, r)| l == Some(i) || r == Some(i)).unwrap_or_else(|| {
            // A line without a row of its own (meta): the next row.
            rows.iter().position(|&(l, r)| l.or(r).is_some_and(|k| k > i)).unwrap_or(rows.len().saturating_sub(1))
        })
    }

    /// The line under (x, y) in a split body painted from row `top`: the
    /// side under the pointer, else the other one.
    pub(super) fn split_line_at(&mut self, diff: &FileDiff, top: usize, body: Rect, x: i32, y: i32) -> Option<usize> {
        if y < body.y {
            return None;
        }
        let (p, _) = self.diff_meta(diff);
        let rows = Self::split_rows(diff, &p);
        let &(l, r) = rows.get(top + ((y - body.y) / self.code_lh().max(1)) as usize)?;
        if x < body.x + body.w / 2 { l.or(r) } else { r.or(l) }
    }

    /// Where line `i` sits in a split body painted from row `top`: its
    /// half's left edge, y and height, if it is on screen.
    pub(super) fn split_line_rect(&mut self, diff: &FileDiff, top: usize, body: Rect, i: usize) -> Option<Rect> {
        let (p, _) = self.diff_meta(diff);
        let rows = Self::split_rows(diff, &p);
        let k = rows.iter().position(|&(l, r)| l == Some(i) || r == Some(i))?;
        let lh = self.code_lh();
        let y = body.y + (k as i32 - top as i32) * lh;
        if k < top || y >= body.bottom() {
            return None;
        }
        let half = body.w / 2;
        let right = rows[k].0 != Some(i);
        Some(Rect::new(if right { body.x + half + 1 } else { body.x }, y, half - 1, lh))
    }

    /// The rows a diff has in the current layout (for scrolling).
    /// The rows where a block of changes starts (unified or split).
    pub(super) fn change_starts(&mut self) -> Vec<usize> {
        use ffwd_core::diff::LineKind;
        let Some(d) = self.r.diff.clone() else { return Vec::new() };
        let changed = |i: usize| matches!(d.lines[i].kind, LineKind::Add | LineKind::Del);
        let rows: Vec<bool> = if self.diff_split {
            let (p, _) = self.diff_meta(&d);
            Self::split_rows(&d, &p)
                .into_iter()
                .map(|(l, r)| l.is_some_and(changed) || r.is_some_and(changed))
                .collect()
        } else {
            (0..d.lines.len()).map(changed).collect()
        };
        (0..rows.len()).filter(|&i| rows[i] && (i == 0 || !rows[i - 1])).collect()
    }

    /// Scrolls to the next (or previous) block of changes, with three
    /// lines of context above it (FF-41).
    pub(super) fn jump_change(&mut self, next: bool) {
        const CONTEXT: usize = 3;
        let starts = self.change_starts();
        let pivot = self.r.diff_top + CONTEXT;
        let target = if next {
            starts.iter().copied().find(|&s| s > pivot)
        } else {
            starts.iter().rev().copied().find(|&s| s < pivot)
        };
        match target {
            Some(s) => {
                self.r.diff_top = s.saturating_sub(CONTEXT);
                self.damage.rects.push(self.diff_rect());
            }
            None => self.set_notice(if next { "No more changes below".into() } else { "No more changes above".into() }),
        }
    }

    pub(super) fn diff_row_count(&mut self) -> usize {
        let Some(d) = self.r.diff.clone() else { return 0 };
        if self.diff_split {
            let (p, _) = self.diff_meta(&d);
            Self::split_rows(&d, &p).len()
        } else {
            d.lines.len()
        }
    }

    /// The bar's controls, drawn from the right edge; returns their left edge.
    pub(super) fn paint_diff_bar_controls(&mut self, cv: &mut Canvas, bar: Rect) -> f32 {
        let (ui, semi) = (self.f.ui, self.f.semi);
        let mut hits = Vec::new();
        let h = bar.h - self.px(8.0);
        let y = bar.y + self.px(4.0);
        let base = (bar.y as f32 + bar.h as f32 / 2.0 + self.pf(4.0)).round();
        let mut x = bar.right() - self.px(10.0);
        let seg = |ui_: &mut Ui,
                   cv: &mut Canvas,
                   x: &mut i32,
                   label: &str,
                   on: bool,
                   hit: DiffBar,
                   hits: &mut Vec<(Rect, DiffBar)>| {
            let w = (ui_.te.measure(semi, ui_.pf(11.5), label) + ui_.pf(16.0)) as i32;
            let r = Rect::new(*x - w, y, w, h);
            if on {
                cv.fill_round(r, ui_.pf(5.0), BG_CHOSEN());
            } else if ui_.is_hot(Hot::DiffBar(hits.len())) {
                cv.fill_round(r, ui_.pf(5.0), CONTROL_HOVER());
            }
            ui_.te.draw(
                cv,
                semi,
                ui_.pf(11.5),
                r.x as f32 + ui_.pf(8.0),
                base,
                label,
                if on { TEXT() } else { TEXT_2() },
                w as f32,
            );
            hits.push((r, hit));
            *x = r.x - ui_.px(2.0);
        };
        seg(self, cv, &mut x, "…", false, DiffBar::More, &mut hits);
        x -= self.px(4.0);
        // Icon buttons first (the click twins of p, n and Ctrl+F; FF-57): on a
        // narrow pane the wider text options drop off before them.
        let max = if self.diff_max { super::Glyph::Collapse } else { super::Glyph::Expand };
        for (g, hit) in [
            (max, DiffBar::Max),
            (super::Glyph::Search, DiffBar::Find),
            (super::Glyph::Down, DiffBar::Next),
            (super::Glyph::Up, DiffBar::Prev),
        ] {
            let r = Rect::new(x - h, y, h, h);
            let on = hit == DiffBar::Find && self.diff_find.is_some();
            if on {
                cv.fill_round(r, self.pf(5.0), BG_CHOSEN());
            } else if self.is_hot(Hot::DiffBar(hits.len())) {
                cv.fill_round(r, self.pf(5.0), CONTROL_HOVER());
            }
            let c = if on { TEXT() } else { TEXT_2() };
            self.glyph(cv, g, r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0, self.pf(6.5), c);
            hits.push((r, hit));
            x = r.x - self.px(2.0);
        }
        // The text options come in groups, and a group shows whole or not
        // at all: on a narrow pane the context presets go first, then the
        // whitespace switch; Unified / Side by side stays longest.
        let ws = self.diff_opts.ignore_whitespace;
        let ws_label = if ws { "Whitespace ignored" } else { "Whitespace shown" };
        let segw =
            |ui_: &mut Ui, label: &str| (ui_.te.measure(semi, ui_.pf(11.5), label) + ui_.pf(16.0)) as i32 + ui_.px(2.0);
        let sep = self.px(11.0);
        let w_ws = segw(self, ws_label) + sep;
        let w_ctx = CONTEXTS.iter().map(|&(_, l)| segw(self, l)).sum::<i32>() + sep;
        let w_split = segw(self, "Side by side") + segw(self, "Unified") + sep;
        // Room for the controls, keeping some for the file name on the left.
        let mut room = x - bar.x - self.px(60.0);
        let mut fits = |w: i32| {
            let ok = room >= w;
            if ok {
                room -= w;
            }
            ok
        };
        let (show_split, show_ws, show_ctx) = (fits(w_split), fits(w_ws), fits(w_ctx));
        let divider = |ui_: &Ui, cv: &mut Canvas, x: &mut i32| {
            *x -= ui_.px(5.0);
            cv.fill(Rect::new(*x, y + ui_.px(4.0), 1, h - ui_.px(8.0)), BORDER());
            *x -= ui_.px(5.0);
        };
        if show_ws {
            divider(self, cv, &mut x);
            seg(self, cv, &mut x, ws_label, ws, DiffBar::Whitespace, &mut hits);
        }
        if show_ctx {
            divider(self, cv, &mut x);
            for &(n, label) in CONTEXTS.iter().rev() {
                let on = self.diff_opts.context == n;
                seg(self, cv, &mut x, label, on, DiffBar::Context(n), &mut hits);
            }
        }
        if show_split {
            divider(self, cv, &mut x);
            let split = self.diff_split;
            seg(self, cv, &mut x, "Side by side", split, DiffBar::Split, &mut hits);
            seg(self, cv, &mut x, "Unified", !split, DiffBar::Unified, &mut hits);
        }
        let _ = ui;
        // Controls pushed past the pane's left edge are clipped away; they
        // must not take clicks there either.
        hits.retain(|(r, _)| r.x >= bar.x);
        self.diff_bar_hits = hits;
        x as f32
    }

    pub(super) fn diff_bar_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.diff_bar_hits.iter().position(|h| h.0.contains(x, y))
    }

    pub(super) fn diff_bar_click(&mut self, x: i32, y: i32) -> bool {
        let Some(i) = self.diff_bar_hit_at(x, y) else { return false };
        let (r, h) = self.diff_bar_hits[i];
        if h == DiffBar::More {
            let items = self.diff_view_menu();
            self.open_menu(r.x, r.bottom(), items);
            return true;
        }
        self.apply_diff_bar(h);
        true
    }

    /// Code text size in pixels (diffs and the file viewer).
    pub(super) fn code_fs(&self) -> f32 {
        self.pf(self.code_pt as f32)
    }

    /// A length that scales with the code text (12 pt is 1:1).
    pub(super) fn code_px(&self, v: f32) -> f32 {
        self.pf(v * self.code_pt as f32 / 12.0)
    }

    /// The height of a line of code.
    pub(super) fn code_lh(&self) -> i32 {
        self.px((20.0 * self.code_pt as f32 / 12.0).round())
    }

    /// Sets the code text size (FF-41): one of [`CODE_SIZES`].
    pub fn set_code_size(&mut self, pt: u8) {
        if !CODE_SIZES.contains(&pt) || pt == self.code_pt {
            return;
        }
        self.code_pt = pt;
        self.settings.set_string("diff.font_size", &pt.to_string());
        // Rows change height in every code pane (the same line stays at
        // the top): one full repaint.
        self.damage.full = true;
    }

    /// The code column of a unified diff body: its left edge (where text
    /// starts) and the width text may use before it wraps, a whole number
    /// of code characters so rows break between glyphs.
    pub(super) fn code_column(&mut self, body: Rect) -> (f32, f32) {
        let gut_w = (self.code_px(38.0) * 2.0 + self.pf(8.0)) as i32;
        let tx = (body.x + gut_w) as f32 + self.pf(8.0);
        // Text starts 14 px after `tx` and ends clear of the overview strip.
        let w = (body.right() as f32 - tx - self.pf(14.0) - self.pf(24.0)).max(self.pf(40.0));
        let cw = self.te.measure(self.f.mono, self.code_fs(), "M").max(1.0);
        (tx + self.pf(14.0), ((w / cw).floor().max(4.0)) * cw)
    }

    /// How many rows diff line `i` takes in `body` (FF-41 word wrap):
    /// one, or with wrapping on (unified view) as many as its text needs.
    pub(super) fn diff_rows(&mut self, diff: &FileDiff, i: usize, body: Rect) -> usize {
        let Some(line) = diff.lines.get(i) else { return 1 };
        if !self.diff_wrap || !matches!(line.kind, LineKind::Add | LineKind::Del | LineKind::Context) {
            return 1;
        }
        let per = self.wrap_chars(body);
        let n = super::diff_find::shown_text(&line.text, self.diff_tab).chars().count();
        n.div_ceil(per).clamp(1, 64)
    }

    /// Characters per wrapped row in `body`.
    pub(super) fn wrap_chars(&mut self, body: Rect) -> usize {
        let (_, w) = self.code_column(body);
        let cw = self.te.measure(self.f.mono, self.code_fs(), "M").max(1.0);
        ((w / cw).round() as usize).max(4)
    }

    /// The line at screen `y` in a unified body painted from line `top`,
    /// walking rows the way the painter does.
    pub(super) fn diff_line_at_y(&mut self, diff: &FileDiff, top: usize, body: Rect, y: i32) -> Option<usize> {
        if y < body.y {
            return None;
        }
        let lh = self.code_lh().max(1);
        if !self.diff_wrap {
            let i = top + ((y - body.y) / lh) as usize;
            return (i < diff.lines.len()).then_some(i);
        }
        let mut yy = body.y;
        for i in top..diff.lines.len() {
            let h = lh * self.diff_rows(diff, i, body) as i32;
            if y < yy + h {
                return Some(i);
            }
            yy += h;
            if yy >= body.bottom() {
                break;
            }
        }
        None
    }

    /// Where line `i` sits in a unified body painted from `top`: its y and
    /// height, if it is on screen.
    pub(super) fn diff_line_y(&mut self, diff: &FileDiff, top: usize, body: Rect, i: usize) -> Option<(i32, i32)> {
        if i < top {
            return None;
        }
        let lh = self.code_lh();
        let mut y = body.y;
        for k in top..=i.min(diff.lines.len().saturating_sub(1)) {
            let h = lh * self.diff_rows(diff, k, body) as i32;
            if k == i {
                return (y < body.bottom()).then_some((y, h));
            }
            y += h;
            if y >= body.bottom() {
                return None;
            }
        }
        None
    }

    /// Turns long-line wrapping on or off.
    pub(super) fn set_diff_wrap(&mut self, on: bool) {
        self.diff_wrap = on;
        self.settings.set_string("diff.wrap", if on { "true" } else { "false" });
        self.damage.full = true;
    }

    /// Shows or hides whitespace characters, or sets the tab width.
    pub(super) fn set_diff_view(&mut self, show_ws: Option<bool>, tab: Option<u8>) {
        if let Some(on) = show_ws {
            self.diff_show_ws = on;
            self.settings.set_string("diff.show_whitespace", if on { "true" } else { "false" });
        }
        if let Some(t) = tab {
            self.diff_tab = t as usize;
            self.settings.set_string("diff.tab_width", &t.to_string());
            // Match positions move with the tab stops.
            self.diff_find_on_new_diff();
        }
        self.damage.rects.push(self.diff_rect());
    }

    /// Draws `·` for spaces and `→` for tabs over a line painted from `x`
    /// (its raw text, tabs at stops of `diff_tab`), dimmed.
    pub(super) fn draw_whitespace(&mut self, cv: &mut Canvas, x: f32, base: f32, raw: &str, maxw: f32) {
        if !raw.contains([' ', '\t']) {
            return;
        }
        let mono = self.f.mono;
        let fs = self.code_fs();
        let sp = self.te.measure(mono, fs, " ");
        let tab = self.diff_tab.max(1);
        let end = x + maxw;
        let (mut pen, mut col, mut run) = (x, 0usize, 0usize);
        let mut iter = raw.char_indices().peekable();
        while let Some((i, c)) = iter.next() {
            if c != ' ' && c != '\t' {
                col += 1;
                // A run of other characters is measured at its end.
                if iter.peek().is_none_or(|&(_, n)| n == ' ' || n == '\t') {
                    let stop = iter.peek().map_or(raw.len(), |&(j, _)| j);
                    pen += self.te.measure(mono, fs, &raw[run..stop]);
                }
                continue;
            }
            if pen >= end {
                break;
            }
            let n = if c == '\t' { tab - col % tab } else { 1 };
            let mark = if c == '\t' { "→" } else { "·" };
            let w = self.te.measure(mono, fs, mark);
            let at = if c == '\t' { pen + self.pf(1.0) } else { pen + (sp - w) / 2.0 };
            self.te.draw(cv, mono, fs, at, base, mark, WS_MARK(), end - at);
            pen += sp * n as f32;
            col += n;
            run = i + c.len_utf8();
        }
    }

    /// The diff's view menu (the bar's `…`).
    pub(super) fn diff_view_menu(&self) -> Vec<MenuItem> {
        let mark = |on: bool, t: &str| if on { format!("✓ {t}") } else { t.to_string() };
        let item = |label: String, act: MenuAct| MenuItem { label, act: Some(act), danger: false };
        // The bar's layout, context and whitespace choices too: they are the
        // first to drop off a narrow bar.
        let mut v = vec![item(
            mark(self.diff_split, "Side by side"),
            MenuAct::DiffBar(if self.diff_split { DiffBar::Unified } else { DiffBar::Split }),
        )];
        for &(n, label) in CONTEXTS.iter() {
            v.push(item(
                mark(self.diff_opts.context == n, &format!("Context: {label}")),
                MenuAct::DiffBar(DiffBar::Context(n)),
            ));
        }
        v.push(item(
            mark(self.diff_opts.ignore_whitespace, "Ignore whitespace"),
            MenuAct::DiffBar(DiffBar::Whitespace),
        ));
        v.push(MenuItem { label: String::new(), act: None, danger: false });
        v.extend([
            item(mark(self.diff_wrap, "Wrap long lines"), MenuAct::DiffWrap(!self.diff_wrap)),
            item(mark(self.diff_show_ws, "Show whitespace characters"), MenuAct::DiffShowWs(!self.diff_show_ws)),
        ]);
        v.push(MenuItem { label: String::new(), act: None, danger: false });
        for t in TAB_WIDTHS {
            v.push(item(mark(self.diff_tab == t as usize, &format!("Tab width {t}")), MenuAct::DiffTab(t)));
        }
        v.push(MenuItem { label: String::new(), act: None, danger: false });
        let at = CODE_SIZES.iter().position(|&s| s == self.code_pt).unwrap_or(2);
        if let Some(&s) = CODE_SIZES.get(at + 1) {
            v.push(item(format!("Larger text ({s} pt)"), MenuAct::CodeSize(s)));
        }
        if let Some(&s) = at.checked_sub(1).and_then(|i| CODE_SIZES.get(i)) {
            v.push(item(format!("Smaller text ({s} pt)"), MenuAct::CodeSize(s)));
        }
        if self.code_pt != 12 {
            v.push(item("Default text size (12 pt)".to_string(), MenuAct::CodeSize(12)));
        }
        v
    }

    pub(super) fn apply_diff_bar(&mut self, h: DiffBar) {
        match h {
            DiffBar::More => {}
            DiffBar::Max => self.toggle_diff_max(),
            DiffBar::Prev => self.jump_change(false),
            DiffBar::Next => self.jump_change(true),
            DiffBar::Find => {
                if self.diff_find.is_some() {
                    self.close_diff_find();
                } else {
                    self.open_diff_find();
                }
            }
            DiffBar::Unified | DiffBar::Split => {
                self.diff_split = h == DiffBar::Split;
                self.r.diff_top = 0;
                self.settings.set_string("diff.layout", if self.diff_split { "split" } else { "unified" });
            }
            DiffBar::Context(n) => {
                self.diff_opts.context = n;
                let v = if n == u32::MAX { "all".to_string() } else { n.to_string() };
                self.settings.set_string("diff.context", &v);
                self.request_diff();
            }
            DiffBar::Whitespace => {
                self.diff_opts.ignore_whitespace = !self.diff_opts.ignore_whitespace;
                let v = if self.diff_opts.ignore_whitespace { "true" } else { "false" };
                self.settings.set_string("diff.ignore_whitespace", v);
                self.request_diff();
                if self.r.changes_view {
                    // Local Changes shows the same choice.
                    self.request_change_diff();
                    self.damage.full = true;
                }
            }
        }
        let r = self.diff_rect();
        self.damage.rects.push(r);
    }
}

/// The side-by-side rows of a diff (left, right line), for other views.
pub(super) fn split_rows_of(d: &FileDiff, pairs: &[u32]) -> Vec<(Option<usize>, Option<usize>)> {
    Ui::split_rows(d, pairs)
}
