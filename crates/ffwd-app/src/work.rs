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
    /// The last commit's message, for amend.
    LastMessage(String),
    /// A repository's own settings file, read on a worker.
    RepoSettings(PathBuf, crate::settings::Settings),
}

/// Runs a fetch, pull or push on a worker, reporting progress and the outcome.
pub fn run_op(git: Git, op: ffwd_core::ops::Op, sink: &Sink) {
    spawn("op", sink, move |sink| {
        let out = ffwd_core::ops::run(&git, op, |line| sink(Msg::OpProgress(line)));
        sink(Msg::OpDone(out));
    });
}

/// What a Local Changes operation does.
pub enum ChangeOp {
    Stage(Vec<String>),
    Unstage(Vec<String>),
    Hunk(Box<ffwd_core::changes::Entry>, ffwd_core::changes::Side, usize),
    Commit(String, ffwd_core::changes::CommitOptions),
}

pub fn changes_load(git: Git, sink: &Sink) {
    spawn("changes", sink, move |sink| {
        sink(Msg::Changes(ffwd_core::changes::load(&git).map_err(|e| e.to_string())));
        // The sidebar's counts follow.
        if let Ok(s) = refs::status(&git) {
            sink(Msg::Status(s));
        }
    });
}

pub fn changes_op(git: Git, op: ChangeOp, sink: &Sink) {
    spawn("change-op", sink, move |sink| {
        use ffwd_core::changes as c;
        let (commit, result) = match &op {
            ChangeOp::Stage(p) => (false, c::stage(&git, &refs(p)).map(|_| format!("Staged {}", count(p.len())))),
            ChangeOp::Unstage(p) => (false, c::unstage(&git, &refs(p)).map(|_| format!("Unstaged {}", count(p.len())))),
            ChangeOp::Hunk(e, side, h) => (
                false,
                c::toggle_hunk(&git, e, *side, *h).map(|_| {
                    if *side == c::Side::Unstaged { "Staged the hunk".into() } else { "Unstaged the hunk".into() }
                }),
            ),
            ChangeOp::Commit(m, o) => (true, c::commit(&git, m, o).map(|s| format!("Committed {s}"))),
        };
        sink(Msg::ChangesOp { commit, result });
        sink(Msg::Changes(c::load(&git).map_err(|e| e.to_string())));
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
                    Req::Diff { seq, id, parents, file, change } => {
                        let result =
                            diff::file_diff(&git, id, &parents, &change).map(Box::new).map_err(|e| e.to_string());
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

    pub fn diff(&self, seq: u64, id: Oid, parents: Vec<Oid>, file: usize, change: diff::FileChange) {
        let (m, cv) = &*self.slot;
        if let Ok(mut g) = m.lock() {
            g.1 = Some(Req::Diff { seq, id, parents, file, change: Box::new(change) });
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
