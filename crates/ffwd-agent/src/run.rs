//! One agent run, start to end, on a worker thread:
//!
//! 1. a new branch and worktree from `base` (never the user's checkout);
//! 2. the prompt file and the run record;
//! 3. the setup script, the agent, the after script (each sandboxed, with
//!    the time limit and cancel);
//! 4. ffwd commits what the agent left (`auto_commit`), counts the commits;
//! 5. the on-done script (on the host, for notifications).
//!
//! Merge and discard are separate calls that the UI makes only after the
//! user confirmed them.

use crate::config::{self, Config, SandboxKind};
use crate::proc::{self, Exit, Launch};
use crate::record::{self, Record, Status};
use crate::sandbox::{self, Ctx};
use ffwd_core::Git;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct Event {
    /// The repository path the UI used (its tab), to route the event.
    pub repo: PathBuf,
    /// The run's id; empty for repository-wide events.
    pub id: String,
    pub kind: EventKind,
}

#[derive(Clone, Debug)]
pub enum EventKind {
    /// The record changed (status, branch, worktree, counts).
    Update(Box<Record>),
    /// A step began: "Creating worktree", "Setup script", "Agent", …
    Phase(String),
    Log(Vec<String>),
    /// Runs recorded on disk for this repository.
    Loaded(Vec<Record>),
    /// A merge or discard ended.
    ActionDone {
        ok: bool,
        message: String,
    },
}

/// Lets the UI cancel a running run.
#[derive(Clone, Debug, Default)]
pub struct Handle {
    cancel: Arc<AtomicBool>,
}

impl Handle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A short, branch-safe name from the prompt plus a time suffix:
/// "Fix the login bug!" → `fix-the-login-bug-k3x9a`.
pub fn make_id(prompt: &str, secs: i64) -> String {
    let mut slug = String::new();
    for w in prompt.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).take(5) {
        if slug.len() + w.len() + 1 > 32 {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&w.to_ascii_lowercase());
    }
    if slug.is_empty() {
        slug.push_str("task");
    }
    let mut n = secs.rem_euclid(36i64.pow(5)) as u64;
    let mut suffix = [b'0'; 5];
    for d in suffix.iter_mut().rev() {
        *d = b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize];
        n /= 36;
    }
    format!("{slug}-{}", String::from_utf8_lossy(&suffix))
}

/// The prompt's first line, at most 72 characters.
pub fn title(prompt: &str) -> String {
    let first = prompt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Agent task");
    if first.chars().count() > 72 { format!("{}…", first.chars().take(71).collect::<String>()) } else { first.into() }
}

/// Git prints `E:/x/y` on Windows; programs such as `cmd` read `/` as an
/// option, so paths handed on use the platform's separator.
fn native(p: &str) -> PathBuf {
    if cfg!(windows) { PathBuf::from(p.replace('/', "\\")) } else { PathBuf::from(p) }
}

fn git_text(git: &Git, args: &[&str]) -> Result<String, String> {
    git.output_limited(args, 1 << 20).map(|b| String::from_utf8_lossy(&b).trim().to_string()).map_err(|e| e.to_string())
}

/// The run log: every output line, also on disk (capped at 8 MB).
struct Log {
    file: Option<std::fs::File>,
    bytes: usize,
    emit: Emit,
    repo: PathBuf,
    id: String,
}

impl Log {
    fn lines(&mut self, lines: Vec<String>) {
        if let Some(f) = &mut self.file {
            for l in &lines {
                if self.bytes < 8 << 20 {
                    self.bytes += l.len() + 1;
                    let _ = writeln!(f, "{l}");
                    if self.bytes >= 8 << 20 {
                        let _ = writeln!(f, "[ffwd: log truncated at 8 MB]");
                    }
                }
            }
        }
        (self.emit)(Event { repo: self.repo.clone(), id: self.id.clone(), kind: EventKind::Log(lines) });
    }

    fn note(&mut self, s: impl Into<String>) {
        self.lines(vec![format!("» {}", s.into())]);
    }

