# Changelog

## Unreleased

- **Light theme:** Settings → Appearance → Theme: System (follows your computer, also when it changes), Dark or Light.
- **Pane borders move:** drag any border between panes (sidebar, history and details, the diff, Local Changes, Files, agents); double click resets it; sizes are kept.
- **Sidebar:** a card at the top says where you are: branch, ahead/behind or in sync, upstream, changed files or a stopped merge or rebase. Branches show their age, worktrees their branch, and empty sections stay out of the way.
- **Settings → Git:** your name and email (for every repository and for the open one), the Git program and version, credential helper and certificates; **Browse…** beside every path.
- **Local Changes:** the commit controls no longer overlap in a narrow column.
- **Agents (experimental), redesigned:** a page of its own (and **Agents** in the sidebar, with a live badge). A composer with the agent to use, runs as cards grouped by state, and for each run the actions that fit: **Review changes** opens the branch's diff in the history, then Merge, Continue in terminal, Run again or Discard. Empty and off states explain how it works and turn agents on in one click.
- **Settings, redesigned:** a page of its own with sections in a sidebar, cards with full descriptions, switches and dropdowns instead of rows of buttons, search, and a clear **Applies to** picker for agent settings: every row set for one repository says so and can be reset. The agent sections show what is in effect for the open repository.
## 0.2.0-beta.6

A fix release for the experimental agents' terminal sessions.

- **Agents (experimental), fix:** a sandboxed terminal session closed at once (the tool had no terminal inside bubblewrap). It now keeps its terminal, and a tool that fails leaves its window open with the exit code.
- **Agents (experimental):** **Open in terminal** lists every built-in agent, not only the configured one.
## 0.2.0-beta.5

A feature release: daily-driver gaps closed, and terminal sessions for the experimental agents.

- **Agents (experimental): terminal sessions.** **Open in terminal** in the agents panel opens the tool's own interactive CLI in a terminal window: in a new worktree (listed, committed when the window closes) or in this repository, with the sandbox or without.
- **Worktrees** in the sidebar have a menu: open in a new tab, open a terminal or the agent's CLI there, show the folder, copy the path, remove (asks first; Git refuses one with changes).
- **Checkout progress:** a long checkout shows Git's progress ("Updating files: 45%") in the status bar.
- **Merge and diff tools:** Settings → General lists the merge and diff tools Git can start on your PC (including your own); pick one, or keep using your Git config.
- **Local Changes:** the diff can show side by side (the **Side by side** button in its bar); staging hunks and picking lines work there too.
- **Automatic fetch:** the open repository fetches its remotes every 10 minutes (Settings → General → **Fetch automatically**: Off, 5, 10 or 30 minutes). It stays quiet unless something new arrives, and it never asks for a password.
- **History:** stashes show in the commit list, each on the commit it was made from, with the stash's menu (apply, pop, drop…). The list title is now a menu: all commits or the current branch, and **Show stashes** to hide them.
- **Several at once:** pick branches, remote branches, tags or stashes in the sidebar (`Ctrl+click`, `Shift+click`, or **Select for several** in their menu) and push them in one push, delete them here or on the remote, or drop the stashes. Every delete asks first and names how to restore.
- **Agents (experimental):** access per path. Settings → Agents lists every folder or file the agent may use, below **Home folder**: the tool's own paths (switch each to Off, Read or Write) and the ones you add (**Add a folder or file**, then Read, Write or Remove). The home folder itself can stay private.

## 0.2.0-beta.4

A fix release for the experimental agents.

- **Agents (experimental), fix:** inside WSL, an agent installed in your own folders (for example `~/.local/bin/claude`, put on PATH by your shell profile) was reported as not installed. ffwd now asks your login shell for it, starts it by its real path, and mounts its folder read-only into the sandbox, so it also runs with a private home.

## 0.2.0-beta.3

The third beta: easier to read, more to click, and a stronger diff viewer.

- **Readability:** clearer contrast for secondary text, borders that separate panes, a clearer selection, stronger diff colours, and menus and the palette on their own surface.
- **Icons:** in every context menu, on the toolbar buttons and on branch, remote and tag labels.
- **Everything has a button:**
  - Install update, Discard and Undo in the status bar;
  - Stash on the toolbar, and Find in the history header;
  - previous and next change, find and the large view in the diff bar;
  - Stage, Discard and Add to .gitignore in the Local Changes file menu.
- **Conflicts:** ours and theirs are told apart by colour and labels, conflict headers are clearer, and the banner never clips (two rows when narrow).
- **History:**
  - compare two commits (Swap, Close);
  - Space opens a large view of the selected commit;
  - up to five lines of the commit message in the details.
- **Diff viewer:**
  - wrap long lines and scroll sideways (Shift+wheel, touchpad);
  - code text size and tab width, and whitespace characters;
  - JPEG in the image diff;
  - right-click a line to open your editor there;
  - each file's scroll position is remembered.
