//! What ffwd remembers about a run, in `<root>/.runs/<id>/run.toml` next to
//! the worktrees. A run survives an ffwd restart: its worktree and branch
//! stay, and the agents panel lists it again for review or discard.

use crate::config::Config;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Preparing,
    Running,
    Finishing,
    /// The agent ended with exit code 0.
    Done,
    Failed,
    Cancelled,
    TimedOut,
    /// ffwd closed while the run was active.
    Interrupted,
    Merged,
    Discarded,
}

impl Status {
    pub fn active(self) -> bool {
        matches!(self, Status::Preparing | Status::Running | Status::Finishing)
    }

    pub fn word(self) -> &'static str {
        match self {
            Status::Preparing => "preparing",
            Status::Running => "running",
            Status::Finishing => "finishing",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
            Status::TimedOut => "timed out",
            Status::Interrupted => "interrupted",
            Status::Merged => "merged",
            Status::Discarded => "discarded",
        }
    }

    fn parse(s: &str) -> Status {
        match s {
            "preparing" | "running" | "finishing" => Status::Interrupted,
            "done" => Status::Done,
            "failed" => Status::Failed,
            "cancelled" => Status::Cancelled,
            "timed out" => Status::TimedOut,
            "merged" => Status::Merged,
            "discarded" => Status::Discarded,
            _ => Status::Interrupted,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub id: String,
    /// The repository's top folder.
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub branch: String,
    /// The commit the branch started from.
    pub base: String,
    /// The prompt's first line.
    pub title: String,
    pub started: i64,
    pub ended: Option<i64>,
    pub status: Status,
    /// One line: why it failed, or what it produced.
    pub detail: String,
    pub commits: u32,
    /// `git diff --shortstat` of the branch against its base.
    pub shortstat: String,
    pub sandbox: String,
}

/// Where a repository's agent worktrees go.
pub fn root(cfg: &Config, repo: &Path) -> PathBuf {
    let name = repo.file_name().map_or_else(|| "repo".into(), |n| n.to_string_lossy().into_owned());
    match &cfg.worktree_root {
        Some(r) => r.join(&name),
        None => repo.parent().unwrap_or(repo).join(".ffwd-agents").join(&name),
    }
}

pub fn run_dir(root: &Path, id: &str) -> PathBuf {
    root.join(".runs").join(id)
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn unesc(s: &str) -> Option<String> {
    let inner = s.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut o = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next()? {
                'n' => o.push('\n'),
                'r' => o.push('\r'),
                c => o.push(c),
            }
        } else {
            o.push(c);
        }
    }
    Some(o)
}

impl Record {
    pub fn to_text(&self) -> String {
        let p = |p: &Path| p.to_string_lossy().into_owned();
        let rows: [(&str, String); 13] = [
            ("id", self.id.clone()),
            ("repo", p(&self.repo)),
            ("worktree", p(&self.worktree)),
            ("branch", self.branch.clone()),
            ("base", self.base.clone()),
            ("title", self.title.clone()),
            ("started", self.started.to_string()),
            ("ended", self.ended.map(|e| e.to_string()).unwrap_or_default()),
            ("status", self.status.word().into()),
            ("detail", self.detail.clone()),
            ("commits", self.commits.to_string()),
            ("shortstat", self.shortstat.clone()),
            ("sandbox", self.sandbox.clone()),
        ];
        let mut t = String::from("# ffwd agent run (experimental). ffwd rewrites this file.\n");
        for (k, v) in rows {
            t.push_str(&format!("{k} = {}\n", esc(&v)));
        }
        t
    }

    pub fn parse(text: &str) -> Option<Record> {
        let mut r = Record {
            id: String::new(),
            repo: PathBuf::new(),
            worktree: PathBuf::new(),
            branch: String::new(),
            base: String::new(),
            title: String::new(),
            started: 0,
            ended: None,
            status: Status::Interrupted,
            detail: String::new(),
            commits: 0,
            shortstat: String::new(),
            sandbox: String::new(),
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let Some(v) = unesc(v) else { continue };
            match k.trim() {
                "id" => r.id = v,
                "repo" => r.repo = v.into(),
                "worktree" => r.worktree = v.into(),
                "branch" => r.branch = v,
                "base" => r.base = v,
                "title" => r.title = v,
                "started" => r.started = v.parse().unwrap_or(0),
                "ended" => r.ended = v.parse().ok(),
                "status" => r.status = Status::parse(&v),
                "detail" => r.detail = v,
                "commits" => r.commits = v.parse().unwrap_or(0),
                "shortstat" => r.shortstat = v,
                "sandbox" => r.sandbox = v,
                _ => {}
            }
        }
        (!r.id.is_empty() && !r.branch.is_empty()).then_some(r)
    }

    /// Writes the record (temporary file, then rename).
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join("run.toml.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(tmp, dir.join("run.toml"))
    }
}

/// Every run recorded under `root`, newest first. Runs that were active
/// when ffwd closed come back as interrupted. Discarded runs are skipped.
pub fn load_all(root: &Path) -> Vec<Record> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(root.join(".runs")) else { return v };
    for e in rd.flatten().take(1000) {
        if let Ok(t) = std::fs::read_to_string(e.path().join("run.toml"))
            && let Some(r) = Record::parse(&t)
            && r.status != Status::Discarded
        {
            v.push(r);
        }
    }
    v.sort_by_key(|r| std::cmp::Reverse(r.started));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_interrupted() {
        let r = Record {
            id: "fix-login-k3x".into(),
            repo: PathBuf::from(r"E:\Development\app"),
            worktree: PathBuf::from(r"E:\Development\.ffwd-agents\app\fix-login-k3x"),
            branch: "agent/fix-login-k3x".into(),
            base: "0123abcd".into(),
            title: "Fix \"login\" \\ bug\nsecond".into(),
            started: 1_700_000_000,
            ended: None,
            status: Status::Running,
            detail: String::new(),
            commits: 2,
            shortstat: "3 files changed".into(),
            sandbox: "bubblewrap".into(),
        };
        let back = Record::parse(&r.to_text()).unwrap();
        assert_eq!(back.title, r.title);
        assert_eq!(back.worktree, r.worktree);
        assert_eq!(back.commits, 2);
        assert_eq!(back.status, Status::Interrupted, "an active run read back after a restart");
        assert!(Record::parse("id = \"x\"\n").is_none());
        assert!(Record::parse("garbage\n\u{0}").is_none());
    }

    #[test]
    fn default_root_sits_next_to_the_repository() {
        let c = Config::default();
        assert_eq!(root(&c, Path::new("/src/app")), PathBuf::from("/src/.ffwd-agents/app"));
        let c = Config { worktree_root: Some("/agents".into()), ..Config::default() };
        assert_eq!(root(&c, Path::new("/src/app")), PathBuf::from("/agents/app"));
    }
}
