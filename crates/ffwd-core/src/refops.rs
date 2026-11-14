//! Branches, tags and history operations: checkout, create, rename,
//! delete, merge, rebase, cherry-pick, revert and reset. Each runs the
//! `git` CLI (hooks, signing and config apply) on a worker thread and
//! returns one plain sentence. Operations that drop commits from a branch
//! keep a way back and name it (AGENTS.md rule 8).

use crate::git::{Git, GitError};

/// How a failed operation should be shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpError {
    /// Git refused or failed; the message says why.
    Failed(String),
    /// The branch has commits nowhere else; deleting needs force.
    NotMerged { branch: String, tip: String },
    /// Stopped with conflicts; the repository is mid-operation.
    Conflicts { files: usize, abort: &'static str },
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::Failed(m) => write!(f, "{m}"),
            OpError::NotMerged { branch, .. } => write!(f, "{branch} has commits that are on no other branch"),
            OpError::Conflicts { files, abort } => write!(
                f,
                "stopped with conflicts in {files} file{}: resolve them and continue, or run {abort}",
                if *files == 1 { "" } else { "s" }
            ),
        }
    }
}

type R = Result<String, OpError>;

fn why(e: GitError) -> OpError {
    OpError::Failed(match e {
        GitError::Failed { stderr, .. } if !stderr.trim().is_empty() => {
            let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            lines
                .iter()
                .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
                .or(lines.first())
                .copied()
                .unwrap_or("git reported an error")
                .trim_start_matches("fatal: ")
                .trim_start_matches("error: ")
                .to_string()
        }
        e => e.to_string(),
    })
}

pub(crate) fn run(git: &Git, args: &[&str]) -> Result<String, OpError> {
    git.run_with(args, None, &[], 1 << 20).map(|b| String::from_utf8_lossy(&b).trim().to_string()).map_err(why)
}

fn short(git: &Git, rev: &str) -> String {
    run(git, &["rev-parse", "--short", rev]).unwrap_or_else(|_| rev.chars().take(10).collect())
}

/// A valid branch name, or why not (`git check-ref-format --branch`).
pub fn check_branch_name(git: &Git, name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("the name is empty".into());
    }
    run(git, &["check-ref-format", "--branch", n])
        .map(|_| ())
        .map_err(|_| format!("\"{n}\" is not a valid branch name"))
}

pub fn check_tag_name(git: &Git, name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("the name is empty".into());
    }
    run(git, &["check-ref-format", &format!("refs/tags/{n}")])
        .map(|_| ())
        .map_err(|_| format!("\"{n}\" is not a valid tag name"))
}

fn exists(git: &Git, full_ref: &str) -> bool {
    run(git, &["rev-parse", "--verify", "--quiet", full_ref]).is_ok()
}

/// Checks out a local branch; a remote branch (`origin/x`) gets a local
/// tracking branch (or switches to it when it exists); anything else
/// (a tag, a SHA) is checked out detached. Git's progress ("Updating
/// files") reaches a hook set with `git::with_progress` (FF-48).
pub fn checkout(git: &Git, target: &str) -> R {
    if exists(git, &format!("refs/heads/{target}")) {
        run(git, &["switch", "--progress", target])?;
        return Ok(format!("Checked out {target}"));
    }
    if let Some((remote, name)) = target.split_once('/')
        && exists(git, &format!("refs/remotes/{target}"))
    {
        if exists(git, &format!("refs/heads/{name}")) {
            run(git, &["switch", "--progress", name])?;
            return Ok(format!("Checked out {name} (it already tracks a branch; pull to update it)"));
        }
        run(git, &["switch", "--progress", "--track", target])?;
        return Ok(format!("Checked out {name}, tracking {remote}/{name}"));
    }
    run(git, &["switch", "--progress", "--detach", target])?;
    Ok(format!("Checked out {} (detached HEAD: create a branch before committing)", short(git, "HEAD")))
}

