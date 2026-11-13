//! Submodules: list, init and update, add (with an automatic relative
//! URL), move, and switching URLs between relative and absolute (FF-45,
//! FF-54). Everything runs through `git`; `.git` is never edited by hand.
//! Moving refuses while the submodule has work that is not safe yet
//! (AGENTS.md rule 8).

use crate::git::Git;
use crate::refops::{OpError, run};

type R = Result<String, OpError>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Submodule {
    pub name: Box<str>,
    pub path: Box<str>,
    /// As written in `.gitmodules` (may be relative).
    pub url: Box<str>,
    pub state: State,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// Checked out at the recorded commit.
    #[default]
    Clean,
    /// Not initialised (`-`).
    Uninitialized,
    /// Checked out at another commit than the superproject records (`+`).
    Moved,
    /// Merge conflicts (`U`).
    Conflict,
}

/// The submodules in `.gitmodules` with their state.
pub fn list(git: &Git) -> Vec<Submodule> {
    let cfg = run(git, &["config", "-f", ".gitmodules", "--null", "--get-regexp", r"^submodule\..*\.(path|url)$"])
        .unwrap_or_default();
    let mut v: Vec<Submodule> = Vec::new();
    for entry in cfg.split('\0').filter(|e| !e.is_empty()) {
        let Some((key, value)) = entry.split_once('\n') else { continue };
        let Some(rest) = key.strip_prefix("submodule.") else { continue };
        let Some((name, field)) = rest.rsplit_once('.') else { continue };
        let i = match v.iter().position(|s| &*s.name == name) {
            Some(i) => i,
            None => {
                v.push(Submodule { name: name.into(), ..Default::default() });
                v.len() - 1
            }
        };
        match field {
            "path" => v[i].path = value.into(),
            "url" => v[i].url = value.into(),
            _ => {}
        }
    }
    v.retain(|s| !s.path.is_empty());
    if let Ok(st) = run(git, &["submodule", "status"]) {
        for line in st.lines() {
            let state = match line.chars().next() {
                Some('-') => State::Uninitialized,
                Some('+') => State::Moved,
                Some('U') => State::Conflict,
                _ => State::Clean,
            };
            let path = line.get(1..).unwrap_or("").split_whitespace().nth(1).unwrap_or("");
            if let Some(s) = v.iter_mut().find(|s| &*s.path == path) {
                s.state = state;
            }
        }
    }
    v
}

/// `git submodule update --init --recursive` for one path or all.
pub fn update(git: &Git, path: Option<&str>) -> R {
    let mut args = vec!["submodule", "update", "--init", "--recursive"];
    if let Some(p) = path {
        args.extend(["--", p]);
    }
    run(git, &args)?;
    Ok(match path {
        Some(p) => format!("Updated {p}"),
        None => "Updated all submodules".into(),
    })
}

/// A URL split into host and path parts, for https, ssh, git and scp forms.
#[derive(Debug, PartialEq, Eq)]
struct Parsed<'a> {
    scheme: &'a str,
    /// `user@host[:port]`, or the scp host.
    host: &'a str,
    parts: Vec<&'a str>,
}

fn parse(url: &str) -> Option<Parsed<'_>> {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        if scheme == "file" {
            return None;
        }
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let parts = path.split('/').filter(|p| !p.is_empty()).collect();
        return Some(Parsed { scheme, host, parts });
    }
    // scp form: "git@github.com:owner/repo.git" (never "C:/…").
    let (host, path) = url.split_once(':')?;
    if host.len() < 2 || !(host.contains('@') || host.contains('.')) || host.contains('/') {
        return None;
    }
    Some(Parsed { scheme: "scp", host, parts: path.split('/').filter(|p| !p.is_empty()).collect() })
}

/// The host without user and port, for comparing forms.
fn bare_host(h: &str) -> &str {
    let h = h.rsplit_once('@').map_or(h, |x| x.1);
    h.split_once(':').map_or(h, |x| x.0)
}

