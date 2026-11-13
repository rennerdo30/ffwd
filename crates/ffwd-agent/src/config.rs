//! The `[agents]` settings. ffwd's settings file stores every value as
//! text; this module turns them into a typed [`Config`] with safe defaults
//! and reports what it could not read instead of guessing.

use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxKind {
    /// Bubblewrap on Linux, `sandbox-exec` on macOS, WSL + bubblewrap on
    /// Windows.
    Auto,
    Bubblewrap,
    /// Windows: the agent runs inside WSL, under bubblewrap.
    Wsl,
    /// macOS `sandbox-exec` with a generated profile.
    MacOs,
    /// The user's own wrapper (`sandbox_command`).
    Custom,
    /// No sandbox. Refused unless `allow_unsandboxed = true`.
    None,
}

impl SandboxKind {
    pub fn parse(s: &str) -> Option<SandboxKind> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => SandboxKind::Auto,
            "bwrap" | "bubblewrap" => SandboxKind::Bubblewrap,
            "wsl" => SandboxKind::Wsl,
            "macos" | "sandbox-exec" | "seatbelt" => SandboxKind::MacOs,
            "custom" => SandboxKind::Custom,
            "none" | "off" => SandboxKind::None,
            _ => return None,
        })
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            SandboxKind::Auto => "auto",
            SandboxKind::Bubblewrap => "bwrap",
            SandboxKind::Wsl => "wsl",
            SandboxKind::MacOs => "macos",
            SandboxKind::Custom => "custom",
            SandboxKind::None => "none",
        }
    }

    /// What `Auto` means on this platform.
    pub fn resolve(self) -> SandboxKind {
        match self {
            SandboxKind::Auto if cfg!(target_os = "macos") => SandboxKind::MacOs,
            SandboxKind::Auto if cfg!(windows) => SandboxKind::Wsl,
            SandboxKind::Auto => SandboxKind::Bubblewrap,
            k => k,
        }
    }

    pub fn label(self) -> &'static str {
        match self.resolve() {
            SandboxKind::Bubblewrap => "bubblewrap",
            SandboxKind::Wsl => "WSL + bubblewrap",
            SandboxKind::MacOs => "macOS sandbox",
            SandboxKind::Custom => "custom wrapper",
            _ => "none",
        }
    }
}

/// What the agent sees of the user's home folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeAccess {
    /// An empty, temporary home (the default): no keys, no shell history.
    Private,
    /// The real home, read-only (agents that need their login or config).
    Read,
    /// The real home, writable.
    Write,
}

