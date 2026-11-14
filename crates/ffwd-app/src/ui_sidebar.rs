//! The sidebar: workspace entries, branches (with folders), remotes, tags,
//! stashes and worktrees. It keeps its own selection, which
//! follows clicks and keyboard moves and jumps the history to the ref.
//!
//! Items are rebuilt into a flat list when refs, the status, the collapse
//! state or the filter change; painting only walks the visible rows.

use super::*;
use std::collections::{BTreeMap, HashSet};

pub(super) const SIDE_ROW: f32 = 28.0;
pub(super) const FILTER_H: f32 = 40.0;

#[derive(Clone, Debug)]
pub(super) struct SideItem {
    pub depth: u8,
    pub kind: SideKind,
    pub label: String,
    pub badge: String,
    /// Stable identity: selection and collapse state survive rebuilds.
    pub key: String,
    pub target: Option<Oid>,
    /// The checked-out branch.
    pub current: bool,
    /// `Some(expanded)` for headers, folders and remotes.
    pub expander: Option<bool>,
    /// A quiet note at the right: a branch's age, a worktree's branch.
    pub age: String,
    /// How old a branch's last commit is, in seconds (0: unknown).
    pub age_secs: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SideKind {
    Header,
    /// "Add remote…" where a repository has none.
    AddRemote,
    Branch,
    Folder,
    Remote,
    RemoteBranch,
    Tag,
    Stash,
    Worktree,
    PullRequest,
    Submodule,
}

/// How the sidebar offers the views (setting `ui.sidebar`, FF-57).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SideStyle {
    /// A rail of labelled icons along the left edge.
    Rail,
    /// Tabs over the content, right of the sidebar.
    Tabs,
    /// One row per view under the card.
    List,
    /// A row of icons under the card, the open view named beneath.
    Bar,
}

