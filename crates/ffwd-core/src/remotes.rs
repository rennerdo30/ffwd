//! Remotes: list, add, change the URL, rename, remove, fetch one, and
//! test the connection, through the `git` CLI. Removing names the command
//! that adds the remote back (AGENTS.md rule 8).

use crate::git::Git;
use crate::refops::{OpError, run};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

type R = Result<String, OpError>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    pub name: Box<str>,
    pub fetch_url: Box<str>,
    /// Only when it differs from the fetch URL.
    pub push_url: Option<Box<str>>,
}

/// The configured remotes, in `git remote -v` order.
pub fn list(git: &Git) -> Vec<Remote> {
    let out = git.output_limited(&["remote", "-v"], 1 << 20).unwrap_or_default();
    parse(&String::from_utf8_lossy(&out))
}

fn parse(out: &str) -> Vec<Remote> {
    let mut v: Vec<Remote> = Vec::new();
    for line in out.lines() {
        let mut it = line.split('\t');
        let (Some(name), Some(rest)) = (it.next(), it.next()) else { continue };
        let (url, kind) = rest.rsplit_once(' ').unwrap_or((rest, "(fetch)"));
        match v.iter_mut().find(|r| &*r.name == name) {
            Some(r) if kind == "(push)" && &*r.fetch_url != url => r.push_url = Some(url.into()),
            Some(_) => {}
            None => v.push(Remote { name: name.into(), fetch_url: url.into(), push_url: None }),
        }
    }
    v
}

fn check_name(git: &Git, name: &str) -> Result<(), OpError> {
    let ok = !name.is_empty()
        && !name.contains('/')
        && run(git, &["check-ref-format", &format!("refs/remotes/{name}/x")]).is_ok();
    if ok { Ok(()) } else { Err(OpError::Failed(format!("\"{name}\" is not a valid remote name"))) }
}

fn check_url(url: &str) -> Result<(), OpError> {
    if url.is_empty() || url.chars().any(char::is_whitespace) || url.starts_with('-') {
        return Err(OpError::Failed("Type the remote's URL or path".into()));
    }
    Ok(())
}

/// Whether `url` names a server (`https://`, `ssh://`, `git@host:path`)
/// rather than a folder (`../x`, `C:/repos/x`, `file://`).
fn hosted(url: &str) -> bool {
    if let Some((scheme, _)) = url.split_once("://") {
        return scheme != "file";
    }
    // scp form: "user@host:path" or "host.tld:path", never "C:/…".
    url.split_once(':').is_some_and(|(host, _)| host.len() > 1 && (host.contains('@') || host.contains('.')))
}

/// A remote name from a URL: `origin` when free, else the owner of a
/// hosted repository (`git@github.com:alice/x.git` gives `alice`) or the
/// folder name of a local one, made unique.
pub fn suggest_name(url: &str, existing: &[Remote]) -> String {
    let taken = |n: &str| existing.iter().any(|r| &*r.name == n);
    if !taken("origin") {
        return "origin".into();
    }
    let path = url.trim_end_matches(['/', '\\']).trim_end_matches(".git");
    let parts: Vec<&str> = path.split(['/', '\\', ':']).filter(|p| !p.is_empty()).collect();
    let pick = if hosted(url) { parts.len().checked_sub(2) } else { parts.len().checked_sub(1) };
    let owner = pick.and_then(|i| parts.get(i)).copied().unwrap_or("upstream");
    let base: String =
        owner.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect::<String>();
    let base = if !base.chars().any(|c| c.is_ascii_alphanumeric()) { "upstream".to_string() } else { base };
    if !taken(&base) {
        return base;
    }
    (2..).map(|i| format!("{base}{i}")).find(|n| !taken(n)).unwrap_or(base)
}

/// Adds a remote, then fetches it.
pub fn add(git: &Git, name: &str, url: &str) -> R {
    let (name, url) = (name.trim(), url.trim());
    check_name(git, name)?;
    check_url(url)?;
    run(git, &["remote", "add", name, url])?;
    match run(git, &["fetch", "--prune", name]) {
        Ok(_) => Ok(format!("Added {name} and fetched it")),
        Err(e) => Ok(format!("Added {name}, but the first fetch failed: {e}")),
    }
}

pub fn set_url(git: &Git, name: &str, url: &str) -> R {
    let url = url.trim();
    check_url(url)?;
    run(git, &["remote", "set-url", name, url])?;
    Ok(format!("{name} now points to {url}"))
}

pub fn rename(git: &Git, old: &str, new: &str) -> R {
    let new = new.trim();
    check_name(git, new)?;
    run(git, &["remote", "rename", old, new])?;
    Ok(format!("Renamed {old} to {new}"))
}

/// Removes a remote and its remote-tracking branches (the branches on the
/// server stay).
pub fn remove(git: &Git, name: &str) -> R {
    let url = list(git).into_iter().find(|r| &*r.name == name).map(|r| r.fetch_url.to_string());
    run(git, &["remote", "remove", name])?;
    Ok(match url {
        Some(u) => format!("Removed {name}. Add it back with: git remote add {name} {u}"),
        None => format!("Removed {name}"),
    })
}

