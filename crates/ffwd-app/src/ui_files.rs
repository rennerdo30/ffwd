//! The Files view (FF-51, FF-43): the repository's folder tree with change
//! marks and a filter, and the selected file's content (syntax colours),
//! blame (author and commit per line) or history (the commits that
//! changed it). Everything loads on workers; only visible rows paint.

use super::*;
use crate::highlight;
use ffwd_core::files::{Blame, Content, Tree};

/// A listed tree with each file's lowercase path (for the filter).
pub type TreeResult = Result<(Box<Tree>, Vec<(u32, Box<str>)>), String>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FTab {
    Content,
    Blame,
    History,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FFocus {
    Tree,
    Filter,
    Pane,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FHit {
    Row(usize),
    Filter,
    Tab(FTab),
    History(usize),
}

/// The loaded content with each line's block-comment state.
pub struct Loaded {
    pub content: Content,
    pub blocks: Vec<bool>,
}

pub struct Files {
    pub tree: Option<Box<Tree>>,
    /// Lowercase path of every file node, for the filter: (node, path).
    lower: Vec<(u32, Box<str>)>,
    pub open: HashSet<u32>,
    /// The visible rows: (node, depth).
    pub rows: Vec<(u32, u16)>,
    pub sel: usize,
    top: usize,
    pub filter: TextField,
    pub focus: FFocus,
    /// The open file: (node, path).
    pub file: Option<(u32, String)>,
    pub tab: FTab,
    pub content: Option<Loaded>,
    pub blame: Option<Blame>,
    pub history: Option<Vec<Oid>>,
    pub hist_sel: usize,
    ctop: usize,
    pub seq: u64,
    pub error: Option<String>,
    hits: Vec<(Rect, FHit)>,
}

impl Default for Files {
    fn default() -> Self {
        Files {
            tree: None,
            lower: Vec::new(),
            open: HashSet::new(),
            rows: Vec::new(),
            sel: 0,
            top: 0,
            filter: TextField::default(),
            focus: FFocus::Tree,
            file: None,
            tab: FTab::Content,
            content: None,
            blame: None,
            history: None,
            hist_sel: 0,
            ctop: 0,
            seq: 0,
            error: None,
            hits: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FLayout {
    pub area: Rect,
    pub filter: Rect,
    pub tree: Rect,
    pub tabs: Rect,
    pub pane: Rect,
}

impl Files {
    /// Rebuilds the visible rows: open folders, or with a filter every
    /// matching file with its folders (at most 5,000).
    fn rebuild(&mut self) {
        let Some(t) = &self.tree else {
            self.rows.clear();
            return;
        };
        self.rows.clear();
        let q = self.filter.text().trim().to_lowercase();
        if q.is_empty() {
            let mut stack: Vec<(u32, u16)> = t.nodes[0].children.iter().rev().map(|&c| (c, 0)).collect();
            while let Some((id, d)) = stack.pop() {
                self.rows.push((id, d));
                if t.nodes[id as usize].dir && self.open.contains(&id) {
                    stack.extend(t.nodes[id as usize].children.iter().rev().map(|&c| (c, d + 1)));
                }
            }
            return;
        }
        // Matching files, shown under their folders.
        let mut shown: HashSet<u32> = HashSet::new();
        let mut n = 0;
        for (id, p) in &self.lower {
            if p.contains(&q) {
                let mut a = *id;
                while a != 0 && shown.insert(a) {
                    a = t.nodes[a as usize].parent;
                }
                n += 1;
                if n >= 5000 {
                    break;
                }
            }
        }
        let mut stack: Vec<(u32, u16)> =
            t.nodes[0].children.iter().rev().filter(|c| shown.contains(c)).map(|&c| (c, 0)).collect();
        while let Some((id, d)) = stack.pop() {
            self.rows.push((id, d));
            if t.nodes[id as usize].dir {
                stack.extend(
                    t.nodes[id as usize].children.iter().rev().filter(|c| shown.contains(c)).map(|&c| (c, d + 1)),
                );
            }
        }
    }
}

impl Ui {
    pub(super) fn files_layout(&self) -> FLayout {
        let side = self.lay.side;
        let area =
            Rect::new(side.right(), self.lay.list.y, self.w as i32 - side.right(), self.lay.status.y - self.lay.list.y);
        let tw = ((area.w as f32 * 0.30) as i32).clamp(self.px(260.0), self.px(460.0)).min(area.w / 2);
        let fh = self.px(40.0);
        FLayout {
            area,
            filter: Rect::new(area.x, area.y, tw, fh),
            tree: Rect::new(area.x, area.y + fh, tw, area.h - fh),
            tabs: Rect::new(area.x + tw, area.y, area.w - tw, self.px(34.0)),
            pane: Rect::new(area.x + tw, area.y + self.px(34.0), area.w - tw, area.h - self.px(34.0)),
        }
    }

    fn files_dirty(&mut self) {
        let a = self.files_layout().area;
        self.damage.rects.push(a);
    }

    pub fn enter_files(&mut self) {
        if !self.r.files_view {
            self.r.files_view = true;
            self.r.changes_view = false;
            self.r.activity_view = false;
            self.r.reflog_view = false;
            self.focus = Focus::List;
        }
        self.side_select("files");
        self.load_files_tree();
        self.damage.full = true;
    }

    pub(super) fn leave_files(&mut self) {
        if self.r.files_view {
            self.r.files_view = false;
            self.damage.full = true;
        }
    }

    pub(super) fn load_files_tree(&mut self) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(t.id);
        crate::work::spawn("files", &sink, move |sink| {
            // Change marks from the working tree's status.
            let marks: std::collections::HashMap<String, u8> = ffwd_core::changes::load(&g)
                .map(|s| {
                    s.entries
                        .into_iter()
                        .map(|e| {
                            let m = if e.conflicted {
                                b'U'
                            } else if e.untracked() {
                                b'?'
                            } else if e.unstaged != b'.' {
                                e.unstaged
                            } else {
                                e.staged
                            };
                            (e.path.to_string(), m)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let r = ffwd_core::files::list_worktree(&g, &marks).map_err(|e| e.to_string()).map(|t| {
                let lower: Vec<(u32, Box<str>)> = (1..t.nodes.len() as u32)
                    .filter(|&i| !t.nodes[i as usize].dir)
                    .map(|i| (i, t.path(i).to_lowercase().into()))
                    .collect();
                (Box::new(t), lower)
            });
            sink(Msg::FilesTree(r));
        });
    }

    pub(super) fn on_files_tree(&mut self, r: TreeResult) {
        let f = &mut self.r.files;
        match r {
            Ok((t, lower)) => {
                // Keep the open folders and the open file by path.
                let open_paths: Vec<String> =
                    f.tree.as_ref().map_or(Vec::new(), |old| f.open.iter().map(|&i| old.path(i)).collect());
                let file_path = f.file.as_ref().map(|x| x.1.clone());
                f.open = open_paths.iter().filter_map(|p| t.find(p)).collect();
                if f.open.is_empty() && f.tree.is_none() {
                    // A first look: open the top folder when there is only one.
                    let dirs: Vec<u32> =
                        t.nodes[0].children.iter().copied().filter(|&c| t.nodes[c as usize].dir).collect();
                    if dirs.len() == 1 {
                        f.open.insert(dirs[0]);
                    }
                }
                f.file = file_path.and_then(|p| t.find(&p).map(|i| (i, p)));
                f.tree = Some(t);
                f.lower = lower;
                f.error = None;
                f.rebuild();
                f.sel = f.sel.min(f.rows.len().saturating_sub(1));
            }
            Err(e) => f.error = Some(e),
        }
        self.files_dirty();
    }

    fn open_file(&mut self, node: u32) {
        let Some(t) = &self.r.files.tree else { return };
        let path = t.path(node);
        let f = &mut self.r.files;
        f.file = Some((node, path.clone()));
        f.content = None;
        f.blame = None;
        f.history = None;
        f.ctop = 0;
        f.hist_sel = 0;
        f.seq += 1;
        let seq = f.seq;
        let (Some(g), Some(tab)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(tab.id);
        let p = path.clone();
        crate::work::spawn("file", &sink, move |sink| {
            let r = ffwd_core::files::read(&g, None, &p).map(|content| {
                let lang = highlight::lang_for(&p);
                let mut st = false;
                let blocks = content
                    .lines
                    .iter()
                    .map(|l| {
                        let s = st;
                        if let Some(lang) = lang {
                            st = highlight::ends_in_block(lang, l, st);
                        }
                        s
                    })
                    .collect();
                Box::new(Loaded { content, blocks })
            });
            sink(Msg::FileContent { seq, result: r });
        });
        // Blame and history load once their tab is shown.
        self.load_file_tab();
        self.files_dirty();
    }

    /// Loads what the current tab needs (blame, history) once.
    fn load_file_tab(&mut self) {
        let f = &self.r.files;
        let Some((_, path)) = f.file.clone() else { return };
        let (seq, tab) = (f.seq, f.tab);
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let sink = (self.sinks)(t.id);
        match tab {
            FTab::Blame if f.blame.is_none() => crate::work::spawn("blame", &sink, move |sink| {
                let r = ffwd_core::files::blame(&g, None, &path).map(Box::new).map_err(|e| e.to_string());
                sink(Msg::FileBlame { seq, result: r });
            }),
            FTab::History if f.history.is_none() => crate::work::spawn("file-history", &sink, move |sink| {
                let r = ffwd_core::files::history(&g, &path).map_err(|e| e.to_string());
                sink(Msg::FileHistory { seq, result: r });
            }),
            _ => {}
        }
    }

    pub(super) fn on_file_content(&mut self, seq: u64, r: Result<Box<Loaded>, String>) {
        let f = &mut self.r.files;
        if seq != f.seq {
            return;
        }
        match r {
            Ok(l) => f.content = Some(*l),
            Err(e) => f.error = Some(e),
        }
        let p = self.files_layout().pane;
        self.damage.rects.push(p);
    }

    pub(super) fn on_file_blame(&mut self, seq: u64, r: Result<Box<Blame>, String>) {
        let f = &mut self.r.files;
        if seq != f.seq {
            return;
        }
        match r {
            Ok(b) => f.blame = Some(*b),
            Err(e) => f.error = Some(e),
        }
        let p = self.files_layout().pane;
        self.damage.rects.push(p);
    }

    pub(super) fn on_file_history(&mut self, seq: u64, r: Result<Vec<Oid>, String>) {
        let f = &mut self.r.files;
        if seq != f.seq {
            return;
        }
        match r {
            Ok(h) => f.history = Some(h),
            Err(e) => f.error = Some(e),
        }
        let p = self.files_layout().pane;
        self.damage.rects.push(p);
    }

    fn set_file_tab(&mut self, t: FTab) {
        self.r.files.tab = t;
        self.r.files.ctop = 0;
        self.load_file_tab();
        self.files_dirty();
    }

    /// Selects tree row `i`, keeping it on screen.
    fn select_tree(&mut self, i: usize) {
        let fit = (self.files_layout().tree.h / self.px(26.0).max(1)).max(1) as usize;
        let f = &mut self.r.files;
        if f.rows.is_empty() {
            return;
        }
        f.sel = i.min(f.rows.len() - 1);
        if f.sel < f.top {
            f.top = f.sel;
        } else if f.sel >= f.top + fit {
            f.top = f.sel + 1 - fit;
        }
        let r = self.files_layout().tree;
        self.damage.rects.push(r);
    }

    /// Enter or a click on a row: a folder folds, a file opens.
    fn activate_row(&mut self, i: usize) {
        let Some(&(id, _)) = self.r.files.rows.get(i) else { return };
        let dir = self.r.files.tree.as_ref().is_some_and(|t| t.nodes[id as usize].dir);
        if dir {
            let f = &mut self.r.files;
            if !f.open.remove(&id) {
                f.open.insert(id);
            }
            f.rebuild();
            self.select_tree(i);
            let r = self.files_layout().tree;
            self.damage.rects.push(r);
        } else {
            self.open_file(id);
        }
    }

    fn jump_to_history_commit(&mut self) {
        let Some(id) = self.r.files.history.as_ref().and_then(|h| h.get(self.r.files.hist_sel)).copied() else {
            return;
        };
        match self.r.by_id.get(&id) {
            Some(&row) => {
                self.leave_files();
                self.side_select("all");
                self.select(row as usize);
            }
            None => self.set_notice("That commit is not loaded in the history yet.".into()),
        }
    }

    pub(super) fn files_key(&mut self, k: Key) -> bool {
        let fit_tree = (self.files_layout().tree.h / self.px(26.0).max(1)).max(1) as usize;
        let page = (self.files_layout().pane.h / self.code_lh().max(1)).max(1) as usize;
        let f = &mut self.r.files;
        match f.focus {
            FFocus::Filter => {
                let t = &mut f.filter;
                match k {
                    Key::Esc => {
                        if t.text().is_empty() {
                            f.focus = FFocus::Tree;
                        } else {
                            *t = TextField::default();
                        }
                    }
                    Key::Enter | Key::Down | Key::Tab => f.focus = FFocus::Tree,
                    Key::Char(c) => t.insert(c.encode_utf8(&mut [0; 4])),
                    Key::Backspace => t.backspace(),
                    Key::CtrlBackspace => t.delete_word(),
                    Key::Delete => t.delete(),
                    Key::Left => t.left(),
                    Key::Right => t.right(),
                    Key::Home => t.home(),
                    Key::End => t.end(),
                    Key::Ctrl('a') => t.select_all(),
                    Key::Ctrl('v') => {
                        if let Some(p) = crate::platform::clipboard() {
                            t.insert(p.lines().next().unwrap_or(""));
                        }
                    }
                    _ => return false,
                }
                f.rebuild();
                f.sel = 0;
                f.top = 0;
            }
            FFocus::Tree => {
                let sel = f.sel;
                let row = f.rows.get(sel).copied();
                let tree = f.tree.as_ref();
                let is_dir = |id: u32| tree.is_some_and(|t| t.nodes[id as usize].dir);
                match k {
                    Key::Up => self.select_tree(sel.saturating_sub(1)),
                    Key::Down => self.select_tree(sel + 1),
                    Key::PageUp => self.select_tree(sel.saturating_sub(fit_tree)),
                    Key::PageDown => self.select_tree(sel + fit_tree),
                    Key::Home => self.select_tree(0),
                    Key::End => self.select_tree(usize::MAX),
                    Key::Enter | Key::Char(' ') => self.activate_row(sel),
                    Key::Right => match row {
                        Some((id, _)) if is_dir(id) && !f.open.contains(&id) => self.activate_row(sel),
                        Some((id, _)) if is_dir(id) => self.select_tree(sel + 1),
                        Some(_) => f.focus = FFocus::Pane,
                        None => {}
                    },
                    Key::Left => match row {
                        Some((id, _)) if is_dir(id) && f.open.contains(&id) => self.activate_row(sel),
                        Some((id, _)) => {
                            // To the parent folder.
                            let parent = tree.map_or(0, |t| t.nodes[id as usize].parent);
                            if let Some(p) = f.rows.iter().position(|r| r.0 == parent) {
                                self.select_tree(p);
                            }
                        }
                        None => {}
                    },
                    Key::Tab => f.focus = FFocus::Pane,
                    Key::Ctrl('f') | Key::Char('/') => f.focus = FFocus::Filter,
                    Key::Char(c) if c.is_alphanumeric() || c == '.' || c == '_' => {
                        // Typing filters.
                        f.focus = FFocus::Filter;
                        f.filter.insert(c.encode_utf8(&mut [0; 4]));
                        f.rebuild();
                        f.sel = 0;
                        f.top = 0;
                    }
                    _ => return false,
                }
            }
            FFocus::Pane => match k {
                Key::Tab | Key::Esc | Key::Left => f.focus = FFocus::Tree,
                Key::Char('c') | Key::Char('1') => self.set_file_tab(FTab::Content),
                Key::Char('b') | Key::Char('2') => self.set_file_tab(FTab::Blame),
                Key::Char('h') | Key::Char('3') => self.set_file_tab(FTab::History),
                Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End if f.tab == FTab::History => {
                    let n = f.history.as_ref().map_or(0, |h| h.len());
                    let d: i64 = match k {
                        Key::Up => -1,
                        Key::Down => 1,
                        Key::PageUp => -(page as i64),
                        Key::PageDown => page as i64,
                        Key::Home => -(n as i64),
                        _ => n as i64,
                    };
                    f.hist_sel = (f.hist_sel as i64 + d).clamp(0, n.saturating_sub(1) as i64) as usize;
                    if f.hist_sel < f.ctop {
                        f.ctop = f.hist_sel;
                    } else if f.hist_sel >= f.ctop + page.saturating_sub(1) {
                        f.ctop = f.hist_sel + 2 - page.min(f.hist_sel + 2);
                    }
                }
                Key::Enter if f.tab == FTab::History => self.jump_to_history_commit(),
                Key::Up | Key::Char('k') => f.ctop = f.ctop.saturating_sub(1),
                Key::Down | Key::Char('j') => f.ctop += 1,
                Key::PageUp => f.ctop = f.ctop.saturating_sub(page),
                Key::PageDown => f.ctop += page,
                Key::Home => f.ctop = 0,
                _ => return false,
            },
        }
        self.files_dirty();
        true
    }

    pub(super) fn files_click(&mut self, x: i32, y: i32) -> bool {
        let l = self.files_layout();
        if !l.area.contains(x, y) {
            return false;
        }
        let hit = self.r.files.hits.iter().find(|h| h.0.contains(x, y)).map(|h| h.1);
        match hit {
            Some(FHit::Row(i)) => {
                self.r.files.focus = FFocus::Tree;
                self.select_tree(i);
                self.activate_row(i);
            }
            Some(FHit::Filter) => self.r.files.focus = FFocus::Filter,
            Some(FHit::Tab(t)) => {
                self.r.files.focus = FFocus::Pane;
                self.set_file_tab(t);
            }
            Some(FHit::History(i)) => {
                let again = self.double && self.r.files.hist_sel == i;
                self.r.files.focus = FFocus::Pane;
                self.r.files.hist_sel = i;
                if again {
                    self.jump_to_history_commit();
                }
            }
            None if l.pane.contains(x, y) => self.r.files.focus = FFocus::Pane,
            None => {}
        }
        self.files_dirty();
        true
    }

    pub(super) fn files_wheel(&mut self, x: i32, y: i32, n: i64) {
        let l = self.files_layout();
        let f = &mut self.r.files;
        if l.tree.contains(x, y) {
            f.top = (f.top as i64 + n).clamp(0, f.rows.len().saturating_sub(1) as i64) as usize;
        } else if l.pane.contains(x, y) {
            f.ctop = (f.ctop as i64 + n).max(0) as usize;
        }
        self.files_dirty();
    }

    /// The repository-relative path of the file row at (x, y).
    pub(super) fn files_path_at(&self, x: i32, y: i32) -> Option<String> {
        let i = self.files_hit_at(x, y)?;
        let Some((_, FHit::Row(r))) = self.r.files.hits.get(i) else { return None };
        let (id, _) = *self.r.files.rows.get(*r)?;
        let t = self.r.files.tree.as_ref()?;
        (!t.nodes[id as usize].dir).then(|| t.path(id))
    }

    pub(super) fn files_hit_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.r.files_view {
            return None;
        }
        self.r.files.hits.iter().position(|h| h.0.contains(x, y))
    }

    /// Opens the folders above `path`, selects it and opens it. False
    /// when the path is not in the tree.
    pub fn reveal_file(&mut self, path: &str) -> bool {
        let f = &mut self.r.files;
        let Some(t) = &f.tree else { return false };
        let Some(id) = t.find(path).filter(|&i| !t.nodes[i as usize].dir) else { return false };
        let mut a = t.nodes[id as usize].parent;
        while a != 0 {
            f.open.insert(a);
            a = t.nodes[a as usize].parent;
        }
        f.filter = TextField::default();
        f.rebuild();
        let Some(i) = f.rows.iter().position(|r| r.0 == id) else { return false };
        f.focus = FFocus::Tree;
        self.select_tree(i);
        self.open_file(id);
        true
    }

    /// Snapshot mode: shows tab `name` (content, blame, history).
    pub fn snapshot_files_tab(&mut self, name: &str) {
        let t = match name {
            "blame" => FTab::Blame,
            "history" => FTab::History,
            _ => FTab::Content,
        };
        self.set_file_tab(t);
    }

    /// Snapshot mode: whether what the current tab shows has arrived.
    pub fn files_ready(&self) -> bool {
        let f = &self.r.files;
        f.error.is_some()
            || f.content.is_some()
                && match f.tab {
                    FTab::Content => true,
                    FTab::Blame => f.blame.is_some(),
                    FTab::History => f.history.is_some(),
                }
            || f.tab == FTab::History && f.history.is_some()
    }

    pub(super) fn files_hit_rect(&self, i: usize) -> Rect {
        self.r.files.hits.get(i).map_or(self.files_layout().area, |h| h.0.inset(-2, -2))
    }

    pub(super) fn files_hit_is_text(&self, i: usize) -> bool {
        matches!(self.r.files.hits.get(i), Some((_, FHit::Filter)))
    }

    pub(super) fn paint_files(&mut self, cv: &mut Canvas) {
        let l = self.files_layout();
        let mut hits: Vec<(Rect, FHit)> = Vec::new();
        let (ui, semi) = (self.f.ui, self.f.semi);
        // Filter field.
        cv.fill(Rect::new(l.filter.x, l.filter.y, l.filter.w, l.filter.h + l.tree.h), BG_SIDEBAR);
        cv.fill(Rect::new(l.tree.right() - 1, l.area.y, 1, l.area.h), BORDER);
        let ff = l.filter.inset(self.px(10.0), self.px(6.0));
        let typing = self.r.files.focus == FFocus::Filter;
        let border = if typing {
            ACCENT
        } else if self.is_hot(Hot::Files(hits.len())) {
            BORDER_HOVER
        } else {
            BORDER
        };
        cv.round_border(ff, self.pf(7.0), border, BG_INPUT);
        let fb = (ff.y as f32 + ff.h as f32 / 2.0 + self.pf(4.5)).round();
        let ft = self.r.files.filter.clone();
        let tx = ff.x as f32 + self.pf(10.0);
        if ft.text().is_empty() {
            self.te.draw(cv, ui, self.pf(12.5), tx, fb, "Filter files (type to search)", TEXT_4, ff.w as f32);
        } else {
            self.te.draw(cv, ui, self.pf(12.5), tx, fb, ft.text(), TEXT, ff.w as f32 - self.pf(20.0));
        }
        if typing {
            let cx = tx + self.te.measure(ui, self.pf(12.5), &ft.text()[..ft.cursor()]);
            cv.fill(Rect::new(cx as i32 + 1, ff.y + self.px(6.0), self.px(1.5).max(1), ff.h - self.px(12.0)), ACCENT);
        }
        hits.push((ff, FHit::Filter));
        // Tree rows.
        let rh = self.px(26.0);
        let fit = (l.tree.h / rh.max(1)) as usize + 1;
        let f = &self.r.files;
        let top = f.top.min(f.rows.len().saturating_sub(1));
        let rows: Vec<(u32, u16)> = f.rows.iter().skip(top).take(fit).copied().collect();
        let sel = f.sel;
        let tree_focus = f.focus == FFocus::Tree;
        let open_file = f.file.as_ref().map(|x| x.0);
        let old = cv.set_clip(cv.clip().intersect(l.tree));
        if f.tree.is_none() {
            let t = f.error.clone().unwrap_or_else(|| "Reading the files…".into());
            self.te.draw(
                cv,
                ui,
                self.pf(12.0),
                l.tree.x as f32 + self.pf(16.0),
                l.tree.y as f32 + self.pf(22.0),
                &t,
                TEXT_4,
                l.tree.w as f32,
            );
        }
        for (k, (id, depth)) in rows.into_iter().enumerate() {
            let i = top + k;
            let y = l.tree.y + k as i32 * rh;
            let row = Rect::new(l.tree.x + self.px(6.0), y, l.tree.w - self.px(12.0), rh);
            let Some(t) = &self.r.files.tree else { break };
            let n = &t.nodes[id as usize];
            let (name, dir, mark) = (n.name.to_string(), n.dir, n.mark);
            let open = self.r.files.open.contains(&id) || !self.r.files.filter.text().is_empty();
            if i == sel {
                cv.fill_round(row, self.pf(6.0), if tree_focus { BG_SELECT } else { BG_SELECT_DIM });
            } else if Some(id) == open_file {
                cv.fill_round(row, self.pf(6.0), BG_SELECT_DIM);
            } else if self.is_hot(Hot::Files(hits.len())) {
                cv.fill_round(row, self.pf(6.0), BG_HOVER);
            }
            let cy = y as f32 + rh as f32 / 2.0;
            let b = (cy + self.pf(4.5)).round();
            let ix = row.x as f32 + self.pf(10.0) + depth as f32 * self.pf(14.0);
            if dir {
                // Chevron and folder.
                let s = self.pf(3.0);
                let w = self.pf(1.4);
                if open {
                    cv.stroke(&[(ix - s, cy - s / 2.0), (ix, cy + s / 2.0), (ix + s, cy - s / 2.0)], w, TEXT_3);
                } else {
                    cv.stroke(&[(ix - s / 2.0, cy - s), (ix + s / 2.0, cy), (ix - s / 2.0, cy + s)], w, TEXT_3);
                }
            }
            let nx = ix + self.pf(12.0);
            let color = match mark {
                b'?' | b'A' => ADD,
                b'D' => DEL,
                b'U' => DEL,
                0 => {
                    if dir {
                        TEXT_2
                    } else {
                        TEXT
                    }
                }
                _ => MOD,
            };
            let font = if dir { semi } else { ui };
            self.te.draw(cv, font, self.pf(12.5), nx, b, &name, color, row.right() as f32 - nx - self.pf(24.0));
            if mark != 0 && !dir {
                let m = if mark == b'?' { "new".to_string() } else { (mark as char).to_string() };
                let mw = self.te.measure(semi, self.pf(10.5), &m);
                self.te.draw(cv, semi, self.pf(10.5), row.right() as f32 - mw - self.pf(8.0), b, &m, color, mw + 2.0);
            }
            hits.push((row, FHit::Row(i)));
        }
        cv.set_clip(old);
        // Tabs.
        cv.fill(l.tabs, BG_BAR);
        cv.fill(Rect::new(l.tabs.x, l.tabs.bottom() - 1, l.tabs.w, 1), BORDER_SOFT);
        let tb = (l.tabs.y as f32 + l.tabs.h as f32 / 2.0 + self.pf(4.5)).round();
        let path = self.r.files.file.as_ref().map(|x| x.1.clone());
        let title = path.clone().unwrap_or_else(|| "Select a file".into());
        let e = self.te.draw(
            cv,
            semi,
            self.pf(12.5),
            l.tabs.x as f32 + self.pf(16.0),
            tb,
            &title,
            if path.is_some() { TEXT } else { TEXT_4 },
            l.tabs.w as f32 * 0.5,
        );
        let mut x = e + self.pf(18.0);
        if path.is_some() {
            for (t, label) in
                [(FTab::Content, "Content (c)"), (FTab::Blame, "Blame (b)"), (FTab::History, "History (h)")]
            {
                let w = self.te.measure(semi, self.pf(11.5), label) + self.pf(18.0);
                let r = Rect::new(x as i32, l.tabs.y + self.px(5.0), w as i32, l.tabs.h - self.px(10.0));
                if self.r.files.tab == t {
                    cv.fill_round(r, self.pf(6.0), BG_CHOSEN);
                } else if self.is_hot(Hot::Files(hits.len())) {
                    cv.fill_round(r, self.pf(6.0), CONTROL_HOVER);
                }
                let c = if self.r.files.tab == t { TEXT } else { TEXT_3 };
                self.te.draw(cv, semi, self.pf(11.5), r.x as f32 + self.pf(9.0), tb, label, c, w);
                hits.push((r, FHit::Tab(t)));
                x += w + self.pf(4.0);
            }
        }
        // Pane.
        cv.fill(l.pane, BG);
        if path.is_some() {
            match self.r.files.tab {
                FTab::History => self.paint_file_history(cv, l.pane, &mut hits),
                tab => self.paint_file_lines(cv, l.pane, tab == FTab::Blame),
            }
        } else {
            let t = "Pick a file on the left: ↑↓ move, → opens folders, Enter opens a file, typing filters";
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                l.pane.x as f32 + self.pf(16.0),
                l.pane.y as f32 + self.pf(26.0),
                t,
                TEXT_4,
                l.pane.w as f32,
            );
        }
        self.r.files.hits = hits;
    }

    fn paint_file_lines(&mut self, cv: &mut Canvas, r: Rect, blame: bool) {
        let (ui, mono) = (self.f.ui, self.f.mono);
        let Some(loaded) = self.r.files.content.take() else {
            let t = self.r.files.error.clone().unwrap_or_else(|| "Loading…".into());
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                r.x as f32 + self.pf(16.0),
                r.y as f32 + self.pf(26.0),
                &t,
                TEXT_4,
                r.w as f32,
            );
            return;
        };
        if loaded.content.binary {
            let t = format!("Binary file · {} bytes", fmt::num(loaded.content.bytes as usize));
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                r.x as f32 + self.pf(16.0),
                r.y as f32 + self.pf(26.0),
                &t,
                TEXT_3,
                r.w as f32,
            );
            self.r.files.content = Some(loaded);
            return;
        }
        let blame_data = if blame { self.r.files.blame.take() } else { None };
        let lh = self.code_lh();
        let fit = (r.h / lh) as usize + 1;
        let n = loaded.content.lines.len();
        let top = self.r.files.ctop.min(n.saturating_sub(1));
        self.r.files.ctop = top;
        let path = self.r.files.file.as_ref().map(|x| x.1.clone()).unwrap_or_default();
        let lang = highlight::lang_for(&path);
        let num_w = self.code_px(52.0);
        let blame_w = if blame { self.pf(300.0).min(r.w as f32 * 0.4) } else { 0.0 };
        let gut = Rect::new(r.x, r.y, (num_w + blame_w) as i32, r.h);
        cv.fill(gut, BG_SIDEBAR);
        let clip = cv.clip();
        for (k, line) in loaded.content.lines.iter().enumerate().skip(top).take(fit) {
            let y = r.y + (k - top) as i32 * lh;
            let row = Rect::new(r.x, y, r.w, lh);
            if clip.intersect(row).is_empty() {
                continue;
            }
            let lb = (y as f32 + lh as f32 / 2.0 + self.code_px(4.0)).round();
            if let Some(b) = &blame_data
                && let Some(&ci) = b.lines.get(k)
            {
                let c = &b.commits[ci as usize];
                // A band per commit, a rule where a run starts, the label
                // on a run's first visible line and the subject on its second.
                let starts = |j: usize| j == 0 || b.lines.get(j - 1) != Some(&b.lines[j]);
                let first = k == top || starts(k);
                let second = !first && (k - 1 == top || starts(k - 1));
                let band = if ci % 2 == 0 { BG_SIDEBAR } else { BG_PANEL };
                cv.fill(Rect::new(r.x, y, blame_w as i32, lh), band);
                if k > 0 && starts(k) {
                    cv.fill(Rect::new(r.x, y, blame_w as i32, 1), BORDER_SOFT);
                }
                let (font, text, color) = if first {
                    let when = fmt::relative(c.time, now_secs(), self.r.tz);
                    let sha = c.id.short(7);
                    if sha.bytes().all(|b| b == b'0') {
                        (ui, "Not committed yet".to_string(), MOD)
                    } else {
                        (mono, format!("{sha} · {} · {when}", c.author), TEXT_3)
                    }
                } else if second {
                    (ui, c.summary.to_string(), TEXT_4)
                } else {
                    (ui, String::new(), TEXT_4)
                };
                if !text.is_empty() {
                    let x = r.x as f32 + self.pf(10.0);
                    self.te.draw(cv, font, self.pf(11.0), x, lb, &text, color, blame_w - self.pf(16.0));
                }
            }
            let s = (k + 1).to_string();
            let w = self.te.measure(mono, self.code_px(11.0), &s);
            self.te.draw(
                cv,
                mono,
                self.pf(11.0),
                r.x as f32 + blame_w + num_w - w - self.pf(8.0),
                lb,
                &s,
                TEXT_4,
                num_w,
            );
            let text = line.replace('\t', "    ");
            let st = loaded.blocks.get(k).copied().unwrap_or(false);
            let tx = r.x as f32 + blame_w + num_w + self.pf(10.0);
            self.draw_code(cv, tx, lb, row, &text, TEXT_2, r.right() as f32 - tx - self.pf(12.0), lang, st, &[]);
        }
        if blame && blame_data.is_none() {
            self.te.draw(
                cv,
                ui,
                self.pf(11.5),
                r.x as f32 + self.pf(10.0),
                r.y as f32 + self.pf(16.0),
                "Blame is loading…",
                TEXT_4,
                blame_w,
            );
        }
        if loaded.content.truncated {
            let t = "Only the first 4 MB are shown";
            let w = self.te.measure(ui, self.pf(11.0), t);
            self.te.draw(
                cv,
                ui,
                self.pf(11.0),
                r.right() as f32 - w - self.pf(16.0),
                r.bottom() as f32 - self.pf(10.0),
                t,
                MOD,
                w + 2.0,
            );
        }
        self.r.files.blame = blame_data;
        self.r.files.content = Some(loaded);
    }

    fn paint_file_history(&mut self, cv: &mut Canvas, r: Rect, hits: &mut Vec<(Rect, FHit)>) {
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let Some(h) = self.r.files.history.clone() else {
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                r.x as f32 + self.pf(16.0),
                r.y as f32 + self.pf(26.0),
                "Reading the history…",
                TEXT_4,
                r.w as f32,
            );
            return;
        };
        if h.is_empty() {
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                r.x as f32 + self.pf(16.0),
                r.y as f32 + self.pf(26.0),
                "No commits touch this file yet (untracked?)",
                TEXT_4,
                r.w as f32,
            );
            return;
        }
        let rh = self.px(30.0);
        let fit = (r.h / rh) as usize + 1;
        let top = self.r.files.ctop.min(h.len().saturating_sub(1));
        let sel = self.r.files.hist_sel;
        let focus = self.r.files.focus == FFocus::Pane;
        for (k, id) in h.iter().enumerate().skip(top).take(fit) {
            let y = r.y + (k - top) as i32 * rh;
            let row = Rect::new(r.x + self.px(8.0), y + self.px(2.0), r.w - self.px(16.0), rh - self.px(4.0));
            if k == sel {
                cv.fill_round(row, self.pf(6.0), if focus { BG_SELECT } else { BG_SELECT_DIM });
            } else if self.is_hot(Hot::Files(hits.len())) {
                cv.fill_round(row, self.pf(6.0), BG_HOVER);
            }
            let b = (row.y as f32 + row.h as f32 / 2.0 + self.pf(4.5)).round();
            let c = self.r.by_id.get(id).and_then(|&i| self.r.commits.get(i as usize)).cloned();
            let x0 = row.x as f32 + self.pf(10.0);
            self.te.draw(cv, mono, self.pf(11.5), x0, b, &id.short(8), TEXT_3, self.pf(70.0));
            match c {
                Some(c) => {
                    let when = fmt::relative(c.time, now_secs(), self.r.tz);
                    let right = format!("{} · {when}", c.author);
                    let rw = self.te.measure(ui, self.pf(11.5), &right);
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(11.5),
                        row.right() as f32 - rw - self.pf(10.0),
                        b,
                        &right,
                        TEXT_3,
                        rw + 2.0,
                    );
                    self.te.draw(
                        cv,
                        semi,
                        self.pf(12.5),
                        x0 + self.pf(76.0),
                        b,
                        &c.subject,
                        TEXT,
                        row.right() as f32 - rw - x0 - self.pf(100.0),
                    );
                }
                None => {
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(12.0),
                        x0 + self.pf(76.0),
                        b,
                        "not in the loaded history",
                        TEXT_4,
                        row.w as f32,
                    );
                }
            }
            hits.push((row, FHit::History(k)));
        }
        let hint = "Enter or double click: show in history";
        let hw = self.te.measure(ui, self.pf(11.0), hint);
        self.te.draw(
            cv,
            ui,
            self.pf(11.0),
            r.right() as f32 - hw - self.pf(16.0),
            r.bottom() as f32 - self.pf(10.0),
            hint,
            TEXT_4,
            hw + 2.0,
        );
    }
}
