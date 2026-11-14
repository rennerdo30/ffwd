//! The working tree: what changed, staging (whole files or single hunks)
//! and committing. Everything runs the `git` CLI on a worker thread, so the
//! user's hooks, signing and config apply exactly as on the command line.

use crate::diff::{FileDiff, parse_patch};
use crate::git::{Git, GitError};

/// One changed path. `staged` and `unstaged` are Git's X and Y status
/// letters (`.` means unchanged on that side, `?` untracked).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: Box<str>,
    /// The source of a staged rename or copy.
    pub old_path: Option<Box<str>>,
    pub staged: u8,
    pub unstaged: u8,
    pub conflicted: bool,
}

impl Entry {
    pub fn untracked(&self) -> bool {
        self.unstaged == b'?'
    }

    /// An ignored file or folder (shown only on request; FF-35).
    pub fn ignored(&self) -> bool {
        self.unstaged == b'!'
    }

    /// An ignored path as an entry of the unstaged list.
    pub fn ignored_entry(path: &str) -> Entry {
        Entry { path: path.into(), old_path: None, staged: b'.', unstaged: b'!', conflicted: false }
    }

    /// Shown in the staged list.
    pub fn in_staged(&self) -> bool {
        !self.conflicted && self.staged != b'.' && self.staged != b'?'
    }

    /// Shown in the unstaged list (conflicts too: they need the user).
    pub fn in_unstaged(&self) -> bool {
        self.conflicted || self.unstaged != b'.'
    }

