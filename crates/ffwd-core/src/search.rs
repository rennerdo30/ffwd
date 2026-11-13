//! Commit search by message, author, content or file path, streamed from
//! `git log` over the same history the list shows. A newer search stops
//! the older one at once (its `git` process is killed), so typing never
//! queues up searches.

use crate::git::Git;
use crate::log::Oid;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The commit message (case-insensitive, literal text).
    Message,
    /// Author name or e-mail (case-insensitive, literal text).
    Author,
    /// Commits that add or remove this text (`git log -S`).
    Content,
    /// Commits that touch a path containing this text.
    Path,
}

impl Field {
    pub const ALL: [Field; 4] = [Field::Message, Field::Author, Field::Content, Field::Path];

    pub fn label(self) -> &'static str {
        match self {
            Field::Message => "Message",
            Field::Author => "Author",
            Field::Content => "Content",
            Field::Path => "Path",
        }
    }
}

/// The `git log` arguments for a search.
pub fn args(field: Field, query: &str) -> Vec<String> {
    let mut a: Vec<String> =
        ["log", "--branches", "--remotes", "--tags", "HEAD", "--topo-order", "--format=%H"].map(String::from).to_vec();
    match field {
        Field::Message => a.extend(["-i".into(), "--fixed-strings".into(), format!("--grep={query}")]),
        Field::Author => a.extend(["-i".into(), "--fixed-strings".into(), format!("--author={query}")]),
        Field::Content => a.push(format!("-S{query}")),
        Field::Path => {
            // Any path that contains the text, case-insensitive.
            let q = query.replace(['*', '?', '['], "");
            a.extend(["--".into(), format!(":(glob,icase)**/*{q}*"), format!(":(glob,icase)**/*{q}*/**")]);
        }
    }
    a
}

/// Runs a search, calling `batch` with matching commit ids (at most every
/// 100 ms, and once at the end with `done = true`). Stops early when `cur`
/// no longer equals `seq`. Returns the number of matches.
pub fn stream(
    git: &Git,
    field: Field,
    query: &str,
    seq: u64,
    cur: &Arc<AtomicU64>,
    mut batch: impl FnMut(Vec<Oid>, bool),
) -> usize {
    let a = args(field, query);
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    let Ok(mut child) = git.spawn(&refs) else {
        batch(Vec::new(), true);
        return 0;
    };
    let mut total = 0;
    let mut buf = Vec::new();
    let mut last = Instant::now();
    if let Some(out) = child.stdout.take() {
        for line in BufReader::with_capacity(64 * 1024, out).split(b'\n') {
            if cur.load(Ordering::Relaxed) != seq {
                let _ = child.kill();
                let _ = child.wait();
                return total;
            }
            let Ok(line) = line else { break };
            if let Some(id) = Oid::from_hex(line.trim_ascii()) {
                buf.push(id);
                total += 1;
            }
            if last.elapsed() >= Duration::from_millis(100) && !buf.is_empty() {
                batch(std::mem::take(&mut buf), false);
                last = Instant::now();
            }
        }
    }
    let _ = child.wait();
    batch(buf, true);
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_fields_find_the_right_commits() {
        let dir = std::env::temp_dir().join(format!("ffwd-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/ui")).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "Ada Lovelace"],
            &["config", "user.email", "ada@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        let commit = |file: &str, text: &str, msg: &str| {
            std::fs::write(dir.join(file), text).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", msg]).unwrap();
            Oid::from_hex(g.output(&["rev-parse", "HEAD"]).unwrap().trim_ascii()).unwrap()
        };
        let a = commit("README.md", "hello\n", "Add the README");
        let b = commit("src/ui/panel.rs", "fn draw_panel() {}\n", "Draw the panel");
        let c = commit("README.md", "hello\nworld\n", "Fix typo [ci]");
        let cur = Arc::new(AtomicU64::new(1));
        let run = |f: Field, q: &str| {
            let mut v = Vec::new();
            let mut done = false;
            stream(&g, f, q, 1, &cur, |b, d| {
                v.extend(b);
                done |= d;
            });
            assert!(done);
            v
        };
        assert_eq!(run(Field::Message, "readme"), vec![a], "case-insensitive");
        assert_eq!(run(Field::Message, "[ci]"), vec![c], "literal text, not a regex");
        assert_eq!(run(Field::Author, "lovelace").len(), 3);
        assert_eq!(run(Field::Content, "draw_panel"), vec![b]);
        assert_eq!(run(Field::Path, "PANEL"), vec![b], "any part of the path, case-insensitive");
        assert_eq!(run(Field::Path, "ui"), vec![b], "a folder matches too");
        // A superseded search stops and reports nothing more.
        let stale = Arc::new(AtomicU64::new(2));
        let mut got = 0;
        stream(&g, Field::Author, "ada", 1, &stale, |b, _| got += b.len());
        assert_eq!(got, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
