//! External diff and merge tools (FF-47) through `git difftool` and
//! `git mergetool`, so the user's Git configuration (`diff.tool`,
//! `merge.tool`, `difftool.<tool>.cmd`) decides which tool opens. ffwd
//! refuses instead of hanging when no tool is configured: Git would fall
//! back to a terminal tool that has no terminal here.

use crate::git::Git;

/// What an external diff compares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffOf {
    /// Working tree against the index.
    Unstaged,
    /// Index against HEAD.
    Staged,
    /// A commit against its first parent.
    Commit(String),
}

fn configured(git: &Git, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        git.output_limited(&["config", "--get", k], 4096)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

/// The configured diff tool, if any.
pub fn diff_tool(git: &Git) -> Option<String> {
    configured(git, &["diff.guitool", "diff.tool", "merge.tool"])
}

/// The configured merge tool, if any.
pub fn merge_tool(git: &Git) -> Option<String> {
    configured(git, &["merge.guitool", "merge.tool"])
}

/// Opens the external diff tool for `path` and waits until it closes.
pub fn difftool(git: &Git, path: &str, of: &DiffOf) -> Result<String, String> {
    let tool = diff_tool(git).ok_or_else(|| {
        "No diff tool is set in your Git config; set one, for example: git config --global diff.tool vscode".to_string()
    })?;
    let range;
    let mut args = vec!["difftool", "--no-prompt", "--tool", &tool];
    match of {
        DiffOf::Unstaged => {}
        DiffOf::Staged => args.push("--cached"),
        DiffOf::Commit(sha) => {
            range = format!("{sha}^!");
            args.push(&range);
        }
    }
    args.extend(["--", path]);
    git.run_with(&args, None, &[], 1 << 16).map_err(|e| e.to_string())?;
    Ok(format!("Closed {tool}"))
}

/// Opens the merge tool for a conflicted `path` and waits until it closes.
/// Git marks the file resolved when the tool reports success.
pub fn mergetool(git: &Git, path: &str) -> Result<String, String> {
    let tool = merge_tool(git).ok_or_else(|| {
        "No merge tool is set in your Git config; set one, for example: git config --global merge.tool vscode"
            .to_string()
    })?;
    git.run_with(&["mergetool", "--no-prompt", "--tool", &tool, "--", path], None, &[], 1 << 16)
        .map_err(|e| e.to_string())?;
    Ok(format!("Closed {tool}; check the file in Local Changes"))
}

/// Splits a command line into program and arguments: spaces separate,
/// double quotes group (`"C:\Program Files\Editor\editor.exe" --wait`).
pub fn split_command(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in cmd.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// The program and arguments that open `file` with the editor command
/// line `editor`, at `line` when given (FF-41).
///
/// `{file}` and `{line}` in the command are replaced; without `{file}`
/// the file is appended, and for editors ffwd knows the line is passed
/// the way that editor expects (`code -g f:12`, `idea --line 12 f`,
/// `vim +12 f`, `notepad++ -n12 f`, `subl f:12`).
pub fn editor_command(editor: &str, file: &str, line: Option<u32>) -> Option<(String, Vec<String>)> {
    let parts = split_command(editor);
    let (prog, rest) = parts.split_first()?;
    let n = line.unwrap_or(1).max(1).to_string();
    if parts.iter().any(|p| p.contains("{file}")) {
        let fill = |p: &String| p.replace("{file}", file).replace("{line}", &n);
        return Some((fill(prog), rest.iter().map(fill).collect()));
    }
    let mut args: Vec<String> = rest.to_vec();
    let Some(line) = line.filter(|&l| l > 0) else {
        args.push(file.to_string());
        return Some((prog.clone(), args));
    };
    // Both separators on every OS (a settings file may be shared), and the
    // usual executable extensions.
    let base = prog.rsplit(['/', '\\']).next().unwrap_or(prog).to_ascii_lowercase();
    let name = [".exe", ".cmd", ".bat"].iter().find_map(|e| base.strip_suffix(e)).unwrap_or(&base).to_string();
    let at = format!("{file}:{line}");
    match name.as_str() {
        "code" | "code-insiders" | "codium" | "vscodium" | "cursor" | "windsurf" => args.extend(["-g".to_string(), at]),
        "subl" | "sublime_text" | "zed" | "zeditor" | "hx" | "helix" | "fleet" => args.push(at),
        "idea" | "idea64" | "rider" | "rider64" | "clion" | "clion64" | "pycharm" | "pycharm64" | "webstorm"
        | "webstorm64" | "goland" | "goland64" | "phpstorm" | "phpstorm64" | "rustrover" | "rustrover64" | "studio"
        | "studio64" => args.extend(["--line".to_string(), line.to_string(), file.to_string()]),
        "notepad++" => args.extend([format!("-n{line}"), file.to_string()]),
        "vim" | "nvim" | "gvim" | "vi" | "emacs" | "emacsclient" | "nano" | "micro" | "kak" => {
            args.extend([format!("+{line}"), file.to_string()])
        }
        "kate" | "kwrite" => args.extend(["-l".to_string(), line.to_string(), file.to_string()]),
        _ => args.push(file.to_string()),
    }
    Some((prog.clone(), args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editors_open_at_a_line() {
        let run = |e: &str, l: Option<u32>| editor_command(e, "src/a b.rs", l).map(|(p, a)| (p, a.join("|")));
        let s = |p: &str, a: &str| Some((p.to_string(), a.to_string()));
        assert_eq!(run("code --wait", Some(12)), s("code", "--wait|-g|src/a b.rs:12"));
        assert_eq!(run(r#""C:\Tools\Code.exe""#, Some(3)), s(r"C:\Tools\Code.exe", "-g|src/a b.rs:3"));
        assert_eq!(run("subl", Some(7)), s("subl", "src/a b.rs:7"));
        assert_eq!(run("idea64", Some(7)), s("idea64", "--line|7|src/a b.rs"));
        assert_eq!(run("nvim", Some(7)), s("nvim", "+7|src/a b.rs"));
        assert_eq!(
            run(r#""C:\Program Files\Notepad++\notepad++.exe""#, Some(7)),
            s(r"C:\Program Files\Notepad++\notepad++.exe", "-n7|src/a b.rs")
        );
        assert_eq!(run("unknown-editor", Some(7)), s("unknown-editor", "src/a b.rs"), "unknown: just the file");
        assert_eq!(run("code", None), s("code", "src/a b.rs"), "no line: just the file");
        assert_eq!(run("myed --goto {file}@{line}", Some(9)), s("myed", "--goto|src/a b.rs@9"));
        assert_eq!(run("myed {file}", None), s("myed", "src/a b.rs"));
        assert_eq!(run("   ", Some(1)), None);
    }

    #[test]
    fn commands_split_like_a_shell() {
        assert_eq!(split_command("code --wait"), ["code", "--wait"]);
        assert_eq!(
            split_command(r#""C:\Program Files\Some Editor\editor.exe" -n"#),
            [r"C:\Program Files\Some Editor\editor.exe", "-n"]
        );
        assert!(split_command("   ").is_empty());
    }

    #[test]
    fn no_tool_configured_is_refused_clearly() {
        let dir = std::env::temp_dir().join(format!("ffwd-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git { root: dir.clone() };
        g.output(&["init", "-q"]).unwrap();
        // Whatever the user's global config says, a repository-local empty
        // value cannot be expressed; so only check the message when unset.
        if diff_tool(&g).is_none() {
            assert!(difftool(&g, "a.txt", &DiffOf::Unstaged).unwrap_err().contains("No diff tool"));
        }
        // A repository-local tool that only exits: ffwd runs it and returns.
        let ok = if cfg!(windows) { "cmd /c exit 0" } else { "true" };
        g.output(&["config", "diff.tool", "ffwdtest"]).unwrap();
        g.output(&["config", "difftool.ffwdtest.cmd", ok]).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        g.output(&["add", "."]).unwrap();
        assert_eq!(difftool(&g, "a.txt", &DiffOf::Staged).unwrap(), "Closed ffwdtest");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
