//! Operations that change a repository or talk to remotes. They run the
//! `git` CLI, so hooks, signing, credential helpers and config behave
//! exactly as in the user's own Git. Every call runs on a worker thread,
//! reports progress lines and returns a typed outcome with the command and
//! its duration (the start of the operation log, FF-13).

use crate::git::{Git, GitError};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Fetch,
    Pull,
    /// Pull and merge (diverged branches, the user chose).
    PullMerge,
    /// Pull and rebase (diverged branches, the user chose).
    PullRebase,
    Push,
}

impl Op {
    pub fn verb(self) -> &'static str {
        match self {
            Op::Fetch => "Fetching",
            Op::Pull | Op::PullMerge => "Pulling",
            Op::PullRebase => "Pulling with rebase",
            Op::Push => "Pushing",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Op::Fetch => "Fetch",
            Op::Pull | Op::PullMerge | Op::PullRebase => "Pull",
            Op::Push => "Push",
        }
    }

    /// The toolbar button that shows this operation.
    pub fn button(self) -> Op {
        match self {
            Op::PullMerge | Op::PullRebase => Op::Pull,
            o => o,
        }
    }
}

/// Git refuses a plain pull when both sides have commits and the user's
/// config does not say merge or rebase.
pub fn diverged(output: &str) -> bool {
    output.contains("divergent branches") || output.contains("Need to specify how to reconcile")
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub op: Op,
    pub ok: bool,
    /// One line for the status bar.
    pub summary: String,
    /// The exact command, for the operation log.
    pub command: String,
    /// Git's output (stderr then stdout), capped.
    pub output: String,
    pub ms: f64,
    /// The user cancelled it.
    pub cancelled: bool,
}

const MAX_OUTPUT: usize = 64 * 1024;

/// Arguments for `op` on the current branch. Pull and push respect the
/// user's config (merge or rebase, push.default); a branch without an
/// upstream is pushed to `origin` and starts tracking it.
fn args(git: &Git, op: Op) -> Result<Vec<String>, GitError> {
    Ok(match op {
        Op::Fetch => vec!["fetch".into(), "--all".into(), "--prune".into(), "--progress".into()],
        Op::Pull => vec!["pull".into(), "--progress".into()],
        Op::PullMerge => vec!["pull".into(), "--no-rebase".into(), "--progress".into()],
        Op::PullRebase => vec!["pull".into(), "--rebase".into(), "--progress".into()],
        Op::Push => {
            let upstream = git.output_limited(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"], 4096);
            if upstream.is_ok() {
                vec!["push".into(), "--progress".into()]
            } else {
                let branch = String::from_utf8_lossy(&git.output_limited(&["symbolic-ref", "--short", "HEAD"], 4096)?)
                    .trim()
                    .to_string();
                vec!["push".into(), "--progress".into(), "--set-upstream".into(), "origin".into(), branch]
            }
        }
    })
}

/// Runs `op`, calling `progress` with git's latest progress line (at most
/// a few times per second). Setting `cancel` ends git (and its helpers).
/// Never panics on git output.
pub fn run(git: &Git, op: Op, cancel: Arc<AtomicBool>, mut progress: impl FnMut(String)) -> Outcome {
    let t = Instant::now();
    let fail = |summary: String, command: String| Outcome {
        op,
        ok: false,
        summary,
        command,
        output: String::new(),
        ms: t.elapsed().as_secs_f64() * 1000.0,
        cancelled: false,
    };
    let a = match args(git, op) {
        Ok(a) => a,
        Err(e) => return fail(format!("{} failed: {e}", op.name()), String::new()),
    };
    let command = format!("git {}", a.join(" "));
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    let (mut child, killer) = match git.spawn_killable(&refs) {
        Ok(c) => c,
        Err(e) => return fail(format!("{} failed: {e}", op.name()), command),
    };
    // stdout and stderr are read on their own threads; this loop only
    // waits on a channel, so a cancel never waits for the pipes to close
    // (a helper such as ssh may hold them after git itself has ended).
    let out_reader = child.stdout.take().map(|mut so| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = (&mut so).take(MAX_OUTPUT as u64).read_to_end(&mut b);
            // Keep draining so git never blocks on a full pipe.
            let _ = std::io::copy(&mut so, &mut std::io::sink());
            b
        })
    });
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    if let Some(mut se) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = se.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
    }
    // stderr carries progress ("Receiving objects:  45% (…)\r") and errors.
    let mut err = Vec::new();
    let mut line = Vec::new();
    let mut last = Instant::now();
    let mut killed: Option<Instant> = None;
    loop {
        if killed.is_none() && cancel.load(Ordering::Relaxed) {
            killer.kill();
            killed = Some(Instant::now());
        }
        // After a cancel, wait briefly for the rest, then stop listening.
        if killed.is_some_and(|k| k.elapsed() > Duration::from_secs(2)) {
            break;
        }
        let chunk = match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        for &b in &chunk {
            if err.len() < MAX_OUTPUT {
                err.push(b);
            }
            if b == b'\r' || b == b'\n' {
                if !line.is_empty() && last.elapsed().as_millis() >= 150 {
                    progress(String::from_utf8_lossy(&line).trim().to_string());
                    last = Instant::now();
                }
                line.clear();
            } else if line.len() < 512 {
                line.push(b);
            }
        }
    }
    let status = child.wait();
    let cancelled = cancel.load(Ordering::Relaxed);
    // A cancelled run does not wait for stdout either (see above).
    let out = match out_reader {
        Some(h) if !cancelled || h.is_finished() => h.join().unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut output = String::from_utf8_lossy(&err).into_owned();
    output.push_str(&String::from_utf8_lossy(&out));
    let ok = status.as_ref().is_ok_and(|s| s.success()) && !cancelled;
    let summary = if cancelled { format!("{} cancelled", op.name()) } else { summarize(op, ok, &output) };
    Outcome { op, ok, summary, command, output, ms: t.elapsed().as_secs_f64() * 1000.0, cancelled }
}

