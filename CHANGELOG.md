# Changelog

## Unreleased

- **Local Changes** (FF-35):
  - unstaged and staged lists, and the diff of the selected file;
  - stage or unstage files (Space, double click, Stage all, Unstage all) and single hunks;
  - commit with a subject and a description, and amend (`Ctrl+Enter`);
  - a tab badge for uncommitted changes, and a refresh when the window gets the focus back;
  - stage single lines (`Space` picks, `S` stages);
  - discard with a confirmation and `Ctrl+Z` to undo;
  - add to `.gitignore` (`I`), skip hooks, recent messages (`Ctrl+R`).
- **Branches, tags and history** (FF-36, FF-37):
  - right-click menus and `Ctrl+.` for commits and sidebar refs;
  - check out (also remote branches, with tracking, and detached), create, rename and delete branches and tags;
  - merge (four modes), rebase, cherry-pick, revert, reset (soft, mixed, hard), each with a backup ref where commits could be lost;
  - double-click checkout; `Ctrl+B` and `Ctrl+Shift+G`.
- **Stashes** (FF-38): stash with a message (`Ctrl+Shift+S`, untracked files optional), and apply, pop, rename and drop from the sidebar. A drop asks first and keeps a backup; a conflicting pop keeps the stash.
- **Files view** (FF-51, FF-43):
  - the repository's folder tree with change marks and a type-to-filter search;
  - the selected file's content with syntax colours, its blame, and its history (following renames), which jumps to the commit.
- **Conflict resolver** (FF-42):
  - a banner for a stopped merge, rebase, cherry-pick or revert, with Continue and Abort (which asks first);
  - per-conflict choices (ours, theirs, both in either order), or one side for the whole file, then save and mark resolved;
  - keys `o`, `t`, `b`, `Enter`.
- **Diff viewer** (FF-41): side by side or unified, syntax colours, word highlights within changed lines, context of 3 or 10 lines or the whole file, whitespace ignored or shown (remembered; `s`, `c`, `w` in the diff).
- **Find bar** (`Ctrl+F`, FF-34): search messages, authors, SHAs, content (`-S`) and paths. Results stream in, non-matches are dimmed, matches are marked on the scroll track, and `↑`/`↓` jump between them.
- **Fix:** the Settings backdrop no longer stays on screen after closing (FF-52).
- **Fix:** the taskbar and Alt+Tab show the ffwd mark. The window now sets its big icon at the monitor's DPI (again on a scale change), and the Windows executable embeds the app icon, so Explorer, pinned buttons and shortcuts show it too.

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
