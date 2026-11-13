use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[derive(Debug)]
pub enum GitError {
    Spawn(std::io::Error),
    Failed { args: String, code: Option<i32>, stderr: String },
    NotARepository(PathBuf),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::Spawn(e) => write!(f, "could not start git: {e}"),
            GitError::Failed { args, code, stderr } => {
                write!(f, "git {args} failed ({}): {}", code.map_or("signal".into(), |c| c.to_string()), stderr.trim())
            }
            GitError::NotARepository(p) => write!(f, "{} is not a Git repository", p.display()),
        }
    }
}

impl std::error::Error for GitError {}

/// A repository handle that runs `git` with a fixed working tree.
#[derive(Clone, Debug)]
pub struct Git {
    pub root: PathBuf,
}

impl Git {
    /// Resolves the top level of the repository that contains `path`.
    pub fn discover(path: &Path) -> Result<Git, GitError> {
        let probe = Git { root: path.to_path_buf() };
        let out = probe.output(&["rev-parse", "--show-toplevel"]).map_err(|_| GitError::NotARepository(path.into()))?;
        let top = String::from_utf8_lossy(&out).trim().to_string();
        if top.is_empty() {
            return Err(GitError::NotARepository(path.into()));
        }
        Ok(Git { root: PathBuf::from(top) })
    }

