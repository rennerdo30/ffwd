//! Operations that change a repository or talk to remotes. They run the
//! `git` CLI, so hooks, signing, credential helpers and config behave
//! exactly as in the user's own Git. Every call runs on a worker thread,
//! reports progress lines and returns a typed outcome with the command and
//! its duration (the start of the operation log, FF-13).

use crate::git::{Git, GitError};
use std::io::Read;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Fetch,
    Pull,
    Push,
}

impl Op {
    pub fn verb(self) -> &'static str {
        match self {
            Op::Fetch => "Fetching",
            Op::Pull => "Pulling",
            Op::Push => "Pushing",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Op::Fetch => "Fetch",
            Op::Pull => "Pull",
            Op::Push => "Push",
        }
    }
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
}

const MAX_OUTPUT: usize = 64 * 1024;

/// Arguments for `op` on the current branch. Pull and push respect the
/// user's config (merge or rebase, push.default); a branch without an
/// upstream is pushed to `origin` and starts tracking it.
fn args(git: &Git, op: Op) -> Result<Vec<String>, GitError> {
    Ok(match op {
        Op::Fetch => vec!["fetch".into(), "--all".into(), "--prune".into(), "--progress".into()],
        Op::Pull => vec!["pull".into(), "--progress".into()],
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
/// a few times per second). Never panics on git output.
pub fn run(git: &Git, op: Op, mut progress: impl FnMut(String)) -> Outcome {
    let t = Instant::now();
    let fail = |summary: String, command: String| Outcome {
        op,
        ok: false,
        summary,
        command,
        output: String::new(),
        ms: t.elapsed().as_secs_f64() * 1000.0,
    };
    let a = match args(git, op) {
        Ok(a) => a,
        Err(e) => return fail(format!("{} failed: {e}", op.name()), String::new()),
    };
    let command = format!("git {}", a.join(" "));
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    let mut child = match git.spawn(&refs) {
        Ok(c) => c,
        Err(e) => return fail(format!("{} failed: {e}", op.name()), command),
    };
    let out_reader = child.stdout.take().map(|mut so| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = (&mut so).take(MAX_OUTPUT as u64).read_to_end(&mut b);
            // Keep draining so git never blocks on a full pipe.
            let _ = std::io::copy(&mut so, &mut std::io::sink());
            b
        })
    });
    // stderr carries progress ("Receiving objects:  45% (…)\r") and errors.
    let mut err = Vec::new();
    let mut line = Vec::new();
    let mut last = Instant::now();
    if let Some(mut se) = child.stderr.take() {
        let mut buf = [0u8; 4096];
        loop {
            let n = match se.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &b in &buf[..n] {
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
    }
    let status = child.wait();
    let out = out_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    let mut output = String::from_utf8_lossy(&err).into_owned();
    output.push_str(&String::from_utf8_lossy(&out));
    let ok = status.as_ref().is_ok_and(|s| s.success());
    let summary = summarize(op, ok, &output);
    Outcome { op, ok, summary, command, output, ms: t.elapsed().as_secs_f64() * 1000.0 }
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
        Op::Pull if lines.iter().any(|l| l.starts_with("Already up to date")) => "Pulled: already up to date".into(),
        Op::Pull => "Pulled".into(),
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
        let out = run(&git, Op::Fetch, |_| lines += 1);
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
    }
}