    /// The letter shown for a side.
    pub fn letter(&self, side: Side) -> u8 {
        match side {
            _ if self.conflicted => b'U',
            Side::Staged => self.staged,
            Side::Unstaged if self.untracked() => b'A',
            Side::Unstaged if self.ignored() => b'I',
            Side::Unstaged => self.unstaged,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Unstaged,
    Staged,
}

#[derive(Clone, Debug, Default)]
pub struct WorkStatus {
    pub entries: Vec<Entry>,
    /// More than `MAX_ENTRIES` changes: the rest is not listed.
    pub truncated: bool,
}

impl WorkStatus {
    pub fn side(&self, side: Side) -> Vec<&Entry> {
        self.entries.iter().filter(|e| if side == Side::Staged { e.in_staged() } else { e.in_unstaged() }).collect()
    }
}

/// The target is 100k changed files (SPEC §3); beyond that the list stops.
pub const MAX_ENTRIES: usize = 200_000;

/// Reads the working tree's state (`git status --porcelain=v2 -z`).
/// The most ignored paths listed.
pub const MAX_IGNORED: usize = 10_000;

/// Ignored files, a wholly ignored folder as one `dir/` entry (so a
/// `target/` or `node_modules/` stays one line), at most [`MAX_IGNORED`].
pub fn ignored(git: &Git) -> Result<Vec<String>, GitError> {
    let out =
        git.output_limited(&["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"], 8 << 20)?;
    Ok(out
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .take(MAX_IGNORED)
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

pub fn load(git: &Git) -> Result<WorkStatus, GitError> {
    let out = git.output_limited(&["status", "--porcelain=v2", "-z", "--untracked-files=all"], 256 << 20)?;
    Ok(parse_status(&out))
}

pub fn parse_status(out: &[u8]) -> WorkStatus {
    let mut s = WorkStatus::default();
    let mut it = out.split(|&b| b == 0).peekable();
    let text = |b: &[u8]| -> Box<str> { String::from_utf8_lossy(b).into() };
    while let Some(e) = it.next() {
        if s.entries.len() >= MAX_ENTRIES {
            s.truncated = true;
            break;
        }
        // Fields are separated by spaces; the path is the last field and
        // may itself contain spaces, so split a fixed number of times.
        match e.first() {
            Some(b'1') => {
                let f: Vec<&[u8]> = e.splitn(9, |&b| b == b' ').collect();
                if f.len() == 9 && f[1].len() == 2 {
                    s.entries.push(Entry {
                        path: text(f[8]),
                        old_path: None,
                        staged: f[1][0],
                        unstaged: f[1][1],
                        conflicted: false,
                    });
                }
            }
            Some(b'2') => {
                let f: Vec<&[u8]> = e.splitn(10, |&b| b == b' ').collect();
                let orig = it.next().map(text);
                if f.len() == 10 && f[1].len() == 2 {
                    s.entries.push(Entry {
                        path: text(f[9]),
                        old_path: orig,
                        staged: f[1][0],
                        unstaged: f[1][1],
                        conflicted: false,
                    });
                }
            }
            Some(b'u') => {
                let f: Vec<&[u8]> = e.splitn(11, |&b| b == b' ').collect();
                if f.len() == 11 {
                    s.entries.push(Entry {
                        path: text(f[10]),
                        old_path: None,
                        staged: b'U',
                        unstaged: b'U',
                        conflicted: true,
                    });
                }
            }
            Some(b'?') if e.len() > 2 => s.entries.push(Entry {
                path: text(&e[2..]),
                old_path: None,
                staged: b'.',
                unstaged: b'?',
                conflicted: false,
            }),
            _ => {}
        }
    }
    s.entries.sort_by(|a, b| a.path.cmp(&b.path));
    s
}

/// The raw patch of one side of `e` (what `diff` parses and what hunk
/// staging cuts hunks from).
fn raw_patch(git: &Git, e: &Entry, side: Side) -> Result<Vec<u8>, GitError> {
    raw_patch_with(git, e, side, false)
}

/// The patch, with `-w` when `ignore_ws`. Staging always uses the exact
/// patch (`ignore_ws` false): its hunks and lines are what gets applied.
fn raw_patch_with(git: &Git, e: &Entry, side: Side, ignore_ws: bool) -> Result<Vec<u8>, GitError> {
    let base = ["diff", "--no-color", "--no-ext-diff", "-U3"];
    let mut args: Vec<&str> = base.to_vec();
    if ignore_ws {
        args.push("-w");
    }
    if e.untracked() && side == Side::Unstaged {
        // An untracked file: everything is added. Exit code 1 = "differs".
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        args.extend(["--no-index", "--", null, &e.path]);
        return git.run_with(&args, None, &[1], 32 << 20);
    }
    if side == Side::Staged {
        args.push("--cached");
        args.push("-M");
    }
    args.push("--");
    if let (Side::Staged, Some(old)) = (side, &e.old_path) {
        args.push(old);
    }
    args.push(&e.path);
    git.output_limited(&args, 32 << 20)
}

/// The diff of one side of a changed file.
pub fn diff(git: &Git, e: &Entry, side: Side) -> Result<FileDiff, GitError> {
    diff_with(git, e, side, false)
}

/// The diff of one side, with changes in whitespace only left out when
/// `ignore_ws`. Such a diff is for reading: its hunks and lines don't
/// match the patch that staging applies.
pub fn diff_with(git: &Git, e: &Entry, side: Side, ignore_ws: bool) -> Result<FileDiff, GitError> {
    Ok(parse_patch(&e.path, &raw_patch_with(git, e, side, ignore_ws)?))
}

/// Splits a one-file patch into its header and hunks.
pub fn split_hunks(patch: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut header = Vec::new();
    let mut hunks: Vec<Vec<u8>> = Vec::new();
    for line in patch.split_inclusive(|&b| b == b'\n') {
        if line.starts_with(b"@@") {
            hunks.push(line.to_vec());
        } else if let Some(h) = hunks.last_mut() {
            h.extend_from_slice(line);
        } else {
            header.extend_from_slice(line);
        }
    }
    (header, hunks)
}

fn why(e: GitError) -> String {
    match e {
        GitError::Failed { stderr, .. } if !stderr.trim().is_empty() => {
            let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            let pick = lines
                .iter()
                .rev()
                .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
                .or(lines.last())
                .copied()
                .unwrap_or("git reported an error");
            pick.trim_start_matches("fatal: ").trim_start_matches("error: ").to_string()
        }
        e => e.to_string(),
    }
}

/// Is there a commit yet? (`git restore --staged` needs one.)
fn has_head(git: &Git) -> bool {
    git.output_limited(&["rev-parse", "--verify", "--quiet", "HEAD"], 256).is_ok()
}

/// Stages whole files (also deletions and untracked files).
pub fn stage(git: &Git, paths: &[&str]) -> Result<(), String> {
    for chunk in paths.chunks(500) {
        let mut args = vec!["add", "-A", "--"];
        args.extend_from_slice(chunk);
        git.run_with(&args, None, &[], 1 << 20).map_err(why)?;
    }
    Ok(())
}

/// Unstages whole files; the working tree keeps them.
pub fn unstage(git: &Git, paths: &[&str]) -> Result<(), String> {
    let head = has_head(git);
    for chunk in paths.chunks(500) {
        let mut args = if head { vec!["restore", "--staged", "--"] } else { vec!["rm", "--cached", "-r", "-q", "--"] };
        args.extend_from_slice(chunk);
        git.run_with(&args, None, &[], 1 << 20).map_err(why)?;
    }
    Ok(())
}

/// Stages (from the unstaged side) or unstages (from the staged side) the
/// hunk with index `hunk` of `e`. An untracked file is staged whole.
pub fn toggle_hunk(git: &Git, e: &Entry, side: Side, hunk: usize) -> Result<(), String> {
    if e.untracked() && side == Side::Unstaged {
        return stage(git, &[&e.path]);
    }
    let patch = raw_patch(git, e, side).map_err(why)?;
    let (header, hunks) = split_hunks(&patch);
    let Some(h) = hunks.get(hunk) else {
        return Err("the file changed meanwhile; the diff was reloaded".into());
    };
    let mut one = header;
    one.extend_from_slice(h);
    if !one.ends_with(b"\n") {
        one.push(b'\n');
    }
    let mut args = vec!["apply", "--cached", "--recount", "--whitespace=nowarn"];
    if side == Side::Staged {
        args.push("-R");
    }
    args.push("-");
    git.run_with(&args, Some(&one), &[], 1 << 20).map_err(why)?;
    Ok(())
}

/// Keeps only the chosen changed lines of one hunk. `chosen` holds the
/// indexes of the hunk's body lines (0 = the line after the `@@` header)
/// to keep. For a forward patch, an unchosen `+` is dropped and an
/// unchosen `-` becomes context; `reverse` (unstaging) swaps the roles,
/// because the patch is applied with `-R`.
pub fn select_lines(hunk: &[u8], chosen: &[usize], reverse: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(hunk.len());
    for (i, line) in hunk.split_inclusive(|&b| b == b'\n').enumerate() {
        if i == 0 {
            out.extend_from_slice(line); // the @@ header; --recount fixes it
            continue;
        }
        let keep = chosen.contains(&(i - 1));
        match line.first() {
            Some(b'+') if !keep && !reverse => {}
            Some(b'-') if !keep && reverse => {}
            Some(b'+') | Some(b'-') if !keep => {
                out.push(b' ');
                out.extend_from_slice(&line[1..]);
            }
            _ => out.extend_from_slice(line),
        }
    }
    out
}

/// Stages (unstaged side) or unstages (staged side) only the chosen lines
/// of hunk `hunk` (see `select_lines` for `lines`).
pub fn toggle_lines(git: &Git, e: &Entry, side: Side, hunk: usize, lines: &[usize]) -> Result<(), String> {
    if e.untracked() && side == Side::Unstaged {
        // Git can't stage part of a file it doesn't track yet: mark it
        // as intended to add, then its diff is an ordinary one.
        git.run_with(&["add", "-N", "--", &e.path], None, &[], 1 << 16).map_err(why)?;
        let tracked = Entry { unstaged: b'A', ..e.clone() };
        return toggle_lines(git, &tracked, side, hunk, lines);
    }
    let patch = raw_patch(git, e, side).map_err(why)?;
    let (header, hunks) = split_hunks(&patch);
    let Some(h) = hunks.get(hunk) else {
        return Err("the file changed meanwhile; the diff was reloaded".into());
    };
    let mut one = header;
    one.extend_from_slice(&select_lines(h, lines, side == Side::Staged));
    if !one.ends_with(b"\n") {
        one.push(b'\n');
    }
    let mut args = vec!["apply", "--cached", "--recount", "--whitespace=nowarn"];
    if side == Side::Staged {
        args.push("-R");
    }
    args.push("-");
    git.run_with(&args, Some(&one), &[], 1 << 20).map_err(why)?;
    Ok(())
}

/// What a discard removed, kept so it can be put back. Each file's
/// content was written to Git's object store first (`git hash-object
/// -w`), so nothing is lost until Git's garbage collection, and the undo
/// works even after ffwd restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Backup {
    /// (path, blob id of the discarded content).
    pub files: Vec<(String, String)>,
}

/// Throws away the unstaged changes of `entries` (untracked files are
/// deleted), after backing each file up. Staged changes stay.
pub fn discard(git: &Git, entries: &[Entry]) -> Result<Backup, String> {
    let mut backup = Backup::default();
    for e in entries {
        let full = git.root.join(&*e.path);
        if !full.is_file() {
            continue; // a deletion: the content is in the index already
        }
        let id = git.run_with(&["hash-object", "-w", "--", &e.path], None, &[], 4096).map_err(why)?;
        backup.files.push((e.path.to_string(), String::from_utf8_lossy(&id).trim().to_string()));
    }
    let tracked: Vec<&str> = entries.iter().filter(|e| !e.untracked() && !e.conflicted).map(|e| &*e.path).collect();
    for chunk in tracked.chunks(500) {
        let mut args = vec!["restore", "--worktree", "--"];
        args.extend_from_slice(chunk);
        git.run_with(&args, None, &[], 1 << 20).map_err(why)?;
    }
    for e in entries.iter().filter(|e| e.untracked()) {
        std::fs::remove_file(git.root.join(&*e.path)).map_err(|err| format!("{}: {err}", e.path))?;
    }
    Ok(backup)
}

/// Puts back what `discard` removed.
pub fn restore_backup(git: &Git, b: &Backup) -> Result<(), String> {
    for (path, id) in &b.files {
        let data = git.run_with(&["cat-file", "blob", id], None, &[], 512 << 20).map_err(why)?;
        let full = git.root.join(path);
        if let Some(d) = full.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{path}: {e}"))?;
        }
        std::fs::write(&full, data).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(())
}

/// Adds `pattern` to the repository's `.gitignore` (once).
pub fn ignore(git: &Git, pattern: &str) -> Result<(), String> {
    let p = git.root.join(".gitignore");
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    if old.lines().any(|l| l.trim() == pattern) {
        return Ok(());
    }
    let mut text = old;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern);
    text.push('\n');
    std::fs::write(&p, text).map_err(|e| format!(".gitignore: {e}"))
}

#[derive(Clone, Debug, Default)]
pub struct CommitOptions {
    pub amend: bool,
    /// `--no-verify`: skip the pre-commit and commit-msg hooks.
    pub no_verify: bool,
    /// `--signoff`.
    pub signoff: bool,
}

/// Commits the index with `message` (subject, blank line, description).
/// Returns the new commit's short id and subject.
pub fn commit(git: &Git, message: &str, opts: &CommitOptions) -> Result<String, String> {
    if message.trim().is_empty() && !opts.amend {
        return Err("the commit message is empty".into());
    }
    let mut args = vec!["commit", "--cleanup=strip", "-F", "-"];
    if opts.amend {
        args.push("--amend");
    }
    if opts.no_verify {
        args.push("--no-verify");
    }
    if opts.signoff {
        args.push("--signoff");
    }
    git.run_with(&args, Some(message.as_bytes()), &[], 1 << 20).map_err(why)?;
    let out = git.output_limited(&["log", "-1", "--format=%h %s"], 4096).map_err(why)?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// The last commit's full message (for amend).
pub fn last_message(git: &Git) -> Result<String, GitError> {
    let out = git.output_limited(&["log", "-1", "--format=%B"], 1 << 20)?;
    Ok(String::from_utf8_lossy(&out).trim_end().to_string())
}

/// Joins a subject and a description as Git expects.
pub fn message(subject: &str, description: &str) -> String {
    let (s, d) = (subject.trim(), description.trim_end());
    if d.trim().is_empty() { format!("{s}\n") } else { format!("{s}\n\n{d}\n") }
}

#[cfg(test)]
mod tests {

    #[test]
    fn ignored_paths_fold_folders() {
        use crate::git::Git;
        let dir = std::env::temp_dir().join(format!("ffwd-ignored-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        let g = Git { root: dir.clone() };
        g.output(&["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(dir.join("target/debug/a.o"), "x").unwrap();
        std::fs::write(dir.join("target/debug/b.o"), "x").unwrap();
        std::fs::write(dir.join("run.log"), "x").unwrap();
        std::fs::write(dir.join("kept.txt"), "x").unwrap();
        let mut v = super::ignored(&g).unwrap();
        v.sort();
        assert_eq!(v, ["run.log", "target/"], "a whole folder is one entry");
        let e = super::Entry::ignored_entry("run.log");
        assert!(e.ignored() && e.in_unstaged() && !e.untracked());
        assert_eq!(e.letter(super::Side::Unstaged), b'I');
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_porcelain_v2() {
        let out = b"1 .M N... 100644 100644 100644 aaa bbb src/a b.rs\0\
2 R. N... 100644 100644 100644 ccc ddd R100 new.rs\0old.rs\0\
u UU N... 100644 100644 100644 100644 e f g conflict.txt\0\
? new dir/file.txt\0# branch.oid x\0";
        let s = parse_status(out);
        let names: Vec<&str> = s.entries.iter().map(|e| &*e.path).collect();
        assert_eq!(names, vec!["conflict.txt", "new dir/file.txt", "new.rs", "src/a b.rs"]);
        let ren = s.entries.iter().find(|e| &*e.path == "new.rs").unwrap();
        assert_eq!(ren.old_path.as_deref(), Some("old.rs"));
        assert!(ren.in_staged() && !ren.in_unstaged());
        let m = s.entries.iter().find(|e| &*e.path == "src/a b.rs").unwrap();
        assert!(m.in_unstaged() && !m.in_staged());
        let c = s.entries.iter().find(|e| e.conflicted).unwrap();
        assert!(c.in_unstaged() && !c.in_staged());
        assert_eq!(s.side(Side::Unstaged).len(), 3);
        assert_eq!(s.side(Side::Staged).len(), 1);
    }

    #[test]
    fn splits_hunks() {
        let p = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-a\n+b\n c\n@@ -9 +9 @@\n-y\n+z\n\\ No newline at end of file\n";
        let (h, hunks) = split_hunks(p);
        assert!(h.ends_with(b"+++ b/x\n"));
        assert_eq!(hunks.len(), 2);
        assert!(hunks[1].ends_with(b"\\ No newline at end of file\n"));
    }

    #[test]
    fn message_join() {
        assert_eq!(message(" Fix it ", ""), "Fix it\n");
        assert_eq!(message("Fix", "Why:\n- a\n"), "Fix\n\nWhy:\n- a\n");
    }

    fn repo(name: &str) -> (PathBuf, Git) {
        let dir = std::env::temp_dir().join(format!("ffwd-changes-{name}-{}", std::process::id()));
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
        (dir, g)
    }

    fn lines(n: usize, change: Option<(usize, &str)>) -> String {
        (0..n)
            .map(|i| match change {
                Some((at, s)) if at == i => format!("{s}\n"),
                _ => format!("line {i}\n"),
            })
            .collect()
    }

    /// Stage one of two hunks, commit, then unstage and stage files: the
    /// index and history end up exactly as intended.
    #[test]
    fn hunk_staging_and_commit() {
        let (dir, g) = repo("hunks");
        std::fs::write(dir.join("f.txt"), lines(40, None)).unwrap();
        stage(&g, &["f.txt"]).unwrap();
        assert_eq!(
            commit(&g, &message("first", ""), &CommitOptions::default()).unwrap().split_once(' ').unwrap().1,
            "first"
        );
        // Two separate hunks: line 2 and line 30.
        let mut text = lines(40, Some((2, "two")));
        text = text.replace("line 30\n", "thirty\n");
        std::fs::write(dir.join("f.txt"), &text).unwrap();
        std::fs::write(dir.join("new.txt"), "hello\n").unwrap();
        let st = load(&g).unwrap();
        let f = st.entries.iter().find(|e| &*e.path == "f.txt").unwrap().clone();
        let d = diff(&g, &f, Side::Unstaged).unwrap();
        assert_eq!(d.lines.iter().filter(|l| l.kind == crate::diff::LineKind::Hunk).count(), 2);
        // Stage the second hunk only.
        toggle_hunk(&g, &f, Side::Unstaged, 1).unwrap();
        let cached = String::from_utf8(g.output(&["diff", "--cached"]).unwrap()).unwrap();
        assert!(cached.contains("+thirty") && !cached.contains("+two"), "{cached}");
        // The untracked file shows as all added, and stages whole.
        let n = load(&g).unwrap().entries.into_iter().find(|e| &*e.path == "new.txt").unwrap();
        assert!(n.untracked());
        let nd = diff(&g, &n, Side::Unstaged).unwrap();
        assert!(nd.lines.iter().any(|l| l.kind == crate::diff::LineKind::Add && &*l.text == "hello"), "{nd:?}");
        toggle_hunk(&g, &n, Side::Unstaged, 0).unwrap();
        // Unstage the staged hunk again from the staged side, then restage.
        let f2 = load(&g).unwrap().entries.into_iter().find(|e| &*e.path == "f.txt").unwrap();
        toggle_hunk(&g, &f2, Side::Staged, 0).unwrap();
        assert!(
            !String::from_utf8(g.output(&["diff", "--cached", "--", "f.txt"]).unwrap()).unwrap().contains("thirty")
        );
        toggle_hunk(&g, &f2, Side::Unstaged, 1).unwrap();
        let out =
            commit(&g, &message("second", "only the thirty hunk\nand new.txt"), &CommitOptions::default()).unwrap();
        assert!(out.ends_with("second"));
        let body = last_message(&g).unwrap();
        assert_eq!(body, "second\n\nonly the thirty hunk\nand new.txt");
        // What is left: the "two" change, unstaged.
        let left = load(&g).unwrap();
        assert_eq!(left.entries.len(), 1);
        assert!(left.entries[0].in_unstaged() && !left.entries[0].in_staged());
        stage(&g, &["f.txt"]).unwrap();
        assert!(load(&g).unwrap().entries[0].in_staged());
        unstage(&g, &["f.txt"]).unwrap();
        assert!(!load(&g).unwrap().entries[0].in_staged());
        // Amend keeps the message when none is given... and an empty
        // message without amend is refused.
        assert!(commit(&g, "  ", &CommitOptions::default()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ignoring whitespace hides a change in indentation only, keeps a
    /// real change, and leaves the staging patch exact.
    #[test]
    fn whitespace_only_changes_can_be_hidden() {
        let (dir, g) = repo("ws");
        std::fs::write(dir.join("f.txt"), lines(40, None)).unwrap();
        stage(&g, &["f.txt"]).unwrap();
        commit(&g, &message("first", ""), &CommitOptions::default()).unwrap();
        let text = lines(40, Some((2, "    line 2"))).replace(
            "line 30
", "thirty
",
        );
        std::fs::write(dir.join("f.txt"), &text).unwrap();
        let f = load(&g).unwrap().entries.into_iter().find(|e| &*e.path == "f.txt").unwrap();
        let hunks = |d: &FileDiff| d.lines.iter().filter(|l| l.kind == crate::diff::LineKind::Hunk).count();
        assert_eq!(hunks(&diff_with(&g, &f, Side::Unstaged, false).unwrap()), 2);
        let shown = diff_with(&g, &f, Side::Unstaged, true).unwrap();
        assert_eq!(hunks(&shown), 1, "{shown:?}");
        assert!(shown.lines.iter().any(|l| &*l.text == "thirty"));
        assert!(!shown.lines.iter().any(|l| l.text.contains("    line 2")));
        // Staging hunk 0 still stages the indentation change.
        toggle_hunk(&g, &f, Side::Unstaged, 0).unwrap();
        let cached = String::from_utf8(g.output(&["diff", "--cached"]).unwrap()).unwrap();
        assert!(cached.contains("+    line 2") && !cached.contains("thirty"), "{cached}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_selection_rules() {
        let h = b"@@ -1,3 +1,3 @@\n ctx\n-old\n+new\n+more\n";
        // Keep only "+new" (body line 2): -old becomes context, +more goes.
        let f = select_lines(h, &[2], false);
        assert_eq!(f, b"@@ -1,3 +1,3 @@\n ctx\n old\n+new\n".to_vec());
        // Reverse (unstage) keeping "-old": +lines unchosen become context.
        let r = select_lines(h, &[1], true);
        assert_eq!(r, b"@@ -1,3 +1,3 @@\n ctx\n-old\n new\n more\n".to_vec());
    }

    /// Stage two of three added lines, then discard the rest and undo it.
    #[test]
    fn line_staging_discard_and_undo() {
        let (dir, g) = repo("lines");
        std::fs::write(dir.join("f.txt"), "a\nb\n").unwrap();
        stage(&g, &["f.txt"]).unwrap();
        commit(&g, "first", &CommitOptions::default()).unwrap();
        std::fs::write(dir.join("f.txt"), "a\nb\nx1\nx2\nx3\n").unwrap();
        let e = load(&g).unwrap().entries.remove(0);
        // Body lines: " a"? With -U3 the hunk is " a\n b\n+x1\n+x2\n+x3".
        toggle_lines(&g, &e, Side::Unstaged, 0, &[2, 4]).unwrap();
        let staged = String::from_utf8(g.output(&["show", ":f.txt"]).unwrap()).unwrap();
        assert_eq!(staged, "a\nb\nx1\nx3\n");
        // Unstage x3 again from the staged side.
        let e = load(&g).unwrap().entries.remove(0);
        let sd = diff(&g, &e, Side::Staged).unwrap();
        let x3 =
            sd.lines.iter().filter(|l| l.kind != crate::diff::LineKind::Hunk).position(|l| &*l.text == "x3").unwrap();
        toggle_lines(&g, &e, Side::Staged, 0, &[x3]).unwrap();
        assert_eq!(String::from_utf8(g.output(&["show", ":f.txt"]).unwrap()).unwrap(), "a\nb\nx1\n");
        // Discard the unstaged rest (and an untracked file), then undo.
        std::fs::write(dir.join("junk.txt"), "junk\n").unwrap();
        let entries = load(&g).unwrap().entries;
        let b = discard(&g, &entries).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("f.txt")).unwrap(),
            "a\nb\nx1\n",
            "the worktree now equals the index"
        );
        assert!(!dir.join("junk.txt").exists());
        restore_backup(&g, &b).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "a\nb\nx1\nx2\nx3\n");
        assert_eq!(std::fs::read_to_string(dir.join("junk.txt")).unwrap(), "junk\n");
        // Ignore once, never twice.
        ignore(&g, "junk.txt").unwrap();
        ignore(&g, "junk.txt").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), "junk.txt\n");
        assert!(load(&g).unwrap().entries.iter().all(|e| &*e.path != "junk.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Part of an untracked file can be staged.
    #[test]
    fn partial_staging_of_an_untracked_file() {
        let (dir, g) = repo("untracked-lines");
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        stage(&g, &["a.txt"]).unwrap();
        commit(&g, "first", &CommitOptions::default()).unwrap();
        std::fs::write(dir.join("n.txt"), "one\ntwo\n").unwrap();
        let e = load(&g).unwrap().entries.into_iter().find(|e| &*e.path == "n.txt").unwrap();
        toggle_lines(&g, &e, Side::Unstaged, 0, &[1]).unwrap();
        assert_eq!(String::from_utf8(g.output(&["show", ":n.txt"]).unwrap()).unwrap(), "two\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unstage_before_the_first_commit() {
        let (dir, g) = repo("initial");
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        stage(&g, &["a.txt"]).unwrap();
        assert!(load(&g).unwrap().entries[0].in_staged());
        unstage(&g, &["a.txt"]).unwrap();
        let e = &load(&g).unwrap().entries[0];
        assert!(e.untracked() && !e.in_staged());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_hook_reports_its_message() {
        let (dir, g) = repo("hook");
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        stage(&g, &["a.txt"]).unwrap();
        let hooks = dir.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\necho 'lint failed: a.txt' >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(hooks.join("pre-commit"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let e = commit(&g, "x", &CommitOptions::default()).unwrap_err();
        assert!(e.contains("lint failed"), "{e}");
        // Skipping hooks commits.
        commit(&g, "x", &CommitOptions { no_verify: true, ..Default::default() }).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
