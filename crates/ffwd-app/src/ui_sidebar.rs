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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SideKind {
    Header,
    Changes,
    Files,
    Activity,
    All,
    Branch,
    Folder,
    Remote,
    RemoteBranch,
    Tag,
    Stash,
    Worktree,
    Submodule,
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
        }
    }
}

/// Adds `refs` as a tree: names with `/` group into folders (folders
/// first, then branches, each sorted) `t3/…` folders.
fn tree(
    out: &mut Vec<SideItem>,
    items: Vec<(String, &Ref)>,
    depth: u8,
    prefix: &str,
    kind: SideKind,
    collapsed: &HashSet<String>,
    filtering: bool,
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
    for (seg, children) in folders {
        let key = format!("{prefix}{seg}/");
        let open = filtering || !collapsed.contains(&key);
        let mut f = SideItem::new(depth, SideKind::Folder, &seg, key.clone());
        f.badge = fmt::num(children.len());
        f.expander = Some(open);
        out.push(f);
        if open {
            tree(out, children, depth + 1, &key, kind, collapsed, filtering);
        }
    }
    leaves.sort_by_key(|a| a.0.to_lowercase());
    for (name, r) in leaves {
        let mut it = SideItem::new(depth, kind, &name, format!("{prefix}{name}"));
        it.target = Some(r.target);
        it.current = r.is_head;
        if kind == SideKind::Branch {
            let mut b = String::new();
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
        }
        out.push(it);
    }
}

impl Ui {
    pub(super) fn side_row_h(&self) -> i32 {
        self.px(SIDE_ROW)
    }

    /// The filter field at the top of the sidebar.
    pub(super) fn side_filter_rect(&self) -> Rect {
        let s = self.lay.side;
        Rect::new(s.x + self.px(10.0), s.y + self.px(8.0), s.w - self.px(20.0), self.px(FILTER_H - 14.0))
    }

