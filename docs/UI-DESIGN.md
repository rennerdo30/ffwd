# UI design

Status: first design, shown in the proof of concept (2026-11-13).

![ffwd proof of concept: history, details and diff](images/poc-history.png)

The capture above is the real PoC UI. `ffwd --snapshot` rendered it offscreen from the Kwil repository, using the same paint code that the window uses.

## Layout

```
┌ top bar ──────────────────────────────────────────────────────────────────┐
│ ▶▶ repo  ⑂ branch      [ Jump to branch, commit or command   Ctrl K ]  Fetch Pull↓ Push↑ AI │
├ sidebar ──┬ history ───────────────────────────────┬ details ─────────────┤
│ WORKSPACE │ graph │ refs · subject │ author │ date │ sha │ subject, body  │
│ BRANCHES  │                                        │ author, date, SHA    │
│ REMOTES   │  virtualised rows, one per commit      │ FILES (status, +/−)  │
│ TAGS      │                                        │ diff of the file     │
│ STASHES   │                                        │                      │
├───────────┴────────────────────────────────────────┴──────────────────────┤
│ ● state · measured timings                     session · renderer · paint │
└───────────────────────────────────────────────────────────────────────────┘
```

- **The top bar holds one input for everything:** branches, tags, SHAs and commands, with `Ctrl+K`, `Ctrl+P` or `/`. actions (Fetch, Pull, Push) stay visible with live ahead/behind counts. The AI button is the single entry point to AI features.
- **The sidebar is the map, not a second list.**
  - Local Changes shows a count badge.
  - The current branch is marked in the accent colour and shows its ahead/behind counts.
  - Remotes, tags, stashes and worktrees are grouped, and long sections end in "N more".
- **The history list shows:**
  - the graph;
  - ref chips: the current branch in accent, local in green, remote in blue, tags in amber;
  - the subject, with a "…" marker when the commit has a body;
  - the author, a relative date and a short SHA.
  The selected row has a 3 px accent bar, and the scroll track marks where the selection is.
- **The details pane has three layers that arrive at different times.** The header (subject, author, date, SHA, parents) paints in the same frame as the selection, from the history row. The body, the files and the diff fill in behind it. Placeholders are solid, still bars, never spinners.
- **The status bar tells the truth:** what is loading, real measured timings, the session type (local or remote) and the renderer.

## Window frame

- **Windows 11:** the top bar is the title row, the way Explorer, Settings and Terminal look. ffwd draws minimise, maximise/restore and close with Windows 11 metrics (46 px wide, red close hover). The maximise button reports itself as `HTMAXBUTTON`, so hovering it opens Snap Layouts. The empty parts of the bar drag the window and double-click maximises; side, bottom and top resize edges stay native (`crates/ffwd-app/src/frame.rs`, the same approach as Ferrow).
- **macOS:** a transparent, full-size title bar; the native traffic lights sit over the left of the bar, which leaves room for them.
- **Linux:** server-side decorations from the desktop.
- **Icon:** the window icon is drawn from the logo's geometry at startup; `ffwd --icon-png out.png 256` writes it as a PNG for packaging.

## Sidebar

- Sections: Workspace (Local Changes, All Commits), Branches, Remotes, Tags, Stashes, Worktrees. Every section, remote and branch folder (`t3/…`) folds open and closed; the state is remembered.
- The sidebar keeps **its own selection**: clicking a branch or tag highlights it there and jumps the history to its commit. Choosing a branch in `Ctrl+K` selects it in the sidebar too.
- The checked-out branch has a check mark; ahead/behind counts sit on the right.
- A filter field at the top narrows branches, remotes, tags and stashes (folders open while filtering). Typing while the sidebar has focus starts filtering.
- Keyboard: `Tab` cycles sidebar → history → files → diff; in the sidebar `↑ ↓`, `←` closes or goes to the parent, `→` opens, `Enter` jumps.

## Settings

The gear in the top bar (or `Ctrl+,`) opens Settings: details layout, automatic updates and channel, repository folders, AI endpoint and model, **Open settings.toml**, **Check for updates now**. Changes save at once; the file keeps your comments.

## Agents panel (experimental)

`Ctrl+Shift+A` (or *Agents (experimental)…* in `Ctrl+K`) opens the agents panel of the current repository. It is off by default and labelled EXPERIMENTAL in the panel and on its settings page. All settings are listed in [AGENTS-EXPERIMENTAL.md](AGENTS-EXPERIMENTAL.md).

