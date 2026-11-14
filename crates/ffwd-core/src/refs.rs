//! Branches, remotes, tags, stashes, worktrees and the working-tree summary.

use crate::git::{Git, GitError, text};
use crate::log::Oid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefKind {
    Head,
    Local,
    Remote,
    Tag,
    Stash,
}

#[derive(Clone, Debug)]
pub struct Ref {
    pub kind: RefKind,
    /// Short name: `main`, `origin/main`, `v1.0`.
    pub name: Box<str>,
    pub target: Oid,
    pub upstream: Option<Box<str>>,
    /// The upstream branch no longer exists on the remote (git's "gone").
    pub upstream_gone: bool,
    pub ahead: u32,
    pub behind: u32,
    pub is_head: bool,
    /// The target commit's committer time (Unix seconds; 0 when unknown).
    pub time: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Refs {
    pub head_branch: Option<Box<str>>,
    pub head: Option<Oid>,
    pub refs: Vec<Ref>,
    pub stashes: Vec<(Box<str>, Box<str>)>,
    pub worktrees: Vec<Box<str>>,
    /// Each worktree's branch (same order), or "detached".
    pub worktree_branches: Vec<Box<str>>,
    /// The configured remotes with their URLs.
    pub remotes: Vec<crate::remotes::Remote>,
    /// The submodules (only read when `.gitmodules` exists).
    pub submodules: Vec<crate::submodules::Submodule>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StatusSummary {
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub conflicted: u32,
}

impl StatusSummary {
    pub fn total(&self) -> u32 {
        self.staged + self.unstaged + self.untracked + self.conflicted
    }
}

pub fn load(git: &Git) -> Result<Refs, GitError> {
    let fmt = "--format=%(refname)%1f%(objectname)%1f%(*objectname)%1f%(HEAD)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(committerdate:unix)%1f%(*committerdate:unix)";
    let out = git.output(&["for-each-ref", fmt, "refs/heads", "refs/remotes", "refs/tags"])?;
    let mut refs = Refs::default();
    for line in out.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        let f: Vec<&[u8]> = line.split(|&b| b == 0x1f).collect();
        if f.len() < 6 {
            continue;
        }
        let full = String::from_utf8_lossy(f[0]);
        // Annotated tags point at the tag object; %(*objectname) is the commit.
        let Some(target) = Oid::from_hex(if f[2].is_empty() { f[1] } else { f[2] }) else { continue };
        let (kind, name) = if let Some(n) = full.strip_prefix("refs/heads/") {
            (RefKind::Local, n)
        } else if let Some(n) = full.strip_prefix("refs/remotes/") {
            if n.ends_with("/HEAD") {
                continue;
            }
            (RefKind::Remote, n)
        } else if let Some(n) = full.strip_prefix("refs/tags/") {
            (RefKind::Tag, n)
        } else {
            continue;
        };
        let is_head = f[3] == b"*";
        // Annotated tags carry the commit's date in the dereferenced field.
        let num = |b: Option<&&[u8]>| b.and_then(|b| std::str::from_utf8(b).ok()?.trim().parse::<i64>().ok());
        let time = num(f.get(7)).or_else(|| num(f.get(6))).unwrap_or(0);
        let (ahead, behind) = parse_track(&String::from_utf8_lossy(f[5]));
        if is_head {
            refs.head_branch = Some(name.into());
        }
        refs.refs.push(Ref {
            kind,
            name: name.into(),
            target,
            upstream: (!f[4].is_empty()).then(|| text(f[4])),
            upstream_gone: f[5].trim_ascii() == b"gone",
            ahead,
            behind,
            is_head,
            time,
        });
    }
    if let Ok(h) = git.output(&["rev-parse", "--verify", "-q", "HEAD"]) {
        refs.head = Oid::from_hex(h.trim_ascii());
    }
    // The reflog message (%gs), which a rename changes, not the commit subject.
    // Each stash is also a ref of kind Stash on its commit, for the chips
    // of stash rows in the history (FF-33).
    if let Ok(s) = git.output_limited(&["stash", "list", "--format=%gd%x1f%H%x1f%gs"], 1 << 20) {
        for line in s.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            let mut it = line.splitn(3, |&b| b == 0x1f);
            if let (Some(a), Some(h), Some(b)) = (it.next(), it.next(), it.next()) {
                refs.stashes.push((text(a), text(b)));
                if let Some(target) = Oid::from_hex(h.trim_ascii()) {
                    refs.refs.push(Ref {
                        kind: RefKind::Stash,
                        name: text(a),
                        target,
                        upstream: None,
                        upstream_gone: false,
                        ahead: 0,
                        behind: 0,
                        is_head: false,
                        time: 0,
                    });
                }
            }
        }
    }
    refs.remotes = crate::remotes::list(git);
    if git.root.join(".gitmodules").exists() {
        refs.submodules = crate::submodules::list(git);
    }
    if let Ok(w) = git.output_limited(&["worktree", "list", "--porcelain"], 1 << 20) {
        for line in w.split(|&b| b == b'\n') {
            if let Some(p) = line.strip_prefix(b"worktree ") {
                refs.worktrees.push(text(p));
                refs.worktree_branches.push("".into());
            } else if let Some(b) = line.strip_prefix(b"branch refs/heads/") {
                if let Some(last) = refs.worktree_branches.last_mut() {
                    *last = text(b);
                }
            } else if line == b"detached"
                && let Some(last) = refs.worktree_branches.last_mut()
            {
                *last = "detached".into();
            }
        }
    }
    Ok(refs)
}