impl SideStyle {
    pub const KEYS: [&'static str; 4] = ["rail", "tabs", "list", "bar"];

    pub fn from_setting(s: Option<&str>) -> SideStyle {
        match s {
            Some("tabs") => SideStyle::Tabs,
            Some("list") => SideStyle::List,
            Some("bar") => SideStyle::Bar,
            _ => SideStyle::Rail,
        }
    }
}

/// A view the sidebar (or the tabs) opens, with its live figure.
pub(super) struct View {
    pub key: &'static str,
    pub name: &'static str,
    pub glyph: Glyph,
    /// The figure in words: "3 changed", "1 failed".
    pub stat: String,
    /// The figure as a count for a badge; empty when calm.
    pub count: String,
    pub color: u32,
    /// The figure asks for attention.
    pub alert: bool,
}

impl SideItem {
    fn new(depth: u8, kind: SideKind, label: &str, key: String) -> SideItem {
        SideItem {
            depth,
            kind,
            label: label.to_string(),
            badge: String::new(),
            key,
            target: None,
            current: false,
            expander: None,
            age: String::new(),
            age_secs: 0,
        }
    }
}

/// Adds `refs` as a tree: names with `/` group into folders (folders
/// first, then branches, each sorted), as `t3/…` folders.
#[allow(clippy::too_many_arguments)]
fn tree(
    out: &mut Vec<SideItem>,
    items: Vec<(String, &Ref)>,
    depth: u8,
    prefix: &str,
    kind: SideKind,
    collapsed: &HashSet<String>,
    filtering: bool,
    recent: bool,
) {
    let mut folders: BTreeMap<String, Vec<(String, &Ref)>> = BTreeMap::new();
    let mut leaves: Vec<(String, &Ref)> = Vec::new();
    for (name, r) in items {
        match name.split_once('/') {
            Some((seg, rest)) if !seg.is_empty() && !rest.is_empty() => {
                folders.entry(seg.to_string()).or_default().push((rest.to_string(), r))
            }
            _ => leaves.push((name, r)),
        }
    }
    // By name, or (recent) folders by their newest branch.
    let mut folders: Vec<(String, Vec<(String, &Ref)>)> = folders.into_iter().collect();
    if recent {
        let newest = |c: &Vec<(String, &Ref)>| c.iter().map(|x| x.1.time).max().unwrap_or(0);
        folders.sort_by(|a, b| newest(&b.1).cmp(&newest(&a.1)).then_with(|| a.0.cmp(&b.0)));
    }
    for (seg, children) in folders {
        let key = format!("{prefix}{seg}/");
        let open = filtering || !collapsed.contains(&key);
        let mut f = SideItem::new(depth, SideKind::Folder, &seg, key.clone());
        f.badge = fmt::num(children.len());
        f.expander = Some(open);
        out.push(f);
        if open {
            tree(out, children, depth + 1, &key, kind, collapsed, filtering, recent);
        }
    }
    if recent {
        leaves.sort_by(|a, b| b.1.time.cmp(&a.1.time).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    } else {
        leaves.sort_by_key(|a| a.0.to_lowercase());
    }
    let now = super::now_secs();
    for (name, r) in leaves {
        let mut it = SideItem::new(depth, kind, &name, format!("{prefix}{name}"));
        it.target = Some(r.target);
        it.current = r.is_head;
        if kind == SideKind::Branch {
            let mut b = String::new();
            // An upstream that was deleted on the remote: say so (FF-36).
            if r.upstream_gone {
                b.push_str("gone");
            }
            if r.ahead > 0 {
                b.push_str(&format!("↑{}", r.ahead));
            }
            if r.behind > 0 {
                if !b.is_empty() {
                    b.push(' ');
                }
                b.push_str(&format!("↓{}", r.behind));
            }
            it.badge = b;
            if it.badge.is_empty() && r.time > 0 {
                // How old its last commit is, quietly.
                it.age = short_age(now - r.time);
                it.age_secs = now - r.time;
            }
        }
        out.push(it);
    }
}

impl Ui {
    pub(super) fn side_row_h(&self) -> i32 {
        self.px(SIDE_ROW)
    }

    /// How the sidebar offers the views (setting `ui.sidebar`).
    pub(super) fn side_style(&self) -> SideStyle {
        SideStyle::from_setting(self.settings.get("ui.sidebar"))
    }

    /// Switches the sidebar style ("rail", "tabs", "list", "bar").
    pub fn set_side_style(&mut self, key: &str) {
        self.settings.set_string("ui.sidebar", key);
        self.side_view_hot = None;
        self.resize(self.w, self.h, self.s);
        self.rebuild_side();
    }

    /// The icon rail's width (Rail style), else 0.
    fn rail_w(&self) -> i32 {
        if self.side_style() == SideStyle::Rail { self.px(64.0) } else { 0 }
    }

    /// The sidebar without the rail and without its right border.
    fn side_panel(&self) -> Rect {
        let s = self.lay.side;
        let rw = self.rail_w();
        Rect::new(s.x + rw, s.y, (s.w - rw - 1).max(0), s.h)
    }

    /// Where you are, at the top of the sidebar (FF-57): the branch, its
    /// sync with the upstream, the changes and a stopped operation.
    pub(super) fn side_here_rect(&self) -> Rect {
        let p = self.side_panel();
        Rect::new(p.x + self.px(10.0), p.y + self.px(10.0), p.w - self.px(20.0), self.px(62.0))
    }

    /// The views, with their live figures (FF-57).
    pub(super) fn views(&self) -> Vec<View> {
        let calm = |key, name, glyph, stat: &str| View {
            key,
            name,
            glyph,
            stat: stat.to_string(),
            count: String::new(),
            color: TEXT_4(),
            alert: false,
        };
        let loud = |key, name, glyph, stat: String, n: usize, color| View {
            key,
            name,
            glyph,
            stat,
            count: fmt::num(n),
            color,
            alert: true,
        };
        let changes = match self.r.status {
            Some(s) if s.conflicted > 0 => {
                let n = s.conflicted as usize;
                loud("changes", "Changes", Glyph::Edit, format!("{} conflicts", fmt::num(n)), n, DEL())
            }
            Some(s) if s.total() > 0 => {
                let n = s.total() as usize;
                loud("changes", "Changes", Glyph::Edit, format!("{} changed", fmt::num(n)), n, MOD())
            }
            Some(_) => calm("changes", "Changes", Glyph::Edit, "clean"),
            None => calm("changes", "Changes", Glyph::Edit, "…"),
        };
        let n = self.r.commits.len();
        let history = if self.r.log_done { format!("{} commits", fmt::num(n)) } else { format!("{}…", fmt::num(n)) };
        let activity = match self.activity.unseen_failures {
            0 => calm("activity", "Activity", Glyph::Text, "all fine"),
            f => loud("activity", "Activity", Glyph::Text, format!("{f} failed"), f as usize, DEL()),
        };
        let agents = if !self.agent_config().0.enabled {
            calm("agents", "Agents", Glyph::Terminal, "off")
        } else {
            match self.agent_counts() {
                (0, 0) => calm("agents", "Agents", Glyph::Terminal, "idle"),
                (0, r) => loud("agents", "Agents", Glyph::Terminal, format!("{r} to review"), r, ADD()),
                (n, _) => loud("agents", "Agents", Glyph::Terminal, format!("{n} running"), n, ACCENT_FG()),
            }
        };
        vec![
            changes,
            calm("all", "History", Glyph::Commit, &history),
            calm("files", "Files", Glyph::Folder, "browse the tree"),
            activity,
            calm("reflog", "Reflog", Glyph::Clock, "undo trail"),
            agents,
        ]
    }

    /// The view on screen.
    pub(super) fn current_view(&self) -> &'static str {
        if self.agent_view.is_some() {
            return "agents";
        }
        match self.r.side_sel.as_str() {
            "changes" => "changes",
            "files" => "files",
            "activity" => "activity",
            "reflog" => "reflog",
            _ => "all",
        }
    }

    /// The rail's buttons (Rail style).
    fn rail_rects(&self) -> Vec<Rect> {
        let s = self.lay.side;
        let (h, gap) = (self.px(50.0), self.px(4.0));
        (0..6)
            .map(|k| {
                Rect::new(s.x + self.px(6.0), s.y + self.px(10.0) + k * (h + gap), self.rail_w() - self.px(12.0), h)
            })
            .collect()
    }

    /// The rows under the card (List style).
    fn view_list_rects(&self) -> Vec<Rect> {
        let p = self.side_panel();
        let top = self.side_here_rect().bottom() + self.px(8.0);
        let h = self.px(28.0);
        (0..6).map(|k| Rect::new(p.x + self.px(6.0), top + k * h, p.w - self.px(12.0), h)).collect()
    }

    /// The icon bar under the card (Bar style), and its segments.
    fn view_bar_rect(&self) -> Rect {
        let p = self.side_panel();
        Rect::new(
            p.x + self.px(10.0),
            self.side_here_rect().bottom() + self.px(8.0),
            p.w - self.px(20.0),
            self.px(36.0),
        )
    }

    fn view_bar_rects(&self) -> Vec<Rect> {
        let b = self.view_bar_rect().inset(self.px(3.0), self.px(3.0));
        (0..6)
            .map(|k| {
                let x0 = b.x + b.w * k / 6;
                let x1 = b.x + b.w * (k + 1) / 6;
                Rect::new(x0, b.y, x1 - x0, b.h)
            })
            .collect()
    }

    /// The tabs over the content (Tabs style): equal widths, so they stay
    /// put while their figures change.
    fn view_tab_rects(&self) -> Vec<Rect> {
        let v = self.lay.views;
        let w = self.px(124.0).min((v.w - self.px(16.0)) / 6).max(1);
        (0..6).map(|k| Rect::new(v.x + self.px(8.0) + k * w, v.y, w, v.h)).collect()
    }

    /// The view buttons of the current style, in the order of `views`.
    pub(super) fn view_rects(&self) -> Vec<Rect> {
        match self.side_style() {
            SideStyle::Rail => self.rail_rects(),
            SideStyle::Tabs => self.view_tab_rects(),
            SideStyle::List => self.view_list_rects(),
            SideStyle::Bar => self.view_bar_rects(),
        }
    }

    /// The view button under the pointer.
    fn view_at(&self, x: i32, y: i32) -> Option<usize> {
        self.view_rects().iter().position(|r| r.contains(x, y))
    }

    /// A click in the tabs strip over the content.
    pub(super) fn view_click(&mut self, x: i32, y: i32) {
        if let Some(k) = self.view_at(x, y) {
            let key = self.views()[k].key;
            self.view_activate(key);
        }
    }

    /// Opens a view.
    pub(super) fn view_activate(&mut self, key: &str) {
        if key != "agents" {
            self.close_agents();
        }
        match key {
            "changes" => self.enter_changes(),
            "files" => self.enter_files(),
            "activity" => self.enter_activity(),
            "reflog" => self.enter_reflog(),
            "agents" => self.open_agents(),
            _ => {
                self.leave_changes();
                self.leave_files();
                self.leave_activity();
                self.leave_reflog();
                self.side_select("all");
            }
        }
        self.damage.rects.extend([self.lay.side, self.lay.views]);
    }

    /// Tests: a view's live figure.
    #[cfg(test)]
    pub(super) fn view_stat(&self, key: &str) -> Option<String> {
        self.views().into_iter().find(|t| t.key == key).map(|t| t.stat)
    }

    /// A count badge: a pill in the view's colour.
    fn view_badge(&mut self, cv: &mut Canvas, right: f32, cy: f32, v: &View) {
        let semi = self.f.semi;
        let fs = self.pf(10.0);
        let tw = self.te.measure(semi, fs, &v.count);
        let h = self.pf(15.0);
        let w = (tw + self.pf(9.0)).max(h);
        let r = Rect::new((right - w) as i32, (cy - h / 2.0) as i32, w as i32, h as i32);
        cv.fill_round(r, h / 2.0, v.color);
        let fg = if crate::theme::is_light() { 0xffffff } else { BG() };
        self.te.draw(cv, semi, fs, right - w + (w - tw) / 2.0, cy + self.pf(3.5), &v.count, fg, w);
    }

    fn paint_views(&mut self, cv: &mut Canvas) {
        match self.side_style() {
            SideStyle::Rail => self.paint_rail(cv),
            SideStyle::Tabs => {}
            SideStyle::List => self.paint_view_list(cv),
            SideStyle::Bar => self.paint_view_bar(cv),
        }
    }

    /// Rail: a labelled icon per view down the left edge; the open one
    /// lit, with an accent bar at the edge; counts as pills on the icon.
    fn paint_rail(&mut self, cv: &mut Canvas) {
        let s = self.lay.side;
        let rw = self.rail_w();
        cv.fill(Rect::new(s.x, s.y, rw, s.h), BG_BAR());
        cv.fill(Rect::new(s.x + rw - 1, s.y, 1, s.h), BORDER_SOFT());
        let cur = self.current_view();
        let ui = self.f.ui;
        for (k, (v, r)) in self.views().into_iter().zip(self.rail_rects()).enumerate() {
            let on = v.key == cur;
            let hot = self.side_view_hot == Some(k);
            if on {
                cv.fill_round(r, self.pf(9.0), ACCENT_SOFT());
                cv.fill_round(
                    Rect::new(s.x, r.y + self.px(12.0), self.px(3.0), r.h - self.px(24.0)),
                    self.pf(1.5),
                    ACCENT(),
                );
            } else if hot {
                cv.fill_round(r, self.pf(9.0), BG_HOVER());
            }
            let cx = r.x as f32 + r.w as f32 / 2.0;
            let gy = r.y as f32 + self.pf(18.0);
            let gc = if on {
                ACCENT_FG()
            } else if hot {
                TEXT()
            } else {
                TEXT_3()
            };
            self.glyph(cv, v.glyph, cx, gy, self.pf(7.0), gc);
            if v.alert {
                self.view_badge(cv, r.right() as f32 - self.pf(3.0), r.y as f32 + self.pf(9.0), &v);
            }
            let fs = self.pf(10.5);
            let tw = self.te.measure(ui, fs, v.name);
            let tc = if on { ACCENT_FG() } else { TEXT_3() };
            self.te.draw(cv, ui, fs, cx - tw / 2.0, r.y as f32 + self.pf(41.0), v.name, tc, r.w as f32);
        }
    }

    /// List: one row per view, its figure at the right.
    fn paint_view_list(&mut self, cv: &mut Canvas) {
        let (ui, semi) = (self.f.ui, self.f.semi);
        let cur = self.current_view();
        for (k, (v, r)) in self.views().into_iter().zip(self.view_list_rects()).enumerate() {
            let on = v.key == cur;
            let hot = self.side_view_hot == Some(k);
            if on {
                cv.fill_round(r, self.pf(6.0), BG_SELECT_DIM());
                cv.fill_round(
                    Rect::new(r.x, r.y + self.px(6.0), self.px(3.0), r.h - self.px(12.0)),
                    self.pf(1.5),
                    ACCENT(),
                );
            } else if hot {
                cv.fill_round(r, self.pf(6.0), BG_HOVER());
            }
            let cy = r.y as f32 + r.h as f32 / 2.0;
            let base = (cy + self.pf(4.5)).round();
            let gx = r.x as f32 + self.pf(16.0);
            self.glyph(cv, v.glyph, gx, cy, self.pf(5.5), if on { ACCENT() } else { TEXT_3() });
            let font = if on { semi } else { ui };
            self.te.draw(
                cv,
                font,
                self.pf(13.0),
                gx + self.pf(14.0),
                base,
                v.name,
                if on { TEXT() } else { TEXT_2() },
                r.w as f32,
            );
            let fs = self.pf(11.5);
            let f2 = if v.alert { semi } else { ui };
            let tw = self.te.measure(f2, fs, &v.stat);
            let mut sx = r.right() as f32 - tw - self.pf(10.0);
            self.te.draw(cv, f2, fs, sx, base - self.pf(0.5), &v.stat, v.color, tw + 2.0);
            if v.alert {
                sx -= self.pf(9.0);
                cv.fill_circle(sx, cy, self.pf(3.0), v.color);
            }
        }
        // A quiet rule between the views and the refs.
        let rs = self.view_list_rects();
        if let Some(last) = rs.last() {
            let p = self.side_panel();
            cv.fill(
                Rect::new(p.x + self.px(14.0), last.bottom() + self.px(5.0), p.w - self.px(28.0), 1),
                BORDER_SOFT(),
            );
        }
    }

    /// Bar: a segmented row of icons, the open view (or the hovered one)
    /// named with its figure beneath.
    fn paint_view_bar(&mut self, cv: &mut Canvas) {
        let (ui, semi) = (self.f.ui, self.f.semi);
        let bar = self.view_bar_rect();
        cv.round_border(bar, self.pf(9.0), BORDER_SOFT(), BG_PANEL());
        let cur = self.current_view();
        let views = self.views();
        for (k, (v, r)) in views.iter().zip(self.view_bar_rects()).enumerate() {
            let on = v.key == cur;
            let hot = self.side_view_hot == Some(k);
            if on {
                cv.fill_round(r, self.pf(7.0), ACCENT_SOFT());
            } else if hot {
                cv.fill_round(r, self.pf(7.0), BG_HOVER());
            }
            let cx = r.x as f32 + r.w as f32 / 2.0;
            let cy = r.y as f32 + r.h as f32 / 2.0;
            let gc = if on {
                ACCENT_FG()
            } else if hot {
                TEXT()
            } else {
                TEXT_3()
            };
            self.glyph(cv, v.glyph, cx, cy, self.pf(6.5), gc);
            if v.alert {
                cv.fill_circle(cx + self.pf(8.0), cy - self.pf(7.0), self.pf(3.5), v.color);
            }
        }
        // The caption: what the hovered (else the open) view is and holds.
        let k = self.side_view_hot.unwrap_or_else(|| views.iter().position(|v| v.key == cur).unwrap_or(0));
        if let Some(v) = views.get(k) {
            let base = bar.bottom() as f32 + self.pf(17.0);
            let x = bar.x as f32 + self.pf(6.0);
            let end = self.te.draw(cv, semi, self.pf(12.0), x, base, v.name, TEXT_2(), bar.w as f32);
            self.te.draw(
                cv,
                if v.alert { semi } else { ui },
                self.pf(12.0),
                end + self.pf(8.0),
                base,
                &v.stat,
                v.color,
                bar.right() as f32 - end - self.pf(8.0),
            );
        }
    }

    /// Tabs: the views over the content, right of the sidebar.
    pub(super) fn paint_view_tabs(&mut self, cv: &mut Canvas) {
        let v = self.lay.views;
        if v.h == 0 {
            return;
        }
        let (ui, semi) = (self.f.ui, self.f.semi);
        cv.fill(v, BG_BAR());
        cv.fill(Rect::new(v.x, v.bottom() - 1, v.w, 1), BORDER());
        let cur = self.current_view();
        for (k, (t, r)) in self.views().into_iter().zip(self.view_tab_rects()).enumerate() {
            let on = t.key == cur;
            let hot = self.side_view_hot == Some(k);
            let inner = Rect::new(r.x + self.px(2.0), r.y + self.px(5.0), r.w - self.px(4.0), r.h - self.px(10.0));
            if on {
                cv.fill_round(inner, self.pf(7.0), BG());
                cv.fill(
                    Rect::new(r.x + self.px(10.0), r.bottom() - self.px(2.0), r.w - self.px(20.0), self.px(2.0)),
                    ACCENT(),
                );
            } else if hot {
                cv.fill_round(inner, self.pf(7.0), BG_HOVER());
            }
            let cy = r.y as f32 + r.h as f32 / 2.0;
            let gx = r.x as f32 + self.pf(18.0);
            self.glyph(cv, t.glyph, gx, cy, self.pf(5.5), if on { ACCENT() } else { TEXT_3() });
            let font = if on { semi } else { ui };
            let right = r.right() as f32 - self.pf(8.0);
            let end = self.te.draw(
                cv,
                font,
                self.pf(12.5),
                gx + self.pf(13.0),
                (cy + self.pf(4.5)).round(),
                t.name,
                if on { TEXT() } else { TEXT_2() },
                right - gx,
            );
            if t.alert {
                let fs = self.pf(10.0);
                let w = (self.te.measure(semi, fs, &t.count) + self.pf(9.0)).max(self.pf(15.0));
                self.view_badge(cv, (end + self.pf(6.0) + w).min(right), cy, &t);
            }
        }
    }

    /// The filter field under the card (and the views, when they live in
    /// the sidebar).
    pub(super) fn side_filter_rect(&self) -> Rect {
        let p = self.side_panel();
        let top = match self.side_style() {
            SideStyle::Rail | SideStyle::Tabs => self.side_here_rect().bottom() + self.px(10.0),
            SideStyle::List => self.view_list_rects().last().map_or(0, |r| r.bottom()) + self.px(12.0),
            SideStyle::Bar => self.view_bar_rect().bottom() + self.px(28.0),
        };
        Rect::new(p.x + self.px(10.0), top, p.w - self.px(20.0), self.px(FILTER_H - 14.0))
    }

    pub(super) fn side_list_rect(&self) -> Rect {
        let p = self.side_panel();
        let top = self.side_filter_rect().bottom() + self.px(8.0);
        Rect::new(p.x, top, p.w, (p.bottom() - top).max(0))
    }

    /// The card: line one the branch (or the detached commit) and its sync
    /// as chips, line two the upstream and what waits in the working tree.
    fn paint_side_here(&mut self, cv: &mut Canvas) {
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let r = self.side_here_rect();
        let hot = self.side_here_hot;
        cv.round_border(
            r,
            self.pf(9.0),
            if hot { BORDER_HOVER() } else { BORDER_SOFT() },
            if hot { BG_HOVER() } else { BG_PANEL() },
        );
        let head = self.r.refs.refs.iter().find(|x| x.kind == RefKind::Local && x.is_head).cloned();
        let x0 = r.x as f32 + self.pf(12.0);
        let b1 = r.y as f32 + self.pf(24.0);
        let b2 = r.y as f32 + self.pf(46.0);
        let right = r.right() as f32 - self.pf(10.0);
        // Line one: branch, then the sync chips from the right.
        let mut chips: Vec<(String, u32, u32)> = Vec::new();
        match &head {
            Some(h) if h.upstream_gone => chips.push(("upstream gone".into(), DEL_BG(), DEL_TEXT())),
            Some(h) if h.upstream.is_none() => chips.push(("no upstream".into(), BG_HOVER(), TEXT_3())),
            Some(h) if h.ahead == 0 && h.behind == 0 => chips.push(("in sync".into(), ADD_BG(), ADD_TEXT())),
            Some(h) => {
                if h.behind > 0 {
                    chips.push((format!("↓{}", h.behind), REMOTE_BG(), REMOTE_FG()));
                }
                if h.ahead > 0 {
                    chips.push((format!("↑{}", h.ahead), ACCENT_SOFT(), ACCENT_FG()));
                }
            }
            None => {}
        }
        let mut cx = right;
        for (t, bg, fg) in chips.iter().rev() {
            let w = self.te.measure(semi, self.pf(11.0), t) + self.pf(14.0);
            cx -= w;
            let cr = Rect::new(cx as i32, (b1 - self.pf(14.0)) as i32, w as i32, self.px(19.0));
            cv.fill_round(cr, self.pf(9.5), *bg);
            self.te.draw(cv, semi, self.pf(11.0), cx + self.pf(7.0), b1 - self.pf(0.5), t, *fg, w);
            cx -= self.pf(6.0);
        }
        let name_w = cx - x0 - self.pf(20.0);
        match &head {
            Some(h) => {
                self.branch_icon(cv, x0 + self.pf(5.0), b1 - self.pf(4.5), ACCENT());
                let fs = self.pf(14.0);
                let name = super::fit_middle(&mut self.te, semi, fs, &h.name, name_w);
                self.te.draw(cv, semi, self.pf(14.0), x0 + self.pf(18.0), b1, &name, TEXT(), name_w + 2.0);
            }
            None => {
                let at = self.r.refs.head.map(|h| h.short(8)).unwrap_or_default();
                let t = if at.is_empty() { "No commit yet".to_string() } else { format!("Detached at {at}") };
                self.te.draw(cv, semi, self.pf(14.0), x0, b1, &t, MOD(), name_w + 20.0);
            }
        }
        // Line two: a stopped operation first; else upstream and changes.
        let st = self.r.status;
        let conflicts = st.map_or(0, |s| s.conflicted as usize);
        let changed = st.map_or(0, |s| s.total() as usize);
        let mut x = x0;
        let w2 = right - x0;
        if let Some(op) = self.r.changes.in_progress {
            let t = if conflicts > 0 {
                format!("{op} stopped · {conflicts} conflicts")
            } else {
                format!("{op} in progress")
            };
            cv.fill_circle(x + self.pf(3.5), b2 - self.pf(4.0), self.pf(3.5), DEL());
            self.te.draw(cv, semi, self.pf(12.0), x + self.pf(12.0), b2, &t, DEL_TEXT(), w2);
            return;
        }
        if let Some(up) = head.as_ref().and_then(|h| h.upstream.clone()) {
            x = self.te.draw(cv, mono, self.pf(11.5), x, b2, &format!("→ {up}"), TEXT_3(), w2 * 0.6) + self.pf(10.0);
        }
        let (t, c) = match changed {
            0 => ("clean".to_string(), TEXT_4()),
            n => (format!("{n} changed"), MOD()),
        };
        if changed > 0 {
            cv.fill_circle(x + self.pf(3.0), b2 - self.pf(4.0), self.pf(3.0), MOD());
            x += self.pf(10.0);
        }
        self.te.draw(cv, if changed > 0 { semi } else { ui }, self.pf(12.0), x, b2, &t, c, right - x);
    }

    fn side_visible_rows(&self) -> usize {
        (self.side_list_rect().h / self.side_row_h().max(1)).max(1) as usize
    }

    pub(super) fn rebuild_side(&mut self) {
        let filter = self.side_filter.to_lowercase();
        let filtering = !filter.is_empty();
        let matches = |name: &str| !filtering || name.to_lowercase().contains(&filter);
        let collapsed = &self.side_collapsed;
        let open = |key: &str| filtering || !collapsed.contains(key);
        let mut v: Vec<SideItem> = Vec::new();
        let header = |v: &mut Vec<SideItem>, label: &str, key: &str, badge: String| {
            let mut h = SideItem::new(0, SideKind::Header, label, key.to_string());
            h.badge = badge;
            h.expander = Some(open(key));
            v.push(h);
            open(key)
        };

        let refs = &self.r.refs.refs;
        let recent = self.settings.get("sidebar.branch_order") == Some("recent");
        let locals: Vec<(String, &Ref)> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Local && matches(&r.name))
            .map(|r| (r.name.to_string(), r))
            .collect();
        if !locals.is_empty() || !filtering {
            let n = fmt::num(locals.len());
            if header(&mut v, "Branches", "h:branches", n) {
                tree(&mut v, locals, 0, "b:", SideKind::Branch, collapsed, filtering, recent);
            }
        }
        let remotes: Vec<&Ref> = refs.iter().filter(|r| r.kind == RefKind::Remote && matches(&r.name)).collect();
        // Configured remotes show even before their first fetch; the header
        // always shows (its menu adds a remote).
        let configured = &self.r.refs.remotes;
        if (!filtering || !remotes.is_empty())
            && header(
                &mut v,
                "Remotes",
                "h:remotes",
                if configured.is_empty() { String::new() } else { fmt::num(configured.len()) },
            )
        {
            let mut names: Vec<&str> = remotes.iter().filter_map(|r| r.name.split('/').next()).collect();
            if !filtering {
                names.extend(configured.iter().map(|r| &*r.name));
            }
            names.sort_unstable();
            names.dedup();
            if names.is_empty() && !filtering {
                v.push(SideItem::new(0, SideKind::AddRemote, "Add remote…", "act:addremote".into()));
            }
            for n in names {
                let rs: Vec<(String, &Ref)> = remotes
                    .iter()
                    .filter(|r| r.name.split('/').next() == Some(n))
                    .map(|r| (r.name.split_once('/').map_or(r.name.to_string(), |x| x.1.to_string()), *r))
                    .collect();
                let key = format!("r:{n}/");
                let mut it = SideItem::new(0, SideKind::Remote, n, key.clone());
                it.badge = fmt::num(rs.len());
                it.expander = Some(open(&key));
                v.push(it);
                if open(&key) {
                    tree(&mut v, rs, 1, &key, SideKind::RemoteBranch, collapsed, filtering, recent);
                }
            }
        }
        let mut tags: Vec<&Ref> = refs.iter().filter(|r| r.kind == RefKind::Tag && matches(&r.name)).collect();
        if !tags.is_empty() && header(&mut v, "Tags", "h:tags", fmt::num(tags.len())) {
            // Newest names first (v0.9 above v0.1), like a release list.
            tags.sort_by(|a, b| natural_desc(&a.name, &b.name));
            for r in tags {
                let mut it = SideItem::new(0, SideKind::Tag, &r.name, format!("t:{}", r.name));
                it.target = Some(r.target);
                v.push(it);
            }
        }
        let stashes: Vec<&(Box<str>, Box<str>)> = self.r.refs.stashes.iter().filter(|(_, m)| matches(m)).collect();
        if !stashes.is_empty() {
            let n = fmt::num(stashes.len());
            if header(&mut v, "Stashes", "h:stashes", n) {
                for (name, msg) in stashes {
                    let mut it = SideItem::new(0, SideKind::Stash, msg, format!("s:{name}"));
                    // Its row in the history, when stashes show there (FF-33).
                    it.target = refs.iter().find(|r| r.kind == RefKind::Stash && *r.name == **name).map(|r| r.target);
                    v.push(it);
                }
            }
        }
        let subs: Vec<&ffwd_core::submodules::Submodule> =
            self.r.refs.submodules.iter().filter(|s| matches(&s.path)).collect();
        if !subs.is_empty() && header(&mut v, "Submodules", "h:submodules", fmt::num(subs.len())) {
            use ffwd_core::submodules::State;
            for s in subs {
                let mut it = SideItem::new(0, SideKind::Submodule, &s.path, format!("m:{}", s.name));
                it.badge = match s.state {
                    State::Clean => String::new(),
                    State::Uninitialized => "not set up".into(),
                    State::Moved => "moved".into(),
                    State::Conflict => "conflict".into(),
                };
                v.push(it);
            }
        }
        // Open pull requests (GitHub through gh, FF-40).
        if let Some(Ok(prs)) = &self.r.prs {
            let shown: Vec<&ffwd_core::hosting::PullRequest> =
                prs.iter().filter(|p| matches(&p.title) || matches(&p.head)).collect();
            if (!filtering || !shown.is_empty()) && header(&mut v, "Pull requests", "h:prs", fmt::num(shown.len())) {
                for p in shown {
                    let mut it = SideItem::new(
                        0,
                        SideKind::PullRequest,
                        &format!("#{} {}", p.number, p.title),
                        format!("p:{}", p.number),
                    );
                    if p.draft {
                        it.badge = "draft".into();
                    }
                    v.push(it);
                }
            }
        }
        if self.r.refs.worktrees.len() > 1 {
            let n = fmt::num(self.r.refs.worktrees.len());
            if header(&mut v, "Worktrees", "h:worktrees", n) {
                let here = self.r.git.as_ref().map(|g| g.root.clone());
                for (k, w) in self.r.refs.worktrees.iter().enumerate() {
                    let name = w.rsplit(['/', '\\']).next().unwrap_or(w);
                    if matches(name) {
                        let mut it = SideItem::new(0, SideKind::Worktree, name, format!("w:{w}"));
                        // Its branch, unless the name already says it
                        // (agent worktrees: agent/<folder>).
                        let b = self.r.refs.worktree_branches.get(k).map(|b| b.to_string()).unwrap_or_default();
                        it.age = if b.rsplit('/').next() == Some(name) { String::new() } else { b };
                        it.current = here.as_ref().is_some_and(|h| same_path(h, std::path::Path::new(&**w)));
                        v.push(it);
                    }
                }
            }
        }
        self.r.side = v;
        let max = self.r.side.len().saturating_sub(self.side_visible_rows());
        self.r.side_top = self.r.side_top.min(max);
        self.damage.rects.extend([self.lay.side, self.lay.views]);
    }