    fn phase(&mut self, s: &str) {
        self.note(s.to_string());
        (self.emit)(Event { repo: self.repo.clone(), id: self.id.clone(), kind: EventKind::Phase(s.into()) });
    }
}

/// Starts a run on a worker thread and returns its id and cancel handle at
/// once. `repo` is the tab's folder (any folder inside the repository).
pub fn start(cfg: Config, repo: PathBuf, prompt: String, emit: Emit) -> (String, Handle) {
    let id = make_id(&prompt, now());
    let handle = Handle::default();
    let h = handle.clone();
    let rid = id.clone();
    let _ = std::thread::Builder::new().name("ffwd-agent".into()).spawn(move || {
        let e2 = emit.clone();
        let (repo2, id2) = (repo.clone(), rid.clone());
        if let Err(p) = catch_unwind(AssertUnwindSafe(|| execute(&cfg, &repo, &rid, &prompt, &h.cancel, emit))) {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            e2(Event { repo: repo2, id: id2, kind: EventKind::Log(vec![format!("» internal error: {what}")]) });
        }
    });
    (id, handle)
}

/// The shell argv for a script.
fn script_argv(cfg: &Config, sandboxed: bool, script: &str) -> Result<Vec<String>, String> {
    let mut v = if cfg.script_shell.is_empty() {
        if sandboxed || !cfg!(windows) {
            vec!["sh".to_string(), "-c".into()]
        } else {
            vec!["cmd".to_string(), "/C".into()]
        }
    } else {
        config::split_command(&cfg.script_shell).map_err(|e| format!("agents.script_shell: {e}"))?
    };
    v.push(script.to_string());
    Ok(v)
}