pub fn create_branch(git: &Git, name: &str, at: &str, switch: bool) -> R {
    let name = name.trim();
    check_branch_name(git, name).map_err(OpError::Failed)?;
    if exists(git, &format!("refs/heads/{name}")) {
        return Err(OpError::Failed(format!("a branch named {name} already exists")));
    }
    if switch {
        run(git, &["switch", "-c", name, at])?;
        Ok(format!("Created and checked out {name}"))
    } else {
        run(git, &["branch", name, at])?;
        Ok(format!("Created {name} at {}", short(git, at)))
    }
}

pub fn rename_branch(git: &Git, old: &str, new: &str) -> R {
    let new = new.trim();
    check_branch_name(git, new).map_err(OpError::Failed)?;
    run(git, &["branch", "-m", old, new])?;
    Ok(format!("Renamed {old} to {new}"))
}

/// Makes `branch` track `upstream` (a remote branch such as `origin/x`).
pub fn set_upstream(git: &Git, branch: &str, upstream: &str) -> R {
    let to = format!("--set-upstream-to={upstream}");
    run(git, &["branch", &to, branch])?;
    Ok(format!("{branch} now tracks {upstream}"))
}

/// Stops `branch` tracking its upstream (for one that is gone on the
/// remote). Nothing else changes; setting an upstream again is a push.
pub fn unset_upstream(git: &Git, branch: &str) -> R {
    run(git, &["branch", "--unset-upstream", branch])?;
    Ok(format!("{branch} no longer tracks an upstream"))
}

/// Deletes a local branch. Without `force`, a branch with commits on no
/// other branch is refused with `NotMerged`. The message names the
/// command that brings the branch back.
pub fn delete_branch(git: &Git, name: &str, force: bool) -> R {
    let tip = run(git, &["rev-parse", "--verify", &format!("refs/heads/{name}")])?;
    let flag = if force { "-D" } else { "-d" };
    match git.run_with(&["branch", flag, name], None, &[], 1 << 16) {
        Ok(_) => Ok(format!("Deleted {name}. To restore it: git branch {name} {}", &tip[..tip.len().min(12)])),
        Err(GitError::Failed { stderr, .. }) if !force && stderr.contains("not fully merged") => {
            Err(OpError::NotMerged { branch: name.into(), tip })
        }
        Err(e) => Err(why(e)),
    }
}

pub fn create_tag(git: &Git, name: &str, at: &str, message: Option<&str>) -> R {
    let name = name.trim();
    check_tag_name(git, name).map_err(OpError::Failed)?;
    if exists(git, &format!("refs/tags/{name}")) {
        return Err(OpError::Failed(format!("a tag named {name} already exists")));
    }
    match message.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => run(git, &["tag", "-a", name, "-m", m, at])?,
        None => run(git, &["tag", name, at])?,
    };
    Ok(format!("Tagged {} as {name}", short(git, at)))
}

pub fn delete_tag(git: &Git, name: &str) -> R {
    let tip = run(git, &["rev-parse", "--verify", &format!("refs/tags/{name}")])?;
    run(git, &["tag", "-d", name])?;
    Ok(format!("Deleted tag {name}. To restore it: git tag {name} {}", &tip[..tip.len().min(12)]))
}

/// Pushes tag `name` to `remote`.
pub fn push_tag(git: &Git, remote: &str, name: &str) -> R {
    run(git, &["push", remote, &format!("refs/tags/{name}:refs/tags/{name}")])?;
    Ok(format!("Pushed tag {name} to {remote}"))
}

/// Deletes tag `name` on `remote` (the local tag stays).
pub fn delete_remote_tag(git: &Git, remote: &str, name: &str) -> R {
    run(git, &["push", remote, &format!(":refs/tags/{name}")])?;
    Ok(format!("Deleted tag {name} on {remote}; the local tag stays, so git push {remote} {name} brings it back"))
}

/// Pushes local branch `name` to `remote` and makes it its upstream.
pub fn push_branch(git: &Git, remote: &str, name: &str) -> R {
    run(git, &["push", "--set-upstream", remote, &format!("refs/heads/{name}:refs/heads/{name}")])?;
    Ok(format!("Pushed {name} to {remote}"))
}