    fn side_index(&self, key: &str) -> Option<usize> {
        self.r.side.iter().position(|i| i.key == key)
    }

    pub(super) fn side_item_at(&self, x: i32, y: i32) -> Option<usize> {
        let l = self.side_list_rect();
        if !l.contains(x, y) {
            return None;
        }
        let i = self.r.side_top + ((y - l.y) / self.side_row_h().max(1)) as usize;
        (i < self.r.side.len()).then_some(i)
    }

    /// Makes `key` the sidebar selection, scrolled into view.
    pub(super) fn side_select(&mut self, key: &str) {
        self.r.side_sel = key.to_string();
        if let Some(i) = self.side_index(key) {
            let vis = self.side_visible_rows();
            if i < self.r.side_top {
                self.r.side_top = i;
            } else if i >= self.r.side_top + vis {
                self.r.side_top = i + 1 - vis;
            }
        }
        self.damage.rects.extend([self.lay.side, self.lay.views]);
    }

    /// The ref item for a commit chosen elsewhere (palette), if any.
    pub(super) fn side_select_ref(&mut self, name: &str, kind: RefKind) {
        let key = match kind {
            RefKind::Local => format!("b:{name}"),
            RefKind::Remote => format!("r:{name}"),
            RefKind::Tag => format!("t:{name}"),
            RefKind::Stash => format!("s:{name}"),
            _ => return,
        };
        // Open the folders on the way so the item is visible.
        let mut changed = false;
        let mut acc = String::new();
        let (prefix, rest) = key.split_at(2);
        acc.push_str(prefix);
        let parts: Vec<&str> = rest.split('/').collect();
        for p in &parts[..parts.len().saturating_sub(1)] {
            acc.push_str(p);
            acc.push('/');
            changed |= self.side_collapsed.remove(&acc);
        }
        if changed {
            self.save_collapsed();
            self.rebuild_side();
        }
        self.side_select(&key);
    }

