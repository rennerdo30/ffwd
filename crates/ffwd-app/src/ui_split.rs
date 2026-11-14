//! Movable pane borders (FF-57): every border between two panes can be
//! dragged, vertical and horizontal ones alike. The pointer shows a resize
//! cursor and the border an accent line; a double click puts the default
//! back. Sizes are kept in the window state (`ui.split.*`), so they survive
//! a restart.

use super::*;

/// A border the user can drag.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Split {
    /// Sidebar | everything else (its width).
    Side,
    /// History list and commit details (below: the list's height share;
    /// beside: the list's width share).
    List,
    /// Commit header and files | diff, when the details are below.
    Info,
    /// Local Changes: the lists | the diff.
    ChangesLeft,
    /// Local Changes: unstaged / staged (the unstaged share).
    ChangesLists,
    /// Local Changes: the lists / the commit box (its height).
    ChangesCommit,
    /// Files: the tree | the file.
    FilesTree,
    /// Agents: the rail | the run.
    AgentsRail,
}

impl Split {
    fn key(self) -> &'static str {
        match self {
            Split::Side => "ui.split.side",
            Split::List => "ui.split.list",
            Split::Info => "ui.split.info",
            Split::ChangesLeft => "ui.split.changes_left",
            Split::ChangesLists => "ui.split.changes_lists",
            Split::ChangesCommit => "ui.split.changes_commit",
            Split::FilesTree => "ui.split.files_tree",
            Split::AgentsRail => "ui.split.agents_rail",
        }
    }

    /// A vertical border (dragged sideways), else horizontal.
    pub fn vertical(self, beside: bool) -> bool {
        match self {
            Split::List => beside,
            Split::ChangesLists | Split::ChangesCommit => false,
            _ => true,
        }
    }
}

