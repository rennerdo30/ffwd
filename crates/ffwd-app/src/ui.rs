//! The ffwd main view: state, layout, input and painting.
//!
//! Painting is clip-driven: every damage rectangle is painted by running
//! the scene with the canvas clipped to it, and each region skips work
//! outside the clip. Damage is recorded where state changes, so a key
//! press repaints a few rows, never the window.

use crate::fmt;
use crate::theme::*;
use crate::work::{DetailWorker, Msg, Sink};
use ffwd_core::diff::{Details, FileDiff, LineKind};
use ffwd_core::refs::{RefKind, Refs, StatusSummary};
use ffwd_core::{Commit, Git, Graph, Oid};
use ffwd_render::{Canvas, FontId, Rect, TextEngine};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Fonts {
    pub ui: FontId,
    pub semi: FontId,
    pub mono: FontId,
}

pub fn load_fonts(te: &mut TextEngine) -> Option<Fonts> {
    type P = &'static [&'static str];
    #[cfg(windows)]
    let (ui, semi, mono): (P, P, P) = (
        &[r"C:\Windows\Fonts\Inter-Regular.ttf", r"C:\Windows\Fonts\segoeui.ttf"],
        &[r"C:\Windows\Fonts\Inter-SemiBold.ttf", r"C:\Windows\Fonts\seguisb.ttf", r"C:\Windows\Fonts\segoeuib.ttf"],
        &[r"C:\Windows\Fonts\CascadiaMono.ttf", r"C:\Windows\Fonts\consola.ttf"],
    );
    #[cfg(target_os = "macos")]
    let (ui, semi, mono): (P, P, P) = (
        &["/Library/Fonts/Inter-Regular.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf"],
        &["/Library/Fonts/Inter-SemiBold.ttf", "/System/Library/Fonts/Supplemental/Arial Bold.ttf"],
        &["/Library/Fonts/CascadiaMono.ttf", "/System/Library/Fonts/Supplemental/Courier New.ttf"],
    );
    #[cfg(not(any(windows, target_os = "macos")))]
    let (ui, semi, mono): (P, P, P) = (
        &["/usr/share/fonts/truetype/inter/Inter-Regular.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"],
        &["/usr/share/fonts/truetype/inter/Inter-SemiBold.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"],
        &["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"],
    );
    // fontdue parses every outline up front (~10 ms per face), so the three
    // faces load in parallel.
    let read = |paths: &'static [&'static str]| {
        std::thread::spawn(move || paths.iter().find_map(|p| std::fs::read(p).ok()).and_then(TextEngine::parse))
    };
    let (a, b, c) = (read(ui), read(semi), read(mono));
    let ui = te.add(a.join().ok().flatten()?);
    let semi = b.join().ok().flatten().map_or(ui, |f| te.add(f));
    let mono = c.join().ok().flatten().map_or(ui, |f| te.add(f));
    Some(Fonts { ui, semi, mono })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    List,
    Files,
    Diff,
}

#[derive(Clone, Copy, Debug)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Tab,
    BackTab,
    Enter,
    Esc,
    Backspace,
    Palette,
    Char(char),
}

#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub top: Rect,
    pub status: Rect,
    pub side: Rect,
    pub list: Rect,
    pub list_head: Rect,
    pub list_body: Rect,
    pub scrollbar: Rect,
    pub details: Rect,
    pub row_h: i32,
    /// Minimise, maximise and close, drawn by ffwd (Windows 11 frame).
    pub caption: Rect,
    /// Commit header and file list (the whole details pane when beside).
    pub info: Rect,
}

/// Where the commit details go (setting `ui.layout`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DetailsLayout {
    /// Under the history, with the diff on the right.
    Below,
    /// To the right of the history, diff under the file list.
    Beside,
}

impl DetailsLayout {
    pub fn from_setting(s: Option<&str>) -> DetailsLayout {
        match s {
            Some("beside") => DetailsLayout::Beside,
            _ => DetailsLayout::Below,
        }
    }
    pub fn as_setting(self) -> &'static str {
        match self {
            DetailsLayout::Below => "below",
            DetailsLayout::Beside => "beside",
        }
    }
}

/// How the window's title row is made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chrome {
    /// The system draws the title bar (Linux: server-side decorations).
    Native,
    /// ffwd's top bar is the title row; ffwd draws the caption buttons.
    Windows11,
    /// The top bar runs under macOS's traffic lights, which keep their place.
    MacInset,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WinAction {
    Minimize,
    Close,
}

/// Title-row geometry for the platform frame, in physical pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitleGeometry {
    pub height: i32,
    pub controls: Vec<(i32, i32)>,
    pub buttons_left: i32,
    pub max_button: (i32, i32),
}

#[derive(Default)]
pub struct Damage {
    pub full: bool,
    pub rects: Vec<Rect>,
    /// Absolute commit rows to repaint (resolved after any scroll).
    pub rows: Vec<usize>,
    /// Rows the list content moved up (negative: down) since the last paint.
    pub scroll: i64,
}

impl Damage {
    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty() && self.rows.is_empty() && self.scroll == 0
    }
}

/// The result of one paint: what to present, and an optional pixel copy
/// that the window should perform on screen first (scroll by copy).
pub struct Painted {
    pub rects: Vec<Rect>,
    pub copy: Option<(Rect, i32)>,
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub first_frame_ms: Option<f64>,
    pub presents: u64,
    pub last_paint_ms: f64,
    pub last_damage_px: u64,
}

struct SideItem {
    indent: u8,
    kind: SideKind,
    label: String,
    badge: String,
    active: bool,
    target: Option<Oid>,
}

#[derive(Clone, Copy, PartialEq)]
enum SideKind {
    Header,
    Changes,
    All,
    Branch,
    Remote,
    RemoteBranch,
    Tag,
    Stash,
    Worktree,
    More,
}

struct PItem {
    label: String,
    hint: String,
    kind: RefKind,
    action: PAction,
}

#[derive(Clone)]
enum PAction {
    Goto(Oid),
    Command(&'static str),
}

pub struct Palette {
    pub query: String,
    items: Vec<PItem>,
    sel: usize,
}

const COMMANDS: &[(&str, &str)] = &[
    ("Fetch all remotes", "Ctrl Shift F"),
    ("Pull", "Ctrl Shift L"),
    ("Push", "Ctrl Shift P"),
    ("Commit staged changes", "Ctrl Enter"),
    ("Create branch here", "Ctrl B"),
    ("Create tag here", "Ctrl Shift G"),
    ("Cherry-pick this commit", ""),
    ("Revert this commit", ""),
    ("Interactive rebase from here", "Ctrl Shift R"),
    ("Stash changes", "Ctrl Shift S"),
    ("Open file history", ""),
    ("Blame file", ""),
    ("AI: Explain this commit", "Ctrl I"),
    ("AI: Draft commit message", "Ctrl Shift I"),
    ("AI: Review branch against main", ""),
    ("Settings: AI endpoint", ""),
    (LAYOUT_BELOW, ""),
    (LAYOUT_BESIDE, ""),
];

const LAYOUT_BELOW: &str = "Layout: details below the history";
const LAYOUT_BESIDE: &str = "Layout: details beside the history";

pub struct Ui {
    pub s: f32,
    pub w: usize,
    pub h: usize,
    pub lay: Layout,
    pub te: TextEngine,
    pub f: Fonts,
    pub repo_name: String,
    pub git: Option<Git>,
    pub error: Option<String>,
    pub tz: i64,
    now: i64,
    pub commits: Vec<Commit>,
    pub graph: Graph,
    by_id: HashMap<Oid, u32>,
    pub refs: Refs,
    chips: HashMap<Oid, Vec<u32>>,
    side: Vec<SideItem>,
    pub status: Option<StatusSummary>,
    pub log_done: bool,
    pub log_ms: f64,
    pub first_batch_ms: Option<f64>,
    pub sel: usize,
    pub top: usize,
    pub focus: Focus,
    seq: u64,
    pub details: Option<Box<Details>>,
    cache: HashMap<Oid, Box<Details>>,
    pub sel_file: usize,
    file_top: usize,
    pub diff: Option<Box<FileDiff>>,
    diff_top: usize,
    pub palette: Option<Palette>,
    worker: Option<DetailWorker>,
    pub notice: Option<String>,
    pub stats: Stats,
    pub remote: bool,
    pub layout: DetailsLayout,
    pub settings: crate::settings::Settings,
    pub chrome: Chrome,
    cap_hover: Option<u8>,
    cap_pressed: Option<u8>,
    /// Hover and press of the maximise button, reported by the frame.
    pub max_state: (bool, bool),
    pub maximized: bool,
    pub active: bool,
    title_controls: Vec<(i32, i32)>,
    search_rect: Rect,
    pub win_action: Option<WinAction>,
    pub damage: Damage,
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Ui {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        te: TextEngine,
        f: Fonts,
        settings: crate::settings::Settings,
        repo: &std::path::Path,
        sink: Sink,
        w: usize,
        h: usize,
        s: f32,
    ) -> Ui {
        let repo_name = repo.file_name().map_or("repository".into(), |n| n.to_string_lossy().into_owned());
        // Details load as soon as the first rows arrive; git finds the
        // repository from any folder inside it, so no discovery wait.
        let worker = Some(DetailWorker::new(Git { root: repo.to_path_buf() }, &sink));
        let mut ui = Ui {
            s,
            w,
            h,
            lay: Layout::default(),
            te,
            f,
            repo_name,
            git: None,
            error: None,
            tz: 0,
            now: now_secs(),
            commits: Vec::new(),
            graph: Graph::new(),
            by_id: HashMap::new(),
            refs: Refs::default(),
            chips: HashMap::new(),
            side: Vec::new(),
            status: None,
            log_done: false,
            log_ms: 0.0,
            first_batch_ms: None,
            sel: 0,
            top: 0,
            focus: Focus::List,
            seq: 0,
            details: None,
            cache: HashMap::new(),
            sel_file: 0,
            file_top: 0,
            diff: None,
            diff_top: 0,
            palette: None,
            worker,
            stats: Stats::default(),
            remote: false,
            layout: DetailsLayout::from_setting(settings.get("ui.layout")),
            notice: settings.errors.first().cloned(),
            settings,
            chrome: if cfg!(windows) {
                Chrome::Windows11
            } else if cfg!(target_os = "macos") {
                Chrome::MacInset
            } else {
                Chrome::Native
            },
            cap_hover: None,
            cap_pressed: None,
            max_state: (false, false),
            maximized: false,
            active: true,
            title_controls: Vec::new(),
            search_rect: Rect::default(),
            win_action: None,
            damage: Damage { full: true, ..Default::default() },
        };
        ui.rebuild_side();
        ui.resize(w, h, s);
        ui
    }