    /// Starts git so that [`Killable::kill`] ends it together with every
    /// helper it starts (ssh, `git-remote-*`), even orphaned ones.
    pub fn spawn_killable(&self, args: &[&str]) -> Result<(Child, Killable), GitError> {
        let mut cmd = self.command(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let child = cmd.spawn().map_err(GitError::Spawn)?;
        let k = Killable::new(&child);
        Ok((child, k))
    }

    /// Ends the process `pid` that ffwd started, with its children on
    /// Windows (git starts helpers such as `git-remote-https`).
    pub fn kill_tree(pid: u32) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        #[cfg(not(windows))]
        {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        // Stable, machine-readable output whatever the user's config says.
        cmd.args(["-c", "color.ui=false", "-c", "core.quotepath=false", "-c", "log.showSignature=false"]);
        cmd.args(args).current_dir(&self.root).stdin(Stdio::null());
        cmd.env("GIT_TERMINAL_PROMPT", "0").env("GIT_OPTIONAL_LOCKS", "0").env("LC_ALL", "C");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // A GUI process must not flash a console window per git call.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    pub fn spawn(&self, args: &[&str]) -> Result<Child, GitError> {
        self.command(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(GitError::Spawn)
    }

    /// Runs git to completion and returns stdout. Output above `limit` bytes
    /// is cut, so a pathological repository cannot exhaust memory.
    pub fn output_limited(&self, args: &[&str], limit: usize) -> Result<Vec<u8>, GitError> {
        let t = std::time::Instant::now();
        let mut child = self.spawn(args)?;
        // stderr drains on its own thread so a chatty git can never block on
        // a full pipe while we read stdout.
        let err_reader = child.stderr.take().map(|se| {
            std::thread::spawn(move || {
                let mut err = Vec::new();
                let _ = se.take(64 * 1024).read_to_end(&mut err);
                err
            })
        });
        let mut out = Vec::new();
        if let Some(so) = child.stdout.take() {
            let _ = so.take(limit as u64).read_to_end(&mut out);
        }
        let err = err_reader.and_then(|t| t.join().ok()).unwrap_or_default();
        // Dropping stdout early may make git exit on SIGPIPE; that is fine
        // once we have read as much as we accept.
        let status = child.wait().map_err(GitError::Spawn)?;
        note(args, status.code(), &err, t);
        if !status.success() && out.len() < limit {
            return Err(GitError::Failed {
                args: args.join(" "),
                code: status.code(),
                stderr: String::from_utf8_lossy(&err).into_owned(),
            });
        }
        Ok(out)
    }

    /// Runs git with `input` on standard input; exit codes in `ok` count
    /// as success (`git diff --no-index` exits 1 when files differ).
    /// Returns stdout, capped at `limit` bytes.
    pub fn run_with(&self, args: &[&str], input: Option<&[u8]>, ok: &[i32], limit: usize) -> Result<Vec<u8>, GitError> {
        let t = std::time::Instant::now();
        let mut cmd = self.command(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        if input.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd.spawn().map_err(GitError::Spawn)?;
        let writer = match (child.stdin.take(), input) {
            (Some(mut si), Some(data)) => {
                let data = data.to_vec();
                Some(std::thread::spawn(move || {
                    use std::io::Write;
                    let _ = si.write_all(&data);
                }))
            }
            _ => None,
        };
        let err_reader = child.stderr.take().map(|se| {
            std::thread::spawn(move || {
                let mut err = Vec::new();
                let _ = se.take(64 * 1024).read_to_end(&mut err);
                err
            })
        });
        let mut out = Vec::new();
        if let Some(so) = child.stdout.take() {
            let mut so = so;
            let _ = (&mut so).take(limit as u64).read_to_end(&mut out);
            let _ = std::io::copy(&mut so, &mut std::io::sink());
        }
        if let Some(w) = writer {
            let _ = w.join();
        }
        let err = err_reader.and_then(|t| t.join().ok()).unwrap_or_default();
        let status = child.wait().map_err(GitError::Spawn)?;
        let code = status.code();
        note(args, code, &err, t);
        if status.success() || code.is_some_and(|c| ok.contains(&c)) {
            return Ok(out);
        }
        Err(GitError::Failed { args: args.join(" "), code, stderr: String::from_utf8_lossy(&err).into_owned() })
    }

    pub fn output(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        self.output_limited(args, 256 * 1024 * 1024)
    }
}

/// One git command an operation ran, for the activity log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    /// The arguments after `git` (without ffwd's fixed `-c` options).
    pub args: String,
    pub code: Option<i32>,
    /// What git wrote to stderr, capped.
    pub stderr: String,
    pub ms: u32,
}

thread_local! {
    static RECORD: std::cell::RefCell<Option<Vec<Ran>>> = const { std::cell::RefCell::new(None) };
}

/// At most this many commands and this much stderr per command are kept.
const MAX_RAN: usize = 200;
const MAX_RAN_STDERR: usize = 16 * 1024;

/// Runs `f` and returns the git commands it ran on this thread (an
/// operation's worker), for the activity log. Nests: an inner recording
/// does not hide commands from the outer one's caller.
pub fn recording<T>(f: impl FnOnce() -> T) -> (T, Vec<Ran>) {
    let outer = RECORD.with(|r| r.replace(Some(Vec::new())));
    let out = f();
    let ran = RECORD.with(|r| r.replace(outer)).unwrap_or_default();
    if !ran.is_empty() {
        RECORD.with(|r| {
            if let Some(v) = r.borrow_mut().as_mut() {
                v.extend(ran.iter().cloned());
                v.truncate(MAX_RAN);
            }
        });
    }
    (out, ran)
}

/// Records a finished command when this thread is recording.
pub fn note(args: &[&str], code: Option<i32>, stderr: &[u8], t: std::time::Instant) {
    RECORD.with(|r| {
        if let Some(v) = r.borrow_mut().as_mut()
            && v.len() < MAX_RAN
        {
            let e = &stderr[..stderr.len().min(MAX_RAN_STDERR)];
            v.push(Ran {
                args: args.join(" "),
                code,
                stderr: String::from_utf8_lossy(e).into_owned(),
                ms: t.elapsed().as_millis().min(u32::MAX as u128) as u32,
            });
        }
    });
}

/// Lossy UTF-8 into a boxed str: repository text is never trusted to be valid.
pub fn text(b: &[u8]) -> Box<str> {
    String::from_utf8_lossy(b).into()
}

/// Ends a git process ffwd started with everything it started: a job
/// object on Windows, the process group elsewhere.
pub struct Killable {
    pid: u32,
    /// The job handle (0 when none could be made; then a tree kill).
    #[cfg(windows)]
    job: isize,
}

impl Killable {
    fn new(child: &Child) -> Killable {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
            // SAFETY: plain Win32 calls on a handle we own and the child's
            // live process handle; failures fall back to a tree kill.
            let job = unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if !job.is_null() && AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
                    CloseHandle(job);
                    std::ptr::null_mut()
                } else {
                    job
                }
            };
            Killable { pid: child.id(), job: job as isize }
        }
        #[cfg(not(windows))]
        Killable { pid: child.id() }
    }

    pub fn kill(&self) {
        #[cfg(windows)]
        {
            if self.job != 0 {
                // SAFETY: the handle is ours and still open (closed on drop).
                unsafe {
                    windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job as _, 1);
                }
                return;
            }
            Git::kill_tree(self.pid);
        }
        #[cfg(not(windows))]
        {
            // The whole process group (git was started as its leader).
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{}", self.pid)])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[cfg(windows)]
impl Drop for Killable {
    fn drop(&mut self) {
        if self.job != 0 {
            // SAFETY: closing our own handle once; the processes keep running
            // (no kill-on-close), only an explicit kill ends them.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.job as _);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_collects_the_commands_of_this_thread() {
        let g = Git { root: std::env::temp_dir() };
        let ((), ran) = recording(|| {
            let _ = g.output(&["--version"]);
            let _ = g.run_with(&["rev-parse", "--verify", "--quiet", "no-such-ref-ffwd"], None, &[], 1024);
        });
        assert_eq!(ran.len(), 2);
        assert_eq!(ran[0].args, "--version");
        assert_eq!(ran[0].code, Some(0));
        assert_ne!(ran[1].code, Some(0));
        // Not recording: nothing is kept.
        let _ = g.output(&["--version"]);
        let ((), again) = recording(|| {});
        assert!(again.is_empty());
    }
}
