//! Child processes for agents and scripts: streamed output, a hard time
//! limit, cancel, and killing the whole process tree of exactly the
//! process ffwd started (never anything picked by name).

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

/// A fully prepared process: program, arguments, folder and environment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Start from an empty environment and set only `env`.
    pub clear_env: bool,
    pub env: Vec<(String, String)>,
    /// Written to standard input, which is then closed.
    pub stdin: Option<Vec<u8>>,
}

impl Launch {
    /// The command as one line, for the run log.
    pub fn display(&self) -> String {
        let q = |s: &str| {
            if s.is_empty() || s.contains(char::is_whitespace) || s.contains(['"', '\'']) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.to_string()
            }
        };
        let mut v = vec![q(&self.program)];
        v.extend(self.args.iter().map(|a| {
            // Long arguments (a prompt, a sandbox profile) are shortened.
            if a.chars().count() > 120 { q(&format!("{}…", a.chars().take(117).collect::<String>())) } else { q(a) }
        }));
        v.join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    /// Ended by a signal (Unix).
    Signal,
    TimedOut,
    Cancelled,
    SpawnFailed(String),
}

impl Exit {
    pub fn ok(&self) -> bool {
        *self == Exit::Code(0)
    }

    pub fn describe(&self) -> String {
        match self {
            Exit::Code(c) => format!("exit code {c}"),
            Exit::Signal => "ended by a signal".into(),
            Exit::TimedOut => "stopped at the time limit".into(),
            Exit::Cancelled => "cancelled".into(),
            Exit::SpawnFailed(e) => format!("could not start: {e}"),
        }
    }
}

/// PIDs of the processes ffwd started and that still run.
static RUNNING: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Kills every agent and script ffwd started (at exit). Only PIDs ffwd
/// recorded itself are touched.
pub fn kill_all() {
    let pids: Vec<u32> = RUNNING.lock().map(|v| v.clone()).unwrap_or_default();
    for pid in pids {
        kill_tree(pid, true);
    }
}

