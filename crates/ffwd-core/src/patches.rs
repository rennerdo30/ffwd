//! Patches (FF-46): a commit, a stash or the working changes as a patch,
//! and applying a patch with a check first. Applying touches only the
//! working tree (like `git apply`); the 3-way fallback leaves conflict
//! markers for Local Changes, never a half-applied file without them.

use crate::git::Git;
use crate::refops::{OpError, run};

/// The most patch text ffwd reads or writes at once.
pub const MAX_PATCH: usize = 32 << 20;

fn text(git: &Git, args: &[&str]) -> Result<String, String> {
    let out = git.run_with(args, None, &[], MAX_PATCH).map_err(|e| e.to_string())?;
    if out.len() >= MAX_PATCH {
        return Err("The patch is larger than 32 MB".into());
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// A commit as a mail-style patch (`git format-patch`), with its message.
pub fn commit_patch(git: &Git, sha: &str) -> Result<String, String> {
    text(git, &["format-patch", "-1", "--stdout", "--binary", sha])
}

/// A stash's changes (`git stash show -p`).
pub fn stash_patch(git: &Git, name: &str) -> Result<String, String> {
    text(git, &["stash", "show", "-p", "--binary", name])
}

/// The staged or the unstaged changes.
pub fn working_patch(git: &Git, staged: bool) -> Result<String, String> {
    let mut args = vec!["diff", "--binary", "--no-color", "--no-ext-diff"];
    if staged {
        args.push("--cached");
    }
    let p = text(git, &args)?;
    if p.trim().is_empty() {
        return Err(if staged { "Nothing is staged".into() } else { "There are no unstaged changes".into() });
    }
    Ok(p)
}

/// The files a patch touches (from its `diff --git` lines).
pub fn files(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|l| l.strip_prefix("diff --git a/"))
        .filter_map(|l| l.split_once(" b/").map(|x| x.1.to_string()))
        .collect()
}

/// Whether `patch` applies cleanly: `Ok(files)` or the reason it does not.
pub fn check(git: &Git, patch: &str) -> Result<Vec<String>, String> {
    if !patch.contains("diff --git") && !patch.contains("\n--- ") && !patch.starts_with("--- ") {
        return Err("That is not a patch (no diff in it)".into());
    }
    git.run_with(&["apply", "--check", "--whitespace=nowarn", "-"], Some(patch.as_bytes()), &[], 1 << 16)
        .map_err(|e| why(&e.to_string()))?;
    Ok(files(patch))
}

fn why(e: &str) -> String {
    let lines: Vec<&str> = e.lines().map(str::trim).filter(|l| l.starts_with("error:")).collect();
    if lines.is_empty() {
        return e.lines().last().unwrap_or(e).trim().to_string();
    }
    lines.iter().take(4).map(|l| l.trim_start_matches("error: ")).collect::<Vec<_>>().join("; ")
}

/// Applies `patch` to the working tree; `three_way` falls back to a
/// 3-way merge that leaves conflict markers.
pub fn apply(git: &Git, patch: &str, three_way: bool) -> Result<String, OpError> {
    let n = files(patch).len();
    let mut args = vec!["apply", "--whitespace=nowarn"];
    if three_way {
        args.push("--3way");
    }
    args.push("-");
    match git.run_with(&args, Some(patch.as_bytes()), &[], 1 << 16) {
        Ok(_) => Ok(format!("Applied the patch to {n} file{}", if n == 1 { "" } else { "s" })),
        Err(e) => {
            // A 3-way apply that stops with conflicts has still written them.
            let conflicted = run(git, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
            let c = conflicted.lines().filter(|l| !l.is_empty()).count();
            if three_way && c > 0 {
                return Err(OpError::Conflicts {
                    files: c,
                    abort: "git checkout -- <file> (or Discard in Local Changes)",
                });
            }
            Err(OpError::Failed(why(&e.to_string())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str) -> (std::path::PathBuf, Git) {
        let dir = std::env::temp_dir().join(format!("ffwd-patches-{name}-{}", std::process::id()));
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
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        (dir, g)
    }

    #[test]
    fn round_trip_check_and_conflicts() {
        let (dir, g) = repo("all");
        // A commit's patch, undone, then applied again.
        std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(dir.join("b.txt"), "new\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "change two, add b"]).unwrap();
        let p = commit_patch(&g, "HEAD").unwrap();
        assert!(p.contains("Subject: [PATCH] change two, add b"));
        assert_eq!(files(&p), ["a.txt", "b.txt"]);
        g.output(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        assert_eq!(check(&g, &p).unwrap(), ["a.txt", "b.txt"]);
        apply(&g, &p, false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\nTWO\nthree\n");
        assert!(dir.join("b.txt").exists());
        // The working changes as a patch; nothing staged is an error.
        assert!(working_patch(&g, true).is_err());
        let w = working_patch(&g, false).unwrap();
        assert!(w.contains("+TWO"));
        // A patch that no longer fits: the check says why; 3-way leaves markers.
        g.output(&["checkout", "--", "a.txt"]).unwrap();
        std::fs::remove_file(dir.join("b.txt")).unwrap();
        std::fs::write(dir.join("a.txt"), "one\nzwei\nthree\n").unwrap();
        g.output(&["commit", "-q", "-am", "theirs"]).unwrap();
        let only_a = w.split("diff --git a/b.txt").next().unwrap().to_string();
        assert!(check(&g, &only_a).is_err());
        assert!(matches!(apply(&g, &only_a, true), Err(OpError::Conflicts { files: 1, .. })));
        let a = std::fs::read_to_string(dir.join("a.txt")).unwrap();
        assert!(a.contains("<<<<<<<") && a.contains("TWO") && a.contains("zwei"), "{a}");
        assert!(check(&g, "hello").unwrap_err().contains("not a patch"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stash_patches() {
        let (dir, g) = repo("stash");
        std::fs::write(dir.join("a.txt"), "one\nstashed\nthree\n").unwrap();
        g.output(&["stash", "-q"]).unwrap();
        let p = stash_patch(&g, "stash@{0}").unwrap();
        assert!(p.contains("+stashed"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