    pub(super) fn side_list_rect(&self) -> Rect {
        let s = self.lay.side;
        let top = s.y + self.px(FILTER_H);
        Rect::new(s.x, top, s.w - 1, (s.bottom() - top).max(0))
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

        if !filtering && header(&mut v, "Workspace", "h:workspace", String::new()) {
            let mut c = SideItem::new(0, SideKind::Changes, "Local Changes", "changes".into());
            c.badge = self
                .r
                .status
                .map_or(String::new(), |s| if s.total() > 0 { fmt::num(s.total() as usize) } else { String::new() });
            v.push(c);
            v.push(SideItem::new(0, SideKind::All, "All Commits", "all".into()));
            v.push(SideItem::new(0, SideKind::Files, "Files", "files".into()));
            let mut a = SideItem::new(0, SideKind::Activity, "Activity", "activity".into());
            if self.activity.unseen_failures > 0 {
                a.badge = format!("{} failed", self.activity.unseen_failures);
            }
            v.push(a);
        }
        let refs = &self.r.refs.refs;
        let locals: Vec<(String, &Ref)> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Local && matches(&r.name))
            .map(|r| (r.name.to_string(), r))
            .collect();
        if !locals.is_empty() || !filtering {
            let n = fmt::num(locals.len());
            if header(&mut v, "Branches", "h:branches", n) {
                tree(&mut v, locals, 0, "b:", SideKind::Branch, collapsed, filtering);
            }
        }
        let remotes: Vec<&Ref> = refs.iter().filter(|r| r.kind == RefKind::Remote && matches(&r.name)).collect();
        // Configured remotes show even before their first fetch; the header
        // always shows (its menu adds a remote).
        let configured = &self.r.refs.remotes;
        if (!filtering || !remotes.is_empty()) && header(&mut v, "Remotes", "h:remotes", fmt::num(configured.len())) {
            let mut names: Vec<&str> = remotes.iter().filter_map(|r| r.name.split('/').next()).collect();
            if !filtering {
                names.extend(configured.iter().map(|r| &*r.name));
            }
            names.sort_unstable();
            names.dedup();
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
                    tree(&mut v, rs, 1, &key, SideKind::RemoteBranch, collapsed, filtering);
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
        if !filtering || !stashes.is_empty() {
            let n = fmt::num(stashes.len());
            if header(&mut v, "Stashes", "h:stashes", n) {
                for (name, msg) in stashes {
                    v.push(SideItem::new(0, SideKind::Stash, msg, format!("s:{name}")));
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
        if self.r.refs.worktrees.len() > 1 {
            let n = fmt::num(self.r.refs.worktrees.len());
            if header(&mut v, "Worktrees", "h:worktrees", n) {
                for w in &self.r.refs.worktrees {
                    let name = w.rsplit(['/', '\\']).next().unwrap_or(w);
                    if matches(name) {
                        v.push(SideItem::new(0, SideKind::Worktree, name, format!("w:{w}")));
                    }
                }
            }
        }
        self.r.side = v;
        let max = self.r.side.len().saturating_sub(self.side_visible_rows());
        self.r.side_top = self.r.side_top.min(max);
        self.damage.rects.push(self.lay.side);
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
        self.damage.rects.push(self.lay.side);
    }

    /// The ref item for a commit chosen elsewhere (palette), if any.
    pub(super) fn side_select_ref(&mut self, name: &str, kind: RefKind) {
        let key = match kind {
            RefKind::Local => format!("b:{name}"),
            RefKind::Remote => format!("r:{name}"),
            RefKind::Tag => format!("t:{name}"),
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
        self.side_select(&it.key);
        match it.kind {
            SideKind::Header | SideKind::Folder | SideKind::Remote => self.toggle_side(i),
            SideKind::Changes => self.enter_changes(),
            SideKind::Files => self.enter_files(),
            SideKind::Submodule => self.open_submodule(it.key.trim_start_matches("m:")),
            SideKind::Activity => self.enter_activity(),
            SideKind::All => {
                self.leave_changes();
                self.leave_files();
                self.leave_activity();
            }
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
        // The chevron toggles; elsewhere the item activates (folders toggle too).
        self.side_activate(i);
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
        cv.fill(r, BG_SIDEBAR);
        cv.fill(Rect::new(r.right() - 1, r.y, 1, r.h), BORDER);
        let (ui, semi) = (self.f.ui, self.f.semi);
        // Filter field.
        let fr = self.side_filter_rect();
        let typing = self.focus == Focus::SideFilter;
        cv.round_border(fr, self.pf(6.0), if typing { ACCENT } else { BORDER }, BG_INPUT);
        let fcy = fr.y as f32 + fr.h as f32 / 2.0;
        let fx = fr.x as f32 + self.pf(12.0);
        cv.ring(fx + self.pf(3.5), fcy - self.pf(1.0), self.pf(4.0), self.pf(1.4), TEXT_3);
        cv.stroke(
            &[(fx + self.pf(6.5), fcy + self.pf(2.0)), (fx + self.pf(9.0), fcy + self.pf(4.5))],
            self.pf(1.4),
            TEXT_3,
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
                TEXT_4,
                fr.w as f32 - self.pf(40.0),
            );
        } else {
            let filt = self.side_filter.clone();
            let end = self.te.draw(cv, ui, self.pf(12.5), tx, fb, &filt, TEXT, fr.w as f32 - self.pf(40.0));
            if typing {
                cv.fill(
                    Rect::new(end as i32 + 1, fr.y + self.px(6.0), self.px(1.5).max(1), fr.h - self.px(12.0)),
                    ACCENT,
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
            if selected {
                cv.fill_round(row, self.pf(6.0), if focus { BG_SELECT } else { BG_SELECT_DIM });
            } else if self.side_hover == Some(k) {
                cv.fill_round(row, self.pf(6.0), BG_HOVER);
            }
            let cy = y as f32 + rh as f32 / 2.0;
            let base = (cy + self.pf(4.5)).round();
            let indent = it.depth as f32 * self.pf(14.0);
            let cx = row.x as f32 + self.pf(12.0) + indent;
            if let Some(open) = it.expander {
                self.chevron(cv, cx, cy, open, TEXT_3);
            }
            if it.kind == SideKind::Header {
                self.te.draw(
                    cv,
                    semi,
                    self.pf(11.0),
                    cx + self.pf(10.0),
                    base - self.pf(0.5),
                    &it.label.to_uppercase(),
                    TEXT_3,
                    row.w as f32,
                );
                if !it.badge.is_empty() {
                    let bw = self.te.measure(ui, self.pf(11.0), &it.badge);
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(11.0),
                        row.right() as f32 - bw - self.pf(10.0),
                        base - self.pf(0.5),
                        &it.badge,
                        TEXT_4,
                        bw + 2.0,
                    );
                }
                continue;
            }
            let ix = cx + if it.expander.is_some() { self.pf(16.0) } else { self.pf(4.0) };
            let (icon, text) = match it.kind {
                SideKind::Branch if it.current => (ACCENT, TEXT),
                SideKind::Branch => (LOCAL_FG, TEXT_2),
                SideKind::Folder => (TEXT_3, TEXT_2),
                SideKind::Remote => (REMOTE_FG, TEXT_2),
                SideKind::RemoteBranch => (REMOTE_FG, TEXT_3),
                SideKind::Tag => (TAG_FG, TEXT_2),
                SideKind::Stash => (STASH_FG, TEXT_2),
                SideKind::Changes => (MOD, TEXT),
                SideKind::All | SideKind::Files | SideKind::Activity => (TEXT_2, TEXT),
                SideKind::Worktree | SideKind::Submodule => (TEXT_3, TEXT_2),
                SideKind::Header => (TEXT_3, TEXT_3),
            };
            let text = if selected { TEXT } else { text };
            match it.kind {
                SideKind::Branch if it.current => self.check(cv, ix, cy, icon),
                SideKind::Branch | SideKind::RemoteBranch => self.branch_icon(cv, ix, cy, icon),
                SideKind::Folder | SideKind::Files => self.folder_icon(cv, ix, cy, icon),
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
                    cv.round_border(b, self.pf(2.0), icon, BG_SIDEBAR);
                }
                SideKind::Changes => cv.fill_circle(ix, cy, self.pf(4.0), icon),
                SideKind::Activity => {
                    // A pulse line.
                    let s = self.pf(5.0);
                    cv.stroke(
                        &[
                            (ix - s, cy),
                            (ix - s * 0.4, cy),
                            (ix - s * 0.1, cy - s * 0.8),
                            (ix + s * 0.3, cy + s * 0.8),
                            (ix + s * 0.6, cy),
                            (ix + s, cy),
                        ],
                        self.pf(1.4),
                        icon,
                    );
                }
                SideKind::All => {
                    for j in 0..3 {
                        let yy = cy - self.pf(4.0) + j as f32 * self.pf(4.0);
                        cv.stroke(&[(ix - self.pf(5.0), yy), (ix + self.pf(5.0), yy)], self.pf(1.5), icon);
                    }
                }
                SideKind::Submodule => {
                    // A box inside a box: a repository in the repository.
                    let o =
                        Rect::new((ix - self.pf(5.0)) as i32, (cy - self.pf(5.0)) as i32, self.px(10.0), self.px(10.0));
                    cv.round_border(o, self.pf(2.0), icon, BG_SIDEBAR);
                    cv.fill(
                        Rect::new((ix - self.pf(1.5)) as i32, (cy - self.pf(1.5)) as i32, self.px(3.0), self.px(3.0)),
                        icon,
                    );
                }
                SideKind::Worktree => {
                    let b =
                        Rect::new((ix - self.pf(5.0)) as i32, (cy - self.pf(5.0)) as i32, self.px(10.0), self.px(10.0));
                    cv.round_border(b, self.pf(2.5), icon, BG_SIDEBAR);
                }
                SideKind::Header => {}
            }
            let font = if it.current || selected { semi } else { ui };
            let lx = ix + self.pf(14.0);
            let badge_w =
                if it.badge.is_empty() { 0.0 } else { self.te.measure(semi, self.pf(11.0), &it.badge) + self.pf(14.0) };
            self.te.draw(
                cv,
                font,
                self.pf(13.0),
                lx,
                base,
                &it.label,
                text,
                row.right() as f32 - lx - badge_w - self.pf(6.0),
            );
            if !it.badge.is_empty() {
                let bw = badge_w - self.pf(14.0);
                let bx = row.right() as f32 - bw - self.pf(10.0);
                let color = match it.kind {
                    SideKind::Changes => MOD,
                    SideKind::Activity => DEL,
                    SideKind::Branch => ACCENT,
                    _ => TEXT_4,
                };
                if it.kind == SideKind::Changes {
                    let b = Rect::new(
                        (bx - self.pf(6.0)) as i32,
                        (cy - self.pf(9.0)) as i32,
                        (bw + self.pf(12.0)) as i32,
                        self.px(18.0),
                    );
                    cv.fill_round(b, self.pf(9.0), ACCENT_SOFT);
                }
                self.te.draw(cv, semi, self.pf(11.0), bx, base - self.pf(0.5), &it.badge, color, bw + 2.0);
            }
        }
        // Scroll thumb when the list is longer than the panel.
        let vis = self.side_visible_rows();
        if items.len() > vis {
            let th = ((l.h as f32 * vis as f32 / items.len() as f32) as i32).max(self.px(20.0));
            let ty = l.y + ((l.h - th) as f32 * self.r.side_top as f32 / (items.len() - vis) as f32) as i32;
            cv.fill_round(Rect::new(l.right() - self.px(5.0), ty, self.px(3.0), th), self.pf(1.5), BORDER_HOVER);
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
        cv.round_border(body, self.pf(2.0), color, BG_SIDEBAR);
    }
}

/// Descending order that compares digit runs as numbers (v0.10 > v0.9).
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
        }
    }

    #[test]
    fn branches_group_into_folders() {
        let refs = [r("main"), r("t3/a"), r("t3/b"), r("feat/x/y"), r("zed")];
        let items: Vec<(String, &Ref)> = refs.iter().map(|x| (x.name.to_string(), x)).collect();
        let mut out = Vec::new();
        let mut collapsed = HashSet::new();
        tree(&mut out, items.clone(), 0, "b:", SideKind::Branch, &collapsed, false);
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
        tree(&mut out, items.clone(), 0, "b:", SideKind::Branch, &collapsed, false);
        assert!(!out.iter().any(|i| i.label == "a"));
        let mut out = Vec::new();
        tree(&mut out, items, 0, "b:", SideKind::Branch, &collapsed, true);
        assert!(out.iter().any(|i| i.label == "a"));
    }
}