    fn px(&self, v: f32) -> i32 {
        (v * self.s).round() as i32
    }
    fn pf(&self, v: f32) -> f32 {
        v * self.s
    }

    pub fn resize(&mut self, w: usize, h: usize, s: f32) {
        self.w = w;
        self.h = h;
        self.s = s;
        let (wi, hi) = (w as i32, h as i32);
        let top_h = self.px(46.0);
        let status_h = self.px(26.0);
        let side_w = self.px(236.0).min(wi / 4);
        let mid_h = (hi - top_h - status_h).max(0);
        let list_x = side_w;
        let (list_w, list_h, details) = match self.layout {
            DetailsLayout::Beside => {
                let det_w = ((wi as f32 * 0.40) as i32).clamp(self.px(420.0), self.px(780.0)).min(wi / 2);
                let list_w = (wi - side_w - det_w).max(0);
                (list_w, mid_h, Rect::new(list_x + list_w, top_h, det_w, mid_h))
            }
            DetailsLayout::Below => {
                let list_w = (wi - side_w).max(0);
                let list_h = ((mid_h as f32 * 0.46) as i32).max(self.px(150.0)).min(mid_h);
                (list_w, list_h, Rect::new(list_x, top_h + list_h, list_w, mid_h - list_h))
            }
        };
        let info = match self.layout {
            DetailsLayout::Beside => details,
            DetailsLayout::Below => {
                let iw = ((details.w as f32 * 0.38) as i32).clamp(self.px(360.0), self.px(620.0)).min(details.w);
                Rect::new(details.x, details.y, iw, details.h)
            }
        };
        let head_h = self.px(30.0);
        let row_h = self.px(30.0).max(12);
        let sb_w = self.px(8.0);
        let list = Rect::new(list_x, top_h, list_w, list_h);
        self.lay = Layout {
            top: Rect::new(0, 0, wi, top_h),
            status: Rect::new(0, hi - status_h, wi, status_h),
            side: Rect::new(0, top_h, side_w, mid_h),
            list,
            list_head: Rect::new(list_x, top_h, list_w, head_h),
            list_body: Rect::new(list_x, top_h + head_h, list_w - sb_w, list_h - head_h),
            scrollbar: Rect::new(list_x + list_w - sb_w, top_h + head_h, sb_w, list_h - head_h),
            details,
            info,
            row_h,
            caption: if self.chrome == Chrome::Windows11 {
                let cw = self.px(46.0) * 3;
                Rect::new(wi - cw, 0, cw, top_h)
            } else {
                Rect::default()
            },
        };
        self.top = self.top.min(self.max_top());
        self.damage.full = true;
    }

    pub fn visible_rows(&self) -> usize {
        (self.lay.list_body.h / self.lay.row_h.max(1)).max(1) as usize
    }

    fn max_top(&self) -> usize {
        self.commits.len().saturating_sub(self.visible_rows())
    }

    fn row_rect(&self, row: usize) -> Option<Rect> {
        if row < self.top {
            return None;
        }
        let i = (row - self.top) as i32;
        let b = self.lay.list_body;
        let y = b.y + i * self.lay.row_h;
        (y < b.bottom()).then(|| Rect::new(self.lay.list.x, y, self.lay.list.w - self.lay.scrollbar.w, self.lay.row_h))
    }

    // ---------------------------------------------------------------- data

