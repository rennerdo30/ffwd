//! Merge conflicts: parse a conflicted file's markers into text and
//! conflict blocks, write the user's choices back, take one side for the
//! whole file, and continue or abort the stopped operation. Git is never
//! asked to open an editor (`GIT_EDITOR=true`), so nothing can hang.

use crate::git::Git;
use std::process::Stdio;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Text(Vec<String>),
    Conflict(Block),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The marker labels: `HEAD`, the other branch or commit.
    pub ours_label: String,
    pub theirs_label: String,
    pub ours: Vec<String>,
    /// The common base (diff3 style), when the file has it.
    pub base: Option<Vec<String>>,
    pub theirs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Ours,
    Theirs,
    OursThenTheirs,
    TheirsThenOurs,
}

/// Splits a file into text and conflicts. Line endings are kept out of
/// the lines; `eol` says which one the file uses. An unterminated block
/// stays text, exactly as it was.
pub fn parse(text: &str) -> (Vec<Seg>, &'static str) {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let mut segs: Vec<Seg> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l.starts_with("<<<<<<<") {
            // Find the whole block before taking it.
            let mut j = i + 1;
            let (mut base_at, mut mid, mut end) = (None, None, None);
            while j < lines.len() {
                let m = lines[j];
                if m.starts_with("|||||||") && mid.is_none() && base_at.is_none() {
                    base_at = Some(j);
                } else if m.starts_with("=======") && mid.is_none() {
                    mid = Some(j);
                } else if m.starts_with(">>>>>>>") && mid.is_some() {
                    end = Some(j);
                    break;
                } else if m.starts_with("<<<<<<<") {
                    break;
                }
                j += 1;
            }
            if let (Some(mid), Some(end)) = (mid, end) {
                if !plain.is_empty() {
                    segs.push(Seg::Text(std::mem::take(&mut plain)));
                }
                let own = |a: usize, b: usize| lines[a..b].iter().map(|s| s.to_string()).collect::<Vec<_>>();
                let ours_end = base_at.unwrap_or(mid);
                segs.push(Seg::Conflict(Block {
                    ours_label: l.trim_start_matches('<').trim().to_string(),
                    theirs_label: lines[end].trim_start_matches('>').trim().to_string(),
                    ours: own(i + 1, ours_end),
                    base: base_at.map(|b| own(b + 1, mid)),
                    theirs: own(mid + 1, end),
                }));
                i = end + 1;
                continue;
            }
        }
        plain.push(l.to_string());
        i += 1;
    }
    if !plain.is_empty() {
        segs.push(Seg::Text(plain));
    }
    (segs, eol)
}

/// The file with every conflict replaced by its choice. `choices` has one
/// entry per conflict; `None` keeps that conflict's markers.
pub fn render(segs: &[Seg], choices: &[Option<Choice>], eol: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut k = 0;
    for s in segs {
        match s {
            Seg::Text(t) => out.extend(t.iter().cloned()),
            Seg::Conflict(b) => {
                match choices.get(k).copied().flatten() {
                    Some(Choice::Ours) => out.extend(b.ours.iter().cloned()),
                    Some(Choice::Theirs) => out.extend(b.theirs.iter().cloned()),
                    Some(Choice::OursThenTheirs) => out.extend(b.ours.iter().chain(&b.theirs).cloned()),
                    Some(Choice::TheirsThenOurs) => out.extend(b.theirs.iter().chain(&b.ours).cloned()),
                    None => {
                        out.push(format!("<<<<<<< {}", b.ours_label));
                        out.extend(b.ours.iter().cloned());
                        if let Some(base) = &b.base {
                            out.push("|||||||".into());
                            out.extend(base.iter().cloned());
                        }
                        out.push("=======".into());
                        out.extend(b.theirs.iter().cloned());
                        out.push(format!(">>>>>>> {}", b.theirs_label));
                    }
                }
                k += 1;
            }
        }
    }
    out.join(eol)
}

pub fn conflict_count(segs: &[Seg]) -> usize {
    segs.iter().filter(|s| matches!(s, Seg::Conflict(_))).count()
}

fn run(git: &Git, args: &[&str]) -> Result<String, String> {
    let out = git
        .command(args)
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not start git: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let msg = err
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("error:") || l.starts_with("fatal:"))
        .or_else(|| err.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("git reported an error");
    Err(msg.trim_start_matches("error: ").trim_start_matches("fatal: ").to_string())
}

/// Reads a conflicted file of the working tree (bounded).
pub fn read(git: &Git, path: &str) -> Result<String, String> {
    let p = git.root.join(path);
    let meta = std::fs::metadata(&p).map_err(|e| format!("{path}: {e}"))?;
    if meta.len() > 32 << 20 {
        return Err(format!("{path} is larger than 32 MB; resolve it in your editor"));
    }
    let b = std::fs::read(&p).map_err(|e| format!("{path}: {e}"))?;
    String::from_utf8(b).map_err(|_| format!("{path} is not text; take one side for the whole file"))
}

/// Writes the resolved text and stages it (marks the file resolved).
pub fn save_resolved(git: &Git, path: &str, text: &str) -> Result<String, String> {
    std::fs::write(git.root.join(path), text).map_err(|e| format!("{path}: {e}"))?;
    run(git, &["add", "--", path])?;
    Ok(format!("Resolved {path}"))
}