    fn toggle_side(&mut self, i: usize) {
        let Some(it) = self.r.side.get(i) else { return };
        if it.expander.is_none() || !self.side_filter.is_empty() {
            return;
        }
        let key = it.key.clone();
        if !self.side_collapsed.remove(&key) {
            self.side_collapsed.insert(key.clone());
        }
        self.save_collapsed();
        self.rebuild_side();
        self.side_select(&key);
    }

    fn save_collapsed(&mut self) {
        let mut v: Vec<String> = self.side_collapsed.iter().cloned().collect();
        v.sort();
        self.state.set_list("sidebar.collapsed", &v);
    }

    /// Acts on an item: refs jump the history to their commit.
    fn side_activate(&mut self, i: usize) {
        let Some(it) = self.r.side.get(i).cloned() else { return };
        if !matches!(it.kind, SideKind::Header | SideKind::Folder | SideKind::Remote | SideKind::AddRemote) {
            self.close_agents();
        }
        self.side_select(&it.key);
        match it.kind {
            SideKind::Header | SideKind::Folder | SideKind::Remote => self.toggle_side(i),
            SideKind::Submodule => self.open_submodule(it.key.trim_start_matches("m:")),
            SideKind::AddRemote => self.open_dialog(super::menu::DialogKind::Name {
                act: super::menu::NameAct::AddRemoteUrl,
                field: TextField::default(),
                checkout: false,
            }),
            _ => match it.target.and_then(|t| self.r.by_id.get(&t).copied()) {
                Some(row) => {
                    self.leave_changes();
                    self.leave_files();
                    self.leave_activity();
                    self.select(row as usize)
                }
                None if it.target.is_some() => {
                    self.set_notice(format!("{} points to a commit that is not loaded yet.", it.label))
                }
                None => {}
            },
        }
    }

