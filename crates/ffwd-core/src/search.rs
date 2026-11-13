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

/// One matching commit. For content and path searches, `files` holds the
/// files that matched (git's `-S` and pathspec filter the list), at most
/// [`FILES_PER_COMMIT`] of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub id: Oid,
    pub files: Vec<String>,
}

/// Matching files kept per commit, and over a whole search, so a broad
/// path search over a huge history stays bounded in memory.
pub const FILES_PER_COMMIT: usize = 1000;
pub const FILES_TOTAL: usize = 200_000;

impl Field {
    /// Content and path matches name the files that matched.
    pub fn lists_files(self) -> bool {
        matches!(self, Field::Content | Field::Path)
    }
}

/// The `git log` arguments for a search over the same history the list
/// shows: every branch, remote and tag, or with `head_only` only the
/// current branch. Commit lines start with a NUL byte; for content and
/// path searches the matching file names follow.
pub fn args(field: Field, query: &str, head_only: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-c", "core.quotePath=false", "log"].map(String::from).to_vec();
    if !head_only {
        a.extend(["--branches", "--remotes", "--tags"].map(String::from));
    }
    a.extend(["HEAD", "--topo-order", "--format=%x00%H"].map(String::from));
    if field.lists_files() {
        a.push("--name-only".into());
    }
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

/// Runs a search, calling `batch` with matching commits (at most every
/// 100 ms, and once at the end with `done = true`). Stops early when `cur`
/// no longer equals `seq`. Returns the number of matches.
pub fn stream(
    git: &Git,
    field: Field,
    query: &str,
    head_only: bool,
    seq: u64,
    cur: &Arc<AtomicU64>,
    mut batch: impl FnMut(Vec<Match>, bool),
) -> usize {
    let a = args(field, query, head_only);
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    let Ok(mut child) = git.spawn(&refs) else {
        batch(Vec::new(), true);
        return 0;
    };
    let mut total = 0;
    let mut kept = 0usize;
    // The last match stays open until the next commit line: its files follow it.
    let mut buf: Vec<Match> = Vec::new();
    let mut open: Option<Match> = None;
    let mut last = Instant::now();
    if let Some(out) = child.stdout.take() {
        for line in BufReader::with_capacity(64 * 1024, out).split(b'\n') {
            if cur.load(Ordering::Relaxed) != seq {
                let _ = child.kill();
                let _ = child.wait();
                return total;
            }
            let Ok(line) = line else { break };
            if let Some(hex) = line.strip_prefix(b"\0") {
                if let Some(id) = Oid::from_hex(hex.trim_ascii()) {
                    buf.extend(open.replace(Match { id, files: Vec::new() }));
                    total += 1;
                }
            } else if let Some(m) = &mut open
                && !line.is_empty()
                && m.files.len() < FILES_PER_COMMIT
                && kept < FILES_TOTAL
                && line.len() <= 4096
            {
                // Names with control characters stay quoted, as git prints them.
                m.files.push(String::from_utf8_lossy(&line).into_owned());
                kept += 1;
            }
            if last.elapsed() >= Duration::from_millis(100) && !buf.is_empty() {
                batch(std::mem::take(&mut buf), false);
                last = Instant::now();
            }
        }
    }
    let _ = child.wait();
    buf.extend(open);
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
            stream(&g, f, q, false, 1, &cur, |b, d| {
                v.extend(b.into_iter().map(|m| m.id));
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
        // Content and path matches name the files that matched, and only those.
        let files = |f: Field, q: &str| {
            let mut v = Vec::new();
            stream(&g, f, q, false, 1, &cur, |b, _| v.extend(b));
            v.into_iter().map(|m| (m.id, m.files)).collect::<Vec<_>>()
        };
        let d = commit("src/ui/other.rs", "fn draw_panel_again() {}\n", "Both");
        std::fs::write(dir.join("README.md"), "draw_panel in the docs\n").unwrap();
        g.output(&["commit", "-q", "-am", "Docs"]).unwrap();
        let e = Oid::from_hex(g.output(&["rev-parse", "HEAD"]).unwrap().trim_ascii()).unwrap();
        let one = |p: &str| vec![p.to_string()];
        assert_eq!(
            files(Field::Content, "draw_panel"),
            vec![(e, one("README.md")), (d, one("src/ui/other.rs")), (b, one("src/ui/panel.rs"))]
        );
        assert_eq!(files(Field::Path, "panel.rs"), vec![(b, one("src/ui/panel.rs"))]);
        assert!(files(Field::Message, "Both").iter().all(|(_, f)| f.is_empty()), "message matches list no files");
        // The current branch only: a commit on another branch is not found.
        g.output(&["checkout", "-q", "-b", "side"]).unwrap();
        let s = commit("side.txt", "x\n", "Only on side");
        g.output(&["checkout", "-q", "main"]).unwrap();
        let scoped = |head_only: bool| {
            let mut v = Vec::new();
            stream(&g, Field::Message, "only on side", head_only, 1, &cur, |b, _| {
                v.extend(b.into_iter().map(|m| m.id))
            });
            v
        };
        assert_eq!(scoped(false), vec![s]);
        assert!(scoped(true).is_empty(), "main's history has no such commit");
        // A superseded search stops and reports nothing more.
        let stale = Arc::new(AtomicU64::new(2));
        let mut got = 0;
        stream(&g, Field::Author, "ada", false, 1, &stale, |b, _| got += b.len());
        assert_eq!(got, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
