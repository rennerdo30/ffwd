//! The ffwd main view: state, layout, input and painting.
//!
//! Painting is clip-driven: every damage rectangle is painted by running
//! the scene with the canvas clipped to it, and each region skips work
//! outside the clip. Damage is recorded where state changes, so a key
//! press repaints a few rows, never the window.

use crate::fmt;
use crate::text_field::TextField;
use crate::theme::*;
use crate::work::{DetailWorker, Msg, Sink};
use ffwd_core::diff::{Details, FileDiff, LineKind};
use ffwd_core::ops::Op;
use ffwd_core::refs::{Ref, RefKind, Refs, StatusSummary};
use std::collections::HashSet;

#[path = "ui_sidebar.rs"]
mod sidebar;
use sidebar::SideItem;
#[path = "ui_activity.rs"]
mod activity_view;
#[path = "ui_agents.rs"]
mod agents_view;
#[path = "ui_changes.rs"]
mod changes_view;
#[path = "ui_conflict.rs"]
mod conflict_view;
#[path = "ui_difffind.rs"]
mod diff_find;
#[path = "ui_diff.rs"]
mod diff_view;
pub use activity_view::Entry as ActivityEntry;
#[path = "ui_ai.rs"]
mod ai_view;
pub use ai_view::Prepared as AiPrepared;
#[path = "ui_files.rs"]
mod files_view;
#[path = "ui_rebase.rs"]
mod rebase_view;
#[path = "ui_reflog.rs"]
mod reflog_view;
pub use files_view::{Loaded as FileLoaded, TreeResult as FilesTreeResult};
#[path = "ui_icons.rs"]
mod icons;
#[path = "ui_menu.rs"]
mod menu;
use icons::Glyph;
#[path = "ui_search.rs"]
mod search_view;
use menu::{Dialog, DialogKind, Menu, NameAct, PatchSrc};
#[path = "ui_hover.rs"]
mod hover;
pub use hover::Cursor;
use hover::Hot;
#[path = "ui_settings.rs"]
mod settings_view;
use agents_view::{AgentRun, AgentView};
use ffwd_core::{Commit, Git, Graph, Oid};
use ffwd_render::{Canvas, FontId, Rect, TextEngine};
use settings_view::SettingsView;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// See `load_fonts`.
pub const TYPE_SCALE: f32 = 1.08;

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
    // The type scale: the sizes in the code are design sizes; on screen
    // they are 8 % larger, so the smallest text is about 12 px (FF-30).
    te.set_size_boost(TYPE_SCALE);
    Some(Fonts { ui, semi, mono })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Side,
    /// Typing into the sidebar's filter field.
    SideFilter,
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
    /// Ctrl (Cmd on macOS) with a lowercase letter or digit.
    Ctrl(char),
    CtrlShift(char),
    CtrlBackspace,
    Left,
    Right,
    Delete,
    NextTab,
    PrevTab,
    /// Ctrl+Enter (Cmd+Enter on macOS): commit.
    CtrlEnter,
}

#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub top: Rect,
    pub tabs: Rect,
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

struct PItem {
    label: String,
    hint: String,
    kind: RefKind,
    action: PAction,
}