    pub(super) fn side_click(&mut self, x: i32, y: i32) {
        if self.side_here_rect().contains(x, y) {
            // The card leads to the working tree.
            return self.view_activate("changes");
        }
        if let Some(k) = self.view_at(x, y) {
            let key = self.views()[k].key;
            return self.view_activate(key);
        }
        if self.side_filter_rect().contains(x, y) {
            self.focus = Focus::SideFilter;
            self.damage.rects.push(self.lay.side);
            return;
        }
        let Some(i) = self.side_item_at(x, y) else { return };
        if self.focus != Focus::Side {
            self.focus = Focus::Side;
            self.damage.rects.extend([self.lay.details, self.lay.side]);
            self.damage.rows.push(self.r.sel);
        }
        // A double click on a branch or tag checks it out.
        if self.double && self.r.side.get(i).is_some_and(|it| it.key == self.r.side_sel) {
            let it = self.r.side[i].clone();
            let name = it.key.get(2..).unwrap_or("").to_string();
            if matches!(it.kind, SideKind::Branch | SideKind::RemoteBranch | SideKind::Tag)
                && !it.current
                && !name.is_empty()
            {
                return self.run_refop(crate::work::RefOp::Checkout(name));
            }
        }
        // Ctrl+click picks refs for a batch, Shift+click a range (FF-36).
        let (ctrl, shift) = std::mem::take(&mut self.click_mods);
        if (ctrl || shift) && self.r.side.get(i).is_some_and(|it| multi_kind(it.kind)) {
            self.side_pick(i, shift);
            return;
        }
        if !self.r.side_multi.is_empty() {
            self.r.side_multi.clear();
            self.damage.rects.push(self.lay.side);
        }
        // The chevron toggles; elsewhere the item activates (folders toggle too).
        self.side_activate(i);
    }

