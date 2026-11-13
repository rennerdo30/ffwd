# Feature inventory

**Sources, read on 2026-11-13:**



**Status in ffwd:**
- **Built:** ffwd has it today.
- **Partial:** started.
- **M1** = daily driver, **M2** = the full feature set, **M3** = ffwd's own additions ([SPEC.md](SPEC.md) §6).
- **Skip:** deliberately not planned.

## 1. History and commit list (FF-33)

| Feature | ffwd | Notes |
|---|---|---|
| Commit list with branch graph, ref labels, avatars | **Built** | Streamed; avatars are initials so far |
| Labels in their graph colours, compact labels | Partial | Chips are coloured by kind |
| Stashes shown in the commit list (can be hidden) | M1 | |
| Highlight commits to push and to pull | M1 | |
| Branch filter: active branch (`Ctrl+Shift+A`), several branches, auto-switch on checkout | M1 | |
| Hide branches, folders or remotes; filter folders and remotes | M2 | |
| Collapse merges and branches in the graph (arrow keys, collapse all) | M2 | |
| "First parent" view mode | M2 | The diff already offers first-parent |
| Pin (star) branches and tags | M2 | |
| Message prefixes highlighted (`feat:`, `fix:`) | M2 | |
| Icon when a commit has a description | Partial | "…" after the subject |
| Simplified graph tooltip on merge nodes; highlight the node under the mouse | M2 | |
| Commit-link hover shows commit details | M2 | |
| Double-click a label to check it out | **Built** | Row or sidebar |
| Space: commit details popup or large side-by-side view | M1 | |
| Collapsible details (`Ctrl+Shift+D`), vertical layout, one-line list layout | Partial | Below/beside layout is built |
| Compare two selected commits (swap order); compare a commit with local changes | M1 | |
| Open a commit for review in its own window | M2 | |
| Open a commit in the hosting site's web page | M1 | |
| Highlight one author with a colour; `.mailmap` | M2 | |
| Horizontal scroll (Shift+wheel, touchpad) | M1 | |

## 2. Search (FF-34)

| Feature | ffwd | Notes |
|---|---|---|
| Commit search by message, author, SHA | **Built** | Find bar (`Ctrl+F`) |
| Search by diff content and by file path, with matching files listed | Partial | Content and path search are built; the list of matching files comes later |
| Search the current branch or the whole repository | M1 | |
| Recent search queries | M1 | |
| Remove matches with Delete (a to-do list) | M2 | |
| Inline search in the diff (`Ctrl+F`), kept when switching files | M1 | |
| Filter fields for the sidebar and file lists | Partial | The sidebar filter is built |

## 3. Local changes and committing (FF-35)

