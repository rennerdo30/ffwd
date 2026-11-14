//! Turns an agent or script command into a sandboxed [`Launch`].
//!
//! Default deny: the agent sees the system's programs and libraries, its
//! worktree, the run folder (read-only), the repository's Git data (read-only
//! unless `git_access = "write"`) and the folders the user granted. Nothing
//! else of the user's files, unless the user opts in (`home_access`).

use crate::config::{Config, GitAccess, HomeAccess, SandboxKind};
use crate::proc::Launch;
use std::path::{Path, PathBuf};

/// The folders of one run.
#[derive(Clone, Debug)]
pub struct Ctx<'a> {
    pub cfg: &'a Config,
    pub worktree: &'a Path,
    /// The repository's shared Git folder (`git rev-parse --git-common-dir`).
    pub git_common: &'a Path,
    /// This worktree's own Git folder (`<common>/worktrees/<name>`).
    pub git_dir: &'a Path,
    /// ffwd's folder for this run (prompt, record); read-only inside.
    pub run_dir: &'a Path,
    /// `FFWD_AGENT_*` variables for the process.
    pub vars: Vec<(String, String)>,
    /// The home folder where the agent runs: the host's, or inside WSL the
    /// Linux one (found by the preflight check). `~/…` grants resolve here.
    pub home: Option<String>,
    /// The agent's program where it runs (inside WSL: found through the
    /// user's login shell): (its name in the command, its real path). The
    /// sandbox starts the real path and mounts its folder read-only, so a
    /// tool installed in the home folder runs with a private home too.
    pub program: Option<(String, String)>,
}

/// `~/rest` (or `~` + backslash + `rest`) gives `rest`; `~` alone gives "".
pub fn home_rel(p: &Path) -> Option<String> {
    let t = s(p);
    if t == "~" {
        return Some(String::new());
    }
    t.strip_prefix("~/").or_else(|| t.strip_prefix("~\\")).map(|r| r.replace('\\', "/"))
}

/// The user's home on this machine.
pub fn host_home() -> Option<String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| h.to_string_lossy().into_owned())
        .filter(|h| !h.is_empty())
}