    /// Adds item `i` to the batch selection or takes it out (Ctrl+click);
    /// with `range`, everything from the sidebar selection to it of a kind
    /// that can be picked (Shift+click). The selection itself joins first.
    pub(super) fn side_pick(&mut self, i: usize, range: bool) {
        let Some(it) = self.r.side.get(i).cloned() else { return };
        let anchor = self.side_index(&self.r.side_sel.clone());
        if self.r.side_multi.is_empty()
            && let Some(a) = anchor
            && a != i
            && self.r.side.get(a).is_some_and(|s| multi_kind(s.kind))
        {
            self.r.side_multi.insert(self.r.side[a].key.clone());
        }
        match anchor.filter(|_| range) {
            Some(a) => {
                let (lo, hi) = if a <= i { (a, i) } else { (i, a) };
                for s in &self.r.side[lo..=hi] {
                    if multi_kind(s.kind) {
                        self.r.side_multi.insert(s.key.clone());
                    }
                }
            }
            None => {
                if !self.r.side_multi.remove(&it.key) {
                    self.r.side_multi.insert(it.key.clone());
                }
            }
        }
        self.r.side_sel = it.key.clone();
        let n = self.r.side_multi.len();
        if n > 1 {
            self.set_notice(format!("{n} selected: right-click one of them for push, delete or drop"));
        }
        self.damage.rects.push(self.lay.side);
    }

    pub(super) fn side_wheel(&mut self, n: i64) {
        let max = self.r.side.len().saturating_sub(self.side_visible_rows()) as i64;
        let top = (self.r.side_top as i64 + n).clamp(0, max) as usize;
        if top != self.r.side_top {
            self.r.side_top = top;
            self.damage.rects.push(self.lay.side);
        }
    }

    /// Keyboard in the sidebar. Returns false when the key is not for it.
    pub(super) fn side_key(&mut self, k: Key) -> bool {
        if self.focus == Focus::SideFilter {
            match k {
                Key::Char(c) => {
                    self.side_filter.push(c);
                    self.rebuild_side();
                }
                Key::Backspace => {
                    self.side_filter.pop();
                    self.rebuild_side();
                }
                Key::Esc => {
                    self.side_filter.clear();
                    self.focus = Focus::Side;
                    self.rebuild_side();
                }
                Key::Down | Key::Enter => {
                    self.focus = Focus::Side;
                    let first = self.r.side.iter().position(|i| i.target.is_some()).unwrap_or(0);
                    if let Some(key) = self.r.side.get(first).map(|i| i.key.clone()) {
                        self.side_select(&key);
                    }
                }
                _ => return false,
            }
            return true;
        }
        if self.focus != Focus::Side {
            return false;
        }
        let cur = self.side_index(&self.r.side_sel.clone()).unwrap_or(0);
        let n = self.r.side.len();
        match k {
            Key::Up | Key::Char('k') if cur > 0 => {
                let key = self.r.side[cur - 1].key.clone();
                self.side_select(&key);
            }
            Key::Down | Key::Char('j') if cur + 1 < n => {
                let key = self.r.side[cur + 1].key.clone();
                self.side_select(&key);
            }
            Key::Enter | Key::Char(' ') => self.side_activate(cur),
            Key::Esc if !self.r.side_multi.is_empty() => {
                self.r.side_multi.clear();
                self.damage.rects.push(self.lay.side);
            }
            Key::Right => {
                if self.r.side.get(cur).and_then(|i| i.expander) == Some(false) {
                    self.toggle_side(cur);
                }
            }
            Key::Left => {
                if self.r.side.get(cur).and_then(|i| i.expander) == Some(true) {
                    self.toggle_side(cur);
                } else if let Some(d) = self.r.side.get(cur).map(|i| i.depth)
                    && let Some(parent) =
                        (0..cur).rev().find(|&j| self.r.side[j].depth < d || self.r.side[j].kind == SideKind::Header)
                {
                    let key = self.r.side[parent].key.clone();
                    self.side_select(&key);
                }
            }
            Key::Char(c) if c != 'j' && c != 'k' && c != ' ' && c != '/' => {
                // Typing starts filtering.
                self.focus = Focus::SideFilter;
                self.side_filter.push(c);
                self.rebuild_side();
            }
            Key::Up | Key::Down | Key::Char('j') | Key::Char('k') => {}
            _ => return false,
        }
        true
    }

    pub(super) fn paint_side(&mut self, cv: &mut Canvas) {
        let r = self.lay.side;
        cv.fill(r, BG_SIDEBAR());
        cv.fill(Rect::new(r.right() - 1, r.y, 1, r.h), BORDER());
        self.paint_side_here(cv);
        self.paint_views(cv);
        let (ui, semi) = (self.f.ui, self.f.semi);
        // Filter field.
        let fr = self.side_filter_rect();
        let typing = self.focus == Focus::SideFilter;
        cv.round_border(fr, self.pf(6.0), if typing { ACCENT() } else { BORDER() }, BG_INPUT());
        let fcy = fr.y as f32 + fr.h as f32 / 2.0;
        let fx = fr.x as f32 + self.pf(12.0);
        cv.ring(fx + self.pf(3.5), fcy - self.pf(1.0), self.pf(4.0), self.pf(1.4), TEXT_3());
        cv.stroke(
            &[(fx + self.pf(6.5), fcy + self.pf(2.0)), (fx + self.pf(9.0), fcy + self.pf(4.5))],
            self.pf(1.4),
            TEXT_3(),
        );
        let fb = (fcy + self.pf(4.5)).round();
        let tx = fx + self.pf(16.0);
        if self.side_filter.is_empty() && !typing {
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                tx,
                fb,
                "Filter branches and tags",
                TEXT_4(),
                fr.w as f32 - self.pf(40.0),
            );
        } else {
            let filt = self.side_filter.clone();
            let end = self.te.draw(cv, ui, self.pf(12.5), tx, fb, &filt, TEXT(), fr.w as f32 - self.pf(40.0));
            if typing {
                cv.fill(
                    Rect::new(end as i32 + 1, fr.y + self.px(6.0), self.px(1.5).max(1), fr.h - self.px(12.0)),
                    ACCENT(),
                );
            }
        }