#[derive(Clone)]
enum PAction {
    Goto(Oid),
    /// A branch or tag: the history jumps to it and the sidebar selects it.
    GotoRef(Oid, String, RefKind),
    Command(&'static str),
    InstallUpdate,
    OpenRepo(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PMode {
    /// Branches, tags, SHAs and commands.
    Jump,
    /// Repositories to open in a tab.
    Repos,
}

pub struct Palette {
    pub input: TextField,
    items: Vec<PItem>,
    sel: usize,
    mode: PMode,
}

const CMD_FETCH: &str = "Fetch all remotes";
const CMD_PULL: &str = "Pull";
const CMD_PUSH: &str = "Push";
const CMD_OPEN: &str = "Open repository…";
const CMD_CLOSE: &str = "Close tab";
const CMD_REOPEN: &str = "Reopen closed tab";
const CMD_UPDATES: &str = "Check for updates";
const CMD_SETTINGS: &str = "Settings…";
const CMD_AGENTS: &str = "Agents (experimental)…";
const CMD_ACTIONS: &str = "Commit actions…";
const CMD_SEARCH: &str = "Search commits…";
const CMD_BRANCH: &str = "Create branch here";
const CMD_TAG: &str = "Create tag here";
const CMD_PICK: &str = "Cherry-pick this commit";
const CMD_REVERT: &str = "Revert this commit";
const CMD_STASH: &str = "Stash changes";
const CMD_PATCH_UNSTAGED: &str = "Copy unstaged changes as patch";
const CMD_PATCH_STAGED: &str = "Copy staged changes as patch";
const CMD_APPLY_CLIP: &str = "Apply patch from the clipboard";
const CMD_APPLY_FILE: &str = "Apply patch from a file…";
const CMD_CREATE_PR: &str = "Create pull request on GitHub";
const CMD_REFLOG: &str = "Reflog (recover lost commits)";
const CMD_AI_COMMIT: &str = "AI: Draft a commit message";
const CMD_CLONE: &str = "Clone repository…";
const CMD_INIT: &str = "Create a new repository…";
const CMD_CANCEL_CLONE: &str = "Cancel the running clone";
const CMD_SCOPE: &str = "Show only the current branch (or all again)";
const CMD_ADD_SUBMODULE: &str = "Add submodule…";
const CMD_ACTIVITY: &str = "Activity log";
const CMD_REBASE_I: &str = "Interactive rebase from here";
const CMD_ADD_REMOTE: &str = "Add remote…";
const CMD_CANCEL_OP: &str = "Cancel the running fetch, pull or push";

/// The toolbar action of a fetch, pull or push button.
fn op_action(op: Op) -> TopAction {
    match op.button() {
        Op::Fetch => TopAction::Fetch,
        Op::Push => TopAction::Push,
        _ => TopAction::Pull,
    }
}

/// Commands that work today; the rest of COMMANDS are listed so the
/// palette shows the plan, and say so when chosen.
const WORKING: &[&str] = &[
    CMD_SEARCH,
    CMD_ACTIONS,
    CMD_BRANCH,
    CMD_TAG,
    CMD_PICK,
    CMD_REVERT,
    CMD_STASH,
    CMD_PATCH_UNSTAGED,
    CMD_PATCH_STAGED,
    CMD_APPLY_CLIP,
    CMD_APPLY_FILE,
    CMD_CREATE_PR,
    CMD_REFLOG,
    CMD_AI_COMMIT,
    CMD_CLONE,
    CMD_INIT,
    CMD_CANCEL_CLONE,
    CMD_SCOPE,
    CMD_ADD_SUBMODULE,
    CMD_ACTIVITY,
    CMD_REBASE_I,
    CMD_ADD_REMOTE,
    CMD_CANCEL_OP,
    CMD_SETTINGS,
    CMD_AGENTS,
    CMD_FETCH,
    CMD_PULL,
    CMD_PUSH,
    CMD_OPEN,
    CMD_CLOSE,
    CMD_REOPEN,
    CMD_UPDATES,
    LAYOUT_BELOW,
    LAYOUT_BESIDE,
];

const COMMANDS: &[(&str, &str)] = &[
    (CMD_FETCH, "Ctrl Shift F"),
    (CMD_PULL, "Ctrl Shift L"),
    (CMD_PUSH, "Ctrl Shift K"),
    (CMD_OPEN, "Ctrl O"),
    (CMD_CLOSE, "Ctrl W"),
    (CMD_REOPEN, "Ctrl Shift T"),
    (CMD_UPDATES, ""),
    (CMD_SETTINGS, "Ctrl ,"),
    (CMD_AGENTS, "Ctrl Shift A"),
    (CMD_ACTIONS, "Ctrl ."),
    (CMD_SEARCH, "Ctrl F"),
    ("Commit staged changes", "Ctrl Enter"),
    (CMD_BRANCH, "Ctrl B"),
    (CMD_TAG, "Ctrl Shift G"),
    (CMD_PICK, ""),
    (CMD_REVERT, ""),
    (CMD_REBASE_I, "Ctrl Shift R"),
    (CMD_STASH, "Ctrl Shift S"),
    (CMD_PATCH_UNSTAGED, ""),
    (CMD_PATCH_STAGED, ""),
    (CMD_APPLY_CLIP, ""),
    (CMD_APPLY_FILE, ""),
    (CMD_CREATE_PR, ""),
    (CMD_REFLOG, ""),
    (CMD_AI_COMMIT, ""),
    (CMD_CLONE, ""),
    (CMD_INIT, ""),
    (CMD_CANCEL_CLONE, ""),
    (CMD_SCOPE, ""),
    (CMD_ADD_SUBMODULE, ""),
    (CMD_ACTIVITY, ""),
    (CMD_ADD_REMOTE, ""),
    (CMD_CANCEL_OP, ""),
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

/// Everything that belongs to one open repository. The active tab's state
/// lives in `Ui::r`; inactive tabs keep theirs in `Tab::state`.
#[derive(Default)]
pub struct RepoState {
    /// A commit marked for comparison, and a comparison shown (from, to).
    pub compare_mark: Option<Oid>,
    pub compare: Option<(Oid, Oid)>,
    /// Where each file's diff was scrolled to, by commit and path (FF-41).
    pub diff_scroll: HashMap<(Oid, Box<str>), usize>,
    pub repo_name: String,
    pub git: Option<Git>,
    pub error: Option<String>,
    pub tz: i64,
    pub commits: Vec<Commit>,
    pub graph: Graph,
    by_id: HashMap<Oid, u32>,
    pub refs: Refs,
    chips: HashMap<Oid, Vec<u32>>,
    side: Vec<SideItem>,
    /// The sidebar's own selection (an item key), separate from the history.
    side_sel: String,
    /// Refs and stashes picked for a batch (FF-36): Ctrl+click, Shift+click
    /// or the menu's "Select for several". Item keys.
    pub(super) side_multi: std::collections::BTreeSet<String>,
    side_top: usize,
    pub status: Option<StatusSummary>,
    pub log_done: bool,
    pub log_ms: f64,
    pub first_batch_ms: Option<f64>,
    pub sel: usize,
    pub top: usize,
    seq: u64,
    pub details: Option<Box<Details>>,
    cache: HashMap<Oid, Box<Details>>,
    pub sel_file: usize,
    file_top: usize,
    pub diff: Option<Box<FileDiff>>,
    /// The selected image file's old and new version, for diff `seq`.
    #[allow(clippy::type_complexity)]
    pub diff_images:
        Option<(u64, Option<Result<crate::image::Img, String>>, Option<Result<crate::image::Img, String>>)>,
    diff_top: usize,
    worker: Option<DetailWorker>,
    /// A fetch, pull or push running for this repository.
    op: Option<Op>,
    /// Set to cancel the running fetch, pull or push.
    op_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// After a reload, the commit to select again once it streams in.
    reselect: Option<Oid>,
    /// The Local Changes view replaces history and details (FF-35).
    pub changes_view: bool,
    pub changes: changes_view::Changes,
    /// The Files view: folder tree, content, blame, file history (FF-51, FF-43).
    pub files_view: bool,
    pub files: files_view::Files,
    /// The activity log view (FF-48); the log itself is app-wide.
    pub activity_view: bool,
    /// The reflog view (FF-43).
    pub reflog_view: bool,
    pub reflog: reflog_view::Reflog,
    /// The interactive rebase plan, shown over the other views (FF-37).
    pub rebase: Option<rebase_view::RebaseState>,
    /// The find bar over the commit list (FF-34).
    pub search: Option<search_view::Search>,
    /// Open pull requests (GitHub through gh); `Err` holds why not (or
    /// "loading").
    pub prs: Option<Result<Vec<ffwd_core::hosting::PullRequest>, String>>,
    /// CI status by commit, with the deciding run's URL (FF-24).
    pub ci: HashMap<Oid, (ffwd_core::hosting::Ci, String)>,
    /// Commits to push and to pull (marked in the list).
    pub ahead_ids: HashSet<Oid>,
    pub behind_ids: HashSet<Oid>,
}

/// One repository tab.
pub struct Tab {
    /// Messages are tagged with this id; a reload gets a new one, so
    /// results of the old loaders are dropped.
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    /// `None` while the tab is active (its state is in `Ui::r`).
    state: Option<RepoState>,
    /// Messages that arrived while the tab was in the background.
    pending: Vec<Msg>,
    started: bool,
    /// Changed in the background (an agent finished or merged): reload
    /// when activated.
    stale: bool,
    /// History of the current branch only (FF-33).
    head_only: bool,
    /// A name the user gave the tab (FF-44); `name` stays the folder's.
    label: Option<String>,
    /// One of [`TAB_COLORS`].
    color: Option<u8>,
}

/// Tab colours (FF-44): red, amber, green, blue, purple.
pub(super) const TAB_COLORS: [(&str, u32); 5] =
    [("Red", DEL), ("Amber", MOD), ("Green", ADD), ("Blue", REMOTE_FG), ("Purple", STASH_FG)];

impl Tab {
    /// What the tab shows.
    fn title(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

/// Creates the message sink for a tab id (the window wraps messages into
/// event-loop events, snapshots into a channel).
pub type SinkFactory = Arc<dyn Fn(u64) -> Sink + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TopAction {
    Repo,
    Branch,
    Fetch,
    Pull,
    Push,
    Stash,
    Ai,
    Settings,
}

/// A button next to the status bar's notice: the click twin of a
/// shortcut (FF-57).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoticeAct {
    InstallUpdate,
    Discard,
    Undo,
}

impl NoticeAct {
    fn label(self) -> (&'static str, Glyph) {
        match self {
            NoticeAct::InstallUpdate => ("Install update", Glyph::Update),
            NoticeAct::Discard => ("Discard", Glyph::Trash),
            NoticeAct::Undo => ("Undo", Glyph::Undo),
        }
    }
}

pub struct Ui {
    pub s: f32,
    pub w: usize,
    pub h: usize,
    pub lay: Layout,
    pub te: TextEngine,
    pub f: Fonts,
    now: i64,
    /// The active repository.
    pub r: RepoState,
    pub tabs: Vec<Tab>,
    pub cur_tab: usize,
    next_tab_id: u64,
    closed_tabs: Vec<PathBuf>,
    tab_hover: Option<usize>,
    side_collapsed: HashSet<String>,
    side_filter: String,
    side_hover: Option<usize>,
    sinks: SinkFactory,
    /// Recent repositories first, then ones found next to open ones.
    known_repos: Vec<PathBuf>,
    pub state: crate::settings::Settings,
    pub title_dirty: bool,
    pub focus: Focus,
    pub palette: Option<Palette>,
    settings_view: Option<SettingsView>,
    /// Experimental agent runs of every tab, and the panel when open.
    agents: Vec<AgentRun>,
    agent_view: Option<AgentView>,
    /// Set to cancel the running clone (FF-44).
    clone_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// The activity log (FF-48), across tabs.
    activity: activity_view::Activity,
    agents_loaded: HashSet<String>,
    /// Per-repository settings (agent overrides), by `repo_key`.
    repo_settings: HashMap<String, crate::settings::Settings>,
    repo_settings_requested: HashSet<String>,
    /// Settings → Agents edits this repository's overrides.
    agent_scope_repo: bool,
    /// A context menu and a dialog (ui_menu.rs).
    menu: Option<Menu>,
    dialog: Option<Dialog>,
    /// Diff options (FF-41): side by side, context, whitespace.
    pub diff_split: bool,
    pub diff_opts: ffwd_core::diff::DiffOpts,
    /// Find in the diff (FF-34); kept across files and commits.
    pub diff_find: Option<diff_find::DiffFind>,
    /// Whitespace characters shown in diffs, and the tab width (FF-41).
    pub diff_show_ws: bool,
    /// Long diff lines wrap (FF-41; unified view).
    pub diff_wrap: bool,
    /// How far code is scrolled sideways in the diff (pixels; FF-41).
    pub code_left: f32,
    /// The commit's details and diff fill the main area, the history list
    /// hidden (Space or the diff bar's expand button; FF-33).
    pub diff_max: bool,
    /// Installed WSL distributions, read when Settings opens (Windows).
    pub wsl_distros: Vec<String>,
    /// Merge and diff tools Git can start (Settings choices, FF-42).
    pub merge_tools: Vec<String>,
    pub diff_tools: Vec<String>,
    /// Minutes between automatic fetches (0: off), read by the clock
    /// thread (FF-39).
    pub(super) auto_fetch_minutes: Arc<std::sync::atomic::AtomicU64>,
    /// The last automatic fetch failed (its notice shows once).
    auto_fetch_failed: bool,
    pub diff_tab: usize,
    /// Code text size in points (diffs, file viewer; FF-41).
    pub code_pt: u8,
    diff_bar_hits: Vec<(Rect, diff_view::DiffBar)>,
    /// Line pairing and block-comment states of the shown diff.
    #[allow(clippy::type_complexity)]
    pairs_cache: Option<((usize, usize), std::rc::Rc<Vec<u32>>, std::rc::Rc<Vec<bool>>)>,
    /// The newest search; older search workers stop when it moves on.
    search_seq: Arc<std::sync::atomic::AtomicU64>,
    pub notice: Option<String>,
    update_available: bool,
    update_requested: bool,
    update_check_requested: bool,
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
    top_buttons: Vec<(Rect, TopAction)>,
    search_rect: Rect,
    /// The diff's overview strip (change marks; a click jumps there).
    diff_strip: Rect,
    /// The list title, which switches all branches / current branch.
    scope_rect: Rect,
    /// The comparison header's Swap and Close buttons.
    compare_btns: [Rect; 2],
    /// Extra height of the details header for a commit message longer
    /// than fits by default (up to five body lines; FF-57).
    details_extra: i32,
    /// The history header's Find button (FF-57).
    head_find_rect: Rect,
    /// The status bar's action for the notice (Install update, Discard,
    /// Undo) and where its button is.
    pub notice_act: Option<NoticeAct>,
    notice_btn: Rect,
    pub win_action: Option<WinAction>,
    /// Under the pointer, held by the mouse button, the cursor to show and
    /// the last pointer position (ui_hover.rs).
    hot: Option<Hot>,
    pressed: Option<Hot>,
    /// This click follows the last one at the same place (a double click).
    double: bool,
    last_click: Option<(std::time::Instant, i32, i32)>,
    /// Ctrl (Cmd on macOS) and Shift held at the last click.
    pub click_mods: (bool, bool),
    cursor: Cursor,
    pointer_at: Option<(i32, i32)>,
    pub damage: Damage,
}

/// "12 KB", "1.4 MB".
fn fmt_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{} KB", n.div_ceil(1024))
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

/// Splits a conventional-commit prefix off a subject: `("feat(ui): ",
/// "add x")`. Only short lowercase types count, so ordinary subjects with
/// a colon stay whole.
fn split_prefix(subject: &str) -> (&str, &str) {
    let Some(colon) = subject.find(": ") else { return ("", subject) };
    let head = &subject[..colon];
    let ty = head.split(['(', '!']).next().unwrap_or("");
    let scope_ok = match head.find('(') {
        Some(i) => head[i..].trim_end_matches('!').ends_with(')') && !head[i..].contains(' '),
        None => !head.contains(' '),
    };
    let ok = (2..=10).contains(&ty.len()) && ty.chars().all(|c| c.is_ascii_lowercase()) && scope_ok && colon <= 30;
    if ok { subject.split_at(colon + 2) } else { ("", subject) }
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn repo_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// One key per folder, whatever its spelling.
fn repo_key(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Two spellings of one folder (case, separators) are the same tab.
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

impl Ui {
    /// `first` is the tab whose loaders already run (id and path); the other
    /// tabs saved in `state` are restored but load only when activated.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        te: TextEngine,
        f: Fonts,
        settings: crate::settings::Settings,
        state: crate::settings::Settings,
        first: Option<(u64, PathBuf)>,
        sinks: SinkFactory,
        w: usize,
        h: usize,
        s: f32,
    ) -> Ui {
        let mut ui = Ui {
            s,
            w,
            h,
            lay: Layout::default(),
            te,
            f,
            now: now_secs(),
            r: RepoState::default(),
            tabs: Vec::new(),
            cur_tab: 0,
            next_tab_id: first.as_ref().map_or(1, |f| f.0 + 1),
            closed_tabs: Vec::new(),
            tab_hover: None,
            side_collapsed: state.get_list("sidebar.collapsed").into_iter().collect(),
            side_filter: String::new(),
            side_hover: None,
            sinks,
            known_repos: state.get_list("recent.repos").into_iter().map(PathBuf::from).collect(),
            state,
            title_dirty: true,
            focus: Focus::List,
            palette: None,
            settings_view: None,
            agents: Vec::new(),
            agent_view: None,
            clone_cancel: None,
            activity: Default::default(),
            agents_loaded: HashSet::new(),
            repo_settings: HashMap::new(),
            repo_settings_requested: HashSet::new(),
            agent_scope_repo: false,
            menu: None,
            dialog: None,
            search_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            diff_split: diff_view::opts_from(&settings).0,
            diff_find: None,
            diff_show_ws: diff_view::view_from(&settings).0,
            diff_wrap: diff_view::wrap_from(&settings),
            code_left: 0.0,
            diff_max: false,
            wsl_distros: Vec::new(),
            merge_tools: Vec::new(),
            diff_tools: Vec::new(),
            auto_fetch_minutes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            auto_fetch_failed: false,
            diff_tab: diff_view::view_from(&settings).1 as usize,
            code_pt: diff_view::code_size_from(&settings),
            diff_opts: diff_view::opts_from(&settings).1,
            diff_bar_hits: Vec::new(),
            pairs_cache: None,
            update_available: false,
            update_requested: false,
            update_check_requested: false,
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
            top_buttons: Vec::new(),
            search_rect: Rect::default(),
            diff_strip: Rect::default(),
            scope_rect: Rect::default(),
            compare_btns: [Rect::default(); 2],
            details_extra: 0,
            head_find_rect: Rect::default(),
            notice_act: None,
            notice_btn: Rect::default(),
            win_action: None,
            hot: None,
            pressed: None,
            double: false,
            last_click: None,
            click_mods: (false, false),
            cursor: Cursor::Default,
            pointer_at: None,
            damage: Damage { full: true, ..Default::default() },
        };
        // Saved tabs, then the one ffwd was started with.
        let saved: Vec<PathBuf> = ui.state.get_list("session.tabs").into_iter().map(PathBuf::from).collect();
        let saved_active = ui.state.get("session.active").map(PathBuf::from);
        // Names and colours the user gave tabs, by repository path.
        let styles: Vec<(PathBuf, Option<String>, Option<u8>)> = ui
            .state
            .get_list("session.tab_style")
            .into_iter()
            .filter_map(|e| {
                let mut f = e.split('\u{1f}');
                let p = PathBuf::from(f.next()?);
                let label = f.next().filter(|s| !s.is_empty()).map(str::to_string);
                let color = f.next().and_then(|c| c.parse::<u8>().ok()).filter(|&c| (c as usize) < TAB_COLORS.len());
                Some((p, label, color))
            })
            .collect();
        for p in &saved {
            let id = ui.take_id();
            let style = styles.iter().find(|x| same_path(&x.0, p));
            ui.tabs.push(Tab {
                id,
                path: p.clone(),
                name: repo_name(p),
                state: Some(RepoState::default()),
                pending: Vec::new(),
                started: false,
                stale: false,
                head_only: false,
                label: style.and_then(|x| x.1.clone()),
                color: style.and_then(|x| x.2),
            });
        }
        let mut active = saved_active.and_then(|a| ui.tabs.iter().position(|t| same_path(&t.path, &a))).unwrap_or(0);
        if let Some((id, path)) = first {
            match ui.tabs.iter().position(|t| same_path(&t.path, &path)) {
                Some(i) => {
                    ui.tabs[i].id = id;
                    ui.tabs[i].started = true;
                    active = i;
                }
                None => {
                    ui.tabs.push(Tab {
                        id,
                        path: path.clone(),
                        name: repo_name(&path),
                        state: Some(RepoState::default()),
                        pending: Vec::new(),
                        started: true,
                        stale: false,
                        head_only: false,
                        label: None,
                        color: None,
                    });
                    active = ui.tabs.len() - 1;
                }
            }
        }
        if ui.tabs.is_empty() {
            ui.r.repo_name = "ffwd".into();
            ui.r.error = Some("No repository open. Press Ctrl+O to open one.".into());
        } else {
            ui.cur_tab = active;
            ui.r = ui.tabs[active].state.take().unwrap_or_default();
            ui.activate_current();
        }
        ui.rebuild_side();
        ui.resize(w, h, s);
        ui.scan_repos();
        ui
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        id
    }

    // ----------------------------------------------------------------- tabs

    /// Makes `tabs[cur_tab]` (whose state is already in `self.r`) live:
    /// starts its loaders the first time, replays what arrived in the
    /// background, and repaints everything.
    fn activate_current(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.cur_tab) else { return };
        let (id, path) = (tab.id, tab.path.clone());
        let pending = std::mem::take(&mut tab.pending);
        let start = !tab.started;
        tab.started = true;
        if self.r.repo_name.is_empty() {
            self.r.repo_name = tab.name.clone();
        }
        if self.r.side_sel.is_empty() {
            self.r.side_sel = "all".into();
        }
        let sink = (self.sinks)(id);
        if start {
            let head_only = self.tabs.get(self.cur_tab).is_some_and(|t| t.head_only);
            crate::work::start_scoped(path.clone(), head_only, self.show_stashes(), &sink);
        }
        if self.r.worker.is_none() {
            // Git finds the repository from any folder inside it.
            self.r.worker = Some(DetailWorker::new(Git { root: path }, &sink));
        }
        for m in pending {
            self.on_msg(m);
        }
        self.load_repo_settings();
        self.load_agent_runs();
        if self.r.changes_view {
            self.reload_changes();
        }
        if self.r.files_view {
            self.load_files_tree();
        }
        self.palette = None;
        self.title_dirty = true;
        self.damage.full = true;
        self.save_session();
    }

    pub fn switch_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.cur_tab {
            return;
        }
        // The agents panel shows one repository's runs.
        self.agent_view = None;
        let old = std::mem::take(&mut self.r);
        if let Some(t) = self.tabs.get_mut(self.cur_tab) {
            t.state = Some(old);
        }
        self.cur_tab = i;
        self.r = self.tabs[i].state.take().unwrap_or_default();
        self.activate_current();
        if std::mem::take(&mut self.tabs[i].stale) {
            self.reload_current();
        }
    }

    /// Opens `path` in a new tab, or switches to it when it is already open.
    pub fn open_tab(&mut self, path: PathBuf) {
        if let Some(i) = self.tabs.iter().position(|t| same_path(&t.path, &path)) {
            self.switch_tab(i);
            return;
        }
        let id = self.take_id();
        let name = repo_name(&path);
        self.tabs.push(Tab {
            id,
            path: path.clone(),
            name,
            state: Some(RepoState::default()),
            pending: Vec::new(),
            started: false,
            stale: false,
            head_only: false,
            label: None,
            color: None,
        });
        self.remember_recent(&path);
        let i = self.tabs.len() - 1;
        if self.tabs.len() == 1 {
            self.cur_tab = 0;
            self.r = self.tabs[0].state.take().unwrap_or_default();
            self.activate_current();
        } else {
            self.switch_tab(i);
        }
        self.scan_repos();
    }

    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(i);
        self.closed_tabs.push(tab.path);
        if i == self.cur_tab {
            self.agent_view = None;
        }
        if self.tabs.is_empty() {
            self.r = RepoState { repo_name: "ffwd".into(), ..Default::default() };
            self.r.error = Some("No repository open. Press Ctrl+O to open one.".into());
            self.cur_tab = 0;
            self.title_dirty = true;
            self.damage.full = true;
            self.save_session();
            return;
        }
        if i == self.cur_tab {
            // The closed tab's state is dropped; its loaders' messages find no
            // tab and are discarded.
            self.cur_tab = i.min(self.tabs.len() - 1);
            self.r = self.tabs[self.cur_tab].state.take().unwrap_or_default();
            self.activate_current();
        } else {
            if i < self.cur_tab {
                self.cur_tab -= 1;
            }
            self.damage.rects.push(self.lay.tabs);
            self.save_session();
        }
    }

    pub fn reopen_closed_tab(&mut self) {
        if let Some(p) = self.closed_tabs.pop() {
            self.open_tab(p);
        }
    }

    /// Reloads the active repository (after a fetch, pull or push) and
    /// selects the same commit again once it is back.
    fn reload_current(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.cur_tab) else { return };
        let path = tab.path.clone();
        let reselect = self.r.commits.get(self.r.sel).map(|c| c.id);
        let id = self.take_id();
        let tab = &mut self.tabs[self.cur_tab];
        tab.id = id;
        tab.started = false;
        let name = std::mem::take(&mut self.r.repo_name);
        let (git, tz, notice) = (self.r.git.take(), self.r.tz, self.notice.take());
        let (changes_view, changes) = (self.r.changes_view, std::mem::take(&mut self.r.changes));
        let (files_view, files) = (self.r.files_view, std::mem::take(&mut self.r.files));
        let activity_view = self.r.activity_view;
        let reflog_view = self.r.reflog_view;
        // Refs picked for a batch stay picked across the reload (FF-36).
        let side_multi = std::mem::take(&mut self.r.side_multi);
        self.r = RepoState {
            repo_name: name,
            git,
            tz,
            reselect,
            changes_view,
            changes,
            files_view,
            files,
            activity_view,
            reflog_view,
            side_multi,
            ..Default::default()
        };
        self.notice = notice;
        let _ = path;
        self.activate_current();
    }

    /// Routes a loader message to its tab: applied now for the active tab,
    /// kept for later otherwise, dropped for closed or reloaded tabs.
    pub fn on_tab_msg(&mut self, id: u64, m: Msg) {
        if let Msg::Agent(e) = m {
            return self.on_agent_event(e);
        }
        if let Msg::RepoSettings(p, s) = m {
            return self.on_repo_settings(p, s);
        }
        // Clones and new repositories belong to no tab.
        if let Msg::CloneProgress(line) = m {
            return self.set_notice(format!("Cloning… {line} (the palette cancels it)"));
        }
        if let Msg::RepoReady(r) = m {
            return self.on_repo_ready(r);
        }
        // The activity log is app-wide: background tabs log at once.
        if let Msg::Activity(e) = m {
            return self.on_activity(*e);
        }
        if let Msg::AgentNotice(r) = m {
            return self.on_agent_notice(r);
        }
        if let Msg::AutoFetchTick = m {
            return self.auto_fetch_due();
        }
        if let Msg::Tools(merge, diff) = m {
            self.merge_tools = merge;
            self.diff_tools = diff;
            if self.settings_view.is_some() {
                let r = self.settings_rect();
                self.damage.rects.push(r);
            }
            return;
        }
        if let Msg::WslDistros(list) = m {
            self.wsl_distros = list;
            if self.settings_view.is_some() {
                let r = self.settings_rect();
                self.damage.rects.push(r);
            }
            return;
        }
        if let Msg::Repos(list) = m {
            for p in list {
                if !self.known_repos.iter().any(|k| same_path(k, &p)) {
                    self.known_repos.push(p);
                }
            }
            if self.palette.as_ref().is_some_and(|p| p.mode == PMode::Repos) {
                self.filter_palette();
            }
            return;
        }
        match self.tabs.iter().position(|t| t.id == id) {
            Some(i) if i == self.cur_tab => self.on_msg(m),
            Some(i) => {
                if let Msg::Repo(Ok((g, _))) = &m
                    && let Some(n) = g.root.file_name()
                {
                    self.tabs[i].name = n.to_string_lossy().into_owned();
                    self.damage.rects.push(self.lay.tabs);
                }
                self.tabs[i].pending.push(m);
            }
            None => {}
        }
    }

    fn remember_recent(&mut self, path: &Path) {
        let mut recent: Vec<String> = self.state.get_list("recent.repos");
        recent.retain(|p| !same_path(Path::new(p), path));
        recent.insert(0, path.to_string_lossy().into_owned());
        recent.truncate(30);
        self.state.set_list("recent.repos", &recent);
        self.known_repos.retain(|p| !same_path(p, path));
        self.known_repos.insert(0, path.to_path_buf());
    }

    fn save_session(&mut self) {
        let tabs: Vec<String> = self.tabs.iter().map(|t| t.path.to_string_lossy().into_owned()).collect();
        if self.state.get_list("session.tabs") != tabs {
            self.state.set_list("session.tabs", &tabs);
        }
        let styles: Vec<String> = self
            .tabs
            .iter()
            .filter(|t| t.label.is_some() || t.color.is_some())
            .map(|t| {
                format!(
                    "{}\u{1f}{}\u{1f}{}",
                    t.path.to_string_lossy(),
                    t.label.as_deref().unwrap_or(""),
                    t.color.map_or(String::new(), |c| c.to_string())
                )
            })
            .collect();
        if self.state.get_list("session.tab_style") != styles {
            self.state.set_list("session.tab_style", &styles);
        }
        if let Some(t) = self.tabs.get(self.cur_tab) {
            let a = t.path.to_string_lossy().into_owned();
            if self.state.get("session.active") != Some(a.as_str()) {
                self.state.set_string("session.active", &a);
            }
            let p = t.path.clone();
            if self.state.get_list("recent.repos").first().is_none_or(|f| !same_path(Path::new(f), &p)) {
                self.remember_recent(&p);
            }
        }
    }

    /// Looks for repositories next to the open ones (and in `repos.roots`,
    /// `;`-separated) on a worker, for the repository picker.
    fn scan_repos(&mut self) {
        let mut roots: Vec<PathBuf> = self
            .settings
            .get("repos.roots")
            .map(|s| s.split(';').filter(|x| !x.trim().is_empty()).map(|x| PathBuf::from(x.trim())).collect())
            .unwrap_or_default();
        for t in &self.tabs {
            if let Some(parent) = t.path.parent()
                && !roots.iter().any(|r| same_path(r, parent))
            {
                roots.push(parent.to_path_buf());
            }
        }
        if !roots.is_empty() {
            crate::work::scan_repos(roots, &(self.sinks)(0));
        }
    }

    pub fn window_title(&self) -> String {
        match self.tabs.get(self.cur_tab) {
            Some(t) => format!("{} · ffwd", t.title()),
            None => "ffwd".into(),
        }
    }

    // ------------------------------------------------------ top bar actions

    pub(super) fn start_op(&mut self, op: Op) {
        // Fetch while the automatic one runs: it becomes the user's (its
        // result shows), nothing restarts.
        if op == Op::Fetch && self.r.op == Some(Op::AutoFetch) {
            self.r.op = Some(Op::Fetch);
            self.set_notice("Fetching… (click Fetch again to cancel)".into());
            self.damage.rects.push(self.lay.top);
            return;
        }
        if let Some(running) = self.r.op {
            // The running operation's own button cancels it.
            if running.button() == op.button() {
                return self.cancel_op();
            }
            self.set_notice(format!("{} is still running (click its button to cancel).", running.name()));
            return;
        }
        let Some((id, path)) = self.tabs.get(self.cur_tab).map(|t| (t.id, t.path.clone())) else {
            self.set_notice("Open a repository first (Ctrl+O).".into());
            return;
        };
        let git = self.r.git.clone().unwrap_or(Git { root: path });
        self.r.op = Some(op);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.r.op_cancel = Some(cancel.clone());
        if op != Op::AutoFetch {
            self.set_notice(format!("{}… (click {} again to cancel)", op.verb(), op.button().name()));
        }
        self.damage.rects.push(self.lay.top);
        crate::work::run_op(git, op, cancel, &(self.sinks)(id));
    }

    /// Ends the running fetch, pull or push (git and its helpers).
    pub(super) fn cancel_op(&mut self) {
        if let (Some(op), Some(c)) = (self.r.op, &self.r.op_cancel) {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
            self.set_notice(format!("Cancelling the {}…", op.name().to_lowercase()));
        }
    }

    fn top_action(&mut self, a: TopAction) {
        match a {
            TopAction::Fetch => self.start_op(Op::Fetch),
            TopAction::Pull => self.start_op(Op::Pull),
            TopAction::Push => self.start_op(Op::Push),
            TopAction::Stash => self.open_stash_dialog(),
            TopAction::Ai => self.open_palette_with("AI: "),
            TopAction::Settings => self.open_settings(),
            TopAction::Branch => self.open_palette(),
            TopAction::Repo => self.open_repo_picker(),
        }
    }

    pub fn open_repo_picker(&mut self) {
        self.palette = Some(Palette { input: TextField::default(), items: Vec::new(), sel: 0, mode: PMode::Repos });
        self.filter_palette();
    }

    pub fn request_update_check(&mut self) {
        self.update_check_requested = true;
        self.set_notice("Checking for updates…".into());
    }

    pub fn take_update_check_requested(&mut self) -> bool {
        std::mem::take(&mut self.update_check_requested)
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
        let bar_h = self.px(46.0);
        // Repository tabs under the top bar.
        let tabs_h = self.px(32.0);
        let top_h = bar_h + tabs_h;
        let status_h = self.px(26.0);
        let side_w = self.px(236.0).min(wi / 4);
        let mid_h = (hi - top_h - status_h).max(0);
        let list_x = side_w;
        let (list_w, list_h, details) = match self.layout {
            // The large view: no history list, the details take it all.
            _ if self.diff_max => {
                let list_w = (wi - side_w).max(0);
                (list_w, 0, Rect::new(list_x, top_h, list_w, mid_h))
            }
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
        let sb_w = self.px(12.0);
        let list = Rect::new(list_x, top_h, list_w, list_h);
        self.lay = Layout {
            top: Rect::new(0, 0, wi, bar_h),
            tabs: Rect::new(0, bar_h, wi, tabs_h),
            status: Rect::new(0, hi - status_h, wi, status_h),
            side: Rect::new(0, top_h, side_w, mid_h),
            list,
            // No list in the large view: no header either.
            list_head: Rect::new(list_x, top_h, list_w, if self.diff_max { 0 } else { head_h }),
            list_body: Rect::new(list_x, top_h + head_h, list_w - sb_w, (list_h - head_h).max(0)),
            scrollbar: Rect::new(list_x + list_w - sb_w, top_h + head_h, sb_w, (list_h - head_h).max(0)),
            details,
            info,
            row_h,
            caption: if self.chrome == Chrome::Windows11 {
                let cw = self.px(46.0) * 3;
                Rect::new(wi - cw, 0, cw, bar_h)
            } else {
                Rect::default()
            },
        };
        self.r.top = self.r.top.min(self.max_top());
        // The message wraps differently at the new width.
        self.details_extra = self.details_extra_for();
        self.damage.full = true;
    }

    pub fn visible_rows(&self) -> usize {
        (self.lay.list_body.h / self.lay.row_h.max(1)).max(1) as usize
    }

    fn max_top(&self) -> usize {
        self.r.commits.len().saturating_sub(self.visible_rows())
    }

    fn row_rect(&self, row: usize) -> Option<Rect> {
        if row < self.r.top {
            return None;
        }
        let i = (row - self.r.top) as i32;
        let b = self.lay.list_body;
        let y = b.y + i * self.lay.row_h;
        (y < b.bottom()).then(|| Rect::new(self.lay.list.x, y, self.lay.list.w - self.lay.scrollbar.w, self.lay.row_h))
    }

    // ---------------------------------------------------------------- data

    pub fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Repo(Ok((git, tz))) => {
                if let Some(n) = git.root.file_name() {
                    self.r.repo_name = n.to_string_lossy().into_owned();
                    if let Some(t) = self.tabs.get_mut(self.cur_tab) {
                        t.name = self.r.repo_name.clone();
                    }
                    self.title_dirty = true;
                }
                self.r.tz = tz;
                self.r.git = Some(git);
                self.damage.rects.extend([self.lay.top, self.lay.tabs, self.lay.list, self.lay.details]);
            }
            Msg::Repo(Err(e)) => {
                self.r.error = Some(e);
                self.damage.full = true;
            }
            Msg::Refs(Ok(r)) => {
                self.r.refs = r;
                self.index_chips();
                self.rebuild_side();
                self.damage.rects.extend([self.lay.side, self.lay.top]);
                self.damage_visible_rows();
                // Pull requests come from the hosting site (FF-40).
                if self.hosting_enabled() {
                    self.load_prs();
                }
            }
            Msg::Refs(Err(e)) => self.set_notice(format!("Could not read refs: {e}")),
            Msg::Status(s) => {
                self.r.status = Some(s);
                self.rebuild_side();
                self.damage.rects.push(self.lay.side);
                // The tab shows a dot for uncommitted changes.
                self.damage.rects.push(self.lay.tabs);
            }
            Msg::Log { batch, done, elapsed_ms } => {
                let was_empty = self.r.commits.is_empty();
                let start = self.r.commits.len();
                for c in batch {
                    self.r.graph.push(&c);
                    self.r.by_id.insert(c.id, self.r.commits.len() as u32);
                    self.r.commits.push(c);
                }
                self.search_new_rows(start);
                if self.r.first_batch_ms.is_none() {
                    self.r.first_batch_ms = Some(elapsed_ms);
                }
                self.r.log_done = done;
                self.r.log_ms = elapsed_ms;
                if was_empty && !self.r.commits.is_empty() {
                    self.index_chips();
                    if self.r.reselect.is_none() {
                        self.select_head_or_first();
                    }
                }
                if let Some(id) = self.r.reselect
                    && let Some(&row) = self.r.by_id.get(&id)
                {
                    self.r.reselect = None;
                    self.select(row as usize);
                } else if done && self.r.reselect.take().is_some() {
                    self.select_head_or_first();
                }
                // Only rows that became visible need paint.
                let end = (self.r.top + self.visible_rows() + 1).min(self.r.commits.len());
                self.damage.rows.extend(start.max(self.r.top)..end);
                self.damage.rects.extend([self.lay.status, self.lay.scrollbar, self.lay.list_head]);
            }
            Msg::Compared { seq, result } => match result {
                Ok(d) if seq == self.r.seq => {
                    // Not cached: the details of `b` stay its own.
                    self.r.details = Some(d);
                    self.r.diff = None;
                    self.r.diff_top = 0;
                    self.show_first_file();
                    self.request_diff();
                    // The diff pane now clips its own drawing (FF-58), so the
                    // details damage is enough.
                    self.damage.rects.extend([self.lay.details, self.lay.status]);
                }
                Err(e) if seq == self.r.seq => {
                    self.r.compare = None;
                    self.set_notice(format!("Could not compare: {e}"));
                }
                _ => {}
            },
            Msg::Details { seq, result } => match result {
                Ok(d) => {
                    let id = d.id;
                    if self.r.cache.len() > 512 {
                        self.r.cache.clear();
                    }
                    self.r.cache.insert(id, d.clone());
                    if seq == self.r.seq {
                        self.r.details = Some(d);
                        self.show_first_file();
                        self.request_diff();
                        self.damage.rects.push(self.lay.details);
                    }
                }
                Err(e) if seq == self.r.seq => self.set_notice(format!("Could not load commit: {e}")),
                Err(_) => {}
            },
            Msg::Diff { seq, file, result } => {
                if seq == self.r.seq && file == self.r.sel_file {
                    match result {
                        Ok(d) => {
                            let image = d.binary && crate::image::is_image(&d.path);
                            // A newly shown file goes back where it was scrolled
                            // to last time; an option change (the old diff still
                            // shown) keeps the current scroll.
                            let back = self
                                .r
                                .details
                                .as_ref()
                                .and_then(|det| self.r.diff_scroll.get(&(det.id, d.path.clone())));
                            if self.r.diff.is_none() {
                                self.r.diff_top = back.copied().unwrap_or(0);
                            }
                            self.r.diff = Some(d);
                            if self.r.diff_top > 0 {
                                let n = self.diff_row_count();
                                self.r.diff_top = self.r.diff_top.min(n.saturating_sub(1));
                            }
                            if image {
                                self.load_diff_images(seq);
                            }
                            self.code_left = 0.0;
                            // The find query stays: search the new diff.
                            self.diff_find_on_new_diff();
                        }
                        Err(e) => self.set_notice(format!("Could not load diff: {e}")),
                    }
                    self.damage.rects.push(self.diff_rect());
                }
            }
            Msg::DiffImages { seq, old, new } => {
                if seq == self.r.seq {
                    self.r.diff_images = Some((seq, old, new));
                    self.damage.rects.push(self.diff_rect());
                }
            }
            Msg::WorkerPanic(e) => self.set_notice(format!("Internal error (window kept alive): {e}")),
            Msg::OpProgress(line) => {
                // The automatic fetch stays quiet (FF-39).
                if let Some(op) = self.r.op.filter(|o| *o != Op::AutoFetch) {
                    self.set_notice(format!("{}… {line}", op.verb()));
                }
            }
            Msg::RefProgress(text) => self.set_notice(text),
            Msg::OpDone(out) => {
                self.log_sync(&out);
                let quiet = self.r.op == Some(Op::AutoFetch);
                self.r.op = None;
                self.r.op_cancel = None;
                if quiet {
                    return self.auto_fetch_done(&out);
                }
                self.set_notice(format!("{} ({:.1} s)", out.summary, out.ms / 1000.0));
                self.damage.rects.push(self.lay.top);
                if out.op == Op::Pull && !out.ok && ffwd_core::ops::diverged(&out.output) {
                    self.open_dialog(DialogKind::Ask(menu::Ask {
                        title: "Both sides have new commits".into(),
                        text: "Your branch and its upstream have each moved on, and your Git settings do not say how to combine them. Merge makes a merge commit; rebase replays your commits on top of the upstream. Git stops on conflicts either way.".into(),
                        choices: vec![
                            ("Merge".into(), crate::work::RefOp::Sync(Op::PullMerge), false),
                            ("Rebase".into(), crate::work::RefOp::Sync(Op::PullRebase), false),
                        ],
                    }));
                }
                // Refs, status and history changed: load them again.
                if out.ok {
                    self.reload_current();
                }
            }
            // Handled in on_tab_msg; repeated here so a stray one is harmless.
            Msg::Repos(_) | Msg::WslDistros(_) | Msg::Tools(..) | Msg::AgentNotice(_) | Msg::AutoFetchTick => {}
            Msg::Agent(e) => self.on_agent_event(e),
            Msg::RepoSettings(p, s) => self.on_repo_settings(p, s),
            Msg::Changes(r) => self.on_changes(r),
            Msg::Ignored(r) => self.on_ignored(r),
            Msg::ChangesDiff { seq, result } => self.on_changes_diff(seq, result),
            Msg::ChangesOp { commit, result } => self.on_changes_op(commit, result),
            Msg::LastMessage(m) => self.on_last_message(m),
            Msg::Discarded(b) => self.r.changes.undo.push(b),
            Msg::RefOp(r) => self.on_refop(r),
            Msg::SearchBatch { seq, ids, done } => self.on_search_batch(seq, ids, done),
            Msg::InProgress(k) => {
                if self.r.changes.in_progress != k {
                    self.r.changes.in_progress = k;
                    if self.r.changes_view {
                        self.damage.full = true;
                    }
                }
            }
            Msg::ConflictFile { seq, path, result } => self.on_conflict_file(seq, path, result),
            Msg::Activity(e) => self.on_activity(*e),
            Msg::PatchMade(r) => self.on_patch_made(r),
            Msg::PatchChecked { patch, result } => self.on_patch_checked(patch, result),
            Msg::ToolDone { merge, result } => self.on_tool_done(merge, result),
            Msg::CiRuns(runs) => {
                self.r.ci = runs
                    .into_iter()
                    .filter_map(|(sha, st, url)| Oid::from_hex(sha.as_bytes()).map(|id| (id, (st, url))))
                    .collect();
                self.damage.rects.push(self.lay.list);
            }
            Msg::PullRequests(r) => {
                self.r.prs = Some(r);
                self.rebuild_side();
                self.damage.rects.push(self.lay.side);
            }
            Msg::PrCreate(r) => match r {
                Ok(m) => self.set_notice(m),
                Err(e) => self.set_notice(format!("Failed: {e}")),
            },
            Msg::Reflog(r) => self.on_reflog(r),
            Msg::AiPrepared { seq, result } => self.on_ai_prepared(seq, result),
            Msg::AiToken { seq, text } => self.on_ai_token(seq, text),
            Msg::AiDone { seq, result } => self.on_ai_done(seq, result),
            Msg::CloneProgress(_) | Msg::RepoReady(_) => {}
            Msg::SyncMarks(ahead, behind) => {
                self.r.ahead_ids = ahead.into_iter().collect();
                self.r.behind_ids = behind.into_iter().collect();
                self.damage.rects.push(self.lay.list);
            }
            Msg::RebasePlan(r) => self.on_rebase_plan(r),
            Msg::FilesTree(r) => self.on_files_tree(r),
            Msg::FileContent { seq, result } => self.on_file_content(seq, result),
            Msg::FileBlame { seq, result } => self.on_file_blame(seq, result),
            Msg::FileHistory { seq, result } => self.on_file_history(seq, result),
        }
    }

    /// Switches the details layout and saves it as `ui.layout`.
    pub fn set_layout(&mut self, l: DetailsLayout) {
        if l == self.layout {
            return;
        }
        self.layout = l;
        self.settings.set_string("ui.layout", l.as_setting());
        self.r.file_top = 0;
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
        self.notice_act = None;
        self.damage.rects.push(self.lay.status);
    }

    /// A notice with a button for what it offers.
    pub(super) fn set_notice_with(&mut self, n: String, act: NoticeAct) {
        self.set_notice(n);
        self.notice_act = Some(act);
    }

    /// The status bar's button was clicked.
    fn notice_click(&mut self) {
        let Some(a) = self.notice_act.take() else { return };
        self.damage.rects.push(self.lay.status);
        match a {
            NoticeAct::InstallUpdate => self.request_update_install(),
            NoticeAct::Discard => self.discard_now(),
            NoticeAct::Undo => self.undo_discard_now(),
        }
    }

    pub fn offer_update(&mut self, version: &ffwd_update::Version) {
        self.update_available = true;
        self.set_notice_with(crate::update::notice(version), NoticeAct::InstallUpdate);
    }

    pub fn set_update_message(&mut self, message: String) {
        self.set_notice(message);
    }

    pub fn request_update_install(&mut self) {
        self.update_requested = self.update_available;
        if !self.update_available {
            self.set_notice("No update is waiting. Ctrl+K → Check for updates looks now.".into());
        }
    }

    pub fn take_update_requested(&mut self) -> bool {
        std::mem::take(&mut self.update_requested)
    }

    /// Settings `[update] check` (default true) and `channel`.
    pub fn update_policy(&self) -> crate::update::Policy {
        crate::update::Policy {
            enabled: self.settings.get("update.check") != Some("false"),
            channel: self.settings.get("update.channel").map(str::to_owned),
            force: false,
        }
    }

    fn index_chips(&mut self) {
        self.r.chips.clear();
        for (i, r) in self.r.refs.refs.iter().enumerate() {
            self.r.chips.entry(r.target).or_default().push(i as u32);
        }
        for v in self.r.chips.values_mut() {
            let refs = &self.r.refs.refs;
            v.sort_by_key(|&i| {
                let r = &refs[i as usize];
                (!r.is_head, r.kind, r.name.len())
            });
        }
    }

    fn select_head_or_first(&mut self) {
        let head = self.r.refs.head.and_then(|h| self.r.by_id.get(&h).copied());
        self.select(head.map_or(0, |i| i as usize));
    }

    // ------------------------------------------------------------ selection

    fn damage_visible_rows(&mut self) {
        let end = (self.r.top + self.visible_rows() + 1).min(self.r.commits.len());
        self.damage.rows.extend(self.r.top..end);
    }

    pub fn select(&mut self, row: usize) {
        if self.r.commits.is_empty() {
            return;
        }
        let row = row.min(self.r.commits.len() - 1);
        let old = self.r.sel;
        self.r.sel = row;
        self.damage.rows.extend([old, row]);
        let vis = self.visible_rows();
        if row < self.r.top {
            self.set_top(row);
        } else if row >= self.r.top + vis {
            self.set_top(row + 1 - vis);
        }
        self.r.seq += 1;
        self.remember_diff_scroll();
        self.r.diff = None;
        self.r.diff_top = 0;
        self.r.sel_file = 0;
        self.r.file_top = 0;
        let id = self.r.commits[row].id;
        self.r.compare = None;
        // Same-frame answer: cached details show at once; otherwise the
        // header paints from the list data and the rest fills in.
        self.r.details = self.r.cache.get(&id).cloned();
        if self.r.details.is_some() {
            self.show_first_file();
            self.request_diff();
        } else {
            self.update_details_extra();
            self.request_details();
        }
        // The scroll track carries the selection marker.
        self.damage.rects.extend([self.lay.details, self.lay.status, self.lay.scrollbar]);
    }

    /// Selects the commit's first file, or the first that matched the
    /// search (FF-34), and scrolls the file list to it.
    fn show_first_file(&mut self) {
        self.update_details_extra();
        let i = self.first_file();
        self.r.sel_file = i;
        self.r.file_top = (i + 1).saturating_sub(self.file_rows_visible().max(1));
    }

    /// Keeps where the shown diff is scrolled to, for when its file is
    /// shown again; a diff at the top is forgotten.
    fn remember_diff_scroll(&mut self) {
        let (Some(d), Some(det)) = (&self.r.diff, &self.r.details) else { return };
        let key = (det.id, d.path.clone());
        if self.r.diff_top == 0 {
            self.r.diff_scroll.remove(&key);
            return;
        }
        if self.r.diff_scroll.len() >= 1000 {
            self.r.diff_scroll.clear();
        }
        self.r.diff_scroll.insert(key, self.r.diff_top);
    }

    fn request_details(&mut self) {
        if let (Some(w), Some(c)) = (&self.r.worker, self.r.commits.get(self.r.sel)) {
            w.details(self.r.seq, c.id);
        }
    }

    fn request_diff(&mut self) {
        if let (Some(w), Some(d)) = (&self.r.worker, &self.r.details)
            && let Some(f) = d.files.get(self.r.sel_file)
        {
            w.diff(self.r.seq, d.id, d.parents.clone(), self.r.sel_file, f.clone(), self.diff_opts);
        }
    }

    pub fn set_top(&mut self, top: usize) {
        let top = top.min(self.max_top());
        let d = top as i64 - self.r.top as i64;
        if d == 0 {
            return;
        }
        self.r.top = top;
        if d.unsigned_abs() as usize >= self.visible_rows() {
            self.damage.rects.push(self.lay.list_body);
            self.damage_visible_rows();
        } else {
            self.damage.scroll += d;
        }
        self.damage.rects.extend([self.lay.scrollbar, self.lay.status]);
    }

    /// Snapshot only: show file `i` of the selected commit.
    pub fn snapshot_file(&mut self, i: usize) {
        self.select_file(i);
    }

    fn select_file(&mut self, i: usize) {
        let Some(d) = &self.r.details else { return };
        if d.files.is_empty() {
            return;
        }
        let i = i.min(d.files.len() - 1);
        if i == self.r.sel_file && self.r.diff.is_some() {
            return;
        }
        self.r.sel_file = i;
        let vis = self.file_rows_visible();
        if i < self.r.file_top {
            self.r.file_top = i;
        } else if i >= self.r.file_top + vis {
            self.r.file_top = i + 1 - vis;
        }
        self.remember_diff_scroll();
        self.r.diff = None;
        self.r.diff_top = 0;
        self.r.seq += 1;
        self.request_diff();
        self.damage.rects.push(self.lay.details);
    }

    // ---------------------------------------------------------------- input

    pub fn key(&mut self, k: Key) {
        if self.dialog.is_some() {
            return self.dialog_key(k);
        }
        if self.menu.is_some() {
            return self.menu_key(k);
        }
        // The rebase plan takes the keys while it is open (like a dialog),
        // unless the palette or a panel is on top.
        if self.r.rebase.is_some()
            && self.palette.is_none()
            && self.settings_view.is_none()
            && self.agent_view.is_none()
        {
            return self.rebase_key(k);
        }
        if matches!(k, Key::Ctrl('.'))
            && self.settings_view.is_none()
            && self.agent_view.is_none()
            && !self.tabs.is_empty()
        {
            return self.open_menu_for_selection();
        }
        if self.settings_view.is_none() && self.agent_view.is_none() && self.palette.is_none() {
            let history = !self.r.changes_view && !self.r.files_view && !self.r.activity_view;
            if history && self.focus == Focus::Diff {
                if matches!(k, Key::Ctrl('f')) {
                    return self.open_diff_find();
                }
                if self.diff_find.as_ref().is_some_and(|f| f.typing) && self.diff_find_key(k) {
                    return;
                }
            }
            if matches!(k, Key::Ctrl('f')) && !self.r.changes_view && !self.r.files_view && !self.r.activity_view {
                return self.open_search();
            }
            if self.r.search.as_ref().is_some_and(|s| s.typing) && !self.r.changes_view && self.search_key(k) {
                return;
            }
        }
        if matches!(k, Key::Ctrl(',')) && self.settings_view.is_none() {
            return self.open_settings();
        }
        if self.settings_view.is_some() {
            return self.settings_key(k);
        }
        if matches!(k, Key::CtrlShift('a')) && self.agent_view.is_none() {
            return self.open_agents();
        }
        if self.agent_view.is_some() {
            return self.agents_key(k);
        }
        // Text editing keys belong to the palette while it is open.
        if self.palette.is_some()
            && matches!(
                k,
                Key::Ctrl('a' | 'c' | 'v' | 'x')
                    | Key::CtrlBackspace
                    | Key::Left
                    | Key::Right
                    | Key::Home
                    | Key::End
                    | Key::Delete
            )
        {
            self.palette_key(k);
            return;
        }
        // Local Changes' own Ctrl keys (text editing, undo, recent
        // messages, commit) come before the global shortcuts.
        if self.r.changes_view
            && self.palette.is_none()
            && !matches!(self.focus, Focus::Side | Focus::SideFilter)
            && matches!(k, Key::Ctrl('a' | 'c' | 'v' | 'x' | 'z' | 'r') | Key::CtrlBackspace | Key::CtrlEnter)
            && self.changes_key(k)
        {
            return;
        }
        // Global shortcuts work everywhere, also with the palette open.
        let n = self.tabs.len();
        match k {
            Key::Ctrl('o') | Key::Ctrl('t') => return self.open_repo_picker(),
            Key::Ctrl('w') => return self.close_tab(self.cur_tab),
            Key::CtrlShift('t') => return self.reopen_closed_tab(),
            Key::NextTab if n > 1 => return self.switch_tab((self.cur_tab + 1) % n),
            Key::PrevTab if n > 1 => return self.switch_tab((self.cur_tab + n - 1) % n),
            Key::Ctrl(d @ '1'..='9') => {
                // Ctrl+9 is the last tab, as in browsers.
                let i = if d == '9' { n.saturating_sub(1) } else { d as usize - '1' as usize };
                return self.switch_tab(i);
            }
            Key::CtrlShift('f') => return self.start_op(Op::Fetch),
            Key::CtrlShift('l') => return self.start_op(Op::Pull),
            Key::CtrlShift('k') => return self.start_op(Op::Push),
            Key::CtrlShift('u') => return self.request_update_install(),
            Key::CtrlShift('s') => return self.open_stash_dialog(),
            Key::CtrlShift('r') if !self.r.changes_view => return self.rebase_from_selection(),
            Key::Ctrl('b') if !self.r.changes_view => return self.selected_commit_command(CMD_BRANCH),
            Key::CtrlShift('g') if !self.r.changes_view => return self.selected_commit_command(CMD_TAG),
            Key::Ctrl(_) | Key::CtrlShift(_) | Key::NextTab | Key::PrevTab => return,
            _ => {}
        }
        if self.palette.is_some() {
            self.palette_key(k);
            return;
        }
        if self.side_key(k) {
            return;
        }
        if self.r.changes_view && self.focus != Focus::Side && self.focus != Focus::SideFilter && self.changes_key(k) {
            return;
        }
        if self.r.files_view && self.focus != Focus::Side && self.focus != Focus::SideFilter && self.files_key(k) {
            return;
        }
        if self.r.activity_view && self.focus != Focus::Side && self.focus != Focus::SideFilter && self.activity_key(k)
        {
            return;
        }
        if self.r.reflog_view && self.focus != Focus::Side && self.focus != Focus::SideFilter && self.reflog_key(k) {
            return;
        }
        let page = self.visible_rows().saturating_sub(1).max(1);
        match (self.focus, k) {
            (_, Key::Palette) => self.open_palette(),
            (_, Key::Tab) => self.cycle_focus(1),
            (_, Key::BackTab) => self.cycle_focus(-1),
            (Focus::List, Key::Up) | (Focus::List, Key::Char('k')) => self.select(self.r.sel.saturating_sub(1)),
            (Focus::List, Key::Down) | (Focus::List, Key::Char('j')) => self.select(self.r.sel + 1),
            (Focus::List, Key::PageUp) => self.select(self.r.sel.saturating_sub(page)),
            (Focus::List, Key::PageDown) => self.select(self.r.sel + page),
            (Focus::List, Key::Home) => self.select(0),
            (Focus::List, Key::End) => self.select(usize::MAX),
            (Focus::List, Key::Enter) => self.cycle_focus(1),
            // Space: the large view of the selected commit, and back.
            (Focus::List | Focus::Files | Focus::Diff, Key::Char(' ')) => self.toggle_diff_max(),
            (_, Key::Esc) if self.diff_max => self.toggle_diff_max(),
            (Focus::Files, Key::Up) | (Focus::Files, Key::Char('k')) => {
                self.select_file(self.r.sel_file.saturating_sub(1))
            }
            (Focus::Files, Key::Down) | (Focus::Files, Key::Char('j')) => self.select_file(self.r.sel_file + 1),
            (Focus::Files, Key::Enter) => self.cycle_focus(1),
            (Focus::Diff, Key::Up) | (Focus::Diff, Key::Char('k')) => self.scroll_diff(-1),
            (Focus::Diff, Key::Down) | (Focus::Diff, Key::Char('j')) => self.scroll_diff(1),
            (Focus::Diff, Key::PageUp) => self.scroll_diff(-20),
            (Focus::Diff, Key::PageDown) => self.scroll_diff(20),
            (Focus::Diff, Key::Char('s')) => {
                let h = if self.diff_split { diff_view::DiffBar::Unified } else { diff_view::DiffBar::Split };
                self.apply_diff_bar(h);
            }
            (Focus::Diff, Key::Char('w')) => self.apply_diff_bar(diff_view::DiffBar::Whitespace),
            (Focus::Diff, Key::Char('n')) => self.jump_change(true),
            (Focus::Diff, Key::Char('p')) => self.jump_change(false),
            (Focus::Diff, Key::Char('c')) => {
                let next = match self.diff_opts.context {
                    3 => 10,
                    10 => u32::MAX,
                    _ => 3,
                };
                self.apply_diff_bar(diff_view::DiffBar::Context(next));
            }
            (_, Key::Esc) => {
                if self.focus != Focus::List {
                    self.focus = Focus::List;
                    self.damage.rects.extend([self.lay.details, self.lay.side]);
                    self.damage.rows.push(self.r.sel);
                }
            }
            (_, Key::Char('/')) => self.open_palette(),
            _ => {}
        }
    }

    fn cycle_focus(&mut self, dir: i32) {
        let order = [Focus::Side, Focus::List, Focus::Files, Focus::Diff];
        let cur = if self.focus == Focus::SideFilter { Focus::Side } else { self.focus };
        let i = order.iter().position(|f| *f == cur).unwrap_or(1) as i32;
        self.focus = order[((i + dir).rem_euclid(order.len() as i32)) as usize];
        self.damage.rects.extend([self.lay.details, self.lay.side]);
        self.damage.rows.push(self.r.sel);
    }

    fn scroll_diff(&mut self, d: i64) {
        let n = self.diff_row_count();
        let top = (self.r.diff_top as i64 + d).clamp(0, n.saturating_sub(1) as i64) as usize;
        if top != self.r.diff_top {
            self.r.diff_top = top;
            self.damage.rects.push(self.diff_rect());
        }
    }

    /// The large view of the selected commit (details and diff fill the
    /// main area) on or off.
    pub(super) fn toggle_diff_max(&mut self) {
        self.diff_max = !self.diff_max;
        if self.diff_max {
            // The hidden header's controls must not catch clicks.
            self.scope_rect = Rect::default();
            self.head_find_rect = Rect::default();
        } else {
            self.focus = Focus::List;
        }
        self.resize(self.w, self.h, self.s);
    }

    /// Scrolls code sideways by `px` over a diff (no wrap), up to the
    /// widest line on screen.
    pub fn wheel_h(&mut self, x: i32, y: i32, px: f32) {
        if self.diff_wrap || px == 0.0 {
            return;
        }
        let (diff, top, body) = if self.r.changes_view {
            let r = self.changes_layout().diff;
            if !r.contains(x, y) || self.r.changes.ai.is_some() || self.r.changes.conflict.is_some() {
                return;
            }
            (self.r.changes.diff.clone(), self.changes_diff_top(), Rect::new(r.x, r.y + self.px(32.0), r.w, r.h))
        } else {
            let r = self.diff_rect();
            if !r.contains(x, y) {
                return;
            }
            (self.r.diff.clone(), self.r.diff_top, Rect::new(r.x + 1, r.y + self.px(32.0), r.w - 1, r.h))
        };
        let Some(d) = diff else { return };
        let (_, w) = self.code_column(body);
        let rows = (body.h / self.code_lh().max(1)) as usize + 1;
        let (mono, fs) = (self.f.mono, self.code_fs());
        let widest = d
            .lines
            .iter()
            .skip(top)
            .take(rows)
            .map(|l| self.te.measure(mono, fs, &diff_find::shown_text(&l.text, self.diff_tab)))
            .fold(0.0f32, f32::max);
        let max = (widest - w + self.pf(24.0)).max(0.0);
        let left = (self.code_left + px).clamp(0.0, max);
        if left != self.code_left {
            self.code_left = left;
            let r = if self.r.changes_view { self.changes_layout().diff } else { self.diff_rect() };
            self.damage.rects.push(r);
        }
    }

    pub fn wheel(&mut self, x: i32, y: i32, lines: f32) {
        let n = lines.round() as i64;
        if n == 0 {
            return;
        }
        self.pointer_at = Some((x, y));
        if self.settings_view.is_some() {
            self.settings_wheel(n);
        } else if self.agent_view.is_some() {
            self.agents_wheel(n);
        } else if self.lay.side.contains(x, y) {
            self.side_wheel(n);
        } else if self.r.rebase.is_some() {
            self.rebase_wheel(n);
        } else if self.r.reflog_view {
            self.reflog_wheel(n);
        } else if self.r.activity_view {
            self.activity_wheel(x, y, n);
        } else if self.r.files_view {
            self.files_wheel(x, y, n);
        } else if self.r.changes_view {
            self.changes_wheel(x, y, n);
        } else if self.lay.list.contains(x, y) {
            self.set_top((self.r.top as i64 + n).max(0) as usize);
        } else if self.diff_rect().contains(x, y) {
            self.scroll_diff(n);
        }
        // The content moved under a resting pointer.
        self.refresh_hot();
    }

    pub fn click(&mut self, x: i32, y: i32) {
        if self.dialog.is_some() {
            self.press_at(x, y);
            return self.dialog_click(x, y);
        }
        if self.menu.is_some() {
            return self.menu_click(x, y);
        }
        let now = std::time::Instant::now();
        self.double = self.last_click.is_some_and(|(t, px, py)| {
            now.duration_since(t).as_millis() < 450 && (px - x).abs() < 5 && (py - y).abs() < 5
        });
        self.last_click = Some((now, x, y));
        self.press_at(x, y);
        if self.settings_view.is_some() {
            return self.settings_click(x, y);
        }
        if self.agent_view.is_some() {
            return self.agents_click(x, y);
        }
        if self.r.rebase.is_some() && self.palette.is_none() && self.rebase_area().contains(x, y) {
            return self.rebase_click(x, y);
        }
        if let Some(b) = self.caption_button_at(x, y) {
            if b != 1 {
                self.cap_pressed = Some(b);
                self.damage.rects.push(self.lay.caption);
            }
            return;
        }
        if let Some(Hot::Palette(k)) = self.hot {
            // A click on an entry chooses it, as Enter does.
            if let Some(p) = &mut self.palette {
                p.sel = k;
            }
            self.palette_key(Key::Enter);
            return;
        }
        if self.palette.is_some() {
            self.close_palette();
            return;
        }
        if self.lay.tabs.contains(x, y) {
            let (rects, plus) = self.tab_rects();
            if let Some(i) = rects.iter().position(|r| r.contains(x, y)) {
                if self.tab_close_rect(rects[i]).contains(x, y) {
                    self.close_tab(i);
                } else {
                    self.switch_tab(i);
                }
            } else if plus.contains(x, y) {
                self.open_repo_picker();
            }
            return;
        }
        if let Some(&(_, a)) = self.top_buttons.iter().find(|(r, _)| r.contains(x, y)) {
            self.top_action(a);
            return;
        }
        if self.r.reflog_view && self.reflog_click(x, y) {
            if self.focus == Focus::Side || self.focus == Focus::SideFilter {
                self.focus = Focus::List;
                self.damage.rects.push(self.lay.side);
            }
            return;
        }
        if self.r.activity_view && self.activity_click(x, y) {
            if self.focus == Focus::Side || self.focus == Focus::SideFilter {
                self.focus = Focus::List;
                self.damage.rects.push(self.lay.side);
            }
            return;
        }
        if self.r.files_view && self.files_click(x, y) {
            if self.focus == Focus::Side || self.focus == Focus::SideFilter {
                self.focus = Focus::List;
                self.damage.rects.push(self.lay.side);
            }
            return;
        }
        if !self.r.changes_view
            && !self.r.files_view
            && !self.r.activity_view
            && self.r.search.is_none()
            && self.scope_rect.contains(x, y)
        {
            let items = self.scope_menu();
            let r = self.scope_rect;
            return self.open_menu(r.x, r.bottom(), items);
        }
        if self.notice_btn.contains(x, y) {
            return self.notice_click();
        }
        if self.r.compare.is_some() {
            if self.compare_btns[0].contains(x, y)
                && let Some((a, b)) = self.r.compare
            {
                return self.start_compare(b, a);
            }
            if self.compare_btns[1].contains(x, y) {
                let row = self.r.sel;
                self.r.compare = None;
                return self.select(row);
            }
        }
        if !self.r.changes_view && self.r.search.is_none() && self.head_find_rect.contains(x, y) {
            return self.open_search();
        }
        if !self.r.changes_view && self.r.search.is_some() && self.search_click(x, y) {
            return;
        }
        if !self.r.changes_view && self.diff_find_click(x, y) {
            return;
        }
        if !self.r.changes_view && self.diff_bar_click(x, y) {
            return;
        }
        if !self.r.changes_view && self.r.diff.is_some() && self.diff_strip.contains(x, y) {
            return self.diff_strip_click(y);
        }
        if self.r.changes_view && self.changes_click(x, y) {
            if self.focus == Focus::Side || self.focus == Focus::SideFilter {
                self.focus = Focus::List;
                self.damage.rects.push(self.lay.side);
            }
            return;
        }
        let b = self.lay.list_body;
        if b.contains(x, y) {
            let row = self.r.top + ((y - b.y) / self.lay.row_h) as usize;
            if row < self.r.commits.len() {
                if self.focus != Focus::List {
                    self.focus = Focus::List;
                    self.damage.rects.extend([self.lay.details, self.lay.side]);
                }
                let again = self.double && row == self.r.sel;
                self.select(row);
                if again {
                    // A double click checks out the branch at that commit.
                    let id = self.r.commits[row].id;
                    if let Some(b) =
                        self.r.refs.refs.iter().find(|r| r.kind == RefKind::Local && r.target == id && !r.is_head)
                    {
                        let name = b.name.to_string();
                        self.run_refop(crate::work::RefOp::Checkout(name));
                    }
                }
            }
            return;
        }
        let fr = self.files_rect();
        if fr.contains(x, y) {
            let i = self.r.file_top + ((y - fr.y) / self.px(26.0)) as usize;
            self.focus = Focus::Files;
            self.damage.rows.push(self.r.sel);
            self.select_file(i);
            self.damage.rects.push(self.lay.details);
            return;
        }
        if self.lay.side.contains(x, y) {
            self.side_click(x, y);
            return;
        }
        if self.search_rect.contains(x, y) {
            self.open_palette();
        }
    }

    // -------------------------------------------------------------- palette

    pub fn open_palette(&mut self) {
        self.palette = Some(Palette { input: TextField::default(), items: Vec::new(), sel: 0, mode: PMode::Jump });
        self.filter_palette();
    }

    pub fn open_palette_with(&mut self, q: &str) {
        self.open_palette();
        if let Some(p) = &mut self.palette {
            p.input = TextField::new(q);
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
        let q = p.input.text().to_lowercase();
        let mut scored: Vec<(i32, PItem)> = Vec::new();
        if p.mode == PMode::Repos {
            // Recent repositories rank first; open ones are marked.
            for (i, path) in self.known_repos.iter().enumerate() {
                let name = repo_name(path);
                let full = path.to_string_lossy().into_owned();
                let Some(sc) = fuzzy(&q, &name).or_else(|| fuzzy(&q, &full).map(|s| s - 10)) else { continue };
                let open = self.tabs.iter().any(|t| same_path(&t.path, path));
                scored.push((
                    sc + 30 - (i as i32).min(30),
                    PItem {
                        label: name,
                        hint: if open { format!("open · {full}") } else { full },
                        kind: RefKind::Local,
                        action: PAction::OpenRepo(path.clone()),
                    },
                ));
            }
            // A typed folder path opens directly (checked when opened, so no
            // file system call happens per key press).
            let raw = p.input.text().trim().to_string();
            let raw = raw.as_str();
            if raw.contains(['\\', '/']) || raw.contains(':') {
                scored.push((
                    1000,
                    PItem {
                        label: format!("Open {raw}"),
                        hint: "folder".into(),
                        kind: RefKind::Head,
                        action: PAction::OpenRepo(PathBuf::from(raw)),
                    },
                ));
            }
            scored.sort_by_key(|x| std::cmp::Reverse(x.0));
            p.items = scored.into_iter().take(10).map(|x| x.1).collect();
            p.sel = 0;
            let r = self.palette_rect(10);
            self.damage.rects.push(r);
            return;
        }
        for r in &self.r.refs.refs {
            if let Some(sc) = fuzzy(&q, &r.name) {
                let hint = match r.kind {
                    RefKind::Local => "branch",
                    RefKind::Remote => "remote branch",
                    RefKind::Tag => "tag",
                    RefKind::Stash => "stash",
                    _ => "",
                };
                scored.push((
                    sc + if r.is_head { 5 } else { 0 },
                    PItem {
                        label: r.name.to_string(),
                        hint: hint.into(),
                        kind: r.kind,
                        action: PAction::GotoRef(r.target, r.name.to_string(), r.kind),
                    },
                ));
            }
        }
        for (c, keys) in COMMANDS {
            if let Some(sc) = fuzzy(&q, c) {
                scored.push((
                    sc - if WORKING.contains(c) { 2 } else { 6 },
                    PItem {
                        label: c.to_string(),
                        hint: if WORKING.contains(c) { keys.to_string() } else { "planned".into() },
                        kind: RefKind::Head,
                        action: PAction::Command(c),
                    },
                ));
            }
        }
        if self.update_available && fuzzy(&q, "Install update").is_some() {
            scored.push((
                70,
                PItem {
                    label: "Install update".into(),
                    hint: "Ctrl Shift U".into(),
                    kind: RefKind::Head,
                    action: PAction::InstallUpdate,
                },
            ));
        }
        if q.len() >= 4 && q.bytes().all(|b| b.is_ascii_hexdigit()) {
            for c in self.r.commits.iter().filter(|c| c.id.to_string().starts_with(&q)).take(3) {
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

    /// Stash the working changes: a message and whether untracked files go too.
    pub fn open_stash_dialog(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.open_dialog(DialogKind::Name { act: NameAct::StashSave, field: TextField::default(), checkout: true });
    }

    /// Interactive rebase from the selected commit (palette, Ctrl+Shift+R).
    fn rebase_from_selection(&mut self) {
        match self.r.commits.get(self.r.sel).map(|c| c.id.to_string()) {
            Some(sha) => self.open_rebase(sha),
            None => self.set_notice("Select a commit first.".into()),
        }
    }

    /// Logs a finished fetch, pull or push (FF-48).
    fn log_sync(&mut self, out: &ffwd_core::ops::Outcome) {
        let Some(g) = self.r.git.clone() else { return };
        let ran = vec![ffwd_core::git::Ran {
            args: out.command.strip_prefix("git ").unwrap_or(&out.command).to_string(),
            code: out.ok.then_some(0),
            stderr: out.output.chars().take(16 * 1024).collect(),
            ms: out.ms as u32,
        }];
        let e = ActivityEntry::new(&g, out.op.name(), out.ok, out.summary.clone(), out.ms as u32, ran);
        self.on_activity(e);
    }

    /// Where new clones and repositories go: the first repository root,
    /// else the current repository's parent folder, else the home folder.
    pub(super) fn default_repo_folder(&self) -> PathBuf {
        let root = self
            .settings
            .get("repos.roots")
            .and_then(|s| s.split(';').map(str::trim).find(|x| !x.is_empty()).map(PathBuf::from));
        root.or_else(|| self.tabs.get(self.cur_tab).and_then(|t| t.path.parent().map(Path::to_path_buf)))
            .or_else(|| std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from))
            .unwrap_or_default()
    }

    fn start_clone(&mut self, url: String, dest: PathBuf) {
        if self.clone_cancel.is_some() {
            return self.set_notice("A clone is already running (the palette cancels it).".into());
        }
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.clone_cancel = Some(cancel.clone());
        self.set_notice(format!("Cloning {url}…"));
        let sink = (self.sinks)(0);
        crate::work::spawn("clone", &sink, move |sink| {
            let s2 = sink.clone();
            let r = ffwd_core::clone::clone(&url, &dest, &cancel, |l| s2(Msg::CloneProgress(l)));
            sink(Msg::RepoReady(r));
        });
    }

    fn start_init(&mut self, dir: PathBuf) {
        let sink = (self.sinks)(0);
        crate::work::spawn("init", &sink, move |sink| sink(Msg::RepoReady(ffwd_core::clone::init(&dir))));
    }

    fn on_repo_ready(&mut self, r: Result<PathBuf, String>) {
        self.clone_cancel = None;
        match r {
            Ok(p) => {
                self.set_notice(format!("Opened {}", p.display()));
                self.open_tab(p);
            }
            Err(e) => self.set_notice(format!("Failed: {e}")),
        }
    }

    /// Renames tab `i` (an empty name goes back to the folder's).
    pub(super) fn rename_tab(&mut self, i: usize, name: &str) {
        if let Some(t) = self.tabs.get_mut(i) {
            let n = name.trim();
            t.label = (!n.is_empty() && n != t.name).then(|| n.to_string());
            self.save_session();
            self.damage.rects.push(self.lay.tabs);
            self.title_dirty = true;
        }
    }

    pub(super) fn set_tab_color(&mut self, i: usize, color: Option<u8>) {
        if let Some(t) = self.tabs.get_mut(i) {
            t.color = color;
            self.save_session();
            self.damage.rects.push(self.lay.tabs);
        }
    }

    /// Closes every tab but `keep`.
    pub(super) fn close_other_tabs(&mut self, keep: usize) {
        let n = self.tabs.len();
        for i in (0..n).rev() {
            if i != keep {
                self.close_tab(i);
            }
        }
    }

    /// Stashes as rows of the history (setting `history.stashes`, default
    /// on; FF-33).
    pub(super) fn show_stashes(&self) -> bool {
        self.settings.get("history.stashes") != Some("false")
    }

    pub(super) fn set_show_stashes(&mut self, on: bool) {
        self.settings.set_string("history.stashes", if on { "true" } else { "false" });
        self.set_notice(if on {
            "Stashes show in the history".into()
        } else {
            "Stashes are hidden in the history".into()
        });
        self.reload_current();
    }

    /// The history title's menu: all branches or the current one, and
    /// whether stashes show (FF-33).
    pub(super) fn scope_menu(&self) -> Vec<menu::MenuItem> {
        let head_only = self.tabs.get(self.cur_tab).is_some_and(|t| t.head_only);
        let mark = |on: bool, t: &str| if on { format!("✓ {t}") } else { t.to_string() };
        let stashes = self.show_stashes();
        vec![
            menu::MenuItem {
                label: mark(!head_only, "All commits"),
                act: Some(menu::MenuAct::Scope(false)),
                danger: false,
            },
            menu::MenuItem {
                label: mark(head_only, "Current branch only"),
                act: Some(menu::MenuAct::Scope(true)),
                danger: false,
            },
            menu::MenuItem { label: String::new(), act: None, danger: false },
            menu::MenuItem {
                label: mark(stashes, "Show stashes"),
                act: Some(menu::MenuAct::ShowStashes(!stashes)),
                danger: false,
            },
        ]
    }

    /// Minutes between automatic fetches (setting `remotes.auto_fetch`,
    /// default 10; 0 or "off" turns it off; FF-39).
    pub(super) fn auto_fetch_setting(&self) -> u64 {
        match self.settings.get("remotes.auto_fetch") {
            None => 10,
            Some(v) => v.trim().parse().unwrap_or(0).min(24 * 60),
        }
    }

    /// Starts the clock thread once. It sleeps the whole interval, so an
    /// idle window gets no wakeups in between (rule 5); never in tests.
    pub(super) fn start_auto_fetch_clock(&mut self) {
        use std::sync::atomic::Ordering;
        self.auto_fetch_minutes.store(self.auto_fetch_setting(), Ordering::Relaxed);
        if cfg!(test) || self.settings.get("hosting.enabled") == Some("false") {
            return;
        }
        let minutes = self.auto_fetch_minutes.clone();
        let sink = (self.sinks)(0);
        let _ = std::thread::Builder::new().name("auto-fetch".into()).spawn(move || {
            loop {
                let m = minutes.load(Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_secs(if m == 0 { 60 } else { m * 60 }));
                if m > 0 && minutes.load(Ordering::Relaxed) > 0 {
                    sink(Msg::AutoFetchTick);
                }
            }
        });
    }

    /// The clock says a fetch is due: fetch the current repository when
    /// it has a remote and nothing else runs.
    pub(super) fn auto_fetch_due(&mut self) {
        let busy = self.r.op.is_some() || self.dialog.is_some() || self.r.rebase.is_some();
        if busy || self.auto_fetch_setting() == 0 || self.r.git.is_none() || self.r.refs.remotes.is_empty() {
            return;
        }
        self.start_op(Op::AutoFetch);
    }

    /// An automatic fetch ended: news reload the repository and say so; a
    /// failure shows once until a fetch works again.
    fn auto_fetch_done(&mut self, out: &ffwd_core::ops::Outcome) {
        self.damage.rects.push(self.lay.top);
        if !out.ok {
            if !out.cancelled && !self.auto_fetch_failed {
                self.auto_fetch_failed = true;
                self.set_notice(format!(
                    "Automatic fetch: {} (it tries again later; Settings turns it off)",
                    out.summary
                ));
            }
            return;
        }
        self.auto_fetch_failed = false;
        if out.summary != "Fetched: everything is up to date" {
            self.set_notice(format!("{} (automatic fetch)", out.summary));
            self.reload_current();
        }
    }

    /// Shows only the current branch's history, or all again.
    pub(super) fn toggle_scope(&mut self) {
        let Some(t) = self.tabs.get_mut(self.cur_tab) else { return };
        t.head_only = !t.head_only;
        let on = t.head_only;
        self.set_notice(if on { "Showing the current branch only".into() } else { "Showing all branches".into() });
        // An open search stays and runs again over the new scope.
        let search = self.r.search.take();
        self.reload_current();
        if let Some(s) = search {
            let run = !s.input.text().trim().is_empty();
            self.r.search = Some(s);
            if run {
                self.restart_search();
            }
        }
    }

    /// Whether ffwd asks the hosting site (setting `hosting.enabled`,
    /// default on; tests and snapshots turn it off).
    fn hosting_enabled(&self) -> bool {
        // Tests never reach the network unless they ask for it.
        if cfg!(test) {
            return self.settings.get("hosting.enabled") == Some("true");
        }
        self.settings.get("hosting.enabled") != Some("false") && std::env::var_os("FFWD_NO_HOSTING").is_none()
    }

    /// Loads the selected image file's old and new version on a worker.
    fn load_diff_images(&mut self, seq: u64) {
        let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) else { return };
        let Some(c) = self.r.commits.get(self.r.sel) else { return };
        let Some(f) = self.r.details.as_ref().and_then(|d| d.files.get(self.r.sel_file)).cloned() else { return };
        let sha = c.id.to_string();
        let parent = c.parents.first().map(|p| p.to_string());
        self.r.diff_images = None;
        let sink = (self.sinks)(t.id);
        crate::work::spawn("diff-images", &sink, move |sink| {
            let old_path = f.old_path.as_deref().unwrap_or(&f.path).to_string();
            let old = match (&parent, f.status) {
                (Some(p), s) if s != b'A' => Some(crate::image::load(&g, p, &old_path)),
                _ => None,
            };
            let new = (f.status != b'D').then(|| crate::image::load(&g, &sha, &f.path));
            sink(Msg::DiffImages { seq, old, new });
        });
    }

    /// The old and new image side by side, each fitted into its half on a
    /// checkerboard, with its size.
    fn paint_diff_images(&mut self, cv: &mut Canvas, body: Rect) {
        let ui = self.f.ui;
        // Only the current diff's images; anything else is still loading.
        let Some((_, old, new)) = self.r.diff_images.clone().filter(|x| x.0 == self.r.seq) else {
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                body.x as f32 + self.pf(16.0),
                body.y as f32 + self.pf(26.0),
                "Loading the images…",
                TEXT_3,
                body.w as f32,
            );
            return;
        };
        let pad = self.px(14.0);
        let half = (body.w - pad * 3) / 2;
        for (k, (label, img)) in [("Before", old), ("After", new)].into_iter().enumerate() {
            let x = body.x + pad + k as i32 * (half + pad);
            let cap_y = body.y as f32 + self.pf(24.0);
            let caption = match &img {
                None if k == 0 => "Before: the file is new".to_string(),
                None => "After: the file was deleted".to_string(),
                Some(Err(e)) => format!("{label}: {e}"),
                Some(Ok(i)) => format!("{label} · {}×{} · {}", i.orig.0, i.orig.1, fmt_bytes(i.bytes)),
            };
            self.te.draw(cv, ui, self.pf(12.0), x as f32, cap_y, &caption, TEXT_3, half as f32);
            let area = Rect::new(x, body.y + self.px(36.0), half, body.h - self.px(48.0));
            if let Some(Ok(i)) = img {
                // Fit, never enlarge beyond 2x.
                let s = (area.w as f32 / i.w as f32).min(area.h as f32 / i.h as f32).min(2.0);
                let (w, h) = (((i.w as f32 * s) as i32).max(1), ((i.h as f32 * s) as i32).max(1));
                let r = Rect::new(area.x + (area.w - w) / 2, area.y, w, h);
                // A checkerboard shows transparency.
                let c = self.px(8.0).max(2);
                let old_clip = cv.set_clip(cv.clip().intersect(r));
                let mut yy = r.y;
                while yy < r.bottom() {
                    let mut xx = r.x;
                    while xx < r.right() {
                        let dark = ((xx - r.x) / c + (yy - r.y) / c) % 2 == 0;
                        cv.fill(Rect::new(xx, yy, c, c), if dark { BG_INPUT } else { BG_HOVER });
                        xx += c;
                    }
                    yy += c;
                }
                cv.draw_rgba_scaled(r, i.w, i.h, &i.rgba);
                cv.set_clip(old_clip);
                cv.stroke_rect(r, BORDER);
            }
        }
    }

    /// A command for the selected commit (palette or shortcut).
    fn selected_commit_command(&mut self, c: &str) {
        let Some(sha) = self.r.commits.get(self.r.sel).map(|c| c.id.to_string()) else {
            return self.set_notice("Select a commit first.".into());
        };
        match c {
            CMD_BRANCH => self.open_dialog(DialogKind::Name {
                act: NameAct::CreateBranch { at: sha },
                field: TextField::default(),
                checkout: true,
            }),
            CMD_TAG => self.open_dialog(DialogKind::Name {
                act: NameAct::CreateTag { at: sha },
                field: TextField::default(),
                checkout: false,
            }),
            CMD_PICK => self.run_refop(crate::work::RefOp::CherryPick(sha)),
            _ => self.run_refop(crate::work::RefOp::Revert(sha)),
        }
    }

    fn run_command(&mut self, c: &str) {
        match c {
            CMD_FETCH => self.start_op(Op::Fetch),
            CMD_PULL => self.start_op(Op::Pull),
            CMD_PUSH => self.start_op(Op::Push),
            CMD_OPEN => self.open_repo_picker(),
            CMD_CLOSE => self.close_tab(self.cur_tab),
            CMD_REOPEN => self.reopen_closed_tab(),
            CMD_UPDATES => self.request_update_check(),
            CMD_SETTINGS => self.open_settings(),
            CMD_AGENTS => self.open_agents(),
            CMD_ACTIONS => self.open_menu_for_selection(),
            CMD_SEARCH => self.open_search(),
            CMD_BRANCH | CMD_TAG | CMD_PICK | CMD_REVERT => self.selected_commit_command(c),
            CMD_STASH => self.open_stash_dialog(),
            CMD_PATCH_UNSTAGED => self.make_patch(PatchSrc::Working(false), None),
            CMD_PATCH_STAGED => self.make_patch(PatchSrc::Working(true), None),
            CMD_APPLY_CLIP => match crate::platform::clipboard() {
                Some(p) if !p.trim().is_empty() => self.check_patch(p),
                _ => self.set_notice("The clipboard holds no text.".into()),
            },
            CMD_APPLY_FILE => self.open_dialog(DialogKind::Name {
                act: NameAct::ApplyPatchFile,
                field: TextField::default(),
                checkout: false,
            }),
            CMD_CREATE_PR => self.create_pr(),
            CMD_REFLOG => self.enter_reflog(),
            CMD_AI_COMMIT => self.ai_draft(),
            CMD_CLONE => self.open_dialog(DialogKind::Name {
                act: NameAct::CloneUrl,
                field: TextField::default(),
                checkout: false,
            }),
            CMD_INIT => {
                let dir = self.default_repo_folder().join("new-repository");
                self.open_dialog(DialogKind::Name {
                    act: NameAct::InitDir,
                    field: TextField::new(&dir.to_string_lossy()),
                    checkout: false,
                })
            }
            CMD_CANCEL_CLONE => match &self.clone_cancel {
                Some(c) => {
                    c.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.set_notice("Cancelling the clone…".into());
                }
                None => self.set_notice("No clone is running.".into()),
            },
            CMD_SCOPE => self.toggle_scope(),
            CMD_ADD_SUBMODULE => self.open_dialog(DialogKind::Name {
                act: NameAct::AddSubmoduleUrl,
                field: TextField::default(),
                checkout: false,
            }),
            CMD_ACTIVITY => self.enter_activity(),
            CMD_REBASE_I => self.rebase_from_selection(),
            CMD_ADD_REMOTE => self.open_dialog(DialogKind::Name {
                act: NameAct::AddRemoteUrl,
                field: TextField::default(),
                checkout: false,
            }),
            CMD_CANCEL_OP if self.r.op.is_some() => self.cancel_op(),
            CMD_CANCEL_OP => self.set_notice("Nothing is running.".into()),
            LAYOUT_BELOW => self.set_layout(DetailsLayout::Below),
            LAYOUT_BESIDE => self.set_layout(DetailsLayout::Beside),
            c if c.starts_with("AI:") || c.starts_with("Settings: AI") => self.set_notice(
                "AI needs an endpoint: set [ai] endpoint in settings.toml (Ollama, LM Studio, OpenAI-compatible). The AI features come next (FF-12).".into(),
            ),
            c => self.set_notice(format!("“{c}” is planned for the daily-driver milestone (FF-12) and not built yet.")),
        }
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
                p.input.backspace();
                self.filter_palette();
            }
            Key::CtrlBackspace => {
                p.input.delete_word();
                self.filter_palette();
            }
            Key::Delete => {
                p.input.delete();
                self.filter_palette();
            }
            Key::Char(c) => {
                p.input.insert(c.encode_utf8(&mut [0; 4]));
                self.filter_palette();
            }
            Key::Left | Key::Right | Key::Home | Key::End | Key::Ctrl('a') => {
                match k {
                    Key::Left => p.input.left(),
                    Key::Right => p.input.right(),
                    Key::Home => p.input.home(),
                    Key::End => p.input.end(),
                    _ => p.input.select_all(),
                }
                let r = self.palette_rect(10);
                self.damage.rects.push(r);
            }
            Key::Ctrl('c') => crate::platform::set_clipboard(p.input.copy_text()),
            Key::Ctrl('x') => {
                let t = p.input.cut();
                crate::platform::set_clipboard(&t);
                self.filter_palette();
            }
            Key::Ctrl('v') => {
                if let Some(t) = crate::platform::clipboard() {
                    p.input.insert(&t);
                    self.filter_palette();
                }
            }
            Key::Enter => {
                let action = p.items.get(p.sel).map(|i| i.action.clone());
                self.close_palette();
                match action {
                    Some(PAction::Goto(_) | PAction::GotoRef(..))
                        if self.r.changes_view || self.r.files_view || self.r.activity_view || self.r.reflog_view =>
                    {
                        // A jump shows history again.
                        self.leave_reflog();
                        self.leave_activity();
                        self.leave_changes();
                        self.leave_files();
                        if let Some(PAction::Goto(id) | PAction::GotoRef(id, ..)) = action
                            && let Some(&row) = self.r.by_id.get(&id)
                        {
                            self.select(row as usize);
                        }
                    }
                    Some(PAction::Goto(id)) => match self.r.by_id.get(&id) {
                        Some(&row) => self.select(row as usize),
                        None => self.set_notice("That commit is not loaded yet.".into()),
                    },
                    Some(PAction::GotoRef(id, name, kind)) => {
                        self.side_select_ref(&name, kind);
                        match self.r.by_id.get(&id) {
                            Some(&row) => self.select(row as usize),
                            None => self.set_notice("That commit is not loaded yet.".into()),
                        }
                    }
                    Some(PAction::Command(c)) => self.run_command(c),
                    Some(PAction::InstallUpdate) => self.request_update_install(),
                    Some(PAction::OpenRepo(path)) => self.open_tab(path),
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
                        (self.r.top + (rows - d) as usize..self.r.top + rows as usize + 1).collect()
                    } else {
                        (self.r.top..self.r.top + (-d) as usize).collect()
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
        if !clip.intersect(l.tabs).is_empty() {
            self.paint_tabs(cv);
        }
        if !clip.intersect(l.side).is_empty() {
            self.paint_side(cv);
        }
        if self.r.rebase.is_some() {
            if !clip.intersect(self.rebase_area()).is_empty() {
                self.paint_rebase(cv);
            }
        } else if self.r.reflog_view {
            if !clip.intersect(self.reflog_area()).is_empty() {
                self.paint_reflog(cv);
            }
        } else if self.r.activity_view {
            if !clip.intersect(self.activity_layout().area).is_empty() {
                self.paint_activity(cv);
            }
        } else if self.r.files_view {
            if !clip.intersect(self.files_layout().area).is_empty() {
                self.paint_files(cv);
            }
        } else if self.r.changes_view {
            if !clip.intersect(self.changes_layout().area).is_empty() {
                self.paint_changes(cv);
            }
        } else {
            if !clip.intersect(l.list).is_empty() {
                self.paint_list(cv);
            }
            if !clip.intersect(l.details).is_empty() {
                self.paint_details(cv);
            }
        }
        if !clip.intersect(l.status).is_empty() {
            self.paint_status(cv);
        }
        if self.palette.is_some() && !clip.intersect(self.palette_rect(10)).is_empty() {
            self.paint_palette(cv);
        }
        // A modal panel dims what is behind it, so the eye goes to the
        // panel. Remote sessions get no translucency (performance rule 6).
        // Settings is a page of its own: nothing behind it to dim.
        if (self.agent_view.is_some() || self.dialog.is_some()) && !self.remote {
            cv.fill_alpha(clip, 0x000000, 100);
        }
        if self.agent_view.is_some() && !clip.intersect(self.agents_rect()).is_empty() {
            self.paint_agents(cv);
        }
        if self.settings_view.is_some() && !clip.intersect(self.settings_rect()).is_empty() {
            self.paint_settings(cv);
        }
        if self.menu.is_some() && !clip.intersect(self.menu_rect()).is_empty() {
            self.paint_menu(cv);
        }
        if self.dialog.is_some() && !clip.intersect(self.dialog_rect()).is_empty() {
            self.paint_dialog(cv);
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
        // App mark: the logo itself (icon.rs), the same as the window icon.
        // macOS keeps its traffic lights at the left of the bar.
        let left = if self.chrome == Chrome::MacInset { self.pf(80.0) } else { self.pf(14.0) };
        let mut controls = Vec::new();
        let mark = Rect::new(left as i32, (cy - self.pf(13.0)) as i32, self.px(26.0), self.px(26.0));
        crate::icon::draw_mark(cv, mark);
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let base = (cy + self.pf(5.0)).round();
        let mut buttons = Vec::new();
        let mut x = mark.right() as f32 + self.pf(12.0);
        let name_x = x;
        if self.is_hot(Hot::Top(TopAction::Repo)) {
            // The previous paint's hit area (the name's width is known then).
            if let Some(&(nr, _)) = self.top_buttons.iter().find(|(_, a)| *a == TopAction::Repo) {
                let (bg, _) = self.control_colors(Hot::Top(TopAction::Repo), BG_BAR, BORDER);
                cv.fill_round(nr.inset(-self.px(4.0), 0), self.pf(8.0), bg);
                crate::icon::draw_mark(cv, mark);
            }
        }
        x = self.te.draw(cv, semi, self.pf(14.5), x, base, &self.r.repo_name.clone(), TEXT, self.pf(220.0));
        // Clicking the name (or the mark) opens another repository.
        let name_r =
            Rect::new(mark.x, (cy - self.pf(14.0)) as i32, (x - mark.x as f32) as i32 + self.px(4.0), self.px(28.0));
        let _ = name_x;
        buttons.push((name_r, TopAction::Repo));
        controls.push((name_r.x, name_r.right()));
        x += self.pf(12.0);
        // Current branch pill.
        let branch = self
            .r
            .refs
            .head_branch
            .as_deref()
            .unwrap_or(if self.r.git.is_some() { "detached" } else { "…" })
            .to_string();
        let bw = self.te.measure(semi, self.pf(12.5), &branch) + self.pf(36.0);
        let pill = Rect::new(x as i32, (cy - self.pf(13.0)) as i32, bw as i32, self.px(26.0));
        let (pbg, pborder) = self.control_colors(Hot::Top(TopAction::Branch), BG_INPUT, BORDER);
        cv.round_border(pill, self.pf(13.0), pborder, pbg);
        controls.push((pill.x, pill.right()));
        buttons.push((pill, TopAction::Branch));
        self.branch_icon(cv, pill.x as f32 + self.pf(14.0), cy, TEXT_2);
        self.te.draw(cv, semi, self.pf(12.5), pill.x as f32 + self.pf(25.0), base - self.pf(0.5), &branch, TEXT, bw);
        // Actions with ahead/behind from the current branch.
        let head = self.r.refs.refs.iter().find(|r| r.is_head);
        let (ahead, behind) = head.map_or((0, 0), |h| (h.ahead, h.behind));
        let right = if self.lay.caption.is_empty() { r.right() } else { self.lay.caption.x };
        // Gear: settings.
        let gs = self.px(30.0);
        let gear = Rect::new((right as f32 - self.pf(10.0)) as i32 - gs, (cy - self.pf(15.0)) as i32, gs, gs);
        let (gbg, gborder) = self.control_colors(Hot::Top(TopAction::Settings), BG_BAR, BORDER);
        cv.round_border(gear, self.pf(8.0), gborder, gbg);
        let gc = if self.is_hot(Hot::Top(TopAction::Settings)) { TEXT } else { TEXT_2 };
        self.gear_icon(cv, gear.x as f32 + gs as f32 / 2.0, cy, gc);
        buttons.push((gear, TopAction::Settings));
        let right = gear.x;
        let mut bx = right as f32 - self.pf(8.0);
        let running = self.r.op;
        let busy = running.map(Op::button);
        let hot = self.hot;
        let label = |op: Op, idle: &'static str| match busy {
            // Hovering the running operation's button offers to cancel it.
            Some(b) if b == op && hot == Some(Hot::Top(op_action(op))) => "Cancel".to_string(),
            Some(b) if b == op => format!("{}…", running.unwrap_or(op).verb()),
            _ => idle.to_string(),
        };
        let (row_y, row_h) = ((cy - self.pf(14.0)) as i32, self.px(28.0));
        let row = move |l: f32, r: f32| Rect::new(l as i32, row_y, (r - l) as i32, row_h);
        let ai = self.button(cv, bx, cy, "AI", None, true, TopAction::Ai, None);
        buttons.push((row(ai, bx), TopAction::Ai));
        bx = ai - self.pf(8.0);
        let push = self.button(
            cv,
            bx,
            cy,
            &label(Op::Push, "Push"),
            (ahead > 0).then(|| format!("↑{ahead}")),
            false,
            TopAction::Push,
            Some(Glyph::Push),
        );
        buttons.push((row(push, bx), TopAction::Push));
        bx = push - self.pf(8.0);
        let pull = self.button(
            cv,
            bx,
            cy,
            &label(Op::Pull, "Pull"),
            (behind > 0).then(|| format!("↓{behind}")),
            false,
            TopAction::Pull,
            Some(Glyph::Pull),
        );
        buttons.push((row(pull, bx), TopAction::Pull));
        bx = pull - self.pf(8.0);
        let fetch =
            self.button(cv, bx, cy, &label(Op::Fetch, "Fetch"), None, false, TopAction::Fetch, Some(Glyph::Fetch));
        buttons.push((row(fetch, bx), TopAction::Fetch));
        bx = fetch - self.pf(8.0);
        // Stash: the working changes, with a message (also Ctrl+Shift+S).
        let stash = self.button(cv, bx, cy, "Stash", None, false, TopAction::Stash, Some(Glyph::Stash));
        buttons.push((row(stash, bx), TopAction::Stash));
        // Stash to the gear's right edge: the gear is a control too, not
        // part of the title bar's drag area (FF-29).
        controls.push((stash as i32, gear.right()));
        // Search / command field.
        // Between the branch pill and the toolbar buttons, centred where it fits.
        let (lo, hi) = (pill.right() + self.px(16.0), stash as i32 - self.px(12.0));
        let sw = self.px(460.0).min(hi - lo).max(self.px(160.0));
        let sx0 = ((r.w - sw) / 2).min(hi - sw).max(lo);
        let search = Rect::new(sx0, (cy - self.pf(15.0)) as i32, sw, self.px(30.0));
        let sborder = if self.is_hot(Hot::Search) { BORDER_HOVER } else { BORDER };
        cv.round_border(search, self.pf(8.0), sborder, BG_INPUT);
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
        self.title_controls = controls;
        self.top_buttons = buttons;
        let _ = mono;
        self.paint_caption(cv);
    }

    /// Tab rectangles and the "+" button, left to right, sharing the row.
    fn tab_rects(&self) -> (Vec<Rect>, Rect) {
        let r = self.lay.tabs;
        let plus_w = self.px(36.0);
        let n = self.tabs.len().max(1) as i32;
        let avail = (r.w - plus_w - self.px(8.0)).max(0);
        let tw = (avail / n).clamp(self.px(80.0), self.px(240.0));
        let rects: Vec<Rect> = (0..self.tabs.len() as i32).map(|i| Rect::new(r.x + i * tw, r.y, tw, r.h - 1)).collect();
        let px = rects.last().map_or(r.x, |l| l.right()) + self.px(2.0);
        (rects, Rect::new(px, r.y, plus_w, r.h - 1))
    }

    fn tab_close_rect(&self, t: Rect) -> Rect {
        let s = self.px(22.0);
        Rect::new(t.right() - s - self.px(6.0), t.y + (t.h - s) / 2, s, s)
    }

    pub(super) fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.lay.tabs.contains(x, y) {
            return None;
        }
        self.tab_rects().0.iter().position(|r| r.contains(x, y))
    }

    fn paint_tabs(&mut self, cv: &mut Canvas) {
        let r = self.lay.tabs;
        cv.fill(r, BG_SIDEBAR);
        cv.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1), BORDER);
        let (rects, plus) = self.tab_rects();
        let (ui, semi) = (self.f.ui, self.f.semi);
        for (i, t) in rects.iter().enumerate() {
            if t.x >= r.right() {
                break;
            }
            let active = i == self.cur_tab;
            let hover = self.tab_hover == Some(i);
            let bg = if active {
                BG
            } else if hover {
                BG_HOVER
            } else {
                BG_SIDEBAR
            };
            cv.fill(Rect::new(t.x, t.y, t.w, t.h + if active { 1 } else { 0 }), bg);
            if active {
                cv.fill(Rect::new(t.x, t.y, t.w, self.px(2.0)), ACCENT);
            }
            // Separator between inactive tabs.
            if !active && i + 1 != self.cur_tab {
                cv.fill(Rect::new(t.right() - 1, t.y + self.px(8.0), 1, t.h - self.px(16.0)), BORDER);
            }
            let name = self.tabs[i].title().to_string();
            // The tab's colour: the top bar when active, a thin line otherwise.
            if let Some(c) = self.tabs[i].color.and_then(|c| TAB_COLORS.get(c as usize)) {
                let h = if active { self.px(3.0) } else { self.px(2.0) };
                cv.fill(Rect::new(t.x, t.y, t.w, h), c.1);
            }
            let base = (t.y as f32 + t.h as f32 / 2.0 + self.pf(4.5)).round();
            let close_w = if active || hover { self.pf(28.0) } else { 0.0 };
            let font = if active { semi } else { ui };
            let max = t.w as f32 - self.pf(28.0) - close_w;
            let tw = self.te.measure(font, self.pf(12.5), &name).min(max);
            // Centred.
            let tx = t.x as f32 + ((t.w as f32 - close_w - tw) / 2.0).max(self.pf(14.0));
            let end = self.te.draw(cv, font, self.pf(12.5), tx, base, &name, if active { TEXT } else { TEXT_3 }, max);
            // An agent runs (accent) or finished unseen (green, red).
            if let Some(c) = self.agent_badge(&self.tabs[i].path.clone()) {
                cv.fill_circle(end + self.pf(7.0), base - self.pf(4.0), self.pf(3.0), c);
            } else {
                // Uncommitted changes (FF-35).
                let st = if active { self.r.status } else { self.tabs[i].state.as_ref().and_then(|s| s.status) };
                if st.is_some_and(|s| s.total() > 0) {
                    cv.ring(end + self.pf(7.0), base - self.pf(4.0), self.pf(3.0), self.pf(1.4), MOD);
                }
            }
            if active || hover {
                let c = self.tab_close_rect(*t);
                let (cx, cy, s) = (c.x as f32 + c.w as f32 / 2.0, c.y as f32 + c.h as f32 / 2.0, self.pf(3.5));
                let w = self.pf(1.3);
                cv.stroke(&[(cx - s, cy - s), (cx + s, cy + s)], w, TEXT_3);
                cv.stroke(&[(cx - s, cy + s), (cx + s, cy - s)], w, TEXT_3);
            }
        }
        // "+" opens another repository.
        let (cx, cy, s) = (plus.x as f32 + plus.w as f32 / 2.0, plus.y as f32 + plus.h as f32 / 2.0, self.pf(5.0));
        cv.stroke(&[(cx - s, cy), (cx + s, cy)], self.pf(1.5), TEXT_2);
        cv.stroke(&[(cx, cy - s), (cx, cy + s)], self.pf(1.5), TEXT_2);
    }