fn hidden(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Kills `pid` and its descendants: `taskkill /T` on Windows, the process
/// group on Unix (agents start in their own group).
fn kill_tree(pid: u32, force: bool) {
    #[cfg(windows)]
    {
        let _ = force;
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        hidden(&mut c);
        let _ = c.status();
    }
    #[cfg(not(windows))]
    {
        let sig = if force { "-KILL" } else { "-TERM" };
        let _ = Command::new("kill")
            .args([sig, "--", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Finds `program` on `PATH` the way a shell would, including `.cmd` and
/// `.bat` shims on Windows (Rust's standard library then quotes the
/// arguments safely for them).
pub fn resolve_program(program: &str, path_var: Option<&str>) -> String {
    if !cfg!(windows) || program.contains(['/', '\\']) || std::path::Path::new(program).extension().is_some() {
        return program.to_string();
    }
    let Some(path) = path_var.map(str::to_string).or_else(|| std::env::var("PATH").ok()) else {
        return program.to_string();
    };
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    for dir in path.split(';').filter(|d| !d.is_empty()) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let p = std::path::Path::new(dir).join(format!("{program}{}", ext.to_ascii_lowercase()));
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    program.to_string()
}

/// Where `program` would be found: a path that exists, or a match on
/// `PATH` (with `PATHEXT` on Windows).
pub fn find_program(program: &str) -> Option<PathBuf> {
    if program.contains(['/', '\\']) {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    let mut exts = vec![String::new()];
    if cfg!(windows) {
        let pe = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        exts.extend(pe.split(';').filter(|e| !e.is_empty()).map(|e| e.to_ascii_lowercase()));
    }
    let bare = std::path::Path::new(program).extension().is_none();
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            // On Windows an extension-less file (an npm shell shim) does not
            // run; only PATHEXT matches count.
            if cfg!(windows) && ext.is_empty() && bare {
                continue;
            }
            let p = dir.join(format!("{program}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

const MAX_LINE: usize = 4000;

fn reader(mut r: impl Read + Send + 'static, tx: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut line = Vec::new();
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &b in &buf[..n] {
                if b == b'\n' || b == b'\r' {
                    if !line.is_empty() {
                        let _ = tx.send(String::from_utf8_lossy(&line).into_owned());
                        line.clear();
                    }
                } else if line.len() < MAX_LINE {
                    line.push(b);
                }
            }
        }
        if !line.is_empty() {
            let _ = tx.send(String::from_utf8_lossy(&line).into_owned());
        }
    });
}

/// Runs `l` to its end, the time limit or a cancel, sending its output in
/// batches of lines (at most five times per second). Never panics.
pub fn run(l: &Launch, timeout: Duration, cancel: &AtomicBool, mut on_lines: impl FnMut(Vec<String>)) -> Exit {
    let path_var = l.env.iter().find(|(k, _)| k.eq_ignore_ascii_case("PATH")).map(|(_, v)| v.as_str());
    let mut cmd = Command::new(resolve_program(&l.program, if l.clear_env { path_var } else { None }));
    cmd.args(&l.args).current_dir(&l.cwd).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if l.stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    if l.clear_env {
        cmd.env_clear();
    }
    cmd.envs(l.env.iter().map(|(k, v)| (k, v)));
    hidden(&mut cmd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Exit::SpawnFailed(format!("{}: {e}", l.program)),
    };
    let pid = child.id();
    if let Ok(mut v) = RUNNING.lock() {
        v.push(pid);
    }
    if let (Some(mut si), Some(data)) = (child.stdin.take(), l.stdin.clone()) {
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = si.write_all(&data);
        });
    }
    let (tx, rx) = mpsc::channel();
    if let Some(o) = child.stdout.take() {
        reader(o, tx.clone());
    }
    if let Some(e) = child.stderr.take() {
        reader(e, tx.clone());
    }
    drop(tx);
    let t0 = Instant::now();
    let mut batch = Vec::new();
    let mut last_flush = Instant::now();
    let mut stopped: Option<Exit> = None;
    let mut ended_at: Option<Instant> = None;
    let mut pipes_closed = false;
    let status = loop {
        if pipes_closed {
            // The process closed its output but may still run: keep
            // watching the time limit and cancel, never block on wait().
            std::thread::sleep(Duration::from_millis(100));
        } else {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => batch.push(line),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => pipes_closed = true,
            }
        }
        if !batch.is_empty() && (last_flush.elapsed() >= Duration::from_millis(200) || batch.len() >= 500) {
            on_lines(std::mem::take(&mut batch));
            last_flush = Instant::now();
        }
        if stopped.is_none() {
            let why = if cancel.load(Ordering::Relaxed) {
                Some(Exit::Cancelled)
            } else if t0.elapsed() >= timeout {
                Some(Exit::TimedOut)
            } else {
                None
            };
            if let Some(why) = why {
                kill_tree(pid, false);
                // Give the agent a moment to stop cleanly, then force it.
                let until = Instant::now() + Duration::from_secs(3);
                while Instant::now() < until && matches!(child.try_wait(), Ok(None)) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                kill_tree(pid, true);
                let _ = child.kill();
                stopped = Some(why);
            }
        }
        match child.try_wait() {
            Ok(Some(s)) => {
                // A descendant may keep the pipes open; read for at most two
                // more seconds after the process itself ended.
                let at = *ended_at.get_or_insert_with(Instant::now);
                if pipes_closed || at.elapsed() >= Duration::from_secs(2) {
                    break Some(s);
                }
            }
            Ok(None) => {}
            Err(_) => break None,
        }
    };
    while let Ok(line) = rx.try_recv() {
        batch.push(line);
    }
    if !batch.is_empty() {
        on_lines(batch);
    }
    if let Ok(mut v) = RUNNING.lock() {
        v.retain(|p| *p != pid);
    }
    if let Some(s) = stopped {
        return s;
    }
    match status.and_then(|s| s.code()) {
        Some(c) => Exit::Code(c),
        None => Exit::Signal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str) -> Launch {
        let (program, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
        Launch {
            program: program.into(),
            args: vec![flag.into(), script.into()],
            cwd: std::env::temp_dir(),
            ..Launch::default()
        }
    }

    #[test]
    fn streams_output_and_exit_code() {
        let mut lines = Vec::new();
        let exit = run(&shell("echo one&& echo two&& exit 3"), Duration::from_secs(30), &AtomicBool::new(false), |b| {
            lines.extend(b)
        });
        assert_eq!(exit, Exit::Code(3));
        let lines: Vec<String> = lines.iter().map(|l| l.trim().to_string()).collect();
        assert_eq!(lines, vec!["one", "two"]);
    }

    #[test]
    fn cancel_stops_a_long_process_quickly() {
        let cancel = AtomicBool::new(true);
        let long = if cfg!(windows) { "ping -n 30 127.0.0.1 >NUL" } else { "sleep 30" };
        let t = Instant::now();
        let exit = run(&shell(long), Duration::from_secs(60), &cancel, |_| {});
        assert_eq!(exit, Exit::Cancelled);
        assert!(t.elapsed() < Duration::from_secs(15), "{:?}", t.elapsed());
    }

    #[test]
    fn time_limit() {
        let long = if cfg!(windows) { "ping -n 30 127.0.0.1 >NUL" } else { "sleep 30" };
        let exit = run(&shell(long), Duration::from_millis(300), &AtomicBool::new(false), |_| {});
        assert_eq!(exit, Exit::TimedOut);
    }

    #[test]
    fn missing_program_is_reported() {
        let l = Launch { program: "ffwd-no-such-program-xyz".into(), cwd: std::env::temp_dir(), ..Launch::default() };
        assert!(matches!(run(&l, Duration::from_secs(5), &AtomicBool::new(false), |_| {}), Exit::SpawnFailed(_)));
    }
}
