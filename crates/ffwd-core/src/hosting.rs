//! Hosting services (FF-40) through their own command-line tools: GitHub
//! through `gh`, signed in by the user. ffwd never sees a token: `gh`
//! keeps it in the OS keychain. Without `gh` (or not signed in) the
//! features stay hidden and say why when asked.

use crate::git::Git;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u32,
    pub title: Box<str>,
    /// The branch it merges from.
    pub head: Box<str>,
    pub author: Box<str>,
    pub draft: bool,
    pub url: Box<str>,
}

/// The open pull requests of the repository's GitHub remote (at most 50),
/// or why there are none to show.
pub fn pull_requests(git: &Git) -> Result<Vec<PullRequest>, String> {
    let mut cmd = std::process::Command::new("gh");
    cmd.args([
        "pr",
        "list",
        "--state",
        "open",
        "--limit",
        "50",
        "--json",
        "number,title,headRefName,author,isDraft,url",
    ])
    .current_dir(&git.root)
    .env("GH_PROMPT_DISABLED", "1")
    .env("NO_COLOR", "1")
    .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|_| "The GitHub CLI (gh) is not installed".to_string())?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if e.contains("auth login") || e.contains("not logged") {
            "The GitHub CLI is not signed in: run gh auth login".into()
        } else if e.is_empty() {
            "gh could not list pull requests".into()
        } else {
            e.lines().next().unwrap_or("").to_string()
        });
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

fn parse(json: &str) -> Result<Vec<PullRequest>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("gh answered something unexpected: {e}"))?;
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|v| v.as_str()).unwrap_or("").into();
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(PullRequest {
                        number: p.get("number")?.as_u64()? as u32,
                        title: s(p, "title"),
                        head: s(p, "headRefName"),
                        author: p.get("author").map(|a| s(a, "login")).unwrap_or_default(),
                        draft: p.get("isDraft").and_then(|d| d.as_bool()).unwrap_or(false),
                        url: s(p, "url"),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The state of a commit's CI runs (FF-24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ci {
    Passed,
    Running,
    Failed,
}

/// The newest GitHub Actions runs (at most 100) by commit: failed if any
/// run failed, else running if any runs, else passed; with the URL of the
/// run that decided it.
pub fn ci_runs(git: &Git) -> Result<Vec<(String, Ci, String)>, String> {
    let mut cmd = std::process::Command::new("gh");
    cmd.args(["run", "list", "--limit", "100", "--json", "headSha,status,conclusion,url"])
        .current_dir(&git.root)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|_| "The GitHub CLI (gh) is not installed".to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("gh failed").to_string());
    }
    parse_runs(&String::from_utf8_lossy(&out.stdout))
}

fn parse_runs(json: &str) -> Result<Vec<(String, Ci, String)>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("gh answered something unexpected: {e}"))?;
    let mut by: Vec<(String, Ci, String)> = Vec::new();
    for r in v.as_array().into_iter().flatten() {
        let (Some(sha), Some(status)) =
            (r.get("headSha").and_then(|x| x.as_str()), r.get("status").and_then(|x| x.as_str()))
        else {
            continue;
        };
        let conclusion = r.get("conclusion").and_then(|x| x.as_str()).unwrap_or("");
        let url = r.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let state = if status != "completed" {
            Ci::Running
        } else if matches!(conclusion, "success" | "skipped" | "neutral") {
            Ci::Passed
        } else {
            Ci::Failed
        };
        match by.iter_mut().find(|x| x.0 == sha) {
            // Worst wins: failed, then running, then passed.
            Some(x) => {
                let rank = |c: Ci| match c {
                    Ci::Failed => 2,
                    Ci::Running => 1,
                    Ci::Passed => 0,
                };
                if rank(state) > rank(x.1) {
                    x.1 = state;
                    x.2 = url;
                }
            }
            None => by.push((sha.to_string(), state, url)),
        }
    }
    Ok(by)
}

/// Whether the repository has a GitHub remote (origin, else any).
pub fn has_github_remote(remotes: &[crate::remotes::Remote]) -> bool {
    remotes.iter().any(|r| crate::web::host(&r.fetch_url).is_some_and(|h| h == "github.com"))
}

/// Opens GitHub's page for a new pull request from the current branch
/// (`gh pr create --web`; GitHub fills in the repository's template).
pub fn create_web(git: &Git) -> Result<String, String> {
    let mut cmd = std::process::Command::new("gh");
    cmd.args(["pr", "create", "--web"])
        .current_dir(&git.root)
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|_| "The GitHub CLI (gh) is not installed".to_string())?;
    if out.status.success() {
        return Ok("Opened the new pull request page in the browser".into());
    }
    let e = String::from_utf8_lossy(&out.stderr);
    let first = e.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("gh could not open it").to_string();
    Err(if first.contains("must first push") || first.contains("push the current branch") {
        "Push the branch first, then create the pull request".into()
    } else {
        first
    })
}

/// Fetches a pull request's head into `refs/heads/pr/<n>` and checks it
/// out (no `gh` needed: GitHub publishes `pull/<n>/head`).
pub fn checkout(git: &Git, remote: &str, number: u32) -> Result<String, crate::refops::OpError> {
    let branch = format!("pr/{number}");
    crate::refops::run(git, &["fetch", remote, &format!("+pull/{number}/head:refs/heads/{branch}")])?;
    crate::refops::run(git, &["switch", &branch])?;
    Ok(format!("Checked out pull request #{number} as {branch}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci_runs_by_commit_worst_wins() {
        let j = r#"[
            {"headSha":"aaa","status":"completed","conclusion":"success","url":"u1"},
            {"headSha":"aaa","status":"completed","conclusion":"failure","url":"u2"},
            {"headSha":"bbb","status":"in_progress","conclusion":"","url":"u3"},
            {"headSha":"ccc","status":"completed","conclusion":"skipped","url":"u4"}
        ]"#;
        let r = parse_runs(j).unwrap();
        assert_eq!(r[0], ("aaa".into(), Ci::Failed, "u2".into()));
        assert_eq!(r[1].1, Ci::Running);
        assert_eq!(r[2].1, Ci::Passed);
    }

    #[test]
    fn parses_gh_output() {
        let j = r#"[{"author":{"login":"alice","name":"A"},"headRefName":"feat/x","isDraft":true,"number":7,"title":"Add x","url":"https://github.com/o/r/pull/7"},{"bad":1}]"#;
        let p = parse(j).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].number, &*p[0].head, &*p[0].author, p[0].draft), (7, "feat/x", "alice", true));
        assert!(parse("not json").is_err());
        assert!(parse("[]").unwrap().is_empty());
    }

    #[test]
    fn checks_out_a_pull_request_head() {
        // A local "GitHub": a bare repository with refs/pull/1/head.
        let base = std::env::temp_dir().join(format!("ffwd-hosting-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let g = Git { root: base.join("work") };
        std::fs::create_dir_all(&g.root).unwrap();
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(g.root.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        Git { root: base.clone() }.output(&["init", "-q", "--bare", "hub.git"]).unwrap();
        g.output(&["remote", "add", "origin", &base.join("hub.git").to_string_lossy()]).unwrap();
        g.output(&["push", "-q", "origin", "main"]).unwrap();
        std::fs::write(g.root.join("b.txt"), "b").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "pr work"]).unwrap();
        g.output(&["push", "-q", "origin", "HEAD:refs/pull/1/head"]).unwrap();
        g.output(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        let msg = checkout(&g, "origin", 1).unwrap();
        assert!(msg.contains("pr/1"));
        assert!(g.root.join("b.txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