fn execute(cfg: &Config, repo: &Path, id: &str, prompt: &str, cancel: &AtomicBool, emit: Emit) {
    let mut rec = Record {
        id: id.into(),
        repo: repo.into(),
        worktree: PathBuf::new(),
        branch: format!("{}{id}", cfg.branch_prefix),
        base: String::new(),
        title: title(prompt),
        started: now(),
        ended: None,
        status: Status::Preparing,
        detail: String::new(),
        commits: 0,
        shortstat: String::new(),
        sandbox: cfg.sandbox.label().into(),
    };
    let send =
        |r: &Record| emit(Event { repo: repo.into(), id: id.into(), kind: EventKind::Update(Box::new(r.clone())) });
    let mut log = Log { file: None, bytes: 0, emit: emit.clone(), repo: repo.into(), id: id.into() };
    let fail = |rec: &mut Record, log: &mut Log, why: String| {
        log.note(format!("failed: {why}"));
        rec.status = Status::Failed;
        rec.detail = why;
        rec.ended = Some(now());
    };
    send(&rec);

    // 1. Repository, base and folders.
    let prep = (|| -> Result<(Git, PathBuf, PathBuf, PathBuf), String> {
        if let Some(why) = cfg.refusal() {
            return Err(why);
        }
        let mut top = Git::discover(repo).map_err(|e| e.to_string())?;
        top.root = native(&top.root.to_string_lossy());
        let common = native(&git_text(&top, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?);
        rec.repo = top.root.clone();
        rec.base = git_text(&top, &["rev-parse", "--verify", &format!("{}^{{commit}}", cfg.base)])
            .map_err(|_| format!("the base \"{}\" is not a commit", cfg.base))?;
        git_text(&top, &["check-ref-format", "--branch", &rec.branch])
            .map_err(|_| format!("\"{}\" is not a valid branch name; check agents.branch_prefix", rec.branch))?;
        if git_text(&top, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{}", rec.branch)]).is_ok() {
            return Err(format!("the branch {} already exists", rec.branch));
        }
        let root = record::root(cfg, &top.root);
        let run_dir = record::run_dir(&root, id);
        rec.worktree = root.join(id);
        if rec.worktree.exists() {
            return Err(format!("{} already exists", rec.worktree.display()));
        }
        std::fs::create_dir_all(&run_dir).map_err(|e| format!("cannot create {}: {e}", run_dir.display()))?;
        Ok((top, common, root, run_dir))
    })();
    let (top, common, _root, run_dir) = match prep {
        Ok(v) => v,
        Err(why) => {
            fail(&mut rec, &mut log, why);
            send(&rec);
            return;
        }
    };
    log.file = std::fs::File::create(run_dir.join("log.txt")).ok();
    let save = |rec: &Record| {
        let _ = rec.save(&run_dir);
    };
    log.note(format!("ffwd agent run {id} (experimental) · {}", cfg.summary()));
    save(&rec);

    // 2. Worktree on a new branch.
    log.phase("Creating worktree");
    let wt = rec.worktree.to_string_lossy().into_owned();
    if let Err(e) = git_text(&top, &["worktree", "add", "-b", &rec.branch, &wt, &rec.base]) {
        fail(&mut rec, &mut log, format!("git worktree add failed: {e}"));
        save(&rec);
        send(&rec);
        return;
    }
    log.note(format!("worktree {wt} on {} from {}", rec.branch, &rec.base[..rec.base.len().min(10)]));
    send(&rec);
    let wgit = Git { root: rec.worktree.clone() };
    let git_dir = git_text(&wgit, &["rev-parse", "--path-format=absolute", "--git-dir"])
        .map(|s| native(&s))
        .unwrap_or_else(|_| common.join("worktrees").join(id));

    // 3. The prompt file.
    let template = match &cfg.prompt_template_file {
        Some(p) => std::fs::read(p)
            .map(|b| String::from_utf8_lossy(&b[..b.len().min(1 << 20)]).into_owned())
            .unwrap_or_else(|e| {
                log.note(format!("prompt_template_file {}: {e}; using the prompt as typed", p.display()));
                "{prompt}".into()
            }),
        None => cfg.prompt_template.clone(),
    };
    let top_s = top.root.to_string_lossy().into_owned();
    let full_prompt = config::expand(
        &template,
        &[("prompt", prompt), ("branch", &rec.branch), ("base", &rec.base), ("repo", &top_s), ("worktree", &wt)],
    );
    let prompt_file = run_dir.join("prompt.md");
    let _ = std::fs::write(&prompt_file, &full_prompt);
    let pf = prompt_file.to_string_lossy().into_owned();
    // Inside WSL the agent needs the Linux spelling of the paths.
    let in_wsl = cfg.sandbox.resolve() == SandboxKind::Wsl;
    let shown = |p: &Path| -> String {
        if in_wsl {
            sandbox::to_wsl(p, &cfg.wsl_path_map).unwrap_or_else(|_| p.to_string_lossy().into())
        } else {
            p.to_string_lossy().into()
        }
    };
    let vars: Vec<(String, String)> = vec![
        ("FFWD_AGENT_ID".into(), id.into()),
        ("FFWD_AGENT_BRANCH".into(), rec.branch.clone()),
        ("FFWD_AGENT_BASE".into(), rec.base.clone()),
        ("FFWD_AGENT_REPO".into(), shown(&top.root)),
        ("FFWD_AGENT_WORKTREE".into(), shown(&rec.worktree)),
        ("FFWD_AGENT_RUN_DIR".into(), shown(&run_dir)),
        ("FFWD_AGENT_PROMPT_FILE".into(), shown(&prompt_file)),
        ("FFWD_AGENT_TITLE".into(), rec.title.clone()),
    ];
    let wt_path = rec.worktree.clone();
    let ctx =
        Ctx { cfg, worktree: &wt_path, git_common: &common, git_dir: &git_dir, run_dir: &run_dir, vars: vars.clone() };
    rec.status = Status::Running;
    save(&rec);
    send(&rec);

    let run_script = |name: &str, script: &str, log: &mut Log, extra: &[(String, String)]| -> Exit {
        log.phase(name);
        let sandboxed = cfg.scripts_in_sandbox && cfg.sandbox.resolve() != SandboxKind::None;
        let argv = match script_argv(cfg, sandboxed, script) {
            Ok(a) => a,
            Err(e) => return Exit::SpawnFailed(e),
        };
        let mut c2 = ctx.clone();
        c2.vars.extend(extra.iter().cloned());
        let launch = if sandboxed {
            match sandbox::wrap(&argv, &c2, None) {
                Ok(l) => l,
                Err(e) => return Exit::SpawnFailed(e),
            }
        } else {
            let mut env = cfg.env.clone();
            env.extend(c2.vars.iter().cloned());
            Launch {
                program: argv[0].clone(),
                args: argv[1..].to_vec(),
                cwd: wt_path.clone(),
                clear_env: false,
                env,
                stdin: None,
            }
        };
        log.note(launch.display());
        let exit = proc::run(&launch, cfg.script_timeout, cancel, |l| log.lines(l));
        log.note(format!("{name}: {}", exit.describe()));
        exit
    };

    // 4. Setup script, agent.
    let mut exit = Exit::Code(0);
    if !cfg.setup_script.is_empty() {
        exit = run_script("Setup script", &cfg.setup_script, &mut log, &[]);
        if !exit.ok() && !matches!(exit, Exit::Cancelled) {
            rec.detail = format!("setup script: {}", exit.describe());
        }
    }
    if exit.ok() {
        log.phase("Agent");
        let pf_shown = shown(&prompt_file);
        let wt_shown = shown(&rec.worktree);
        let top_shown = shown(&top.root);
        let pairs = [
            ("prompt", full_prompt.as_str()),
            ("prompt_file", pf_shown.as_str()),
            ("worktree", wt_shown.as_str()),
            ("branch", rec.branch.as_str()),
            ("base", rec.base.as_str()),
            ("repo", top_shown.as_str()),
            ("run_id", id),
        ];
        exit = match config::argv(&cfg.command, &pairs)
            .and_then(|argv| sandbox::wrap(&argv, &ctx, cfg.prompt_stdin.then(|| full_prompt.clone().into_bytes())))
        {
            Ok(launch) => {
                log.note(launch.display());
                proc::run(&launch, cfg.timeout, cancel, |l| log.lines(l))
            }
            Err(e) => Exit::SpawnFailed(e),
        };
        log.note(format!("agent: {}", exit.describe()));
        if !exit.ok() {
            rec.detail = format!("agent: {}", exit.describe());
        }
    }
    rec.status = Status::Finishing;
    send(&rec);

    // 5. After script (not after a cancel: the user wants it to stop).
    let cancelled = cancel.load(Ordering::Relaxed);
    if !cfg.after_script.is_empty() && !cancelled {
        let code = match &exit {
            Exit::Code(c) => c.to_string(),
            e => e.describe(),
        };
        let a = run_script("After script", &cfg.after_script, &mut log, &[("FFWD_AGENT_EXIT".into(), code)]);
        if !a.ok() && rec.detail.is_empty() {
            rec.detail = format!("after script: {}", a.describe());
        }
    }

    // 6. Commit what the agent left, so it is on the branch even after a
    // cancel or time-out (never lose work).
    if cfg.auto_commit {
        log.phase("Committing");
        let changed = git_text(&wgit, &["status", "--porcelain"]).map(|s| !s.is_empty()).unwrap_or(false);
        if changed {
            let msg = config::expand(
                &cfg.commit_message,
                &[("title", &rec.title), ("prompt", prompt), ("branch", &rec.branch), ("run_id", id)],
            );
            let mut args = vec!["commit", "-q", "-m", msg.as_str()];
            if cfg.commit_no_verify {
                args.push("--no-verify");
            }
            match git_text(&wgit, &["add", "-A"]).and_then(|_| git_text(&wgit, &args)) {
                Ok(_) => log.note("committed the agent's changes"),
                Err(e) => {
                    log.note(format!("commit failed (changes stay in the worktree): {e}"));
                    if rec.detail.is_empty() {
                        rec.detail = "commit failed; the changes stay in the worktree".into();
                    }
                }
            }
        } else {
            log.note("nothing left to commit");
        }
    }
    rec.commits = git_text(&top, &["rev-list", "--count", &format!("{}..{}", rec.base, rec.branch)])
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    rec.shortstat = git_text(&top, &["diff", "--shortstat", &rec.base, &rec.branch]).unwrap_or_default();
    rec.status = match (&exit, cancelled) {
        (_, true) | (Exit::Cancelled, _) => Status::Cancelled,
        (Exit::TimedOut, _) => Status::TimedOut,
        (e, _) if e.ok() => Status::Done,
        _ => Status::Failed,
    };
    if rec.status == Status::Done && rec.detail.is_empty() {
        rec.detail = if rec.commits == 0 {
            "no changes".into()
        } else if rec.shortstat.is_empty() {
            format!("{} commit{}", rec.commits, if rec.commits == 1 { "" } else { "s" })
        } else {
            format!("{} commit{} · {}", rec.commits, if rec.commits == 1 { "" } else { "s" }, rec.shortstat)
        };
    }
    rec.ended = Some(now());
    save(&rec);

    // 7. On-done hook, on the host (a notification, a chat message, …).
    if !cfg.on_done_script.is_empty() {
        let mut extra = vec![
            ("FFWD_AGENT_STATUS".to_string(), rec.status.word().to_string()),
            ("FFWD_AGENT_COMMITS".to_string(), rec.commits.to_string()),
        ];
        extra.extend(vars.iter().map(|(k, v)| {
            // On the host the Windows paths are the right ones.
            match k.as_str() {
                "FFWD_AGENT_REPO" => (k.clone(), top_s.clone()),
                "FFWD_AGENT_WORKTREE" => (k.clone(), wt.clone()),
                "FFWD_AGENT_RUN_DIR" => (k.clone(), run_dir.to_string_lossy().into_owned()),
                "FFWD_AGENT_PROMPT_FILE" => (k.clone(), pf.clone()),
                _ => (k.clone(), v.clone()),
            }
        }));
        log.phase("On-done script");
        match script_argv(cfg, false, &cfg.on_done_script) {
            Ok(argv) => {
                let mut env = cfg.env.clone();
                env.extend(extra);
                let l = Launch {
                    program: argv[0].clone(),
                    args: argv[1..].to_vec(),
                    cwd: rec.worktree.clone(),
                    clear_env: false,
                    env,
                    stdin: None,
                };
                let never = AtomicBool::new(false);
                let e = proc::run(&l, Duration::from_secs(60), &never, |l| log.lines(l));
                log.note(format!("on-done script: {}", e.describe()));
            }
            Err(e) => log.note(e),
        }
    }
    log.note(format!("{} · {}", rec.status.word(), rec.detail));
    send(&rec);
}

/// Reads the recorded runs of the repository at `repo` on a worker.
pub fn load(cfg: Config, repo: PathBuf, emit: Emit) {
    let _ = std::thread::Builder::new().name("ffwd-agent-load".into()).spawn(move || {
        let runs = match Git::discover(&repo) {
            Ok(top) => record::load_all(&record::root(&cfg, &top.root)),
            Err(_) => Vec::new(),
        };
        emit(Event { repo, id: String::new(), kind: EventKind::Loaded(runs) });
    });
}

/// Is `rec` a run ffwd made for this repository (its worktree under the
/// agents folder)? Merge and discard refuse anything else.
fn owned(cfg: &Config, rec: &Record) -> Result<Git, String> {
    let top = Git::discover(&rec.repo).map_err(|e| e.to_string())?;
    let root = record::root(cfg, &top.root);
    if !rec.worktree.starts_with(&root) || !rec.branch.starts_with(&cfg.branch_prefix) {
        return Err("this run is not in ffwd's agents folder; ffwd leaves it alone".into());
    }
    Ok(top)
}

/// Merges the run's branch into the repository's current branch
/// (`git merge --no-ff`), after the user confirmed it.
pub fn merge(cfg: Config, tab: PathBuf, mut rec: Record, emit: Emit) {
    let _ = std::thread::Builder::new().name("ffwd-agent-merge".into()).spawn(move || {
        let done = |ok: bool, message: String| {
            emit(Event { repo: tab.clone(), id: rec.id.clone(), kind: EventKind::ActionDone { ok, message } })
        };
        let top = match owned(&cfg, &rec) {
            Ok(t) => t,
            Err(e) => return done(false, e),
        };
        let msg = format!("Merge {}: {}", rec.branch, rec.title);
        match git_text(&top, &["merge", "--no-ff", "-m", &msg, &rec.branch]) {
            Ok(_) => {
                rec.status = Status::Merged;
                let _ = rec.save(&record::run_dir(&record::root(&cfg, &top.root), &rec.id));
                emit(Event { repo: tab.clone(), id: rec.id.clone(), kind: EventKind::Update(Box::new(rec.clone())) });
                done(true, format!("Merged {} into the current branch.", rec.branch))
            }
            Err(e) => {
                let conflicts = git_text(&top, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
                let n = conflicts.lines().count();
                if n > 0 {
                    done(
                        false,
                        format!(
                            "The merge stopped with conflicts in {n} file{}. Resolve them, or run git merge --abort to go back.",
                            if n == 1 { "" } else { "s" }
                        ),
                    )
                } else {
                    done(false, format!("Merge failed: {e}"))
                }
            }
        }
    });
}

/// Removes the run's worktree and deletes its branch, after the user
/// confirmed it. The message names the commit to restore the branch from.
pub fn discard(cfg: Config, tab: PathBuf, mut rec: Record, emit: Emit) {
    let _ = std::thread::Builder::new().name("ffwd-agent-discard".into()).spawn(move || {
        let done = |ok: bool, message: String| {
            emit(Event { repo: tab.clone(), id: rec.id.clone(), kind: EventKind::ActionDone { ok, message } })
        };
        let top = match owned(&cfg, &rec) {
            Ok(t) => t,
            Err(e) => return done(false, e),
        };
        let tip = git_text(&top, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{}", rec.branch)]).ok();
        let wt = rec.worktree.to_string_lossy().into_owned();
        if rec.worktree.exists()
            && let Err(e) = git_text(&top, &["worktree", "remove", "--force", &wt])
        {
            return done(false, format!("Could not remove the worktree: {e}"));
        }
        let _ = git_text(&top, &["worktree", "prune"]);
        if tip.is_some()
            && let Err(e) = git_text(&top, &["branch", "-D", &rec.branch])
        {
            return done(false, format!("Removed the worktree, but could not delete the branch: {e}"));
        }
        rec.status = Status::Discarded;
        let _ = rec.save(&record::run_dir(&record::root(&cfg, &top.root), &rec.id));
        emit(Event { repo: tab.clone(), id: rec.id.clone(), kind: EventKind::Update(Box::new(rec.clone())) });
        done(
            true,
            match tip {
                Some(t) => format!(
                    "Discarded {}. To restore it: git branch {} {}",
                    rec.branch,
                    rec.branch,
                    &t[..t.len().min(12)]
                ),
                None => format!("Discarded {}.", rec.id),
            },
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn ids_are_branch_safe() {
        assert_eq!(make_id("Fix the login bug!", 0), "fix-the-login-bug-00000");
        assert_eq!(make_id("   ", 35), "task-0000z");
        assert_eq!(make_id("ä ö --- ../x", 36), "x-00010");
        let long = make_id("refactor everything in the whole repository please", 1);
        assert!(long.len() <= 32 + 6, "{long}");
        assert_eq!(title("\n  first line \nsecond"), "first line");
    }

    /// A throwaway repository with one commit, in the temp folder.
    fn temp_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ffwd-agent-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let g = Git { root: repo.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        repo
    }

    fn collect() -> (Emit, Arc<Mutex<Vec<Event>>>) {
        let v = Arc::new(Mutex::new(Vec::new()));
        let v2 = v.clone();
        (Arc::new(move |e| v2.lock().unwrap().push(e)), v)
    }

    fn wait_final(events: &Arc<Mutex<Vec<Event>>>) -> Record {
        let t = std::time::Instant::now();
        loop {
            if let Some(r) = events.lock().unwrap().iter().rev().find_map(|e| match &e.kind {
                EventKind::Update(r) if !r.status.active() => Some((**r).clone()),
                _ => None,
            }) {
                return r;
            }
            assert!(t.elapsed() < Duration::from_secs(60), "run did not end");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// End to end without a sandbox: the agent writes a file, ffwd commits
    /// it on the agent's branch, the user's checkout is untouched, and
    /// discard removes the worktree and the branch.
    #[test]
    fn run_commit_and_discard() {
        let repo = temp_repo("e2e");
        // The agent copies its prompt file into the worktree.
        let cmd = if cfg!(windows) { "cmd /C copy /Y {prompt_file} agent.txt" } else { "cp {prompt_file} agent.txt" };
        let cfg = Config {
            enabled: true,
            command: cmd.into(),
            sandbox: SandboxKind::None,
            allow_unsandboxed: true,
            env_inherit: true,
            setup_script: "echo setup-ran".into(),
            ..Config::default()
        };
        let (emit, events) = collect();
        let (id, _h) = start(cfg.clone(), repo.clone(), "Write the file".into(), emit.clone());
        let rec = wait_final(&events);
        assert_eq!(rec.status, Status::Done, "{rec:?}");
        assert_eq!(rec.commits, 1, "{rec:?}");
        assert!(rec.branch.starts_with("agent/") && rec.branch.ends_with(&id));
        let log: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| if let EventKind::Log(l) = &e.kind { Some(l.clone()) } else { None })
            .flatten()
            .collect();
        assert!(log.iter().any(|l| l.contains("setup-ran")), "{log:?}");
        let g = Git { root: repo.clone() };
        let shown =
            String::from_utf8_lossy(&g.output(&["show", &format!("{}:agent.txt", rec.branch)]).unwrap()).into_owned();
        assert!(shown.contains("Write the file"), "{shown}");
        assert!(!repo.join("agent.txt").exists(), "the user's checkout stays untouched");
        assert_eq!(record::load_all(&record::root(&cfg, &rec.repo)).len(), 1);

        let (emit2, ev2) = collect();
        discard(cfg.clone(), repo.clone(), rec.clone(), emit2);
        let t = std::time::Instant::now();
        let msg = loop {
            if let Some(m) = ev2.lock().unwrap().iter().find_map(|e| match &e.kind {
                EventKind::ActionDone { ok, message } => Some((*ok, message.clone())),
                _ => None,
            }) {
                break m;
            }
            assert!(t.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(msg.0, "{}", msg.1);
        assert!(msg.1.contains("git branch agent/"), "names how to restore: {}", msg.1);
        assert!(!rec.worktree.exists());
        assert!(g.output(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{}", rec.branch)]).is_err());
        assert!(record::load_all(&record::root(&cfg, &rec.repo)).is_empty());
        let _ = std::fs::remove_dir_all(repo.parent().unwrap());
        let _ = std::fs::remove_dir_all(record::root(&cfg, &rec.repo).parent().unwrap());
    }

    #[test]
    fn refused_config_never_touches_the_repository() {
        let repo = temp_repo("refuse");
        let cfg = Config { enabled: true, command: "agent".into(), sandbox: SandboxKind::None, ..Config::default() };
        let (emit, events) = collect();
        start(cfg.clone(), repo.clone(), "x".into(), emit);
        let rec = wait_final(&events);
        assert_eq!(rec.status, Status::Failed);
        assert!(rec.detail.contains("allow_unsandboxed"), "{}", rec.detail);
        let g = Git { root: repo.clone() };
        let wts = String::from_utf8_lossy(&g.output(&["worktree", "list"]).unwrap()).lines().count();
        assert_eq!(wts, 1);
        let _ = std::fs::remove_dir_all(repo.parent().unwrap());
    }
}
