//! The diff viewer's options and painters (FF-41): unified or side by
//! side, context size, whitespace, syntax colours and word highlights.
//! Only the lines on screen are tokenized; the line pairing is computed
//! once per diff and cached.

use super::*;
use crate::highlight::{self, Kind, Lang};
use ffwd_core::diff::DiffOpts;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffBar {
    Unified,
    Split,
    Context(u32),
    Whitespace,
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

    /// Draws one line of code: the changed part's background (`word`,
    /// a byte range and its colour), then the text in syntax colours.
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
        word: Option<(usize, usize, u32)>,
    ) {
        let mono = self.f.mono;
        let fs = self.pf(12.0);
        if let Some((a, b, c)) = word
            && b > a
            && b <= text.len()
        {
            let x0 = x + self.te.measure(mono, fs, &text[..a]);
            let x1 = x + self.te.measure(mono, fs, &text[..b]);
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
                Kind::Keyword => SYN_KEYWORD,
                Kind::Type => SYN_TYPE,
                Kind::String => SYN_STRING,
                Kind::Number => SYN_NUMBER,
                Kind::Comment => SYN_COMMENT,
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
        let c = if line.kind == LineKind::Add { ADD_WORD } else { DEL_WORD };
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

    pub(super) fn paint_diff_split(&mut self, cv: &mut Canvas, body: Rect, diff: &FileDiff, top: usize) {
        let mono = self.f.mono;
        let lh = self.px(20.0);
        let (pairs, blocks) = self.diff_meta(diff);
        let rows = Self::split_rows(diff, &pairs);
        let lang = highlight::lang_for(&diff.path);
        let half = body.w / 2;
        let num_w = self.pf(40.0);
        let clip = cv.clip();
        let fit = (body.h / lh + 1) as usize;
        // Divider between the sides.
        cv.fill(Rect::new(body.x + half, body.y, 1, body.h), BORDER);
        for (k, &(l, r)) in rows.iter().enumerate().skip(top).take(fit) {
            let y = body.y + (k - top) as i32 * lh;
            let row = Rect::new(body.x, y, body.w, lh);
            if clip.intersect(row).is_empty() {
                continue;
            }
            let lb = (y as f32 + lh as f32 / 2.0 + self.pf(4.0)).round();
            if let Some(i) = l
                && diff.lines[i].kind == LineKind::Hunk
            {
                cv.fill(row, HUNK_BG);
                let t = diff.lines[i].text.clone();
                self.te.draw(cv, mono, self.pf(11.5), body.x as f32 + self.pf(10.0), lb, &t, HUNK_TEXT, body.w as f32);
                continue;
            }
            for (side, li) in [(0, l), (1, r)] {
                let x0 = body.x + side * half + side;
                let cell = Rect::new(x0, y, half - 1, lh);
                let Some(i) = li else {
                    // Nothing on this side: a quiet hatch-free fill.
                    cv.fill(cell, BG_SIDEBAR);
                    continue;
                };
                let line = &diff.lines[i];
                let (bg, fg, gut) = match (line.kind, side) {
                    (LineKind::Del, _) => (DEL_BG, DEL_TEXT, DIFF_DEL_LINE),
                    (LineKind::Add, _) => (ADD_BG, ADD_TEXT, DIFF_ADD_LINE),
                    _ => (BG, TEXT_2, BG_SIDEBAR),
                };
                cv.fill(cell, bg);
                cv.fill(Rect::new(x0, y, num_w as i32, lh), gut);
                let n = if side == 0 { line.old } else { line.new };
                if n > 0 {
                    let s = n.to_string();
                    let w = self.te.measure(mono, self.pf(11.0), &s);
                    let nc = match line.kind {
                        LineKind::Add => ADD_GUTTER,
                        LineKind::Del => DEL_GUTTER,
                        _ => TEXT_4,
                    };
                    self.te.draw(cv, mono, self.pf(11.0), x0 as f32 + num_w - w - self.pf(6.0), lb, &s, nc, num_w);
                }
                let text = line.text.replace('\t', "    ");
                let word = Self::word_range(diff, &pairs, i).filter(|_| !line.text.contains('\t'));
                let tx = x0 as f32 + num_w + self.pf(8.0);
                let st = blocks.get(i).copied().unwrap_or(false);
                let old = cv.set_clip(clip.intersect(cell));
                self.draw_code(cv, tx, lb, cell, &text, fg, (cell.right() as f32) - tx - self.pf(8.0), lang, st, word);
                cv.set_clip(old);
            }
        }
    }

    /// The rows a diff has in the current layout (for scrolling).
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
            let w = (ui_.te.measure(semi, ui_.pf(11.0), label) + ui_.pf(16.0)) as i32;
            let r = Rect::new(*x - w, y, w, h);
            if on {
                cv.fill_round(r, ui_.pf(5.0), BG_CHOSEN);
            } else if ui_.is_hot(Hot::DiffBar(hits.len())) {
                cv.fill_round(r, ui_.pf(5.0), CONTROL_HOVER);
            }
            ui_.te.draw(
                cv,
                semi,
                ui_.pf(11.0),
                r.x as f32 + ui_.pf(8.0),
                base,
                label,
                if on { TEXT } else { TEXT_3 },
                w as f32,
            );
            hits.push((r, hit));
            *x = r.x - ui_.px(2.0);
        };
        let ws = self.diff_opts.ignore_whitespace;
        seg(
            self,
            cv,
            &mut x,
            if ws { "Whitespace ignored" } else { "Whitespace shown" },
            ws,
            DiffBar::Whitespace,
            &mut hits,
        );
        x -= self.px(10.0);
        for &(n, label) in CONTEXTS.iter().rev() {
            let on = self.diff_opts.context == n;
            seg(self, cv, &mut x, label, on, DiffBar::Context(n), &mut hits);
        }
        x -= self.px(10.0);
        let split = self.diff_split;
        seg(self, cv, &mut x, "Side by side", split, DiffBar::Split, &mut hits);
        seg(self, cv, &mut x, "Unified", !split, DiffBar::Unified, &mut hits);
        let _ = ui;
        self.diff_bar_hits = hits;
        x as f32
    }

    pub(super) fn diff_bar_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.diff_bar_hits.iter().position(|h| h.0.contains(x, y))
    }

    pub(super) fn diff_bar_click(&mut self, x: i32, y: i32) -> bool {
        let Some(i) = self.diff_bar_hit_at(x, y) else { return false };
        let h = self.diff_bar_hits[i].1;
        self.apply_diff_bar(h);
        true
    }

    pub(super) fn apply_diff_bar(&mut self, h: DiffBar) {
        match h {
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
            }
        }
        let r = self.diff_rect();
        self.damage.rects.push(r);
    }
}
