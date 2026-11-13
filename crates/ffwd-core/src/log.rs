//! History streaming: `git log` is read incrementally and handed out in
//! batches, so the first screen of commits is ready long before the whole
//! history is parsed.

use crate::git::{Git, GitError, text};
use std::fmt;
use std::io::{BufRead, BufReader, Read};
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
/// Each record ends with RECORD, so a record is complete as soon as its own
/// bytes arrive (git adds a newline after each one).
pub const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%b%x1e";

/// Parses one log record (the bytes between two record separators).
/// Malformed records return `None` and are skipped, never panic.
pub fn parse_record(rec: &[u8]) -> Option<Commit> {
    let rec = rec.trim_ascii_start();
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
    sink: impl FnMut(Vec<Commit>, bool),
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
    let total = stream_from(BufReader::with_capacity(256 * 1024, out), first, interval, sink);
    let _ = child.wait();
    Ok(total)
}

/// Longest record kept from `git log`. The fields we show come first; the
/// rest of an oversized record (a huge message body) is skipped unread.
pub const MAX_RECORD: usize = 16 * 1024;

/// Reads one record up to the next separator into `rec`, keeping at most
/// `MAX_RECORD` bytes. Returns `false` at end of input.
fn read_record(rd: &mut impl BufRead, rec: &mut Vec<u8>) -> bool {
    rec.clear();
    let mut any = false;
    loop {
        let buf = match rd.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return any,
        };
        if buf.is_empty() {
            return any;
        }
        any = true;
        let (chunk, found) = match memchr(RECORD, buf) {
            Some(i) => (&buf[..i], Some(i)),
            None => (buf, None),
        };
        let room = MAX_RECORD.saturating_sub(rec.len());
        rec.extend_from_slice(&chunk[..chunk.len().min(room)]);
        let used = found.map_or(buf.len(), |i| i + 1);
        rd.consume(used);
        if found.is_some() {
            return true;
        }
    }
}

fn memchr(b: u8, s: &[u8]) -> Option<usize> {
    s.iter().position(|&c| c == b)
}

/// The streaming core of [`stream`], over any reader.
///
/// The first batch goes out as soon as `first` commits are parsed or the
/// reader has nothing more buffered (the next read would wait on git), so
/// a small or slow history shows up without waiting for the end.
pub fn stream_from<R: Read>(
    mut rd: BufReader<R>,
    first: usize,
    interval: Duration,
    mut sink: impl FnMut(Vec<Commit>, bool),
) -> usize {
    let mut batch = Vec::with_capacity(first);
    let mut total = 0usize;
    let mut sent_first = false;
    let mut last = Instant::now();
    let mut rec = Vec::with_capacity(512);
    while read_record(&mut rd, &mut rec) {
        if let Some(c) = parse_record(&rec) {
            batch.push(c);
            total += 1;
        }
        // uffer() never reads, so this check cannot wait on git.
        let drained = rd.buffer().iter().all(|b| b.is_ascii_whitespace());
        let due = if sent_first {
            last.elapsed() >= interval && !batch.is_empty()
        } else {
            batch.len() >= first || (drained && !batch.is_empty())
        };
        if due {
            sink(std::mem::take(&mut batch), false);
            sent_first = true;
            last = Instant::now();
        }
    }
    sink(batch, true);
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn rec(n: u8, body_len: usize) -> Vec<u8> {
        let mut r = format!("{:040x}\x1f\x1fA\x1fa@x\x1f1700000000\x1fsubject {n}\x1f", n).into_bytes();
        r.extend(std::iter::repeat_n(b'x', body_len));
        r.extend_from_slice(b"\x1e\n");
        r
    }

    /// Serves one chunk per read and counts reads, standing in for a git
    /// pipe that delivers output in pieces.
    struct Chunks {
        parts: Vec<Vec<u8>>,
        reads: Arc<Mutex<usize>>,
    }

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            *self.reads.lock().unwrap() += 1;
            if self.parts.is_empty() {
                return Ok(0);
            }
            let p = &mut self.parts[0];
            let n = p.len().min(buf.len());
            buf[..n].copy_from_slice(&p[..n]);
            p.drain(..n);
            if p.is_empty() {
                self.parts.remove(0);
            }
            Ok(n)
        }
    }

    #[test]
    fn first_batch_does_not_wait_for_end_of_log() {
        let reads = Arc::new(Mutex::new(0));
        let mut part1 = rec(1, 0);
        part1.extend(rec(2, 0));
        let parts = vec![part1, rec(3, 0), rec(4, 0)];
        let rd = BufReader::new(Chunks { parts, reads: reads.clone() });
        let mut calls: Vec<(usize, usize, bool)> = Vec::new();
        let r2 = reads.clone();
        let total = stream_from(rd, 200, Duration::from_secs(60), |b, done| {
            calls.push((b.len(), *r2.lock().unwrap(), done));
        });
        assert_eq!(total, 4);
        // The two commits of the first chunk go out after the first read,
        // long before the 200-commit batch size or the end of the log.
        assert_eq!(calls[0], (2, 1, false));
        assert!(calls.last().unwrap().2);
    }

    #[test]
    fn oversized_records_are_capped_and_the_stream_continues() {
        let mut data = rec(1, 5 * MAX_RECORD);
        data.extend(rec(2, 0));
        let mut probe = BufReader::with_capacity(4096, &data[..]);
        let mut kept = Vec::new();
        assert!(read_record(&mut probe, &mut kept));
        assert_eq!(kept.len(), MAX_RECORD, "an oversized record is cut at the cap");
        let rd = BufReader::with_capacity(4096, &data[..]);
        let mut got = Vec::new();
        stream_from(rd, 1, Duration::ZERO, |b, _| got.extend(b));
        assert_eq!(got.len(), 2);
        assert_eq!(&*got[0].subject, "subject 1");
        assert!(got[0].has_body);
        assert_eq!(&*got[1].subject, "subject 2");
    }

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
