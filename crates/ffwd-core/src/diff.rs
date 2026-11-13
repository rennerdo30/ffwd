//! Commit details, changed files and per-file diffs, parsed from bounded
//! git output.

use crate::git::{Git, GitError, text};
use crate::log::Oid;

#[derive(Clone, Debug)]
pub struct Details {
    pub id: Oid,
    pub parents: Vec<Oid>,
    pub author: Box<str>,
    pub email: Box<str>,
    pub time: i64,
    pub committer: Box<str>,
    pub commit_time: i64,
    pub message: Box<str>,
    pub files: Vec<FileChange>,
    pub files_truncated: bool,
}

#[derive(Clone, Debug)]
pub struct FileChange {
    /// `A`, `M`, `D`, `R`, `C`, `T` or `?`.
    pub status: u8,
    pub path: Box<str>,
    pub old_path: Option<Box<str>>,
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
    Hunk,
    Meta,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old: u32,
    pub new: u32,
    pub text: Box<str>,
}

#[derive(Clone, Debug, Default)]
pub struct FileDiff {
    pub path: Box<str>,
    pub lines: Vec<DiffLine>,
    pub binary: bool,
    pub truncated: bool,
}

const MAX_FILES: usize = 5_000;
const MAX_DIFF_LINES: usize = 20_000;
const MAX_LINE_CHARS: usize = 2_000;

fn base_args(parents: &[Oid]) -> Vec<String> {
    match parents.first() {
        Some(p) => vec![p.to_string()],
        None => vec!["--root".into()],
    }
}

