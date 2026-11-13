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

#[cfg(test)]
mod tests {
    use super::*;

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