    pub fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Repo(Ok((git, tz))) => {
                if let Some(n) = git.root.file_name() {
                    self.repo_name = n.to_string_lossy().into_owned();
                }
                self.tz = tz;
                self.git = Some(git);
                self.damage.rects.extend([self.lay.top, self.lay.list, self.lay.details]);
            }
            Msg::Repo(Err(e)) => {
                self.error = Some(e);
                self.damage.full = true;
            }
            Msg::Refs(Ok(r)) => {
                self.refs = r;
                self.index_chips();
                self.rebuild_side();
                self.damage.rects.extend([self.lay.side, self.lay.top]);
                self.damage_visible_rows();
            }
            Msg::Refs(Err(e)) => self.set_notice(format!("Could not read refs: {e}")),
            Msg::Status(s) => {
                self.status = Some(s);
                self.rebuild_side();
                self.damage.rects.push(self.lay.side);
            }
            Msg::Log { batch, done, elapsed_ms } => {
                let was_empty = self.commits.is_empty();
                let start = self.commits.len();
                for c in batch {
                    self.graph.push(&c);
                    self.by_id.insert(c.id, self.commits.len() as u32);
                    self.commits.push(c);
                }
                if self.first_batch_ms.is_none() {
                    self.first_batch_ms = Some(elapsed_ms);
                }
                self.log_done = done;
                self.log_ms = elapsed_ms;
                if was_empty && !self.commits.is_empty() {
                    self.index_chips();
                    self.select_head_or_first();
                }
                // Only rows that became visible need paint.
                let end = (self.top + self.visible_rows() + 1).min(self.commits.len());
                self.damage.rows.extend(start.max(self.top)..end);
                self.damage.rects.extend([self.lay.status, self.lay.scrollbar, self.lay.list_head]);
            }
            Msg::Details { seq, result } => match result {
                Ok(d) => {
                    let id = d.id;
                    if self.cache.len() > 512 {
                        self.cache.clear();
                    }
                    self.cache.insert(id, d.clone());
                    if seq == self.seq {
                        self.details = Some(d);
                        self.sel_file = 0;
                        self.file_top = 0;
                        self.request_diff();
                        self.damage.rects.push(self.lay.details);
                    }
                }
                Err(e) if seq == self.seq => self.set_notice(format!("Could not load commit: {e}")),
                Err(_) => {}
            },
            Msg::Diff { seq, file, result } => {
                if seq == self.seq && file == self.sel_file {
                    match result {
                        Ok(d) => self.diff = Some(d),
                        Err(e) => self.set_notice(format!("Could not load diff: {e}")),
                    }
                    self.damage.rects.push(self.diff_rect());
                }
            }
            Msg::WorkerPanic(e) => self.set_notice(format!("Internal error (window kept alive): {e}")),
        }
    }

    /// Switches the details layout and saves it as `ui.layout`.
    pub fn set_layout(&mut self, l: DetailsLayout) {
        if l == self.layout {
            return;
        }
        self.layout = l;
        self.settings.set_string("ui.layout", l.as_setting());
        self.file_top = 0;
        self.resize(self.w, self.h, self.s);
    }

    pub fn set_remote(&mut self, remote: bool) {
        if remote != self.remote {
            self.remote = remote;
            self.damage.rects.push(self.lay.status);
        }
    }

    fn set_notice(&mut self, n: String) {
        self.notice = Some(n);
        self.damage.rects.push(self.lay.status);
    }

    fn index_chips(&mut self) {
        self.chips.clear();
        for (i, r) in self.refs.refs.iter().enumerate() {
            self.chips.entry(r.target).or_default().push(i as u32);
        }
        for v in self.chips.values_mut() {
            let refs = &self.refs.refs;
            v.sort_by_key(|&i| {
                let r = &refs[i as usize];
                (!r.is_head, r.kind, r.name.len())
            });
        }
    }

    fn rebuild_side(&mut self) {
        let mut v = Vec::new();
        let item = |indent, kind, label: &str, badge: String, active, target| SideItem {
            indent,
            kind,
            label: label.to_string(),
            badge,
            active,
            target,
        };
        v.push(item(0, SideKind::Header, "Workspace", String::new(), false, None));
        let changes = self
            .status
            .map_or(String::new(), |s| if s.total() > 0 { fmt::num(s.total() as usize) } else { String::new() });
        v.push(item(0, SideKind::Changes, "Local Changes", changes, false, None));
        v.push(item(0, SideKind::All, "All Commits", String::new(), true, None));
        let refs = &self.refs.refs;
        let locals: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Local).collect();
        v.push(item(0, SideKind::Header, "Branches", fmt::num(locals.len()), false, None));
        for r in locals.iter().take(14) {
            let mut badge = String::new();
            if r.ahead > 0 {
                badge.push_str(&format!("↑{}", r.ahead));
            }
            if r.behind > 0 {
                if !badge.is_empty() {
                    badge.push(' ');
                }
                badge.push_str(&format!("↓{}", r.behind));
            }
            v.push(item(0, SideKind::Branch, &r.name, badge, r.is_head, Some(r.target)));
        }
        if locals.len() > 14 {
            v.push(item(0, SideKind::More, &format!("{} more", locals.len() - 14), String::new(), false, None));
        }
        let remotes: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Remote).collect();
        if !remotes.is_empty() {
            v.push(item(0, SideKind::Header, "Remotes", String::new(), false, None));
            let mut names: Vec<&str> = remotes.iter().filter_map(|r| r.name.split('/').next()).collect();
            names.dedup();
            for n in names {
                let rs: Vec<_> = remotes.iter().filter(|r| r.name.split('/').next() == Some(n)).collect();
                v.push(item(0, SideKind::Remote, n, fmt::num(rs.len()), false, None));
                for r in rs.iter().take(6) {
                    let short = r.name.split_once('/').map_or(&*r.name, |x| x.1);
                    v.push(item(1, SideKind::RemoteBranch, short, String::new(), false, Some(r.target)));
                }
                if rs.len() > 6 {
                    v.push(item(1, SideKind::More, &format!("{} more", rs.len() - 6), String::new(), false, None));
                }
            }
        }
        let tags: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Tag).collect();
        if !tags.is_empty() {
            v.push(item(0, SideKind::Header, "Tags", fmt::num(tags.len()), false, None));
            for r in tags.iter().rev().take(5) {
                v.push(item(0, SideKind::Tag, &r.name, String::new(), false, Some(r.target)));
            }
        }
        v.push(item(0, SideKind::Header, "Stashes", fmt::num(self.refs.stashes.len()), false, None));
        for (_, msg) in self.refs.stashes.iter().take(4) {
            v.push(item(0, SideKind::Stash, msg, String::new(), false, None));
        }
        if self.refs.worktrees.len() > 1 {
            v.push(item(0, SideKind::Header, "Worktrees", fmt::num(self.refs.worktrees.len()), false, None));
            for w in self.refs.worktrees.iter().take(4) {
                let name = w.rsplit(['/', '\\']).next().unwrap_or(w);
                v.push(item(0, SideKind::Worktree, name, String::new(), false, None));
            }
        }
        self.side = v;
    }

    fn select_head_or_first(&mut self) {
        let head = self.refs.head.and_then(|h| self.by_id.get(&h).copied());
        self.select(head.map_or(0, |i| i as usize));
    }

    // ------------------------------------------------------------ selection

    fn damage_visible_rows(&mut self) {
        let end = (self.top + self.visible_rows() + 1).min(self.commits.len());
        self.damage.rows.extend(self.top..end);
    }

    pub fn select(&mut self, row: usize) {
        if self.commits.is_empty() {
            return;
        }
        let row = row.min(self.commits.len() - 1);
        let old = self.sel;
        self.sel = row;
        self.damage.rows.extend([old, row]);
        let vis = self.visible_rows();
        if row < self.top {
            self.set_top(row);
        } else if row >= self.top + vis {
            self.set_top(row + 1 - vis);
        }
        self.seq += 1;
        self.diff = None;
        self.diff_top = 0;
        self.sel_file = 0;
        self.file_top = 0;
        let id = self.commits[row].id;
        // Same-frame answer: cached details show at once; otherwise the
        // header paints from the list data and the rest fills in.
        self.details = self.cache.get(&id).cloned();
        if self.details.is_some() {
            self.request_diff();
        } else {
            self.request_details();
        }
        self.damage.rects.extend([self.lay.details, self.lay.status]);
    }

    fn request_details(&mut self) {
        if let (Some(w), Some(c)) = (&self.worker, self.commits.get(self.sel)) {
            w.details(self.seq, c.id);
        }
    }

    fn request_diff(&mut self) {
        if let (Some(w), Some(d)) = (&self.worker, &self.details)
            && let Some(f) = d.files.get(self.sel_file)
        {
            w.diff(self.seq, d.id, d.parents.clone(), self.sel_file, f.clone());
        }
    }

    pub fn set_top(&mut self, top: usize) {
        let top = top.min(self.max_top());
        let d = top as i64 - self.top as i64;
        if d == 0 {
            return;
        }
        self.top = top;
        if d.unsigned_abs() as usize >= self.visible_rows() {
            self.damage.rects.push(self.lay.list_body);
            self.damage_visible_rows();
        } else {
            self.damage.scroll += d;
        }
        self.damage.rects.extend([self.lay.scrollbar, self.lay.status]);
    }

    fn select_file(&mut self, i: usize) {
        let Some(d) = &self.details else { return };
        if d.files.is_empty() {
            return;
        }
        let i = i.min(d.files.len() - 1);
        if i == self.sel_file && self.diff.is_some() {
            return;
        }
        self.sel_file = i;
        let vis = self.file_rows_visible();
        if i < self.file_top {
            self.file_top = i;
        } else if i >= self.file_top + vis {
            self.file_top = i + 1 - vis;
        }
        self.diff = None;
        self.diff_top = 0;
        self.seq += 1;
        self.request_diff();
        self.damage.rects.push(self.lay.details);
    }

    // ---------------------------------------------------------------- input

    pub fn key(&mut self, k: Key) {
        if self.palette.is_some() {
            self.palette_key(k);
            return;
        }
        let page = self.visible_rows().saturating_sub(1).max(1);
        match (self.focus, k) {
            (_, Key::Palette) => self.open_palette(),
            (_, Key::Tab) => self.cycle_focus(1),
            (_, Key::BackTab) => self.cycle_focus(-1),
            (Focus::List, Key::Up) | (Focus::List, Key::Char('k')) => self.select(self.sel.saturating_sub(1)),
            (Focus::List, Key::Down) | (Focus::List, Key::Char('j')) => self.select(self.sel + 1),
            (Focus::List, Key::PageUp) => self.select(self.sel.saturating_sub(page)),
            (Focus::List, Key::PageDown) => self.select(self.sel + page),
            (Focus::List, Key::Home) => self.select(0),
            (Focus::List, Key::End) => self.select(usize::MAX),
            (Focus::List, Key::Enter) => self.cycle_focus(1),
            (Focus::Files, Key::Up) | (Focus::Files, Key::Char('k')) => {
                self.select_file(self.sel_file.saturating_sub(1))
            }
            (Focus::Files, Key::Down) | (Focus::Files, Key::Char('j')) => self.select_file(self.sel_file + 1),
            (Focus::Files, Key::Enter) => self.cycle_focus(1),
            (Focus::Diff, Key::Up) | (Focus::Diff, Key::Char('k')) => self.scroll_diff(-1),
            (Focus::Diff, Key::Down) | (Focus::Diff, Key::Char('j')) => self.scroll_diff(1),
            (Focus::Diff, Key::PageUp) => self.scroll_diff(-20),
            (Focus::Diff, Key::PageDown) => self.scroll_diff(20),
            (_, Key::Esc) => {
                if self.focus != Focus::List {
                    self.focus = Focus::List;
                    self.damage.rects.push(self.lay.details);
                    self.damage.rows.push(self.sel);
                }
            }
            (_, Key::Char('/')) => self.open_palette(),
            _ => {}
        }
    }

    fn cycle_focus(&mut self, dir: i32) {
        let order = [Focus::List, Focus::Files, Focus::Diff];
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0) as i32;
        self.focus = order[((i + dir).rem_euclid(3)) as usize];
        self.damage.rects.push(self.lay.details);
        self.damage.rows.push(self.sel);
    }

    fn scroll_diff(&mut self, d: i64) {
        let n = self.diff.as_ref().map_or(0, |d| d.lines.len());
        let top = (self.diff_top as i64 + d).clamp(0, n.saturating_sub(1) as i64) as usize;
        if top != self.diff_top {
            self.diff_top = top;
            self.damage.rects.push(self.diff_rect());
        }
    }

    pub fn wheel(&mut self, x: i32, y: i32, lines: f32) {
        let n = lines.round() as i64;
        if n == 0 {
            return;
        }
        if self.lay.list.contains(x, y) {
            self.set_top((self.top as i64 + n).max(0) as usize);
        } else if self.diff_rect().contains(x, y) {
            self.scroll_diff(n);
        }
    }

    pub fn click(&mut self, x: i32, y: i32) {
        if let Some(b) = self.caption_button_at(x, y) {
            if b != 1 {
                self.cap_pressed = Some(b);
                self.damage.rects.push(self.lay.caption);
            }
            return;
        }
        if self.palette.is_some() {
            self.close_palette();
            return;
        }
        let b = self.lay.list_body;
        if b.contains(x, y) {
            let row = self.top + ((y - b.y) / self.lay.row_h) as usize;
            if row < self.commits.len() {
                if self.focus != Focus::List {
                    self.focus = Focus::List;
                    self.damage.rects.push(self.lay.details);
                }
                self.select(row);
            }
            return;
        }
        let fr = self.files_rect();
        if fr.contains(x, y) {
            let i = self.file_top + ((y - fr.y) / self.px(26.0)) as usize;
            self.focus = Focus::Files;
            self.damage.rows.push(self.sel);
            self.select_file(i);
            self.damage.rects.push(self.lay.details);
            return;
        }
        if self.lay.side.contains(x, y) {
            let mut yy = self.lay.side.y + self.px(8.0);
            for it in &self.side {
                let h = if it.kind == SideKind::Header { self.px(32.0) } else { self.px(28.0) };
                if y >= yy && y < yy + h {
                    if let Some(t) = it.target
                        && let Some(&row) = self.by_id.get(&t)
                    {
                        self.select(row as usize);
                    }
                    return;
                }
                yy += h;
            }
        }
        if self.search_rect.contains(x, y) {
            self.open_palette();
        }
    }

    // -------------------------------------------------------------- palette

    pub fn open_palette(&mut self) {
        self.palette = Some(Palette { query: String::new(), items: Vec::new(), sel: 0 });
        self.filter_palette();
    }

    pub fn open_palette_with(&mut self, q: &str) {
        self.open_palette();
        if let Some(p) = &mut self.palette {
            p.query = q.into();
        }
        self.filter_palette();
    }

    fn close_palette(&mut self) {
        if self.palette.take().is_some() {
            self.damage.rects.push(self.palette_rect(10));
        }
    }

    fn filter_palette(&mut self) {
        let Some(p) = &mut self.palette else { return };
        let q = p.query.to_lowercase();
        let mut scored: Vec<(i32, PItem)> = Vec::new();
        for r in &self.refs.refs {
            if let Some(sc) = fuzzy(&q, &r.name) {
                let hint = match r.kind {
                    RefKind::Local => "branch",
                    RefKind::Remote => "remote branch",
                    RefKind::Tag => "tag",
                    _ => "",
                };
                scored.push((
                    sc + if r.is_head { 5 } else { 0 },
                    PItem {
                        label: r.name.to_string(),
                        hint: hint.into(),
                        kind: r.kind,
                        action: PAction::Goto(r.target),
                    },
                ));
            }
        }
        for (c, keys) in COMMANDS {
            if let Some(sc) = fuzzy(&q, c) {
                scored.push((
                    sc - 2,
                    PItem {
                        label: c.to_string(),
                        hint: keys.to_string(),
                        kind: RefKind::Head,
                        action: PAction::Command(c),
                    },
                ));
            }
        }
        if q.len() >= 4 && q.bytes().all(|b| b.is_ascii_hexdigit()) {
            for c in self.commits.iter().filter(|c| c.id.to_string().starts_with(&q)).take(3) {
                scored.push((
                    100,
                    PItem {
                        label: c.subject.to_string(),
                        hint: c.id.short(9),
                        kind: RefKind::Stash,
                        action: PAction::Goto(c.id),
                    },
                ));
            }
        }
        scored.sort_by_key(|x| std::cmp::Reverse(x.0));
        p.items = scored.into_iter().take(10).map(|x| x.1).collect();
        p.sel = 0;
        let r = self.palette_rect(10);
        self.damage.rects.push(r);
    }

    fn palette_key(&mut self, k: Key) {
        let Some(p) = &mut self.palette else { return };
        match k {
            Key::Esc | Key::Palette => self.close_palette(),
            Key::Up => {
                p.sel = p.sel.saturating_sub(1);
                let r = self.palette_rect(10);
                self.damage.rects.push(r);
            }
            Key::Down | Key::Tab => {
                p.sel = (p.sel + 1).min(p.items.len().saturating_sub(1));
                let r = self.palette_rect(10);
                self.damage.rects.push(r);
            }
            Key::Backspace => {
                p.query.pop();
                self.filter_palette();
            }
            Key::Char(c) => {
                p.query.push(c);
                self.filter_palette();
            }
            Key::Enter => {
                let action = p.items.get(p.sel).map(|i| i.action.clone());
                self.close_palette();
                match action {
                    Some(PAction::Goto(id)) => match self.by_id.get(&id) {
                        Some(&row) => self.select(row as usize),
                        None => self.set_notice("That commit is not loaded yet.".into()),
                    },
                    Some(PAction::Command(c)) if c == LAYOUT_BELOW => self.set_layout(DetailsLayout::Below),
                    Some(PAction::Command(c)) if c == LAYOUT_BESIDE => self.set_layout(DetailsLayout::Beside),
                    Some(PAction::Command(c)) => self.set_notice(format!("“{c}” is not in the proof of concept yet.")),
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn palette_rect(&self, items: usize) -> Rect {
        let w = self.px(640.0).min(self.w as i32 - self.px(80.0));
        let h = self.px(52.0) + items as i32 * self.px(34.0) + self.px(10.0);
        Rect::new((self.w as i32 - w) / 2, self.lay.top.bottom() + self.px(10.0), w, h)
    }

    // ------------------------------------------------------------- painting

    pub fn paint(&mut self, cv: &mut Canvas, force_full: bool) -> Painted {
        let t = std::time::Instant::now();
        self.now = now_secs();
        let mut dmg = std::mem::take(&mut self.damage);
        let mut copy = None;
        let mut rects = Vec::new();
        if dmg.full || force_full {
            rects.push(cv.bounds());
        } else {
            if dmg.scroll != 0 {
                let d = dmg.scroll;
                let rows = self.visible_rows() as i64;
                let area = self.lay.list_body;
                if d.abs() < rows {
                    let dy = -(d as i32) * self.lay.row_h;
                    // Only whole rows move; a partial last row is repainted.
                    let full_rows = Rect::new(area.x, area.y, area.w, rows as i32 * self.lay.row_h);
                    cv.scroll(full_rows, dy);
                    copy = Some((full_rows, dy));
                    let exposed: Vec<usize> = if d > 0 {
                        (self.top + (rows - d) as usize..self.top + rows as usize + 1).collect()
                    } else {
                        (self.top..self.top + (-d) as usize).collect()
                    };
                    dmg.rows.extend(exposed);
                    rects.push(Rect::new(area.x, full_rows.bottom(), area.w, area.bottom() - full_rows.bottom()));
                } else {
                    rects.push(area);
                }
            }
            dmg.rows.sort_unstable();
            dmg.rows.dedup();
            for r in dmg.rows {
                if let Some(rr) = self.row_rect(r) {
                    rects.push(rr.intersect(self.lay.list_body.intersect(Rect::new(
                        0,
                        0,
                        self.w as i32,
                        self.h as i32,
                    ))));
                }
            }
            rects.extend(dmg.rects);
        }
        let rects = merge_rects(rects);
        for &r in &rects {
            let old = cv.set_clip(r);
            self.paint_scene(cv);
            cv.set_clip(old);
        }
        self.stats.presents += 1;
        self.stats.last_damage_px = rects.iter().map(|r| r.area()).sum();
        self.stats.last_paint_ms = t.elapsed().as_secs_f64() * 1000.0;
        Painted { rects, copy }
    }

    fn paint_scene(&mut self, cv: &mut Canvas) {
        let clip = cv.clip();
        let l = self.lay;
        if !clip.intersect(l.top).is_empty() {
            self.paint_top(cv);
        }
        if !clip.intersect(l.side).is_empty() {
            self.paint_side(cv);
        }
        if !clip.intersect(l.list).is_empty() {
            self.paint_list(cv);
        }
        if !clip.intersect(l.details).is_empty() {
            self.paint_details(cv);
        }
        if !clip.intersect(l.status).is_empty() {
            self.paint_status(cv);
        }
        if self.palette.is_some() && !clip.intersect(self.palette_rect(10)).is_empty() {
            self.paint_palette(cv);
        }
    }

    fn chevrons(&self, cv: &mut Canvas, x: f32, cy: f32, size: f32, color: u32) {
        let h = size / 2.0;
        let w = self.pf(2.6);
        for dx in [0.0, size * 0.62] {
            cv.stroke(&[(x + dx, cy - h), (x + dx + h, cy), (x + dx, cy + h)], w, color);
        }
    }

    fn paint_top(&mut self, cv: &mut Canvas) {
        let r = self.lay.top;
        cv.fill(r, BG_BAR);
        cv.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1), BORDER);
        let cy = r.y as f32 + r.h as f32 / 2.0;
        // App mark: the logo's fast-forward chevrons, solid.
        // macOS keeps its traffic lights at the left of the bar.
        let left = if self.chrome == Chrome::MacInset { self.pf(80.0) } else { self.pf(14.0) };
        let mut controls = Vec::new();
        let mark = Rect::new(left as i32, (cy - self.pf(13.0)) as i32, self.px(26.0), self.px(26.0));
        cv.fill_round(mark, self.pf(7.0), ACCENT);
        self.chevrons(cv, mark.x as f32 + self.pf(6.5), cy, self.pf(10.0), ACCENT_TEXT);
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let base = (cy + self.pf(5.0)).round();
        let mut x = mark.right() as f32 + self.pf(12.0);
        x = self.te.draw(cv, semi, self.pf(14.5), x, base, &self.repo_name.clone(), TEXT, self.pf(220.0));
        x += self.pf(12.0);
        // Current branch pill.
        let branch =
            self.refs.head_branch.as_deref().unwrap_or(if self.git.is_some() { "detached" } else { "…" }).to_string();
        let bw = self.te.measure(semi, self.pf(12.5), &branch) + self.pf(36.0);
        let pill = Rect::new(x as i32, (cy - self.pf(13.0)) as i32, bw as i32, self.px(26.0));
        cv.round_border(pill, self.pf(13.0), BORDER, BG_INPUT);
        controls.push((pill.x, pill.right()));
        self.branch_icon(cv, pill.x as f32 + self.pf(14.0), cy, TEXT_2);
        self.te.draw(cv, semi, self.pf(12.5), pill.x as f32 + self.pf(25.0), base - self.pf(0.5), &branch, TEXT, bw);
        // Search / command field.
        let sw = self.px(460.0).min(r.w - self.px(760.0)).max(self.px(200.0));
        let search = Rect::new((r.w - sw) / 2, (cy - self.pf(15.0)) as i32, sw, self.px(30.0));
        cv.round_border(search, self.pf(8.0), BORDER, BG_INPUT);
        controls.push((search.x, search.right()));
        self.search_rect = search;
        let sx = search.x as f32 + self.pf(14.0);
        cv.ring(sx + self.pf(4.5), cy - self.pf(1.0), self.pf(5.0), self.pf(1.6), TEXT_3);
        cv.stroke(
            &[(sx + self.pf(8.0), cy + self.pf(2.5)), (sx + self.pf(11.0), cy + self.pf(5.5))],
            self.pf(1.6),
            TEXT_3,
        );
        self.te.draw(
            cv,
            ui,
            self.pf(13.0),
            sx + self.pf(20.0),
            base,
            "Jump to branch, commit or command",
            TEXT_3,
            sw as f32 - self.pf(110.0),
        );
        self.keycap(cv, search.right() as f32 - self.pf(12.0), cy, "Ctrl K", true);
        // Actions with ahead/behind from the current branch.
        let head = self.refs.refs.iter().find(|r| r.is_head);
        let (ahead, behind) = head.map_or((0, 0), |h| (h.ahead, h.behind));
        let right = if self.lay.caption.is_empty() { r.right() } else { self.lay.caption.x };
        let mut bx = right as f32 - self.pf(12.0);
        let ai = self.button(cv, bx, cy, "AI", None, true);
        bx = ai - self.pf(8.0);
        let push = self.button(cv, bx, cy, "Push", (ahead > 0).then(|| format!("↑{ahead}")), false);
        bx = push - self.pf(8.0);
        let pull = self.button(cv, bx, cy, "Pull", (behind > 0).then(|| format!("↓{behind}")), false);
        bx = pull - self.pf(8.0);
        let fetch = self.button(cv, bx, cy, "Fetch", None, false);
        controls.push((fetch as i32, (right as f32 - self.pf(12.0)) as i32));
        self.title_controls = controls;
        let _ = mono;
        self.paint_caption(cv);
    }

    /// Minimise, maximise/restore and close, with Windows 11 metrics and
    /// colours. Crisp 1 px glyphs drawn as pixel-aligned fills.
    fn paint_caption(&mut self, cv: &mut Canvas) {
        let r = self.lay.caption;
        if r.is_empty() || cv.clip().intersect(r).is_empty() {
            return;
        }
        let bw = r.w / 3;
        let line = self.px(1.0).max(1);
        let s = self.px(5.0);
        for i in 0..3u8 {
            let x = r.x + i as i32 * bw;
            let b = Rect::new(x, r.y, if i == 2 { r.right() - x } else { bw }, r.h - 1);
            let (hover, pressed) = if i == 1 {
                self.max_state
            } else {
                (self.cap_hover == Some(i), self.cap_pressed == Some(i) && self.cap_hover == Some(i))
            };
            let idle = if self.active { TEXT } else { TEXT_3 };
            let (fill, glyph) = match (i, hover, pressed) {
                (2, true, true) => (0x9e2a1f, 0xffffff),
                (2, true, false) => (0xc42b1c, 0xffffff),
                (_, true, true) => (0x262a31, TEXT),
                (_, true, false) => (0x1f232a, TEXT),
                _ => (BG_BAR, idle),
            };
            cv.fill(b, fill);
            let (cx, cy) = (b.x + b.w / 2, b.y + b.h / 2);
            match i {
                0 => cv.fill(Rect::new(cx - s, cy, 2 * s, line), glyph),
                1 if self.maximized => {
                    let k = self.px(2.0);
                    // Back window: only its top and right edges show.
                    cv.fill(Rect::new(cx - s + k, cy - s, 2 * s - k, line), glyph);
                    cv.fill(Rect::new(cx + s - line, cy - s, line, 2 * s - k), glyph);
                    let front = Rect::new(cx - s, cy - s + k, 2 * s - k, 2 * s - k);
                    cv.fill(front, fill);
                    cv.stroke_rect(front, glyph);
                }
                1 => cv.stroke_rect(Rect::new(cx - s, cy - s, 2 * s, 2 * s), glyph),
                _ => {
                    let (fx, fy, fs) = (cx as f32, cy as f32, s as f32);
                    let w = self.pf(1.0).max(1.0);
                    cv.stroke(&[(fx - fs, fy - fs), (fx + fs, fy + fs)], w, glyph);
                    cv.stroke(&[(fx - fs, fy + fs), (fx + fs, fy - fs)], w, glyph);
                }
            }
        }
    }

    fn caption_button_at(&self, x: i32, y: i32) -> Option<u8> {
        let r = self.lay.caption;
        if r.is_empty() || !r.contains(x, y) {
            return None;
        }
        Some((((x - r.x) / (r.w / 3).max(1)) as u8).min(2))
    }

    /// Pointer moved in the client area (hover for minimise and close).
    pub fn pointer(&mut self, x: i32, y: i32) {
        let h = self.caption_button_at(x, y).filter(|&b| b != 1);
        if h != self.cap_hover {
            self.cap_hover = h;
            self.damage.rects.push(self.lay.caption);
        }
    }

    pub fn pointer_left(&mut self) {
        if self.cap_hover.take().is_some() {
            self.damage.rects.push(self.lay.caption);
        }
    }

    /// Mouse button released: a caption button acts only if the press
    /// started on it and the pointer is still over it (as on Windows).
    pub fn release(&mut self, x: i32, y: i32) {
        if let Some(b) = self.cap_pressed.take() {
            if self.caption_button_at(x, y) == Some(b) {
                self.win_action = Some(if b == 0 { WinAction::Minimize } else { WinAction::Close });
            }
            self.damage.rects.push(self.lay.caption);
        }
    }

    /// The frame reported a change of the maximise button or window state.
    pub fn set_frame_state(&mut self, max_state: (bool, bool), maximized: bool) {
        if max_state != self.max_state || maximized != self.maximized {
            self.max_state = max_state;
            self.maximized = maximized;
            self.damage.rects.push(self.lay.caption);
        }
    }

    pub fn set_active(&mut self, active: bool) {
        if active != self.active {
            self.active = active;
            self.damage.rects.push(self.lay.caption);
        }
    }

    pub fn title_geometry(&self) -> TitleGeometry {
        let c = self.lay.caption;
        let bw = c.w / 3;
        TitleGeometry {
            height: self.lay.top.h,
            controls: self.title_controls.clone(),
            buttons_left: if c.is_empty() { i32::MAX } else { c.x },
            max_button: (c.x + bw, c.x + 2 * bw),
        }
    }

    /// Right-aligned button ending at `right`; returns its left edge.
    fn button(
        &mut self,
        cv: &mut Canvas,
        right: f32,
        cy: f32,
        label: &str,
        badge: Option<String>,
        accent: bool,
    ) -> f32 {
        let semi = self.f.semi;
        let size = self.pf(12.5);
        let tw = self.te.measure(semi, size, label);
        let bw_badge = badge.as_ref().map_or(0.0, |b| self.te.measure(semi, self.pf(11.5), b) + self.pf(8.0));
        let icon = if accent { self.pf(16.0) } else { 0.0 };
        let w = tw + bw_badge + icon + self.pf(24.0);
        let r = Rect::new((right - w) as i32, (cy - self.pf(14.0)) as i32, w as i32, self.px(28.0));
        if accent {
            cv.round_border(r, self.pf(8.0), 0x5a2a1c, 0x2a1712);
            self.sparkle(cv, r.x as f32 + self.pf(16.0), cy, self.pf(5.5), ACCENT);
        } else {
            cv.round_border(r, self.pf(8.0), BORDER, BG_BAR);
        }
        let base = (cy + self.pf(4.5)).round();
        let tx = r.x as f32 + self.pf(12.0) + icon;
        let end = self.te.draw(cv, semi, size, tx, base, label, if accent { 0xffb79c } else { TEXT }, w);
        if let Some(b) = badge {
            self.te.draw(cv, semi, self.pf(11.5), end + self.pf(6.0), base, &b, ACCENT, w);
        }
        r.x as f32
    }

    fn sparkle(&self, cv: &mut Canvas, cx: f32, cy: f32, r: f32, color: u32) {
        let w = self.pf(1.8);
        cv.stroke(&[(cx, cy - r), (cx, cy + r)], w, color);
        cv.stroke(&[(cx - r, cy), (cx + r, cy)], w, color);
        cv.fill_circle(cx, cy, self.pf(2.2), color);
    }

    fn branch_icon(&self, cv: &mut Canvas, cx: f32, cy: f32, color: u32) {
        let w = self.pf(1.5);
        let (x0, x1) = (cx - self.pf(3.0), cx + self.pf(3.5));
        cv.stroke(&[(x0, cy - self.pf(5.0)), (x0, cy + self.pf(5.0))], w, color);
        cv.bezier((x1, cy - self.pf(2.5)), (x1, cy + self.pf(1.0)), (x0, cy), (x0, cy + self.pf(3.5)), w, color);
        cv.fill_circle(x1, cy - self.pf(3.5), self.pf(2.0), color);
    }

    fn keycap(&mut self, cv: &mut Canvas, right: f32, cy: f32, label: &str, dim: bool) -> f32 {
        let ui = self.f.ui;
        let size = self.pf(11.0);
        let w = self.te.measure(ui, size, label) + self.pf(12.0);
        let r = Rect::new((right - w) as i32, (cy - self.pf(10.0)) as i32, w as i32, self.px(20.0));
        cv.round_border(r, self.pf(5.0), BORDER, if dim { BG_BAR } else { BG_INPUT });
        self.te.draw(cv, ui, size, r.x as f32 + self.pf(6.0), (cy + self.pf(4.0)).round(), label, TEXT_3, w);
        r.x as f32
    }

    fn paint_side(&mut self, cv: &mut Canvas) {
        let r = self.lay.side;
        cv.fill(r, BG_SIDEBAR);
        cv.fill(Rect::new(r.right() - 1, r.y, 1, r.h), BORDER);
        let (ui, semi) = (self.f.ui, self.f.semi);
        let mut y = r.y + self.px(8.0);
        let items = std::mem::take(&mut self.side);
        for it in &items {
            let h = if it.kind == SideKind::Header { self.px(32.0) } else { self.px(28.0) };
            if y > r.bottom() {
                break;
            }
            let row = Rect::new(r.x + self.px(8.0), y, r.w - self.px(16.0), h);
            if cv.clip().intersect(row).is_empty() {
                y += h;
                continue;
            }
            let base = (y as f32 + h as f32 / 2.0 + self.pf(4.5)).round();
            let cy = y as f32 + h as f32 / 2.0;
            if it.kind == SideKind::Header {
                let base = (y as f32 + h as f32 - self.pf(9.0)).round();
                self.te.draw(
                    cv,
                    semi,
                    self.pf(11.0),
                    row.x as f32 + self.pf(10.0),
                    base,
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
                        base,
                        &it.badge,
                        TEXT_4,
                        bw + 2.0,
                    );
                }
                y += h;
                continue;
            }
            if it.active {
                cv.fill_round(row, self.pf(6.0), if it.kind == SideKind::All { BG_SELECT } else { BG_HOVER });
            }
            let ix = row.x as f32 + self.pf(16.0) + it.indent as f32 * self.pf(14.0);
            let (icon_color, text_color) = match it.kind {
                SideKind::Branch if it.active => (ACCENT, TEXT),
                SideKind::Branch => (LOCAL_FG, TEXT_2),
                SideKind::Remote => (REMOTE_FG, TEXT_2),
                SideKind::RemoteBranch => (TEXT_4, TEXT_3),
                SideKind::Tag => (TAG_FG, TEXT_2),
                SideKind::Stash => (STASH_FG, TEXT_2),
                SideKind::Changes => (MOD, TEXT),
                SideKind::All => (TEXT_2, TEXT),
                SideKind::Worktree => (TEXT_3, TEXT_2),
                SideKind::More => (TEXT_4, TEXT_4),
                SideKind::Header => (TEXT_3, TEXT_3),
            };
            match it.kind {
                SideKind::Branch | SideKind::RemoteBranch => self.branch_icon(cv, ix, cy, icon_color),
                SideKind::Tag => {
                    let s = self.pf(4.0);
                    cv.stroke(
                        &[(ix - s, cy), (ix, cy - s), (ix + s, cy), (ix, cy + s), (ix - s, cy)],
                        self.pf(1.5),
                        icon_color,
                    );
                }
                SideKind::Remote => {
                    cv.ring(ix, cy, self.pf(5.0), self.pf(1.5), icon_color);
                    cv.stroke(&[(ix - self.pf(5.0), cy), (ix + self.pf(5.0), cy)], self.pf(1.2), icon_color);
                }
                SideKind::Stash => {
                    let b =
                        Rect::new((ix - self.pf(5.0)) as i32, (cy - self.pf(4.0)) as i32, self.px(10.0), self.px(8.0));
                    cv.round_border(b, self.pf(2.0), icon_color, BG_SIDEBAR);
                }
                SideKind::Changes => cv.fill_circle(ix, cy, self.pf(4.0), icon_color),
                SideKind::All => {
                    for k in 0..3 {
                        let yy = cy - self.pf(4.0) + k as f32 * self.pf(4.0);
                        cv.stroke(&[(ix - self.pf(5.0), yy), (ix + self.pf(5.0), yy)], self.pf(1.5), icon_color);
                    }
                }
                SideKind::Worktree => {
                    let b =
                        Rect::new((ix - self.pf(5.0)) as i32, (cy - self.pf(5.0)) as i32, self.px(10.0), self.px(10.0));
                    cv.round_border(b, self.pf(2.5), icon_color, BG_SIDEBAR);
                }
                _ => {}
            }
            let font = if it.active { semi } else { ui };
            let tx = ix + self.pf(14.0);
            let badge_w =
                if it.badge.is_empty() { 0.0 } else { self.te.measure(semi, self.pf(11.0), &it.badge) + self.pf(14.0) };
            self.te.draw(
                cv,
                font,
                self.pf(13.0),
                tx,
                base,
                &it.label,
                text_color,
                row.right() as f32 - tx - badge_w - self.pf(6.0),
            );
            if !it.badge.is_empty() {
                let bw = badge_w - self.pf(14.0);
                let bx = row.right() as f32 - bw - self.pf(10.0);
                let color = if it.kind == SideKind::Changes {
                    MOD
                } else if it.kind == SideKind::Branch {
                    ACCENT
                } else {
                    TEXT_3
                };
                if it.kind == SideKind::Changes {
                    let b = Rect::new(
                        (bx - self.pf(6.0)) as i32,
                        (cy - self.pf(9.0)) as i32,
                        (bw + self.pf(12.0)) as i32,
                        self.px(18.0),
                    );
                    cv.fill_round(b, self.pf(9.0), 0x33290f);
                }
                self.te.draw(cv, semi, self.pf(11.0), bx, base - self.pf(0.5), &it.badge, color, bw + 2.0);
            }
            y += h;
        }
        self.side = items;
    }

    fn lane_w(&self) -> f32 {
        self.pf(14.0)
    }

    fn graph_lanes_shown(&self) -> u16 {
        self.graph.max_width.clamp(1, 10)
    }

    fn list_columns(&self) -> (f32, f32, f32, f32) {
        // (text start, author x, date x, sha x)
        let l = self.lay.list;
        let gx = l.x as f32 + self.pf(10.0);
        let text_x = gx + self.graph_lanes_shown() as f32 * self.lane_w() + self.pf(10.0);
        let sha_x = (l.right() - self.lay.scrollbar.w) as f32 - self.pf(78.0);
        let date_x = sha_x - self.pf(104.0);
        let author_x = date_x - self.pf(138.0);
        (text_x, author_x, date_x, sha_x)
    }

    fn paint_list(&mut self, cv: &mut Canvas) {
        let l = self.lay.list;
        let clip = cv.clip();
        let (ui, semi) = (self.f.ui, self.f.semi);
        let (text_x, author_x, date_x, sha_x) = self.list_columns();
        let head = self.lay.list_head;
        if !clip.intersect(head).is_empty() {
            cv.fill(head, BG);
            cv.fill(Rect::new(head.x, head.bottom() - 1, head.w, 1), BORDER_SOFT);
            let base = (head.y as f32 + head.h as f32 / 2.0 + self.pf(4.0)).round();
            let n = if self.log_done {
                fmt::num(self.commits.len())
            } else {
                format!("{}…", fmt::num(self.commits.len()))
            };
            let title = format!("All commits  ·  {n}");
            self.te.draw(cv, semi, self.pf(11.5), l.x as f32 + self.pf(14.0), base, &title, TEXT_3, self.pf(260.0));
            for (x, t) in [(author_x, "Author"), (date_x, "Date"), (sha_x, "Commit")] {
                self.te.draw(cv, semi, self.pf(11.5), x, base, t, TEXT_4, self.pf(100.0));
            }
        }
        let body = Rect::new(l.x, self.lay.list_body.y, l.w - self.lay.scrollbar.w, self.lay.list_body.h);
        if !clip.intersect(body).is_empty() {
            let first = self.top + ((clip.y - body.y).max(0) / self.lay.row_h) as usize;
            let last = self.top + ((clip.bottom() - body.y).max(0) / self.lay.row_h) as usize;
            let mut y = body.y + (first - self.top) as i32 * self.lay.row_h;
            for row in first..=last {
                if y >= body.bottom() {
                    break;
                }
                let rr = Rect::new(body.x, y, body.w, self.lay.row_h);
                let old = cv.set_clip(clip.intersect(body).intersect(rr));
                self.paint_row(cv, row, rr, text_x, author_x, date_x, sha_x, ui, semi);
                cv.set_clip(old);
                y += self.lay.row_h;
            }
        }
        let sb = self.lay.scrollbar;
        if !clip.intersect(sb).is_empty() {
            cv.fill(sb, BG);
            let n = self.commits.len().max(1);
            let vis = self.visible_rows();
            if n > vis {
                let th = ((sb.h as f32 * vis as f32 / n as f32) as i32).max(self.px(24.0));
                let ty = sb.y + ((sb.h - th) as f32 * self.top as f32 / (n - vis) as f32) as i32;
                cv.fill_round(Rect::new(sb.x + self.px(2.0), ty, sb.w - self.px(4.0), th), self.pf(3.0), 0x2a2f38);
            }
            // Selection marker on the scroll track.
            let my = sb.y + (sb.h as f32 * self.sel as f32 / n as f32) as i32;
            cv.fill(Rect::new(sb.x + self.px(1.0), my, sb.w - self.px(2.0), self.px(2.0)), ACCENT);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_row(
        &mut self,
        cv: &mut Canvas,
        row: usize,
        r: Rect,
        text_x: f32,
        author_x: f32,
        date_x: f32,
        sha_x: f32,
        ui: FontId,
        semi: FontId,
    ) {
        let selected = row == self.sel;
        let bg = if selected { if self.focus == Focus::List { BG_SELECT } else { BG_SELECT_DIM } } else { BG };
        cv.fill(r, bg);
        if row >= self.commits.len() {
            if !self.log_done && row < self.top + self.visible_rows() {
                // Placeholder bars while history streams in: solid, no animation.
                let y = r.y + r.h / 2 - self.px(4.0);
                cv.fill_round(Rect::new(text_x as i32, y, self.px(240.0), self.px(8.0)), self.pf(4.0), BG_HOVER);
            }
            return;
        }
        if selected {
            cv.fill(Rect::new(r.x, r.y, self.px(3.0), r.h), ACCENT);
        }
        self.paint_graph(cv, row, r, bg);
        let c = &self.commits[row];
        let base = (r.y as f32 + r.h as f32 / 2.0 + self.pf(4.5)).round();
        let mut x = text_x;
        // Ref chips.
        let chips: Vec<u32> = self.chips.get(&c.id).cloned().unwrap_or_default();
        let max_x = author_x - self.pf(16.0);
        for (k, &ri) in chips.iter().enumerate() {
            if k == 3 {
                let more = format!("+{}", chips.len() - 3);
                x = self.te.draw(cv, semi, self.pf(11.0), x, base, &more, TEXT_3, max_x - x) + self.pf(8.0);
                break;
            }
            let rf = &self.refs.refs[ri as usize];
            let (cbg, cfg) = match rf.kind {
                _ if rf.is_head => (ACCENT, ACCENT_TEXT),
                RefKind::Local => (LOCAL_BG, LOCAL_FG),
                RefKind::Remote => (REMOTE_BG, REMOTE_FG),
                RefKind::Tag => (TAG_BG, TAG_FG),
                _ => (STASH_BG, STASH_FG),
            };
            let name = rf.name.to_string();
            let tw = self.te.measure(semi, self.pf(11.0), &name).min(self.pf(160.0));
            let cw = tw + self.pf(14.0);
            if x + cw > max_x - self.pf(60.0) {
                break;
            }
            let chip =
                Rect::new(x as i32, (r.y as f32 + r.h as f32 / 2.0 - self.pf(9.5)) as i32, cw as i32, self.px(19.0));
            cv.fill_round(chip, self.pf(5.0), cbg);
            self.te.draw(cv, semi, self.pf(11.0), x + self.pf(7.0), base - self.pf(0.5), &name, cfg, tw + 1.0);
            x += cw + self.pf(6.0);
        }
        let merge = c.parents.len() > 1;
        let subj_color = if merge { TEXT_2 } else { TEXT };
        let end = self.te.draw(cv, ui, self.pf(13.0), x, base, &c.subject, subj_color, max_x - x);
        if c.has_body && end + self.pf(16.0) < max_x {
            // "Has a description" marker.
            let mx = end + self.pf(9.0);
            for k in 0..3 {
                cv.fill_circle(mx + k as f32 * self.pf(3.5), base - self.pf(3.5), self.pf(1.1), TEXT_4);
            }
        }
        let dim = if selected { TEXT_2 } else { TEXT_3 };
        self.te.draw(cv, ui, self.pf(12.5), author_x, base, &c.author, TEXT_2, date_x - author_x - self.pf(12.0));
        let when = fmt::relative(c.time, self.now, self.tz);
        self.te.draw(cv, ui, self.pf(12.5), date_x, base, &when, dim, sha_x - date_x - self.pf(10.0));
        self.te.draw(cv, self.f.mono, self.pf(12.0), sha_x, base, &c.id.short(8), dim, self.pf(80.0));
    }

    fn paint_graph(&self, cv: &mut Canvas, row: usize, r: Rect, bg: u32) {
        let Some(g) = self.graph.rows.get(row) else { return };
        let gx = self.lay.list.x as f32 + self.pf(10.0);
        let lw = self.lane_w();
        let shown = self.graph_lanes_shown();
        let gclip = Rect::new(gx as i32, r.y, (shown as f32 * lw) as i32 + 2, r.h);
        let old = cv.set_clip(cv.clip().intersect(gclip));
        let x = |lane: u16| gx + lane as f32 * lw + lw / 2.0;
        let (top, mid, bot) = (r.y as f32, r.y as f32 + r.h as f32 / 2.0, r.bottom() as f32);
        let width = self.pf(2.0);
        for s in &g.segs {
            let color = lane(s.color);
            let (y0, y1) = if s.top { (top, mid) } else { (mid, bot) };
            let (x0, x1) = (x(s.from), x(s.to));
            if s.from == s.to {
                cv.stroke(&[(x0, y0), (x1, y1)], width, color);
            } else {
                let k = (y1 - y0) * 0.55;
                cv.bezier((x0, y0), (x0, y0 + k), (x1, y1 - k), (x1, y1), width, color);
            }
        }
        let cx = x(g.node);
        let color = lane(g.color);
        let is_head = self.refs.head.is_some_and(|h| h == self.commits[row].id);
        cv.fill_circle(cx, mid, self.pf(6.5), bg);
        if is_head {
            cv.fill_circle(cx, mid, self.pf(5.5), ACCENT);
            cv.fill_circle(cx, mid, self.pf(2.2), bg);
        } else if g.is_merge {
            cv.ring(cx, mid, self.pf(4.6), self.pf(2.0), color);
        } else {
            cv.fill_circle(cx, mid, self.pf(4.6), color);
        }
        cv.set_clip(old);
    }

    fn header_h(&self) -> i32 {
        self.px(190.0)
    }

    fn file_rows_visible(&self) -> usize {
        match self.layout {
            DetailsLayout::Beside => 8,
            DetailsLayout::Below => {
                let free = self.lay.info.h - self.header_h() - self.px(34.0) - self.px(8.0);
                (free / self.px(26.0).max(1)).max(1) as usize
            }
        }
    }

    fn files_rect(&self) -> Rect {
        let d = self.lay.info;
        let n = self.details.as_ref().map_or(4, |x| x.files.len().clamp(1, self.file_rows_visible()));
        Rect::new(d.x, d.y + self.header_h() + self.px(34.0), d.w, n as i32 * self.px(26.0))
    }

    fn diff_rect(&self) -> Rect {
        let d = self.lay.details;
        if self.layout == DetailsLayout::Below {
            let i = self.lay.info;
            return Rect::new(i.right(), d.y, (d.right() - i.right()).max(0), d.h);
        }
        let f = self.files_rect();
        let y = f.bottom() + self.px(10.0);
        Rect::new(d.x, y, d.w, (d.bottom() - y).max(0))
    }

    fn paint_details(&mut self, cv: &mut Canvas) {
        let d = self.lay.details;
        cv.fill(d, BG_PANEL);
        match self.layout {
            DetailsLayout::Beside => cv.fill(Rect::new(d.x, d.y, 1, d.h), BORDER),
            DetailsLayout::Below => cv.fill(Rect::new(d.x, d.y, d.w, 1), BORDER),
        }
        let d = self.lay.info;
        let Some(c) = self.commits.get(self.sel).cloned() else {
            let msg = if let Some(e) = &self.error { e.clone() } else { "Reading history…".into() };
            self.te.draw(
                cv,
                self.f.ui,
                self.pf(13.0),
                d.x as f32 + self.pf(24.0),
                d.y as f32 + self.pf(40.0),
                &msg,
                TEXT_3,
                d.w as f32,
            );
            return;
        };
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let pad = self.pf(22.0);
        let x0 = d.x as f32 + pad;
        let maxw = d.w as f32 - 2.0 * pad;
        let mut y = d.y as f32 + self.pf(34.0);
        // Subject (from list data: instant) and body (from details).
        let subject = c.subject.to_string();
        let (s16, s13) = (self.pf(16.0), self.pf(13.0));
        for line in wrap(&mut self.te, semi, s16, &subject, maxw).into_iter().take(2) {
            self.te.draw(cv, semi, self.pf(16.0), x0, y, &line, TEXT, maxw);
            y += self.pf(22.0);
        }
        let body: Vec<String> = match &self.details {
            Some(det) => {
                det.message.lines().skip(1).skip_while(|l| l.trim().is_empty()).map(|s| s.to_string()).collect()
            }
            None => Vec::new(),
        };
        // The body gets the lines that fit above the author row; the last
        // visible line ends in an ellipsis when more text follows.
        let meta_y = d.y as f32 + self.pf(140.0);
        let lh = self.pf(19.0);
        let fit = ((meta_y - self.pf(30.0) - y) / lh).floor().max(0.0) as usize;
        let lines: Vec<String> = body.iter().flat_map(|p| wrap(&mut self.te, ui, s13, p, maxw)).collect();
        for (k, line) in lines.iter().take(fit).enumerate() {
            let more = k + 1 == fit && lines.len() > fit;
            let text = if more { format!("{line} …") } else { line.clone() };
            self.te.draw(cv, ui, s13, x0, y + self.pf(2.0), &text, TEXT_2, maxw);
            y += lh;
        }
        // Meta block.
        let av = self.pf(15.0);
        cv.fill_circle(x0 + av, meta_y, av, avatar(&c.email));
        let ini = fmt::initials(&c.author);
        let iw = self.te.measure(semi, self.pf(11.5), &ini);
        self.te.draw(cv, semi, self.pf(11.5), x0 + av - iw / 2.0, meta_y + self.pf(4.0), &ini, 0xffffff, av * 2.0);
        let mx = x0 + av * 2.0 + self.pf(12.0);
        let end = self.te.draw(cv, semi, self.pf(13.0), mx, meta_y - self.pf(3.0), &c.author, TEXT, maxw * 0.5);
        self.te.draw(
            cv,
            ui,
            self.pf(12.0),
            end + self.pf(8.0),
            meta_y - self.pf(3.0),
            &c.email,
            TEXT_3,
            x0 + maxw - end - self.pf(8.0),
        );
        let when = format!("{}  ·  {}", fmt::date_time(c.time, self.tz), fmt::relative(c.time, self.now, self.tz));
        self.te.draw(cv, ui, self.pf(12.0), mx, meta_y + self.pf(14.0), &when, TEXT_3, maxw);
        let sha_y = meta_y + self.pf(40.0);
        let mut sx = self.te.draw(cv, mono, self.pf(12.0), x0, sha_y, &c.id.short(12), TEXT_2, maxw);
        for p in c.parents.iter().take(2) {
            sx = self.te.draw(cv, ui, self.pf(12.0), sx + self.pf(14.0), sha_y, "parent", TEXT_4, maxw);
            sx = self.te.draw(cv, mono, self.pf(12.0), sx + self.pf(6.0), sha_y, &p.short(8), TEXT_3, maxw);
        }
        // Files.
        let fh_y = d.y + self.header_h();
        cv.fill(Rect::new(d.x + 1, fh_y, d.w - 1, 1), BORDER_SOFT);
        let fbase = fh_y as f32 + self.pf(23.0);
        let focus_files = self.focus == Focus::Files;
        match &self.details {
            None => {
                self.te.draw(cv, semi, self.pf(11.5), x0, fbase, "CHANGED FILES", TEXT_3, maxw);
                let fr = self.files_rect();
                for k in 0..3 {
                    let yy = fr.y + k * self.px(26.0) + self.px(9.0);
                    cv.fill_round(
                        Rect::new(x0 as i32, yy, self.px(180.0 - k as f32 * 30.0), self.px(8.0)),
                        self.pf(4.0),
                        BG_HOVER,
                    );
                }
            }
            Some(det) => {
                let det = det.clone();
                let (adds, dels) = det.files.iter().fold((0u64, 0u64), |a, f| {
                    (a.0 + f.added.unwrap_or(0) as u64, a.1 + f.removed.unwrap_or(0) as u64)
                });
                let n = det.files.len();
                let title = format!("{} {}", fmt::num(n), if n == 1 { "FILE" } else { "FILES" });
                let e = self.te.draw(
                    cv,
                    semi,
                    self.pf(11.5),
                    x0,
                    fbase,
                    &title,
                    if focus_files { ACCENT } else { TEXT_3 },
                    maxw,
                );
                let e = self.te.draw(
                    cv,
                    semi,
                    self.pf(11.5),
                    e + self.pf(10.0),
                    fbase,
                    &format!("+{}", fmt::num(adds as usize)),
                    ADD,
                    maxw,
                );
                self.te.draw(
                    cv,
                    semi,
                    self.pf(11.5),
                    e + self.pf(6.0),
                    fbase,
                    &format!("−{}", fmt::num(dels as usize)),
                    DEL,
                    maxw,
                );
                let fr = self.files_rect();
                let rh = self.px(26.0);
                for (k, f) in det.files.iter().enumerate().skip(self.file_top).take(self.file_rows_visible()) {
                    let ry = fr.y + (k - self.file_top) as i32 * rh;
                    let row = Rect::new(d.x + self.px(10.0), ry, d.w - self.px(20.0), rh);
                    if k == self.sel_file {
                        cv.fill_round(row, self.pf(6.0), if focus_files { BG_SELECT } else { BG_SELECT_DIM });
                    }
                    let cy = ry as f32 + rh as f32 / 2.0;
                    let base = (cy + self.pf(4.5)).round();
                    let (sc, sl) = match f.status {
                        b'A' => (ADD, "A"),
                        b'D' => (DEL, "D"),
                        b'R' => (REN, "R"),
                        b'C' => (REN, "C"),
                        _ => (MOD, "M"),
                    };
                    let badge = Rect::new(x0 as i32, (cy - self.pf(8.0)) as i32, self.px(16.0), self.px(16.0));
                    cv.fill_round(badge, self.pf(4.0), crate::ui::tint(sc));
                    let lw = self.te.measure(semi, self.pf(10.5), sl);
                    self.te.draw(
                        cv,
                        semi,
                        self.pf(10.5),
                        badge.x as f32 + (badge.w as f32 - lw) / 2.0,
                        base - self.pf(0.5),
                        sl,
                        sc,
                        20.0,
                    );
                    let (dir, name) = match f.path.rfind('/') {
                        Some(i) => (&f.path[..=i], &f.path[i + 1..]),
                        None => ("", &*f.path),
                    };
                    let stat = match (f.added, f.removed) {
                        (Some(a), Some(r)) => format!("+{a} −{r}"),
                        _ => "bin".into(),
                    };
                    let stw = self.te.measure(ui, self.pf(11.5), &stat);
                    let tx = x0 + self.pf(26.0);
                    let avail = row.right() as f32 - tx - stw - self.pf(18.0);
                    let e = self.te.draw(cv, ui, self.pf(13.0), tx, base, name, TEXT, avail);
                    if !dir.is_empty() {
                        self.te.draw(
                            cv,
                            ui,
                            self.pf(12.0),
                            e + self.pf(8.0),
                            base,
                            dir.trim_end_matches('/'),
                            TEXT_4,
                            avail - (e - tx) - self.pf(8.0),
                        );
                    }
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(11.5),
                        row.right() as f32 - stw - self.pf(10.0),
                        base,
                        &stat,
                        TEXT_3,
                        stw + 2.0,
                    );
                }
                if n > self.file_rows_visible() {
                    let more = format!("{} more · Tab to browse", n - self.file_rows_visible());
                    let mw = self.te.measure(ui, self.pf(11.0), &more);
                    self.te.draw(cv, ui, self.pf(11.0), d.right() as f32 - mw - pad, fbase, &more, TEXT_4, mw + 2.0);
                }
            }
        }
        self.paint_diff(cv);
    }

    fn paint_diff(&mut self, cv: &mut Canvas) {
        let r = self.diff_rect();
        if cv.clip().intersect(r).is_empty() || r.h <= 0 {
            return;
        }
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        cv.fill(Rect::new(r.x + 1, r.y, r.w - 1, r.h), BG);
        let bar = Rect::new(r.x + 1, r.y, r.w - 1, self.px(32.0));
        cv.fill(bar, BG_BAR);
        cv.fill(Rect::new(bar.x, bar.bottom() - 1, bar.w, 1), BORDER_SOFT);
        let base = (bar.y as f32 + bar.h as f32 / 2.0 + self.pf(4.5)).round();
        let pad = self.pf(16.0);
        let Some(diff) = self.diff.clone() else {
            let label = match self.details.as_ref().and_then(|d| d.files.get(self.sel_file)) {
                Some(f) => f.path.to_string(),
                None if self.details.is_some() => "No file changes".into(),
                None => String::new(),
            };
            self.te.draw(cv, semi, self.pf(12.5), r.x as f32 + pad, base, &label, TEXT_2, r.w as f32 - 2.0 * pad);
            return;
        };
        let focus = self.focus == Focus::Diff;
        self.te.draw(
            cv,
            semi,
            self.pf(12.5),
            r.x as f32 + pad,
            base,
            &diff.path,
            if focus { ACCENT } else { TEXT },
            r.w as f32 - self.pf(160.0),
        );
        let hint = if diff.truncated { "first 20,000 lines" } else { "unified · first parent" };
        let hw = self.te.measure(ui, self.pf(11.0), hint);
        self.te.draw(cv, ui, self.pf(11.0), r.right() as f32 - hw - pad, base, hint, TEXT_4, hw + 2.0);
        let lh = self.px(20.0);
        let body = Rect::new(r.x + 1, bar.bottom(), r.w - 1, r.bottom() - bar.bottom());
        if diff.binary {
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                r.x as f32 + pad,
                body.y as f32 + self.pf(26.0),
                "Binary file: no text diff",
                TEXT_3,
                r.w as f32,
            );
            return;
        }
        let num_w = self.pf(38.0);
        let gut = Rect::new(body.x, body.y, (num_w * 2.0 + self.pf(8.0)) as i32, body.h);
        cv.fill(gut, BG_SIDEBAR);
        let fs = self.pf(12.0);
        let old_clip = cv.clip();
        let rows = (body.h / lh + 1) as usize;
        for (k, line) in diff.lines.iter().skip(self.diff_top).take(rows).enumerate() {
            let y = body.y + k as i32 * lh;
            let row = Rect::new(body.x, y, body.w, lh);
            if old_clip.intersect(row).is_empty() {
                continue;
            }
            let lb = (y as f32 + lh as f32 / 2.0 + self.pf(4.0)).round();
            let (bgc, fg, mark) = match line.kind {
                LineKind::Add => (ADD_BG, ADD_TEXT, "+"),
                LineKind::Del => (DEL_BG, DEL_TEXT, "−"),
                LineKind::Hunk => (HUNK_BG, HUNK_TEXT, ""),
                LineKind::Meta => (BG, TEXT_4, ""),
                LineKind::Context => (BG, TEXT_2, ""),
            };
            if bgc != BG {
                cv.fill(Rect::new(gut.right(), y, body.w - gut.w, lh), bgc);
                let gbg = match line.kind {
                    LineKind::Add => 0x0f1d16,
                    LineKind::Del => 0x221114,
                    _ => BG_SIDEBAR,
                };
                cv.fill(Rect::new(gut.x, y, gut.w, lh), gbg);
            }
            if line.kind == LineKind::Hunk {
                self.te.draw(
                    cv,
                    mono,
                    self.pf(11.5),
                    gut.right() as f32 + self.pf(10.0),
                    lb,
                    &line.text,
                    fg,
                    (body.right() - gut.right()) as f32 - self.pf(20.0),
                );
                continue;
            }
            let numc = match line.kind {
                LineKind::Add => 0x3a7a5a,
                LineKind::Del => 0x8a4a4f,
                _ => TEXT_4,
            };
            if line.old > 0 {
                let s = line.old.to_string();
                let w = self.te.measure(mono, self.pf(11.0), &s);
                self.te.draw(cv, mono, self.pf(11.0), body.x as f32 + num_w - w, lb, &s, numc, num_w);
            }
            if line.new > 0 {
                let s = line.new.to_string();
                let w = self.te.measure(mono, self.pf(11.0), &s);
                self.te.draw(cv, mono, self.pf(11.0), body.x as f32 + num_w * 2.0 - w, lb, &s, numc, num_w);
            }
            let tx = gut.right() as f32 + self.pf(8.0);
            if !mark.is_empty() {
                self.te.draw(cv, mono, fs, tx, lb, mark, if line.kind == LineKind::Add { ADD } else { DEL }, 20.0);
            }
            let text = line.text.replace('\t', "    ");
            self.te.draw(cv, mono, fs, tx + self.pf(14.0), lb, &text, fg, (body.right() as f32) - tx - self.pf(24.0));
        }
        let _ = ui;
    }

    fn paint_status(&mut self, cv: &mut Canvas) {
        let r = self.lay.status;
        cv.fill(r, BG_SIDEBAR);
        cv.fill(Rect::new(r.x, r.y, r.w, 1), BORDER);
        let ui = self.f.ui;
        let base = (r.y as f32 + r.h as f32 / 2.0 + self.pf(4.0)).round();
        let cy = r.y as f32 + r.h as f32 / 2.0;
        let mut x = self.pf(14.0);
        let (dot, msg) = if let Some(e) = &self.error {
            (DEL, e.clone())
        } else if let Some(n) = &self.notice {
            (MOD, n.clone())
        } else if !self.log_done {
            (MOD, format!("Reading history · {} commits", fmt::num(self.commits.len())))
        } else {
            (ADD, "Ready".to_string())
        };
        cv.fill_circle(x + self.pf(3.0), cy, self.pf(3.5), dot);
        x += self.pf(14.0);
        x = self.te.draw(cv, ui, self.pf(12.0), x, base, &msg, TEXT_2, self.pf(520.0));
        if self.log_done && self.error.is_none() {
            let mut facts = format!("{} commits in {:.0} ms", fmt::num(self.commits.len()), self.log_ms);
            if let Some(fb) = self.first_batch_ms {
                facts.push_str(&format!(" · first rows {fb:.0} ms"));
            }
            if let Some(ff) = self.stats.first_frame_ms {
                facts.push_str(&format!(" · first frame {ff:.0} ms"));
            }
            self.te.draw(cv, ui, self.pf(12.0), x + self.pf(16.0), base, &facts, TEXT_3, self.pf(560.0));
        }
        let right = format!(
            "{}  ·  CPU renderer, damage-only  ·  last paint {:.1} ms",
            if self.remote { "Remote session" } else { "Local session" },
            self.stats.last_paint_ms
        );
        let rw = self.te.measure(ui, self.pf(12.0), &right);
        self.te.draw(cv, ui, self.pf(12.0), r.right() as f32 - rw - self.pf(14.0), base, &right, TEXT_4, rw + 2.0);
    }

    fn paint_palette(&mut self, cv: &mut Canvas) {
        let Some(p) = &self.palette else { return };
        let full = self.palette_rect(10);
        let shown = p.items.len().max(1);
        let r = self.palette_rect(shown);
        let clip = cv.clip();
        // Below the shrunken palette: repaint what the full rect covered.
        if r.h < full.h {
            let below = Rect::new(full.x, r.bottom(), full.w, full.bottom() - r.bottom());
            if !clip.intersect(below).is_empty() {
                let pal = self.palette.take();
                let old = cv.set_clip(clip.intersect(below));
                self.paint_scene(cv);
                cv.set_clip(old);
                self.palette = pal;
            }
        }
        let Some(p) = &self.palette else { return };
        let (query, sel) = (p.query.clone(), p.sel);
        let items: Vec<(String, String, RefKind, bool)> = p
            .items
            .iter()
            .map(|i| (i.label.clone(), i.hint.clone(), i.kind, matches!(i.action, PAction::Command(_))))
            .collect();
        let old = cv.set_clip(clip.intersect(r));
        cv.round_border(r, self.pf(12.0), 0x343a45, BG_INPUT);
        let (ui, semi) = (self.f.ui, self.f.semi);
        let input = Rect::new(r.x, r.y, r.w, self.px(52.0));
        let base = (input.y as f32 + input.h as f32 / 2.0 + self.pf(5.5)).round();
        let ix = r.x as f32 + self.pf(20.0);
        self.chevrons(cv, ix, input.y as f32 + input.h as f32 / 2.0, self.pf(9.0), ACCENT);
        let qx = ix + self.pf(24.0);
        let end = if query.is_empty() {
            self.te.draw(
                cv,
                ui,
                self.pf(15.0),
                qx,
                base,
                "Type a branch, tag, SHA or command",
                TEXT_3,
                r.w as f32 - self.pf(80.0),
            );
            qx
        } else {
            self.te.draw(cv, ui, self.pf(15.0), qx, base, &query, TEXT, r.w as f32 - self.pf(80.0))
        };
        cv.fill(Rect::new(end as i32 + 1, input.y + self.px(15.0), self.px(2.0), self.px(22.0)), ACCENT);
        cv.fill(Rect::new(r.x + 1, input.bottom() - 1, r.w - 2, 1), BORDER);
        if items.is_empty() {
            self.te.draw(
                cv,
                ui,
                self.pf(13.0),
                qx,
                input.bottom() as f32 + self.pf(22.0),
                "No matches",
                TEXT_3,
                r.w as f32,
            );
        }
        for (k, (label, hint, kind, is_cmd)) in items.iter().enumerate() {
            let y = input.bottom() + self.px(5.0) + k as i32 * self.px(34.0);
            let row = Rect::new(r.x + self.px(6.0), y, r.w - self.px(12.0), self.px(34.0));
            if k == sel {
                cv.fill_round(row, self.pf(7.0), BG_SELECT);
            }
            let cy = y as f32 + self.pf(17.0);
            let b = (cy + self.pf(4.5)).round();
            let icx = ix + self.pf(4.0);
            let color = if *is_cmd {
                TEXT_3
            } else {
                match kind {
                    RefKind::Local => LOCAL_FG,
                    RefKind::Remote => REMOTE_FG,
                    RefKind::Tag => TAG_FG,
                    _ => ACCENT,
                }
            };
            if *is_cmd {
                if label.starts_with("AI") {
                    self.sparkle(cv, icx, cy, self.pf(5.0), ACCENT);
                } else {
                    self.chevrons(cv, icx - self.pf(4.0), cy, self.pf(7.0), color);
                }
            } else if *kind == RefKind::Tag {
                let s = self.pf(4.0);
                cv.stroke(
                    &[(icx - s, cy), (icx, cy - s), (icx + s, cy), (icx, cy + s), (icx - s, cy)],
                    self.pf(1.5),
                    color,
                );
            } else {
                self.branch_icon(cv, icx, cy, color);
            }
            let font = if k == sel { semi } else { ui };
            self.te.draw(cv, font, self.pf(13.5), qx, b, label, TEXT, r.w as f32 - self.pf(220.0));
            if !hint.is_empty() {
                let hw = self.te.measure(ui, self.pf(11.5), hint);
                if *is_cmd {
                    self.keycap(cv, row.right() as f32 - self.pf(10.0), cy, hint, false);
                } else {
                    self.te.draw(
                        cv,
                        ui,
                        self.pf(11.5),
                        row.right() as f32 - hw - self.pf(14.0),
                        b,
                        hint,
                        TEXT_3,
                        hw + 2.0,
                    );
                }
            }
        }
        cv.set_clip(old);
    }
}

