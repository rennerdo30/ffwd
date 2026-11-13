# ffwd specification (draft)

Status: draft from 2026-11-13. The owner sets the final scope and numbers.

## 1. Product

ffwd is a desktop Git client for Windows, macOS and Linux. It covers everything a complete desktop Git client does ([FEATURES.md](FEATURES.md)), adds AI help against an endpoint the user configures, and is faster than any other Git client at every step.

**Why it exists:** the Git clients the owner uses daily render late, crash, and are slow, above all over remote desktop. ffwd's core promise is that every action feels instant, the keyboard comes first, and the app stays out of the way.

**Not a goal:** a Git hosting front end. Hosting integrations (pull requests, CI status) are a later milestone; local Git work comes first.

## 2. Principles

The binding versions are the performance rules in [`AGENTS.md`](../AGENTS.md). In short:

1. Nothing blocks the UI thread.
2. Input answers in the same frame, from data already in memory.
3. The first frame comes first; data streams in behind it.
4. Work scales with the viewport, not with the repository.
5. Event-driven, damage-only drawing; scrolling is a pixel copy.
6. The CPU renderer is the remote-desktop path.
7. Nothing in user data can crash the app.
8. No user work is ever lost; destructive actions can be undone.

## 3. Speed budgets (proposed)

Reference machine: the owner's Windows PC (Ryzen 9 9950X3D, NVMe). A mid-range laptop and an RDP session follow once they can be measured.

| Measure | Target | Hard limit | PoC, measured 2026-11-13 |
|---|---|---|---|
| Launch to first frame (window and chrome painted) | ≤ 30 ms | 60 ms | **21 to 30 ms**, median 22 ms (release, `--smoke`, 2160×1350 px; 2026-11-14, FF-8) |
| Launch to first commit rows, repository with 1k commits | ≤ 60 ms | 120 ms | 45 to 90 ms (`git log` process; see §4) |
| Launch to first commit rows, 1M commits | ≤ 150 ms | 300 ms | not measured yet |
| Key press to pixels (selection, scroll, palette) | ≤ 1 frame (8.3 ms at 120 Hz) | 16 ms | 0.4 to 0.9 ms paint |
| Selected commit: header | same frame | same frame | same frame (from the history row) |
| Selected commit: files and diff | ≤ 30 ms | 100 ms | ~100 ms (two `git` processes; see §4) |
| Full repaint, 1600×1000 | ≤ 4 ms | 8 ms | 3.4 to 4.6 ms |
| Idle | 0 presents, 0 % CPU | none | 0 presents (event-driven) |
| Key Down in the history | repaints 2 rows, the details pane and the status field | none | 2 rows, details, status: 24 to 42 % of the window |
| Scroll one step | 1 copy and the exposed rows | none | copy (`BitBlt`) and the exposed rows |
| Memory, 1M-commit repository | ≤ 300 MB | 600 MB | not measured yet |
| Clean dev build (sccache warm) | ≤ 10 s | 30 s | 2.8 s |
| Incremental rebuild after a UI edit | ≤ 2 s | 5 s | 1.0 s |

## 4. Architecture

A Cargo workspace. Core crates have no UI dependencies; heavy dependencies stay in leaf crates.

```
crates/
  ffwd-core    repository model: history streaming, refs, graph lanes, diffs (no UI)
  ffwd-render  CPU canvas, anti-aliased vector shapes, text (no windowing)
  ffwd-app     binary: winit window, softbuffer presents, views, workers
```

Planned: `ffwd-ai` (endpoint client, prompt building, streaming), `ffwd-diff` (word diff, alignment, syntax highlighting), and per-platform integration crates (credentials, file associations).

**Threads:** one UI thread plus workers (`std::thread`, channels). The history worker streams commits in batches (the first after 200 commits, then at most every 100 ms). A single details worker always serves only the newest request, so holding an arrow key never queues Git processes.

**Rendering:** the same model as Kwil ([Kwil ADR 0001](https://github.com/rennerdo30/kwil/blob/main/docs/decisions/0001-renderer.md)): winit plus softbuffer, our own text and vector code, damage-only presents, scrolling by screen copy. See [decisions/0001-ui-renderer.md](decisions/0001-ui-renderer.md).

**Git backend: open, with a first measurement.** The PoC reads through the `git` CLI. On the owner's PC, every `git` process costs about 30 ms just to start (`git rev-parse HEAD`: median 33 ms; `git show -s`: 32 ms). So a commit's files and diff take at least two process starts, about 100 ms in total. That is the PoC's slowest visible step, and CLI-based clients share the same structural cost.

Proposal, to be measured in M1 before deciding: read in-process with gitoxide (history, refs, trees, blobs, diffs, status), and write through the `git` CLI (commit, merge, rebase, push, pull), so that hooks, credentials and config behave exactly as they do in the user's own Git.

## 5. AI

- **Endpoint setting:** an OpenAI-compatible URL with model and key, with presets for Ollama, llama.cpp and LM Studio. Keys are stored in the OS credential store, never in plain settings.
- **Off by default.** No data leaves the machine until an endpoint is set. Before each request the user sees exactly what will be sent; secrets and ignored files are never included.
- **First features:**
  - draft a commit message from the staged diff;
  - explain a commit or a range;
  - review a branch against its base;
  - suggest a resolution for a conflict hunk;
  - search history in natural language (translated into Git queries that the user can see).
- **Rules:** streaming, cancellable, and on a worker. Results are suggestions; nothing is written without the user's confirmation.

## 6. Milestones (proposed)

- **M0, foundation (now):** rules, the fast build, the renderer and history PoC, logo, public repository.
- **M1, daily driver:**
  - Git backend decision, by measurement;
  - local changes with line and hunk staging;
  - commit and amend;
  - fetch, pull and push with credentials;
  - branch and tag operations, checkout, stash;
  - side-by-side diff;
  - search;
  - settings;
  - Windows installer.
- **M2, the full feature set:** interactive rebase, merge conflict resolver, blame, file history, reflog, submodules, worktrees, LFS, bisect, custom commands, image diff, macOS and Linux packages.
- **M3, ffwd's own additions:**
  - AI features;
  - undo for every operation;
  - hosting integrations;
  - remote-desktop qualification with measured bytes on the wire.

## 7. Known PoC limits (from the 2026-11-13 review)

- **History memory grows with the repository.** Every commit row and its graph row stay in memory, about 150 to 250 bytes each, so 1M commits need roughly 150 to 250 MB. M1 replaces this with a compact columnar store and a string arena, measured against the 300 MB budget in §3. It comes together with the backend decision, because gitoxide can re-read rows on demand.
- **Font loading is off the startup path** (FF-8, 2026-11-14). The window is shown as soon as it exists. If the font worker is not done by then, a frame of plain chrome in the theme's colours is presented (`FFWD_TRACE` mark "chrome presented"), and the full UI follows when the worker wakes the event loop. The frame hook's `SetWindowPos(SWP_FRAMECHANGED)` on the hidden window had cost 10 to 97 ms; with `SWP_NOREDRAW` it takes 1.4 to 2 ms.
- **Remote mode is re-checked on focus and resize, not on `WM_WTSSESSION_CHANGE`.** RDP connects and disconnects trigger both, but a direct session notification is still the proper signal. Remote mode changes no behaviour yet, because the PoC has no animations.

## 8. Open questions for the owner

1. Your daily Git workflows that ffwd must make faster first.
2. Typical repository sizes (commits, files, LFS, submodules), for the M1 benchmarks.
3. GitHub integration (pull requests, CI status) in M1, or later?
4. Keymap: common Git-client shortcuts by default, with a Vim-style preset?
