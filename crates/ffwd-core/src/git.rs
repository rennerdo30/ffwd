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
        if !status.success() && out.len() < limit {
            return Err(GitError::Failed {
                args: args.join(" "),
                code: status.code(),
                stderr: String::from_utf8_lossy(&err).into_owned(),
            });
        }
        Ok(out)
    }

    pub fn output(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        self.output_limited(args, 256 * 1024 * 1024)
    }
}

/// Lossy UTF-8 into a boxed str: repository text is never trusted to be valid.
pub fn text(b: &[u8]) -> Box<str> {
    String::from_utf8_lossy(b).into()
}
