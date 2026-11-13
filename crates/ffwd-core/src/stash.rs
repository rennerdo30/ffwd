//! Stashes: save, apply, pop, rename and drop, through the `git` CLI.
//! Dropping keeps the stash commit under `refs/ffwd/backup/` and names the
//! command that brings it back (AGENTS.md rule 8); renaming stores the new
//! entry before the old one is dropped, so a failure never loses it.

use crate::git::Git;
use crate::refops::{OpError, conflicts, run};

type R = Result<String, OpError>;

/// The index of `stash@{n}`.
fn index(name: &str) -> Option<usize> {
    name.strip_prefix("stash@{")?.strip_suffix('}')?.parse().ok()
}

fn sha(git: &Git, name: &str) -> Result<String, OpError> {
    run(git, &["rev-parse", "--verify", "--quiet", &format!("{name}^{{commit}}")])
        .map_err(|_| OpError::Failed(format!("{name} does not exist (any more)")))
}

/// Keeps stash commit `id` as `refs/ffwd/backup/stash-<time>-<sha>`.
fn backup(git: &Git, id: &str) -> Result<String, OpError> {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let name = format!("refs/ffwd/backup/stash-{secs}-{}", &id[..id.len().min(10)]);
    run(git, &["update-ref", &name, id])?;
    Ok(name)
}

/// What a save does with untracked files and the index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveOptions {
    pub untracked: bool,
    pub keep_index: bool,
}

/// Stashes the working changes (`git stash push`).
pub fn save(git: &Git, message: &str, o: SaveOptions) -> R {
    let before = run(git, &["rev-parse", "--verify", "--quiet", "refs/stash"]).ok();
    let mut args = vec!["stash", "push"];
    if o.untracked {
        args.push("--include-untracked");
    }
    if o.keep_index {
        args.push("--keep-index");
    }
    let message = message.trim();
    if !message.is_empty() {
        args.extend(["-m", message]);
    }
    run(git, &args)?;
    // "No local changes to save" exits 0 and adds nothing.
    if run(git, &["rev-parse", "--verify", "--quiet", "refs/stash"]).ok() == before {
        return Err(OpError::Failed("There are no changes to stash".into()));
    }
    Ok(if message.is_empty() { "Stashed the changes".into() } else { format!("Stashed \"{message}\"") })
}

/// Applies `name` to the working tree; `pop` also drops it when it applied
/// cleanly (Git keeps it on conflicts).
pub fn apply(git: &Git, name: &str, pop: bool) -> R {
    sha(git, name)?;
    let verb = if pop { "pop" } else { "apply" };
    match run(git, &["stash", verb, name]) {
        Ok(_) if pop => Ok(format!("Applied and dropped {name}")),
        Ok(_) => Ok(format!("Applied {name}; the stash is kept")),
        Err(e) => match conflicts(git, "") {
            Some(OpError::Conflicts { files, .. }) => Err(OpError::Failed(format!(
                "{name} applied with conflicts in {files} file{}: resolve them in Local Changes. The stash is kept",
                if files == 1 { "" } else { "s" }
            ))),
            _ => Err(e),
        },
    }
}

/// Drops `name`, keeping its commit as a backup ref.
pub fn drop(git: &Git, name: &str) -> R {
    let id = sha(git, name)?;
    let message = run(git, &["log", "-1", "--format=%gs", "-g", name]).unwrap_or_default();
    backup(git, &id)?;
    run(git, &["stash", "drop", name])?;
    let short = &id[..id.len().min(10)];
    let m = if message.is_empty() { String::new() } else { format!(" -m \"{}\"", message.replace('"', "'")) };
    Ok(format!("Dropped {name}. Bring it back with: git stash store{m} {short}"))
}

