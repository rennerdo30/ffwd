//! Worker threads. Every Git call happens here, never on the UI thread
//! (AGENTS.md performance rule 1). Results come back as `Msg` through a
//! `Sink`: the winit event-loop proxy in a window, a channel in snapshots.

use ffwd_core::diff::{self, Details, FileDiff};
use ffwd_core::refs::{self, Refs, StatusSummary};
use ffwd_core::{Commit, Git, Oid, log};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub enum Msg {
    Repo(Result<(Git, i64), String>),
    Refs(Result<Refs, String>),
    Status(StatusSummary),
    Log {
        batch: Vec<Commit>,
        done: bool,
        elapsed_ms: f64,
    },
    Details {
        seq: u64,
        result: Result<Box<Details>, String>,
    },
    Diff {
        seq: u64,
        file: usize,
        result: Result<Box<FileDiff>, String>,
    },
    WorkerPanic(String),
    /// Repositories found for the "Open repository" picker.
    Repos(Vec<PathBuf>),
    OpProgress(String),
    OpDone(ffwd_core::ops::Outcome),
    /// An agent run's progress (experimental agents).
    Agent(ffwd_agent::Event),
    /// The working tree's changes (Local Changes view).
    Changes(Result<ffwd_core::changes::WorkStatus, String>),
    /// The diff of one changed file (`seq` drops stale answers).
    ChangesDiff {
        seq: u64,
        result: Result<Box<FileDiff>, String>,
    },
    /// A stage, unstage or commit finished.
    ChangesOp {
        commit: bool,
        result: Result<String, String>,
    },
    /// Search results (`seq` drops stale searches).
    SearchBatch {
        seq: u64,
        ids: Vec<Oid>,
        done: bool,
    },
    /// A merge, rebase, cherry-pick or revert is stopped (or none is).
    InProgress(Option<&'static str>),
    /// A conflicted file's conflicts (Local Changes).
    ConflictFile {
        seq: u64,
        path: String,
        result: Result<(Vec<ffwd_core::conflict::Seg>, &'static str), String>,
    },
    /// The Files view's tree with each file's lowercase path for the filter.
    FilesTree(crate::ui::FilesTreeResult),
    FileContent {
        seq: u64,
        result: Result<Box<crate::ui::FileLoaded>, String>,
    },
    FileBlame {
        seq: u64,
        result: Result<Box<ffwd_core::files::Blame>, String>,
    },
    FileHistory {
        seq: u64,
        result: Result<Vec<ffwd_core::Oid>, String>,
    },
    /// An operation for the activity log (FF-48).
    Activity(Box<crate::ui::ActivityEntry>),
    /// A branch, tag or history operation ended.
    RefOp(Result<String, ffwd_core::refops::OpError>),
    /// What a discard removed (for Ctrl+Z).
    Discarded(ffwd_core::changes::Backup),
    /// The last commit's message, for amend.
    LastMessage(String),
    /// A repository's own settings file, read on a worker.
    RepoSettings(PathBuf, crate::settings::Settings),
}

/// Runs a fetch, pull or push on a worker, reporting progress and the outcome.
pub fn run_op(git: Git, op: ffwd_core::ops::Op, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>, sink: &Sink) {
    spawn("op", sink, move |sink| {
        let out = ffwd_core::ops::run(&git, op, cancel, |line| sink(Msg::OpProgress(line)));
        sink(Msg::OpDone(out));
    });
}

/// A branch, tag or history operation (ffwd-core refops.rs).
#[derive(Clone, Debug)]
pub enum RefOp {
    Checkout(String),
    /// name, start point, check it out
    CreateBranch(String, String, bool),
    Rename(String, String),
    /// name, force
    DeleteBranch(String, bool),
    CreateTag(String, String),
    DeleteTag(String),
    Merge(String, ffwd_core::refops::MergeMode),
    Rebase(String),
    CherryPick(String),
    Revert(String),
    Reset(String, ffwd_core::refops::ResetMode),
    /// Abort the stopped merge, rebase, cherry-pick or revert.
    Abort(&'static str),
    /// message, options
    StashSave(String, ffwd_core::stash::SaveOptions),
    /// name, pop (drop after a clean apply)
    StashApply(String, bool),
    StashDrop(String),
    /// name, new message
    StashRename(String, String),
    /// name, URL
    RemoteAdd(String, String),
    RemoteSetUrl(String, String),
    /// old, new
    RemoteRename(String, String),
    RemoteRemove(String),
    RemoteFetch(String),
    RemoteTest(String),
    /// A fetch, pull or push chosen in a dialog (the UI starts it itself).
    Sync(ffwd_core::ops::Op),
}

impl RefOp {
    /// What the activity log calls it.
    pub fn title(&self) -> String {
        match self {
            RefOp::Checkout(t) => format!("Checkout {t}"),
            RefOp::CreateBranch(n, ..) => format!("Create branch {n}"),
            RefOp::Rename(o, n) => format!("Rename {o} to {n}"),
            RefOp::DeleteBranch(n, _) => format!("Delete branch {n}"),
            RefOp::CreateTag(n, _) => format!("Create tag {n}"),
            RefOp::DeleteTag(n) => format!("Delete tag {n}"),
            RefOp::Merge(w, _) => format!("Merge {w}"),
            RefOp::Rebase(o) => format!("Rebase onto {o}"),
            RefOp::CherryPick(c) => format!("Cherry-pick {}", &c[..c.len().min(10)]),
            RefOp::Revert(c) => format!("Revert {}", &c[..c.len().min(10)]),
            RefOp::Reset(to, _) => format!("Reset to {}", &to[..to.len().min(10)]),
            RefOp::Abort(k) => format!("Abort the {k}"),
            RefOp::StashSave(..) => "Stash".into(),
            RefOp::StashApply(n, true) => format!("Pop {n}"),
            RefOp::StashApply(n, false) => format!("Apply {n}"),
            RefOp::StashDrop(n) => format!("Drop {n}"),
            RefOp::StashRename(n, _) => format!("Rename {n}"),
            RefOp::RemoteAdd(n, _) => format!("Add remote {n}"),
            RefOp::RemoteSetUrl(n, _) => format!("Change the URL of {n}"),
            RefOp::RemoteRename(o, n) => format!("Rename remote {o} to {n}"),
            RefOp::RemoteRemove(n) => format!("Remove remote {n}"),
            RefOp::RemoteFetch(n) => format!("Fetch {n}"),
            RefOp::RemoteTest(n) => format!("Test {n}"),
            RefOp::Sync(op) => op.name().to_string(),
        }
    }

    pub fn verb(&self) -> &'static str {
        match self {
            RefOp::Checkout(_) => "Checking out",
            RefOp::CreateBranch(..) => "Creating the branch",
            RefOp::Rename(..) => "Renaming",
            RefOp::DeleteBranch(..) | RefOp::DeleteTag(_) => "Deleting",
            RefOp::CreateTag(..) => "Tagging",
            RefOp::Merge(..) => "Merging",
            RefOp::Rebase(_) => "Rebasing",
            RefOp::CherryPick(_) => "Cherry-picking",
            RefOp::Revert(_) => "Reverting",
            RefOp::Reset(..) => "Resetting",
            RefOp::Abort(_) => "Aborting",
            RefOp::StashSave(..) => "Stashing",
            RefOp::StashApply(..) => "Applying the stash",
            RefOp::StashDrop(_) => "Dropping the stash",
            RefOp::StashRename(..) => "Renaming the stash",
            RefOp::RemoteAdd(..) => "Adding the remote",
            RefOp::RemoteSetUrl(..) => "Changing the URL",
            RefOp::RemoteRename(..) => "Renaming the remote",
            RefOp::RemoteRemove(_) => "Removing the remote",
            RefOp::RemoteFetch(_) => "Fetching",
            RefOp::RemoteTest(_) => "Testing the connection",
            RefOp::Sync(op) => op.verb(),
        }
    }
}

pub fn refop(git: Git, op: RefOp, sink: &Sink) {
    spawn("refop", sink, move |sink| {
        use ffwd_core::refops as r;
        let t = std::time::Instant::now();
        let (res, ran) = ffwd_core::git::recording(|| match &op {
            RefOp::Checkout(t) => r::checkout(&git, t),
            RefOp::CreateBranch(n, at, sw) => r::create_branch(&git, n, at, *sw),
            RefOp::Rename(o, n) => r::rename_branch(&git, o, n),
            RefOp::DeleteBranch(n, f) => r::delete_branch(&git, n, *f),
            RefOp::CreateTag(n, at) => r::create_tag(&git, n, at, None),
            RefOp::DeleteTag(n) => r::delete_tag(&git, n),
            RefOp::Merge(w, m) => r::merge(&git, w, *m),
            RefOp::Rebase(o) => r::rebase(&git, o),
            RefOp::CherryPick(c) => r::cherry_pick(&git, c),
            RefOp::Revert(c) => r::revert(&git, c),
            RefOp::Reset(to, m) => r::reset(&git, to, *m),
            RefOp::Abort(kind) => ffwd_core::conflict::abort_op(&git, kind).map_err(r::OpError::Failed),
            RefOp::StashSave(m, o) => ffwd_core::stash::save(&git, m, *o),
            RefOp::StashApply(n, pop) => ffwd_core::stash::apply(&git, n, *pop),
            RefOp::StashDrop(n) => ffwd_core::stash::drop(&git, n),
            RefOp::StashRename(n, m) => ffwd_core::stash::rename(&git, n, m),
            RefOp::RemoteAdd(n, u) => ffwd_core::remotes::add(&git, n, u),
            RefOp::RemoteSetUrl(n, u) => ffwd_core::remotes::set_url(&git, n, u),
            RefOp::RemoteRename(o, n) => ffwd_core::remotes::rename(&git, o, n),
            RefOp::RemoteRemove(n) => ffwd_core::remotes::remove(&git, n),
            RefOp::RemoteFetch(n) => ffwd_core::remotes::fetch(&git, n),
            RefOp::RemoteTest(n) => {
                ffwd_core::remotes::test(&git, n, std::time::Duration::from_secs(30), Default::default())
            }
            RefOp::Sync(_) => Err(r::OpError::Failed("internal: a sync runs through start_op".into())),
        });
        let (ok, summary) = match &res {
            Ok(m) => (true, m.clone()),
            Err(e) => (false, e.to_string()),
        };
        let ms = t.elapsed().as_millis() as u32;
        sink(Msg::Activity(Box::new(crate::ui::ActivityEntry::new(&git, op.title(), ok, summary, ms, ran))));
        sink(Msg::RefOp(res));
        sink(Msg::InProgress(r::in_progress(&git)));
    });
}

/// What a Local Changes operation does.
pub enum ChangeOp {
    Stage(Vec<String>),
    Unstage(Vec<String>),
    Hunk(Box<ffwd_core::changes::Entry>, ffwd_core::changes::Side, usize),
    /// Chosen lines, as (hunk, body lines) per hunk.
    Lines(Box<ffwd_core::changes::Entry>, ffwd_core::changes::Side, Vec<(usize, Vec<usize>)>),
    Discard(Vec<ffwd_core::changes::Entry>),
    Restore(ffwd_core::changes::Backup),
    Ignore(String),
    Commit(String, ffwd_core::changes::CommitOptions),
    /// Resolve a conflicted file: one side whole, or the chosen text.
    TakeSide(String, bool),
    SaveResolved(String, String),
    /// Continue the stopped operation.
    Continue(&'static str),
}

pub fn changes_load(git: Git, sink: &Sink) {
    spawn("changes", sink, move |sink| {
        sink(Msg::Changes(ffwd_core::changes::load(&git).map_err(|e| e.to_string())));
        sink(Msg::InProgress(ffwd_core::refops::in_progress(&git)));
        // The sidebar's counts follow.
        if let Ok(s) = refs::status(&git) {
            sink(Msg::Status(s));
        }
    });
}

impl ChangeOp {
    /// What the activity log calls it.
    pub fn title(&self) -> String {
        let files = |n: usize| if n == 1 { "1 file".to_string() } else { format!("{n} files") };
        match self {
            ChangeOp::Stage(p) => format!("Stage {}", files(p.len())),
            ChangeOp::Unstage(p) => format!("Unstage {}", files(p.len())),
            ChangeOp::Hunk(e, side, _) => {
                let verb = if *side == ffwd_core::changes::Side::Unstaged { "Stage" } else { "Unstage" };
                format!("{verb} a hunk of {}", e.path)
            }
            ChangeOp::Lines(e, ..) => format!("Stage lines of {}", e.path),
            ChangeOp::Discard(v) => format!("Discard {}", files(v.len())),
            ChangeOp::Restore(_) => "Undo the discard".into(),
            ChangeOp::Ignore(p) => format!("Ignore {p}"),
            ChangeOp::Commit(..) => "Commit".into(),
            ChangeOp::TakeSide(p, _) => format!("Resolve {p}"),
            ChangeOp::SaveResolved(p, _) => format!("Resolve {p}"),
            ChangeOp::Continue(k) => format!("Continue the {k}"),
        }
    }
}

pub fn changes_op(git: Git, op: ChangeOp, sink: &Sink) {
    spawn("change-op", sink, move |sink| {
        use ffwd_core::changes as c;
        let t = std::time::Instant::now();
        let ((commit, result), ran) = ffwd_core::git::recording(|| match &op {
            ChangeOp::Stage(p) => (false, c::stage(&git, &refs(p)).map(|_| format!("Staged {}", count(p.len())))),
            ChangeOp::Unstage(p) => (false, c::unstage(&git, &refs(p)).map(|_| format!("Unstaged {}", count(p.len())))),
            ChangeOp::Hunk(e, side, h) => (
                false,
                c::toggle_hunk(&git, e, *side, *h).map(|_| {
                    if *side == c::Side::Unstaged { "Staged the hunk".into() } else { "Unstaged the hunk".into() }
                }),
            ),
            ChangeOp::Lines(e, side, picks) => {
                // The last hunk first: earlier hunks keep their indexes.
                let mut r = Ok(());
                let mut n = 0;
                for (h, lines) in picks.iter().rev() {
                    n += lines.len();
                    r = r.and_then(|_| c::toggle_lines(&git, e, *side, *h, lines));
                }
                let verb = if *side == c::Side::Unstaged { "Staged" } else { "Unstaged" };
                (false, r.map(|_| format!("{verb} {n} line{}", if n == 1 { "" } else { "s" })))
            }
            ChangeOp::Discard(entries) => (
                false,
                c::discard(&git, entries).map(|b| {
                    let n = entries.len();
                    sink(Msg::Discarded(b));
                    format!("Discarded the changes to {} (Ctrl+Z undoes it)", count(n))
                }),
            ),
            ChangeOp::Restore(b) => {
                (false, c::restore_backup(&git, b).map(|_| "Restored the discarded changes".into()))
            }
            ChangeOp::Ignore(p) => (false, c::ignore(&git, p).map(|_| format!("Added {p} to .gitignore"))),
            ChangeOp::Commit(m, o) => (true, c::commit(&git, m, o).map(|s| format!("Committed {s}"))),
            ChangeOp::TakeSide(p, ours) => (false, ffwd_core::conflict::take_side(&git, p, *ours)),
            ChangeOp::SaveResolved(p, t) => (false, ffwd_core::conflict::save_resolved(&git, p, t)),
            ChangeOp::Continue(kind) => (true, ffwd_core::conflict::continue_op(&git, kind)),
        });
        let (ok, summary) = match &result {
            Ok(m) => (true, m.clone()),
            Err(e) => (false, e.clone()),
        };
        let ms = t.elapsed().as_millis() as u32;
        sink(Msg::Activity(Box::new(crate::ui::ActivityEntry::new(&git, op.title(), ok, summary, ms, ran))));
        sink(Msg::ChangesOp { commit, result });
        sink(Msg::Changes(c::load(&git).map_err(|e| e.to_string())));
        sink(Msg::InProgress(ffwd_core::refops::in_progress(&git)));
        if let Ok(s) = refs::status(&git) {
            sink(Msg::Status(s));
        }
    });
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn count(n: usize) -> String {
    if n == 1 { "1 file".into() } else { format!("{n} files") }
}

pub fn last_message(git: Git, sink: &Sink) {
    spawn("last-message", sink, move |sink| {
        if let Ok(m) = ffwd_core::changes::last_message(&git) {
            sink(Msg::LastMessage(m));
        }
    });
}

/// Finds Git repositories in `roots` and one level below them (a folder
/// with a `.git` folder), for the repository picker. Bounded: at most 2,000
/// folders are looked at.
pub fn scan_repos(roots: Vec<PathBuf>, sink: &Sink) {
    spawn("scan", sink, move |sink| {
        let mut found = Vec::new();
        let looked = std::cell::Cell::new(0usize);
        let visit = |dir: &std::path::Path, found: &mut Vec<PathBuf>| {
            looked.set(looked.get() + 1);
            // A main repository has a .git folder; linked worktrees (a .git
            // file) belong to their repository and are not listed here.
            if dir.join(".git").is_dir() {
                found.push(dir.to_path_buf());
                return false;
            }
            true
        };
        for root in roots {
            if !visit(&root, &mut found) {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&root) else { continue };
            for e in rd.flatten().take(2000) {
                if looked.get() > 2000 {
                    break;
                }
                let p = e.path();
                let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(['.', '_']));
                if !hidden && p.is_dir() && visit(&p, &mut found) {
                    // One level deeper for folders that group repositories.
                    if let Ok(inner) = std::fs::read_dir(&p) {
                        for e in inner.flatten().take(200) {
                            if e.path().is_dir() {
                                visit(&e.path(), &mut found);
                            }
                        }
                    }
                }
            }
        }
        found.sort();
        found.dedup();
        sink(Msg::Repos(found));
    });
}

pub type Sink = Arc<dyn Fn(Msg) + Send + Sync>;

/// Runs `f` on a named thread; a panic is reported as a message instead
/// of taking the process down.
pub fn spawn(name: &str, sink: &Sink, f: impl FnOnce(&Sink) + Send + 'static) {
    let sink = sink.clone();
    let label = name.to_string();
    let _ = std::thread::Builder::new().name(format!("ffwd-{name}")).spawn(move || {
        if let Err(p) = catch_unwind(AssertUnwindSafe(|| f(&sink))) {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            sink(Msg::WorkerPanic(format!("{label}: {what}")));
        }
    });
}

/// Opens the repository and starts the loaders. Returns immediately.
pub fn start(path: PathBuf, sink: &Sink) {
    // All loaders start at once: git finds the repository from any folder
    // inside it, so nothing waits for discovery.
    let git = Git { root: path.clone() };
    spawn("repo", sink, move |sink| {
        let git = match Git::discover(&path) {
            Ok(g) => g,
            Err(e) => {
                sink(Msg::Repo(Err(e.to_string())));
                return;
            }
        };
        // The local UTC offset, as git sees it (no time-zone crate needed).
        let tz = git
            .output_limited(&["log", "-1", "--format=%cd", "--date=format-local:%z"], 64)
            .ok()
            .and_then(|b| parse_tz(String::from_utf8_lossy(&b).trim()))
            .unwrap_or(0);
        sink(Msg::Repo(Ok((git, tz))));
    });
    let g = git.clone();
    spawn("refs", sink, move |sink| sink(Msg::Refs(refs::load(&g).map_err(|e| e.to_string()))));
    let g = git.clone();
    spawn("status", sink, move |sink| {
        if let Ok(s) = refs::status(&g) {
            sink(Msg::Status(s));
        }
    });
    spawn("history", sink, move |sink| {
        let t = Instant::now();
        let r = log::stream(&git, 200, Duration::from_millis(100), |batch, done| {
            sink(Msg::Log { batch, done, elapsed_ms: t.elapsed().as_secs_f64() * 1000.0 });
        });
        if let Err(e) = r {
            sink(Msg::WorkerPanic(format!("history: {e}")));
        }
    });
}

fn parse_tz(s: &str) -> Option<i64> {
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    if rest.len() != 4 {
        return None;
    }
    let h: i64 = rest[..2].parse().ok()?;
    let m: i64 = rest[2..].parse().ok()?;
    Some(sign * (h * 3600 + m * 60))
}

enum Req {
    /// The tab closed or reloaded: the worker thread ends.
    Stop,
    Details {
        seq: u64,
        id: Oid,
    },
    Diff {
        seq: u64,
        id: Oid,
        parents: Vec<Oid>,
        file: usize,
        change: Box<diff::FileChange>,
        opts: diff::DiffOpts,
    },
    /// A changed file of the working tree or the index.
    ChangeDiff {
        seq: u64,
        entry: Box<ffwd_core::changes::Entry>,
        side: ffwd_core::changes::Side,
    },
}

/// One long-lived worker for commit details and diffs. It only keeps the
/// newest request of each kind: holding an arrow key through history never
/// queues up hundreds of git processes.
type Slot = Arc<(Mutex<(Option<Req>, Option<Req>)>, Condvar)>;

pub struct DetailWorker {
    slot: Slot,
}

impl DetailWorker {
    pub fn new(git: Git, sink: &Sink) -> DetailWorker {
        let slot: Slot = Arc::new((Mutex::new((None, None)), Condvar::new()));
        let s = slot.clone();
        spawn("details", sink, move |sink| {
            loop {
                let req = {
                    let (m, cv) = &*s;
                    let Ok(mut g) = m.lock() else { return };
                    loop {
                        if let Some(r) = g.0.take().or_else(|| g.1.take()) {
                            break r;
                        }
                        g = match cv.wait(g) {
                            Ok(g) => g,
                            Err(_) => return,
                        };
                    }
                };
                match req {
                    Req::Stop => return,
                    Req::Details { seq, id } => {
                        let result = diff::details(&git, id).map(Box::new).map_err(|e| e.to_string());
                        sink(Msg::Details { seq, result });
                    }
                    Req::Diff { seq, id, parents, file, change, opts } => {
                        let result = diff::file_diff_with(&git, id, &parents, &change, opts)
                            .map(Box::new)
                            .map_err(|e| e.to_string());
                        sink(Msg::Diff { seq, file, result });
                    }
                    Req::ChangeDiff { seq, entry, side } => {
                        let result =
                            ffwd_core::changes::diff(&git, &entry, side).map(Box::new).map_err(|e| e.to_string());
                        sink(Msg::ChangesDiff { seq, result });
                    }
                }
            }
        });
        DetailWorker { slot }
    }

    pub fn details(&self, seq: u64, id: Oid) {
        let (m, cv) = &*self.slot;
        if let Ok(mut g) = m.lock() {
            g.0 = Some(Req::Details { seq, id });
            g.1 = None;
            cv.notify_one();
        }
    }

    pub fn diff(
        &self,
        seq: u64,
        id: Oid,
        parents: Vec<Oid>,
        file: usize,
        change: diff::FileChange,
        opts: diff::DiffOpts,
    ) {
        let (m, cv) = &*self.slot;
        if let Ok(mut g) = m.lock() {
            g.1 = Some(Req::Diff { seq, id, parents, file, change: Box::new(change), opts });
            cv.notify_one();
        }
    }
}

impl DetailWorker {
    /// The diff of a changed file; replaces any waiting diff request.
    pub fn change_diff(&self, seq: u64, entry: ffwd_core::changes::Entry, side: ffwd_core::changes::Side) {
        let (m, cv) = &*self.slot;
        if let Ok(mut g) = m.lock() {
            g.1 = Some(Req::ChangeDiff { seq, entry: Box::new(entry), side });
            cv.notify_one();
        }
    }
}

impl Drop for DetailWorker {
    fn drop(&mut self) {
        let (m, cv) = &*self.slot;
        if let Ok(mut g) = m.lock() {
            g.0 = Some(Req::Stop);
            g.1 = None;
            cv.notify_one();
        }
    }
}