/// A short, human line from git's output.
pub fn summarize(op: Op, ok: bool, output: &str) -> String {
    let lines: Vec<&str> = output.split(['\n', '\r']).map(str::trim).filter(|l| !l.is_empty()).collect();
    if !ok {
        let why = lines
            .iter()
            .rev()
            .find(|l| l.starts_with("fatal:") || l.starts_with("error:") || l.contains("rejected"))
            .or(lines.last())
            .map(|l| l.trim_start_matches("fatal: ").trim_start_matches("error: "))
            .unwrap_or("git reported an error");
        return format!("{} failed: {why}", op.name());
    }
    let quiet = !lines.iter().any(|l| {
        l.contains("->") || l.starts_with("Updating") || l.starts_with("Fast-forward") || l.contains("Merge made")
    });
    match op {
        Op::Fetch if quiet => "Fetched: everything is up to date".into(),
        Op::Fetch => {
            let n = lines.iter().filter(|l| l.contains("->")).count();
            format!("Fetched: {n} ref{} updated", if n == 1 { "" } else { "s" })
        }
        Op::Pull | Op::PullMerge | Op::PullRebase if lines.iter().any(|l| l.starts_with("Already up to date")) => {
            "Pulled: already up to date".into()
        }
        Op::PullRebase => "Pulled and rebased".into(),
        Op::Pull | Op::PullMerge => "Pulled".into(),
        Op::Push if lines.iter().any(|l| l.starts_with("Everything up-to-date")) => "Pushed: nothing new".into(),
        Op::Push => "Pushed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fetches this repository from its remote; run by hand with
    /// `cargo test -p ffwd-core -- --ignored`.
    #[test]
    #[ignore]
    fn live_fetch_of_this_repository() {
        let git = Git { root: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")) };
        let mut lines = 0;
        let out = run(&git, Op::Fetch, Default::default(), |_| lines += 1);
        assert!(out.ok, "{} / {}", out.summary, out.output);
        assert!(out.summary.starts_with("Fetched"), "{}", out.summary);
        assert_eq!(out.command, "git fetch --all --prune --progress");
    }

    #[test]
    fn summaries() {
        assert_eq!(summarize(Op::Fetch, true, ""), "Fetched: everything is up to date");
        assert_eq!(
            summarize(
                Op::Fetch,
                true,
                "From github.com:x/y\n   a..b  main -> origin/main\n * [new branch] f -> origin/f\n"
            ),
            "Fetched: 2 refs updated"
        );
        assert_eq!(
            summarize(
                Op::Push,
                false,
                "To x\n ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs to 'x'\n"
            ),
            "Push failed: failed to push some refs to 'x'"
        );
        assert_eq!(
            summarize(Op::Pull, false, "fatal: There is no tracking information for the current branch.\n"),
            "Pull failed: There is no tracking information for the current branch."
        );
        assert_eq!(summarize(Op::Pull, true, "Already up to date.\n"), "Pulled: already up to date");
        assert!(diverged(
            "hint: You have divergent branches and need to specify how to reconcile them.\nfatal: Need to specify how to reconcile divergent branches.\n"
        ));
    }

    fn repo(dir: &std::path::Path) -> Git {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let g = Git { root: dir.to_path_buf() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        g
    }

    fn commit(g: &Git, file: &str, text: &str) {
        std::fs::write(g.root.join(file), text).unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", file]).unwrap();
    }

    /// Diverged branches: a plain pull is refused (and recognised) unless
    /// the user's config decides; merge and rebase each work.
    #[test]
    fn pull_when_diverged() {
        let base = std::env::temp_dir().join(format!("ffwd-ops-pull-{}", std::process::id()));
        let up = repo(&base.join("up"));
        commit(&up, "a.txt", "a\n");
        let down = Git { root: base.join("down") };
        Git { root: base.clone() }.output(&["clone", "-q", "up", "down"]).unwrap();
        for a in [
            &["config", "user.name", "ffwd test"][..],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            down.output(a).unwrap();
        }
        commit(&up, "up.txt", "up\n");
        commit(&down, "down.txt", "down\n");
        // The user's global pull.rebase or pull.ff may decide it; then the
        // question never comes and there is nothing more to test here.
        let out = run(&down, Op::Pull, Default::default(), |_| {});
        if out.ok {
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        assert!(diverged(&out.output), "{}", out.output);
        let out = run(&down, Op::PullRebase, Default::default(), |_| {});
        assert!(out.ok, "{} / {}", out.summary, out.output);
        let log = String::from_utf8(down.output(&["log", "--format=%s"]).unwrap()).unwrap();
        assert!(log.starts_with("down.txt\nup.txt\n"), "{log}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Cancelling ends git: a fetch from a remote that never answers stops.
    #[test]
    fn cancel_ends_git() {
        let dir = std::env::temp_dir().join(format!("ffwd-ops-cancel-{}", std::process::id()));
        let g = repo(&dir);
        commit(&g, "a.txt", "a\n");
        // A remote that never answers: the ext:: transport runs a command
        // that only waits.
        let wait = if cfg!(windows) { "ping -n 30 127.0.0.1" } else { "sleep 30" };
        g.output(&["remote", "add", "slow", &format!("ext::{wait}")]).unwrap();
        g.output(&["config", "protocol.ext.allow", "always"]).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            c.store(true, Ordering::Relaxed);
        });
        let t = Instant::now();
        let out = run(&g, Op::Fetch, cancel, |_| {});
        stopper.join().unwrap();
        assert!(out.cancelled && !out.ok, "{} / {}", out.summary, out.output);
        assert_eq!(out.summary, "Fetch cancelled");
        assert!(t.elapsed() < Duration::from_secs(15), "took {:?}", t.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