        let l = self.side_list_rect();
        let rh = self.side_row_h();
        let old = cv.set_clip(cv.clip().intersect(l));
        let items = std::mem::take(&mut self.r.side);
        let focus = self.focus == Focus::Side;
        for (k, it) in items.iter().enumerate().skip(self.r.side_top) {
            let y = l.y + (k - self.r.side_top) as i32 * rh;
            if y >= l.bottom() {
                break;
            }
            let row = Rect::new(l.x + self.px(6.0), y, l.w - self.px(12.0), rh);
            if cv.clip().intersect(row).is_empty() {
                continue;
            }
            let selected = it.key == self.r.side_sel;
            let picked = self.r.side_multi.contains(&it.key);
            if selected {
                cv.fill_round(row, self.pf(6.0), if focus { BG_SELECT() } else { BG_SELECT_DIM() });
            } else if picked {
                cv.fill_round(row, self.pf(6.0), BG_SELECT_DIM());
            } else if self.side_hover == Some(k) {
                cv.fill_round(row, self.pf(6.0), BG_HOVER());
            }
            if picked {
                // Picked for a batch: an accent bar on the left edge.
                cv.fill_round(
                    Rect::new(row.x, row.y + self.px(5.0), self.px(3.0).max(2), row.h - self.px(10.0)),
                    self.pf(1.5),
                    ACCENT(),
                );
            }
            let cy = y as f32 + rh as f32 / 2.0;
            let base = (cy + self.pf(4.5)).round();
            let indent = it.depth as f32 * self.pf(14.0);
            let cx = row.x as f32 + self.pf(12.0) + indent;
            if let Some(open) = it.expander {
                self.chevron(cv, cx, cy, open, TEXT_3());
            }
            if it.kind == SideKind::Header {
                self.te.draw(
                    cv,
                    semi,
                    self.pf(11.0),
                    cx + self.pf(10.0),
                    base - self.pf(0.5),
                    &it.label.to_uppercase(),
                    TEXT_3(),
                    row.w as f32,
                );
                if !it.badge.is_empty() {
                    let bw = self.te.measure(semi, self.pf(10.5), &it.badge);
                    let pw = bw + self.pf(14.0);
                    let px = row.right() as f32 - pw - self.pf(6.0);
                    let pr = Rect::new(px as i32, (cy - self.pf(8.5)) as i32, pw as i32, self.px(17.0));
                    cv.fill_round(pr, self.pf(8.5), BG_HOVER());
                    self.te.draw(
                        cv,
                        semi,
                        self.pf(10.5),
                        px + self.pf(7.0),
                        base - self.pf(1.0),
                        &it.badge,
                        TEXT_3(),
                        bw + 2.0,
                    );
                }
                continue;
            }
            let ix = cx + if it.expander.is_some() { self.pf(16.0) } else { self.pf(4.0) };
            let (icon, text) = match it.kind {
                SideKind::Branch if it.current => (ACCENT(), TEXT()),
                SideKind::Branch => (LOCAL_FG(), TEXT_2()),
                SideKind::Folder => (TEXT_3(), TEXT_2()),
                SideKind::Remote => (REMOTE_FG(), TEXT_2()),
                SideKind::RemoteBranch => (REMOTE_FG(), TEXT_3()),
                SideKind::Tag => (TAG_FG(), TEXT_2()),
                SideKind::Stash => (STASH_FG(), TEXT_2()),
                SideKind::Worktree | SideKind::Submodule | SideKind::PullRequest => (TEXT_3(), TEXT_2()),
                SideKind::Header => (TEXT_3(), TEXT_3()),
                SideKind::AddRemote => (ACCENT(), TEXT_3()),
            };
            // A branch wears its lane's colour from the history graph.
            let lane = it
                .target
                .filter(|_| matches!(it.kind, SideKind::Branch | SideKind::RemoteBranch))
                .and_then(|t| self.r.by_id.get(&t))
                .and_then(|&row| self.r.graph.rows.get(row as usize))
                .map(|g| crate::theme::lane(g.color));
            let icon = lane.filter(|_| !it.current).unwrap_or(icon);
            // A branch untouched for a month steps back.
            let stale = it.kind == SideKind::Branch && !it.current && it.age_secs > 30 * 86_400;
            let text = if selected {
                TEXT()
            } else if stale {
                TEXT_4()
            } else {
                text
            };
            if it.current && !selected && it.kind == SideKind::Branch {
                cv.fill_round(row, self.pf(6.0), ACCENT_SOFT());
            }
            match it.kind {
                SideKind::Branch if it.current => self.check(cv, ix, cy, icon),
                SideKind::Branch | SideKind::RemoteBranch => self.branch_icon(cv, ix, cy, icon),
                SideKind::Folder => self.folder_icon(cv, ix, cy, icon),
                SideKind::Tag => {
                    let s = self.pf(4.0);
                    cv.stroke(
                        &[(ix - s, cy), (ix, cy - s), (ix + s, cy), (ix, cy + s), (ix - s, cy)],
                        self.pf(1.5),
                        icon,
                    );
                }
                SideKind::Remote => {
                    cv.ring(ix, cy, self.pf(5.0), self.pf(1.5), icon);
                    cv.stroke(&[(ix - self.pf(5.0), cy), (ix + self.pf(5.0), cy)], self.pf(1.2), icon);
                }
                SideKind::Stash => {
                    let b =
                        Rect::new((ix - self.pf(5.0)) as i32, (cy - self.pf(4.0)) as i32, self.px(10.0), self.px(8.0));
                    cv.round_border(b, self.pf(2.0), icon, BG_SIDEBAR());
                }
                SideKind::Submodule => {
                    // A box inside a box: a repository in the repository.
                    let o =
                        Rect::new((ix - self.pf(5.0)) as i32, (cy - self.pf(5.0)) as i32, self.px(10.0), self.px(10.0));
                    cv.round_border(o, self.pf(2.0), icon, BG_SIDEBAR());
                    cv.fill(
                        Rect::new((ix - self.pf(1.5)) as i32, (cy - self.pf(1.5)) as i32, self.px(3.0), self.px(3.0)),
                        icon,
                    );
                }
                SideKind::PullRequest => {
                    // Two dots joined: a branch asking to merge.
                    cv.fill_circle(ix - self.pf(3.0), cy - self.pf(3.5), self.pf(1.8), icon);
                    cv.fill_circle(ix + self.pf(3.0), cy + self.pf(3.5), self.pf(1.8), icon);
                    cv.stroke(
                        &[(ix - self.pf(3.0), cy - self.pf(3.5)), (ix - self.pf(3.0), cy + self.pf(4.5))],
                        self.pf(1.3),
                        icon,
                    );
                    cv.stroke(
                        &[(ix - self.pf(3.0), cy - self.pf(1.0)), (ix + self.pf(3.0), cy + self.pf(3.5))],
                        self.pf(1.3),
                        icon,
                    );
                }
                SideKind::Worktree if it.current => self.check(cv, ix, cy, ACCENT()),
                SideKind::Worktree if it.key.contains(".ffwd-agents") => {
                    self.glyph(cv, super::Glyph::Terminal, ix, cy, self.pf(5.5), icon)
                }
                SideKind::Worktree => self.folder_icon(cv, ix, cy, icon),
                SideKind::AddRemote => self.glyph(cv, super::Glyph::Plus, ix, cy, self.pf(5.0), icon),
                SideKind::Header => {}
            }
            let font = if it.current || selected { semi } else { ui };
            let lx = ix + self.pf(14.0);
            let note = if it.badge.is_empty() { it.age.clone() } else { String::new() };
            let note_font = if it.kind == SideKind::Worktree { self.f.mono } else { ui };
            let note_w = if note.is_empty() {
                0.0
            } else {
                (self.te.measure(note_font, self.pf(11.0), &note) + self.pf(12.0)).min(row.w as f32 * 0.45)
            };
            let badge_w = if it.badge.is_empty() {
                note_w
            } else if it.kind == SideKind::Branch {
                let parts = it.badge.split_whitespace().count() as f32;
                self.te.measure(semi, self.pf(10.5), &it.badge) + parts * self.pf(16.0) + self.pf(8.0)
            } else {
                self.te.measure(semi, self.pf(11.0), &it.badge) + self.pf(14.0)
            };
            // Long names keep both ends (they often differ only at the end).
            let lw = row.right() as f32 - lx - badge_w - self.pf(6.0);
            let fs = self.pf(13.0);
            let label = super::fit_middle(&mut self.te, font, fs, &it.label, lw);
            self.te.draw(cv, font, self.pf(13.0), lx, base, &label, text, lw + 2.0);
            if !note.is_empty() {
                let nw = note_w - self.pf(8.0);
                let nfs = self.pf(11.0);
                let shown = super::fit_middle(&mut self.te, note_font, nfs, &note, nw);
                let tw = self.te.measure(note_font, self.pf(11.0), &shown);
                self.te.draw(
                    cv,
                    note_font,
                    self.pf(11.0),
                    row.right() as f32 - tw - self.pf(10.0),
                    base - self.pf(0.5),
                    &shown,
                    match it.age_secs {
                        s if s > 0 && s < 86_400 => TEXT_3(),
                        _ => TEXT_4(),
                    },
                    nw + 2.0,
                );
            }
            if it.kind == SideKind::Branch && !it.badge.is_empty() {
                // Sync as chips: ↑ ahead (accent), ↓ behind (remote blue),
                // gone (red), from the right.
                let mut x = row.right() as f32 - self.pf(8.0);
                for part in it.badge.split_whitespace().collect::<Vec<_>>().into_iter().rev() {
                    let (bg, fg) = match part {
                        p if p.starts_with('↑') => (ACCENT_SOFT(), ACCENT_FG()),
                        p if p.starts_with('↓') => (REMOTE_BG(), REMOTE_FG()),
                        _ => (DEL_BG(), DEL_TEXT()),
                    };
                    let w = self.te.measure(semi, self.pf(10.5), part) + self.pf(12.0);
                    x -= w;
                    let r = Rect::new(x as i32, (cy - self.pf(8.5)) as i32, w as i32, self.px(17.0));
                    cv.fill_round(r, self.pf(8.5), bg);
                    self.te.draw(cv, semi, self.pf(10.5), x + self.pf(6.0), base - self.pf(1.0), part, fg, w);
                    x -= self.pf(4.0);
                }
            } else if !it.badge.is_empty() {
                let bw = badge_w - self.pf(14.0);
                let bx = row.right() as f32 - bw - self.pf(10.0);
                let color = match it.kind {
                    // A gone upstream reads as a warning.
                    SideKind::Branch if it.badge.starts_with("gone") => DEL(),
                    SideKind::Branch => ACCENT(),
                    _ => TEXT_4(),
                };
                self.te.draw(cv, semi, self.pf(11.0), bx, base - self.pf(0.5), &it.badge, color, bw + 8.0);
            }
        }
        // Scroll thumb when the list is longer than the panel.
        let vis = self.side_visible_rows();
        if items.len() > vis {
            let th = ((l.h as f32 * vis as f32 / items.len() as f32) as i32).max(self.px(20.0));
            let ty = l.y + ((l.h - th) as f32 * self.r.side_top as f32 / (items.len() - vis) as f32) as i32;
            cv.fill_round(Rect::new(l.right() - self.px(5.0), ty, self.px(3.0), th), self.pf(1.5), BORDER_HOVER());
        }
        self.r.side = items;
        cv.set_clip(old);
    }

    /// Snapshots: select a local branch in the sidebar the way a click does.
    pub fn snapshot_side(&mut self, name: &str) {
        self.side_select_ref(name, RefKind::Local);
        let key = format!("b:{name}");
        if let Some(i) = self.side_index(&key) {
            self.focus = Focus::Side;
            self.side_activate(i);
        }
    }

    pub(super) fn side_pointer(&mut self, x: i32, y: i32) {
        let here = self.side_here_rect().contains(x, y);
        if here != self.side_here_hot {
            self.side_here_hot = here;
            let r = self.side_here_rect();
            self.damage.rects.push(r);
        }
        let d = self.view_at(x, y);
        if d != self.side_view_hot {
            self.side_view_hot = d;
            self.damage.rects.extend([self.lay.side, self.lay.views]);
        }
        let h = self.side_item_at(x, y);
        if h != self.side_hover {
            self.side_hover = h;
            self.damage.rects.push(self.lay.side);
        }
    }

    fn chevron(&self, cv: &mut Canvas, cx: f32, cy: f32, open: bool, color: u32) {
        let s = self.pf(3.0);
        let w = self.pf(1.4);
        if open {
            cv.stroke(&[(cx - s, cy - s / 2.0), (cx, cy + s / 2.0), (cx + s, cy - s / 2.0)], w, color);
        } else {
            cv.stroke(&[(cx - s / 2.0, cy - s), (cx + s / 2.0, cy), (cx - s / 2.0, cy + s)], w, color);
        }
    }

    pub(super) fn check(&self, cv: &mut Canvas, cx: f32, cy: f32, color: u32) {
        let s = self.pf(4.5);
        cv.stroke(&[(cx - s, cy), (cx - s * 0.3, cy + s * 0.7), (cx + s, cy - s * 0.8)], self.pf(1.8), color);
    }

    fn folder_icon(&self, cv: &mut Canvas, cx: f32, cy: f32, color: u32) {
        let (w, h) = (self.pf(12.0), self.pf(9.0));
        let body = Rect::new((cx - w / 2.0) as i32, (cy - h / 2.0 + self.pf(1.0)) as i32, w as i32, h as i32);
        cv.fill(Rect::new(body.x, body.y - self.px(2.0), (w * 0.45) as i32, self.px(3.0)), color);
        cv.round_border(body, self.pf(2.0), color, BG_SIDEBAR());
    }
}