/// What the agent may do with the repository's Git data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitAccess {
    /// The agent sees Git read-only and edits files only; ffwd commits its
    /// changes when it ends (outside the sandbox, with the user's hooks).
    Read,
    /// The agent may commit itself. It can then also write other refs of
    /// the repository, because all worktrees share one object store.
    Write,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub enabled: bool,
    /// A built-in tool (`agents.preset`): its command and its grants.
    pub preset: Option<&'static crate::presets::Preset>,
    /// Add the preset's config folders and key variables (default on).
    pub preset_grants: bool,
    /// The agent's command line. Placeholders: `{prompt}`, `{prompt_file}`,
    /// `{worktree}`, `{branch}`, `{base}`, `{repo}`, `{run_id}`.
    pub command: String,
    /// Also write the prompt to the agent's standard input.
    pub prompt_stdin: bool,
    /// Wraps the user's prompt; `{prompt}` is the text typed in ffwd.
    pub prompt_template: String,
    /// A file whose text becomes the template (wins over `prompt_template`).
    pub prompt_template_file: Option<PathBuf>,
    pub sandbox: SandboxKind,
    /// Wrapper for `sandbox = "custom"`; `{command}` stands for the whole
    /// agent command line.
    pub sandbox_command: String,
    pub allow_unsandboxed: bool,
    pub network: bool,
    pub git_access: GitAccess,
    pub home_access: HomeAccess,
    /// Extra folders the agent may read. `~/…` is the home of wherever
    /// the agent runs (inside WSL: the Linux home).
    pub read_paths: Vec<PathBuf>,
    /// Extra folders the agent may write (for example its own config).
    pub write_paths: Vec<PathBuf>,
    /// Host environment variables passed through by name.
    pub env_pass: Vec<String>,
    /// Pass the whole host environment (only with `sandbox = "none"` or
    /// `"custom"`; the bubblewrap modes always start from a clean one).
    pub env_inherit: bool,
    /// Extra `KEY=value` variables.
    pub env: Vec<(String, String)>,
    /// Where worktrees go; empty means `<repo parent>/.ffwd-agents/<repo>`.
    pub worktree_root: Option<PathBuf>,
    pub branch_prefix: String,
    /// What the agent's branch starts from.
    pub base: String,
    pub timeout: Duration,
    pub max_parallel: usize,
    /// Shell commands run at fixed points. Empty means skipped.
    pub setup_script: String,
    pub after_script: String,
    pub on_done_script: String,
    /// The shell for scripts, for example `pwsh -NoProfile -Command`.
    /// Empty: `sh -c` (in a sandbox, Linux, macOS) or `cmd /C` (Windows
    /// without a sandbox).
    pub script_shell: String,
    /// Run setup and after scripts inside the agent's sandbox.
    pub scripts_in_sandbox: bool,
    pub script_timeout: Duration,
    /// ffwd commits what the agent left uncommitted when it ends.
    pub auto_commit: bool,
    /// `{title}` is the prompt's first line, `{prompt}` the whole prompt.
    pub commit_message: String,
    pub commit_no_verify: bool,
    /// WSL distribution (`wsl -d`); empty: the default one.
    pub wsl_distro: String,
    /// Windows path prefix → WSL path, for drives WSL does not mount under
    /// `/mnt/<letter>` (for example a `subst` drive). Longest prefix wins.
    pub wsl_path_map: Vec<(String, String)>,
    /// The bubblewrap program (inside WSL on Windows).
    pub bwrap_path: String,
    /// Lines of agent output kept in memory per run.
    pub log_lines: usize,
}

pub const DEFAULT_ENV_PASS: &str = "PATH;HOME;USER;USERNAME;LOGNAME;LANG;LC_ALL;LC_CTYPE;TERM;TZ;TMPDIR;TEMP;TMP;SYSTEMROOT;SYSTEMDRIVE;WINDIR;COMSPEC;PATHEXT;USERPROFILE;APPDATA;LOCALAPPDATA;PROGRAMDATA;PROGRAMFILES;HOMEDRIVE;HOMEPATH";

impl Default for Config {
    fn default() -> Config {
        Config::from_lookup(|_| None).0
    }
}

fn split_list(s: &str, sep: char) -> Vec<String> {
    s.split(sep).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()
}

fn flag(v: Option<String>, default: bool, key: &str, errors: &mut Vec<String>) -> bool {
    match v.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
        None => default,
        Some(s) if s.is_empty() => default,
        Some(s) if matches!(s.as_str(), "true" | "on" | "yes" | "1") => true,
        Some(s) if matches!(s.as_str(), "false" | "off" | "no" | "0") => false,
        Some(s) => {
            errors.push(format!("agents.{key}: \"{s}\" is not true or false; using {default}"));
            default
        }
    }
}

fn number(v: Option<String>, default: u64, min: u64, max: u64, key: &str, errors: &mut Vec<String>) -> u64 {
    match v.as_deref().map(str::trim) {
        None | Some("") => default,
        Some(s) => match s.parse::<u64>() {
            Ok(n) => n.clamp(min, max),
            Err(_) => {
                errors.push(format!("agents.{key}: \"{s}\" is not a number; using {default}"));
                default
            }
        },
    }
}

