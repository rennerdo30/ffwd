# Changelog

## Unreleased

- **Local Changes** (FF-35):
  - unstaged and staged lists, and the diff of the selected file;
  - stage or unstage files (Space, double click, Stage all, Unstage all) and single hunks;
  - commit with a subject and a description, and amend (`Ctrl+Enter`);
  - a tab badge for uncommitted changes, and a refresh when the window gets the focus back.
- **Fix:** the Settings backdrop no longer stays on screen after closing (FF-52).

## 0.2.0-beta.1

The first beta. ffwd shows history fast and fetches, pulls and pushes; staging and committing come next.

- **History:** a streamed commit graph with refs, commit details and diffs. The first rows show before the log ends.
- **Repository tabs:** open, close and reopen repositories; the tabs come back on the next start. `Ctrl+O` finds repositories next to the open ones.
- **Fetch, Pull and Push** from the top bar, with progress and a one-line result.
- **Sidebar:**
  - Branches, remotes, tags, stashes and worktrees in folding sections, with branch folders.
  - It keeps its own selection, has a filter and works from the keyboard.
- **Command palette** (`Ctrl+K`): jump to branches, tags and SHAs, and run commands. It has real text editing, and entries can be clicked.
- **Settings** (gear or `Ctrl+,`): the layout, updates and channel, repository folders, and the AI endpoint and model. Changes save at once, and `settings.toml` keeps your comments.
- **Look and feel:**
  - A new "graphite and amber" theme and larger, higher-contrast text.
  - Hover and press feedback on every control, and hand and I-beam cursors.
  - One logo everywhere.
- **Agents (experimental, off by default):** hand a task to a coding agent such as Claude Code, Codex, Antigravity, opencode or Muse.
  - Each run gets its own worktree and branch, inside a sandbox, with a time limit and scripts.
  - You review the result and then merge or discard it.
  - Settings can differ per repository.
  - See `docs/AGENTS-EXPERIMENTAL.md`.
- **Windows 11 title row** with Snap Layouts; macOS title bar under the traffic lights.
- **Install and update:**
  - Windows: a per-user setup (no admin rights) and a portable zip. macOS and Linux: archives.
  - Installed Windows copies check daily and install a checksum-verified setup on request (`Ctrl+Shift+U`). Betas follow the beta channel.

**Known limits:**
- The setup is not code-signed yet, so Windows SmartScreen warns on the first run.
- The macOS archive is unsigned, so Gatekeeper asks.
- macOS and Linux only report new releases; they don't install them.