/// Descending order that compares digit runs as numbers (v0.10 > v0.9).
/// An age in a few characters: 5m, 3h, 2d, 6w, 1y.
pub(super) fn short_age(secs: i64) -> String {
    match secs.max(0) {
        s if s < 3600 => format!("{}m", (s / 60).max(1)),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 14 * 86_400 => format!("{}d", s / 86_400),
        s if s < 365 * 86_400 => format!("{}w", s / (7 * 86_400)),
        s => format!("{}y", s / (365 * 86_400)),
    }
}

/// Items that can be picked for a batch (FF-36): branches, remote
/// branches, tags and stashes.
pub(super) fn multi_kind(k: SideKind) -> bool {
    matches!(k, SideKind::Branch | SideKind::RemoteBranch | SideKind::Tag | SideKind::Stash)
}

pub(super) fn natural_desc(a: &str, b: &str) -> std::cmp::Ordering {
    natural(b, a)
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    x.next();
                }
                let mut nb = String::new();
                while let Some(c) = y.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    y.next();
                }
                let o = na.trim_start_matches('0').len().cmp(&nb.trim_start_matches('0').len()).then(na.cmp(&nb));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(p), Some(q)) => {
                let o = p.cmp(&q);
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["v0.1.0-beta.9", "v0.1.0-beta.10", "v0.1.0-beta.2"];
        v.sort_by(|a, b| natural_desc(a, b));
        assert_eq!(v, vec!["v0.1.0-beta.10", "v0.1.0-beta.9", "v0.1.0-beta.2"]);
    }

    fn r(name: &str) -> Ref {
        Ref {
            kind: RefKind::Local,
            name: name.into(),
            target: Oid::default(),
            upstream: None,
            ahead: 0,
            behind: 0,
            is_head: name == "main",
            upstream_gone: false,
            time: 0,
        }
    }

    #[test]
    fn branches_group_into_folders() {
        let refs = [r("main"), r("t3/a"), r("t3/b"), r("feat/x/y"), r("zed")];
        let items: Vec<(String, &Ref)> = refs.iter().map(|x| (x.name.to_string(), x)).collect();
        let mut out = Vec::new();
        let mut collapsed = HashSet::new();
        tree(&mut out, items.clone(), 0, "b:", SideKind::Branch, &collapsed, false, false);
        let labels: Vec<(u8, &str)> = out.iter().map(|i| (i.depth, i.label.as_str())).collect();
        assert_eq!(
            labels,
            vec![(0, "feat"), (1, "x"), (2, "y"), (0, "t3"), (1, "a"), (1, "b"), (0, "main"), (0, "zed")]
        );
        assert_eq!(out[5].key, "b:t3/b");
        assert!(out.iter().find(|i| i.label == "main").unwrap().current);
        // A collapsed folder hides its branches, but filtering opens it.
        collapsed.insert("b:t3/".to_string());
        let mut out = Vec::new();
        tree(&mut out, items.clone(), 0, "b:", SideKind::Branch, &collapsed, false, false);
        assert!(!out.iter().any(|i| i.label == "a"));
        let mut out = Vec::new();
        tree(&mut out, items, 0, "b:", SideKind::Branch, &collapsed, true, false);
        assert!(out.iter().any(|i| i.label == "a"));
    }

    #[test]
    fn branches_sort_by_most_recent_commit() {
        let mut refs = [r("main"), r("t3/a"), r("t3/b"), r("old")];
        for (x, t) in refs.iter_mut().zip([100, 300, 50, 10]) {
            x.time = t;
        }
        let items: Vec<(String, &Ref)> = refs.iter().map(|x| (x.name.to_string(), x)).collect();
        let mut out = Vec::new();
        tree(&mut out, items, 0, "b:", SideKind::Branch, &HashSet::new(), false, true);
        let labels: Vec<&str> = out.iter().map(|i| i.label.as_str()).collect();
        // The t3 folder holds the newest branch; inside it, newest first.
        assert_eq!(labels, ["t3", "a", "b", "main", "old"]);
    }
}