/// The relative URL Git resolves against `super_url` to reach `sub_url`
/// (`../lib.git`), when both are on the same host. Git takes the
/// superproject URL as the starting point; each `../` removes one part.
pub fn relative_url(super_url: &str, sub_url: &str) -> Option<String> {
    let (a, b) = (parse(super_url)?, parse(sub_url)?);
    if !bare_host(a.host).eq_ignore_ascii_case(bare_host(b.host)) || b.parts.is_empty() {
        return None;
    }
    let trim = |p: &str| p.trim_end_matches(".git").to_string();
    let common = a.parts.iter().zip(&b.parts).take_while(|(x, y)| trim(x) == trim(y)).count();
    // The superproject itself (its last part) is always left.
    let common = common.min(a.parts.len().saturating_sub(1));
    let ups = a.parts.len() - common;
    let mut out = "../".repeat(ups);
    out.push_str(&b.parts[common..].join("/"));
    Some(out)
}

/// The absolute URL a relative one resolves to, the way Git does.
pub fn resolve(super_url: &str, rel: &str) -> Option<String> {
    if !rel.starts_with("./") && !rel.starts_with("../") {
        return Some(rel.to_string());
    }
    let base = super_url.trim().trim_end_matches('/');
    let p = parse(base)?;
    let mut parts: Vec<&str> = p.parts.clone();
    let mut rest = rel;
    loop {
        if let Some(r) = rest.strip_prefix("./") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("../") {
            parts.pop()?;
            rest = r;
        } else {
            break;
        }
    }
    let path = parts.into_iter().chain(rest.split('/').filter(|x| !x.is_empty())).collect::<Vec<_>>().join("/");
    Some(match p.scheme {
        "scp" => format!("{}:{path}", p.host),
        s => format!("{s}://{}/{path}", p.host),
    })
}

/// The URL relative submodule URLs resolve against: the remote of the
/// current branch, else `origin`.
pub fn super_url(git: &Git) -> Option<String> {
    let remote = run(git, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .ok()
        .and_then(|u| u.split_once('/').map(|x| x.0.to_string()))
        .unwrap_or_else(|| "origin".into());
    run(git, &["remote", "get-url", &remote]).ok().filter(|u| !u.is_empty())
}

/// Adds a submodule at `path`; `url` may be relative.
pub fn add(git: &Git, url: &str, path: &str) -> R {
    let (url, path) = (url.trim(), path.trim().trim_matches('/'));
    if url.is_empty() || path.is_empty() || url.starts_with('-') || path.starts_with('-') {
        return Err(OpError::Failed("Type the URL and the folder first".into()));
    }
    run(git, &["submodule", "add", "--", url, path])?;
    Ok(format!("Added the submodule {path} ({url}); commit .gitmodules and {path} to keep it"))
}

/// What is not safe yet in a submodule's checkout, if anything.
fn unsafe_work(git: &Git, path: &str) -> Option<String> {
    let sub = Git { root: git.root.join(path) };
    if !sub.root.join(".git").exists() {
        return None;
    }
    let dirty = run(&sub, &["status", "--porcelain"]).unwrap_or_default();
    let n = dirty.lines().filter(|l| !l.is_empty()).count();
    if n > 0 {
        return Some(format!("{path} has {n} uncommitted change{}", if n == 1 { "" } else { "s" }));
    }
    let unpushed = run(&sub, &["log", "--oneline", "--branches", "--not", "--remotes"]).unwrap_or_default();
    let n = unpushed.lines().filter(|l| !l.is_empty()).count();
    if n > 0 {
        return Some(format!("{path} has {n} commit{} on no remote", if n == 1 { "" } else { "s" }));
    }
    None
}

/// Moves a submodule (`git mv`: `.gitmodules`, the index and the
/// checkout follow). Refuses while it has uncommitted or unpushed work.
pub fn move_to(git: &Git, path: &str, new_path: &str) -> R {
    let new_path = new_path.trim().trim_matches('/');
    if new_path.is_empty() || new_path.starts_with('-') || new_path.split('/').any(|p| p == "..") {
        return Err(OpError::Failed("Type a folder inside the repository".into()));
    }
    if git.root.join(new_path).exists() {
        return Err(OpError::Failed(format!("{new_path} already exists")));
    }
    if let Some(why) = unsafe_work(git, path) {
        return Err(OpError::Failed(format!("{why}; commit and push it first, then move it")));
    }
    // git mv rewrites .gitmodules and refuses while it has unstaged edits.
    if run(git, &["diff", "--quiet", "--", ".gitmodules"]).is_err() {
        return Err(OpError::Failed(
            ".gitmodules has unstaged changes; stage or commit them in Local Changes first, then move".into(),
        ));
    }
    if let Some(parent) = std::path::Path::new(new_path).parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(git.root.join(parent))
            .map_err(|e| OpError::Failed(format!("cannot create {}: {e}", parent.display())))?;
    }
    run(git, &["mv", "--", path, new_path])?;
    Ok(format!("Moved {path} to {new_path}; commit the change to keep it"))
}

