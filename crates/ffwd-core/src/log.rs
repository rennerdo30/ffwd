//! History streaming: `git log` is read incrementally and handed out in
//! batches, so the first screen of commits is ready long before the whole
//! history is parsed.

use crate::git::{Git, GitError, text};
use std::fmt;
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

/// A SHA-1 or SHA-256 object id, stored inline (no allocation per commit).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Oid {
    bytes: [u8; 32],
    len: u8,
}

impl Oid {
    pub fn from_hex(hex: &[u8]) -> Option<Oid> {
        if !(hex.len() == 40 || hex.len() == 64) {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (i, pair) in hex.as_chunks::<2>().0.iter().enumerate() {
            bytes[i] = (nib(pair[0])? << 4) | nib(pair[1])?;
        }
        Some(Oid { bytes, len: (hex.len() / 2) as u8 })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn short(&self, n: usize) -> String {
        let mut s = self.to_string();
        s.truncate(n);
        s
    }
}

fn nib(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.as_bytes() {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.short(10))
    }
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub id: Oid,
    pub parents: Vec<Oid>,
    pub author: Box<str>,
    pub email: Box<str>,
    /// Author time, seconds since the Unix epoch.
    pub time: i64,
    pub subject: Box<str>,
    pub has_body: bool,
}

const FIELD: u8 = 0x1f;
const RECORD: u8 = 0x1e;
pub const LOG_FORMAT: &str = "--format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%b";

/// Parses one log record (the bytes between two record separators).
/// Malformed records return `None` and are skipped, never panic.
pub fn parse_record(rec: &[u8]) -> Option<Commit> {
    let rec = rec.strip_suffix(b"\n").unwrap_or(rec);
    let mut f = rec.splitn(7, |&b| b == FIELD);
    let id = Oid::from_hex(f.next()?)?;
    let parents = f.next()?.split(|&b| b == b' ').filter(|p| !p.is_empty()).filter_map(Oid::from_hex).collect();
    let author = text(f.next()?);
    let email = text(f.next()?);
    let time = std::str::from_utf8(f.next()?).ok()?.trim().parse().unwrap_or(0);
    let subject = text(f.next()?);
    let has_body = f.next().is_some_and(|b| b.iter().any(|c| !c.is_ascii_whitespace()));
    Some(Commit { id, parents, author, email, time, subject, has_body })
}

/// Streams the history of all refs in topological order. `sink` receives
/// batches; the first batch is sent after `first` commits, later batches at
/// most every `interval`. The last call has `done = true`.
pub fn stream(
    git: &Git,
    first: usize,
    interval: Duration,
    mut sink: impl FnMut(Vec<Commit>, bool),
) -> Result<usize, GitError> {
    // Branches, remotes, tags and HEAD. Not `--all`: tools add
    // private refs (for example refs/t3/*) that are not part of the history
    // the user works with.
    let mut child = git.spawn(&[
        "log",
        "--branches",
        "--remotes",
        "--tags",
        "HEAD",
        "--topo-order",
        "--no-show-signature",
        LOG_FORMAT,
    ])?;
    let Some(out) = child.stdout.take() else {
        return Ok(0);
    };
    let mut rd = BufReader::with_capacity(256 * 1024, out);
    let mut batch = Vec::with_capacity(first);
    let mut total = 0usize;
    let mut sent_first = false;
    let mut last = Instant::now();
    let mut rec = Vec::with_capacity(512);
    // Records start with RECORD; read_until splits right before the next one.
    let _ = rd.read_until(RECORD, &mut rec);
    loop {
        rec.clear();
        let n = rd.read_until(RECORD, &mut rec).unwrap_or(0);
        if n == 0 {
            break;
        }
        let body = rec.strip_suffix(&[RECORD]).unwrap_or(&rec);
        // Bodies are only needed for the "has description" marker; huge
        // ones are cut at parse time by `splitn` and never stored.
        if let Some(c) = parse_record(body) {
            batch.push(c);
            total += 1;
        }
        let due = if sent_first { last.elapsed() >= interval && !batch.is_empty() } else { batch.len() >= first };
        if due {
            sink(std::mem::take(&mut batch), false);
            sent_first = true;
            last = Instant::now();
        }
    }
    sink(batch, true);
    let _ = child.wait();
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_and_rejects_garbage() {
        let rec = b"0123456789abcdef0123456789abcdef01234567\x1f89abcdef0123456789abcdef0123456789abcdef \x1fA U Thor\x1fa@x.y\x1f1700000000\x1fFix \xff thing\x1f\n";
        let c = parse_record(rec).expect("valid record");
        assert_eq!(c.parents.len(), 1);
        assert_eq!(&*c.author, "A U Thor");
        assert!(c.subject.contains('\u{FFFD}'));
        assert!(!c.has_body);
        assert!(parse_record(b"not a commit").is_none());
        assert!(parse_record(b"").is_none());
    }
}