impl Ui {
    /// A dragged size: logical pixels, or a share (0..1) for List and
    /// ChangesLists. None: the default.
    pub(super) fn split_get(&self, s: Split) -> Option<f32> {
        self.state.get(s.key()).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite() && *v > 0.0)
    }

    fn split_set(&mut self, s: Split, v: Option<f32>) {
        match v {
            Some(v) => self.state.set_string(s.key(), &format!("{v:.3}")),
            None => self.state.remove(s.key()),
        }
    }

    /// The borders on screen now, each with its line (1 px).
    pub(super) fn splitters(&self) -> Vec<(Split, Rect)> {
        let mut v = Vec::new();
        if self.settings_view.is_some() || self.tabs.is_empty() {
            return v;
        }
        let side = self.lay.side;
        if side.w > 0 {
            v.push((Split::Side, Rect::new(side.right() - 1, side.y, 1, side.h)));
        }
        if self.agent_view.is_some() {
            let rail = self.agents_rail();
            v.push((Split::AgentsRail, Rect::new(rail.right() - 1, rail.y, 1, rail.h)));
            return v;
        }
        if self.r.changes_view {
            let l = self.changes_layout();
            v.push((Split::ChangesLeft, Rect::new(l.left.right(), l.area.y, 1, l.area.h)));
            v.push((Split::ChangesLists, Rect::new(l.left.x, l.heads[1].y, l.left.w, 1)));
            v.push((Split::ChangesCommit, Rect::new(l.left.x, l.commit.y, l.left.w, 1)));
            return v;
        }
        if self.r.files_view {
            let f = self.files_layout();
            v.push((Split::FilesTree, Rect::new(f.tree.right(), f.area.y, 1, f.area.h)));
            return v;
        }
        if self.r.activity_view || self.r.reflog_view || self.diff_max {
            return v;
        }
        let d = self.lay.details;
        match self.layout {
            DetailsLayout::Below => {
                v.push((Split::List, Rect::new(d.x, d.y, d.w, 1)));
                v.push((Split::Info, Rect::new(self.lay.info.right(), d.y, 1, d.h)));
            }
            DetailsLayout::Beside => v.push((Split::List, Rect::new(d.x, d.y, 1, d.h))),
        }
        v
    }

    /// The border under (x, y): a few pixels either side of its line.
    pub(super) fn split_at(&self, x: i32, y: i32) -> Option<Split> {
        let pad = self.px(4.0);
        let beside = self.layout == DetailsLayout::Beside;
        self.splitters()
            .into_iter()
            .find(|(s, r)| {
                let grab = if s.vertical(beside) { r.inset(-pad, 0) } else { r.inset(0, -pad) };
                grab.contains(x, y)
            })
            .map(|(s, _)| s)
    }

    /// A press on a border: a drag starts; a double click resets it.
    pub(super) fn split_press(&mut self, x: i32, y: i32) -> bool {
        let Some(s) = self.split_at(x, y) else { return false };
        if self.double {
            self.split_set(s, None);
            self.split_drag = None;
            self.relayout();
            return true;
        }
        self.split_drag = Some(s);
        true
    }

    /// The pointer moved while a border is dragged.
    pub(super) fn split_move(&mut self, x: i32, y: i32) {
        let Some(s) = self.split_drag else { return };
        let k = self.s.max(0.1);
        let v = match s {
            Split::Side => x as f32 / k,
            Split::List => {
                let side = self.lay.side;
                if self.layout == DetailsLayout::Beside {
                    (x - side.right()) as f32 / (self.w as i32 - side.right()).max(1) as f32
                } else {
                    (y - side.y) as f32 / side.h.max(1) as f32
                }
            }
            Split::Info => (x - self.lay.details.x) as f32 / k,
            Split::ChangesLeft => (x - self.lay.side.right()) as f32 / k,
            Split::ChangesLists => {
                let l = self.changes_layout();
                let total = (l.commit.y - l.heads[0].bottom() - l.heads[1].h).max(1);
                (y - l.heads[0].bottom()) as f32 / total as f32
            }
            Split::ChangesCommit => {
                let l = self.changes_layout();
                (l.left.bottom() - y) as f32 / k
            }
            Split::FilesTree => (x - self.lay.side.right()) as f32 / k,
            Split::AgentsRail => (x - self.agents_rect().x) as f32 / k,
        };
        let v = if matches!(s, Split::List | Split::ChangesLists) { v.clamp(0.05, 0.95) } else { v.max(40.0) };
        self.split_set(s, Some(v));
        self.relayout();
    }

    /// The drag ended.
    pub(super) fn split_release(&mut self) -> bool {
        self.split_drag.take().is_some()
    }

    /// Lays the window out again after a border moved.
    fn relayout(&mut self) {
        self.resize(self.w, self.h, self.s);
        self.damage.full = true;
    }

    /// The hovered or dragged border, as an accent line.
    pub(super) fn paint_split(&mut self, cv: &mut Canvas) {
        let Some(s) = self.split_drag.or(self.split_hot) else { return };
        let beside = self.layout == DetailsLayout::Beside;
        if let Some((_, r)) = self.splitters().into_iter().find(|(x, _)| *x == s) {
            let w = self.px(2.0).max(2);
            let line = if s.vertical(beside) {
                Rect::new(r.x - w / 2, r.y, w, r.h)
            } else {
                Rect::new(r.x, r.y - w / 2, r.w, w)
            };
            cv.fill(line, ACCENT());
        }
    }

    /// Hover over borders: the cursor and the line.
    pub(super) fn split_hover(&mut self, x: i32, y: i32) {
        let h = if self.split_drag.is_some() { self.split_drag } else { self.split_at(x, y) };
        if h != self.split_hot {
            self.split_hot = h;
            self.damage.full = true;
        }
        if let Some(s) = h {
            let beside = self.layout == DetailsLayout::Beside;
            self.cursor = if s.vertical(beside) { Cursor::ColResize } else { Cursor::RowResize };
        }
    }
}