/// Writes `url` for submodule `name` in `.gitmodules`, then syncs.
fn set_url(git: &Git, name: &str, url: &str) -> Result<(), OpError> {
    run(git, &["config", "-f", ".gitmodules", &format!("submodule.{name}.url"), url])?;
    run(git, &["submodule", "sync", "--recursive", "--quiet"])?;
    Ok(())
}

/// Rewrites the absolute URLs that can be relative (same host as the
/// superproject's remote). Returns what changed.
pub fn make_relative(git: &Git, only: Option<&str>) -> R {
    let base = super_url(git)
        .ok_or_else(|| OpError::Failed("This repository has no remote, so relative URLs would point nowhere".into()))?;
    let mut changed = Vec::new();
    for s in list(git).into_iter().filter(|s| only.is_none_or(|n| *s.name == *n)) {
        if s.url.starts_with("../") || s.url.starts_with("./") {
            continue;
        }
        if let Some(rel) = relative_url(&base, &s.url) {
            set_url(git, &s.name, &rel)?;
            changed.push(format!("{} → {rel}", s.path));
        }
    }
    if changed.is_empty() {
        return Ok("No URL can be relative: they are on other hosts or already relative".into());
    }
    Ok(format!("Made relative: {}. Commit .gitmodules to keep it", changed.join(", ")))
}