| Feature | ffwd | Notes |
|---|---|---|
| Staged and unstaged lists, or a combined list | **Built** (two lists) | FF-35; combined list in M1 |
| Stage by file, line, chunk; sub-chunk in entire-file mode | **Built** | Files, hunks and picked lines, also across hunks and in new files |
| Stage and unstage by drag and drop; Stage All | Partial | Stage all, Unstage all, double click and Space are built |
| Side-by-side view in local changes | M1 | |
| Discard (file, lines), reset a file to a revision | Partial | Discarding a file is built: it asks first and `Ctrl+Z` undoes it |
| Hide untracked (remembered), show ignored files | M1 | |
| Add to `.gitignore` (file, folder, pattern), template picker | Partial | Adding a file (`I`) is built |
| Commit, amend (shows the last commit's changes), commit and push, push after commit | Partial | Commit and amend (fills the last message) are built |
| Subject length reminder and limit, wrap at ruler, auto-growing description | Partial | The length reminder (50, 72) is built |
| Spell check of the message | M2 | |
| Recent messages, commit templates, `prepare-commit-msg` draft | Partial | Recent messages (`Ctrl+R`, the last 20) are built |
| `Co-authored-by` autocomplete, `--signoff` | M2 | |
| GPG and SSH signing | M1 | Through `git`, so the user's config applies |
| Skip hooks; show the hook's output; keep the message when a hook fails | **Built** | |
| Warning for large files not in LFS | M2 | |
| Uncommitted changes shown as a tab badge | **Built** | |
| UI never freezes during a long commit | **Built** | Rule 1 |

## 4. Branches, tags and refs (FF-36)

| Feature | ffwd | Notes |
|---|---|---|
| Create, rename, delete branches (several at once); create from any commit | **Built** | One at a time; several at once comes later |
| Quick checkout (`Ctrl+B`), checkout a revision (detached) | Partial | Detached checkout and double-click checkout are built; `Ctrl+B` creates a branch |
| Checkout a remote branch: offer to fast-forward, merge, rebase or reset | Partial | It creates or reuses the local tracking branch |
| Branch folders, autocomplete, move branches to a folder | Partial | Folders are shown |
| Space becomes `-` or `_` in branch names; name validation | M1 | |
| Sort references (alphabetical, recently used) | M2 | |
| Upstream state icons; warning for an invalid upstream | M1 | |
| Change the tracking branch; track a remote branch (stash and re-apply) | M1 | |
| Fast-forward a branch without checking it out; several at once | M2 | |
| Select stale local branches | M2 | |
| Tags: create (`Ctrl+Shift+G`), delete, push one or many | Partial | Create and delete are built; push comes with FF-39 |
| Delete remote branches; create and delete remote repositories | M1 | |
| Remember which sidebar items are expanded, per repository | **Built** | |

## 5. Merge, rebase and rewriting history (FF-37)

| Feature | ffwd | Notes |
|---|---|---|
| Merge (fast-forward, `--no-commit`, squash) | **Built** | Default, fast-forward only, no fast-forward, squash |
| Rebase | **Built** | Backup ref first |
| Merge or rebase by drag and drop on the sidebar | M2 | |
| Conflict prediction before merge, cherry-pick, revert, apply patch | M2 | `git merge-tree` on a worker |
| Interactive rebase: dialog, quick actions, multi-select, amend during rebase, `--update-refs` | M2 | |
| Reorder and squash by drag and drop in the commit list | M2 | |
| Squash, drop, cherry-pick several commits | M2 | |
| Cherry-pick and revert, merge commits too | Partial | Single commits are built; merge commits come later |
| Reset soft, mixed, hard | **Built** | Backup ref before every reset |
| Revert one file's bad merge resolution | M2 | |
| "Lean branching" (a branch model) | Skip | Opinionated; Git-flow comes first |
| Git-flow (option to not merge a release into develop) | M3 | |
| Bisect: good and bad marks in colour, clickable refs in the status bar | M2 | |

## 6. Stashes (FF-38)

| Feature | ffwd | Notes |
|---|---|---|
| Stash, apply, pop, drop (several), rename | Partial | Stash, apply, pop, drop and rename are built; dropping several at once comes later |
| Partial stash (selected files); snapshot to a stash | M1 | |
| Stash and re-apply on pull, checkout and track | M1 | |
| Separate sidebar section; recent stashes in a toolbar dropdown | Partial | The sidebar section is built |
| Save a stash as a patch | M2 | |

## 7. Remotes and sync (FF-39)

| Feature | ffwd | Notes |
|---|---|---|
| Fetch, pull, push with progress | **Built** | |
| Cancel a running operation | M1 | |
| Automatic fetch; prune on fetch | M1 | Fetch already prunes |
| Ahead and behind badges on the toolbar | **Built** | |
| Push several branches or tags; push to a branch of another name | M1 | |
| Set an upstream on the first push | **Built** | |
| Multiple remotes; add, edit, remove; test the connection | M1 | |
| Filter branches in the Push, Pull and Track dropdowns | M1 | |
| Pull: choose merge or rebase when diverged | M1 | FF-22 proposal |

## 8. Hosting services (FF-40)

| Feature | ffwd | Notes |
|---|---|---|
| GitHub, GitLab, Bitbucket, Azure DevOps, Gitea accounts, including Enterprise and self-hosted | M2 | Tokens in the OS keychain |
| OAuth sign-in; several accounts per service | M2 | |
| Clone from the account's repository list | M2 | |
| Create pull requests (uses the repository's PR template) | M2 | |
| Pull request and issue tabs with author and assignee filters | M3 | |
| GitHub notifications (unread marks, menu bar on Mac) | M3 | |
| Avatars from GitHub, including anonymous addresses | M1 | |
| CI and pipeline status | M3 | Not in other clients; FF-24 |

## 9. Diff viewer (FF-41)

| Feature | ffwd | Notes |
|---|---|---|
| Unified diff | **Built** | |
| Side-by-side diff | **Built** | |
| Syntax highlighting | **Built** | Built-in tokenizer, about 25 language families |
| Token and inline highlights | **Built** | Word highlights within paired lines |
| Ignore whitespace, show the entire file, context size dropdown | **Built** | 3, 10 or the whole file |
| Show whitespace characters, `-`/`+` marks for colour blindness, word wrap, tab width, font size | M1 | |
| Minimap or change marks on the scrollbar; next and previous change (`Ctrl+Up/Down`) | M1 | The selection mark is built |
| Reveal the line in an editor | M1 | |
| Copy as patch | M1 | |
| Remember the scroll position per file | M1 | |
| Image diff: PNG, JPEG, WebP, TGA, SVG, HEIC; swipe and onion; exact pixel diff, 16-bit | M2 | |
| Binary and LFS file views | M2 | |
| Submodule diff that lists its commits | M2 | |
| Large diffs loaded on request | **Built** | Capped; paging in M1 |

## 10. Merge conflict resolver (FF-42)

| Feature | ffwd | Notes |
|---|---|---|
| Built-in three-way resolver with manual editing | Partial | Choices per conflict (ours, theirs, both) are built; manual editing comes later |
| Alternative three-column layout | M2 | |
| Branch names instead of ours/theirs | **Built** | From the conflict markers |
| Conflict marks on the scrollbar; navigation links | M2 | |
| Resolve several files at once (ours or theirs) | Partial | Take ours or theirs per file is built; several at once comes later |
| Merge images, LFS files and submodules | M2 | |
| External merge tools (many presets, several configured); UnityYAMLMerge | M1 | |
| Diff of conflicted files resolved in an external editor | M2 | |
| Revision links during a conflicted rebase | M2 | |

## 11. Files, history and blame (FF-43)

| Feature | ffwd | Notes |
|---|---|---|
| File tree at any commit, with line numbers, image and LFS previews | Partial | The working tree with change marks, a filter, line numbers and syntax colours is built; other commits, images and LFS come later |
| Save a commit's files to disk; drag files to other apps | M2 | |
| File history (follows renames), by file name from Quick Launch | Partial | The Files view's History tab follows renames; Quick Launch comes later |
| Directory history | M2 | |
| History for the selected code | M2 | |
| Timeline in the history window | M2 | |
| Blame with a timeline, back and forward | Partial | Blame per line with commit bands; timeline and back and forward come later |
| Reflog: show and recover lost commits | M2 | Also feeds undo |

## 12. Repositories, tabs and workspaces (FF-44)

| Feature | ffwd | Notes |
|---|---|---|
| Tabs: close with middle click, restore closed (`Ctrl+Shift+T`), rename, colour, open in terminal or file manager | Partial | Close and restore are built |
| Vertical tabs | M2 | |
| Repository manager: folders and subfolders, recent, rename, colour tags, rearrange | Partial | The picker and recent list are built |
| Several source-code folders; new repositories found automatically | Partial | `repos.roots` |
| Workspaces (groups of tabs; per window on Mac), switch from Quick Launch | M2 | |
| Clone, init, add an existing repository; clone from a URL | M1 | |
| Repository settings (shared between worktrees); copy the repository path | M1 | |
| Repository manager shows the README as Markdown and the remote details | M3 | |
| Command line tool (open a repository, a file's log), URL handlers | M1 | `ffwd <path>` works |

## 13. Submodules, worktrees and LFS (FF-45)

| Feature | ffwd | Notes |
|---|---|---|
| Submodules: add, remove, update (on checkout by default), status on the sidebar, open the parent | M2 | |
| Worktrees: list, create, delete, check out a remote branch as a worktree, remove with its branch | Partial | Listed on the sidebar; agents create them (FF-23) |
| Git LFS: status dialog, locks (filter, unlock several), prune, progress, preview and download | M2 | |
| *ffwd's own:* move a submodule (`git mv`, refuses or asks when it has uncommitted or unpushed work, optional rename) | Planned | FF-54 |
| *ffwd's own:* automatic relative submodule URLs: propose `../lib.git` on add when the host and owner match the superproject's remote, convert existing absolute URLs in bulk (then `git submodule sync`), and make one absolute again | Planned | FF-54 |

## 14. Patches (FF-46)

| Feature | ffwd | Notes |
|---|---|---|
| Save as patch: working changes, a commit, a range or several commits, a stash | M2 | |
| Apply a patch, also from the clipboard (`Ctrl+V`), with a conflict check | M2 | |

## 15. Customisation and tools (FF-47)

| Feature | ffwd | Notes |
|---|---|---|
| Custom commands: shell, bash, URL; UI dialogs with text fields, branch pickers, file pickers and checkboxes; shared with the team; per OS; for branches, files and submodules | M2 | ffwd agents already have scripts (FF-23) |
| Open in an editor, IDE or terminal (long preset list); custom shell and terminal | M1 | |
| External diff and merge tools, several, with presets | M1 | |
| Issue-tracker link rules (Jira, Redmine, GitHub issues) | M2 | |
| Choose the Git binary (preferences or environment variable); bundled Git | M1 | ffwd uses the system `git` |
| SSH key manager, SSH configuration helper, several keys; edit Git credentials | M2 | |
| Preferences: UI zoom (`Ctrl+=`/`-`), light and dark themes (or follow the system), system accent colour, fonts | Partial | ffwd has a dark theme and settings |
| Update channels (stable, develop) | **Built** | Stable and beta |

## 16. Activity and diagnostics (FF-48)

| Feature | ffwd | Notes |
|---|---|---|
| Activity manager: every operation with its output and result, sections so auto-fetch doesn't flood it | M1 | FF-13 operation log |
| Verbose Git output option; highlighted Git output | M1 | |
| Progress for long checkouts | M1 | |
| Repository benchmark | M2 | ffwd measures more: `FFWD_TRACE`, snapshots |

## 17. Statistics (FF-49)

| Feature | ffwd | Notes |
|---|---|---|
| Repository statistics: commits per author, weekday, hour | M3 | |
| Treemap of repository activity over time | M3 | |

## 18. AI (FF-50)

| Feature | ffwd | Notes |
|---|---|---|
| Commit messages and branch review with Claude Code, Codex or Cursor (if installed) | M2 | ffwd: any OpenAI-compatible endpoint, local first; agents run in sandboxes (FF-23, FF-26) |
| Editable AI requests (prompts) in the preferences | M2 | |

## ffwd's own (ffwd's own)

- **Remote-desktop mode:** damage-only CPU presents, copy-scrolling, no animations.
- **Undo for every operation**, from the reflog and backup refs.
- **Agent workspaces (experimental):** a sandboxed agent works in its own worktree, and you review its branch.
- **AI against the user's endpoint:**
  - commit messages;
  - explaining commits;
  - branch review;
  - conflict help;
  - natural-language history search.
- **Linux builds.**
- **Measured speed budgets in CI** ([SPEC.md](SPEC.md) §3).