/// Fetches one remote.
pub fn fetch(git: &Git, name: &str) -> R {
    run(git, &["fetch", "--prune", name])?;
    Ok(format!("Fetched {name}"))
}

/// Asks the remote for its branches (`git ls-remote --heads`), giving up
/// after `timeout` or when `cancel` is set.
pub fn test(git: &Git, name: &str, timeout: Duration, cancel: Arc<AtomicBool>) -> R {
    let t = Instant::now();
    let (mut child, killer) = git
        .spawn_killable(&["ls-remote", "--heads", name])
        .map_err(|e| OpError::Failed(format!("cannot start git: {e}")))?;
    let reader = |r: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(mut r) = r {
                let _ = (&mut r).take(1 << 20).read_to_end(&mut b);
                let _ = std::io::copy(&mut r, &mut std::io::sink());
            }
            b
        })
    };
    use std::io::Read;
    let out = reader(child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>));
    let err = reader(child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>));
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if t.elapsed() < timeout && !cancel.load(Ordering::Relaxed) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Ok(None) => {
                killer.kill();
                let _ = child.wait();
                break None;
            }
            Err(_) => break None,
        }
    };
    // After a kill, never wait on pipes a helper might still hold.
    let join = |h: std::thread::JoinHandle<Vec<u8>>| {
        if status.is_some() || h.is_finished() { h.join().unwrap_or_default() } else { Vec::new() }
    };
    let out = join(out);
    let err = String::from_utf8_lossy(&join(err)).into_owned();
    let secs = t.elapsed().as_secs_f64();
    match status {
        Some(s) if s.success() => {
            let n = out.split(|&b| b == b'\n').filter(|l| !l.is_empty()).count();
            Ok(format!("{name} answers: {n} branch{} ({secs:.1} s)", if n == 1 { "" } else { "es" }))
        }
        Some(_) => {
            let why = err
                .lines()
                .map(str::trim)
                .rfind(|l| l.starts_with("fatal:") || l.starts_with("error:"))
                .or(err.lines().map(str::trim).rfind(|l| !l.is_empty()))
                .unwrap_or("no answer")
                .trim_start_matches("fatal: ")
                .trim_start_matches("error: ")
                .to_string();
            Err(OpError::Failed(format!("{name} does not answer: {why}")))
        }
        None if cancel.load(Ordering::Relaxed) => Err(OpError::Failed("Connection test cancelled".into())),
        None => Err(OpError::Failed(format!("{name} gave no answer within {} s", timeout.as_secs()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo(dir: &PathBuf) -> Git {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        g
    }

    #[test]
    fn parses_remote_v() {
        let r = parse(
            "origin\thttps://h/x.git (fetch)\norigin\thttps://h/x.git (push)\nfork\tgit@h:me/x.git (fetch)\nfork\tssh://h/me/x.git (push)\n",
        );
        assert_eq!(r.len(), 2);
        assert_eq!(&*r[0].fetch_url, "https://h/x.git");
        assert_eq!(r[0].push_url, None);
        assert_eq!(r[1].push_url.as_deref(), Some("ssh://h/me/x.git"));
    }

    #[test]
    fn suggests_names() {
        assert_eq!(suggest_name("https://github.com/alice/x.git", &[]), "origin");
        let o = [Remote { name: "origin".into(), ..Default::default() }];
        assert_eq!(suggest_name("https://github.com/alice/x.git", &o), "alice");
        assert_eq!(suggest_name("git@github.com:bob/x.git", &o), "bob");
        assert_eq!(suggest_name("../x", &o), "x");
        assert_eq!(suggest_name("C:/repos/lib.git", &o), "lib");
        assert_eq!(suggest_name("ssh://host/team/lib.git", &o), "team");
        assert_eq!(suggest_name("..", &o), "upstream");
        let ob = [o[0].clone(), Remote { name: "bob".into(), ..Default::default() }];
        assert_eq!(suggest_name("git@github.com:bob/x.git", &ob), "bob2");
    }

    #[test]
    fn add_rename_set_url_test_remove() {
        let base = std::env::temp_dir().join(format!("ffwd-remotes-{}", std::process::id()));
        let up = repo(&base.join("up"));
        let g = repo(&base.join("down"));
        let path = up.root.to_string_lossy().replace('\\', "/");
        assert!(matches!(add(&g, "bad name", &path), Err(OpError::Failed(_))));
        assert!(add(&g, "up", &path).unwrap().contains("fetched"));
        assert!(g.output(&["rev-parse", "--verify", "refs/remotes/up/main"]).is_ok(), "the add fetched");
        rename(&g, "up", "upstream").unwrap();
        assert_eq!(&*list(&g)[0].name, "upstream");
        let ok = test(&g, "upstream", Duration::from_secs(20), Default::default()).unwrap();
        assert!(ok.contains("answers: 1 branch"), "{ok}");
        set_url(&g, "upstream", &base.join("nowhere").to_string_lossy()).unwrap();
        assert!(test(&g, "upstream", Duration::from_secs(20), Default::default()).is_err());
        let msg = remove(&g, "upstream").unwrap();
        assert!(msg.contains("git remote add upstream"), "{msg}");
        assert!(list(&g).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
