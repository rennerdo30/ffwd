//! Built-in agent presets: the non-interactive command line of each tool,
//! the folders it keeps its login and config in, and the variables that
//! carry its API keys (names only; ffwd never reads the values).
//!
//! The flags were checked against each tool's own `--help` on 2026-11-13:
//! Claude Code 2.1.322, Codex CLI 0.167.2, Antigravity CLI (agy) 1.5.8,
//! opencode 2.2.7, Muse Code 1.4.14. Tools change their flags; every value
//! here can be overridden in the settings (`command`, `write_paths`, …).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    /// The settings value (`agents.preset`).
    pub id: &'static str,
    pub name: &'static str,
    /// The program, for the preflight check.
    pub program: &'static str,
    /// The command line; `{prompt}` / `{prompt_file}` are filled in.
    pub command: &'static str,
    /// Folders in the home the tool needs writable (login, sessions).
    /// `~` is the home of wherever the agent runs (inside WSL: the Linux
    /// home).
    pub home_write: &'static [&'static str],
    /// Variables passed through by name.
    pub env_pass: &'static [&'static str],
    /// How to install it on Linux (inside WSL), when known for certain.
    pub install_linux: &'static str,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "claude",
        name: "Claude Code",
        program: "claude",
        // -p: print mode (non-interactive); acceptEdits: edits without asking.
        command: "claude -p {prompt} --permission-mode acceptEdits",
        home_write: &["~/.claude", "~/.claude.json"],
        env_pass: &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "ANTHROPIC_MODEL"],
        install_linux: "npm install -g @anthropic-ai/claude-code, then run claude once to log in",
    },
    Preset {
        id: "codex",
        name: "Codex",
        program: "codex",
        // exec: non-interactive; workspace-write: may edit the worktree.
        command: "codex exec -s workspace-write {prompt}",
        home_write: &["~/.codex"],
        env_pass: &["OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_HOME"],
        install_linux: "npm install -g @openai/codex, then run codex once to log in",
    },
    Preset {
        id: "antigravity",
        name: "Antigravity",
        program: "agy",
        // -p: print mode; accept-edits: edits without asking.
        command: "agy -p {prompt} --mode accept-edits",
        home_write: &["~/.gemini"],
        env_pass: &["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_CLOUD_PROJECT"],
        install_linux: "",
    },
    Preset {
        id: "opencode",
        name: "opencode",
        program: "opencode",
        // run: one message; --auto: approves what is not denied;
        // --standalone: its own server inside the sandbox, never the
        // background service outside it.
        command: "opencode run --standalone --auto {prompt}",
        home_write: &["~/.config/opencode", "~/.local/share/opencode", "~/.local/state/opencode", "~/.cache/opencode"],
        env_pass: &["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GEMINI_API_KEY", "OPENROUTER_API_KEY"],
        install_linux: "npm install -g opencode-ai",
    },
    Preset {
        id: "muse",
        name: "Muse",
        program: "muse",
        // exec: headless; --worktree off: ffwd already made the worktree;
        // --approval-mode never: no prompts in a headless run.
        command: "muse exec --prompt-file {prompt_file} --worktree off --approval-mode never",
        home_write: &["~/.config/muse"],
        env_pass: &[],
        install_linux: "",
    },
];

pub fn find(id: &str) -> Option<&'static Preset> {
    let id = id.trim().to_ascii_lowercase();
    PRESETS.iter().find(|p| p.id == id || (id == "agy" && p.id == "antigravity"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_parses_and_takes_the_prompt() {
        for p in PRESETS {
            let argv =
                crate::config::argv(p.command, &[("prompt", "a b; c"), ("prompt_file", "/r/prompt.md")]).unwrap();
            assert_eq!(argv[0], p.program, "{}", p.id);
            assert!(argv.iter().any(|a| a == "a b; c" || a == "/r/prompt.md"), "{}: {argv:?}", p.id);
            assert!(p.home_write.iter().all(|h| h.starts_with("~/")), "{}", p.id);
        }
        assert_eq!(find("AGY").map(|p| p.id), Some("antigravity"));
        assert!(find("custom").is_none());
    }
}