fn parse_track(s: &str) -> (u32, u32) {
    let (mut a, mut b) = (0, 0);
    for part in s.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            a = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            b = n.parse().unwrap_or(0);
        }
    }
    (a, b)
}

/// Working-tree summary from `git status --porcelain=v2`. Bounded: a
/// working tree with millions of untracked files stops counting at the cap.
pub fn status(git: &Git) -> Result<StatusSummary, GitError> {
    let out = git.output_limited(&["status", "--porcelain=v2", "-z", "--untracked-files=normal"], 32 << 20)?;
    let mut s = StatusSummary::default();
    let mut it = out.split(|&b| b == 0).peekable();
    while let Some(e) = it.next() {
        match e.first() {
            Some(b'1') | Some(b'2') => {
                let xy = e.get(2..4).unwrap_or(b"..");
                if xy[0] != b'.' {
                    s.staged += 1;
                }
                if xy[1] != b'.' {
                    s.unstaged += 1;
                }
                if e[0] == b'2' {
                    it.next(); // rename source path
                }
            }
            Some(b'u') => s.conflicted += 1,
            Some(b'?') => s.untracked += 1,
            _ => {}
        }
    }
    Ok(s)
}

/// Checks a branch or tag name against git's rules (`git
/// check-ref-format --branch`), saying what is wrong in words.
pub fn check_ref_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Type a name first.".into());
    }
    if name == "@" {
        return Err("\"@\" alone is not a valid name.".into());
    }
    if name.starts_with('-') {
        return Err("A name can't start with \"-\".".into());
    }
    if let Some(c) = name.chars().find(|c| c.is_control() || " ~^:?*[\\".contains(*c)) {
        let shown = if c == ' ' { "a space".to_string() } else { format!("\"{c}\"") };
        return Err(format!("A name can't contain {shown}."));
    }
    if name.contains("..") {
        return Err("A name can't contain \"..\".".into());
    }
    if name.contains("@{") {
        return Err("A name can't contain \"@{\".".into());
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Err("Folders in a name need text on both sides of \"/\".".into());
    }
    if name.ends_with('.') {
        return Err("A name can't end with \".\".".into());
    }
    for part in name.split('/') {
        if part.starts_with('.') {
            return Err("No part of a name can start with \".\".".into());
        }
        if part.ends_with(".lock") {
            return Err("No part of a name can end with \".lock\".".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn ref_names_follow_git_rules() {
        use super::check_ref_name;
        for ok in ["main", "feat/login", "release-1.2", "fix_bug", "a/b/c", "über"] {
            assert!(check_ref_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "@",
            "-x",
            "a b",
            "a~b",
            "a^b",
            "a:b",
            "a?b",
            "a*b",
            "a[b",
            "a\\b",
            "a..b",
            "a@{b",
            "/a",
            "a/",
            "a//b",
            "a.",
            ".a",
            "a/.b",
            "a.lock",
            "a/b.lock",
            "tab\there",
        ] {
            assert!(check_ref_name(bad).is_err(), "{bad:?} should be refused");
        }
        assert_eq!(check_ref_name("my branch").unwrap_err(), "A name can't contain a space.");
    }

    #[test]
    fn track_parsing() {
        assert_eq!(super::parse_track("ahead 3, behind 12"), (3, 12));
        assert_eq!(super::parse_track("gone"), (0, 0));
        assert_eq!(super::parse_track(""), (0, 0));
    }
}