- **Local Changes:**
  - commit and push in one step;
  - hide untracked and show ignored files;
  - list heights follow their content.
- **Branches:**
  - set or change the upstream;
  - a warning for a gone upstream, and Stop tracking;
  - names are checked as you type.
- **Fix:** on some fonts the diff bar drew into the pane next to it.
- **Agents (experimental):** Settings → Agents lists the installed WSL distributions; pick the one the WSL sandbox runs in (it was a settings-file-only value).

## 0.2.0-beta.2

The second beta. ffwd now covers daily work end to end: staging and committing, branches and history operations, interactive rebase, conflicts, stashes, remotes, submodules, a find bar and a much richer diff viewer, and it starts faster.

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
- **Interactive rebase** (FF-37): a plan view to pick, reword, squash, fixup, drop and reorder commits (`Ctrl+Shift+R`); no editor ever opens, a backup ref keeps the old tip, and conflicts stop in Local Changes.
- **History list** (FF-33): ↑/↓ marks for commits to push and to pull; the list title switches to the current branch only; "Open on <host>" for a commit.
- **Fix:** a Local Changes diff could stay empty when history arrived at the same moment; its request now has its own place in the worker (FF-56).
- **Patches** (FF-46): copy or save a commit (or a stash, or the working changes) as a patch; apply one from the clipboard or a file, checked first, with a 3-way fallback.
- **File menus and external tools** (FF-47): right-click files to open them in your editor or the default app, show them in the folder, or open the external diff or merge tool from your Git config; choose the Git binary (`git.path`, `FFWD_GIT`).
- **Diff:** `n` and `p` jump to the next and previous change (FF-41).
- **Conflicts:** take ours or theirs for all conflicted files at once, after asking (FF-42).
- **Conflicts:** edit a conflict by hand (`e`, `Ctrl+Enter` keeps) (FF-42).
- **History:** conventional-commit prefixes (`feat:`, `fix:`) in the accent colour (FF-33).
- **Sidebar:** sort branches by most recent commit or by name (FF-36).
- **Diff:** an overview strip with change marks; a click jumps there (FF-41).
- **Tabs:** rename a tab and give it a colour; both survive a restart (FF-44).
- **CI status** (FF-24): GitHub Actions results as a dot on each commit, and "Open the CI run" in its menu.
- **Image diff** (FF-41): changed PNGs show before and after side by side.
- **Find bar:** recent queries with `↑`/`↓` in the empty field (FF-34).
- **Pull requests** (FF-40): open GitHub pull requests in the sidebar (via the GitHub CLI), check one out as `pr/<n>`, open it in the browser, and create one for the current branch.
- **Reflog** (FF-43): a view of where HEAD has been; commits on no branch are marked lost, and a branch from the menu brings one back.
- **Remote branches and tags** (FF-36): push a branch or a tag from its menu; delete a branch or tag on the remote (asks first, names the way back).
- **AI commit messages** (FF-50): **AI draft** in Local Changes, against any OpenAI-compatible endpoint (local servers such as Ollama first). It shows exactly what will be sent and where before anything leaves, never sends secret-looking files, streams the reply, and only fills the message box. Off until an endpoint is set.
- **Repositories and tabs** (FF-44): clone with progress and cancel, create a new repository, and a tab menu (close others, open in the file manager or a terminal, copy path).
- **Submodules** (FF-45, FF-54):
  - a sidebar section with each submodule's state; open one in a new tab; update one or all; add;
  - **move a submodule** to another folder (refuses while it has unsaved work);
  - **automatic relative submodule URLs**: offered on add when the host matches, convert all or one, and back (setting `submodules.relative_urls`).
- **Activity log** (FF-48): every operation with the exact git commands it ran, their exit codes, times and output; a sidebar badge counts new failures.
- **Remotes and sync** (FF-39):
  - add (with a suggested name and a first fetch), edit the URL, rename, remove (asks first), fetch one, test the connection;
  - cancel a running fetch, pull or push from its button;
  - a diverged pull asks merge or rebase.
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
- **Faster start-up** (FF-8): the window and its colours appear before the fonts are ready, and a slow Windows frame call no longer redraws the hidden window. The first frame takes 21 to 30 ms on the owner PC (it was 35 to 39 ms).
- **Find bar** (FF-34):
  - content and path searches mark the files that matched in the selected commit, and open the first of them;
  - the search covers the history the list shows: all branches, or the current one;
  - `Ctrl+F` with the diff focused finds text in the diff; the query stays when you switch files.
- **Diff viewer** (FF-41):
  - each file's scroll position is remembered;
  - copy or save one file of a commit as a patch;
  - the bar's **…** menu shows whitespace characters and sets the tab width and the code text size;
  - right-click a line (in a commit or in Local Changes) to open your editor at that line, or to copy it;
  - JPEG images in the image diff.
- **Fix:** opening find in the diff left the list's selection colour stale until the next repaint; the conflict view's "unchanged lines" label showed a box instead of "…".

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