/// Deletes `branch` (without the remote prefix) on `remote`. Names the
/// command that restores it from the remote-tracking ref's last value.
pub fn delete_remote_branch(git: &Git, remote: &str, branch: &str) -> R {
    let tip = run(git, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{remote}/{branch}")]).ok();
    run(git, &["push", remote, "--delete", branch])?;
    Ok(match tip {
        Some(t) => format!(
            "Deleted {branch} on {remote}. To restore it: git push {remote} {}:refs/heads/{branch}",
            &t[..t.len().min(12)]
        ),
        None => format!("Deleted {branch} on {remote}"),
    })
}

/// "2 branches and 3 tags".
pub fn count_refs(branches: usize, tags: usize) -> String {
    let n = |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
    match (branches, tags) {
        (0, t) => n(t, "tag", "tags"),
        (b, 0) => n(b, "branch", "branches"),
        (b, t) => format!("{} and {}", n(b, "branch", "branches"), n(t, "tag", "tags")),
    }
}

fn tip12(t: &str) -> &str {
    &t[..t.len().min(12)]
}

/// Pushes several branches (each gets its upstream) and tags to `remote`
/// in one `git push` (FF-36).
pub fn push_many(git: &Git, remote: &str, branches: &[String], tags: &[String]) -> R {
    if branches.is_empty() && tags.is_empty() {
        return Err(OpError::Failed("nothing selected to push".into()));
    }
    let mut args = vec!["push".to_string()];
    if !branches.is_empty() {
        args.push("--set-upstream".into());
    }
    args.push(remote.into());
    args.extend(branches.iter().map(|b| format!("refs/heads/{b}:refs/heads/{b}")));
    args.extend(tags.iter().map(|t| format!("refs/tags/{t}:refs/tags/{t}")));
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    run(git, &a)?;
    Ok(format!("Pushed {} to {remote}", count_refs(branches.len(), tags.len())))
}

/// Deletes several local branches and tags (FF-36). Without `force`, a
/// branch with commits on no other branch is kept and named. The message
/// names the commands that restore what was deleted.
pub fn delete_many(git: &Git, branches: &[String], tags: &[String], force: bool) -> R {
    let mut restore = Vec::new();
    let mut kept = Vec::new();
    let mut failed = Vec::new();
    let mut done = (0, 0);
    for b in branches {
        let tip = match run(git, &["rev-parse", "--verify", &format!("refs/heads/{b}")]) {
            Ok(t) => t,
            Err(e) => {
                failed.push(format!("{b}: {e}"));
                continue;
            }
        };
        match delete_branch(git, b, force) {
            Ok(_) => {
                done.0 += 1;
                restore.push(format!("git branch {b} {}", tip12(&tip)));
            }
            Err(OpError::NotMerged { branch, .. }) => kept.push(branch),
            Err(e) => failed.push(format!("{b}: {e}")),
        }
    }
    let mut tag_tips = Vec::new();
    for t in tags {
        match run(git, &["rev-parse", "--verify", &format!("refs/tags/{t}")]) {
            Ok(tip) => tag_tips.push((t.as_str(), tip)),
            Err(e) => failed.push(format!("{t}: {e}")),
        }
    }
    if !tag_tips.is_empty() {
        let mut args = vec!["tag", "-d"];
        args.extend(tag_tips.iter().map(|(t, _)| *t));
        match run(git, &args) {
            Ok(_) => {
                done.1 = tag_tips.len();
                restore.extend(tag_tips.iter().map(|(t, tip)| format!("git tag {t} {}", tip12(tip))));
            }
            Err(e) => failed.push(e.to_string()),
        }
    }
    if done == (0, 0) {
        return Err(OpError::Failed(if kept.is_empty() {
            failed.join("; ")
        } else {
            format!("nothing deleted: {} have commits on no other branch", kept.join(", "))
        }));
    }
    let mut m = format!("Deleted {}.", count_refs(done.0, done.1));
    if !kept.is_empty() {
        m += &format!(" Kept {}: commits on no other branch.", kept.join(", "));
    }
    if !failed.is_empty() {
        m += &format!(" Not deleted: {}.", failed.join("; "));
    }
    m += &format!(" To restore: {}", restore.join("; "));
    Ok(m)
}

/// Deletes several branches and tags on `remote` in one `git push` (FF-36);
/// local tags stay. Names the commands that push the branches back from
/// their remote-tracking refs.
pub fn delete_remote_many(git: &Git, remote: &str, branches: &[String], tags: &[String]) -> R {
    if branches.is_empty() && tags.is_empty() {
        return Err(OpError::Failed("nothing selected to delete".into()));
    }
    let tips: Vec<(&String, Option<String>)> = branches
        .iter()
        .map(|b| (b, run(git, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{remote}/{b}")]).ok()))
        .collect();
    let mut args = vec!["push".to_string(), remote.into(), "--delete".into()];
    args.extend(branches.iter().map(|b| format!("refs/heads/{b}")));
    args.extend(tags.iter().map(|t| format!("refs/tags/{t}")));
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    run(git, &a)?;
    let mut m = format!("Deleted {} on {remote}.", count_refs(branches.len(), tags.len()));
    let back: Vec<String> = tips
        .iter()
        .filter_map(|(b, t)| t.as_ref().map(|t| format!("git push {remote} {}:refs/heads/{b}", tip12(t))))
        .collect();
    if !back.is_empty() {
        m += &format!(" To restore: {}", back.join("; "));
    }
    if !tags.is_empty() {
        m += &format!(" The local tags stay; git push {remote} <tag> brings one back.");
    }
    Ok(m)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMode {
    /// Fast-forward when possible, else a merge commit (Git's default).
    Default,
    FastForwardOnly,
    NoFastForward,
    /// Stage the combined changes without committing.
    Squash,
}

pub(crate) fn conflicts(git: &Git, abort: &'static str) -> Option<OpError> {
    let out = run(git, &["diff", "--name-only", "--diff-filter=U"]).ok()?;
    let n = out.lines().filter(|l| !l.is_empty()).count();
    (n > 0).then_some(OpError::Conflicts { files: n, abort })
}

/// Merges `what` into the current branch.
pub fn merge(git: &Git, what: &str, mode: MergeMode) -> R {
    let mut args = vec!["merge", "--no-edit"];
    match mode {
        MergeMode::Default => {}
        MergeMode::FastForwardOnly => args.push("--ff-only"),
        MergeMode::NoFastForward => args.push("--no-ff"),
        MergeMode::Squash => args.push("--squash"),
    }
    args.push(what);
    match run(git, &args) {
        Ok(out) if out.contains("Already up to date") => Ok("Already up to date".into()),
        Ok(_) if mode == MergeMode::Squash => Ok(format!("Squashed {what}: the changes are staged; commit them")),
        Ok(out) if out.contains("Fast-forward") => Ok(format!("Fast-forwarded to {what}")),
        Ok(_) => Ok(format!("Merged {what}")),
        Err(e) => Err(conflicts(git, "git merge --abort").unwrap_or(e)),
    }
}

/// Rebases the current branch onto `onto`. A backup ref keeps the old tip.
pub fn rebase(git: &Git, onto: &str) -> R {
    let backup = backup_head(git, "rebase")?;
    match run(git, &["rebase", onto]) {
        Ok(_) => Ok(format!("Rebased onto {onto}. The old tip is kept as {backup}")),
        Err(e) => Err(conflicts(git, "git rebase --abort").unwrap_or(e)),
    }
}

pub fn cherry_pick(git: &Git, commit: &str) -> R {
    match run(git, &["cherry-pick", commit]) {
        Ok(_) => Ok(format!("Cherry-picked {}", short(git, commit))),
        Err(e) => Err(conflicts(git, "git cherry-pick --abort").unwrap_or(e)),
    }
}

pub fn revert(git: &Git, commit: &str) -> R {
    match run(git, &["revert", "--no-edit", commit]) {
        Ok(_) => Ok(format!("Reverted {}", short(git, commit))),
        Err(e) => Err(conflicts(git, "git revert --abort").unwrap_or(e)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    /// Keep the index and the working tree.
    Soft,
    /// Keep the working tree, reset the index.
    Mixed,
    /// Throw away staged and unstaged changes too.
    Hard,
}

/// Saves HEAD as `refs/ffwd/backup/<what>-<time>` and returns that name.
fn backup_head(git: &Git, what: &str) -> Result<String, OpError> {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let name = format!("refs/ffwd/backup/{what}-{secs}");
    run(git, &["update-ref", &name, "HEAD"])?;
    Ok(name)
}

/// Resets the current branch to `to`. HEAD is saved first.
pub fn reset(git: &Git, to: &str, mode: ResetMode) -> R {
    let backup = backup_head(git, "reset")?;
    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
    };
    run(git, &["reset", "-q", flag, to])?;
    Ok(format!("Reset to {} ({}). Undo with: git reset --hard {backup}", short(git, to), &flag[2..]))
}

/// An operation that stopped in the middle (merge, rebase, cherry-pick,
/// revert), from the files Git leaves in its folder.
pub fn in_progress(git: &Git) -> Option<&'static str> {
    let dir = run(git, &["rev-parse", "--path-format=absolute", "--git-dir"]).ok()?;
    let d = std::path::Path::new(&dir);
    if d.join("rebase-merge").exists() || d.join("rebase-apply").exists() {
        Some("rebase")
    } else if d.join("MERGE_HEAD").exists() {
        Some("merge")
    } else if d.join("CHERRY_PICK_HEAD").exists() {
        Some("cherry-pick")
    } else if d.join("REVERT_HEAD").exists() {
        Some("revert")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo(name: &str) -> (PathBuf, Git) {
        let dir = std::env::temp_dir().join(format!("ffwd-refops-{name}-{}", std::process::id()));
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

    fn commit(dir: &std::path::Path, g: &Git, file: &str, text: &str, msg: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", msg]).unwrap();
    }

    fn head(g: &Git) -> String {
        String::from_utf8(g.output(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap()).unwrap().trim().to_string()
    }

    #[test]
    fn branches_and_tags() {
        let (dir, g) = repo("branches");
        commit(&dir, &g, "a.txt", "a\n", "first");
        assert!(create_branch(&g, "bad name", "HEAD", false).is_err());
        create_branch(&g, "feat/x", "HEAD", true).unwrap();
        assert_eq!(head(&g), "feat/x");
        assert!(create_branch(&g, "feat/x", "HEAD", false).unwrap_err().to_string().contains("already exists"));
        commit(&dir, &g, "b.txt", "b\n", "on feat");
        checkout(&g, "main").unwrap();
        // feat/x has a commit main doesn't: deleting asks for force.
        match delete_branch(&g, "feat/x", false) {
            Err(OpError::NotMerged { branch, tip }) => {
                assert_eq!(branch, "feat/x");
                assert_eq!(tip.len(), 40);
            }
            other => panic!("{other:?}"),
        }
        rename_branch(&g, "feat/x", "feat/y").unwrap();
        let msg = delete_branch(&g, "feat/y", true).unwrap();
        assert!(msg.contains("git branch feat/y "), "{msg}");
        create_tag(&g, "v1", "HEAD", Some("first release")).unwrap();
        let kind = String::from_utf8(g.output(&["cat-file", "-t", "v1"]).unwrap()).unwrap();
        assert_eq!(kind.trim(), "tag", "a message makes an annotated tag");
        assert!(delete_tag(&g, "v1").unwrap().contains("git tag v1 "));
        // A SHA checks out detached.
        let first = String::from_utf8(g.output(&["rev-parse", "HEAD"]).unwrap()).unwrap();
        assert!(checkout(&g, first.trim()).unwrap().contains("detached"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_branch_checkout_tracks() {
        let (up, ug) = repo("upstream");
        commit(&up, &ug, "a.txt", "a\n", "first");
        ug.output(&["branch", "topic"]).unwrap();
        let dir = std::env::temp_dir().join(format!("ffwd-refops-clone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Git { root: std::env::temp_dir() }
            .output(&["clone", "-q", &up.to_string_lossy(), &dir.to_string_lossy()])
            .unwrap();
        let g = Git { root: dir.clone() };
        let m = checkout(&g, "origin/topic").unwrap();
        assert!(m.contains("tracking origin/topic"), "{m}");
        assert_eq!(head(&g), "topic");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&up);
    }

    #[test]
    fn merge_rebase_pick_revert_reset() {
        let (dir, g) = repo("history");
        commit(&dir, &g, "a.txt", "a\n", "first");
        create_branch(&g, "topic", "HEAD", true).unwrap();
        commit(&dir, &g, "t.txt", "t\n", "topic work");
        checkout(&g, "main").unwrap();
        assert!(merge(&g, "topic", MergeMode::FastForwardOnly).unwrap().starts_with("Fast-forwarded"));
        commit(&dir, &g, "m.txt", "m\n", "main work");
        checkout(&g, "topic").unwrap();
        commit(&dir, &g, "t2.txt", "t2\n", "more topic");
        let r = rebase(&g, "main").unwrap();
        assert!(r.contains("refs/ffwd/backup/rebase-"), "{r}");
        checkout(&g, "main").unwrap();
        assert!(merge(&g, "topic", MergeMode::NoFastForward).unwrap().starts_with("Merged"));
        // Cherry-pick and revert.
        create_branch(&g, "side", "HEAD~1", true).unwrap();
        commit(&dir, &g, "s.txt", "s\n", "side work");
        let side = String::from_utf8(g.output(&["rev-parse", "HEAD"]).unwrap()).unwrap().trim().to_string();
        checkout(&g, "main").unwrap();
        cherry_pick(&g, &side).unwrap();
        assert!(dir.join("s.txt").exists());
        revert(&g, "HEAD").unwrap();
        assert!(!dir.join("s.txt").exists());
        // Hard reset keeps a backup that brings everything back.
        let before = String::from_utf8(g.output(&["rev-parse", "HEAD"]).unwrap()).unwrap().trim().to_string();
        let msg = reset(&g, "HEAD~2", ResetMode::Hard).unwrap();
        let backup = msg.rsplit(' ').next().unwrap().to_string();
        assert!(backup.starts_with("refs/ffwd/backup/reset-"), "{msg}");
        let saved = String::from_utf8(g.output(&["rev-parse", &backup]).unwrap()).unwrap().trim().to_string();
        assert_eq!(saved, before);
        assert_eq!(in_progress(&g), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conflicts_are_reported_with_the_way_out() {
        let (dir, g) = repo("conflict");
        commit(&dir, &g, "a.txt", "base\n", "first");
        create_branch(&g, "other", "HEAD", true).unwrap();
        commit(&dir, &g, "a.txt", "other\n", "other change");
        checkout(&g, "main").unwrap();
        commit(&dir, &g, "a.txt", "main\n", "main change");
        match merge(&g, "other", MergeMode::Default) {
            Err(OpError::Conflicts { files: 1, abort }) => assert_eq!(abort, "git merge --abort"),
            other => panic!("{other:?}"),
        }
        assert_eq!(in_progress(&g), Some("merge"));
        g.output(&["merge", "--abort"]).unwrap();
        assert_eq!(in_progress(&g), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_tags_and_branches() {
        let (dir, g) = repo("remote-refs");
        commit(&dir, &g, "a.txt", "a", "first");
        let hub = dir.with_extension("hub.git");
        let _ = std::fs::remove_dir_all(&hub);
        Git { root: dir.parent().unwrap().to_path_buf() }
            .output(&["init", "-q", "--bare", &hub.to_string_lossy()])
            .unwrap();
        g.output(&["remote", "add", "origin", &hub.to_string_lossy()]).unwrap();
        let on_hub = |r: &str| Git { root: hub.clone() }.output(&["rev-parse", "--verify", "--quiet", r]).is_ok();
        create_tag(&g, "v1", "HEAD", None).unwrap();
        push_tag(&g, "origin", "v1").unwrap();
        assert!(on_hub("refs/tags/v1"));
        create_branch(&g, "topic", "HEAD", false).unwrap();
        push_branch(&g, "origin", "topic").unwrap();
        assert!(on_hub("refs/heads/topic"));
        let up = String::from_utf8(g.output(&["rev-parse", "--abbrev-ref", "topic@{u}"]).unwrap()).unwrap();
        assert_eq!(up.trim(), "origin/topic");
        let msg = delete_remote_branch(&g, "origin", "topic").unwrap();
        assert!(!on_hub("refs/heads/topic"));
        assert!(msg.contains("git push origin "), "{msg}");
        delete_remote_tag(&g, "origin", "v1").unwrap();
        assert!(!on_hub("refs/tags/v1"));
        let _ = std::fs::remove_dir_all(&hub);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Several at once (FF-36): one push, one remote delete, local deletes
    /// that keep unmerged branches and name the restore commands.
    #[test]
    fn several_refs_at_once() {
        let (dir, g) = repo("many");
        commit(&dir, &g, "a.txt", "a", "first");
        let hub = dir.with_extension("hub.git");
        let _ = std::fs::remove_dir_all(&hub);
        Git { root: dir.parent().unwrap().to_path_buf() }
            .output(&["init", "-q", "--bare", &hub.to_string_lossy()])
            .unwrap();
        g.output(&["remote", "add", "origin", &hub.to_string_lossy()]).unwrap();
        let on_hub = |r: &str| Git { root: hub.clone() }.output(&["rev-parse", "--verify", "--quiet", r]).is_ok();
        let here = |r: &str| g.output(&["rev-parse", "--verify", "--quiet", r]).is_ok();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        for b in ["a", "b"] {
            create_branch(&g, b, "HEAD", false).unwrap();
        }
        // An unmerged branch.
        create_branch(&g, "wip", "HEAD", true).unwrap();
        commit(&dir, &g, "w.txt", "w", "wip");
        g.output(&["checkout", "-q", "main"]).unwrap();
        for t in ["v1", "v2"] {
            create_tag(&g, t, "HEAD", None).unwrap();
        }
        let m = push_many(&g, "origin", &s(&["a", "b"]), &s(&["v1", "v2"])).unwrap();
        assert_eq!(m, "Pushed 2 branches and 2 tags to origin");
        assert!(["refs/heads/a", "refs/heads/b", "refs/tags/v1", "refs/tags/v2"].iter().all(|r| on_hub(r)));
        let m = delete_remote_many(&g, "origin", &s(&["a"]), &s(&["v1", "v2"])).unwrap();
        assert!(!on_hub("refs/heads/a") && !on_hub("refs/tags/v1") && on_hub("refs/heads/b"), "{m}");
        assert!(m.contains("git push origin ") && m.contains(":refs/heads/a"), "{m}");
        let m = delete_many(&g, &s(&["a", "wip"]), &s(&["v1"]), false).unwrap();
        assert!(!here("refs/heads/a") && here("refs/heads/wip") && !here("refs/tags/v1"), "{m}");
        assert!(
            m.starts_with("Deleted 1 branch and 1 tag.") && m.contains("Kept wip") && m.contains("git tag v1 "),
            "{m}"
        );
        assert!(matches!(delete_many(&g, &s(&["wip"]), &[], false), Err(OpError::Failed(e)) if e.contains("wip")));
        delete_many(&g, &s(&["wip"]), &[], true).unwrap();
        assert!(!here("refs/heads/wip"));
        assert_eq!(count_refs(1, 0), "1 branch");
        let _ = std::fs::remove_dir_all(&hub);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
