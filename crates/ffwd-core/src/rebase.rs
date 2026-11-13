//! Interactive rebase without an editor: ffwd builds the todo list from
//! the user's plan and hands it to `git rebase -i` through
//! `GIT_SEQUENCE_EDITOR`. Rewording runs `git commit --amend -F <file>`
//! after the pick, squash keeps Git's combined message (`GIT_EDITOR=true`),
//! so Git never waits for an editor. HEAD is saved under
//! `refs/ffwd/backup/` first (AGENTS.md rule 8); conflicts stop the rebase
//! like the CLI, and Local Changes continues or aborts it.

use crate::git::Git;
use crate::refops::{OpError, conflicts, run};

type R = Result<String, OpError>;

/// The most commits one plan may hold.
pub const MAX_COMMITS: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Pick,
    /// Keep the commit with this new message.
    Reword(String),
    /// Fold into the commit before it, keeping both messages.
    Squash,
    /// Fold into the commit before it, keeping its message only.
    Fixup,
    Drop,
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::Pick => "pick",
            Action::Reword(_) => "reword",
            Action::Squash => "squash",
            Action::Fixup => "fixup",
            Action::Drop => "drop",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Full SHA.
    pub sha: String,
    pub subject: String,
    pub action: Action,
}

/// The commits from `from` (included) up to HEAD, oldest first, as picks.
/// Refuses merges in the range (they need `--rebase-merges`) and more
/// than [`MAX_COMMITS`] commits.
pub fn plan(git: &Git, from: &str) -> Result<(Option<String>, Vec<Step>), OpError> {
    let from_sha = run(git, &["rev-parse", "--verify", &format!("{from}^{{commit}}")])?;
    if run(git, &["merge-base", "--is-ancestor", &from_sha, "HEAD"]).is_err() {
        return Err(OpError::Failed("That commit is not part of the current branch".into()));
    }
    // The base is the parent; a root commit rebases with --root.
    let base = run(git, &["rev-parse", "--verify", "--quiet", &format!("{from_sha}^")]).ok();
    let range = match &base {
        Some(b) => format!("{b}..HEAD"),
        None => "HEAD".into(),
    };
    let merges = run(git, &["rev-list", "--merges", "--count", &range])?;
    if merges.trim() != "0" {
        return Err(OpError::Failed(format!(
            "The range has {} merge commit(s); interactive rebase here keeps history linear",
            merges.trim()
        )));
    }
    let out = run(git, &["log", "--reverse", "--format=%H%x1f%s", &range])?;
    let steps: Vec<Step> = out
        .lines()
        .filter_map(|l| l.split_once('\x1f'))
        .map(|(sha, s)| Step { sha: sha.to_string(), subject: s.to_string(), action: Action::Pick })
        .collect();
    if steps.len() > MAX_COMMITS {
        return Err(OpError::Failed(format!("More than {MAX_COMMITS} commits; pick a later commit")));
    }
    Ok((base, steps))
}

/// Why a plan cannot run, if it cannot.
pub fn check(steps: &[Step]) -> Option<String> {
    let kept: Vec<&Step> = steps.iter().filter(|s| s.action != Action::Drop).collect();
    if kept.is_empty() {
        return Some("Every commit would be dropped; use a reset instead".into());
    }
    if matches!(kept[0].action, Action::Squash | Action::Fixup) {
        return Some("The oldest kept commit has nothing to fold into".into());
    }
    if steps.iter().any(|s| matches!(&s.action, Action::Reword(m) if m.trim().is_empty())) {
        return Some("A reworded commit needs a message".into());
    }
    None
}

/// A path the editor commands can use (`sh` on every platform Git runs on).
fn sh_path(p: &std::path::Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\\', "/").replace('\'', "'\\''"))
}