/// A granted path on this machine (`~` resolved).
pub fn host_path(p: &Path) -> PathBuf {
    match (home_rel(p), host_home()) {
        (Some(r), Some(h)) if r.is_empty() => PathBuf::from(h),
        (Some(r), Some(h)) => Path::new(&h).join(r),
        _ => p.to_path_buf(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvFrom {
    /// The allowlist, or the whole host environment with `env_inherit`.
    Host,
    /// Always the allowlist on a cleared environment (bubblewrap).
    Allowlist,
    /// Only the user's `env` and ffwd's variables (WSL).
    Named,
}

/// The environment the agent gets: host variables per `from`, then the
/// user's `env`, then ffwd's `FFWD_AGENT_*`. Returns (clear first, vars).
fn agent_env(ctx: &Ctx, from: EnvFrom) -> (bool, Vec<(String, String)>) {
    let cfg = ctx.cfg;
    let mut env = Vec::new();
    let clear = match from {
        EnvFrom::Host => !cfg.env_inherit,
        EnvFrom::Allowlist => true,
        EnvFrom::Named => false,
    };
    if from == EnvFrom::Named
        && let Some(p) = cfg.preset.filter(|_| cfg.preset_grants)
    {
        // API keys set on Windows reach the tool inside WSL (by name).
        for name in p.env_pass {
            if let Some(v) = std::env::var_os(name) {
                env.push((name.to_string(), v.to_string_lossy().into_owned()));
            }
        }
    }
    if clear {
        for name in &cfg.env_pass {
            if let Some(v) = std::env::var_os(name) {
                env.push((name.clone(), v.to_string_lossy().into_owned()));
            }
        }
    }
    env.extend(cfg.env.iter().cloned());
    env.extend(ctx.vars.iter().cloned());
    (clear, env)
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A Windows path as WSL sees it: the user's `wsl_path_map` first, then
/// `X:\…` → `/mnt/x/…`.
pub fn to_wsl(p: &Path, map: &[(String, String)]) -> Result<String, String> {
    let raw = s(p);
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string();
    let norm = raw.replace('\\', "/");
    let mut best: Option<&(String, String)> = None;
    for m in map {
        let from = m.0.replace('\\', "/");
        if norm.to_ascii_lowercase().starts_with(&from.to_ascii_lowercase())
            && best.is_none_or(|b| b.0.len() < m.0.len())
        {
            best = Some(m);
        }
    }
    if let Some((from, to)) = best {
        let rest = &norm[from.replace('\\', "/").len()..];
        return Ok(format!("{}/{}", to.trim_end_matches('/'), rest.trim_start_matches('/'))
            .trim_end_matches('/')
            .to_string());
    }
    let b = norm.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        let rest = norm[2..].trim_start_matches('/');
        let base = format!("/mnt/{}", (b[0] as char).to_ascii_lowercase());
        return Ok(if rest.is_empty() { base } else { format!("{base}/{rest}") });
    }
    if norm.starts_with('/') && !norm.starts_with("//") {
        return Ok(norm);
    }
    Err(format!("{raw} has no WSL path; add it to agents.wsl_path_map"))
}

/// Bubblewrap arguments (Linux, or inside WSL with `conv` translating paths).
fn bwrap_args(ctx: &Ctx, conv: &dyn Fn(&Path) -> Result<String, String>) -> Result<Vec<String>, String> {
    let cfg = ctx.cfg;
    let mut a: Vec<String> = ["--die-with-parent", "--new-session", "--unshare-all"].map(String::from).to_vec();
    if cfg.network {
        a.push("--share-net".into());
    }
    for d in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/etc", "/opt", "/nix", "/run/systemd/resolve"] {
        a.extend(["--ro-bind-try".into(), d.into(), d.into()]);
    }
    a.extend(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"].map(String::from));
    let bind = |a: &mut Vec<String>, flag: &str, p: &Path| -> Result<(), String> {
        let w = conv(p)?;
        a.extend([flag.to_string(), w.clone(), w]);
        Ok(())
    };
    let home = ctx.home.clone().map(|h| h.trim_end_matches('/').to_string());
    // Inside, the agent's home is either an empty one or the real one.
    let inside_home = match (cfg.home_access, &home) {
        (HomeAccess::Private, _) | (_, None) => {
            a.extend(["--dir", "/tmp/home", "--setenv", "HOME", "/tmp/home"].map(String::from));
            "/tmp/home".to_string()
        }
        (access, Some(h)) => {
            let flag = if access == HomeAccess::Write { "--bind" } else { "--ro-bind" };
            a.extend([flag.to_string(), h.clone(), h.clone()]);
            h.clone()
        }
    };
    // Granted folders: `~/x` is the real `~/x`, seen at `<inside home>/x`.
    for (flag, list) in [("--ro-bind-try", &cfg.read_paths), ("--bind-try", &cfg.write_paths)] {
        for p in list {
            match (home_rel(p), &home) {
                (Some(r), Some(h)) if !r.is_empty() => {
                    a.extend([flag.to_string(), format!("{h}/{r}"), format!("{inside_home}/{r}")])
                }
                (Some(_), _) => {}
                (None, _) => bind(&mut a, flag, p)?,
            }
        }
    }
    if let Some((_, real)) = &ctx.program
        && let Some((dir, _)) = real.rsplit_once('/')
        && !dir.is_empty()
    {
        a.extend(["--ro-bind-try".to_string(), dir.to_string(), dir.to_string()]);
    }
    bind(&mut a, if cfg.git_access == GitAccess::Write { "--bind" } else { "--ro-bind" }, ctx.git_common)?;
    // The worktree's own Git folder (index, HEAD) stays writable so that
    // `git status` and `git diff` work for the agent.
    bind(&mut a, "--bind", ctx.git_dir)?;
    bind(&mut a, "--ro-bind", ctx.run_dir)?;
    bind(&mut a, "--bind", ctx.worktree)?;
    a.extend(["--chdir".into(), conv(ctx.worktree)?]);
    Ok(a)
}

/// The macOS sandbox profile: the system stays readable, the home folder
/// does not (except granted folders), and writes go only to the worktree,
/// temporary folders and granted folders.
pub fn macos_profile(ctx: &Ctx, home: Option<&Path>) -> String {
    let cfg = ctx.cfg;
    let q = |p: &Path| {
        let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        format!("(subpath \"{}\")", s(&c).replace('\\', "\\\\").replace('"', "\\\""))
    };
    let mut writes: Vec<PathBuf> = vec![ctx.worktree.into(), ctx.git_dir.into()];
    writes.extend(cfg.write_paths.iter().map(|p| host_path(p)));
    if cfg.git_access == GitAccess::Write {
        writes.push(ctx.git_common.into());
    }
    let mut reads: Vec<PathBuf> = vec![ctx.run_dir.into(), ctx.git_common.into()];
    reads.extend(cfg.read_paths.iter().map(|p| host_path(p)));
    let mut p = String::from("(version 1)\n(allow default)\n");
    if cfg.home_access != HomeAccess::Write || home.is_none() {
        p.push_str("(deny file-write*)\n");
        p.push_str(
            "(allow file-write* (subpath \"/private/tmp\") (subpath \"/private/var/folders\") (subpath \"/dev\")",
        );
        for w in &writes {
            p.push(' ');
            p.push_str(&q(w));
        }
        p.push_str(")\n");
    }
    if let (HomeAccess::Private, Some(h)) = (cfg.home_access, home) {
        p.push_str(&format!("(deny file-read* {})\n(allow file-read*", q(h)));
        for r in reads.iter().chain(writes.iter()) {
            p.push(' ');
            p.push_str(&q(r));
        }
        p.push_str(")\n");
    }
    if !cfg.network {
        p.push_str("(deny network*)\n(allow network* (local unix))\n");
    }
    p
}

/// Starts `inner` where the sandbox would (inside WSL for the WSL sandbox,
/// else on this PC) but without the sandbox: for an interactive session
/// the user chose to run with their own permissions (FF-23).
pub fn plain(inner: &[String], ctx: &Ctx) -> Result<Launch, String> {
    let cfg = ctx.cfg;
    if inner.is_empty() {
        return Err("empty command".into());
    }
    let cwd = ctx.worktree.to_path_buf();
    if cfg.sandbox.resolve() == SandboxKind::Wsl {
        let mut args = Vec::new();
        if !cfg.wsl_distro.is_empty() {
            args.extend(["-d".to_string(), cfg.wsl_distro.clone()]);
        }
        args.extend(["--cd".to_string(), to_wsl(ctx.worktree, &cfg.wsl_path_map)?, "-e".into()]);
        let mut inner = inner.to_vec();
        if let Some((name, real)) = &ctx.program
            && inner[0] == *name
        {
            inner[0] = real.clone();
        }
        args.extend(inner);
        let env: Vec<(String, String)> = ctx.vars.clone();
        let mut names: Vec<String> = env.iter().map(|(k, _)| k.clone()).collect();
        if let Ok(prev) = std::env::var("WSLENV")
            && !prev.is_empty()
        {
            names.push(prev);
        }
        let mut env = env;
        env.push(("WSLENV".into(), names.join(":")));
        return Ok(Launch { program: "wsl.exe".into(), args, cwd, clear_env: false, env, stdin: None });
    }
    let mut env = cfg.env.clone();
    env.extend(ctx.vars.iter().cloned());
    Ok(Launch { program: inner[0].clone(), args: inner[1..].to_vec(), cwd, clear_env: false, env, stdin: None })
}

/// Wraps `inner` (the agent's or a script's argv) in the configured sandbox.
pub fn wrap(inner: &[String], ctx: &Ctx, stdin: Option<Vec<u8>>) -> Result<Launch, String> {
    let cfg = ctx.cfg;
    if inner.is_empty() {
        return Err("empty command".into());
    }
    let cwd = ctx.worktree.to_path_buf();
    match cfg.sandbox.resolve() {
        SandboxKind::None | SandboxKind::Auto => {
            let (clear_env, env) = agent_env(ctx, EnvFrom::Host);
            Ok(Launch { program: inner[0].clone(), args: inner[1..].to_vec(), cwd, clear_env, env, stdin })
        }
        SandboxKind::Custom => {
            let vars = template_vars(ctx);
            let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let mut argv = Vec::new();
            let mut spliced = false;
            for a in crate::config::split_command(&cfg.sandbox_command)? {
                if a == "{command}" {
                    argv.extend(inner.iter().cloned());
                    spliced = true;
                } else {
                    argv.push(crate::config::expand(&a, &pairs));
                }
            }
            if !spliced {
                return Err("sandbox_command needs {command} as its own argument".into());
            }
            let (clear_env, env) = agent_env(ctx, EnvFrom::Host);
            Ok(Launch { program: argv[0].clone(), args: argv[1..].to_vec(), cwd, clear_env, env, stdin })
        }
        SandboxKind::Bubblewrap => {
            let mut args = bwrap_args(ctx, &|p| Ok(s(p)))?;
            args.push("--".into());
            args.extend(inner.iter().cloned());
            let (clear_env, env) = agent_env(ctx, EnvFrom::Allowlist);
            Ok(Launch { program: cfg.bwrap_path.clone(), args, cwd, clear_env, env, stdin })
        }
        SandboxKind::Wsl => {
            let map = &cfg.wsl_path_map;
            let conv = |p: &Path| to_wsl(p, map);
            let bw = bwrap_args(ctx, &conv)?;
            let mut args = Vec::new();
            if !cfg.wsl_distro.is_empty() {
                args.extend(["-d".to_string(), cfg.wsl_distro.clone()]);
            }
            args.extend(["--cd".to_string(), conv(ctx.worktree)?, "-e".into(), cfg.bwrap_path.clone()]);
            args.extend(bw);
            args.push("--".into());
            // WSL's plain PATH lacks the user's own folders (~/.local/bin):
            // start the program found by the preflight check by its path.
            let mut inner = inner.to_vec();
            if let Some((name, real)) = &ctx.program
                && inner[0] == *name
            {
                inner[0] = real.clone();
            }
            args.extend(inner);
            // Windows variables do not cross into WSL by themselves: only the
            // user's `env` and ffwd's own go through, named in WSLENV.
            let (_, env) = agent_env(ctx, EnvFrom::Named);
            let mut names: Vec<String> = env.iter().map(|(k, _)| k.clone()).collect();
            if let Ok(prev) = std::env::var("WSLENV")
                && !prev.is_empty()
            {
                names.push(prev);
            }
            let mut env = env;
            env.push(("WSLENV".into(), names.join(":")));
            Ok(Launch { program: "wsl.exe".into(), args, cwd, clear_env: false, env, stdin })
        }
        SandboxKind::MacOs => {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let profile = macos_profile(ctx, home.as_deref());
            let mut args = vec!["-p".to_string(), profile];
            args.extend(inner.iter().cloned());
            let (clear_env, env) = agent_env(ctx, EnvFrom::Host);
            Ok(Launch { program: "/usr/bin/sandbox-exec".into(), args, cwd, clear_env, env, stdin })
        }
    }
}

/// The values for `{…}` placeholders in commands and scripts.
pub fn template_vars(ctx: &Ctx) -> Vec<(&'static str, String)> {
    let get = |k: &str| ctx.vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    vec![
        ("worktree", s(ctx.worktree)),
        ("run_dir", s(ctx.run_dir)),
        ("branch", get("FFWD_AGENT_BRANCH")),
        ("base", get("FFWD_AGENT_BASE")),
        ("repo", get("FFWD_AGENT_REPO")),
        ("run_id", get("FFWD_AGENT_ID")),
        ("prompt_file", get("FFWD_AGENT_PROMPT_FILE")),
    ]
}

#[cfg(test)]
mod tests {

    /// A terminal session without the sandbox (FF-23): inside WSL it still
    /// runs in the distribution, in the worktree, by the tool's real path,
    /// but without bubblewrap.
    #[test]
    fn plain_runs_where_the_sandbox_would_without_it() {
        let mut c = cfg(SandboxKind::Wsl);
        c.wsl_distro = "Debian".into();
        let mut x = ctx(&c);
        x.worktree = Path::new(r"E:\a\wt");
        x.program = Some(("claude".into(), "/home/me/.local/bin/claude".into()));
        let l = plain(&["claude".into()], &x).unwrap();
        assert_eq!(l.program, "wsl.exe");
        assert_eq!(l.args, ["-d", "Debian", "--cd", "/mnt/e/a/wt", "-e", "/home/me/.local/bin/claude"]);
        assert!(!l.args.iter().any(|a| a.contains("bwrap")));
        let c = cfg(SandboxKind::None);
        let l = plain(&["tool".into(), "x".into()], &ctx(&c)).unwrap();
        assert_eq!((l.program.as_str(), l.args.as_slice()), ("tool", &["x".to_string()][..]));
        assert!(!l.clear_env, "the user's own environment");
    }

    use super::*;

    fn cfg(kind: SandboxKind) -> Config {
        Config {
            enabled: true,
            command: "agent".into(),
            sandbox: kind,
            env_pass: vec![],
            env: vec![("API_URL".into(), "http://localhost:11434".into())],
            read_paths: vec![PathBuf::from("/data/docs")],
            ..Config::default()
        }
    }

    fn ctx(c: &Config) -> Ctx<'_> {
        Ctx {
            cfg: c,
            worktree: Path::new("/r/.ffwd-agents/r/fix"),
            git_common: Path::new("/r/r/.git"),
            git_dir: Path::new("/r/r/.git/worktrees/fix"),
            run_dir: Path::new("/r/.ffwd-agents/r/.runs/fix"),
            vars: vec![("FFWD_AGENT_BRANCH".into(), "agent/fix".into())],
            home: Some("/home/me".into()),
            program: None,
        }
    }

    fn pos(v: &[String], w: &[&str]) -> Option<usize> {
        v.windows(w.len()).position(|x| x.iter().zip(w).all(|(a, b)| a == b))
    }

    #[test]
    fn bubblewrap_denies_by_default() {
        let c = cfg(SandboxKind::Bubblewrap);
        let inner = vec!["agent".to_string(), "--do; rm -rf ~".to_string()];
        let l = wrap(&inner, &ctx(&c), None).unwrap();
        assert_eq!(l.program, "bwrap");
        let a = &l.args;
        assert!(pos(a, &["--unshare-all", "--share-net"]).is_some());
        assert!(pos(a, &["--ro-bind", "/r/r/.git", "/r/r/.git"]).is_some(), "Git data read-only");
        assert!(pos(a, &["--bind", "/r/r/.git/worktrees/fix", "/r/r/.git/worktrees/fix"]).is_some());
        assert!(pos(a, &["--bind", "/r/.ffwd-agents/r/fix", "/r/.ffwd-agents/r/fix"]).is_some());
        assert!(pos(a, &["--ro-bind-try", "/data/docs", "/data/docs"]).is_some());
        assert!(pos(a, &["--setenv", "HOME", "/tmp/home"]).is_some(), "private home");
        assert!(!a.iter().any(|x| x.contains("localhost:11434")), "env values never on the command line");
        assert!(l.env.contains(&("API_URL".into(), "http://localhost:11434".into())));
        assert!(l.clear_env);
        // The command comes last, after `--`, unchanged.
        assert_eq!(&a[a.len() - 3..], &["--", "agent", "--do; rm -rf ~"]);
    }

    #[test]
    fn preset_grants_land_in_the_private_home() {
        let get = |k: &str| match k {
            "enabled" => Some("true".to_string()),
            "preset" => Some("claude".to_string()),
            "sandbox" => Some("bwrap".to_string()),
            _ => None,
        };
        let (c, errors) = Config::from_lookup(get);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.command, "claude -p {prompt} --permission-mode acceptEdits");
        assert!(c.env_pass.iter().any(|e| e == "ANTHROPIC_API_KEY"));
        let l = wrap(&["claude".into()], &ctx(&c), None).unwrap();
        assert!(pos(&l.args, &["--bind-try", "/home/me/.claude", "/tmp/home/.claude"]).is_some(), "{:?}", l.args);
        assert!(pos(&l.args, &["--setenv", "HOME", "/tmp/home"]).is_some());
        // The real home itself stays out.
        assert!(!l.args.windows(2).any(|w| w[0] == "--ro-bind" && w[1] == "/home/me"));
        // An explicit command still wins over the preset's.
        let (c2, _) = Config::from_lookup(|k| match k {
            "preset" => Some("claude".into()),
            "command" => Some("my-claude-wrapper {prompt}".into()),
            _ => None,
        });
        assert_eq!(c2.command, "my-claude-wrapper {prompt}");
    }

    #[test]
    fn bubblewrap_options() {
        let mut c = cfg(SandboxKind::Bubblewrap);
        c.network = false;
        c.git_access = GitAccess::Write;
        let l = wrap(&["a".into()], &ctx(&c), None).unwrap();
        assert!(!l.args.contains(&"--share-net".to_string()));
        assert!(pos(&l.args, &["--bind", "/r/r/.git", "/r/r/.git"]).is_some());
    }

    #[test]
    fn wsl_paths() {
        assert_eq!(to_wsl(Path::new(r"E:\Development\ffwd"), &[]).unwrap(), "/mnt/e/Development/ffwd");
        assert_eq!(to_wsl(Path::new(r"\\?\C:\x"), &[]).unwrap(), "/mnt/c/x");
        let map = vec![("E:\\".to_string(), "/mnt/d".to_string())];
        assert_eq!(to_wsl(Path::new(r"e:\Development\ffwd"), &map).unwrap(), "/mnt/d/Development/ffwd");
        assert!(to_wsl(Path::new(r"\\server\share"), &[]).is_err());
    }

    /// A program found in the user's folders inside WSL (FF-23): started by
    /// its real path, its folder mounted read-only, even with a private home.
    #[test]
    fn wsl_starts_the_found_program_by_path() {
        let mut c = cfg(SandboxKind::Wsl);
        c.home_access = HomeAccess::Private;
        let mut x = ctx(&c);
        x.worktree = Path::new(r"E:\a\wt");
        x.git_common = Path::new(r"E:\a\r\.git");
        x.git_dir = Path::new(r"E:\a\r\.git\worktrees\wt");
        x.run_dir = Path::new(r"E:\a\run");
        let real = "/home/me/.local/share/claude/versions/2.1";
        x.program = Some(("claude".into(), real.into()));
        let l = wrap(&["claude".into(), "-p".into(), "hi".into()], &x, None).unwrap();
        let dir = "/home/me/.local/share/claude/versions";
        assert!(pos(&l.args, &["--ro-bind-try", dir, dir]).is_some(), "the program's folder: {:?}", l.args);
        let sep = l.args.iter().rposition(|a| a == "--").unwrap();
        assert_eq!(&l.args[sep + 1..], [real, "-p", "hi"], "started by its real path");
        // Another first word (a script's shell) is left alone.
        let l = wrap(&["sh".into(), "-c".into(), "true".into()], &x, None).unwrap();
        let sep = l.args.iter().rposition(|a| a == "--").unwrap();
        assert_eq!(l.args[sep + 1], "sh");
    }

    #[test]
    fn wsl_passes_only_named_variables() {
        let mut c = cfg(SandboxKind::Wsl);
        c.home_access = HomeAccess::Read;
        c.read_paths.clear();
        let mut x = ctx(&c);
        x.worktree = Path::new(r"E:\a\wt");
        x.git_common = Path::new(r"E:\a\r\.git");
        x.git_dir = Path::new(r"E:\a\r\.git\worktrees\wt");
        x.run_dir = Path::new(r"E:\a\.runs\wt");
        let l = wrap(&["agent".into()], &x, None).unwrap();
        assert_eq!(l.program, "wsl.exe");
        assert!(pos(&l.args, &["--bind", "/mnt/e/a/wt", "/mnt/e/a/wt"]).is_some());
        assert!(pos(&l.args, &["--ro-bind", "/home/me", "/home/me"]).is_some(), "the Linux home");
        let wslenv = l.env.iter().find(|(k, _)| k == "WSLENV").unwrap();
        assert!(wslenv.1.starts_with("API_URL:FFWD_AGENT_BRANCH"), "{}", wslenv.1);
    }

    #[test]
    fn custom_wrapper_splices_the_command() {
        let mut c = cfg(SandboxKind::Custom);
        c.sandbox_command = "firejail --private={worktree} {command}".into();
        let l = wrap(&["agent".into(), "two words".into()], &ctx(&c), None).unwrap();
        assert_eq!(l.program, "firejail");
        assert_eq!(l.args, vec!["--private=/r/.ffwd-agents/r/fix", "agent", "two words"]);
    }

    #[test]
    fn macos_profile_limits_writes_and_home() {
        let mut c = cfg(SandboxKind::MacOs);
        c.network = false;
        let p = macos_profile(&ctx(&c), Some(Path::new("/Users/me")));
        assert!(p.contains("(deny file-write*)"));
        assert!(p.contains("(subpath \"/r/.ffwd-agents/r/fix\")"));
        assert!(p.contains("(deny file-read* (subpath \"/Users/me\"))"));
        assert!(p.contains("(deny network*)"));
    }

    #[test]
    fn unsandboxed_uses_the_allowlist() {
        let mut c = cfg(SandboxKind::None);
        c.env_pass = vec!["PATH".into()];
        let l = wrap(&["agent".into()], &ctx(&c), None).unwrap();
        assert!(l.clear_env);
        assert!(l.env.iter().all(|(k, _)| k == "PATH" || k == "API_URL" || k.starts_with("FFWD_")));
    }
}
