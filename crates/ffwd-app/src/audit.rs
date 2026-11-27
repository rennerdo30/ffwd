//! Programmatic UI audit checks (`--audit` and `--serve audit`).
//!
//! Catches contrast regressions, unexpected clickable hit-rect overlaps,
//! out-of-bounds controls, pane geometry collisions, text truncation counts,
//! and partial-vs-full paint mismatches in milliseconds without requiring
//! manual pixel inspection.

use crate::theme::{self, contrast_ratio};
use crate::ui::{Key, Ui};
use ffwd_render::{Canvas, Rect};

#[derive(Clone, Debug)]
pub struct ContrastCheck {
    pub pair: &'static str,
    pub ratio: f64,
    pub min: f64,
    pub pass: bool,
}

#[derive(Clone, Debug)]
pub struct OverlapIssue {
    pub a: &'static str,
    pub b: &'static str,
    pub area_px: u64,
}

#[derive(Clone, Debug)]
pub struct AuditReport {
    pub screen: String,
    pub theme: &'static str,
    pub size: (usize, usize),
    pub scale: f32,
    pub paint_ms: f64,
    pub ellipsis_count: usize,
    pub partial_paint_ok: Option<bool>,
    pub partial_mismatch_px: usize,
    pub contrast_failures: Vec<ContrastCheck>,
    pub hit_overlaps: Vec<OverlapIssue>,
    pub out_of_bounds_hits: Vec<(&'static str, Rect)>,
    pub pane_overlaps: Vec<OverlapIssue>,
    pub panes: Vec<(&'static str, Rect)>,
}

impl AuditReport {
    pub fn ok(&self) -> bool {
        self.contrast_failures.is_empty()
            && self.hit_overlaps.is_empty()
            && self.out_of_bounds_hits.is_empty()
            && self.pane_overlaps.is_empty()
            && self.partial_paint_ok.unwrap_or(true)
    }

    pub fn to_json(&self) -> String {
        let mut s = String::with_capacity(1024);
        s.push('{');
        s.push_str(&format!("\"ok\":{},", self.ok()));
        s.push_str(&format!("\"screen\":\"{}\",", esc(&self.screen)));
        s.push_str(&format!("\"theme\":\"{}\",", self.theme));
        s.push_str(&format!("\"size\":[{},{}],", self.size.0, self.size.1));
        s.push_str(&format!("\"scale\":{:.2},", self.scale));
        s.push_str(&format!("\"paint_ms\":{:.2},", self.paint_ms));
        s.push_str(&format!("\"ellipsis_count\":{},", self.ellipsis_count));
        match self.partial_paint_ok {
            Some(ok) => {
                s.push_str(&format!("\"partial_paint_ok\":{ok},\"partial_mismatch_px\":{},", self.partial_mismatch_px))
            }
            None => s.push_str("\"partial_paint_ok\":null,"),
        }
        s.push_str("\"contrast_failures\":[");
        for (i, c) in self.contrast_failures.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("{{\"pair\":\"{}\",\"ratio\":{:.2},\"min\":{:.1}}}", c.pair, c.ratio, c.min));
        }
        s.push_str("],\"hit_overlaps\":[");
        for (i, o) in self.hit_overlaps.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("{{\"a\":\"{}\",\"b\":\"{}\",\"area_px\":{}}}", o.a, o.b, o.area_px));
        }
        s.push_str("],\"out_of_bounds_hits\":[");
        for (i, &(n, r)) in self.out_of_bounds_hits.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("{{\"name\":\"{n}\",\"rect\":[{},{},{},{}]}}", r.x, r.y, r.w, r.h));
        }
        s.push_str("],\"pane_overlaps\":[");
        for (i, o) in self.pane_overlaps.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("{{\"a\":\"{}\",\"b\":\"{}\",\"area_px\":{}}}", o.a, o.b, o.area_px));
        }
        s.push_str("],\"panes\":{");
        for (i, &(n, r)) in self.panes.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("\"{n}\":[{},{},{},{}]", r.x, r.y, r.w, r.h));
        }
        s.push_str("}}");
        s
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Checks critical WCAG 2.1 contrast ratios on the currently active theme.
pub fn check_contrast() -> Vec<ContrastCheck> {
    let p = theme::pal();
    let pairs: [(&'static str, u32, u32, f64); 14] = [
        ("text/bg", p.text, p.bg, 7.0),
        ("text_2/bg", p.text_2, p.bg, 4.5),
        ("text_3/bg", p.text_3, p.bg, 4.5),
        ("text_4/bg", p.text_4, p.bg, 3.0),
        ("text/bg_sidebar", p.text, p.bg_sidebar, 7.0),
        ("text/bg_panel", p.text, p.bg_panel, 7.0),
        ("accent_text/accent", p.accent_text, p.accent, 4.5),
        ("accent_fg/bg", p.accent_fg, p.bg, 4.5),
        ("add_text/add_bg", p.add_text, p.add_bg, 4.5),
        ("del_text/del_bg", p.del_text, p.del_bg, 4.5),
        ("hunk_text/hunk_bg", p.hunk_text, p.hunk_bg, 4.5),
        ("local_fg/bg", p.local_fg, p.bg, 4.5),
        ("remote_fg/remote_bg", p.remote_fg, p.remote_bg, 4.5),
        ("tag_fg/tag_bg", p.tag_fg, p.tag_bg, 4.5),
    ];
    pairs
        .into_iter()
        .map(|(pair, fg, bg, min)| {
            let ratio = contrast_ratio(fg, bg);
            ContrastCheck { pair, ratio, min, pass: ratio >= min }
        })
        .collect()
}

/// Runs the full audit on `ui` (assumes `ui.paint` has just rendered into `px`).
pub fn run(ui: &mut Ui, screen: &str, px: &[u32], paint_ms: f64, check_partial: bool) -> AuditReport {
    let bounds = Rect::new(0, 0, ui.w as i32, ui.h as i32);
    let ellipsis_count = ui.te.ellipsis_count();

    let contrast_failures: Vec<ContrastCheck> = check_contrast().into_iter().filter(|c| !c.pass).collect();

    // Major layout panes must not overlap each other.
    let panes = vec![
        ("top", ui.lay.top),
        ("tabs", ui.lay.tabs),
        ("side", ui.lay.side),
        ("list", ui.lay.list),
        ("details", ui.lay.details),
        ("status", ui.lay.status),
    ];
    let mut pane_overlaps = Vec::new();
    for i in 0..panes.len() {
        for j in (i + 1)..panes.len() {
            let area = panes[i].1.intersect(panes[j].1).area();
            if area > 0 {
                pane_overlaps.push(OverlapIssue { a: panes[i].0, b: panes[j].0, area_px: area });
            }
        }
    }

    // Active clickable hit rectangles: check out-of-bounds and overlaps.
    let hits = ui.audit_hit_rects();
    let mut out_of_bounds_hits = Vec::new();
    let mut hit_overlaps = Vec::new();
    for &(name, r) in &hits {
        if !r.is_empty()
            && (r.x < bounds.x || r.y < bounds.y || r.right() > bounds.right() || r.bottom() > bounds.bottom())
        {
            out_of_bounds_hits.push((name, r));
        }
    }
    for i in 0..hits.len() {
        for j in (i + 1)..hits.len() {
            let area = hits[i].1.intersect(hits[j].1).area();
            if area > 4 {
                hit_overlaps.push(OverlapIssue { a: hits[i].0, b: hits[j].0, area_px: area });
            }
        }
    }

    // Optional partial-paint parity check (moves selection down 1 row in History).
    let (partial_paint_ok, partial_mismatch_px) = if check_partial && screen == "history" && ui.r.commits.len() > 1 {
        let mut partial_buf = px.to_vec();
        let mut full_buf = vec![0u32; px.len()];
        let prev_sel = ui.r.sel;
        let prev_top = ui.r.top;
        let prev_sel_file = ui.r.sel_file;
        let prev_details = ui.r.details.clone();
        let prev_diff = ui.r.diff.clone();
        ui.key(Key::Down);
        let mut cv_part = Canvas::new(&mut partial_buf, ui.w, ui.h);
        ui.paint(&mut cv_part, false);
        let mut cv_full = Canvas::new(&mut full_buf, ui.w, ui.h);
        ui.paint(&mut cv_full, true);
        let mismatch = partial_buf.iter().zip(full_buf.iter()).filter(|(a, b)| a != b).count();
        // Restore original selection and cached details/diff so subsequent
        // screens in the same run see the exact populated demo state.
        ui.select(prev_sel);
        ui.r.top = prev_top;
        ui.r.sel_file = prev_sel_file;
        ui.r.details = prev_details;
        ui.r.diff = prev_diff;
        (Some(mismatch == 0), mismatch)
    } else {
        (None, 0)
    };

    AuditReport {
        screen: screen.to_string(),
        theme: if theme::is_light() { "light" } else { "dark" },
        size: (ui.w, ui.h),
        scale: ui.s,
        paint_ms,
        ellipsis_count,
        partial_paint_ok,
        partial_mismatch_px,
        contrast_failures,
        hit_overlaps,
        out_of_bounds_hits,
        pane_overlaps,
        panes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_themes_pass_contrast_audit() {
        for light in [false, true] {
            theme::set_light(light);
            let failures: Vec<_> = check_contrast().into_iter().filter(|c| !c.pass).collect();
            assert!(failures.is_empty(), "contrast failures (light={light}): {failures:?}");
        }
        theme::set_light(false);
    }
}