pub fn tint(c: u32) -> u32 {
    ffwd_render::mix(BG_PANEL, c, 46)
}

fn wrap(te: &mut TextEngine, f: FontId, size: f32, text: &str, maxw: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if te.measure(f, size, &cand) > maxw && !line.is_empty() {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = cand;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// Subsequence match with a bonus for prefix and word starts. `None` if the
/// query does not match.
fn fuzzy(q: &str, s: &str) -> Option<i32> {
    if q.is_empty() {
        return Some(0);
    }
    let lower = s.to_lowercase();
    if lower.starts_with(q) {
        return Some(60 - s.len() as i32 / 4);
    }
    if lower.contains(q) {
        return Some(40 - s.len() as i32 / 4);
    }
    let mut it = lower.chars();
    let mut score = 0;
    for qc in q.chars() {
        let mut gap = 0;
        loop {
            match it.next() {
                Some(c) if c == qc => break,
                Some(_) => gap += 1,
                None => return None,
            }
        }
        score -= gap.min(5);
    }
    Some(20 + score)
}

/// Merges overlapping or touching rectangles and drops empty ones, so a
/// present never sends the same pixels twice.
fn merge_rects(mut v: Vec<Rect>) -> Vec<Rect> {
    v.retain(|r| !r.is_empty());
    let mut changed = true;
    while changed {
        changed = false;
        'outer: for i in 0..v.len() {
            for j in i + 1..v.len() {
                let (a, b) = (v[i], v[j]);
                let contains =
                    |o: Rect, i: Rect| i.x >= o.x && i.y >= o.y && i.right() <= o.right() && i.bottom() <= o.bottom();
                let same_cols = a.x == b.x && a.w == b.w && a.y <= b.bottom() && b.y <= a.bottom();
                if contains(a, b) || contains(b, a) || same_cols {
                    let x = a.x.min(b.x);
                    let y = a.y.min(b.y);
                    v[i] = Rect::new(x, y, a.right().max(b.right()) - x, a.bottom().max(b.bottom()) - y);
                    v.swap_remove(j);
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
    v
}
