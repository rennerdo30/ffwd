//! Cloning and creating repositories (FF-44). Clone streams Git's
//! progress and can be cancelled; a cancelled or failed clone removes only
//! the folder it created itself, never one that existed before.

use crate::git::Git;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The folder name Git would pick for `url` (`…/lib.git` gives `lib`).
pub fn folder_for(url: &str) -> String {
    let u = url.trim().trim_end_matches(['/', '\\']).trim_end_matches(".git");
    let name = u.rsplit(['/', '\\', ':']).next().unwrap_or("");
    let clean: String = name.chars().filter(|c| !matches!(c, '<' | '>' | '"' | '|' | '?' | '*')).collect();
    if clean.is_empty() || clean == "." || clean == ".." { "repository".into() } else { clean }
}

/// Clones `url` into `dest` (which must not exist or be empty). Calls
/// `progress` with Git's latest progress line. Returns `dest`.
pub fn clone(url: &str, dest: &Path, cancel: &AtomicBool, mut progress: impl FnMut(String)) -> Result<PathBuf, String> {
    let url = url.trim();
    if url.is_empty() || url.starts_with('-') {
        return Err("Type the URL to clone".into());
    }
    let existed = dest.exists();
    if existed && std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} already exists and is not empty", dest.display()));
    }
    let parent = dest.parent().filter(|p| !p.as_os_str().is_empty()).ok_or("Choose a folder to clone into")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let dest_s = dest.to_string_lossy().to_string();
    let g = Git { root: parent.to_path_buf() };
    let (mut child, killer) =
        g.spawn_killable(&["clone", "--progress", "--", url, &dest_s]).map_err(|e| e.to_string())?;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    if let Some(mut se) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = se.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
    }
    let mut err = Vec::new();
    let mut line = Vec::new();
    let mut last = Instant::now();
    let mut killed: Option<Instant> = None;
    loop {
        if killed.is_none() && cancel.load(Ordering::Relaxed) {
            killer.kill();
            killed = Some(Instant::now());
        }
        if killed.is_some_and(|k| k.elapsed() > Duration::from_secs(2)) {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => {
                for b in chunk {
                    if err.len() < 64 * 1024 {
                        err.push(b);
                    }
                    if b == b'\r' || b == b'\n' {
                        if !line.is_empty() && last.elapsed() >= Duration::from_millis(150) {
                            progress(String::from_utf8_lossy(&line).trim().to_string());
                            last = Instant::now();
                        }
                        line.clear();
                    } else if line.len() < 512 {
                        line.push(b);
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    let cancelled = cancel.load(Ordering::Relaxed);
    if ok && !cancelled {
        return Ok(dest.to_path_buf());
    }
    // Remove only what this clone created.
    if !existed {
        let _ = std::fs::remove_dir_all(dest);
    }
    if cancelled {
        return Err("Clone cancelled".into());
    }
    let text = String::from_utf8_lossy(&err);
    let why = text
        .lines()
        .map(str::trim)
        .rfind(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .or(text.lines().map(str::trim).rfind(|l| !l.is_empty()))
        .unwrap_or("git could not clone it")
        .trim_start_matches("fatal: ")
        .trim_start_matches("error: ")
        .to_string();
    Err(why)
}

/// Creates a repository in `dir` (made if missing). An existing
/// repository is left as it is.
pub fn init(dir: &Path) -> Result<PathBuf, String> {
    if dir.as_os_str().is_empty() {
        return Err("Choose a folder".into());
    }
    if dir.join(".git").exists() {
        return Ok(dir.to_path_buf());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Git { root: dir.to_path_buf() }.output(&["init", "-q"]).map_err(|e| e.to_string())?;
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names() {
        assert_eq!(folder_for("https://github.com/rennerdo30/ffwd.git"), "ffwd");
        assert_eq!(folder_for("git@github.com:a/b-c.git/"), "b-c");
        assert_eq!(folder_for(r"E:\repos\lib"), "lib");
        assert_eq!(folder_for(".."), "repository");
    }

    #[test]
    fn clone_init_and_failures() {
        let base = std::env::temp_dir().join(format!("ffwd-clone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // A source repository with one commit.
        let src = init(&base.join("src")).unwrap();
        let g = Git { root: src.clone() };
        for a in [
            &["config", "user.name", "ffwd test"][..],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(src.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        assert_eq!(init(&src).unwrap(), src, "an existing repository is left alone");
        // Clone into a new nested folder.
        let dest = base.join("clones/copy");
        let url = src.to_string_lossy().replace('\\', "/");
        let out = clone(&url, &dest, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(out.join("a.txt").exists());
        // Not into a folder with files; a failure removes only its own folder.
        assert!(clone(&url, &dest, &AtomicBool::new(false), |_| {}).unwrap_err().contains("not empty"));
        let bad = base.join("bad");
        assert!(clone(&base.join("nowhere").to_string_lossy(), &bad, &AtomicBool::new(false), |_| {}).is_err());
        assert!(!bad.exists(), "the failed clone's own folder is removed");
        let kept = base.join("kept");
        std::fs::create_dir_all(&kept).unwrap();
        assert!(clone(&base.join("nowhere").to_string_lossy(), &kept, &AtomicBool::new(false), |_| {}).is_err());
        assert!(kept.exists(), "a folder that existed before stays");
        let _ = std::fs::remove_dir_all(&base);
    }
}