/// Runs the plan (oldest first) on top of `base` (`None`: from the root).
pub fn run_plan(git: &Git, base: Option<&str>, steps: &[Step]) -> R {
    if let Some(why) = check(steps) {
        return Err(OpError::Failed(why));
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let backup = format!("refs/ffwd/backup/rebase-i-{secs}");
    run(git, &["update-ref", &backup, "HEAD"])?;
    // The todo and messages go next to Git's own state, not in the tree.
    let gitdir = run(git, &["rev-parse", "--path-format=absolute", "--git-dir"])?;
    let dir = std::path::Path::new(&gitdir).join("ffwd-rebase");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| OpError::Failed(format!("cannot write the plan: {e}")))?;
    let mut todo = String::new();
    for (i, s) in steps.iter().enumerate() {
        let a = match &s.action {
            Action::Reword(_) => "pick",
            other => other.name(),
        };
        todo.push_str(&format!("{a} {}\n", s.sha));
        if let Action::Reword(m) = &s.action {
            let f = dir.join(format!("msg-{i}.txt"));
            std::fs::write(&f, m.trim_end().to_string() + "\n")
                .map_err(|e| OpError::Failed(format!("cannot write the plan: {e}")))?;
            todo.push_str(&format!("exec git commit --amend --allow-empty --only -q -F {}\n", sh_path(&f)));
        }
    }
    let todo_file = dir.join("todo");
    std::fs::write(&todo_file, &todo).map_err(|e| OpError::Failed(format!("cannot write the plan: {e}")))?;
    let mut args = vec!["rebase", "-i", "--autostash"];
    match base {
        Some(b) => args.push(b),
        None => args.push("--root"),
    }
    let mut cmd = git.command(&args);
    cmd.env("GIT_SEQUENCE_EDITOR", format!("cp {}", sh_path(&todo_file)))
        .env("GIT_EDITOR", "true")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let t = std::time::Instant::now();
    let out = cmd.output().map_err(|e| OpError::Failed(format!("could not start git: {e}")))?;
    crate::git::note(&args, out.status.code(), &out.stderr, t);
    let _ = std::fs::remove_dir_all(&dir);
    if out.status.success() {
        return Ok(format!(
            "Rebased {} commit{}. The old tip is kept as {backup}",
            steps.len(),
            if steps.len() == 1 { "" } else { "s" }
        ));
    }
    if let Some(c) = conflicts(git, "git rebase --abort") {
        return Err(c);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let why = err
        .lines()
        .map(str::trim)
        .rfind(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .or(err.lines().map(str::trim).rfind(|l| !l.is_empty()))
        .unwrap_or("git reported an error")
        .trim_start_matches("fatal: ")
        .trim_start_matches("error: ")
        .to_string();
    Err(OpError::Failed(format!("{why}. The old tip is kept as {backup}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo(name: &str) -> (PathBuf, Git) {
        let dir = std::env::temp_dir().join(format!("ffwd-rebase-{name}-{}", std::process::id()));
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
        for (f, m) in [("a", "one"), ("b", "two"), ("c", "three"), ("d", "four")] {
            std::fs::write(dir.join(f), f).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        }
        (dir, g)
    }

    fn subjects(g: &Git) -> Vec<String> {
        String::from_utf8(g.output(&["log", "--format=%s"]).unwrap()).unwrap().lines().map(String::from).collect()
    }

    #[test]
    fn reorder_reword_squash_fixup_drop() {
        let (dir, g) = repo("all");
        let second = String::from_utf8(g.output(&["rev-parse", "HEAD~2"]).unwrap()).unwrap().trim().to_string();
        let (base, mut steps) = plan(&g, &second).unwrap();
        assert!(base.is_some());
        assert_eq!(steps.iter().map(|s| s.subject.as_str()).collect::<Vec<_>>(), ["two", "three", "four"]);
        // four first, reworded; then two; three folds into two; nothing dropped yet.
        steps.swap(0, 2); // four, three, two
        steps.swap(1, 2); // four, two, three
        steps[0].action = Action::Reword("four, reworded".into());
        steps[2].action = Action::Fixup;
        let msg = run_plan(&g, base.as_deref(), &steps).unwrap();
        assert!(msg.contains("refs/ffwd/backup/rebase-i-"), "{msg}");
        assert_eq!(subjects(&g), ["two", "four, reworded", "one"]);
        for f in ["a", "b", "c", "d"] {
            assert!(dir.join(f).exists(), "{f} is kept");
        }
        // Drop the newest and squash: the files follow.
        let (base, mut steps) = plan(&g, "HEAD~1").unwrap();
        steps[1].action = Action::Drop;
        run_plan(&g, base.as_deref(), &steps).unwrap();
        assert_eq!(subjects(&g), ["four, reworded", "one"]);
        assert!(!dir.join("b").exists() && !dir.join("c").exists(), "the dropped commit's files are gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checks_plans_and_refuses_merges() {
        let s = |a: Action| Step { sha: "x".into(), subject: "s".into(), action: a };
        assert!(check(&[s(Action::Squash), s(Action::Pick)]).is_some(), "nothing to fold into");
        assert!(check(&[s(Action::Drop), s(Action::Fixup)]).is_some(), "the first kept one cannot fold");
        assert!(check(&[s(Action::Drop)]).is_some());
        assert!(check(&[s(Action::Reword(" ".into()))]).is_some());
        assert!(check(&[s(Action::Pick), s(Action::Squash)]).is_none());
        let (dir, g) = repo("merges");
        g.output(&["switch", "-q", "-c", "side", "HEAD~1"]).unwrap();
        std::fs::write(dir.join("e"), "e").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "side"]).unwrap();
        g.output(&["switch", "-q", "main"]).unwrap();
        g.output(&["merge", "-q", "--no-ff", "--no-edit", "side"]).unwrap();
        assert!(matches!(plan(&g, "HEAD~2"), Err(OpError::Failed(m)) if m.contains("merge")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn root_and_conflicts() {
        let (dir, g) = repo("root");
        let root = String::from_utf8(g.output(&["rev-list", "--max-parents=0", "HEAD"]).unwrap()).unwrap();
        let (base, mut steps) = plan(&g, root.trim()).unwrap();
        assert!(base.is_none() && steps.len() == 4);
        steps[3].action = Action::Squash; // four into three: keeps both messages
        run_plan(&g, None, &steps).unwrap();
        assert_eq!(subjects(&g), ["three", "two", "one"]);
        let body = String::from_utf8(g.output(&["log", "-1", "--format=%B"]).unwrap()).unwrap();
        assert!(body.contains("three") && body.contains("four"), "{body}");
        // Swapping two commits that touch the same line stops with conflicts.
        std::fs::write(dir.join("a"), "x").unwrap();
        g.output(&["commit", "-q", "-am", "x"]).unwrap();
        std::fs::write(dir.join("a"), "y").unwrap();
        g.output(&["commit", "-q", "-am", "y"]).unwrap();
        let (base, mut steps) = plan(&g, "HEAD~1").unwrap();
        steps.swap(0, 1);
        let e = run_plan(&g, base.as_deref(), &steps).unwrap_err();
        assert!(matches!(e, OpError::Conflicts { .. }), "{e}");
        g.output(&["rebase", "--abort"]).unwrap();
        assert_eq!(subjects(&g)[0], "y", "abort restores the branch");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