/// Takes one side for the whole file, then stages it.
pub fn take_side(git: &Git, path: &str, ours: bool) -> Result<String, String> {
    run(git, &["checkout", if ours { "--ours" } else { "--theirs" }, "--", path])?;
    run(git, &["add", "--", path])?;
    Ok(format!("Took {} for {path}", if ours { "ours" } else { "theirs" }))
}

/// Takes one side for every path in `paths` (FF-42). Stops at the first
/// failure and says how far it got.
pub fn take_side_all(git: &Git, paths: &[String], ours: bool) -> Result<String, String> {
    for (i, p) in paths.iter().enumerate() {
        take_side(git, p, ours).map_err(|e| format!("{e} (after {i} of {} files)", paths.len()))?;
    }
    let side = if ours { "ours" } else { "theirs" };
    Ok(format!("Took {side} for {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
}

/// Continues the stopped operation (`kind` from `refops::in_progress`).
pub fn continue_op(git: &Git, kind: &str) -> Result<String, String> {
    let args: &[&str] = match kind {
        "rebase" => &["rebase", "--continue"],
        "cherry-pick" => &["cherry-pick", "--continue"],
        "revert" => &["revert", "--continue"],
        _ => &["commit", "--no-edit"],
    };
    run(git, args)?;
    Ok(format!("Continued the {kind}"))
}

pub fn abort_op(git: &Git, kind: &str) -> Result<String, String> {
    let args: &[&str] = match kind {
        "rebase" => &["rebase", "--abort"],
        "cherry-pick" => &["cherry-pick", "--abort"],
        "revert" => &["revert", "--abort"],
        _ => &["merge", "--abort"],
    };
    run(git, args)?;
    Ok(format!("Aborted the {kind}; everything is as before it"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_render() {
        let t = "a\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> feature\nb\n<<<<<<< HEAD\nx\n||||||| base\nbase\n=======\ny\n>>>>>>> feature\n";
        let (segs, eol) = parse(t);
        assert_eq!(eol, "\n");
        assert_eq!(conflict_count(&segs), 2);
        let Seg::Conflict(b) = &segs[1] else { panic!() };
        assert_eq!((b.ours_label.as_str(), b.theirs_label.as_str()), ("HEAD", "feature"));
        assert_eq!(b.ours, vec!["ours 1", "ours 2"]);
        let Seg::Conflict(b2) = &segs[3] else { panic!() };
        assert_eq!(b2.base.as_deref(), Some(&["base".to_string()][..]));
        // Unresolved conflicts round-trip as they were (diff3 base too).
        assert_eq!(
            render(&segs, &[None, None], eol).replace("||||||| base", "|||||||"),
            t.replace("||||||| base", "|||||||")
        );
        let r = render(&segs, &[Some(Choice::Theirs), Some(Choice::OursThenTheirs)], eol);
        assert_eq!(r, "a\ntheirs\nb\nx\ny\n");
        // CRLF files stay CRLF; an unterminated block stays text.
        let (s2, e2) = parse("x\r\n<<<<<<< HEAD\r\nno end\r\n");
        assert_eq!(e2, "\r\n");
        assert_eq!(conflict_count(&s2), 0);
        assert_eq!(render(&s2, &[], e2), "x\r\n<<<<<<< HEAD\r\nno end\r\n");
    }

    /// A real merge conflict: resolve one file by choices, the other by
    /// taking a side, then continue; and abort brings everything back.
    #[test]
    fn resolve_continue_and_abort() {
        let dir = std::env::temp_dir().join(format!("ffwd-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "t"],
            &["config", "user.email", "t@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            g.output(a).unwrap();
        }
        let commit = |f: &str, t: &str, m: &str| {
            std::fs::write(dir.join(f), t).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        };
        commit("a.txt", "base\n", "base a");
        commit("b.txt", "base\n", "base b");
        g.output(&["switch", "-q", "-c", "feature"]).unwrap();
        commit("a.txt", "feature a\n", "f a");
        commit("b.txt", "feature b\n", "f b");
        g.output(&["switch", "-q", "main"]).unwrap();
        commit("a.txt", "main a\n", "m a");
        commit("b.txt", "main b\n", "m b");
        assert!(g.output(&["merge", "feature"]).is_err(), "the merge stops");
        assert_eq!(crate::refops::in_progress(&g), Some("merge"));
        // a.txt: both sides, ours first.
        let (segs, eol) = parse(&read(&g, "a.txt").unwrap());
        assert_eq!(conflict_count(&segs), 1);
        save_resolved(&g, "a.txt", &render(&segs, &[Some(Choice::OursThenTheirs)], eol)).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "main a\nfeature a\n");
        // b.txt: theirs for the whole file.
        take_side(&g, "b.txt", false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "feature b\n");
        continue_op(&g, "merge").unwrap();
        assert_eq!(crate::refops::in_progress(&g), None);
        let parents = String::from_utf8(g.output(&["log", "-1", "--format=%P"]).unwrap()).unwrap();
        assert_eq!(parents.split_whitespace().count(), 2, "a merge commit");
        // Abort: a second conflicting merge is undone completely.
        g.output(&["switch", "-q", "-c", "other", "HEAD~1"]).unwrap();
        commit("a.txt", "other a\n", "o a");
        g.output(&["switch", "-q", "main"]).unwrap();
        let before = g.output(&["rev-parse", "HEAD"]).unwrap();
        assert!(g.output(&["merge", "other"]).is_err());
        abort_op(&g, "merge").unwrap();
        assert_eq!(g.output(&["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(crate::refops::in_progress(&g), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