impl Config {
    /// Builds the config from `get("command")`, `get("sandbox")`, … (keys
    /// without the `agents.` prefix). Returns what it could not read.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> (Config, Vec<String>) {
        let mut errors = Vec::new();
        let text =
            |k: &str, d: &str| get(k).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or(d.into());
        let sandbox = match get("sandbox") {
            None => SandboxKind::Auto,
            Some(s) => SandboxKind::parse(&s).unwrap_or_else(|| {
                errors.push(format!(
                    "agents.sandbox: \"{s}\" is unknown (auto, bwrap, wsl, macos, custom, none); using auto"
                ));
                SandboxKind::Auto
            }),
        };
        let git_access = match get("git_access").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("read") => GitAccess::Read,
            Some("write") => GitAccess::Write,
            Some(s) => {
                errors.push(format!("agents.git_access: \"{s}\" is unknown (read, write); using read"));
                GitAccess::Read
            }
        };
        let home_access = match get("home_access").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("private") => HomeAccess::Private,
            Some("read") => HomeAccess::Read,
            Some("write") => HomeAccess::Write,
            Some(s) => {
                errors.push(format!("agents.home_access: \"{s}\" is unknown (private, read, write); using private"));
                HomeAccess::Private
            }
        };
        let mut wsl_path_map = Vec::new();
        for item in split_list(&get("wsl_path_map").unwrap_or_default(), '|') {
            match item.split_once("=>") {
                Some((a, b)) if !a.trim().is_empty() && b.trim().starts_with('/') => {
                    wsl_path_map.push((a.trim().to_string(), b.trim().to_string()))
                }
                _ => errors.push(format!("agents.wsl_path_map: \"{item}\" is not WINDOWS_PREFIX=>/wsl/path")),
            }
        }
        let mut env = Vec::new();
        for item in split_list(&get("env").unwrap_or_default(), '|') {
            match item.split_once('=') {
                Some((k, v)) if !k.trim().is_empty() && !k.contains(char::is_whitespace) => {
                    env.push((k.trim().to_string(), v.to_string()))
                }
                _ => errors.push(format!("agents.env: \"{item}\" is not KEY=value")),
            }
        }
        let paths = |k: &str| -> Vec<PathBuf> {
            split_list(&get(k).unwrap_or_default(), ';').into_iter().map(PathBuf::from).collect()
        };
        let preset = match get("preset").map(|s| s.trim().to_ascii_lowercase()) {
            None => None,
            Some(s) if s.is_empty() || s == "custom" || s == "none" => None,
            Some(s) => {
                let p = crate::presets::find(&s);
                if p.is_none() {
                    errors.push(format!(
                        "agents.preset: \"{s}\" is unknown (custom, claude, codex, antigravity, opencode, muse)"
                    ));
                }
                p
            }
        };
        let preset_grants = flag(get("preset_grants"), true, "preset_grants", &mut errors);
        let mut write_paths = paths("write_paths");
        let mut env_pass = split_list(&text("env_pass", DEFAULT_ENV_PASS), ';');
        if let (Some(p), true) = (preset, preset_grants) {
            for h in p.home_write {
                if !write_paths.iter().any(|w| w == &PathBuf::from(h)) {
                    write_paths.push(PathBuf::from(h));
                }
            }
            for e in p.env_pass {
                if !env_pass.iter().any(|x| x == e) {
                    env_pass.push(e.to_string());
                }
            }
        }
        let c = Config {
            enabled: flag(get("enabled"), false, "enabled", &mut errors),
            preset,
            preset_grants,
            command: text("command", preset.map_or("", |p| p.command)),
            prompt_stdin: flag(get("prompt_stdin"), false, "prompt_stdin", &mut errors),
            prompt_template: text("prompt_template", "{prompt}"),
            prompt_template_file: get("prompt_template_file").filter(|s| !s.trim().is_empty()).map(PathBuf::from),
            sandbox,
            sandbox_command: text("sandbox_command", ""),
            allow_unsandboxed: flag(get("allow_unsandboxed"), false, "allow_unsandboxed", &mut errors),
            network: flag(get("network"), true, "network", &mut errors),
            git_access,
            home_access,
            read_paths: paths("read_paths"),
            write_paths,
            env_pass,
            env_inherit: flag(get("env_inherit"), false, "env_inherit", &mut errors),
            env,
            worktree_root: get("worktree_root").filter(|s| !s.trim().is_empty()).map(|s| PathBuf::from(s.trim())),
            branch_prefix: text("branch_prefix", "agent/"),
            base: text("base", "HEAD"),
            timeout: Duration::from_secs(
                60 * number(get("timeout_minutes"), 60, 1, 24 * 60, "timeout_minutes", &mut errors),
            ),
            max_parallel: number(get("max_parallel"), 2, 1, 16, "max_parallel", &mut errors) as usize,
            setup_script: text("setup_script", ""),
            after_script: text("after_script", ""),
            on_done_script: text("on_done_script", ""),
            script_shell: text("script_shell", ""),
            scripts_in_sandbox: flag(get("scripts_in_sandbox"), true, "scripts_in_sandbox", &mut errors),
            script_timeout: Duration::from_secs(
                60 * number(get("script_timeout_minutes"), 15, 1, 24 * 60, "script_timeout_minutes", &mut errors),
            ),
            auto_commit: flag(get("auto_commit"), true, "auto_commit", &mut errors),
            commit_message: text("commit_message", "Agent: {title}"),
            commit_no_verify: flag(get("commit_no_verify"), false, "commit_no_verify", &mut errors),
            wsl_distro: text("wsl_distro", ""),
            wsl_path_map,
            bwrap_path: text("bwrap_path", "bwrap"),
            log_lines: number(get("log_lines"), 5000, 100, 200_000, "log_lines", &mut errors) as usize,
        };
        if c.sandbox == SandboxKind::Custom && !c.sandbox_command.contains("{command}") {
            errors.push("agents.sandbox_command must contain {command} for sandbox = \"custom\"".into());
        }
        (c, errors)
    }

    /// Why a run cannot start with this config, in one sentence.
    pub fn refusal(&self) -> Option<String> {
        if !self.enabled {
            return Some("Agents are off. Turn them on in Settings → Agents (experimental).".into());
        }
        if self.command.trim().is_empty() {
            return Some("No agent command yet. Set it in Settings → Agents (experimental).".into());
        }
        if let Err(e) = split_command(&self.command) {
            return Some(format!("The agent command cannot be read: {e}."));
        }
        let kind = self.sandbox.resolve();
        if kind == SandboxKind::None && !self.allow_unsandboxed {
            return Some(
                "The sandbox is off. Set allow_unsandboxed = true under [agents] to run agents with all your permissions."
                    .into(),
            );
        }
        if kind == SandboxKind::Custom && !self.sandbox_command.contains("{command}") {
            return Some("The custom sandbox wrapper needs {command} where the agent's command goes.".into());
        }
        if kind == SandboxKind::MacOs && !cfg!(target_os = "macos") {
            return Some("The macOS sandbox only exists on macOS; pick another sandbox.".into());
        }
        if kind == SandboxKind::Wsl && !cfg!(windows) {
            return Some("The WSL sandbox only exists on Windows; pick bubblewrap.".into());
        }
        if kind == SandboxKind::Bubblewrap && !cfg!(target_os = "linux") {
            return Some("Bubblewrap needs Linux; on Windows pick WSL, on macOS the macOS sandbox.".into());
        }
        None
    }

    /// Where the agent runs, for the agents panel: the Windows tools are
    /// not the ones inside WSL.
    pub fn place(&self) -> String {
        match self.sandbox.resolve() {
            SandboxKind::Wsl if self.wsl_distro.is_empty() => "inside WSL (default distribution)".into(),
            SandboxKind::Wsl => format!("inside WSL ({})", self.wsl_distro),
            SandboxKind::Custom => "through your wrapper".into(),
            _ if cfg!(windows) => "on Windows".into(),
            _ if cfg!(target_os = "macos") => "on macOS".into(),
            _ => "on Linux".into(),
        }
    }

    /// One line describing the isolation, for the agents panel.
    pub fn summary(&self) -> String {
        let kind = self.sandbox.resolve();
        let tool = self.preset.map_or_else(String::new, |p| format!("{} ", p.name));
        if kind == SandboxKind::None {
            return format!(
                "{tool}runs {}, not sandboxed: it can read and change everything your user can",
                self.place()
            );
        }
        format!(
            "{tool}runs {} · sandbox: {} · network {} · Git {} · {} min limit",
            self.place(),
            kind.label(),
            if self.network { "on" } else { "off" },
            match self.git_access {
                GitAccess::Read => "read-only (ffwd commits)",
                GitAccess::Write => "writable",
            },
            self.timeout.as_secs() / 60
        )
    }
}