/// Writes the absolute URL for one submodule with a relative one.
pub fn make_absolute(git: &Git, name: &str) -> R {
    let s = list(git)
        .into_iter()
        .find(|s| &*s.name == name)
        .ok_or_else(|| OpError::Failed(format!("No submodule {name}")))?;
    let base =
        super_url(git).ok_or_else(|| OpError::Failed("This repository has no remote to resolve against".into()))?;
    let abs = resolve(&base, &s.url).ok_or_else(|| OpError::Failed(format!("Cannot resolve {}", s.url)))?;
    if abs == *s.url {
        return Ok(format!("{} is already absolute", s.path));
    }
    set_url(git, name, &abs)?;
    Ok(format!("{} now uses {abs}. Commit .gitmodules to keep it", s.path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn relative_urls_across_forms() {
        let gh = "https://github.com/alice/app.git";
        assert_eq!(relative_url(gh, "https://github.com/alice/lib.git").as_deref(), Some("../lib.git"));
        assert_eq!(relative_url(gh, "git@github.com:alice/lib.git").as_deref(), Some("../lib.git"));
        assert_eq!(relative_url(gh, "ssh://git@github.com/bob/lib").as_deref(), Some("../../bob/lib"));
        assert_eq!(
            relative_url("git@github.com:alice/app", "https://github.com/alice/tools/x.git").as_deref(),
            Some("../tools/x.git")
        );
        assert_eq!(relative_url(gh, "https://gitlab.com/alice/lib.git"), None, "other hosts stay absolute");
        assert_eq!(relative_url(gh, "C:/repos/lib"), None);
        // Round trips.
        for sub in
            ["https://github.com/alice/lib.git", "https://github.com/bob/lib", "https://github.com/alice/tools/x.git"]
        {
            let rel = relative_url(gh, sub).unwrap();
            assert_eq!(resolve(gh, &rel).as_deref(), Some(sub), "{rel}");
        }
        assert_eq!(
            resolve("git@github.com:alice/app.git", "../lib.git").as_deref(),
            Some("git@github.com:alice/lib.git")
        );
        assert_eq!(resolve(gh, "https://x/y").as_deref(), Some("https://x/y"));
    }

    fn repo(dir: &PathBuf) -> Git {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "protocol.file.allow", "always"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("f.txt"), "f").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        g
    }

    #[test]
    fn add_list_move_refuse_dirty_and_relative() {
        let base = std::env::temp_dir().join(format!("ffwd-submodules-{}", std::process::id()));
        let lib = repo(&base.join("lib"));
        let app = repo(&base.join("app"));
        let lib_url = lib.root.to_string_lossy().replace('\\', "/");
        // `git submodule add` from a local path needs file protocol allowed.
        app.output(&["config", "protocol.file.allow", "always"]).unwrap();
        let r = app
            .run_with(
                &["-c", "protocol.file.allow=always", "submodule", "add", "--", &lib_url, "libs/lib"],
                None,
                &[],
                1 << 20,
            )
            .map(|_| ());
        assert!(r.is_ok(), "{r:?}");
        app.output(&["commit", "-q", "-m", "add lib"]).unwrap();
        let s = list(&app);
        assert_eq!(s.len(), 1);
        assert_eq!((&*s[0].path, s[0].state), ("libs/lib", State::Clean));
        // A dirty submodule is not moved.
        std::fs::write(app.root.join("libs/lib/f.txt"), "changed").unwrap();
        let e = move_to(&app, "libs/lib", "vendor/lib").unwrap_err();
        assert!(e.to_string().contains("uncommitted"), "{e}");
        Git { root: app.root.join("libs/lib") }.output(&["checkout", "--", "f.txt"]).unwrap();
        // Moving into a folder that does not exist yet.
        move_to(&app, "libs/lib", "vendor/third/lib").unwrap();
        let s = list(&app);
        assert_eq!(&*s[0].path, "vendor/third/lib");
        assert!(app.root.join("vendor/third/lib/f.txt").exists());
        assert!(matches!(move_to(&app, "vendor/third/lib", "../out"), Err(OpError::Failed(_))));
        // Relative URLs: no remote means no conversion.
        assert!(make_relative(&app, None).is_err());
        app.output(&["remote", "add", "origin", "https://example.invalid/team/app.git"]).unwrap();
        app.output(&[
            "config",
            "-f",
            ".gitmodules",
            &format!("submodule.{}.url", s[0].name),
            "https://example.invalid/team/lib.git",
        ])
        .unwrap();
        // An unstaged .gitmodules edit blocks a move, with a clear reason.
        let gm = app.root.join(".gitmodules");
        std::fs::write(
            &gm,
            std::fs::read_to_string(&gm).unwrap()
                + "
",
        )
        .unwrap();
        let e = move_to(&app, "vendor/third/lib", "vendor/lib").unwrap_err();
        assert!(e.to_string().contains(".gitmodules has unstaged changes"), "{e}");
        let msg = make_relative(&app, None).unwrap();
        assert!(msg.contains("../lib.git"), "{msg}");
        assert_eq!(&*list(&app)[0].url, "../lib.git");
        make_absolute(&app, &s[0].name).unwrap();
        assert_eq!(&*list(&app)[0].url, "https://example.invalid/team/lib.git");
        let _ = std::fs::remove_dir_all(&base);
    }
}