pub fn details(git: &Git, id: Oid) -> Result<Details, GitError> {
    let ids = id.to_string();
    let head = git.output_limited(
        &["show", "-s", "--no-show-signature", "--format=%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ct%x1f%B", &ids],
        1 << 20,
    )?;
    let mut f = head.splitn(7, |&b| b == 0x1f);
    let parents: Vec<Oid> =
        f.next().unwrap_or(b"").split(|&b| b == b' ').map(|p| p.trim_ascii()).filter_map(Oid::from_hex).collect();
    let author = text(f.next().unwrap_or(b""));
    let email = text(f.next().unwrap_or(b""));
    let time = std::str::from_utf8(f.next().unwrap_or(b"0")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let committer = text(f.next().unwrap_or(b""));
    let commit_time =
        std::str::from_utf8(f.next().unwrap_or(b"0")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let message = String::from_utf8_lossy(f.next().unwrap_or(b"")).trim_end().into();

    // First-parent diff: what this commit brought into its branch.
    let base = base_args(&parents);
    let mut args: Vec<&str> = vec!["diff-tree", "-r", "-M", "--raw", "--numstat", "-z", "--no-commit-id"];
    args.extend(base.iter().map(|s| s.as_str()));
    args.push(&ids);
    let raw = git.output_limited(&args, 16 << 20)?;
    let (files, files_truncated) = parse_raw_numstat(&raw);
    Ok(Details { id, parents, author, email, time, committer, commit_time, message, files, files_truncated })
}

/// Parses `diff-tree --raw --numstat -z` output: raw records first, then
/// numstat records in the same order.
pub fn parse_raw_numstat(b: &[u8]) -> (Vec<FileChange>, bool) {
    let mut files = Vec::new();
    let mut it = b.split(|&c| c == 0).peekable();
    let mut truncated = false;
    // Raw: ":100644 100644 <a> <b> M" \0 path [\0 newpath]
    while let Some(tok) = it.peek() {
        if !tok.starts_with(b":") {
            break;
        }
        let meta = it.next().unwrap_or_default();
        let status = meta.rsplit(|&c| c == b' ').next().and_then(|s| s.first().copied()).unwrap_or(b'?');
        let p1 = text(it.next().unwrap_or_default());
        let (path, old_path) =
            if matches!(status, b'R' | b'C') { (text(it.next().unwrap_or_default()), Some(p1)) } else { (p1, None) };
        if files.len() < MAX_FILES {
            files.push(FileChange { status, path, old_path, added: None, removed: None });
        } else {
            truncated = true;
        }
    }
    // Numstat: "<add>\t<del>\t<path>" \0, or "<add>\t<del>\t" \0 old \0 new \0
    let mut i = 0;
    while let Some(tok) = it.next() {
        if tok.is_empty() {
            continue;
        }
        let mut cols = tok.splitn(3, |&c| c == b'\t');
        let a = cols.next().and_then(|s| std::str::from_utf8(s).ok()?.parse().ok());
        let d = cols.next().and_then(|s| std::str::from_utf8(s).ok()?.parse().ok());
        if cols.next().is_some_and(|p| p.is_empty()) {
            it.next();
            it.next();
        }
        if let Some(f) = files.get_mut(i) {
            f.added = a;
            f.removed = d;
        }
        i += 1;
    }
    (files, truncated)
}

/// How a commit's diff is made (the diff bar's options).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffOpts {
    /// Lines of context around each change; `u32::MAX` is the whole file.
    pub context: u32,
    /// `-w`: changes in whitespace only are not shown.
    pub ignore_whitespace: bool,
}

impl Default for DiffOpts {
    fn default() -> Self {
        DiffOpts { context: 3, ignore_whitespace: false }
    }
}

pub fn file_diff(git: &Git, id: Oid, parents: &[Oid], file: &FileChange) -> Result<FileDiff, GitError> {
    file_diff_with(git, id, parents, file, DiffOpts::default())
}

pub fn file_diff_with(
    git: &Git,
    id: Oid,
    parents: &[Oid],
    file: &FileChange,
    opts: DiffOpts,
) -> Result<FileDiff, GitError> {
    let ids = id.to_string();
    let base = base_args(parents);
    // The whole file: more context than any file has lines.
    let ctx = format!("-U{}", opts.context.min(1_000_000));
    let mut args: Vec<&str> = vec!["diff-tree", "-p", "-M", "--no-color", "--no-ext-diff", "--no-commit-id", &ctx];
    if opts.ignore_whitespace {
        args.push("-w");
    }
    args.extend(base.iter().map(|s| s.as_str()));
    args.push(&ids);
    args.push("--");
    if let Some(old) = &file.old_path {
        args.push(old);
    }
    args.push(&file.path);
    let out = git.output_limited(&args, 32 << 20)?;
    Ok(parse_patch(&file.path, &out))
}

pub fn parse_patch(path: &str, b: &[u8]) -> FileDiff {
    let mut d = FileDiff { path: path.into(), ..Default::default() };
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    for raw in b.split(|&c| c == b'\n') {
        if d.lines.len() >= MAX_DIFF_LINES {
            d.truncated = true;
            break;
        }
        if raw.starts_with(b"@@") {
            in_hunk = true;
            let (o, n) = parse_hunk_header(raw);
            old = o;
            new = n;
            d.lines.push(DiffLine { kind: LineKind::Hunk, old: 0, new: 0, text: clip(raw) });
            continue;
        }
        if !in_hunk {
            if raw.starts_with(b"Binary files") || raw.starts_with(b"GIT binary patch") {
                d.binary = true;
            }
            continue;
        }
        let (kind, body) = match raw.first() {
            Some(b'+') => (LineKind::Add, &raw[1..]),
            Some(b'-') => (LineKind::Del, &raw[1..]),
            Some(b' ') => (LineKind::Context, &raw[1..]),
            Some(b'\\') => (LineKind::Meta, raw),
            Some(b'd') if raw.starts_with(b"diff --git") => {
                in_hunk = false;
                continue;
            }
            _ => continue,
        };
        let (o, n) = match kind {
            LineKind::Add => (0, new),
            LineKind::Del => (old, 0),
            LineKind::Context => (old, new),
            _ => (0, 0),
        };
        match kind {
            LineKind::Add => new += 1,
            LineKind::Del => old += 1,
            LineKind::Context => {
                old += 1;
                new += 1
            }
            _ => {}
        }
        let body = body.strip_suffix(b"\r").unwrap_or(body);
        d.lines.push(DiffLine { kind, old: o, new: n, text: clip(body) });
    }
    d
}

/// For each line, the line it replaces or is replaced by: within a run of
/// deleted lines followed by added lines, the k-th deletion pairs with the
/// k-th addition. `u32::MAX` means no partner. Used for word highlights
/// and for the side-by-side rows.
pub fn pair_lines(d: &FileDiff) -> Vec<u32> {
    let mut out = vec![u32::MAX; d.lines.len()];
    let mut i = 0;
    while i < d.lines.len() {
        if d.lines[i].kind != LineKind::Del {
            i += 1;
            continue;
        }
        let del0 = i;
        while i < d.lines.len() && d.lines[i].kind == LineKind::Del {
            i += 1;
        }
        let add0 = i;
        while i < d.lines.len() && d.lines[i].kind == LineKind::Add {
            i += 1;
        }
        for k in 0..(add0 - del0).min(i - add0) {
            out[del0 + k] = (add0 + k) as u32;
            out[add0 + k] = (del0 + k) as u32;
        }
    }
    out
}

/// The changed part of `a` against `b`, as a byte range of `a`: what is
/// left after the common prefix and suffix (on character boundaries).
pub fn changed_range(a: &str, b: &str) -> (usize, usize) {
    let pre = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum::<usize>();
    let (ra, rb) = (&a[pre..], &b[pre.min(b.len())..]);
    let suf =
        ra.chars().rev().zip(rb.chars().rev()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum::<usize>();
    (pre, a.len() - suf.min(ra.len()))
}

fn clip(b: &[u8]) -> Box<str> {
    let s = String::from_utf8_lossy(b);
    if s.chars().count() > MAX_LINE_CHARS { s.chars().take(MAX_LINE_CHARS).collect() } else { s.into() }
}

fn parse_hunk_header(h: &[u8]) -> (u32, u32) {
    // @@ -a,b +c,d @@
    let s = String::from_utf8_lossy(h);
    let mut old = 0;
    let mut new = 0;
    for part in s.split_whitespace().skip(1).take(2) {
        let num = |p: &str| p[1..].split(',').next().and_then(|n| n.parse().ok()).unwrap_or(0);
        if part.starts_with('-') {
            old = num(part);
        } else if part.starts_with('+') {
            new = num(part);
        }
    }
    (old, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_numstat_with_rename() {
        let b = b":100644 100644 aaaa bbbb M\0src/a.rs\0:100644 100644 cccc dddd R087\0old.rs\0new.rs\0\
3\t1\tsrc/a.rs\0-\t-\t\0old.rs\0new.rs\0";
        let (f, t) = parse_raw_numstat(b);
        assert!(!t);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].status, f[0].added, f[0].removed), (b'M', Some(3), Some(1)));
        assert_eq!(f[1].status, b'R');
        assert_eq!(f[1].old_path.as_deref(), Some("old.rs"));
        assert_eq!(&*f[1].path, "new.rs");
        assert_eq!(f[1].added, None);
    }

    /// The diff bar's options reach git: whitespace-only changes vanish
    /// with `ignore_whitespace`, and "whole file" is one hunk.
    #[test]
    fn diff_options() {
        let dir = std::env::temp_dir().join(format!("ffwd-diffopts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "t"],
            &["config", "user.email", "t@example.invalid"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let base: Vec<String> = (0..40).map(|i| format!("line {i}")).collect();
        std::fs::write(
            dir.join("f.txt"),
            base.join(
                "
",
            ) + "
",
        )
        .unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "a"]).unwrap();
        let mut next = base.clone();
        next[2] = "line  2".into(); // whitespace only
        next[30] = "line thirty".into();
        std::fs::write(
            dir.join("f.txt"),
            next.join(
                "
",
            ) + "
",
        )
        .unwrap();
        g.output(&["commit", "-qam", "b"]).unwrap();
        let id = Oid::from_hex(g.output(&["rev-parse", "HEAD"]).unwrap().trim_ascii()).unwrap();
        let parent = Oid::from_hex(g.output(&["rev-parse", "HEAD~1"]).unwrap().trim_ascii()).unwrap();
        let file = FileChange { status: b'M', path: "f.txt".into(), old_path: None, added: None, removed: None };
        let hunks = |o: DiffOpts| {
            file_diff_with(&g, id, &[parent], &file, o)
                .unwrap()
                .lines
                .iter()
                .filter(|l| l.kind == LineKind::Hunk)
                .count()
        };
        assert_eq!(hunks(DiffOpts::default()), 2);
        assert_eq!(hunks(DiffOpts { ignore_whitespace: true, ..Default::default() }), 1);
        let whole =
            file_diff_with(&g, id, &[parent], &file, DiffOpts { context: u32::MAX, ignore_whitespace: false }).unwrap();
        assert_eq!(whole.lines.iter().filter(|l| l.kind == LineKind::Hunk).count(), 1);
        assert_eq!(whole.lines.iter().filter(|l| l.kind == LineKind::Context).count(), 38);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pairs_and_changed_ranges() {
        let p = b"@@ -1,4 +1,4 @@
 ctx
-let a = 1;
-let b = 2;
+let a = 10;
 ctx
+new
";
        let d = parse_patch("x", p);
        // Lines: hunk, ctx, del, del, add, ctx, add.
        let pairs = pair_lines(&d);
        assert_eq!(pairs[2], 4);
        assert_eq!(pairs[4], 2);
        assert_eq!(pairs[3], u32::MAX, "a deletion with no addition left");
        assert_eq!(pairs[6], u32::MAX, "an addition after context pairs with nothing");
        assert_eq!(changed_range("let a = 1;", "let a = 10;"), (9, 9), "pure insertion: an empty range");
        assert_eq!(changed_range("let a = 10;", "let a = 1;"), (9, 10));
        assert_eq!(changed_range("äbc", "äxc"), ("ä".len(), "äb".len()));
        assert_eq!(changed_range("same", "same"), (4, 4));
    }

    #[test]
    fn patch_line_numbers() {
        let p = b"diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -10,3 +10,4 @@ fn f()\n ctx\n-old\n+new\n+more\n ctx2\n";
        let d = parse_patch("x", p);
        let nums: Vec<(LineKind, u32, u32)> = d.lines.iter().map(|l| (l.kind, l.old, l.new)).collect();
        assert_eq!(
            nums,
            vec![
                (LineKind::Hunk, 0, 0),
                (LineKind::Context, 10, 10),
                (LineKind::Del, 11, 0),
                (LineKind::Add, 0, 11),
                (LineKind::Add, 0, 12),
                (LineKind::Context, 12, 13),
            ]
        );
    }
}