/// Splits a command line into arguments. Whitespace separates; `'…'` is
/// literal; `"…"` allows `\"` and `\\`. A backslash outside quotes is a
/// normal character, so Windows paths need no escaping.
pub fn split_command(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                have = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err("a ' quote is not closed".into()),
                    }
                }
            }
            '"' => {
                have = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                            if let Some(n) = chars.next() {
                                cur.push(n);
                            }
                        }
                        Some(c) => cur.push(c),
                        None => return Err("a \" quote is not closed".into()),
                    }
                }
            }
            c if c.is_whitespace() => {
                if have {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            c => {
                have = true;
                cur.push(c);
            }
        }
    }
    if have {
        out.push(cur);
    }
    if out.is_empty() {
        return Err("it is empty".into());
    }
    Ok(out)
}

/// Replaces `{name}` with its value. Unknown names stay as they are;
/// `{{` and `}}` are literal braces. Values are inserted as text and are
/// never split or interpreted again.
pub fn expand(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(end) = tail.find('}').filter(|_| tail.starts_with('{'))
            && let Some((_, v)) = vars.iter().find(|(k, _)| *k == &tail[1..end])
        {
            out.push_str(v);
            rest = &tail[end + 1..];
            continue;
        }
        out.push_str(&tail[..1]);
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

/// Splits `template` into arguments first, then expands each one: a value
/// with spaces or quotes stays exactly one argument. This is what keeps a
/// prompt from injecting options or commands.
pub fn argv(template: &str, vars: &[(&str, &str)]) -> Result<Vec<String>, String> {
    Ok(split_command(template)?.iter().map(|a| expand(a, vars)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_a_shell_but_keeps_windows_paths() {
        assert_eq!(
            split_command(r#"claude -p "two words" 'it''s' C:\tools\x.exe "a \"q\" \\ b""#).unwrap(),
            vec!["claude", "-p", "two words", "its", r"C:\tools\x.exe", r#"a "q" \ b"#]
        );
        assert_eq!(split_command(r#"x """#).unwrap(), vec!["x", ""]);
        assert!(split_command("a 'b").is_err());
        assert!(split_command("   ").is_err());
    }

    #[test]
    fn a_prompt_stays_one_argument() {
        let evil = r#"fix it" ; rm -rf / ; echo "$(whoami) --dangerous"#;
        let a = argv("agent --message {prompt} --cwd {worktree}", &[("prompt", evil), ("worktree", "/w t")]).unwrap();
        assert_eq!(a, vec!["agent", "--message", evil, "--cwd", "/w t"]);
    }

    #[test]
    fn expansion() {
        let v = [("a", "1"), ("b", "{a}")];
        assert_eq!(expand("x{a}y{b}{c}{{a}}", &v), "x1y{a}{c}{a}");
        assert_eq!(expand("{", &v), "{");
        assert_eq!(expand("}{a", &v), "}{a");
        assert_eq!(expand("é{a}ü", &v), "é1ü");
    }

    #[test]
    fn defaults_are_safe() {
        let c = Config::default();
        assert!(!c.enabled, "experimental: off by default");
        assert_eq!(c.sandbox, SandboxKind::Auto);
        assert!(!c.allow_unsandboxed);
        assert_eq!(c.git_access, GitAccess::Read);
        assert!(c.auto_commit);
        assert_eq!(c.timeout, Duration::from_secs(3600));
        assert!(c.refusal().is_some());
    }

    #[test]
    fn reads_settings_and_reports_bad_values() {
        let get = |k: &str| -> Option<String> {
            Some(
                match k {
                    "enabled" => "true",
                    "command" => "aider --yes-always --message-file {prompt_file}",
                    "sandbox" => "nonsense",
                    "network" => "off",
                    "timeout_minutes" => "99999",
                    "max_parallel" => "x",
                    "env" => "A=1|bad|B=x=y",
                    "read_paths" => "/a; /b ;",
                    _ => return None,
                }
                .to_string(),
            )
        };
        let (c, errors) = Config::from_lookup(get);
        assert!(c.enabled && !c.network);
        assert_eq!(c.sandbox, SandboxKind::Auto);
        assert_eq!(c.timeout, Duration::from_secs(24 * 3600), "clamped to a day");
        assert_eq!(c.max_parallel, 2);
        assert_eq!(c.env, vec![("A".into(), "1".into()), ("B".into(), "x=y".into())]);
        assert_eq!(c.read_paths, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(errors.len(), 3, "{errors:?}");
    }

    #[test]
    fn unsandboxed_needs_explicit_consent() {
        let mut c = Config { enabled: true, command: "agent".into(), sandbox: SandboxKind::None, ..Config::default() };
        assert!(c.refusal().is_some_and(|r| r.contains("allow_unsandboxed")));
        c.allow_unsandboxed = true;
        assert_eq!(c.refusal(), None);
    }
}