    /// Middle click closes the tab under the pointer, as in browsers.
    pub fn middle_click(&mut self, x: i32, y: i32) {
        if let Some(i) = self.tab_at(x, y) {
            self.close_tab(i);
        }
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
                (2, true, true) => (CLOSE_PRESS, 0xffffff),
                (2, true, false) => (CLOSE_HOVER, 0xffffff),
                (_, true, true) => (BG_PRESS, TEXT),
                (_, true, false) => (BG_HOVER, TEXT),
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
        self.update_hot(x, y);
        self.side_pointer(x, y);
        let th = self.tab_at(x, y);
        if th != self.tab_hover {
            self.tab_hover = th;
            self.damage.rects.push(self.lay.tabs);
        }
        let h = self.caption_button_at(x, y).filter(|&b| b != 1);
        if h != self.cap_hover {
            self.cap_hover = h;
            self.damage.rects.push(self.lay.caption);
        }
    }

    /// The window got the focus back: refresh what may have changed
    /// outside ffwd (the working tree).
    pub fn on_window_focus(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        if self.r.files_view {
            self.load_files_tree();
        }
        if self.r.changes_view {
            self.reload_changes();
        } else if let (Some(g), Some(t)) = (self.git_for_tab(), self.tabs.get(self.cur_tab)) {
            let sink = (self.sinks)(t.id);
            crate::work::spawn("status", &sink, move |sink| {
                if let Ok(s) = ffwd_core::refs::status(&g) {
                    sink(Msg::Status(s));
                }
            });
        }
    }