/// Gives `name` a new message. The stash moves to the top (stash@{0}).
pub fn rename(git: &Git, name: &str, message: &str) -> R {
    let message = message.trim();
    if message.is_empty() {
        return Err(OpError::Failed("Type a message first".into()));
    }
    let n = index(name).ok_or_else(|| OpError::Failed(format!("{name} is not a stash")))?;
    let id = sha(git, name)?;
    if sha(git, "stash@{0}")? == id {
        // Storing the commit refs/stash already points to adds no entry:
        // keep a backup, drop the old entry, store it under the new message.
        let backup = backup(git, &id)?;
        run(git, &["stash", "drop", "stash@{0}"])?;
        run(git, &["stash", "store", "-m", message, &id]).map_err(|e| {
            OpError::Failed(format!("{e}. The stash is kept as {backup}: git stash store -m \"{message}\" {id}"))
        })?;
        return Ok(format!("Renamed the stash to \"{message}\""));
    }
    // Store first: the old entry is now one further down.
    run(git, &["stash", "store", "-m", message, &id])?;
    let old = format!("stash@{{{}}}", n + 1);
    if sha(git, &old).ok().as_deref() != Some(id.as_str()) {
        return Err(OpError::Failed(format!(
            "Stored the renamed stash as stash@{{0}}, but {old} is not the old one; nothing was dropped"
        )));
    }
    run(git, &["stash", "drop", &old])?;
    Ok(format!("Renamed the stash to \"{message}\" (now stash@{{0}})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn repo(name: &str) -> (PathBuf, Git) {
        let dir = std::env::temp_dir().join(format!("ffwd-stash-{name}-{}", std::process::id()));
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
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        (dir, g)
    }

    fn list(g: &Git) -> Vec<String> {
        let out = g.output(&["stash", "list", "--format=%gs"]).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| l.rsplit(": ").next().unwrap_or(l).to_string()).collect()
    }

    fn read(dir: &Path, f: &str) -> String {
        std::fs::read_to_string(dir.join(f)).unwrap_or_default()
    }

    #[test]
    fn save_apply_pop_rename_drop() {
        let (dir, g) = repo("all");
        assert!(matches!(save(&g, "x", SaveOptions::default()), Err(OpError::Failed(m)) if m.contains("no changes")));
        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        std::fs::write(dir.join("new.txt"), "new\n").unwrap();
        // Without untracked files the new file stays.
        save(&g, "first stash", SaveOptions::default()).unwrap();
        assert_eq!(read(&dir, "a.txt"), "one\n");
        assert_eq!(read(&dir, "new.txt"), "new\n");
        save(&g, "with untracked", SaveOptions { untracked: true, keep_index: false }).unwrap();
        assert!(!dir.join("new.txt").exists());
        assert_eq!(list(&g), ["with untracked", "first stash"]);
        // Apply keeps it; pop drops it.
        apply(&g, "stash@{0}", false).unwrap();
        assert_eq!(read(&dir, "new.txt"), "new\n");
        assert_eq!(list(&g).len(), 2);
        std::fs::remove_file(dir.join("new.txt")).unwrap();
        apply(&g, "stash@{0}", true).unwrap();
        assert_eq!(list(&g), ["first stash"]);
        std::fs::remove_file(dir.join("new.txt")).unwrap();
        // Rename moves it to the top under the new message.
        std::fs::write(dir.join("a.txt"), "three\n").unwrap();
        save(&g, "top", SaveOptions::default()).unwrap();
        assert_eq!(list(&g), ["top", "first stash"]);
        rename(&g, "stash@{1}", "renamed").unwrap();
        assert_eq!(list(&g), ["renamed", "top"]);
        // The top one too (Git's store adds nothing for the current tip).
        rename(&g, "stash@{0}", "renamed again").unwrap();
        assert_eq!(list(&g), ["renamed again", "top"]);
        // Drop keeps a backup ref and names the way back.
        let id = String::from_utf8(g.output(&["rev-parse", "stash@{0}"]).unwrap()).unwrap().trim().to_string();
        let msg = drop(&g, "stash@{0}").unwrap();
        assert!(msg.contains("git stash store"), "{msg}");
        let backups =
            String::from_utf8(g.output(&["for-each-ref", "--format=%(objectname)", "refs/ffwd/backup/"]).unwrap())
                .unwrap();
        assert!(backups.contains(&id));
        assert!(matches!(drop(&g, "stash@{9}"), Err(OpError::Failed(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_with_conflicts_keeps_the_stash() {
        let (dir, g) = repo("conflict");
        std::fs::write(dir.join("a.txt"), "stashed\n").unwrap();
        save(&g, "s", SaveOptions::default()).unwrap();
        std::fs::write(dir.join("a.txt"), "committed\n").unwrap();
        g.output(&["commit", "-q", "-am", "second"]).unwrap();
        let e = apply(&g, "stash@{0}", true).unwrap_err();
        assert!(e.to_string().contains("conflicts in 1 file"), "{e}");
        assert_eq!(list(&g), ["s"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