- **Top:** one line names what the agent can reach (the sandbox, network, Git access and time limit). It turns amber when ffwd would refuse to start and red when the agent is not sandboxed.
- **Prompt field:** `Enter` starts a run on a new branch, in its own worktree.
- **Left:** the runs of this repository, newest first, each with a status dot.
- **Right:** the selected run's branch, status, worktree, buttons (Show branch, Merge…, Discard…, Cancel run, Open folder, Open log) and its live log.
- **Confirmation:** Merge and discard ask in place and name exactly what happens. `Enter` confirms; `Esc` keeps the run.
- **Keys:** `↑ ↓`, `Enter` shows the branch, `m` merges, `d` discards, `c` cancels, `o` opens the folder, `l` opens the log, `PageUp`/`PageDown` scroll the log, and typing starts a new task.
- **Tab badge:** a dot after the tab's name. Accent while an agent runs; green or red when a run ended and you have not opened the panel since.

Settings has two pages, **General** and **Agents** (`Ctrl+Tab` switches between them). From the agents panel, `Ctrl+,` opens the Agents page.

## Repository tabs

Like other clients: one tab per repository under the top bar.
- Click a tab to switch; × or a middle click closes it; `+` opens another repository.
- **Open repository** (`Ctrl+O`, `Ctrl+T`, `+` or a click on the repository name): a picker listing recent repositories first, then repositories found next to the open ones (and in `repos.roots` in `settings.toml`, `;`-separated). Typing a folder path opens it directly.
- Each tab keeps its own history, selection and loaders. Background tabs collect their updates and apply them when shown; saved tabs from the last session load only when first shown.
- The open tabs and the active one are restored on the next start (`%APPDATA%\ffwd\state.toml`).

## Top bar actions

- **Fetch / Pull / Push** run `git` on a worker with the user's own config and credential helper. The button reads "Fetching…" while it runs, the status bar shows git's progress line and then the result; the history reloads and keeps the selected commit. Push of a branch without an upstream pushes to `origin` and sets it.
- **AI** opens the AI commands; until an endpoint is configured they say how to set one.
- Commands in `Ctrl+K` that are not built yet are marked **planned**.

## Interaction (keyboard first)

| Key | Action |
|---|---|
| `↑ ↓` / `j k` | Move the selection (history, files) or scroll (diff) |
| `PgUp PgDn Home End` | Page through the focused pane |
| `Tab` / `Shift+Tab` | Move focus: history → files → diff |
| `Enter` | Move focus into the next pane |
| `Esc` | Back to the history |
| `Ctrl+K` / `Ctrl+P` / `/` | Palette: jump to a branch, tag or SHA, or run a command |
| `Ctrl+O` / `Ctrl+T` | Open a repository in a tab |
| `Ctrl+,` | Settings |
| `Ctrl+Shift+A` | Agents panel (experimental) |
| `Ctrl+A` `Ctrl+C` `Ctrl+V` `Ctrl+X` `Ctrl+Backspace` `← →` `Home End` | Text editing in the palette and settings fields |
| `Ctrl+W` / `Ctrl+Shift+T` | Close the tab / reopen the last closed one |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` / `Ctrl+1…9` | Next, previous, n-th tab (9 = last) |
| `Ctrl+Shift+F` / `L` / `K` | Fetch / Pull / Push |
| `Ctrl+Shift+U` | Install a found update |

Mouse: click to select rows and files, click a sidebar ref to jump to its commit, and use the wheel over the history or the diff.

Planned M1 keys follow common Git-client conventions (for example `Ctrl+Shift+G` for create tag, `Ctrl+Shift+T` to restore a tab, `Ctrl+↑/↓` for hunk navigation), so that switching costs nothing.

## Visual language

- **Dark by default, solid fills only.** Depth comes from value steps (sidebar `#0C0E11`, list `#111317`, panels `#13161A`, inputs `#1B1F25`), not from shadows or blur. This rule keeps remote sessions cheap, and it looks calm.
- **One accent:** ffwd coral `#FF6A3D`, used for the current branch, focus, the selection bar and the logo. Graph lanes use ten distinct hues that are easy to tell apart on dark.
- **Type:** Inter for the UI and Cascadia Mono for SHAs and diffs, falling back to the system fonts. Both are open-licensed and can be bundled later.
- **Density:** 30 px rows at 100 %, scaled by the OS DPI factor. Text is never smaller than 11 px.
- **No motion.** There are no animations. In local sessions, smooth scrolling may come later as an option, driven only while a scroll is running.

## Next design steps

1. The Local Changes view: staged and unstaged lists, line and hunk staging, and the commit box with an AI draft.
2. Side-by-side diff with token highlights.
3. Interactive rebase as an editable list in the history pane.
4. A light theme, with theme files in the Kwil format.