    pub fn pointer_left(&mut self) {
        self.clear_hot();
        if self.tab_hover.take().is_some() {
            self.damage.rects.push(self.lay.tabs);
        }
        if self.cap_hover.take().is_some() {
            self.damage.rects.push(self.lay.caption);
        }
    }

    /// Mouse button released: a caption button acts only if the press
    /// started on it and the pointer is still over it (as on Windows).
    pub fn release(&mut self, x: i32, y: i32) {
        self.release_press();
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
    #[allow(clippy::too_many_arguments)]
    fn button(
        &mut self,
        cv: &mut Canvas,
        right: f32,
        cy: f32,
        label: &str,
        badge: Option<String>,
        accent: bool,
        action: TopAction,
        glyph: Option<Glyph>,
    ) -> f32 {
        let semi = self.f.semi;
        let size = self.pf(12.5);
        let tw = self.te.measure(semi, size, label);
        let bw_badge = badge.as_ref().map_or(0.0, |b| self.te.measure(semi, self.pf(11.5), b) + self.pf(8.0));
        let icon = if accent || glyph.is_some() { self.pf(18.0) } else { 0.0 };
        let w = tw + bw_badge + icon + self.pf(24.0);
        let r = Rect::new((right - w) as i32, (cy - self.pf(14.0)) as i32, w as i32, self.px(28.0));
        let h = Hot::Top(action);
        if accent {
            let bg = if self.is_pressed(h) {
                ACCENT_SOFT
            } else if self.is_hot(h) {
                ACCENT_SOFT_HOVER
            } else {
                ACCENT_SOFT
            };
            let border = if self.is_hot(h) { ACCENT } else { ACCENT_BORDER };
            cv.round_border(r, self.pf(8.0), border, bg);
            self.sparkle(cv, r.x as f32 + self.pf(16.0), cy, self.pf(5.5), ACCENT);
        } else {
            let (bg, border) = self.control_colors(h, BG_BAR, BORDER);
            cv.round_border(r, self.pf(8.0), border, bg);
        }
        if let Some(g) = glyph {
            self.glyph(cv, g, r.x as f32 + self.pf(18.0), cy, self.pf(7.0), TEXT);
        }
        let base = (cy + self.pf(4.5)).round();
        let tx = r.x as f32 + self.pf(12.0) + icon;
        let end = self.te.draw(cv, semi, size, tx, base, label, if accent { ACCENT_FG } else { TEXT }, w);
        if let Some(b) = badge {
            self.te.draw(cv, semi, self.pf(11.5), end + self.pf(6.0), base, &b, ACCENT, w);
        }
        r.x as f32
    }

    fn gear_icon(&self, cv: &mut Canvas, cx: f32, cy: f32, color: u32) {
        self.glyph(cv, Glyph::Gear, cx, cy, self.pf(7.0), color);
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

    fn lane_w(&self) -> f32 {
        self.pf(14.0)
    }

    fn graph_lanes_shown(&self) -> u16 {
        self.r.graph.max_width.clamp(1, 10)
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
        if !clip.intersect(head).is_empty() && self.r.search.is_some() {
            self.paint_search_head(cv, head);
        } else if !clip.intersect(head).is_empty() {
            cv.fill(head, BG);
            cv.fill(Rect::new(head.x, head.bottom() - 1, head.w, 1), BORDER_SOFT);
            let base = (head.y as f32 + head.h as f32 / 2.0 + self.pf(4.0)).round();
            let n = if self.r.log_done {
                fmt::num(self.r.commits.len())
            } else {
                format!("{}…", fmt::num(self.r.commits.len()))
            };
            // The title switches all branches / the current branch.
            let head_only = self.tabs.get(self.cur_tab).is_some_and(|t| t.head_only);
            let title = if head_only { format!("Current branch  ·  {n}") } else { format!("All commits  ·  {n}") };
            let tw = self.te.measure(semi, self.pf(11.5), &title);
            let tr = Rect::new(
                l.x + self.px(6.0),
                head.y + self.px(4.0),
                (tw + self.pf(30.0)) as i32,
                head.h - self.px(8.0),
            );
            if self.is_hot(Hot::Scope) {
                cv.fill_round(tr, self.pf(6.0), BG_HOVER);
            }
            let tc = if head_only { ACCENT_FG } else { TEXT_3 };
            let e = self.te.draw(cv, semi, self.pf(11.5), l.x as f32 + self.pf(14.0), base, &title, tc, self.pf(260.0));
            let (cx, cy, s) = (e + self.pf(8.0), head.y as f32 + head.h as f32 / 2.0, self.pf(3.0));
            cv.stroke(&[(cx - s, cy - s / 2.0), (cx, cy + s / 2.0), (cx + s, cy - s / 2.0)], self.pf(1.3), tc);
            self.scope_rect = tr;
            // Find (Ctrl+F), clickable (FF-57).
            let fl = "Find";
            let fw = self.te.measure(semi, self.pf(11.5), fl) + self.pf(34.0);
            let fr = Rect::new(tr.right() + self.px(8.0), tr.y, fw as i32, tr.h);
            if self.is_hot(Hot::HeadFind) {
                cv.fill_round(fr, self.pf(6.0), BG_HOVER);
            }
            self.glyph(cv, Glyph::Search, fr.x as f32 + self.pf(12.0), cy, self.pf(6.0), TEXT_2);
            self.te.draw(cv, semi, self.pf(11.5), fr.x as f32 + self.pf(23.0), base, fl, TEXT_2, fw);
            self.head_find_rect = fr;
            for (x, t) in [(author_x, "Author"), (date_x, "Date"), (sha_x, "Commit")] {
                self.te.draw(cv, semi, self.pf(11.5), x, base, t, TEXT_3, self.pf(100.0));
            }
        }
        let body = Rect::new(l.x, self.lay.list_body.y, l.w - self.lay.scrollbar.w, self.lay.list_body.h);
        if !clip.intersect(body).is_empty() {
            let first = self.r.top + ((clip.y - body.y).max(0) / self.lay.row_h) as usize;
            let last = self.r.top + ((clip.bottom() - body.y).max(0) / self.lay.row_h) as usize;
            let mut y = body.y + (first - self.r.top) as i32 * self.lay.row_h;
            for row in first..=last {
                if y >= body.bottom() {
                    break;
                }
                let rr = Rect::new(body.x, y, body.w, self.lay.row_h);
                let old = cv.set_clip(clip.intersect(body).intersect(rr));
                self.paint_row(cv, row, rr, text_x, author_x, date_x, sha_x, ui, semi);
                // A search dims what doesn't match; the matches stay bright.
                if self.row_dimmed(row) {
                    cv.fill_alpha(rr, BG, 120);
                }
                cv.set_clip(old);
                y += self.lay.row_h;
            }
        }
        let sb = self.lay.scrollbar;
        if !clip.intersect(sb).is_empty() {
            cv.fill(sb, BG);
            let n = self.r.commits.len().max(1);
            let vis = self.visible_rows();
            if n > vis {
                let th = ((sb.h as f32 * vis as f32 / n as f32) as i32).max(self.px(24.0));
                let ty = sb.y + ((sb.h - th) as f32 * self.r.top as f32 / (n - vis) as f32) as i32;
                cv.fill_round(Rect::new(sb.x + self.px(3.0), ty, sb.w - self.px(6.0), th), self.pf(3.0), BORDER_HOVER);
            }
            self.paint_search_marks(cv, sb);
            // Selection marker on the scroll track.
            let my = sb.y + (sb.h as f32 * self.r.sel as f32 / n as f32) as i32;
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
        let selected = row == self.r.sel;
        let bg = if selected {
            if self.focus == Focus::List { BG_SELECT } else { BG_SELECT_DIM }
        } else if self.is_hot(Hot::Row(row)) {
            BG_HOVER
        } else {
            BG
        };
        cv.fill(r, bg);
        if row >= self.r.commits.len() {
            if !self.r.log_done && row < self.r.top + self.visible_rows() {
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
        let c = &self.r.commits[row];
        let base = (r.y as f32 + r.h as f32 / 2.0 + self.pf(4.5)).round();
        let mut x = text_x;
        // Ref chips.
        let chips: Vec<u32> = self.r.chips.get(&c.id).cloned().unwrap_or_default();
        let max_x = author_x - self.pf(16.0);
        for (k, &ri) in chips.iter().enumerate() {
            if k == 3 {
                let more = format!("+{}", chips.len() - 3);
                x = self.te.draw(cv, semi, self.pf(11.0), x, base, &more, TEXT_3, max_x - x) + self.pf(8.0);
                break;
            }
            let rf = &self.r.refs.refs[ri as usize];
            let (cbg, cfg) = match rf.kind {
                _ if rf.is_head => (ACCENT_SOFT_HOVER, ACCENT_FG),
                RefKind::Local => (LOCAL_BG, LOCAL_FG),
                RefKind::Remote => (REMOTE_BG, REMOTE_FG),
                RefKind::Tag => (TAG_BG, TAG_FG),
                _ => (STASH_BG, STASH_FG),
            };
            let name = rf.name.to_string();
            let glyph = match rf.kind {
                _ if rf.is_head => Glyph::Head,
                RefKind::Local => Glyph::Branch,
                RefKind::Remote => Glyph::Remote,
                RefKind::Tag => Glyph::Tag,
                _ => Glyph::Stash,
            };
            let tw = self.te.measure(semi, self.pf(11.0), &name).min(self.pf(160.0));
            let iw = self.pf(15.0);
            let cw = tw + iw + self.pf(14.0);
            if x + cw > max_x - self.pf(60.0) {
                break;
            }
            let chip =
                Rect::new(x as i32, (r.y as f32 + r.h as f32 / 2.0 - self.pf(9.5)) as i32, cw as i32, self.px(19.0));
            cv.fill_round(chip, self.pf(5.0), cbg);
            let cy = r.y as f32 + r.h as f32 / 2.0;
            self.glyph(cv, glyph, x + self.pf(13.0), cy, self.pf(5.5), cfg);
            self.te.draw(cv, semi, self.pf(11.0), x + self.pf(7.0) + iw, base - self.pf(0.5), &name, cfg, tw + 1.0);
            x += cw + self.pf(6.0);
        }
        // To push (↑) or to pull (↓).
        let mark = if self.r.ahead_ids.contains(&c.id) {
            Some(("↑", ACCENT_FG))
        } else if self.r.behind_ids.contains(&c.id) {
            Some(("↓", REMOTE_FG))
        } else {
            None
        };
        if let Some((m, color)) = mark {
            x = self.te.draw(cv, semi, self.pf(12.5), x, base, m, color, self.pf(14.0)) + self.pf(5.0);
        }
        let merge = c.parents.len() > 1;
        let subj_color = if merge { TEXT_2 } else { TEXT };
        // A conventional-commit prefix ("feat:", "fix(ui)!:") in the accent
        // colour, the rest as usual (FF-33).
        let (prefix, rest) = split_prefix(&c.subject);
        let x = if prefix.is_empty() {
            x
        } else {
            self.te.draw(cv, ui, self.pf(13.0), x, base, prefix, ACCENT_FG, max_x - x)
        };
        let end = self.te.draw(cv, ui, self.pf(13.0), x, base, rest, subj_color, max_x - x);
        if c.has_body && end + self.pf(16.0) < max_x {
            // "Has a description" marker.
            let mx = end + self.pf(9.0);
            for k in 0..3 {
                cv.fill_circle(mx + k as f32 * self.pf(3.5), base - self.pf(3.5), self.pf(1.1), TEXT_4);
            }
        }
        let dim = if selected { TEXT_2 } else { TEXT_3 };
        self.te.draw(cv, ui, self.pf(12.5), author_x, base, &c.author, TEXT_2, date_x - author_x - self.pf(12.0));
        let when = fmt::relative(c.time, self.now, self.r.tz);
        self.te.draw(cv, ui, self.pf(12.5), date_x, base, &when, dim, sha_x - date_x - self.pf(10.0));
        self.te.draw(cv, self.f.mono, self.pf(12.0), sha_x, base, &c.id.short(8), dim, self.pf(80.0));
        // CI status before the SHA: green passed, amber running, red failed.
        if let Some((st, _)) = self.r.ci.get(&c.id) {
            let color = match st {
                ffwd_core::hosting::Ci::Passed => ADD,
                ffwd_core::hosting::Ci::Running => MOD,
                ffwd_core::hosting::Ci::Failed => DEL,
            };
            cv.fill_circle(sha_x - self.pf(9.0), base - self.pf(4.0), self.pf(3.2), color);
        }
    }

    fn paint_graph(&self, cv: &mut Canvas, row: usize, r: Rect, bg: u32) {
        let Some(g) = self.r.graph.rows.get(row) else { return };
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
        let is_head = self.r.refs.head.is_some_and(|h| h == self.r.commits[row].id);
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
        self.px(190.0) + self.details_extra
    }

    /// Body lines shown at most, and the default room (two lines).
    const BODY_LINES: usize = 5;

    /// Recomputes the details header's extra height for the selected
    /// commit's message (on new details and on resize, never mid-paint).
    fn update_details_extra(&mut self) {
        let extra = self.details_extra_for();
        if extra != self.details_extra {
            self.details_extra = extra;
            self.damage.rects.push(self.lay.details);
        }
    }

    fn details_extra_for(&mut self) -> i32 {
        let Some(det) = self.r.details.clone() else { return 0 };
        let (ui, semi) = (self.f.ui, self.f.semi);
        let maxw = self.lay.details.w as f32 - 2.0 * self.pf(22.0);
        if maxw <= 0.0 {
            return 0;
        }
        let subject = det.message.lines().next().unwrap_or("").to_string();
        let s16 = self.pf(16.0);
        let subj = wrap(&mut self.te, semi, s16, &subject, maxw).len().clamp(1, 2);
        let body: Vec<String> =
            det.message.lines().skip(1).skip_while(|l| l.trim().is_empty()).map(|s| s.to_string()).collect();
        let s13 = self.pf(13.0);
        let lines: usize = body.iter().map(|p| wrap(&mut self.te, ui, s13, p, maxw).len().max(1)).sum();
        // The same arithmetic as paint_details: the rows that fit by default.
        let lh = self.pf(19.0);
        let y = self.pf(34.0) + subj as f32 * self.pf(22.0);
        let fit = ((self.pf(140.0) - self.pf(30.0) - y) / lh).floor().max(0.0) as usize;
        let more = lines.min(Self::BODY_LINES).saturating_sub(fit);
        (more as f32 * lh).round() as i32
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
        let n = self.r.details.as_ref().map_or(4, |x| x.files.len().clamp(1, self.file_rows_visible()));
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
        let Some(c) = self.r.commits.get(self.r.sel).cloned() else {
            let msg = if let Some(e) = &self.r.error { e.clone() } else { "Reading history…".into() };
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
        let compare = self.r.compare;
        let subject = match compare {
            // Plain ASCII: the UI font has every glyph on every OS.
            Some((a, b)) => format!("Changes from {} to {}", a.short(8), b.short(8)),
            None => c.subject.to_string(),
        };
        let (s16, s13) = (self.pf(16.0), self.pf(13.0));
        for line in wrap(&mut self.te, semi, s16, &subject, maxw).into_iter().take(2) {
            self.te.draw(cv, semi, self.pf(16.0), x0, y, &line, TEXT, maxw);
            y += self.pf(22.0);
        }
        self.compare_btns = [Rect::default(); 2];
        if compare.is_some() {
            // Swap and Close, the comparison's controls (FF-33).
            let mut bx = x0;
            for (k, (label, g)) in [("Swap", Glyph::Reset), ("Close", Glyph::Close)].into_iter().enumerate() {
                let w = self.te.measure(semi, self.pf(11.5), label) + self.pf(32.0);
                let b = Rect::new(bx as i32, (y - self.pf(12.0)) as i32, w as i32, self.px(24.0));
                let hot = self.is_hot(Hot::Compare(k));
                let (bg, border) = if hot { (CONTROL_HOVER, BORDER_HOVER) } else { (BG_BAR, BORDER) };
                cv.round_border(b, self.pf(6.0), border, bg);
                let cy = b.y as f32 + b.h as f32 / 2.0;
                self.glyph(cv, g, b.x as f32 + self.pf(13.0), cy, self.pf(5.0), TEXT_2);
                self.te.draw(
                    cv,
                    semi,
                    self.pf(11.5),
                    b.x as f32 + self.pf(22.0),
                    (cy + self.pf(4.0)).round(),
                    label,
                    TEXT,
                    w,
                );
                self.compare_btns[k] = b;
                bx += w + self.pf(8.0);
            }
        }
        let body: Vec<String> = match &self.r.details {
            Some(_) if compare.is_some() => Vec::new(),
            Some(det) => {
                det.message.lines().skip(1).skip_while(|l| l.trim().is_empty()).map(|s| s.to_string()).collect()
            }
            None => Vec::new(),
        };
        // The body gets the lines that fit above the author row; the last
        // visible line ends in an ellipsis when more text follows.
        let meta_y = d.y as f32 + self.pf(140.0) + self.details_extra as f32;
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
        let when = format!("{}  ·  {}", fmt::date_time(c.time, self.r.tz), fmt::relative(c.time, self.now, self.r.tz));
        self.te.draw(cv, ui, self.pf(12.0), mx, meta_y + self.pf(14.0), &when, TEXT_3, maxw);
        let sha_y = meta_y + self.pf(40.0);
        let mut sx = self.te.draw(cv, mono, self.pf(12.0), x0, sha_y, &c.id.short(12), TEXT_2, maxw);
        for p in c.parents.iter().take(2) {
            sx = self.te.draw(cv, ui, self.pf(12.0), sx + self.pf(14.0), sha_y, "parent", TEXT_3, maxw);
            sx = self.te.draw(cv, mono, self.pf(12.0), sx + self.pf(6.0), sha_y, &p.short(8), TEXT_3, maxw);
        }
        // Files.
        let fh_y = d.y + self.header_h();
        cv.fill(Rect::new(d.x + 1, fh_y, d.w - 1, 1), BORDER_SOFT);
        let fbase = fh_y as f32 + self.pf(23.0);
        let focus_files = self.focus == Focus::Files;
        match &self.r.details {
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
                let e = self.te.draw(
                    cv,
                    semi,
                    self.pf(11.5),
                    e + self.pf(6.0),
                    fbase,
                    &format!("−{}", fmt::num(dels as usize)),
                    DEL,
                    maxw,
                );
                // Files that matched a content or path search (FF-34): git
                // lists exactly those, so only visible rows are checked.
                let matched: Vec<String> = self.match_files(det.id).map(<[String]>::to_vec).unwrap_or_default();
                let nm = matched.len();
                if nm > 0 {
                    self.te.draw(
                        cv,
                        semi,
                        self.pf(11.5),
                        e + self.pf(10.0),
                        fbase,
                        &format!("· {} MATCH{}", fmt::num(nm), if nm == 1 { "" } else { "ES" }),
                        ACCENT,
                        maxw,
                    );
                }
                let fr = self.files_rect();
                let rh = self.px(26.0);
                for (k, f) in det.files.iter().enumerate().skip(self.r.file_top).take(self.file_rows_visible()) {
                    let ry = fr.y + (k - self.r.file_top) as i32 * rh;
                    let row = Rect::new(d.x + self.px(10.0), ry, d.w - self.px(20.0), rh);
                    if k == self.r.sel_file {
                        cv.fill_round(row, self.pf(6.0), if focus_files { BG_SELECT } else { BG_SELECT_DIM });
                    } else if self.is_hot(Hot::File(k)) {
                        cv.fill_round(row, self.pf(6.0), BG_HOVER);
                    }
                    if nm > 0 && matched.iter().any(|p| *p == *f.path) {
                        let m = self.px(5.0);
                        cv.fill(Rect::new(d.x + self.px(3.0), ry + m, self.px(3.0).max(2), rh - 2 * m), ACCENT);
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

    /// The diff pane, clipped to it: its bar's controls can be wider than
    /// the pane (wide fonts, narrow windows) and must not draw into the
    /// info pane, which a damage of the diff pane wouldn't repaint (FF-58).
    fn paint_diff(&mut self, cv: &mut Canvas) {
        let r = self.diff_rect();
        let old = cv.set_clip(cv.clip().intersect(r));
        self.paint_diff_clipped(cv);
        cv.set_clip(old);
    }

    fn paint_diff_clipped(&mut self, cv: &mut Canvas) {
        let r = self.diff_rect();
        if cv.clip().intersect(r).is_empty() || r.h <= 0 {
            return;
        }
        let (ui, semi) = (self.f.ui, self.f.semi);
        cv.fill(Rect::new(r.x + 1, r.y, r.w - 1, r.h), BG);
        let bar = Rect::new(r.x + 1, r.y, r.w - 1, self.px(32.0));
        cv.fill(bar, BG_BAR);
        cv.fill(Rect::new(bar.x, bar.bottom() - 1, bar.w, 1), BORDER_SOFT);
        let base = (bar.y as f32 + bar.h as f32 / 2.0 + self.pf(4.5)).round();
        let pad = self.pf(16.0);
        let Some(diff) = self.r.diff.clone() else {
            let label = match self.r.details.as_ref().and_then(|d| d.files.get(self.r.sel_file)) {
                Some(f) => f.path.to_string(),
                None if self.r.details.is_some() => "No file changes".into(),
                None => String::new(),
            };
            self.te.draw(cv, semi, self.pf(12.5), r.x as f32 + pad, base, &label, TEXT_2, r.w as f32 - 2.0 * pad);
            return;
        };
        let focus = self.focus == Focus::Diff;
        let left = self.paint_diff_bar_controls(cv, bar);
        if self.diff_find.is_some() {
            self.paint_diff_find(cv, bar, left);
        } else {
            let maxw = (left - r.x as f32 - pad - self.pf(8.0)).max(0.0);
            self.te.draw(
                cv,
                semi,
                self.pf(12.5),
                r.x as f32 + pad,
                base,
                &diff.path,
                if focus { ACCENT } else { TEXT },
                maxw,
            );
        }
        if diff.truncated {
            let hint = "first 20,000 lines";
            let hw = self.te.measure(ui, self.pf(11.0), hint);
            self.te.draw(cv, ui, self.pf(11.0), left - hw - pad, base, hint, MOD, hw + 2.0);
        }
        let body = Rect::new(r.x + 1, bar.bottom(), r.w - 1, r.bottom() - bar.bottom());
        let top = self.r.diff_top;
        if self.diff_split && !diff.binary {
            self.paint_diff_split(cv, body, &diff, top, None, 0);
        } else {
            self.paint_diff_body(cv, body, &diff, top, None, 0);
        }
        if !diff.binary {
            self.paint_diff_strip(cv, body);
        }
        let _ = ui;
    }

    /// The overview strip on the diff's right edge (FF-41): a tick where
    /// changes are (green added, red removed, amber both) and a band for
    /// what is in view. One pass over the rows into pixel buckets.
    fn paint_diff_strip(&mut self, cv: &mut Canvas, body: Rect) {
        let w = self.px(10.0);
        let strip = Rect::new(body.right() - w, body.y, w, body.h);
        self.diff_strip = strip;
        let n = self.diff_row_count();
        if n == 0 || strip.h <= 0 {
            return;
        }
        cv.fill(strip, BG_SIDEBAR);
        let fit = (body.h / self.code_lh().max(1)) as usize;
        if n > fit {
            let y0 = strip.y + (self.r.diff_top * strip.h as usize / n) as i32;
            let hh = ((fit * strip.h as usize / n) as i32).max(self.px(6.0));
            cv.fill(Rect::new(strip.x, y0, strip.w, hh), BG_HOVER);
        }
        for (py, m) in self.diff_marks(strip.h as usize).into_iter().enumerate() {
            let color = match m {
                1 => ADD,
                2 => DEL,
                3 => MOD,
                _ => continue,
            };
            cv.fill(
                Rect::new(strip.x + self.px(2.0), strip.y + py as i32, strip.w - self.px(4.0), self.px(2.0).max(1)),
                color,
            );
        }
    }

    /// Change marks per pixel of a strip `h` high: 1 added, 2 removed, 3 both.
    fn diff_marks(&mut self, h: usize) -> Vec<u8> {
        use ffwd_core::diff::LineKind;
        let mut out = vec![0u8; h];
        let Some(d) = self.r.diff.clone() else { return out };
        let kind = |i: usize| match d.lines[i].kind {
            LineKind::Add => 1u8,
            LineKind::Del => 2,
            _ => 0,
        };
        let rows: Vec<u8> = if self.diff_split {
            let (p, _) = self.diff_meta(&d);
            diff_view::split_rows_of(&d, &p).into_iter().map(|(l, r)| l.map_or(0, kind) | r.map_or(0, kind)).collect()
        } else {
            (0..d.lines.len()).map(kind).collect()
        };
        let n = rows.len().max(1);
        for (i, m) in rows.into_iter().enumerate() {
            if m != 0 {
                out[(i * h / n).min(h - 1)] |= m;
            }
        }
        out
    }

    /// A click on the strip scrolls there.
    fn diff_strip_click(&mut self, y: i32) {
        let n = self.diff_row_count();
        let s = self.diff_strip;
        if n == 0 || s.h <= 0 {
            return;
        }
        let row = ((y - s.y).max(0) as usize * n / s.h as usize).min(n - 1);
        let fit = (s.h / self.code_lh().max(1)) as usize;
        self.r.diff_top = row.saturating_sub(fit / 2);
        self.damage.rects.push(self.diff_rect());
    }

    /// The lines of a diff in `body`, from line `top`. With `hunk_label`,
    /// each hunk header gets a button; returns them as (hunk index, rect).
    /// `hit_base` is where those buttons start in the caller's hit list.
    pub(super) fn paint_diff_body(
        &mut self,
        cv: &mut Canvas,
        body: Rect,
        diff: &FileDiff,
        top: usize,
        hunk_label: Option<&str>,
        hit_base: usize,
    ) -> Vec<(usize, Rect)> {
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let mut buttons = Vec::new();
        let lh = self.code_lh();
        // Hunks above the first painted line count too.
        let mut hunk: i64 = diff.lines.iter().take(top).filter(|l| l.kind == LineKind::Hunk).count() as i64;
        if diff.binary {
            if crate::image::is_image(&diff.path) {
                self.paint_diff_images(cv, body);
                return buttons;
            }
            self.te.draw(
                cv,
                ui,
                self.pf(12.5),
                body.x as f32 + self.pf(16.0),
                body.y as f32 + self.pf(26.0),
                "Binary file: no text diff",
                TEXT_3,
                body.w as f32,
            );
            return buttons;
        }
        let num_w = self.code_px(38.0);
        let gut = Rect::new(body.x, body.y, (num_w * 2.0 + self.pf(8.0)) as i32, body.h);
        cv.fill(gut, BG_SIDEBAR);
        let fs = self.code_fs();
        let old_clip = cv.clip();
        let (pairs, blocks) = self.diff_meta(diff);
        let lang = crate::highlight::lang_for(&diff.path);
        let (code_x, code_w) = self.code_column(body);
        // Lines from `top` until the body is full; a wrapped line takes
        // several rows (FF-41).
        let mut next_y = body.y;
        for (k, line) in diff.lines.iter().enumerate().skip(top).map(|(i, l)| (i - top, l)) {
            if next_y >= body.bottom() {
                break;
            }
            let y = next_y;
            let nrows = self.diff_rows(diff, top + k, body);
            let h = lh * nrows as i32;
            next_y += h;
            let row = Rect::new(body.x, y, body.w, h);
            if old_clip.intersect(row).is_empty() {
                continue;
            }
            let lb = (y as f32 + lh as f32 / 2.0 + self.code_px(4.0)).round();
            let (bgc, fg, mark) = match line.kind {
                LineKind::Add => (ADD_BG, ADD_TEXT, "+"),
                LineKind::Del => (DEL_BG, DEL_TEXT, "−"),
                LineKind::Hunk => (HUNK_BG, HUNK_TEXT, ""),
                LineKind::Meta => (BG, TEXT_4, ""),
                LineKind::Context => (BG, TEXT_2, ""),
            };
            if bgc != BG {
                cv.fill(Rect::new(gut.right(), y, body.w - gut.w, h), bgc);
                let gbg = match line.kind {
                    LineKind::Add => DIFF_ADD_LINE,
                    LineKind::Del => DIFF_DEL_LINE,
                    _ => BG_SIDEBAR,
                };
                cv.fill(Rect::new(gut.x, y, gut.w, h), gbg);
            }
            if line.kind == LineKind::Hunk {
                hunk += 1;
                let mut text_w = (body.right() - gut.right()) as f32 - self.pf(20.0);
                if let Some(label) = hunk_label {
                    // A stage or unstage button at the hunk's right end.
                    let w = self.te.measure(semi, self.pf(11.0), label) + self.pf(18.0);
                    let b = Rect::new(
                        body.right() - self.px(10.0) - w as i32,
                        y + self.px(2.0),
                        w as i32,
                        lh - self.px(4.0),
                    );
                    let hot = Hot::Change(hit_base + buttons.len());
                    let (bg, border) = self.control_colors(hot, BG_BAR, HUNK_BORDER);
                    cv.round_border(b, self.pf(5.0), border, bg);
                    self.te.draw(cv, semi, self.pf(11.0), b.x as f32 + self.pf(9.0), lb, label, HUNK_TEXT, w);
                    buttons.push((hunk as usize - 1, b));
                    text_w -= w + self.pf(12.0);
                }
                self.te.draw(
                    cv,
                    mono,
                    self.code_px(11.5),
                    gut.right() as f32 + self.pf(10.0),
                    lb,
                    &line.text,
                    fg,
                    text_w,
                );
                continue;
            }
            let numc = match line.kind {
                LineKind::Add => ADD_GUTTER,
                LineKind::Del => DEL_GUTTER,
                _ => TEXT_4,
            };
            if line.old > 0 {
                let s = line.old.to_string();
                let w = self.te.measure(mono, self.code_px(11.0), &s);
                self.te.draw(cv, mono, self.code_px(11.0), body.x as f32 + num_w - w, lb, &s, numc, num_w);
            }
            if line.new > 0 {
                let s = line.new.to_string();
                let w = self.te.measure(mono, self.code_px(11.0), &s);
                self.te.draw(cv, mono, self.code_px(11.0), body.x as f32 + num_w * 2.0 - w, lb, &s, numc, num_w);
            }
            let tx = gut.right() as f32 + self.pf(8.0);
            if !mark.is_empty() {
                self.te.draw(cv, mono, fs, tx, lb, mark, if line.kind == LineKind::Add { ADD } else { DEL }, 20.0);
            }
            let text = diff_find::shown_text(&line.text, self.diff_tab).into_owned();
            let i = top + k;
            // Word ranges are byte offsets of the raw text: skip them on
            // lines whose tabs were widened.
            let mut marks: Vec<_> =
                Self::word_range(diff, &pairs, i).filter(|_| !line.text.contains('\t')).into_iter().collect();
            if hunk_label.is_none() {
                // The history's diff: find matches (the Local Changes diff has none).
                marks.extend(self.diff_find_marks(i, &text));
            }
            let st = blocks.get(i).copied().unwrap_or(false);
            let maxw = (body.right() as f32) - tx - self.pf(24.0);
            if nrows == 1 {
                let r1 = Rect::new(body.x, y, body.w, lh);
                // Scrolled sideways: the code moves, numbers and marks stay.
                let left = if self.diff_wrap { 0.0 } else { self.code_left };
                let band = Rect::new(code_x as i32, y, (body.right() as f32 - code_x) as i32, lh);
                let prev = cv.set_clip(old_clip.intersect(band));
                self.draw_code(cv, tx + self.pf(14.0) - left, lb, r1, &text, fg, maxw + left, lang, st, &marks);
                if self.diff_show_ws {
                    self.draw_whitespace(cv, tx + self.pf(14.0) - left, lb, &line.text, maxw + left);
                }
                cv.set_clip(prev);
            } else {
                // Each row shows the next `code_w` of the line: the whole
                // line drawn shifted left and clipped to the code column,
                // so colours and highlights stay as one line's.
                // Rows split by characters; each shift is the measured width
                // of the text before the row, so rows meet between glyphs.
                let per = self.wrap_chars(body);
                let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).chain([text.len()]).collect();
                let fs = self.code_fs();
                for r in 0..nrows {
                    let ry = y + r as i32 * lh;
                    let (a, b) =
                        (starts[(r * per).min(starts.len() - 1)], starts[((r + 1) * per).min(starts.len() - 1)]);
                    let shift = self.te.measure(mono, fs, &text[..a]);
                    let rw = self.te.measure(mono, fs, &text[a..b]);
                    let band = Rect::new(code_x as i32, ry, rw.ceil() as i32 + 1, lh);
                    let prev = cv.set_clip(old_clip.intersect(band));
                    let rb = lb + (r as i32 * lh) as f32;
                    let rr = Rect::new(body.x, ry, body.w, lh);
                    // The clip cuts the row; a generous width keeps the
                    // text engine from ending it in an ellipsis.
                    let wide = shift + code_w * 2.0;
                    self.draw_code(cv, code_x - shift, rb, rr, &text, fg, wide, lang, st, &marks);
                    if self.diff_show_ws {
                        self.draw_whitespace(cv, code_x - shift, rb, &line.text, wide);
                    }
                    cv.set_clip(prev);
                }
            }
        }
        let _ = (ui, semi);
        buttons
    }

    fn paint_status(&mut self, cv: &mut Canvas) {
        let r = self.lay.status;
        cv.fill(r, BG_SIDEBAR);
        cv.fill(Rect::new(r.x, r.y, r.w, 1), BORDER);
        let ui = self.f.ui;
        let base = (r.y as f32 + r.h as f32 / 2.0 + self.pf(4.0)).round();
        let cy = r.y as f32 + r.h as f32 / 2.0;
        let mut x = self.pf(14.0);
        let (dot, msg) = if let Some(e) = &self.r.error {
            (DEL, e.clone())
        } else if let Some(n) = &self.notice {
            (MOD, n.clone())
        } else if !self.r.log_done {
            (MOD, format!("Reading history · {} commits", fmt::num(self.r.commits.len())))
        } else {
            (ADD, "Ready".to_string())
        };
        cv.fill_circle(x + self.pf(3.0), cy, self.pf(3.5), dot);
        x += self.pf(14.0);
        x = self.te.draw(cv, ui, self.pf(12.0), x, base, &msg, TEXT_2, self.pf(520.0));
        self.notice_btn = Rect::default();
        if let Some(a) = self.notice_act.filter(|_| self.r.error.is_none() && self.notice.is_some()) {
            let (label, g) = a.label();
            let semi = self.f.semi;
            let w = self.te.measure(semi, self.pf(11.5), label) + self.pf(30.0);
            let b = Rect::new((x + self.pf(10.0)) as i32, r.y + self.px(3.0), w as i32, r.h - self.px(5.0));
            let hot = self.is_hot(Hot::Notice);
            cv.round_border(
                b,
                self.pf(5.0),
                if hot { ACCENT } else { ACCENT_BORDER },
                if hot { ACCENT_SOFT_HOVER } else { ACCENT_SOFT },
            );
            self.glyph(cv, g, b.x as f32 + self.pf(11.0), cy, self.pf(4.5), ACCENT_FG);
            self.te.draw(cv, semi, self.pf(11.5), b.x as f32 + self.pf(20.0), base, label, ACCENT_FG, w);
            self.notice_btn = b;
            x = b.right() as f32;
        }
        if self.r.log_done && self.r.error.is_none() {
            let mut facts = format!("{} commits in {:.0} ms", fmt::num(self.r.commits.len()), self.r.log_ms);
            if let Some(fb) = self.r.first_batch_ms {
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
        let (query, sel, mode) = (p.input.text().to_string(), p.sel, p.mode);
        let (cursor, all) = (p.input.cursor(), p.input.all_selected());
        let items: Vec<(String, String, RefKind, bool)> = p
            .items
            .iter()
            .map(|i| (i.label.clone(), i.hint.clone(), i.kind, matches!(i.action, PAction::Command(_))))
            .collect();
        let old = cv.set_clip(clip.intersect(r));
        cv.round_border(r, self.pf(12.0), BORDER_STRONG, BG_FLOAT);
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
                qx + self.pf(6.0),
                base,
                if mode == PMode::Repos {
                    "Open a repository: type its name or a folder path"
                } else {
                    "Type a branch, tag, SHA or command"
                },
                TEXT_3,
                r.w as f32 - self.pf(80.0),
            );
            qx
        } else {
            if all {
                // Select-all shows as a highlight behind the text.
                let w = self.te.measure(ui, self.pf(15.0), &query).min(r.w as f32 - self.pf(80.0));
                cv.fill_round(
                    Rect::new(
                        qx as i32 - self.px(2.0),
                        input.y + self.px(13.0),
                        w as i32 + self.px(4.0),
                        self.px(26.0),
                    ),
                    self.pf(4.0),
                    BG_TEXT_SELECT,
                );
            }
            self.te.draw(cv, ui, self.pf(15.0), qx, base, &query, TEXT, r.w as f32 - self.pf(80.0));
            qx + self.te.measure(ui, self.pf(15.0), &query[..cursor.min(query.len())])
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
            } else if self.is_hot(Hot::Palette(k)) {
                cv.fill_round(row, self.pf(7.0), BG_HOVER);
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
            } else if mode == PMode::Repos {
                // Folder: a body with a small tab on its top left.
                let (w, h) = (self.pf(12.0), self.pf(9.0));
                let body = Rect::new((icx - w / 2.0) as i32, (cy - h / 2.0 + self.pf(1.0)) as i32, w as i32, h as i32);
                cv.fill(Rect::new(body.x, body.y - self.px(2.0), (w * 0.45) as i32, self.px(3.0)), TEXT_3);
                cv.round_border(body, self.pf(2.0), TEXT_3, if k == sel { BG_SELECT } else { BG_INPUT });
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

/// `text` shortened in the middle ("worktree-agent-a0…9f2b") so it fits
/// `maxw`; unchanged when it fits.
pub(super) fn fit_middle(te: &mut TextEngine, font: FontId, size: f32, text: &str, maxw: f32) -> String {
    if maxw <= 0.0 || te.measure(font, size, text) <= maxw {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // The most characters kept (split 60/40, front first) that still fit.
    let (mut lo, mut hi) = (0usize, chars.len());
    let cut = |k: usize| -> String {
        let front = (k * 3).div_ceil(5);
        let back = k - front;
        let mut s: String = chars[..front].iter().collect();
        s.push('…');
        s.extend(&chars[chars.len() - back..]);
        s
    };
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if te.measure(font, size, &cut(mid)) <= maxw { lo = mid } else { hi = mid - 1 }
    }
    cut(lo)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_folder_in_two_spellings_is_one_tab() {
        assert!(same_path(Path::new(r"E:\Development\Kwil"), Path::new("e:/development/kwil/")));
        assert!(!same_path(Path::new(r"E:\Development\kwil"), Path::new(r"E:\Development\kwil2")));
        #[cfg(windows)]
        assert_eq!(repo_name(Path::new(r"E:\Development\ffwd")), "ffwd");
        assert_eq!(repo_name(Path::new("dev/ffwd")), "ffwd");
    }

    /// Local Changes end to end, as a user drives it: open the view,
    /// Space stages the selected file, type a subject, Ctrl+Enter commits.
    #[test]
    fn local_changes_stage_and_commit_with_keys() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-changes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(
            dir.join("a.txt"),
            "a
",
        )
        .unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(
            dir.join("b.txt"),
            "new file
",
        )
        .unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, &|u| u.r.git.is_some());
        ui.enter_changes();
        pump(&mut ui, &|u| u.r.changes.status.as_ref().is_some_and(|s| s.side(changes_view_side(true)).len() == 1));
        ui.key(Key::Char(' '));
        pump(&mut ui, &|u| {
            u.r.changes.busy.is_none()
                && u.r.changes.status.as_ref().is_some_and(|s| s.side(changes_view_side(false)).len() == 1)
        });
        for c in "Add b".chars() {
            ui.key(Key::Char(c));
        }
        assert_eq!(ui.r.changes.subject.text(), "Add b", "typing in the list starts the subject");
        ui.key(Key::CtrlEnter);
        pump(&mut ui, &|u| u.r.changes.busy.is_none() && u.r.changes.subject.text().is_empty());
        let log = String::from_utf8(g.output(&["log", "--format=%s"]).unwrap()).unwrap();
        assert_eq!(log.lines().collect::<Vec<_>>(), vec!["Add b", "first"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Files view, as a user drives it: the tree opens a single top
    /// folder, typing filters, Enter opens a file, b and h show its blame
    /// and history, Left folds a folder (FF-51, FF-43).
    #[test]
    fn files_view_filter_open_blame_history_with_keys() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.join("README.md"), "# demo\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        g.output(&["commit", "-q", "-am", "second", "--author", "Other <o@example.invalid>"]).unwrap();
        std::fs::write(dir.join("src/new.rs"), "x\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, &|u| u.r.git.is_some() && u.r.commits.len() == 2);
        ui.enter_files();
        pump(&mut ui, &|u| u.r.files.tree.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let names = |u: &Ui| -> Vec<String> {
            let t = u.r.files.tree.as_ref().unwrap();
            u.r.files.rows.iter().map(|&(id, _)| t.path(id)).collect()
        };
        // The only top folder opens on the first look; folders come first.
        assert_eq!(names(&ui), ["src", "src/a.rs", "src/new.rs", "README.md"]);
        let t = ui.r.files.tree.as_ref().unwrap();
        let new = ui.r.files.rows[2].0;
        assert_eq!(t.nodes[new as usize].mark, b'?', "untracked files are marked");
        // Typing filters (to the file and its folders); Down goes back to the tree.
        for c in "a.rs".chars() {
            ui.key(Key::Char(c));
        }
        assert_eq!(names(&ui), ["src", "src/a.rs"]);
        assert_partial_paint_is_full(&mut ui, &mut screen, "filter");
        ui.key(Key::Down);
        ui.key(Key::Down);
        assert_partial_paint_is_full(&mut ui, &mut screen, "move in the tree");
        ui.key(Key::Enter);
        pump(&mut ui, &|u| u.r.files.content.is_some());
        assert_partial_paint_is_full(&mut ui, &mut screen, "open a file");
        ui.pointer(900, 120);
        assert_partial_paint_is_full(&mut ui, &mut screen, "hover a tab");
        assert_eq!(ui.r.files.file.as_ref().map(|x| x.1.as_str()), Some("src/a.rs"));
        assert_eq!(ui.r.files.content.as_ref().unwrap().content.lines.len(), 2);
        ui.key(Key::Tab);
        ui.key(Key::Char('b'));
        pump(&mut ui, &|u| u.r.files.blame.is_some());
        assert_partial_paint_is_full(&mut ui, &mut screen, "blame");
        let b = ui.r.files.blame.as_ref().unwrap();
        let authors: Vec<&str> = b.lines.iter().map(|&i| &*b.commits[i as usize].author).collect();
        assert_eq!(authors, ["ffwd test", "Other"]);
        ui.key(Key::Char('h'));
        pump(&mut ui, &|u| u.r.files.history.is_some());
        assert_eq!(ui.r.files.history.as_ref().unwrap().len(), 2);
        assert_partial_paint_is_full(&mut ui, &mut screen, "history");
        // Enter on a history row shows that commit in the history view.
        ui.key(Key::Enter);
        assert!(!ui.r.files_view, "Enter jumps to the history");
        assert_partial_paint_is_full(&mut ui, &mut screen, "jump to the history");
        assert_eq!(ui.r.sel, 0, "the newest commit is selected");
        // Back in the view, clearing the filter keeps the file; Left folds.
        ui.enter_files();
        pump(&mut ui, &|u| u.r.files.tree.is_some() && u.r.files.file.is_some());
        ui.r.files.focus = files_view::FFocus::Filter;
        ui.key(Key::Esc);
        ui.key(Key::Esc);
        assert_eq!(names(&ui).len(), 4);
        ui.key(Key::Home);
        ui.key(Key::Left);
        assert_eq!(names(&ui), ["src", "README.md"], "Left folds the folder");
        assert_partial_paint_is_full(&mut ui, &mut screen, "fold");
        ui.key(Key::Right);
        assert_eq!(names(&ui).len(), 4, "Right opens it again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Lines, discard and undo, recent messages, all from the keyboard.
    #[test]
    fn local_changes_lines_discard_undo_with_keys() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-changes2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "1\n2\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "1\n2\nadd-one\nadd-two\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let idle = |u: &Ui| u.r.changes.busy.is_none();
        pump(&mut ui, "the repository", &|u| u.r.git.is_some());
        ui.enter_changes();
        pump(&mut ui, "the diff", &|u| u.r.changes.diff.is_some());
        // In the diff: go to "+add-two" and pick it, then S stages it.
        ui.key(Key::Right);
        let target = ui.r.changes.diff.as_ref().unwrap().lines.iter().position(|l| &*l.text == "add-two").unwrap();
        for _ in 0..target {
            ui.key(Key::Down);
        }
        ui.key(Key::Char(' '));
        assert_eq!(ui.r.changes.picked.len(), 1);
        ui.key(Key::Char('s'));
        pump(&mut ui, "line staging", &|u| {
            idle(u) && u.r.changes.status.as_ref().is_some_and(|s| s.side(changes_view_side(false)).len() == 1)
        });
        assert_eq!(String::from_utf8(g.output(&["show", ":a.txt"]).unwrap()).unwrap(), "1\n2\nadd-two\n");
        // Discard the rest: the first Delete only asks.
        ui.key(Key::Esc);
        ui.key(Key::Home);
        ui.key(Key::Delete);
        assert!(std::fs::read_to_string(dir.join("a.txt")).unwrap().contains("add-one"), "one Delete never discards");
        ui.key(Key::Delete);
        pump(&mut ui, "the discard", &|u| idle(u) && !u.r.changes.undo.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "1\n2\nadd-two\n");
        ui.key(Key::Ctrl('z'));
        pump(&mut ui, "the undo", &|u| idle(u) && u.r.changes.undo.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "1\n2\nadd-one\nadd-two\n");
        // Commit, then Ctrl+R brings the message back.
        for c in "Two | lines".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::CtrlEnter);
        pump(&mut ui, "the commit", &|u| idle(u) && u.r.changes.subject.text().is_empty());
        ui.key(Key::Ctrl('r'));
        assert_eq!(ui.r.changes.subject.text(), "Two | lines");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Local Changes side by side (FF-35): the bar's button switches the
    /// layout; clicking a line in its half picks it, the hunk button
    /// stages, and the cursor follows rows; paints are right.
    #[test]
    fn local_changes_side_by_side() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-csplit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "1\nold\n2\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "1\nnew\n2\nadded\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the repository", &|u| u.r.git.is_some());
        ui.enter_changes();
        pump(&mut ui, "the diff", &|u| u.r.changes.diff.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        use changes_view::CHit;
        let hit = |ui: &Ui, h: CHit| ui.r.changes.hits.iter().find(|x| x.1 == h).map(|x| x.0);
        let b = hit(&ui, CHit::Layout).expect("the layout button");
        ui.click(b.x + b.w / 2, b.y + b.h / 2);
        ui.last_click = None;
        assert!(ui.diff_split && ui.changes_split());
        assert_eq!(ui.settings.get("diff.layout"), Some("split"));
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        // "new" is on the right half of the row that shows "old" on the left.
        let d = ui.r.changes.diff.clone().unwrap();
        let new = d.lines.iter().position(|l| &*l.text == "new").unwrap();
        let old = d.lines.iter().position(|l| &*l.text == "old").unwrap();
        let r = ui.changes_layout().diff;
        let body = Rect::new(r.x, r.y + ui.px(32.0), r.w, r.bottom() - r.y - ui.px(32.0));
        let cell = ui.split_line_rect(&d, ui.changes_diff_top(), body, new).expect("on screen");
        assert!(cell.x > body.x + body.w / 3, "the right half");
        assert_eq!(ui.split_line_rect(&d, ui.changes_diff_top(), body, old).unwrap().y, cell.y, "same row");
        ui.click(cell.x + cell.w / 2, cell.y + cell.h / 2);
        ui.last_click = None;
        assert_eq!(ui.r.changes.picked.iter().copied().collect::<Vec<_>>(), [new]);
        assert_partial_paint_is_full(&mut ui, &mut screen, "picked side by side");
        // The hunk's button stages it.
        ui.r.changes.picked.clear();
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let b = hit(&ui, CHit::Hunk(0)).expect("a hunk button side by side");
        ui.click(b.x + b.w / 2, b.y + b.h / 2);
        pump(&mut ui, "the hunk staged", &|u| {
            u.r.changes.busy.is_none()
                && u.r.changes.status.as_ref().is_some_and(|s| s.side(changes_view_side(false)).len() == 1)
        });
        let staged = String::from_utf8(g.output(&["diff", "--cached", "--name-only"]).unwrap()).unwrap();
        assert_eq!(staged.trim(), "a.txt");
        // Back to unified: the button again.
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let b = hit(&ui, CHit::Layout).unwrap();
        ui.click(b.x + b.w / 2, b.y + b.h / 2);
        assert!(!ui.diff_split);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Branches from the keyboard: Ctrl+. opens the commit's menu, a name
    /// prompt creates the branch; deleting an unmerged branch asks twice.
    #[test]
    fn branch_menu_prompt_and_unmerged_delete_with_keys() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-branches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        // "lonely" has a commit no other branch has.
        g.output(&["switch", "-q", "-c", "lonely"]).unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "lonely work"]).unwrap();
        g.output(&["switch", "-q", "main"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let pick = |ui: &mut Ui, label: &str| {
            for _ in 0..40 {
                ui.key(Key::Down);
                let m = ui.menu.as_ref().unwrap();
                if m.sel.is_some_and(|s| m.items[s].label.starts_with(label)) {
                    ui.key(Key::Enter);
                    return;
                }
            }
            panic!("no menu item {label}");
        };
        let branch =
            |name: &str| g.output(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).is_ok();
        pump(&mut ui, "history and refs", &|u| u.r.log_done && u.r.refs.head_branch.is_some());
        // Ctrl+. on the selected (HEAD) commit, then "Create branch here…".
        ui.key(Key::Ctrl('.'));
        assert!(ui.menu.is_some());
        pick(&mut ui, "Create branch here");
        assert!(ui.dialog.is_some() && ui.menu.is_none());
        for c in "feat/from-keys".chars() {
            ui.key(Key::Char(c));
        }
        // Tab: don't check it out, so HEAD stays on main.
        ui.key(Key::Tab);
        ui.key(Key::Enter);
        pump(&mut ui, "the new branch", &|u| u.dialog.is_none());
        assert!(branch("feat/from-keys"));
        assert_eq!(String::from_utf8(g.output(&["branch", "--show-current"]).unwrap()).unwrap().trim(), "main");
        // Delete "lonely" from its sidebar menu: confirm, then "not merged", then delete.
        pump(&mut ui, "the new refs", &|u| u.r.side.iter().any(|i| i.key == "b:lonely"));
        let items = ui.ref_menu("b:lonely");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Delete lonely");
        assert!(ui.dialog.is_some());
        ui.key(Key::Enter);
        pump(&mut ui, "the not-merged question", &|u| {
            u.dialog.as_ref().is_some_and(|d| matches!(&d.kind, DialogKind::Ask(a) if a.title.contains("not merged")))
        });
        assert!(branch("lonely"), "a plain delete never drops unmerged work");
        ui.key(Key::Enter);
        pump(&mut ui, "the forced delete", &|u| u.dialog.is_none());
        assert!(!branch("lonely"));
        assert!(ui.notice.as_deref().is_some_and(|n| n.contains("git branch lonely ")), "{:?}", ui.notice);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stashes from the keyboard: Ctrl+Shift+S saves with a message (Tab
    /// leaves untracked files out), the sidebar menu renames, pops and
    /// drops, and dropping asks first and names the way back (FF-38).
    #[test]
    fn stash_save_rename_pop_drop_with_keys() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-stash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
        std::fs::write(dir.join("new.txt"), "untracked\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let pick = |ui: &mut Ui, label: &str| {
            for _ in 0..40 {
                ui.key(Key::Down);
                let m = ui.menu.as_ref().unwrap();
                if m.sel.is_some_and(|s| m.items[s].label.starts_with(label)) {
                    ui.key(Key::Enter);
                    return;
                }
            }
            panic!("no menu item {label}");
        };
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
        pump(&mut ui, "history and refs", &|u| u.r.log_done && u.r.refs.head_branch.is_some());
        // Save: message, Tab leaves untracked files out, Enter.
        ui.key(Key::CtrlShift('s'));
        assert!(ui.dialog.is_some());
        for c in "wip a".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Tab);
        ui.key(Key::Enter);
        pump(&mut ui, "the stash", &|u| u.dialog.is_none() && u.r.refs.stashes.len() == 1);
        assert_eq!(&*ui.r.refs.stashes[0].1, "On main: wip a");
        assert_eq!(read("a.txt"), "a\n", "the change is stashed");
        assert_eq!(read("new.txt"), "untracked\n", "untracked files stayed");
        // Rename from the sidebar menu.
        let items = ui.ref_menu("s:stash@{0}");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Rename");
        ui.key(Key::Ctrl('a'));
        for c in "better name".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        pump(&mut ui, "the rename", &|u| {
            u.dialog.is_none() && u.r.refs.stashes.first().is_some_and(|s| &*s.1 == "better name")
        });
        // Pop brings the change back and removes the stash.
        let items = ui.ref_menu("s:stash@{0}");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Pop");
        pump(&mut ui, "the pop", &|u| u.r.log_done && u.r.refs.stashes.is_empty() && read("a.txt") == "changed\n");
        // Stash again (with untracked files), then drop: it asks first.
        ui.key(Key::CtrlShift('s'));
        ui.key(Key::Enter);
        pump(&mut ui, "the second stash", &|u| u.dialog.is_none() && u.r.refs.stashes.len() == 1);
        assert!(!dir.join("new.txt").exists(), "untracked files go by default");
        let items = ui.ref_menu("s:stash@{0}");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Drop");
        assert!(ui.dialog.as_ref().is_some_and(|d| matches!(&d.kind, DialogKind::Ask(_))), "dropping asks first");
        ui.key(Key::Enter);
        pump(&mut ui, "the drop", &|u| u.dialog.is_none() && u.r.refs.stashes.is_empty());
        assert!(ui.notice.as_deref().is_some_and(|n| n.contains("git stash store")), "{:?}", ui.notice);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Remotes from the keyboard (FF-39): add one (URL, then a suggested
    /// name, then it is fetched), rename it, test it, remove it (asks
    /// first); a pull with diverged branches asks merge or rebase; the busy
    /// Fetch button cancels a fetch that hangs.
    #[test]
    fn remotes_add_rename_remove_pull_question_and_cancel() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-remotes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("up")).unwrap();
        let init = |g: &Git| {
            for a in [
                &["config", "user.name", "ffwd test"][..],
                &["config", "user.email", "test@example.invalid"],
                &["config", "commit.gpgsign", "false"],
            ] {
                g.output(a).unwrap();
            }
        };
        let commit = |g: &Git, file: &str| {
            std::fs::write(g.root.join(file), file).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", file]).unwrap();
        };
        let up = Git { root: base.join("up") };
        up.output(&["init", "-q", "-b", "main"]).unwrap();
        init(&up);
        commit(&up, "a.txt");
        Git { root: base.clone() }.output(&["clone", "-q", "up", "down"]).unwrap();
        let g = Git { root: base.join("down") };
        init(&g);
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(g.root.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, g.root.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(40);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let pick = |ui: &mut Ui, label: &str| {
            for _ in 0..40 {
                ui.key(Key::Down);
                let m = ui.menu.as_ref().unwrap();
                if m.sel.is_some_and(|s| m.items[s].label.starts_with(label)) {
                    ui.key(Key::Enter);
                    return;
                }
            }
            panic!("no menu item {label}");
        };
        let typ = |ui: &mut Ui, t: &str| {
            ui.key(Key::Ctrl('a'));
            for c in t.chars() {
                ui.key(Key::Char(c));
            }
        };
        let names = |u: &Ui| u.r.refs.remotes.iter().map(|r| r.name.to_string()).collect::<Vec<_>>();
        pump(&mut ui, "history and refs", &|u| u.r.log_done && names(u) == ["origin"]);
        // Add: URL, then the suggested name (the folder: origin is taken).
        let items = ui.ref_menu("h:remotes");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Add remote");
        typ(&mut ui, &base.join("up").to_string_lossy().replace('\\', "/"));
        ui.key(Key::Enter);
        let suggested = match &ui.dialog.as_ref().unwrap().kind {
            DialogKind::Name { act: NameAct::AddRemoteName { .. }, field, .. } => field.text().to_string(),
            _ => panic!("the name step follows the URL"),
        };
        assert_eq!(suggested, "up");
        ui.key(Key::Enter);
        pump(&mut ui, "the new remote", &|u| u.dialog.is_none() && names(u).contains(&"up".to_string()));
        assert!(g.output(&["rev-parse", "--verify", "refs/remotes/up/main"]).is_ok(), "added remotes are fetched");
        // Rename, then test the connection.
        let items = ui.ref_menu("r:up/");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Rename");
        typ(&mut ui, "mirror");
        ui.key(Key::Enter);
        pump(&mut ui, "the rename", &|u| u.dialog.is_none() && names(u).contains(&"mirror".to_string()));
        let items = ui.ref_menu("r:mirror/");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Test connection");
        pump(&mut ui, "the test", &|u| u.notice.as_deref().is_some_and(|n| n.contains("answers")));
        // Remove asks first and names the way back.
        let items = ui.ref_menu("r:mirror/");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Remove mirror");
        assert!(ui.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Ask(_))));
        ui.key(Key::Enter);
        pump(&mut ui, "the removal", &|u| u.dialog.is_none() && names(u) == ["origin"]);
        assert!(ui.notice.as_deref().is_some_and(|n| n.contains("git remote add mirror")), "{:?}", ui.notice);
        // Diverged pull: unless the user's global config decides, ffwd asks.
        commit(&up, "up.txt");
        commit(&g, "down.txt");
        ui.start_op(Op::Pull);
        pump(&mut ui, "the pull", &|u| u.r.op.is_none());
        if let Some(DialogKind::Ask(a)) = ui.dialog.as_ref().map(|d| &d.kind) {
            assert!(a.title.contains("new commits"));
            ui.key(Key::Right); // Rebase
            ui.key(Key::Enter);
            pump(&mut ui, "the rebase pull", &|u| {
                u.r.op.is_none() && u.notice.as_deref().is_some_and(|n| n.contains("rebased"))
            });
            let log = String::from_utf8(g.output(&["log", "--format=%s"]).unwrap()).unwrap();
            assert!(log.starts_with("down.txt\nup.txt\n"), "{log}");
        }
        // Cancel: a fetch from a remote that never answers; Fetch again cancels.
        let wait = if cfg!(windows) { "ping -n 30 127.0.0.1" } else { "sleep 30" };
        g.output(&["remote", "add", "slow", &format!("ext::{wait}")]).unwrap();
        g.output(&["config", "protocol.ext.allow", "always"]).unwrap();
        let t = std::time::Instant::now();
        ui.start_op(Op::Fetch);
        std::thread::sleep(std::time::Duration::from_millis(300));
        ui.start_op(Op::Fetch);
        pump(&mut ui, "the cancel", &|u| u.r.op.is_none());
        assert!(ui.notice.as_deref().is_some_and(|n| n.contains("Fetch cancelled")), "{:?}", ui.notice);
        assert!(t.elapsed() < std::time::Duration::from_secs(15), "cancel took {:?}", t.elapsed());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The activity log (FF-48): a branch operation, a failing checkout and
    /// a staged file each log their git commands; failures raise the
    /// sidebar badge until the view opens; partial repaints match.
    #[test]
    fn activity_log_records_operations_with_their_commands() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-activity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done);
        ui.run_refop(crate::work::RefOp::CreateBranch("feat/x".into(), "main".into(), false));
        pump(&mut ui, "the branch", &|u| u.activity.entries.len() == 1);
        ui.run_refop(crate::work::RefOp::Checkout("no-such-branch".into()));
        pump(&mut ui, "the failed checkout", &|u| u.activity.entries.len() == 2);
        ui.enter_changes();
        pump(&mut ui, "the changes", &|u| u.r.changes.status.as_ref().is_some_and(|s| !s.entries.is_empty()));
        ui.key(Key::Char(' '));
        pump(&mut ui, "the stage", &|u| u.activity.entries.len() == 3);
        let a = &ui.activity;
        assert_eq!(a.get(2).unwrap().title, "Create branch feat/x");
        assert!(a.get(2).unwrap().ok);
        assert!(a.get(2).unwrap().ran.iter().any(|r| r.args.starts_with("branch")), "{:?}", a.get(2).unwrap().ran);
        let failed = a.get(1).unwrap();
        assert!(!failed.ok && failed.title == "Checkout no-such-branch");
        assert!(failed.ran.iter().any(|r| r.code != Some(0)), "the failing command is logged with its code");
        assert_eq!(a.get(0).unwrap().title, "Stage 1 file");
        assert_eq!(&*a.get(0).unwrap().repo, dir.file_name().unwrap().to_str().unwrap());
        assert_eq!(a.unseen_failures, 1);
        assert!(ui.r.side.iter().any(|i| i.key == "activity" && i.badge == "1 failed"));
        // Open the view: the badge clears; moving and copying works.
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        ui.enter_activity();
        assert_eq!(ui.activity.unseen_failures, 0);
        assert!(ui.r.side.iter().any(|i| i.key == "activity" && i.badge.is_empty()), "the badge clears");
        assert_partial_paint_is_full(&mut ui, &mut screen, "open the activity log");
        ui.key(Key::Down);
        assert_eq!(ui.activity.sel, 1);
        assert_partial_paint_is_full(&mut ui, &mut screen, "select the failure");
        ui.pointer(400, 200);
        assert_partial_paint_is_full(&mut ui, &mut screen, "hover a row");
        ui.key(Key::Esc);
        assert!(!ui.r.activity_view);
        assert_partial_paint_is_full(&mut ui, &mut screen, "leave the log");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Interactive rebase from the keyboard (FF-37): Ctrl+Shift+R on a
    /// commit opens the plan; r rewords, f folds a commit into the one
    /// below, Shift+K moves one up, Ctrl+Enter runs it; partial repaints
    /// match full ones while the plan is open.
    #[test]
    fn interactive_rebase_plan_with_keys() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-rebase-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        for (f, m) in [("a", "one"), ("b", "two"), ("c", "three"), ("d", "four")] {
            std::fs::write(dir.join(f), f).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 4 && u.r.refs.head_branch.is_some());
        assert!(ui.commit_menu(2).iter().any(|i| i.label.starts_with("Interactive rebase main from")));
        // Rows: four, three, two, one. Rebase from "two".
        ui.key(Key::Down);
        ui.key(Key::Down);
        assert_eq!(&*ui.r.commits[ui.r.sel].subject, "two");
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        ui.key(Key::CtrlShift('r'));
        pump(&mut ui, "the plan", &|u| matches!(u.r.rebase, Some(rebase_view::RebaseState::Ready(_))));
        assert_partial_paint_is_full(&mut ui, &mut screen, "open the plan");
        // Row 0 "four": reword (the field starts with the subject).
        ui.key(Key::Char('r'));
        for c in ", reworded".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        assert_partial_paint_is_full(&mut ui, &mut screen, "reword");
        // Row 1 "three": fold into "two".
        ui.key(Key::Down);
        ui.key(Key::Char('f'));
        assert_partial_paint_is_full(&mut ui, &mut screen, "fixup");
        ui.pointer(700, 160);
        assert_partial_paint_is_full(&mut ui, &mut screen, "hover");
        ui.key(Key::CtrlEnter);
        assert!(ui.r.rebase.is_none(), "starting closes the plan");
        pump(&mut ui, "the rebase", &|u| u.notice.as_deref().is_some_and(|n| n.starts_with("Rebased")));
        let log = String::from_utf8(g.output(&["log", "--format=%s"]).unwrap()).unwrap();
        assert_eq!(log.lines().collect::<Vec<_>>(), ["four, reworded", "two", "one"]);
        // A new plan after the reload; Esc cancels it.
        pump(&mut ui, "the new history", &|u| u.r.log_done && u.r.commits.len() == 3 && u.r.refs.head_branch.is_some());
        ui.key(Key::CtrlShift('r'));
        pump(&mut ui, "a second plan", &|u| matches!(u.r.rebase, Some(rebase_view::RebaseState::Ready(_))));
        ui.key(Key::Esc);
        assert!(ui.r.rebase.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Submodules (FF-45, FF-54): the sidebar lists them; Move… moves one
    /// into a new folder; "Make the URL relative" rewrites .gitmodules;
    /// adding one on the same host as the remote offers a relative URL.
    #[test]
    fn submodules_move_relative_and_add_question() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-submodules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mk = |name: &str| {
            let g = Git { root: base.join(name) };
            std::fs::create_dir_all(&g.root).unwrap();
            for a in [
                &["init", "-q", "-b", "main"][..],
                &["config", "user.name", "ffwd test"],
                &["config", "user.email", "test@example.invalid"],
                &["config", "commit.gpgsign", "false"],
            ] {
                g.output(a).unwrap();
            }
            std::fs::write(g.root.join("f.txt"), name).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", "first"]).unwrap();
            g
        };
        let lib = mk("lib");
        let app = mk("app");
        let lib_url = lib.root.to_string_lossy().replace('\\', "/");
        app.run_with(
            &["-c", "protocol.file.allow=always", "submodule", "add", "--", &lib_url, "libs/lib"],
            None,
            &[],
            1 << 20,
        )
        .unwrap();
        app.output(&["commit", "-q", "-m", "add lib"]).unwrap();
        app.output(&["remote", "add", "origin", "https://example.invalid/team/app.git"]).unwrap();
        app.output(&["config", "-f", ".gitmodules", "submodule.libs/lib.url", "https://example.invalid/team/lib.git"])
            .unwrap();
        app.output(&["commit", "-q", "-am", "absolute URL"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(app.root.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, app.root.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let pick = |ui: &mut Ui, label: &str| {
            for _ in 0..40 {
                ui.key(Key::Down);
                let m = ui.menu.as_ref().unwrap();
                if m.sel.is_some_and(|s| m.items[s].label.starts_with(label)) {
                    ui.key(Key::Enter);
                    return;
                }
            }
            panic!("no menu item {label}");
        };
        let path = |u: &Ui| u.r.refs.submodules.first().map(|s| s.path.to_string()).unwrap_or_default();
        pump(&mut ui, "refs", &|u| u.r.log_done && !u.r.refs.submodules.is_empty());
        assert!(ui.r.side.iter().any(|i| i.key == "m:libs/lib"), "the sidebar lists the submodule");
        // Move it into a folder that does not exist yet.
        let items = ui.ref_menu("m:libs/lib");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Move");
        ui.key(Key::Ctrl('a'));
        for c in "vendor/lib".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        pump(&mut ui, "the move", &|u| u.dialog.is_none() && path(u) == "vendor/lib");
        assert!(app.root.join("vendor/lib/f.txt").exists());
        // Make its URL relative to the remote.
        let items = ui.ref_menu("m:libs/lib");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Make the URL relative");
        pump(&mut ui, "the relative URL", &|u| u.r.refs.submodules.first().is_some_and(|s| &*s.url == "../lib.git"));
        // Adding one on the same host asks about a relative URL.
        let items = ui.ref_menu("h:submodules");
        ui.open_menu(100, 100, items);
        pick(&mut ui, "Add submodule");
        for c in "https://example.invalid/team/other.git".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        let folder = match &ui.dialog.as_ref().unwrap().kind {
            DialogKind::Name { act: NameAct::AddSubmodulePath { .. }, field, .. } => field.text().to_string(),
            _ => panic!("the folder step follows the URL"),
        };
        assert_eq!(folder, "other");
        ui.key(Key::Enter);
        match &ui.dialog.as_ref().unwrap().kind {
            DialogKind::Ask(a) => assert!(a.choices[0].0.contains("../other.git"), "{:?}", a.choices[0].0),
            _ => panic!("a relative URL is offered"),
        }
        ui.key(Key::Esc);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// AI commit message end to end (FF-50) against a fake local server:
    /// ✦ AI shows the request and sends nothing until Send; the reply
    /// streams in; Use puts it in the message box. Without an endpoint the
    /// button only says that AI is off.
    #[test]
    fn ai_commit_message_preview_send_use() {
        use std::io::{Read as _, Write as _};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut s) = conn else { break };
                seen.fetch_add(1, Ordering::SeqCst);
                let mut req = Vec::new();
                let mut buf = [0u8; 8192];
                while let Ok(n) = s.read(&mut buf) {
                    req.extend_from_slice(&buf[..n]);
                    if let Some(end) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&req[..end]).to_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse().ok()))
                            .unwrap_or(0);
                        if req.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Add the greeting\\n\\nSo the app says hello.\"}}]}\n\ndata: [DONE]\n\n";
                let _ = s.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        let dir = std::env::temp_dir().join(format!("ffwd-ui-ai-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("hello.txt"), "hello\n").unwrap();
        g.output(&["add", "."]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done);
        ui.enter_changes();
        pump(&mut ui, "the changes", &|u| u.r.changes.status.is_some());
        // Off: the button says so and opens Settings; nothing is prepared.
        ui.ai_draft();
        assert!(ui.r.changes.ai.is_none());
        assert!(ui.notice.as_deref().is_some_and(|n| n.starts_with("AI is off")));
        ui.close_settings();
        ui.settings.set_string("ai.endpoint", &format!("http://127.0.0.1:{port}/v1"));
        ui.settings.set_string("ai.model", "test-model");
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        ui.ai_draft();
        pump(&mut ui, "the preview", &|u| u.r.changes.ai.as_ref().is_some_and(|a| a.stage == ai_view::Stage::Preview));
        assert_partial_paint_is_full(&mut ui, &mut screen, "the preview");
        assert_eq!(requests.load(Ordering::SeqCst), 0, "nothing is sent before Send");
        let a = ui.r.changes.ai.as_ref().unwrap();
        assert!(a.messages[1].content.contains("+hello"), "the preview holds the staged diff");
        assert!(a.endpoint.is_local());
        ui.key(Key::Enter);
        pump(&mut ui, "the reply", &|u| u.r.changes.ai.as_ref().is_some_and(|a| a.stage == ai_view::Stage::Done));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_partial_paint_is_full(&mut ui, &mut screen, "the reply");
        ui.key(Key::Enter);
        assert!(ui.r.changes.ai.is_none());
        assert_eq!(ui.r.changes.subject.text(), "Add the greeting");
        assert_eq!(ui.r.changes.desc.text(), "So the app says hello.");
        assert_partial_paint_is_full(&mut ui, &mut screen, "use the message");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Repositories (FF-44): clone from the palette (URL, then a suggested
    /// folder under the first root), create a new repository, and the tab
    /// menu (its file manager and terminal entries are listed, not run).
    #[test]
    fn clone_init_and_tab_menu() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-repos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let g = Git { root: src.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(src.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(src.clone(), &sinks(1));
        let mut settings = crate::settings::Settings::parse("");
        settings.set_string("repos.roots", &base.join("work").to_string_lossy());
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, settings, empty, Some((1, src.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done);
        // Clone: URL, then the folder under the first root.
        ui.run_command(CMD_CLONE);
        for c in src.to_string_lossy().replace('\\', "/").chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        let dest = match &ui.dialog.as_ref().unwrap().kind {
            DialogKind::Name { act: NameAct::CloneDest { .. }, field, .. } => PathBuf::from(field.text()),
            _ => panic!("the folder step follows the URL"),
        };
        assert_eq!(dest, base.join("work").join("src"));
        ui.key(Key::Enter);
        pump(&mut ui, "the clone", &|u| u.tabs.len() == 2);
        assert!(dest.join("a.txt").exists());
        // A new repository.
        ui.run_command(CMD_INIT);
        let dir = match &ui.dialog.as_ref().unwrap().kind {
            DialogKind::Name { act: NameAct::InitDir, field, .. } => PathBuf::from(field.text()),
            _ => panic!("the init dialog"),
        };
        assert_eq!(dir, base.join("work").join("new-repository"));
        ui.key(Key::Enter);
        pump(&mut ui, "the new repository", &|u| u.tabs.len() == 3);
        assert!(dir.join(".git").exists());
        // The tab menu; "Close other tabs" keeps the first.
        let labels: Vec<String> = ui.tab_menu(0).iter().map(|i| i.label.clone()).collect();
        assert!(
            labels.iter().any(|l| l == "Open in terminal") && labels.iter().any(|l| l == "Copy path"),
            "{labels:?}"
        );
        let items = ui.tab_menu(0);
        let k = items.iter().position(|i| i.label == "Close other tabs").unwrap();
        ui.open_menu(100, 60, items);
        ui.menu_activate(k);
        assert_eq!(ui.tabs.len(), 1);
        assert!(same_path(&ui.tabs[0].path, &src));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// History (FF-33): commits to push and to pull are marked; the title
    /// switches to the current branch only and back; the commit menu
    /// links to the hosting site (listed, never opened in tests).
    #[test]
    fn history_marks_scope_and_web_link() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("up")).unwrap();
        let cfg = |g: &Git| {
            for a in [
                &["config", "user.name", "ffwd test"][..],
                &["config", "user.email", "test@example.invalid"],
                &["config", "commit.gpgsign", "false"],
            ] {
                g.output(a).unwrap();
            }
        };
        let commit = |g: &Git, f: &str| {
            std::fs::write(g.root.join(f), f).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", f]).unwrap();
        };
        let up = Git { root: base.join("up") };
        up.output(&["init", "-q", "-b", "main"]).unwrap();
        cfg(&up);
        commit(&up, "a");
        Git { root: base.clone() }.output(&["clone", "-q", "up", "down"]).unwrap();
        let g = Git { root: base.join("down") };
        cfg(&g);
        commit(&up, "theirs");
        g.output(&["fetch", "-q"]).unwrap();
        commit(&g, "mine");
        // A side branch that the current-branch view leaves out.
        g.output(&["branch", "side", "HEAD~1"]).unwrap();
        g.output(&["switch", "-q", "side"]).unwrap();
        commit(&g, "side work");
        g.output(&["switch", "-q", "main"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(g.root.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, g.root.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let subjects = |u: &Ui, ids: &HashSet<Oid>| -> Vec<String> {
            u.r.commits.iter().filter(|c| ids.contains(&c.id)).map(|c| c.subject.to_string()).collect()
        };
        pump(&mut ui, "history and marks", &|u| u.r.log_done && !u.r.ahead_ids.is_empty() && u.r.commits.len() == 4);
        assert_eq!(subjects(&ui, &ui.r.ahead_ids), ["mine"]);
        assert_eq!(subjects(&ui, &ui.r.behind_ids), ["theirs"]);
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        // The title toggles the current branch only: "side work" and the
        // upstream's commit leave the list.
        let t = ui.scope_rect;
        ui.click(t.x + 4, t.y + 4);
        let k = ui.menu.as_ref().and_then(|m| m.items.iter().position(|i| i.label.ends_with("Current branch only")));
        ui.menu_activate(k.expect("the title opens its menu"));
        pump(&mut ui, "the current branch", &|u| u.r.log_done && u.r.commits.len() == 2);
        assert!(ui.r.commits.iter().all(|c| &*c.subject != "side work"));
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        ui.toggle_scope();
        pump(&mut ui, "all again", &|u| u.r.log_done && u.r.commits.len() == 4);
        // The commit menu links to the hosting site.
        g.output(&["remote", "set-url", "origin", "git@github.com:someone/project.git"]).unwrap();
        ui.reload_current();
        pump(&mut ui, "the new remote", &|u| {
            u.r.log_done && u.r.refs.remotes.first().is_some_and(|r| r.fetch_url.contains("github"))
        });
        let labels: Vec<String> = ui.commit_menu(0).iter().map(|i| i.label.clone()).collect();
        assert!(labels.iter().any(|l| l == "Open on github.com"), "{labels:?}");
        let link = ui.commit_menu(0).into_iter().find(|i| i.label == "Copy web link").and_then(|i| i.act);
        let sha = ui.r.commits[0].id.to_string();
        assert!(
            matches!(link, Some(menu::MenuAct::Copy(u)) if u == format!("https://github.com/someone/project/commit/{sha}"))
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Patches (FF-46): save a commit as a patch file from its menu,
    /// undo the commit, apply the file (checked, then asked), and a patch
    /// that no longer fits offers a 3-way apply. The clipboard is not used
    /// here: tests never touch the owner's clipboard.
    #[test]
    fn patches_save_and_apply_from_a_file() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-patches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("repo");
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "one\nTWO\n").unwrap();
        g.output(&["commit", "-q", "-am", "Shout two"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let read = || std::fs::read_to_string(dir.join("a.txt")).unwrap();
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 2);
        // Save the newest commit as a patch (the dialog suggests a name).
        let items = ui.commit_menu(0);
        let k = items.iter().position(|i| i.label == "Save as patch…").unwrap();
        ui.open_menu(100, 100, items);
        ui.menu_activate(k);
        let suggested = match &ui.dialog.as_ref().unwrap().kind {
            DialogKind::Name { act: NameAct::SavePatch { .. }, field, .. } => field.text().to_string(),
            _ => panic!("the save dialog"),
        };
        assert!(suggested.ends_with("-shout-two.patch"), "{suggested}");
        let file = base.join("out").join("x.patch");
        ui.key(Key::Ctrl('a'));
        for c in file.to_string_lossy().chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        pump(&mut ui, "the saved patch", &|u| u.notice.as_deref().is_some_and(|n| n.starts_with("Saved the patch")));
        assert!(std::fs::read_to_string(&file).unwrap().contains("Subject: [PATCH] Shout two"));
        // Undo the commit, then apply the file: checked, asked, applied.
        g.output(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        ui.run_command(CMD_APPLY_FILE);
        for c in file.to_string_lossy().chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        pump(&mut ui, "the check", &|u| {
            u.dialog.as_ref().is_some_and(|d| matches!(&d.kind, DialogKind::Ask(a) if a.title == "Apply the patch?"))
        });
        ui.key(Key::Enter);
        pump(&mut ui, "the apply", &|u| {
            u.dialog.is_none() && u.notice.as_deref().is_some_and(|n| n.starts_with("Applied"))
        });
        assert_eq!(read(), "one\nTWO\n");
        // A patch that no longer fits offers the 3-way apply.
        g.output(&["checkout", "--", "a.txt"]).unwrap();
        std::fs::write(dir.join("a.txt"), "one\nzwei\n").unwrap();
        g.output(&["commit", "-q", "-am", "theirs"]).unwrap();
        ui.check_patch(std::fs::read_to_string(&file).unwrap());
        pump(&mut ui, "the failed check", &|u| {
            u.dialog
                .as_ref()
                .is_some_and(|d| matches!(&d.kind, DialogKind::Ask(a) if a.title.contains("does not apply")))
        });
        ui.key(Key::Enter);
        pump(&mut ui, "the 3-way apply", &|u| u.dialog.is_none() && read().contains("<<<<<<<"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// File menus (FF-47): a right click on a Local Changes row offers the
    /// editor, the default app, the folder and the diff tool; the diff tool
    /// runs (a repository-local test tool that only exits). Nothing that
    /// opens a window on the owner's screen runs here.
    #[test]
    fn file_menus_and_external_diff_tool() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        let ok = if cfg!(windows) { "cmd /c exit 0" } else { "true" };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "diff.guitool", "ffwdtest"],
            &["config", "difftool.ffwdtest.cmd", ok],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "b\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.git.is_some());
        ui.enter_changes();
        pump(&mut ui, "the changes", &|u| u.r.changes.status.as_ref().is_some_and(|s| !s.entries.is_empty()));
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let row = ui.r.changes.hits.iter().find(|h| matches!(h.1, changes_view::CHit::Row(..))).map(|h| h.0).unwrap();
        let (x, y) = (row.x + row.w / 2, row.y + row.h / 2);
        ui.right_click(x, y);
        let labels: Vec<String> = ui.menu.as_ref().unwrap().items.iter().map(|i| i.label.clone()).collect();
        for want in ["Open in editor", "Open with the default app", "Open in the diff tool", "Copy full path"] {
            assert!(labels.iter().any(|l| l == want), "{want} missing: {labels:?}");
        }
        let k = labels.iter().position(|l| l == "Open in the diff tool").unwrap();
        ui.menu_activate(k);
        pump(&mut ui, "the diff tool", &|u| u.notice.as_deref().is_some_and(|n| n.starts_with("Closed ffwdtest")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Diff navigation (FF-41): n and p jump between blocks of changes,
    /// in unified and side-by-side mode; past the last one ffwd says so.
    #[test]
    fn diff_next_and_previous_change() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-diffnav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let lines: Vec<String> = (1..=120).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.join("a.txt"), lines.join("\n") + "\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        let mut changed = lines.clone();
        changed[9] = "CHANGED 10".into();
        changed[99] = "CHANGED 100".into();
        std::fs::write(dir.join("a.txt"), changed.join("\n") + "\n").unwrap();
        g.output(&["commit", "-q", "-am", "two changes"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the diff", &|u| u.r.log_done && u.r.diff.is_some());
        ui.focus = Focus::Diff;
        for split in [false, true] {
            ui.diff_split = split;
            ui.r.diff_top = 0;
            let starts = ui.change_starts();
            assert_eq!(starts.len(), 2, "split {split}: {starts:?}");
            ui.key(Key::Char('n'));
            assert_eq!(ui.r.diff_top, starts[0] - 3, "split {split}");
            ui.key(Key::Char('n'));
            assert_eq!(ui.r.diff_top, starts[1] - 3, "split {split}");
            ui.notice = None;
            ui.key(Key::Char('n'));
            assert_eq!(ui.notice.as_deref(), Some("No more changes below"));
            ui.key(Key::Char('p'));
            assert_eq!(ui.r.diff_top, starts[0] - 3, "split {split}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pull requests (FF-40): listed in the sidebar (fed in as the GitHub
    /// CLI would answer), with a menu; "Check out as pr/<n>" fetches
    /// GitHub's pull/<n>/head (here from a local stand-in) and switches.
    #[test]
    fn pull_requests_listed_and_checked_out() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-prs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let g = Git { root: base.join("work") };
        std::fs::create_dir_all(&g.root).unwrap();
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(g.root.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        Git { root: base.clone() }.output(&["init", "-q", "--bare", "hub.git"]).unwrap();
        g.output(&["remote", "add", "origin", &base.join("hub.git").to_string_lossy()]).unwrap();
        g.output(&["push", "-q", "origin", "main"]).unwrap();
        std::fs::write(g.root.join("b.txt"), "b").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "pr work"]).unwrap();
        g.output(&["push", "-q", "origin", "HEAD:refs/pull/7/head"]).unwrap();
        g.output(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(g.root.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, g.root.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "refs", &|u| u.r.log_done && u.r.refs.head_branch.is_some());
        assert!(ui.r.prs.is_none(), "tests never ask the hosting site");
        let pr = ffwd_core::hosting::PullRequest {
            number: 7,
            title: "Add b".into(),
            head: "feat/b".into(),
            author: "alice".into(),
            draft: true,
            url: "https://github.com/o/r/pull/7".into(),
        };
        ui.on_msg(Msg::PullRequests(Ok(vec![pr])));
        let it = ui.r.side.iter().find(|i| i.key == "p:7").expect("listed in the sidebar");
        assert_eq!((it.label.as_str(), it.badge.as_str()), ("#7 Add b", "draft"));
        let labels: Vec<String> = ui.ref_menu("p:7").iter().map(|i| i.label.clone()).collect();
        assert!(labels.iter().any(|l| l == "Open in the browser"), "{labels:?}");
        let items = ui.ref_menu("p:7");
        let k = items.iter().position(|i| i.label == "Check out as pr/7").unwrap();
        ui.open_menu(100, 100, items);
        ui.menu_activate(k);
        pump(&mut ui, "the checkout", &|u| u.r.refs.head_branch.as_deref() == Some("pr/7"));
        assert!(g.root.join("b.txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The reflog (FF-43): a commit lost by a hard reset shows as "lost";
    /// its menu creates a branch that brings it back; partial repaints
    /// match full ones.
    #[test]
    fn reflog_recovers_a_lost_commit() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-reflog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        for m in ["one", "precious work"] {
            std::fs::write(dir.join("a.txt"), m).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        }
        let lost = String::from_utf8(g.output(&["rev-parse", "HEAD"]).unwrap()).unwrap().trim().to_string();
        g.output(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 1 && u.r.refs.head_branch.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        ui.enter_reflog();
        pump(&mut ui, "the reflog", &|u| u.r.reflog.entries.is_some());
        assert_partial_paint_is_full(&mut ui, &mut screen, "the reflog");
        let entries = ui.r.reflog.entries.clone().unwrap().unwrap();
        let k = entries.iter().position(|e| e.id.to_string() == lost).expect("the lost commit is in the reflog");
        assert!(!ui.r.by_id.contains_key(&entries[k].id), "it is on no branch");
        // Its menu: Create branch here…, named "rescue".
        ui.key(Key::Home);
        for _ in 0..k {
            ui.key(Key::Down);
        }
        assert_partial_paint_is_full(&mut ui, &mut screen, "select the lost commit");
        let items = ui.reflog_menu(k);
        assert!(!items.iter().any(|i| i.label == "Show in history"), "not in the loaded history");
        let c = items.iter().position(|i| i.label == "Create branch here…").unwrap();
        ui.open_menu(100, 100, items);
        ui.menu_activate(c);
        for ch in "rescue".chars() {
            ui.key(Key::Char(ch));
        }
        ui.key(Key::Tab); // do not check it out
        ui.key(Key::Enter);
        pump(&mut ui, "the branch", &|u| u.dialog.is_none() && u.r.refs.refs.iter().any(|r| &*r.name == "rescue"));
        let tip = String::from_utf8(g.output(&["rev-parse", "rescue"]).unwrap()).unwrap();
        assert_eq!(tip.trim(), lost);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CI status (FF-24): runs by commit (fed in as the GitHub CLI would
    /// answer) mark the history row and add "Open the CI run" to the
    /// commit menu; the repaint is partial and correct.
    #[test]
    fn ci_status_marks_commits() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-ci-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        let head = String::from_utf8(g.output(&["rev-parse", "HEAD"]).unwrap()).unwrap().trim().to_string();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 1);
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        ui.on_msg(Msg::CiRuns(vec![(head, ffwd_core::hosting::Ci::Failed, "https://example.invalid/run/1".into())]));
        assert_partial_paint_is_full(&mut ui, &mut screen, "CI status");
        let labels: Vec<String> = ui.commit_menu(0).iter().map(|i| i.label.clone()).collect();
        assert!(labels.iter().any(|l| l == "Open the CI run (failed)"), "{labels:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Remote branches and tags (FF-36): push a branch and a tag from their
    /// menus; deleting a branch on the remote asks first and names the way
    /// back.
    #[test]
    fn push_and_delete_on_the_remote_from_menus() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-rrefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let g = Git { root: base.join("work") };
        std::fs::create_dir_all(&g.root).unwrap();
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(g.root.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        g.output(&["branch", "topic"]).unwrap();
        g.output(&["tag", "v1"]).unwrap();
        let hub = base.join("hub.git");
        Git { root: base.clone() }.output(&["init", "-q", "--bare", "hub.git"]).unwrap();
        g.output(&["remote", "add", "origin", &hub.to_string_lossy()]).unwrap();
        let on_hub = |r: &str| Git { root: hub.clone() }.output(&["rev-parse", "--verify", "--quiet", r]).is_ok();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(g.root.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, g.root.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let run = |ui: &mut Ui, key: &str, label: &str| {
            let items = ui.ref_menu(key);
            let k = items.iter().position(|i| i.label.starts_with(label)).unwrap_or_else(|| {
                panic!("{label} missing: {:?}", items.iter().map(|i| i.label.clone()).collect::<Vec<_>>())
            });
            ui.open_menu(100, 100, items);
            ui.menu_activate(k);
        };
        pump(&mut ui, "refs", &|u| u.r.log_done && !u.r.refs.remotes.is_empty());
        run(&mut ui, "b:topic", "Push topic to origin");
        pump(&mut ui, "the branch push", &|u| u.notice.as_deref() == Some("Pushed topic to origin"));
        assert!(on_hub("refs/heads/topic"));
        // The reload after an operation brings the remotes back first.
        pump(&mut ui, "refs again", &|u| u.r.log_done && !u.r.refs.remotes.is_empty());
        run(&mut ui, "t:v1", "Push tag v1 to origin");
        pump(&mut ui, "the tag push", &|u| u.notice.as_deref() == Some("Pushed tag v1 to origin"));
        assert!(on_hub("refs/tags/v1"));
        pump(&mut ui, "the remote branch", &|u| u.r.side.iter().any(|i| i.key == "r:origin/topic"));
        run(&mut ui, "r:origin/topic", "Delete topic on origin");
        assert!(ui.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Ask(_))), "it asks first");
        ui.key(Key::Enter);
        pump(&mut ui, "the remote delete", &|u| {
            u.notice.as_deref().is_some_and(|n| n.starts_with("Deleted topic on origin"))
        });
        assert!(!on_hub("refs/heads/topic"));
        assert!(ui.notice.as_deref().is_some_and(|n| n.contains("git push origin ")));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Several refs at once (FF-36): Ctrl+click and Shift+click pick
    /// branches and tags in the sidebar; their menu pushes them all in one
    /// push and deletes them after asking; a plain click clears the pick.
    #[test]
    fn several_refs_from_the_sidebar() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-many-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let g = Git { root: base.join("work") };
        std::fs::create_dir_all(&g.root).unwrap();
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(g.root.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        for a in [&["branch", "topic"][..], &["tag", "v1"], &["tag", "v2"], &["tag", "v3"]] {
            g.output(a).unwrap();
        }
        let hub = base.join("hub.git");
        Git { root: base.clone() }.output(&["init", "-q", "--bare", "hub.git"]).unwrap();
        g.output(&["remote", "add", "origin", &hub.to_string_lossy()]).unwrap();
        let on_hub = |r: &str| Git { root: hub.clone() }.output(&["rev-parse", "--verify", "--quiet", r]).is_ok();
        let here = |r: &str| g.output(&["rev-parse", "--verify", "--quiet", r]).is_ok();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(g.root.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, g.root.clone())), sinks, 1280, 900, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "refs", &|u| u.r.log_done && u.r.side.iter().any(|i| i.key == "t:v3"));
        let mut screen = vec![0u32; 1280 * 900];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 900), true);
        let click = |ui: &mut Ui, key: &str, mods: (bool, bool)| {
            let i = ui.r.side.iter().position(|it| it.key == key).unwrap();
            let l = ui.side_list_rect();
            let rh = ui.side_row_h();
            let y = l.y + (i - ui.r.side_top) as i32 * rh + rh / 2;
            ui.click_mods = mods;
            ui.click(l.x + l.w / 2, y);
            // Not a double click next time.
            ui.last_click = None;
        };
        // Select v1, Shift+click v3 (a range), Ctrl+click topic.
        click(&mut ui, "t:v1", (false, false));
        click(&mut ui, "t:v3", (false, true));
        click(&mut ui, "b:topic", (true, false));
        let picked: Vec<&str> = ui.r.side_multi.iter().map(String::as_str).collect();
        assert_eq!(picked, ["b:topic", "t:v1", "t:v2", "t:v3"]);
        assert_partial_paint_is_full(&mut ui, &mut screen, "picked");
        // Ctrl+click again takes one out.
        click(&mut ui, "t:v2", (true, false));
        assert!(!ui.r.side_multi.contains("t:v2"));
        let run = |ui: &mut Ui, key: &str, label: &str| {
            let items = ui.ref_menu(key);
            let k = items.iter().position(|i| i.label.starts_with(label)).unwrap_or_else(|| {
                panic!("{label} missing: {:?}", items.iter().map(|i| i.label.clone()).collect::<Vec<_>>())
            });
            ui.open_menu(100, 100, items);
            ui.menu_activate(k);
        };
        run(&mut ui, "t:v1", "Push 1 branch and 2 tags to origin");
        pump(&mut ui, "the push", &|u| u.notice.as_deref() == Some("Pushed 1 branch and 2 tags to origin"));
        assert!(
            on_hub("refs/heads/topic") && on_hub("refs/tags/v1") && on_hub("refs/tags/v3") && !on_hub("refs/tags/v2")
        );
        pump(&mut ui, "refs again", &|u| u.r.log_done && !u.r.refs.remotes.is_empty());
        assert_eq!(ui.r.side_multi.len(), 3, "the pick survives the reload");
        run(&mut ui, "b:topic", "Delete 1 branch and 2 tags");
        assert!(ui.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Ask(_))), "it asks first");
        ui.key(Key::Enter);
        pump(&mut ui, "the delete", &|u| {
            u.notice.as_deref().is_some_and(|n| n.starts_with("Deleted 1 branch and 2 tags."))
        });
        assert!(!here("refs/heads/topic") && !here("refs/tags/v1") && here("refs/tags/v2"));
        assert!(ui.notice.as_deref().is_some_and(|n| n.contains("git tag v1 ")));
        pump(&mut ui, "refs after the delete", &|u| {
            u.r.log_done && !u.r.side.iter().any(|i| i.key == "t:v1") && u.r.side.iter().any(|i| i.key == "t:v2")
        });
        assert!(ui.r.side_multi.is_empty(), "deleted refs leave the pick");
        // Clickable too: the menu picks, and a plain click clears.
        run(&mut ui, "h:tags", "Select all tags");
        assert_eq!(ui.r.side_multi.len(), 1);
        run(&mut ui, "b:main", "Select for several");
        assert_eq!(ui.r.side_multi.len(), 2);
        click(&mut ui, "t:v2", (false, false));
        assert!(ui.r.side_multi.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Stashes in the history (FF-33): a stash is a row on its base
    /// commit with a stash chip and the stash's menu; the title's menu
    /// hides them; the sidebar's stash jumps to its row.
    #[test]
    fn stashes_show_in_the_history() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-stashrows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        for (k, m) in ["one", "two"].iter().enumerate() {
            std::fs::write(dir.join("a.txt"), format!("{k}")).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "wip").unwrap();
        g.output(&["stash", "push", "-q", "-m", "half done"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && !u.r.refs.stashes.is_empty() && u.r.commits.len() == 3);
        let row = ui.r.commits.iter().position(|c| c.subject.contains("half done")).expect("the stash is a row");
        let id = ui.r.commits[row].id;
        assert_eq!(ui.r.commits[row].parents.len(), 1, "on its base only");
        let chips = ui.r.chips.get(&id).cloned().unwrap_or_default();
        assert!(chips.iter().any(|&i| ui.r.refs.refs[i as usize].kind == RefKind::Stash), "a stash chip");
        let labels: Vec<String> = ui.commit_menu(row).iter().map(|i| i.label.clone()).collect();
        assert!(labels.iter().any(|l| l == "Apply stash@{0}"), "{labels:?}");
        assert!(!labels.iter().any(|l| l.starts_with("Cherry-pick")), "{labels:?}");
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        // The sidebar's stash jumps to its row.
        ui.select(0);
        let i = ui.r.side.iter().position(|it| it.key == "s:stash@{0}").unwrap();
        let l = ui.side_list_rect();
        let y = l.y + (i - ui.r.side_top) as i32 * ui.side_row_h() + ui.side_row_h() / 2;
        ui.click(l.x + l.w / 2, y);
        assert_eq!(ui.r.sel, row);
        assert_partial_paint_is_full(&mut ui, &mut screen, "stash selected");
        // The title's menu hides stashes.
        let t = ui.scope_rect;
        ui.click(t.x + 4, t.y + 4);
        let k = ui.menu.as_ref().and_then(|m| m.items.iter().position(|i| i.label.ends_with("Show stashes")));
        ui.menu_activate(k.expect("the title's menu has Show stashes"));
        assert_eq!(ui.settings.get("history.stashes"), Some("false"));
        pump(&mut ui, "history without stashes", &|u| u.r.log_done && u.r.commits.len() == 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Automatic fetch (FF-39): when the clock says it is due, the open
    /// repository is fetched without a notice; news show and reload the
    /// history; the setting turns it off; a Fetch click while it runs
    /// takes it over.
    #[test]
    fn automatic_fetch_is_quiet_until_news() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-autofetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let top = Git { root: base.clone() };
        top.output(&["init", "-q", "--bare", "-b", "main", "hub.git"]).unwrap();
        let mk = |name: &str| {
            top.output(&["clone", "-q", "hub.git", name]).unwrap();
            let g = Git { root: base.join(name) };
            for a in [
                &["config", "user.name", "ffwd test"][..],
                &["config", "user.email", "test@example.invalid"],
                &["config", "commit.gpgsign", "false"],
            ] {
                g.output(a).unwrap();
            }
            g
        };
        let other = mk("other");
        std::fs::write(other.root.join("a.txt"), "1").unwrap();
        other.output(&["add", "."]).unwrap();
        other.output(&["commit", "-q", "-m", "first"]).unwrap();
        other.output(&["push", "-q", "origin", "HEAD:refs/heads/main"]).unwrap();
        let mine = mk("mine");
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(mine.root.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, mine.root.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && !u.r.refs.remotes.is_empty() && u.r.commits.len() == 1);
        assert_eq!(ui.auto_fetch_setting(), 10, "on by default, every 10 minutes");
        // Nothing new: no notice at all.
        ui.notice = None;
        ui.on_tab_msg(0, Msg::AutoFetchTick);
        assert_eq!(ui.r.op, Some(Op::AutoFetch));
        assert!(ui.notice.is_none(), "it starts quietly");
        pump(&mut ui, "the quiet fetch", &|u| u.r.op.is_none());
        assert!(ui.notice.is_none(), "nothing new, nothing said: {:?}", ui.notice);
        // News: they show and the history reloads.
        std::fs::write(other.root.join("a.txt"), "2").unwrap();
        other.output(&["commit", "-qam", "second"]).unwrap();
        other.output(&["push", "-q", "origin", "HEAD:refs/heads/main"]).unwrap();
        ui.on_tab_msg(0, Msg::AutoFetchTick);
        pump(&mut ui, "the news", &|u| u.notice.as_deref().is_some_and(|n| n.ends_with("(automatic fetch)")));
        pump(&mut ui, "the reload", &|u| u.r.log_done && u.r.commits.len() == 2);
        // A Fetch click while it runs takes it over: its result shows.
        ui.on_tab_msg(0, Msg::AutoFetchTick);
        ui.start_op(Op::Fetch);
        assert_eq!(ui.r.op, Some(Op::Fetch));
        pump(&mut ui, "the user's fetch", &|u| {
            u.r.op.is_none() && u.notice.as_deref().is_some_and(|n| n.starts_with("Fetched: everything is up to date"))
        });
        // Off: the clock's tick does nothing.
        ui.settings.set_string("remotes.auto_fetch", "0");
        ui.on_tab_msg(0, Msg::AutoFetchTick);
        assert!(ui.r.op.is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Image diffs (FF-41): a changed PNG shows its old and new version
    /// (decoded on a worker) instead of "binary file"; repaints are right.
    #[test]
    fn image_diff_shows_before_and_after() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-image-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        for (n, m) in [(24usize, "small"), (40, "large")] {
            crate::snapshot::write_rgba_png(&dir.join("logo.png"), &crate::icon::mark_rgba(n), n, n).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let mut screen = vec![0u32; 1280 * 800];
        pump(&mut ui, "the diff", &|u| u.r.log_done && u.r.diff.as_ref().is_some_and(|d| d.binary));
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        pump(&mut ui, "the images", &|u| u.r.diff_images.as_ref().is_some_and(|x| x.0 == u.r.seq));
        let (_, old, new) = ui.r.diff_images.clone().unwrap();
        assert_eq!(old.unwrap().unwrap().orig, (24, 24));
        assert_eq!(new.unwrap().unwrap().orig, (40, 40));
        assert_partial_paint_is_full(&mut ui, &mut screen, "the images");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Recent queries (FF-34): Enter and Esc remember the query; in an
    /// empty field ↑ walks back through them and ↓ forward, past the newest
    /// to empty again.
    #[test]
    fn find_bar_recent_queries() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done);
        let input = |u: &Ui| u.r.search.as_ref().map(|s| s.input.text().to_string()).unwrap_or_default();
        for q in ["first", "second"] {
            ui.key(Key::Ctrl('f'));
            for c in q.chars() {
                ui.key(Key::Char(c));
            }
            ui.key(Key::Enter);
            ui.key(Key::Esc);
        }
        assert_eq!(ui.state.get_list("search.recent"), ["second", "first"]);
        ui.key(Key::Ctrl('f'));
        assert_eq!(input(&ui), "");
        ui.key(Key::Up);
        assert_eq!(input(&ui), "second");
        ui.key(Key::Up);
        assert_eq!(input(&ui), "first");
        ui.key(Key::Up);
        assert_eq!(input(&ui), "first", "the oldest stays");
        ui.key(Key::Down);
        assert_eq!(input(&ui), "second");
        ui.key(Key::Down);
        assert_eq!(input(&ui), "", "past the newest the field is empty again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Conflicts in several files (FF-42): "All theirs…" asks first, then
    /// takes theirs in every conflicted file and marks them resolved.
    #[test]
    fn take_theirs_for_all_conflicted_files() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-takeall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        write("a.txt", "base\n");
        write("b.txt", "base\n");
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "base"]).unwrap();
        g.output(&["switch", "-q", "-c", "other"]).unwrap();
        write("a.txt", "theirs a\n");
        write("b.txt", "theirs b\n");
        g.output(&["commit", "-q", "-am", "theirs"]).unwrap();
        g.output(&["switch", "-q", "main"]).unwrap();
        write("a.txt", "ours a\n");
        write("b.txt", "ours b\n");
        g.output(&["commit", "-q", "-am", "ours"]).unwrap();
        assert!(g.output(&["merge", "-q", "other"]).is_err(), "the merge stops with conflicts");
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let conflicted =
            |u: &Ui| u.r.changes.status.as_ref().map_or(0, |s| s.entries.iter().filter(|e| e.conflicted).count());
        pump(&mut ui, "history", &|u| u.r.log_done);
        ui.enter_changes();
        pump(&mut ui, "the conflicts", &|u| conflicted(u) == 2 && u.r.changes.in_progress.is_some());
        ui.ask_take_all(false);
        let title = match &ui.dialog.as_ref().unwrap().kind {
            DialogKind::Ask(a) => a.title.clone(),
            _ => panic!("it asks first"),
        };
        assert!(title.contains("all 2 conflicted files"), "{title}");
        ui.key(Key::Enter);
        pump(&mut ui, "the resolution", &|u| u.dialog.is_none() && u.r.changes.status.is_some() && conflicted(u) == 0);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "theirs a\n");
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "theirs b\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Editing a conflict by hand (FF-42): `e` opens the conflict with
    /// ours then theirs, typing replaces it, Ctrl+Enter keeps the edit
    /// (not a commit), Enter saves the file resolved; repaints match.
    #[test]
    fn edit_a_conflict_by_hand() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-conflict-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "t"],
            &["config", "user.email", "t@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let commit = |t: &str, m: &str| {
            std::fs::write(dir.join("a.txt"), t).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        };
        commit("top\nbase\nend\n", "base");
        g.output(&["switch", "-q", "-c", "feature"]).unwrap();
        commit("top\nfeature\nend\n", "feature");
        g.output(&["switch", "-q", "main"]).unwrap();
        commit("top\nmain\nend\n", "main");
        assert!(g.output(&["merge", "feature"]).is_err());
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the repository", &|u| u.r.git.is_some());
        ui.enter_changes();
        pump(&mut ui, "the conflict view", &|u| u.r.changes.conflict.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        ui.key(Key::Char('e'));
        let start = ui.r.changes.conflict.as_ref().and_then(|c| c.editing.as_ref()).map(|e| e.1.text());
        assert_eq!(start.as_deref(), Some("main\nfeature"), "the edit starts with ours then theirs");
        assert_partial_paint_is_full(&mut ui, &mut screen, "start editing");
        ui.key(Key::Ctrl('a'));
        for c in "merged by hand".chars() {
            ui.key(Key::Char(c));
        }
        assert_partial_paint_is_full(&mut ui, &mut screen, "typing");
        ui.key(Key::CtrlEnter);
        let c = ui.r.changes.conflict.as_ref().unwrap();
        assert!(c.editing.is_none() && c.resolved() == 1, "Ctrl+Enter keeps the edit");
        let head = String::from_utf8(g.output(&["rev-parse", "HEAD"]).unwrap()).unwrap();
        ui.key(Key::Enter);
        pump(&mut ui, "the resolution", &|u| {
            u.r.changes.busy.is_none()
                && u.r.changes.status.as_ref().is_some_and(|s| s.entries.iter().all(|e| !e.conflicted))
        });
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "top\nmerged by hand\nend\n");
        let still = String::from_utf8(g.output(&["rev-parse", "HEAD"]).unwrap()).unwrap();
        assert_eq!(head, still, "nothing was committed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The diff's overview strip (FF-41): marks where the changes are
    /// (here near the top and the bottom); a click near the bottom jumps
    /// there; repaints match full paints.
    #[test]
    fn diff_strip_marks_and_jumps() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-strip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let lines: Vec<String> = (1..=300).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.join("a.txt"), lines.join("\n") + "\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        let mut changed = lines.clone();
        changed[4] = "CHANGED 5".into();
        changed[294] = "CHANGED 295".into();
        std::fs::write(dir.join("a.txt"), changed.join("\n") + "\n").unwrap();
        g.output(&["commit", "-q", "-am", "two changes"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the diff", &|u| u.r.log_done && u.r.diff.is_some());
        ui.diff_opts.context = u32::MAX; // the whole file, so the changes sit far apart
        ui.r.diff = None;
        ui.apply_diff_bar(diff_view::DiffBar::Context(u32::MAX));
        pump(&mut ui, "the whole-file diff", &|u| u.r.diff.as_ref().is_some_and(|d| d.lines.len() > 250));
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let h = ui.diff_strip.h as usize;
        assert!(h > 50, "the strip is painted");
        let marks = ui.diff_marks(h);
        let first = marks.iter().position(|&m| m != 0).unwrap();
        let last = marks.iter().rposition(|&m| m != 0).unwrap();
        assert!(first < h / 10 && last > h * 9 / 10, "marks near the top and the bottom: {first} {last} of {h}");
        let near = marks[first] | marks[(first + 1).min(h - 1)] | marks[(first + 2).min(h - 1)];
        assert_eq!(near, 3, "a changed line is removed and added: {:?}", &marks[first..(first + 3).min(h)]);
        let s = ui.diff_strip;
        ui.click(s.x + 2, s.y + (last as i32));
        assert!(ui.r.diff_top > 200, "the click jumps near the bottom: {}", ui.r.diff_top);
        assert_partial_paint_is_full(&mut ui, &mut screen, "strip click");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Tab names and colours (FF-44): rename and colour a tab from its
    /// menu; both are saved with the session and come back on restore; an
    /// empty name shows the folder's name again.
    #[test]
    fn tab_rename_and_colour_survive_a_restart() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-tabstyle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Git { root: dir.clone() }.output(&["init", "-q", "-b", "main"]).unwrap();
        let (tx, _rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        let empty = crate::settings::Settings::parse("");
        let mut ui =
            Ui::new(te, f, empty.clone(), empty.clone(), Some((1, dir.clone())), sinks.clone(), 1280, 800, 1.0);
        let items = ui.tab_menu(0);
        let k = items.iter().position(|i| i.label == "Rename tab…").unwrap();
        ui.open_menu(100, 60, items);
        ui.menu_activate(k);
        ui.key(Key::Ctrl('a'));
        for c in "Work".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        assert_eq!(ui.tabs[0].title(), "Work");
        assert_eq!(ui.window_title(), "Work · ffwd");
        let items = ui.tab_menu(0);
        let k = items.iter().position(|i| i.label == "Blue").unwrap();
        ui.open_menu(100, 60, items);
        ui.menu_activate(k);
        assert_eq!(ui.tabs[0].color, Some(3));
        assert_eq!(ui.state.get_list("session.tab_style").len(), 1);
        // A restart with that session: the name and colour come back.
        let state = ui.state.clone();
        let mut te2 = TextEngine::new();
        let Some(f2) = load_fonts(&mut te2) else { return };
        let mut ui2 = Ui::new(te2, f2, empty.clone(), state, None, sinks, 1280, 800, 1.0);
        assert_eq!(ui2.tabs.len(), 1);
        assert_eq!((ui2.tabs[0].title(), ui2.tabs[0].color), ("Work", Some(3)));
        // An empty name goes back to the folder's.
        ui2.rename_tab(0, "  ");
        assert_eq!(ui2.tabs[0].title(), ui2.tabs[0].name);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Matching files (FF-34): a content search marks the files that
    /// matched in the selected commit and opens the first of them; closing
    /// the search drops the marks; repaints match full paints.
    #[test]
    fn find_marks_the_matching_files() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-findfiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        for (files, msg) in [
            (&[("a.txt", "x\n"), ("b.txt", "x\n"), ("c.txt", "x\n")][..], "Start"),
            (&[("a.txt", "y\n"), ("b.txt", "y\n"), ("c.txt", "the needle\n")][..], "Hide the needle"),
            (&[("a.txt", "z\n")][..], "Later"),
        ] {
            for (file, text) in files {
                std::fs::write(dir.join(file), text).unwrap();
            }
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", msg]).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 3 && u.r.diff.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        // Message → Author → SHA → Content, then Enter.
        ui.key(Key::Ctrl('f'));
        for _ in 0..3 {
            ui.key(Key::Tab);
        }
        for c in "needle".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        let shown = |u: &Ui| {
            u.r.details.as_ref().is_some_and(|d| d.message.starts_with("Hide the needle"))
                && u.r.diff.as_ref().is_some_and(|d| &*d.path == "c.txt")
        };
        pump(&mut ui, "the match and its file", &|u| u.r.search.as_ref().is_some_and(|s| s.done) && shown(u));
        let id = ui.r.details.as_ref().unwrap().id;
        assert_eq!(ui.match_files(id), Some(&["c.txt".to_string()][..]), "only the file with the text matched");
        assert_eq!(ui.r.sel_file, 2, "the matching file is selected, not the first");
        assert_partial_paint_is_full(&mut ui, &mut screen, "matching files");
        ui.key(Key::Esc);
        assert_eq!(ui.match_files(id), None);
        assert_partial_paint_is_full(&mut ui, &mut screen, "search closed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The diff's scroll per file (FF-41): a file shown again comes back
    /// where it was; an option change keeps the scroll; another commit's
    /// file starts at the top.
    #[test]
    fn diff_scroll_is_remembered_per_file() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-diffmem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let long: Vec<String> = (1..=300).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.join("a.txt"), long.join("\n") + "\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), long[..100].join("\n") + "\n").unwrap();
        g.output(&["commit", "-q", "-am", "shorter"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let on = |p: &'static str| move |u: &Ui| u.r.diff.as_ref().is_some_and(|d| &*d.path == p);
        // The older commit: a.txt (300 new lines) and b.txt.
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 2);
        ui.select(1);
        pump(&mut ui, "a.txt's diff", &on("a.txt"));
        ui.focus = Focus::Diff;
        ui.scroll_diff(150);
        assert_eq!(ui.r.diff_top, 150);
        ui.select_file(1);
        pump(&mut ui, "b.txt's diff", &on("b.txt"));
        assert_eq!(ui.r.diff_top, 0, "another file starts at the top");
        ui.select_file(0);
        pump(&mut ui, "a.txt again", &on("a.txt"));
        assert_eq!(ui.r.diff_top, 150, "back where it was");
        // An option change keeps the scroll it has now.
        ui.scroll_diff(-50);
        ui.apply_diff_bar(diff_view::DiffBar::Whitespace);
        let before = ui.r.diff.as_ref().map(|d| d.lines.as_ptr() as usize);
        pump(&mut ui, "the refetched diff", &|u| u.r.diff.as_ref().map(|d| d.lines.as_ptr() as usize) != before);
        assert_eq!(ui.r.diff_top, 100);
        ui.apply_diff_bar(diff_view::DiffBar::Whitespace);
        // The newer commit's a.txt is another diff: it starts at the top,
        // and the older one is still remembered.
        ui.select(0);
        pump(&mut ui, "the newer a.txt", &on("a.txt"));
        assert_eq!(ui.r.diff_top, 0);
        ui.select(1);
        pump(&mut ui, "the older a.txt", &on("a.txt"));
        assert_eq!(ui.r.diff_top, 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The diff's view menu (FF-41): whitespace characters and the tab
    /// width, chosen from `…`, saved, and painted (unified and side by
    /// side) with partial repaints equal to full ones; find matches follow
    /// the tab stops.
    #[test]
    fn diff_whitespace_and_tab_width() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-diffws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "\tindented x\nab\tx  two spaces\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "tabs"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while ui.r.diff.is_none() {
            let left = until.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "timed out waiting for the diff");
            if let Ok((id, m)) = rx.recv_timeout(left) {
                ui.on_tab_msg(id, m);
            }
        }
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        assert!(!ui.diff_show_ws && ui.diff_tab == 4, "defaults");
        let pick = |ui: &mut Ui, label: &str| {
            let items = ui.diff_view_menu();
            let k = items.iter().position(|i| i.label.ends_with(label)).unwrap();
            ui.open_menu(900, 60, items);
            ui.menu_activate(k);
        };
        // Find "x": with tabs of 4 the second line's x is at byte 4.
        ui.key(Key::Tab);
        ui.key(Key::Tab);
        assert_eq!(ui.focus, Focus::Diff);
        ui.key(Key::Ctrl('f'));
        ui.key(Key::Char('x'));
        let first_x = |ui: &Ui, line: &str| ui.diff_find_marks(0, line).first().map(|m| m.0);
        assert_eq!(first_x(&ui, &diff_find::shown_text("ab\tx", ui.diff_tab)), Some(4));
        pick(&mut ui, "Show whitespace characters");
        assert!(ui.diff_show_ws);
        assert_eq!(ui.settings.get("diff.show_whitespace"), Some("true"));
        assert_partial_paint_is_full(&mut ui, &mut screen, "whitespace shown");
        pick(&mut ui, "Tab width 8");
        assert_eq!(ui.diff_tab, 8);
        assert_eq!(ui.settings.get("diff.tab_width"), Some("8"));
        assert_eq!(first_x(&ui, &diff_find::shown_text("ab\tx", ui.diff_tab)), Some(8));
        assert_eq!(ui.diff_find.as_ref().map(|f| f.hits.len()), Some(2), "matches recounted");
        assert_partial_paint_is_full(&mut ui, &mut screen, "tab width 8");
        ui.apply_diff_bar(diff_view::DiffBar::Split);
        assert_partial_paint_is_full(&mut ui, &mut screen, "side by side with whitespace");
        pick(&mut ui, "Show whitespace characters");
        assert!(!ui.diff_show_ws);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A diff line's menu (FF-41): right-click offers the editor at that
    /// line of the new file (a removed line: where it was) and copying
    /// it; headers and other panes get no such menu. Nothing is launched.
    #[test]
    fn diff_line_menu_opens_the_editor_at_the_line() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-reveal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "one\nthree\nfour\nfive\n").unwrap();
        g.output(&["commit", "-q", "-am", "drop two, add five"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while ui.r.diff.is_none() || ui.r.git.is_none() {
            let left = until.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "timed out waiting for the diff");
            if let Ok((id, m)) = rx.recv_timeout(left) {
                ui.on_tab_msg(id, m);
            }
        }
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let r = ui.diff_rect();
        let lines: Vec<(ffwd_core::diff::LineKind, String)> =
            ui.r.diff.as_ref().unwrap().lines.iter().map(|l| (l.kind, l.text.to_string())).collect();
        let y_of = |ui: &Ui, i: usize| r.y + ui.px(32.0) + i as i32 * ui.px(20.0) + ui.px(10.0);
        let x = r.x + r.w / 3;
        let labels = |ui: &mut Ui, i: usize| -> Vec<String> {
            let y = y_of(ui, i);
            ui.diff_line_menu(x, y).map(|v| v.into_iter().map(|m| m.label).collect()).unwrap_or_default()
        };
        let find = |kind: ffwd_core::diff::LineKind, text: &str| {
            lines.iter().position(|l| l.0 == kind && l.1 == text).unwrap()
        };
        use ffwd_core::diff::LineKind as K;
        assert!(labels(&mut ui, 0).is_empty(), "the hunk header has no line menu");
        assert_eq!(labels(&mut ui, find(K::Context, "three")), ["Open in editor at line 2", "Copy line"]);
        assert_eq!(
            labels(&mut ui, find(K::Del, "two")),
            ["Open in editor at line 2", "Copy line"],
            "where it was removed"
        );
        assert_eq!(labels(&mut ui, find(K::Add, "five"))[0], "Open in editor at line 4");
        // A right-click there opens that menu.
        ui.right_click(x, y_of(&ui, find(K::Add, "five")));
        assert!(ui.menu.as_ref().is_some_and(|m| m.items[0].label == "Open in editor at line 4"));
        ui.close_menu();
        // Not in the commit list.
        assert!(ui.diff_line_menu(ui.lay.list_body.x + 40, ui.lay.list_body.y + 5).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Local Changes diff has the line menu too (FF-41): right-click
    /// an unstaged line for the editor at that line of the working file.
    #[test]
    fn changes_diff_line_menu() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-chline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the repository", &|u| u.r.git.is_some());
        ui.enter_changes();
        pump(&mut ui, "the diff", &|u| u.r.changes.diff.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let i = ui.r.changes.diff.as_ref().unwrap().lines.iter().position(|l| &*l.text == "four").unwrap();
        let r = ui.changes_layout().diff;
        let y = r.y + ui.px(32.0) + i as i32 * ui.px(20.0) + ui.px(10.0);
        let x = r.x + r.w / 2;
        let labels: Vec<String> = ui.diff_line_menu(x, y).unwrap().into_iter().map(|m| m.label).collect();
        assert_eq!(labels, ["Open in editor at line 4", "Copy line"]);
        ui.right_click(x, y);
        assert!(ui.menu.as_ref().is_some_and(|m| m.items.len() == 2), "the menu opens");
        ui.close_menu();
        // The diff's bar is no line.
        assert!(ui.diff_line_menu(x, r.y + 4).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Code text size (FF-41): the view menu steps it larger and smaller,
    /// it is saved, line height and the hit test follow it, and repaints
    /// match full paints.
    #[test]
    fn code_text_size() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-codesize-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.join("a.txt"), lines.join("\n") + "\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while ui.r.diff.is_none() || ui.r.git.is_none() {
            let left = until.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "timed out waiting for the diff");
            if let Ok((id, m)) = rx.recv_timeout(left) {
                ui.on_tab_msg(id, m);
            }
        }
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        assert_eq!((ui.code_pt, ui.code_lh()), (12, 20));
        let pick = |ui: &mut Ui, prefix: &str| {
            let items = ui.diff_view_menu();
            let k = items.iter().position(|i| i.label.starts_with(prefix)).unwrap();
            ui.open_menu(900, 60, items);
            ui.menu_activate(k);
        };
        pick(&mut ui, "Larger text");
        pick(&mut ui, "Larger text");
        assert_eq!(ui.code_pt, 14);
        assert_eq!(ui.settings.get("diff.font_size"), Some("14"));
        assert_eq!(ui.code_lh(), 23, "lines grow with the text");
        assert_partial_paint_is_full(&mut ui, &mut screen, "larger code text");
        // The line menu finds the line under the pointer at the new height.
        let r = ui.diff_rect();
        let k = ui.r.diff.as_ref().unwrap().lines.iter().position(|l| &*l.text == "line 10").unwrap();
        let y = r.y + ui.px(32.0) + k as i32 * ui.code_lh() + ui.code_lh() / 2;
        assert_eq!(ui.diff_line_menu(r.x + r.w / 3, y).unwrap()[0].label, "Open in editor at line 10");
        pick(&mut ui, "Default text size");
        assert_eq!(ui.code_pt, 12);
        pick(&mut ui, "Smaller text");
        assert_eq!(ui.code_pt, 11);
        assert_partial_paint_is_full(&mut ui, &mut screen, "smaller code text");
        assert!(ui.diff_view_menu().iter().any(|i| i.label.starts_with("Larger text (12 pt)")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Clickable twins of shortcuts and menu icons (FF-57): the header's
    /// Find, the diff bar's next-change and find buttons, the status bar's
    /// Install update, and Discard then Undo from the file menu and the
    /// status bar; every commit menu item has an icon. Repaints match.
    #[test]
    fn clickable_twins_of_shortcuts() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-twins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let mut lines: Vec<String> = (1..=120).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.join("a.txt"), lines.join("\n") + "\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        lines[5] = "changed 6".into();
        lines[100] = "changed 101".into();
        std::fs::write(dir.join("a.txt"), lines.join("\n") + "\n").unwrap();
        g.output(&["commit", "-q", "-am", "two changes"]).unwrap();
        std::fs::write(dir.join("a.txt"), "local edit\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history and diff", &|u| u.r.log_done && u.r.git.is_some() && u.r.diff.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let center = |r: Rect| (r.x + r.w / 2, r.y + r.h / 2);
        // Every commit menu item has an icon.
        for it in ui.commit_menu(0).iter().filter(|i| i.act.is_some()) {
            assert!(icons::for_act(it.act.as_ref().unwrap()).is_some(), "no icon for {}", it.label);
        }
        // The header's Find opens the find bar.
        let (x, y) = center(ui.head_find_rect);
        ui.click(x, y);
        assert!(ui.r.search.is_some(), "Find opens the find bar");
        ui.key(Key::Esc);
        assert_partial_paint_is_full(&mut ui, &mut screen, "find closed");
        // The diff bar: next change, then find in the diff.
        let bar = |ui: &Ui, h: diff_view::DiffBar| ui.diff_bar_hits.iter().find(|b| b.1 == h).map(|b| b.0).unwrap();
        ui.r.diff_top = 0;
        let (x, y) = center(bar(&ui, diff_view::DiffBar::Next));
        ui.click(x, y);
        assert!(ui.r.diff_top > 0, "next change scrolls: {}", ui.r.diff_top);
        assert_partial_paint_is_full(&mut ui, &mut screen, "next change");
        let (x, y) = center(bar(&ui, diff_view::DiffBar::Find));
        ui.click(x, y);
        assert!(ui.diff_find.is_some());
        assert_partial_paint_is_full(&mut ui, &mut screen, "diff find from the bar");
        ui.close_diff_find();
        // An update offer: its button installs.
        ui.update_available = true;
        ui.set_notice_with("ffwd 9.9.9 is available".into(), NoticeAct::InstallUpdate);
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        assert!(!ui.notice_btn.is_empty());
        let (x, y) = center(ui.notice_btn);
        ui.click(x, y);
        assert!(ui.update_requested, "the button asks for the install");
        // Local Changes: Discard from the file menu, the status bar confirms,
        // and Undo brings the edit back.
        ui.enter_changes();
        pump(&mut ui, "the changes", &|u| u.r.changes.diff.is_some());
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let items = ui.file_menu_at_selected().unwrap();
        let k = items.iter().position(|i| i.label == "Discard changes…").unwrap();
        assert!(items.iter().any(|i| i.label == "Stage"));
        ui.open_menu(400, 200, items);
        ui.menu_activate(k);
        assert_eq!(ui.notice_act, Some(NoticeAct::Discard));
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let (x, y) = center(ui.notice_btn);
        ui.click(x, y);
        pump(&mut ui, "the discard", &|u| u.notice_act == Some(NoticeAct::Undo));
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), lines.join("\n") + "\n");
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let (x, y) = center(ui.notice_btn);
        ui.click(x, y);
        pump(&mut ui, "the undo", &|u| u.notice.as_deref() == Some("Restored the discarded changes"));
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "local edit\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Middle truncation (FF-57): a name that fits stays; a long one keeps
    /// its start and its distinct end and fits the width.
    #[test]
    fn names_shorten_in_the_middle() {
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let name = "worktree-agent-a0e6ecbab37660e9f2b";
        assert_eq!(fit_middle(&mut te, f.ui, 13.0, "main", 200.0), "main");
        let w = te.measure(f.ui, 13.0, name) * 0.6;
        let s = fit_middle(&mut te, f.ui, 13.0, name, w);
        assert!(s.contains('…') && s.starts_with("worktree") && s.ends_with("f2b"), "{s}");
        assert!(te.measure(f.ui, 13.0, &s) <= w);
        assert_eq!(fit_middle(&mut te, f.ui, 13.0, name, 1.0), "…");
    }

    /// The commit message in the details (FF-57): a long body shows up to
    /// five lines and moves the file list down; a short one keeps the
    /// default height; repaints match full paints.
    #[test]
    fn details_show_more_of_the_message() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-body-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "Short"]).unwrap();
        std::fs::write(dir.join("a.txt"), "b\n").unwrap();
        let body = (1..=8).map(|i| format!("Line {i} of the body.")).collect::<Vec<_>>().join("\n");
        g.output(&["commit", "-q", "-am", &format!("Long\n\n{body}")]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let shown =
            |subject: &'static str| move |u: &Ui| u.r.details.as_ref().is_some_and(|d| d.message.starts_with(subject));
        pump(&mut ui, "the long commit", &shown("Long"));
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let long_files = ui.files_rect().y;
        assert!(ui.details_extra > 0, "a long body takes more room");
        assert!(ui.details_extra <= (3.0 * ui.pf(19.0)).round() as i32 + 1, "at most five lines: {}", ui.details_extra);
        ui.key(Key::Down);
        pump(&mut ui, "the short commit", &shown("Short"));
        assert_eq!(ui.details_extra, 0);
        assert!(ui.files_rect().y < long_files, "the file list moves back up");
        assert_partial_paint_is_full(&mut ui, &mut screen, "short commit");
        ui.key(Key::Up);
        pump(&mut ui, "the long commit again", &shown("Long"));
        assert_partial_paint_is_full(&mut ui, &mut screen, "long commit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Word wrap in the diff (FF-41): with "Wrap long lines" a long line
    /// takes several rows, the line menu and clicks find the right line
    /// below it, and repaints match full paints (history and Local Changes).
    #[test]
    fn diff_wraps_long_lines() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-wrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let long = "word ".repeat(120);
        std::fs::write(dir.join("a.txt"), format!("short\n{long}\nafter\n")).unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "long line"]).unwrap();
        std::fs::write(dir.join("a.txt"), format!("short\n{long}\nafter\nlocal\n")).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the diff", &|u| u.r.diff.is_some() && u.r.git.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let items = ui.diff_view_menu();
        let k = items.iter().position(|i| i.label == "Wrap long lines").unwrap();
        ui.open_menu(900, 60, items);
        ui.menu_activate(k);
        assert!(ui.diff_wrap);
        assert_eq!(ui.settings.get("diff.wrap"), Some("true"));
        assert_partial_paint_is_full(&mut ui, &mut screen, "wrapped");
        let d = ui.r.diff.clone().unwrap();
        let li = d.lines.iter().position(|l| l.text.starts_with("word")).unwrap();
        let r = ui.diff_rect();
        let body = Rect::new(r.x + 1, r.y + ui.px(32.0), r.w - 1, r.h - ui.px(32.0));
        let rows = ui.diff_rows(&d, li, body);
        assert!(rows >= 2, "the long line wraps: {rows}");
        assert_eq!(ui.diff_rows(&d, li + 1, body), 1, "short lines don't");
        // The line menu finds "after", below the wrapped rows.
        let (ay, ah) = ui.diff_line_y(&d, ui.r.diff_top, body, li + 1).unwrap();
        let labels: Vec<String> =
            ui.diff_line_menu(r.x + r.w / 3, ay + ah / 2).unwrap().into_iter().map(|m| m.label).collect();
        assert_eq!(labels[0], "Open in editor at line 3");
        // Local Changes: wrapped too, and a click picks the line under it.
        ui.enter_changes();
        pump(&mut ui, "the changes", &|u| u.r.changes.diff.is_some());
        assert_partial_paint_is_full(&mut ui, &mut screen, "wrapped changes");
        let cd = ui.r.changes.diff.clone().unwrap();
        let ci = cd.lines.iter().position(|l| &*l.text == "local").unwrap();
        let cr = ui.changes_layout().diff;
        let cbody = Rect::new(cr.x, cr.y + ui.px(32.0), cr.w, cr.h - ui.px(32.0));
        let (y, h) = ui.diff_line_y(&cd, 0, cbody, ci).unwrap();
        ui.click(cr.x + cr.w / 2, y + h / 2);
        assert!(ui.r.changes.picked.contains(&ci), "the click picks the line below the wrapped one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Branch names (FF-36): a space types "-", an invalid name says why
    /// while typing and Enter refuses it; a valid one creates the branch.
    #[test]
    fn branch_names_are_checked_while_typing() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-brname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["commit", "-q", "--allow-empty", "-m", "first"],
        ] {
            g.output(a).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.git.is_some() && !u.r.commits.is_empty());
        ui.key(Key::Ctrl('b'));
        let field = |ui: &Ui| match &ui.dialog.as_ref().unwrap().kind {
            menu::DialogKind::Name { field, .. } => field.text().to_string(),
            _ => panic!("a name dialog"),
        };
        let error = |ui: &Ui| ui.dialog.as_ref().and_then(|d| d.error.clone());
        for c in "my feature".chars() {
            ui.key(Key::Char(c));
        }
        assert_eq!(field(&ui), "my-feature", "a space types -");
        assert_eq!(error(&ui), None);
        for c in "..x".chars() {
            ui.key(Key::Char(c));
        }
        assert_eq!(error(&ui).as_deref(), Some("A name can't contain \"..\"."));
        ui.key(Key::Enter);
        assert!(ui.dialog.as_ref().is_some_and(|d| !d.busy), "Enter refuses an invalid name");
        for _ in 0..3 {
            ui.key(Key::Backspace);
        }
        assert_eq!(error(&ui), None);
        ui.key(Key::Enter);
        pump(&mut ui, "the new branch", &|u| u.r.refs.refs.iter().any(|r| &*r.name == "my-feature"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Compare two commits (FF-33): mark one from its menu, compare another
    /// with it; the details list the files between them and diffs follow;
    /// Swap reverses it, Close (or selecting a commit) ends it.
    #[test]
    fn compare_two_commits_from_the_menu() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        for (file, text) in [("a.txt", "one\n"), ("b.txt", "two\n"), ("a.txt", "one\nmore\n")] {
            std::fs::write(dir.join(file), text).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", file]).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 3 && u.r.details.is_some());
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let pick = |ui: &mut Ui, row: usize, prefix: &str| {
            let items = ui.commit_menu(row);
            let k = items.iter().position(|i| i.label.starts_with(prefix)).unwrap_or_else(|| panic!("no {prefix}"));
            ui.open_menu(300, 200, items);
            ui.menu_activate(k);
        };
        let (first, last) = (ui.r.commits[2].id, ui.r.commits[0].id);
        pick(&mut ui, 2, "Mark for comparison");
        assert_eq!(ui.r.compare_mark, Some(first));
        pick(&mut ui, 0, "Compare with");
        pump(&mut ui, "the comparison", &|u| {
            u.r.details.as_ref().is_some_and(|d| d.parents == [first]) && u.r.diff.is_some()
        });
        assert_eq!(ui.r.compare, Some((first, last)));
        let mut names: Vec<String> = ui.r.details.as_ref().unwrap().files.iter().map(|f| f.path.to_string()).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "b.txt"]);
        assert_partial_paint_is_full(&mut ui, &mut screen, "comparison");
        // Swap: from the last to the first.
        let b = ui.compare_btns[0];
        ui.click(b.x + 4, b.y + b.h / 2);
        pump(&mut ui, "the swapped comparison", &|u| u.r.details.as_ref().is_some_and(|d| d.parents == [last]));
        assert_eq!(ui.r.compare, Some((last, first)));
        assert_partial_paint_is_full(&mut ui, &mut screen, "swapped");
        // Close: the selected commit's own details again.
        let c = ui.compare_btns[1];
        ui.click(c.x + 4, c.y + c.h / 2);
        assert!(ui.r.compare.is_none());
        pump(&mut ui, "the commit's own details", &|u| {
            u.r.details.as_ref().is_some_and(|d| d.id == u.r.commits[u.r.sel].id && d.parents != [last])
        });
        assert_partial_paint_is_full(&mut ui, &mut screen, "closed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Commit and push (FF-35): the Push checkbox under the message makes
    /// a commit push afterwards; the commit lands on the remote (a local
    /// bare repository, no network) and the choice is remembered.
    #[test]
    fn commit_and_push_from_the_checkbox() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-cpush-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, remote) = (base.join("work"), base.join("remote.git"));
        std::fs::create_dir_all(&dir).unwrap();
        Git { root: base.clone() }.output(&["init", "-q", "--bare", "-b", "main", "remote.git"]).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let url = remote.to_string_lossy().replace('\\', "/");
        g.output(&["remote", "add", "origin", &url]).unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        g.output(&["push", "-q", "-u", "origin", "main"]).unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the repository", &|u| u.r.git.is_some());
        ui.enter_changes();
        pump(&mut ui, "the change", &|u| u.side_len(changes_view_side(true)) == 1);
        ui.key(Key::Char(' '));
        pump(&mut ui, "staged", &|u| u.r.changes.busy.is_none() && u.side_len(changes_view_side(false)) == 1);
        // Tick "Push" by clicking it.
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let (r, _) = *ui.r.changes.hits.iter().find(|h| h.1 == changes_view::CHit::PushAfter).unwrap();
        ui.click(r.x + 4, r.y + r.h / 2);
        assert!(ui.push_after_commit());
        assert_eq!(ui.settings.get("commit.push"), Some("true"), "remembered");
        assert_partial_paint_is_full(&mut ui, &mut screen, "push ticked");
        ui.r.changes.focus = changes_view::CFocus::Subject;
        for c in "Add b".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::CtrlEnter);
        let remote_git = Git { root: remote.clone() };
        let pushed = || {
            remote_git
                .output(&["log", "-1", "--format=%s", "main"])
                .map(|o| String::from_utf8_lossy(&o).trim().to_string())
        };
        pump(&mut ui, "the commit and the push", &|u| u.r.op.is_none() && pushed().is_ok_and(|s| s == "Add b"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Horizontal scroll in the diff (FF-41): sideways over a diff moves
    /// the code, not the line numbers, up to the widest line; repaints
    /// match full paints (unified and side by side); wrapping turns it
    /// off; a new file starts at the left edge.
    #[test]
    fn diff_scrolls_sideways() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-hscroll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), format!("short\n{}\n", "x".repeat(400))).unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "long"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let on = |p: &'static str| move |u: &Ui| u.r.diff.as_ref().is_some_and(|d| &*d.path == p);
        pump(&mut ui, "a.txt", &on("a.txt"));
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let r = ui.diff_rect();
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        ui.wheel_h(x, y, 300.0);
        assert!(ui.code_left > 0.0, "scrolled sideways");
        assert_partial_paint_is_full(&mut ui, &mut screen, "unified, scrolled");
        ui.wheel_h(x, y, 1.0e6);
        let max = ui.code_left;
        assert!(max < 1.0e5, "stops at the widest line: {max}");
        ui.wheel_h(x, y, -1.0e6);
        assert_eq!(ui.code_left, 0.0, "back to the left edge");
        ui.apply_diff_bar(diff_view::DiffBar::Split);
        ui.wheel_h(x, y, 200.0);
        assert!(ui.code_left > 0.0);
        assert_partial_paint_is_full(&mut ui, &mut screen, "split, scrolled");
        ui.apply_diff_bar(diff_view::DiffBar::Unified);
        // The next file starts at the left edge.
        ui.select_file(1);
        pump(&mut ui, "b.txt", &on("b.txt"));
        assert_eq!(ui.code_left, 0.0);
        // Wrapping on: nothing to scroll.
        ui.set_diff_wrap(true);
        ui.wheel_h(x, y, 300.0);
        assert_eq!(ui.code_left, 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Hide untracked files (FF-35): the Unstaged list's "…" toggles them,
    /// the choice is remembered, and the header says how many are hidden.
    #[test]
    fn untracked_files_can_be_hidden() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-untracked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
        std::fs::write(dir.join("new.txt"), "new\n").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the repository", &|u| u.r.git.is_some());
        ui.enter_changes();
        let unstaged = |u: &Ui| u.side_len(changes_view_side(true));
        pump(&mut ui, "both files", &|u| unstaged(u) == 2);
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let toggle = |ui: &mut Ui| {
            let (b, _) = *ui.r.changes.hits.iter().find(|h| h.1 == changes_view::CHit::ListOptions).unwrap();
            ui.click(b.x + 4, b.y + b.h / 2);
            assert!(ui.menu.as_ref().is_some_and(|m| m.items[0].label.ends_with("Show untracked files")));
            ui.menu_activate(0);
        };
        toggle(&mut ui);
        assert_eq!(ui.settings.get("changes.hide_untracked"), Some("true"), "remembered");
        pump(&mut ui, "untracked hidden", &|u| unstaged(u) == 1 && u.r.changes.hidden_untracked == 1);
        assert_partial_paint_is_full(&mut ui, &mut screen, "hidden");
        let mut screen2 = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen2, 1280, 800), true);
        toggle(&mut ui);
        pump(&mut ui, "untracked shown", &|u| unstaged(u) == 2 && u.r.changes.hidden_untracked == 0);
        assert_partial_paint_is_full(&mut ui, &mut screen2, "shown");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A gone upstream (FF-36): a branch whose upstream was deleted on the
    /// remote shows "gone" in the sidebar, and its menu stops the tracking.
    #[test]
    fn gone_upstream_is_shown_and_can_be_dropped() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("work");
        std::fs::create_dir_all(&dir).unwrap();
        Git { root: base.clone() }.output(&["init", "-q", "--bare", "-b", "main", "remote.git"]).unwrap();
        let g = Git { root: dir.clone() };
        let url = base.join("remote.git").to_string_lossy().replace('\\', "/");
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["remote", "add", "origin", &url],
            &["commit", "-q", "--allow-empty", "-m", "first"],
            &["push", "-q", "-u", "origin", "main"],
            &["branch", "feature"],
            &["push", "-q", "-u", "origin", "feature"],
            &["push", "-q", "origin", "--delete", "feature"],
            &["fetch", "-q", "--prune"],
        ] {
            g.output(a).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let feature = |u: &Ui| u.r.refs.refs.iter().find(|r| &*r.name == "feature").cloned();
        pump(&mut ui, "the refs", &|u| feature(u).is_some());
        assert!(feature(&ui).unwrap().upstream_gone, "git says the upstream is gone");
        let badge = ui.r.side.iter().find(|i| i.key == "b:feature").map(|i| i.badge.clone());
        assert_eq!(badge.as_deref(), Some("gone"));
        let items = ui.ref_menu("b:feature");
        let k = items.iter().position(|i| i.label == "Stop tracking origin/feature (gone)").unwrap();
        ui.open_menu(100, 200, items);
        ui.menu_activate(k);
        pump(&mut ui, "the upstream dropped", &|u| feature(u).is_some_and(|r| r.upstream.is_none()));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Set and change a branch's upstream (FF-36): the branch menu's
    /// "Set upstream…" refuses a name that isn't a remote branch, then
    /// tracks the one typed; the menu then offers "Change upstream…".
    #[test]
    fn set_a_branch_upstream_from_the_menu() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let base = std::env::temp_dir().join(format!("ffwd-ui-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("work");
        std::fs::create_dir_all(&dir).unwrap();
        Git { root: base.clone() }.output(&["init", "-q", "--bare", "-b", "main", "remote.git"]).unwrap();
        let g = Git { root: dir.clone() };
        let url = base.join("remote.git").to_string_lossy().replace('\\', "/");
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["remote", "add", "origin", &url],
            &["commit", "-q", "--allow-empty", "-m", "first"],
            &["push", "-q", "-u", "origin", "main"],
            &["push", "-q", "origin", "main:other"],
            &["fetch", "-q"],
            &["branch", "feature"],
        ] {
            g.output(a).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let feature = |u: &Ui| u.r.refs.refs.iter().find(|r| &*r.name == "feature").cloned();
        pump(&mut ui, "the refs", &|u| {
            feature(u).is_some() && u.r.refs.refs.iter().any(|r| &*r.name == "origin/other")
        });
        let items = ui.ref_menu("b:feature");
        let k = items.iter().position(|i| i.label == "Set upstream…").unwrap();
        ui.open_menu(100, 200, items);
        ui.menu_activate(k);
        let field = |ui: &Ui| match &ui.dialog.as_ref().unwrap().kind {
            menu::DialogKind::Name { field, .. } => field.text().to_string(),
            _ => panic!("a name dialog"),
        };
        assert_eq!(field(&ui), "origin/feature");
        ui.key(Key::Enter);
        assert!(
            ui.dialog.as_ref().and_then(|d| d.error.clone()).is_some_and(|e| e.contains("not a remote branch")),
            "an unknown remote branch is refused"
        );
        ui.key(Key::Ctrl('a'));
        for c in "origin/other".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        pump(&mut ui, "the new upstream", &|u| {
            feature(u).is_some_and(|r| r.upstream.as_deref() == Some("origin/other"))
        });
        assert!(ui.ref_menu("b:feature").iter().any(|i| i.label == "Change upstream…"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Show ignored files (FF-35): the Unstaged list's "…" lists them (a
    /// wholly ignored folder as one row, letter I), Space on one only
    /// explains, and turning it off removes them; remembered.
    #[test]
    fn ignored_files_can_be_shown() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-ignored-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
        std::fs::write(dir.join("target/debug/x.o"), "x").unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the repository", &|u| u.r.git.is_some());
        ui.enter_changes();
        let unstaged = |u: &Ui| u.side_len(changes_view_side(true));
        pump(&mut ui, "the change", &|u| unstaged(u) == 1);
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let pick = |ui: &mut Ui, label: &str| {
            let (b, _) = *ui.r.changes.hits.iter().find(|h| h.1 == changes_view::CHit::ListOptions).unwrap();
            ui.click(b.x + 4, b.y + b.h / 2);
            let k = ui.menu.as_ref().unwrap().items.iter().position(|i| i.label.ends_with(label)).unwrap();
            ui.menu_activate(k);
        };
        pick(&mut ui, "Show ignored files");
        assert_eq!(ui.settings.get("changes.show_ignored"), Some("true"));
        pump(&mut ui, "the ignored folder", &|u| unstaged(u) == 2);
        assert_partial_paint_is_full(&mut ui, &mut screen, "ignored shown");
        let rows = ui.side_rows(changes_view_side(true), 0, 2);
        let k = rows.iter().position(|e| &*e.path == "target/").expect("a wholly ignored folder is one row");
        assert_eq!(rows[k].letter(changes_view_side(true)), b'I');
        ui.select_change(changes_view_side(true), k);
        ui.key(Key::Char(' '));
        assert!(ui.notice.as_deref().is_some_and(|n| n.contains("is ignored")), "{:?}", ui.notice);
        let mut screen2 = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen2, 1280, 800), true);
        pick(&mut ui, "Show ignored files");
        pump(&mut ui, "ignored hidden again", &|u| unstaged(u) == 1);
        assert_partial_paint_is_full(&mut ui, &mut screen2, "ignored hidden");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The large view (FF-33): Space gives the selected commit's details
    /// and diff the whole main area, the diff bar's button and Esc bring
    /// the history back; repaints match full paints.
    #[test]
    fn space_opens_the_large_view() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-max-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while ui.r.diff.is_none() {
            let left = until.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "timed out waiting for the diff");
            if let Ok((id, m)) = rx.recv_timeout(left) {
                ui.on_tab_msg(id, m);
            }
        }
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        let normal = ui.lay.details;
        ui.key(Key::Char(' '));
        assert!(ui.diff_max);
        assert_eq!(ui.lay.list_body.h, 0, "no history list");
        assert!(ui.lay.details.h > normal.h || ui.lay.details.w > normal.w, "the details take the room");
        assert_partial_paint_is_full(&mut ui, &mut screen, "large view");
        // The diff bar's button brings the history back.
        let (b, _) = *ui.diff_bar_hits.iter().find(|h| h.1 == diff_view::DiffBar::Max).unwrap();
        ui.click(b.x + b.w / 2, b.y + b.h / 2);
        assert!(!ui.diff_max);
        assert_eq!(ui.lay.details, normal);
        assert_partial_paint_is_full(&mut ui, &mut screen, "back");
        // And Esc.
        ui.key(Key::Char(' '));
        ui.key(Key::Esc);
        assert!(!ui.diff_max);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Clicks a settings row's control; a dropdown then picks choice `i`
    /// from its menu, a segmented row clicks segment `i`.
    fn settings_pick(ui: &mut Ui, screen: &mut [u32], row: settings_view::Row, i: usize) {
        let (w, h) = (ui.w, ui.h);
        ui.paint(&mut Canvas::new(screen, w, h), true);
        let seg = ui.settings_target(row, i).filter(|_| i > 0);
        let r = seg.or_else(|| ui.settings_target(row, 0)).unwrap_or_else(|| panic!("{row:?} is on screen"));
        ui.click(r.x + r.w / 2, r.y + r.h / 2);
        ui.last_click = None;
        if ui.menu.is_some() {
            ui.menu_activate(i);
        }
    }

    /// The WSL distribution (FF-23): a dropdown of the installed ones
    /// (without WSL's own helper distributions); picking one sets
    /// agents.wsl_distro, the default clears it.
    #[test]
    fn wsl_distribution_is_a_choice() {
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let empty = crate::settings::Settings::parse("");
        let sinks: SinkFactory = Arc::new(|_| Arc::new(|_| {}));
        let mut ui = Ui::new(te, f, empty.clone(), empty, None, sinks, 1280, 800, 1.0);
        ui.wsl_distros = vec!["Ubuntu-24.04".into(), "docker-desktop".into(), "Debian".into()];
        ui.open_settings_page(1);
        ui.settings_go("Sandbox");
        assert_eq!(ui.row_choices(settings_view::Row::WslDistro), ["Default", "Ubuntu-24.04", "Debian"]);
        let mut screen = vec![0u32; 1280 * 800];
        settings_pick(&mut ui, &mut screen, settings_view::Row::WslDistro, 1);
        assert_eq!(ui.settings.get("agents.wsl_distro"), Some("Ubuntu-24.04"));
        assert!(ui.menu.is_none(), "the dropdown closes");
        settings_pick(&mut ui, &mut screen, settings_view::Row::WslDistro, 0);
        assert_eq!(ui.settings.get("agents.wsl_distro").unwrap_or(""), "", "the default clears it");
    }

    /// Per-path access (FF-23): the Access section lists the preset's own
    /// paths (Off / Read / Write) and the user's (Remove / Read / Write);
    /// a typed path is added read-only.
    #[test]
    fn agent_access_is_per_path() {
        use settings_view::Row;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let s = crate::settings::Settings::parse("[agents]\npreset = \"claude\"\n");
        let empty = crate::settings::Settings::parse("");
        let sinks: SinkFactory = Arc::new(|_| Arc::new(|_| {}));
        let mut ui = Ui::new(te, f, s, empty, None, sinks, 1280, 900, 1.0);
        ui.open_settings_page(1);
        ui.settings_go("Access");
        let mut screen = vec![0u32; 1280 * 900];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 900), true);
        let g = ui.grants();
        assert_eq!(
            g.iter().map(|g| (g.path.as_str(), g.level)).collect::<Vec<_>>(),
            [("~/.claude", 2), ("~/.claude.json", 2)]
        );
        assert!(ui.rows().contains(&Row::Grant(0)), "a row per path");
        assert_eq!(ui.row_choices(Row::Grant(0)), ["Off", "Read", "Write"]);
        // The preset's path to Read, by its segment.
        settings_pick(&mut ui, &mut screen, Row::Grant(0), 1);
        assert_eq!(ui.settings.get("agents.preset_access"), Some("~/.claude=read"));
        assert_partial_paint_is_full(&mut ui, &mut screen, "preset path read");
        // Add a file: Enter on the add row, type, Enter.
        let add = ui.rows().iter().position(|r| *r == Row::AddGrant).unwrap();
        ui.settings_view.as_mut().unwrap().sel = add;
        ui.key(Key::Enter);
        for c in "~/.gitconfig".chars() {
            ui.key(Key::Char(c));
        }
        ui.key(Key::Enter);
        assert_eq!(ui.settings.get("agents.read_paths"), Some("~/.gitconfig"));
        assert_partial_paint_is_full(&mut ui, &mut screen, "path added");
        let g = ui.grants();
        assert_eq!((g[2].path.as_str(), g[2].level, g[2].tool), ("~/.gitconfig", 1, None));
        assert_eq!(ui.row_choices(Row::Grant(2)), ["Remove", "Read", "Write"]);
        assert!(ui.rows().contains(&Row::Grant(2)));
        // Writable, then removed.
        ui.set_choice(Row::Grant(2), 2);
        assert_eq!(ui.settings.get("agents.read_paths").unwrap_or(""), "");
        assert_eq!(ui.settings.get("agents.write_paths"), Some("~/.gitconfig"));
        ui.set_choice(Row::Grant(2), 0);
        assert_eq!(ui.settings.get("agents.write_paths").unwrap_or(""), "");
        assert_eq!(ui.grants().len(), 2);
        assert_partial_paint_is_full(&mut ui, &mut screen, "path removed");
        // Back to Write drops the preset entry again.
        ui.set_choice(Row::Grant(0), 2);
        assert_eq!(ui.settings.get("agents.preset_access").unwrap_or(""), "");
    }

    /// Merge and diff tools (FF-42): dropdowns of the tools Git can start;
    /// picking one sets tools.merge_tool, "Git config" clears it.
    #[test]
    fn merge_and_diff_tools_are_choices() {
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let empty = crate::settings::Settings::parse("");
        let sinks: SinkFactory = Arc::new(|_| Arc::new(|_| {}));
        let mut ui = Ui::new(te, f, empty.clone(), empty, None, sinks, 1280, 900, 1.0);
        ui.open_settings_page(0);
        ui.settings_go("Git & tools");
        ui.on_tab_msg(0, Msg::Tools(vec!["tool-a".into(), "tool-b".into()], vec!["tool-c".into()]));
        assert_eq!(ui.row_choices(settings_view::Row::MergeTool), ["Git config", "tool-a", "tool-b"]);
        assert_eq!(ui.row_choices(settings_view::Row::DiffTool), ["Git config", "tool-c"]);
        let mut screen = vec![0u32; 1280 * 900];
        settings_pick(&mut ui, &mut screen, settings_view::Row::MergeTool, 2);
        assert_eq!(ui.settings.get("tools.merge_tool"), Some("tool-b"));
        assert_partial_paint_is_full(&mut ui, &mut screen, "merge tool chosen");
        ui.set_choice(settings_view::Row::MergeTool, 0);
        assert_eq!(ui.settings.get("tools.merge_tool").unwrap_or(""), "");
    }

    /// The settings page (FF-57): sections in a sidebar, switches, search,
    /// and the scope picker with overrides that say so and reset.
    #[test]
    fn settings_page_sections_search_and_scope() {
        use settings_view::Row;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let empty = crate::settings::Settings::parse("");
        let sinks: SinkFactory = Arc::new(|_| Arc::new(|_| {}));
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 860, 1.0);
        let mut screen = vec![0u32; 1280 * 860];
        ui.open_settings();
        ui.paint(&mut Canvas::new(&mut screen, 1280, 860), true);
        assert_eq!(ui.settings_section_now(), Some("Appearance"));
        // Ctrl+Tab goes to the next section.
        ui.key(Key::NextTab);
        assert_eq!(ui.settings_section_now(), Some("Updates"));
        ui.settings_go("Agent");
        // A switch: Agents on.
        assert_eq!(ui.settings.get("agents.enabled"), None);
        settings_pick(&mut ui, &mut screen, Row::AgentsOn, 0);
        assert_eq!(ui.settings.get("agents.enabled"), Some("true"));
        assert_partial_paint_is_full(&mut ui, &mut screen, "switch on");
        // The agent as a dropdown: Codex.
        settings_pick(&mut ui, &mut screen, Row::Preset, 2);
        assert_eq!(ui.settings.get("agents.preset"), Some("codex"));
        // Typing searches every section.
        for c in "network".chars() {
            ui.key(Key::Char(c));
        }
        assert_eq!(ui.rows(), [Row::Network]);
        assert_partial_paint_is_full(&mut ui, &mut screen, "search");
        ui.key(Key::Esc);
        assert!(ui.rows().len() > 1, "Esc clears the search");
        // The scope picker: this repository only, then an override that
        // says so and resets.
        ui.key(Key::Esc);
        ui.settings_go("Sandbox");
        ui.set_settings_scope(true);
        settings_pick(&mut ui, &mut screen, Row::Network, 0);
        let repo = ui.current_repo_settings().unwrap().get("agents.network").map(str::to_string);
        assert_eq!(repo.as_deref(), Some("false"), "set for this repository only");
        assert_eq!(ui.settings.get("agents.network"), None, "not for all repositories");
        ui.paint(&mut Canvas::new(&mut screen, 1280, 860), true);
        let reset = ui
            .settings_view
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| matches!(h.1, settings_view::SHit::Reset(_)))
            .map(|h| h.0);
        let r = reset.expect("an overridden row offers Reset");
        ui.click(r.x + r.w / 2, r.y + r.h / 2);
        assert_eq!(ui.current_repo_settings().unwrap().get("agents.network"), None, "reset");
        assert_partial_paint_is_full(&mut ui, &mut screen, "reset");
        ui.key(Key::Esc);
        assert!(ui.settings_view.is_none(), "Esc closes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Terminal sessions (FF-23) and the worktree menu (FF-45): the agents
    /// panel offers the tool's CLI in a new worktree or here, sandboxed or
    /// not; a worktree's menu opens it, a terminal or the CLI there, and
    /// removes it after asking (never the main one).
    #[test]
    fn terminal_sessions_and_worktree_menu() {
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let s =
            crate::settings::Settings::parse("[agents]\nenabled = \"true\"\npreset = \"claude\"\nsandbox = \"wsl\"\n");
        let empty = crate::settings::Settings::parse("");
        let sinks: SinkFactory = Arc::new(|_| Arc::new(|_| {}));
        let mut ui = Ui::new(te, f, s, empty, None, sinks, 1280, 800, 1.0);
        let labels = |v: Vec<menu::MenuItem>| v.into_iter().map(|i| i.label).collect::<Vec<_>>();
        let all = labels(ui.agent_terminal_menu(None, true));
        assert_eq!(
            all[..5],
            [
                "Open Claude Code in a new worktree (WSL + bubblewrap)",
                "Open Claude Code in this repository (WSL + bubblewrap)",
                "Open Claude Code in a new worktree (WSL, no sandbox)",
                "Open Claude Code in this repository (WSL, no sandbox)",
                "",
            ]
        );
        // Every other agent, sandboxed, with its own command.
        assert!(all.iter().any(|l| l == "Open Codex in a new worktree (WSL + bubblewrap)"), "{all:?}");
        assert!(all.iter().any(|l| l == "Open opencode in this repository (WSL + bubblewrap)"), "{all:?}");
        assert_eq!(all.len(), 5 + 2 * (ffwd_agent::presets::PRESETS.len() - 1));
        let (codex, _) = ui.agent_config_for(Some("codex"));
        assert_eq!(codex.interactive_command, "codex");
        ui.r.refs.worktrees = vec!["E:/r/main".into(), "E:/r/.ffwd-agents/r/fix".into()];
        let main = labels(ui.ref_menu("w:E:/r/main"));
        assert!(main.iter().any(|l| l == "Open in a new tab") && main.iter().any(|l| l == "Open terminal here"));
        assert!(main.iter().any(|l| l == "Open Claude Code in this worktree (WSL + bubblewrap)"), "{main:?}");
        assert!(!main.iter().any(|l| l.starts_with("Remove")), "never the main worktree");
        let items = ui.ref_menu("w:E:/r/.ffwd-agents/r/fix");
        let rm = items.iter().find(|i| i.label == "Remove worktree…").expect("a linked worktree can go");
        assert!(rm.danger && matches!(rm.act, Some(menu::MenuAct::Ask(_))), "it asks first");
    }

    /// Find in the diff (FF-34): `Ctrl+F` with the diff focused searches
    /// the shown diff (case-insensitive), Enter steps through the matches
    /// and scrolls to them, the query stays for the next file, and Esc
    /// closes it; with the list focused `Ctrl+F` still searches commits.
    #[test]
    fn find_in_the_diff() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-difffind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        // a.txt: two needles near the top, one far below; b.txt: one.
        let mut lines: Vec<String> = (1..=200).map(|i| format!("line {i}")).collect();
        lines[2] = "a Needle here".into();
        lines[3] = "needle and NEEDLE".into();
        lines[180] = "the last needle".into();
        std::fs::write(dir.join("a.txt"), lines.join("\n") + "\n").unwrap();
        std::fs::write(dir.join("b.txt"), "one needle\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "Needles"]).unwrap();
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let on = |p: &'static str| move |u: &Ui| u.r.diff.as_ref().is_some_and(|d| &*d.path == p);
        pump(&mut ui, "a.txt's diff", &on("a.txt"));
        let mut screen = vec![0u32; 1280 * 800];
        ui.paint(&mut Canvas::new(&mut screen, 1280, 800), true);
        // With the list focused, Ctrl+F searches commits.
        ui.key(Key::Ctrl('f'));
        assert!(ui.r.search.is_some() && ui.diff_find.is_none());
        ui.key(Key::Esc);
        ui.focus = Focus::Diff;
        ui.key(Key::Ctrl('f'));
        assert!(ui.r.search.is_none() && ui.diff_find.is_some(), "the diff's own find");
        for c in "NEEDLE".chars() {
            ui.key(Key::Char(c));
        }
        let hits = |u: &Ui| u.diff_find.as_ref().map(|f| (f.hits.len(), f.cur));
        assert_eq!(hits(&ui), Some((4, 0)), "case-insensitive, every occurrence");
        assert_partial_paint_is_full(&mut ui, &mut screen, "diff find");
        // Enter steps on; the last match (line 181) scrolls into view.
        for _ in 0..3 {
            ui.key(Key::Enter);
        }
        assert_eq!(hits(&ui), Some((4, 3)));
        assert!(ui.r.diff_top > 100, "scrolled to the last match: {}", ui.r.diff_top);
        assert_partial_paint_is_full(&mut ui, &mut screen, "jump to a match");
        ui.key(Key::Enter);
        assert_eq!(hits(&ui), Some((4, 0)), "wraps around");
        // The next file: the query stays and that diff is searched.
        ui.select_file(1);
        pump(&mut ui, "b.txt's diff", &on("b.txt"));
        assert_eq!(ui.diff_find.as_ref().map(|f| f.input.text()), Some("NEEDLE"));
        assert_eq!(hits(&ui), Some((1, 0)));
        assert_partial_paint_is_full(&mut ui, &mut screen, "next file");
        ui.key(Key::Esc);
        assert!(ui.diff_find.is_none());
        assert_partial_paint_is_full(&mut ui, &mut screen, "find closed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The find bar follows the history's scope (FF-34): a commit only on
    /// another branch is found over all branches; switching to the current
    /// branch keeps the search and runs it again, and it finds nothing.
    #[test]
    fn find_follows_the_history_scope() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-findscope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["commit", "-q", "--allow-empty", "-m", "Start"],
            &["checkout", "-q", "-b", "side"],
            &["commit", "-q", "--allow-empty", "-m", "Only on side"],
            &["checkout", "-q", "main"],
        ] {
            g.output(a).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 2);
        ui.key(Key::Ctrl('f'));
        for c in "only on side".chars() {
            ui.key(Key::Char(c));
        }
        let done = |u: &Ui| u.r.search.as_ref().is_some_and(|s| s.done);
        pump(&mut ui, "the search over all branches", &done);
        assert_eq!(ui.r.search.as_ref().unwrap().matches.len(), 1);
        ui.toggle_scope();
        assert_eq!(ui.r.search.as_ref().map(|s| s.input.text()), Some("only on side"), "the search stays open");
        pump(&mut ui, "the search over the current branch", &|u| done(u) && u.r.log_done);
        assert!(ui.r.search.as_ref().unwrap().matches.is_empty(), "main's history has no such commit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The find bar from the keyboard: message search selects the match,
    /// Tab changes the field, content waits for Enter, Esc closes.
    #[test]
    fn find_bar_with_keys() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        for (file, text, msg) in [
            ("a.txt", "a\n", "Add the README"),
            ("p.rs", "fn draw_panel() {}\n", "Draw the panel"),
            ("a.txt", "b\n", "Fix a typo"),
        ] {
            std::fs::write(dir.join(file), text).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", msg]).unwrap();
        }
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}");
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        let done = |u: &Ui| u.r.search.as_ref().is_some_and(|s| s.done);
        pump(&mut ui, "history", &|u| u.r.log_done && u.r.commits.len() == 3);
        ui.key(Key::Ctrl('f'));
        for c in "panel".chars() {
            ui.key(Key::Char(c));
        }
        pump(&mut ui, "the message search", &done);
        assert_eq!(ui.r.search.as_ref().unwrap().rows.len(), 1);
        assert_eq!(&*ui.r.commits[ui.r.sel].subject, "Draw the panel", "the first match is selected");
        assert!(ui.row_dimmed(0) && !ui.row_dimmed(ui.r.sel));
        // Message → Author → SHA: "panel" is no SHA prefix.
        ui.key(Key::Tab);
        ui.key(Key::Tab);
        pump(&mut ui, "the SHA search", &done);
        assert!(ui.r.search.as_ref().unwrap().rows.is_empty());
        // → Content: waits for Enter.
        ui.key(Key::Tab);
        ui.key(Key::Ctrl('a'));
        for c in "draw_panel".chars() {
            ui.key(Key::Char(c));
        }
        assert!(ui.r.search.as_ref().unwrap().rows.is_empty() && !ui.search_active(), "content waits for Enter");
        ui.key(Key::Enter);
        pump(&mut ui, "the content search", &|u| done(u) && u.search_active());
        assert_eq!(ui.r.search.as_ref().unwrap().rows.len(), 1);
        ui.key(Key::Esc);
        assert!(ui.r.search.is_none() && !ui.row_dimmed(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A merge conflict from the keyboard: the banner shows the merge,
    /// Space refuses to stage a conflicted file, `o` picks ours, Enter
    /// saves it as resolved, and Continue makes the merge commit.
    #[test]
    fn resolve_a_conflict_with_keys() {
        use std::sync::mpsc;
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let dir = std::env::temp_dir().join(format!("ffwd-ui-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "t"],
            &["config", "user.email", "t@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let commit = |t: &str, m: &str| {
            std::fs::write(dir.join("a.txt"), t).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        };
        commit("base\n", "base");
        g.output(&["switch", "-q", "-c", "feature"]).unwrap();
        commit("feature\n", "feature");
        g.output(&["switch", "-q", "main"]).unwrap();
        commit("main\n", "main");
        assert!(g.output(&["merge", "feature"]).is_err());
        let (tx, rx) = mpsc::channel::<(u64, Msg)>();
        let tx = Arc::new(std::sync::Mutex::new(tx));
        let sinks: SinkFactory = Arc::new(move |id| {
            let tx = tx.clone();
            Arc::new(move |m| {
                if let Ok(t) = tx.lock() {
                    let _ = t.send((id, m));
                }
            })
        });
        crate::work::start(dir.clone(), &sinks(1));
        let empty = crate::settings::Settings::parse("");
        let mut ui = Ui::new(te, f, empty.clone(), empty, Some((1, dir.clone())), sinks, 1280, 800, 1.0);
        let pump = |ui: &mut Ui, what: &str, done: &dyn Fn(&Ui) -> bool| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done(ui) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                assert!(!left.is_zero(), "timed out waiting for {what}; notice: {:?}", ui.notice);
                if let Ok((id, m)) = rx.recv_timeout(left) {
                    ui.on_tab_msg(id, m);
                }
            }
        };
        pump(&mut ui, "the repository", &|u| u.r.git.is_some());
        ui.enter_changes();
        pump(&mut ui, "the conflict view", &|u| {
            u.r.changes.in_progress == Some("merge") && u.r.changes.conflict.is_some()
        });
        ui.key(Key::Char(' '));
        let staged = String::from_utf8(g.output(&["diff", "--name-only", "--diff-filter=U"]).unwrap()).unwrap();
        assert!(staged.contains("a.txt"), "Space never marks a conflicted file resolved");
        ui.key(Key::Char('o'));
        ui.key(Key::Enter);
        pump(&mut ui, "the resolution", &|u| {
            u.r.changes.busy.is_none()
                && u.r.changes.status.as_ref().is_some_and(|s| s.entries.iter().all(|e| !e.conflicted))
        });
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "main\n");
        ui.conflict_click(conflict_view::KHit::Continue);
        pump(&mut ui, "the merge commit", &|u| u.r.changes.busy.is_none() && u.r.changes.in_progress.is_none());
        let parents = String::from_utf8(g.output(&["log", "-1", "--format=%P"]).unwrap()).unwrap();
        assert_eq!(parents.split_whitespace().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn changes_view_side(unstaged: bool) -> ffwd_core::changes::Side {
        if unstaged { ffwd_core::changes::Side::Unstaged } else { ffwd_core::changes::Side::Staged }
    }

    /// Damage-only painting must end where a full repaint would: after
    /// any action, the incrementally painted screen equals a fresh full
    /// paint (FF-52: the modal backdrop stayed after Settings closed).
    #[test]
    fn incremental_paint_matches_full_paint() {
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let sinks: SinkFactory = Arc::new(|_| Arc::new(|_| {}));
        let empty = crate::settings::Settings::parse("");
        let (w, h) = (1280, 800);
        let mut ui = Ui::new(te, f, empty.clone(), empty, None, sinks, w, h, 1.0);
        let mut screen = vec![0u32; w * h];
        ui.paint(&mut Canvas::new(&mut screen, w, h), true);
        type Step = (&'static str, fn(&mut Ui));
        let steps: [Step; 10] = [
            ("open settings", |u| u.open_settings()),
            ("hover in settings", |u| u.pointer(640, 300)),
            ("next page", |u| u.key(Key::NextTab)),
            ("close settings with Esc", |u| u.key(Key::Esc)),
            ("open settings again", |u| u.key(Key::Ctrl(','))),
            ("close by clicking outside", |u| u.click(5, 790)),
            ("open the palette", |u| u.open_palette()),
            ("close the palette", |u| u.key(Key::Esc)),
            ("open a dialog", |u| {
                u.open_dialog(DialogKind::Ask(menu::Ask {
                    title: "Reset main?".into(),
                    text: "Text".into(),
                    choices: vec![("Mixed".into(), crate::work::RefOp::Revert("x".into()), false)],
                }))
            }),
            ("close the dialog", |u| u.key(Key::Esc)),
        ];
        for (name, step) in steps {
            step(&mut ui);
            assert_partial_paint_is_full(&mut ui, &mut screen, name);
        }
    }

    /// Paints the damage onto `screen` and checks it against a full paint.
    fn assert_partial_paint_is_full(ui: &mut Ui, screen: &mut [u32], name: &str) {
        let (w, h) = (ui.w, ui.h);
        ui.paint(&mut Canvas::new(screen, w, h), false);
        let mut fresh = vec![0u32; w * h];
        ui.paint(&mut Canvas::new(&mut fresh, w, h), true);
        // The status bar shows the last paint's time, so it differs
        // between any two paints; everything above it must match.
        let n = ui.lay.status.y.max(0) as usize * w;
        let bad = screen[..n].iter().zip(&fresh[..n]).position(|(a, b)| a != b);
        assert!(bad.is_none(), "after '{name}': pixel {:?} differs from a full paint", bad.map(|i| (i % w, i / w)));
    }

    /// Every top-bar button lies inside a range the frame treats as a
    /// control; otherwise Windows takes its clicks as title-bar drags
    /// (FF-29: the settings gear did nothing).
    #[test]
    fn top_bar_buttons_are_not_in_the_drag_area() {
        let mut te = TextEngine::new();
        let Some(f) = load_fonts(&mut te) else { return };
        let sinks: SinkFactory = Arc::new(|_| Arc::new(|_| {}));
        let empty = crate::settings::Settings::parse("");
        let (w, h) = (1600, 1000);
        let mut ui = Ui::new(te, f, empty.clone(), empty, None, sinks, w, h, 1.0);
        let mut px = vec![0u32; w * h];
        ui.paint(&mut Canvas::new(&mut px, w, h), true);
        let g = ui.title_geometry();
        assert!(!ui.top_buttons.is_empty());
        for (r, a) in &ui.top_buttons {
            let cx = r.x + r.w / 2;
            assert!(
                g.controls.iter().any(|&(l, rr)| l <= cx && cx <= rr),
                "{a:?} at {cx} is in the drag area: {:?}",
                g.controls
            );
        }
    }

    #[test]
    fn conventional_prefixes() {
        assert_eq!(split_prefix("feat: add x"), ("feat: ", "add x"));
        assert_eq!(split_prefix("fix(ui)!: crash"), ("fix(ui)!: ", "crash"));
        assert_eq!(split_prefix("Merge branch 'x': y"), ("", "Merge branch 'x': y"));
        assert_eq!(split_prefix("Note: something"), ("", "Note: something"));
        assert_eq!(split_prefix("feat(two words): x"), ("", "feat(two words): x"));
        assert_eq!(split_prefix("no colon"), ("", "no colon"));
    }
}
