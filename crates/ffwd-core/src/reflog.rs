//! The reflog of HEAD (FF-43): where HEAD has been, newest first. It is
//! how lost commits come back (after a reset, a rebase, a dropped branch);
//! ffwd only reads it.

use crate::git::Git;
use crate::log::Oid;

/// The most entries read.
pub const MAX: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: Oid,
    /// `HEAD@{n}`.
    pub selector: Box<str>,
    /// What moved HEAD ("commit: …", "checkout: moving from a to b").
    pub message: Box<str>,
    /// Unix seconds.
    pub time: i64,
}

pub fn read(git: &Git) -> Result<Vec<Entry>, String> {
    let n = MAX.to_string();
    let out = git
        .output_limited(&["reflog", "show", "--format=%H%x1f%gd%x1f%gs%x1f%ct", "-n", &n, "HEAD", "--"], 8 << 20)
        .map_err(|e| e.to_string())?;
    Ok(parse(&String::from_utf8_lossy(&out)))
}

fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\x1f');
            let id = Oid::from_hex(f.next()?.trim().as_bytes())?;
            let selector = f.next()?.into();
            let message = f.next().unwrap_or("").into();
            let time = f.next().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
            Some(Entry { id, selector, message, time })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_reads() {
        let e = parse("0123456789abcdef0123456789abcdef01234567\x1fHEAD@{0}\x1fcommit: x\x1f1700000000\nbad line\n");
        assert_eq!(e.len(), 1);
        assert_eq!((&*e[0].selector, &*e[0].message, e[0].time), ("HEAD@{0}", "commit: x", 1_700_000_000));
        let dir = std::env::temp_dir().join(format!("ffwd-reflog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "ffwd test"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        for m in ["one", "two"] {
            std::fs::write(dir.join("a.txt"), m).unwrap();
            g.output(&["add", "."]).unwrap();
            g.output(&["commit", "-q", "-m", m]).unwrap();
        }
        g.output(&["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        let r = read(&g).unwrap();
        assert!(r[0].message.starts_with("reset: moving to"), "{:?}", r[0]);
        assert!(r[1].message.contains("two"), "the reset commit is still there: {:?}", r[1]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
