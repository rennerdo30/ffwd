# Feature inventory

**Source:** website feature list and the Windows release notes up to other clients 2.24 (6 November 2026), read on 2026-11-13. other clients ships for Windows and macOS only; ffwd adds Linux.

**Milestones:** M1 = daily driver, M2 = the full feature set, M3 = ffwd's own additions ([SPEC.md](SPEC.md) §6).

## History and navigation

| Feature | ffwd | Notes |
|---|---|---|
| Commit list with branch graph, refs and stashes | **PoC** | The graph streams in incrementally; stashes in the list come in M1 |
| Commit details: header, files, diff | **PoC** | Header in the same frame, files and diff behind it |
| Quick Launch (fuzzy commands and paths) | **PoC** | Palette with `Ctrl+K`/`Ctrl+P`: branches, tags, SHAs, commands |
| Search commits by message, author, SHA, content | M1 | Runs on a worker and streams results into the list |
| Tabs, restoring a closed tab, vertical tabs | M1 | |
| Repository manager, recent repositories, source folders | M1 | |
| Tab/Shift+Tab between sidebar, list and details | **PoC** | List, files, diff |
| Space: commit details popup | M1 | |
| Collapsible commit details, vertical/horizontal layout | M2 | |
| Collapsing merges in the list | M2 | |
| File tree at any commit | M2 | |
| File history, history for selected code, hunk history | M2 | |
| Blame | M2 | Streamed, viewport-bound |
| Reflog view, restoring lost commits | M2 | Also feeds ffwd's undo (M3) |
| Bisect with good/bad marks | M2 | |
| Repository statistics, treemap | M3 | |

## Working copy and commits

| Feature | ffwd | Notes |
|---|---|---|
| Staged/unstaged lists, staging by line and hunk | M1 | |
| Discard, ignore, `.gitignore` templates | M1 | Discard is undoable |
| Commit, amend, recent messages | M1 | |
| GPG and SSH signing, hooks | M1 | Through the `git` CLI, so the user's config applies |
| Save as patch, apply patch (also from the clipboard) | M2 | |
| Git LFS status and locks | M2 | |
| Hide untracked files | M1 | |

## Branches and history rewriting

| Feature | ffwd | Notes |
|---|---|---|
| Create, rename and delete branches and tags; checkout | M1 | |
| Merge, rebase, conflict prediction before running | M1 / M2 | Prediction in M2 |
| Interactive rebase, drag-and-drop reorder and squash, `--update-refs` | M2 | |
| Cherry-pick and revert, conflict prediction | M1 / M2 | |
| Reset (soft, mixed, hard) | M1 | Hard reset leaves a backup ref |
| Stashes: create, apply, pop, restore staged state, save as patch | M1 | |
| Worktrees: create, remove, check out a remote branch as a worktree | M2 | |
| Submodules | M2 | |
| Git-flow | M3 | |
| Select stale local branches | M2 | |

## Remotes and hosting

| Feature | ffwd | Notes |
|---|---|---|
| Fetch, pull and push; ahead/behind badges | M1 | Badges shown in the PoC |
| Upstream tracking, multiple remotes | M1 | |
| GitHub, GitLab, Bitbucket and Azure accounts; cloning from an account | M2 | |
| GitHub notifications | M3 | |
| Credential manager integration | M1 | |

## Diff and merge

| Feature | ffwd | Notes |
|---|---|---|
| Unified and side-by-side diff | **PoC** (unified) | Side by side in M1 |
| Token-level inline highlighting, syntax highlighting | M1 | |
| Ignore whitespace, entire-file mode, hunk navigation | M1 | |
| Image diff (PNG, JPEG, WebP, TGA and more) | M2 | |
| Built-in three-way merge with conflict marks on the scrollbar | M2 | |
| External diff and merge tools | M1 | |
| Large diffs loaded on request | **PoC** | Capped at 20,000 lines per file; paging in M1 |

## Integration and extension

| Feature | ffwd | Notes |
|---|---|---|
| Open in an editor, terminal or file explorer | M1 | |
| Custom commands with UI controls | M2 | |
| Issue-tracker links | M2 | |
| Activity manager, verbose Git output | M1 | Every Git call visible, with its duration |
| Claude Code branch review and commit messages | M3 | ffwd: any endpoint the user configures, local first |

## ffwd's own (ffwd's own)

- **Remote-desktop mode:** damage-only CPU presents, copy-scrolling, no animations.
- **Undo for every operation**, from reflog and backup refs.
- **AI against the user's endpoint:**
  - commit messages;
  - explaining commits;
  - branch review;
  - conflict help;
  - natural-language history search.
- **Linux builds.**
- **Measured speed budgets in CI** ([SPEC.md](SPEC.md) §3).
