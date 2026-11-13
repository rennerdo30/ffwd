# ffwd repository rules

Read `E:/Development/AGENTS.md` and `E:/Development/general-agent-rules.md` first. They win over anything here unless this file is stricter.

- **Product:** ffwd (say "fast-forward"), a desktop Git client in Rust for Windows, macOS and Linux. It covers everything a complete desktop Git client does, plus AI features against an endpoint the user configures, with local AI preferred. **P0 is speed:** existing clients render late, crash and feel slow, and ffwd must not (owner, 2026-11-13). Scope and budgets: `docs/SPEC.md`.
- **No sub-agents** (owner, 2026-11-13: "Please do not use any sub-agents."): the main session does all work in this repository itself. No `delegate_task`, bridge jobs, native subagents or other delegated chats, including for reviews. This overrides the shared rules' delegation and separate-review-job steps for ffwd; the main reviews its own changes against the rules before merging and says so.
- **Quality bar:** it must feel better than every other Git client, not just match them (owner, 2026-11-13).
- **Pair Desk:** project `ffwd` (ids `FF-n`; owner, 2026-11-13). Handoff, backlog, plans and evidence live there, never in committed files.
- **Identity:** commit only as `rennerdo30 <9086097+rennerdo30@users.noreply.github.com>`.
- **Repository:** public at `https://github.com/rennerdo30/ffwd` (owner, 2026-11-13). Every release needs the owner's yes. Push with explicit refs only: `git push origin HEAD:refs/heads/<branch>`.
- **Public from day one:** no secrets, no owner-PC config in committed files (`.cargo/config.toml` is local), no third-party code or assets, and **no other products' names or trademarks** anywhere public: files, file names, code comments, commit messages, PR titles and bodies, release notes (owner, 2026-11-14).
- **CI runners:** GitHub-hosted (public repository rule, owner 2026-11-04).
- **Licence:** only permissively licensed dependencies (MIT, Apache-2.0, BSD, Zlib, ISC; OFL for fonts). ffwd itself is MIT.
- Paths: always `E:`; `D:` is the same drive. Branch limit: an active branch has a commit in the last 24 h; up to 10 is normal, 11 to 20 needs a recorded reason, 20 is the maximum.

## Performance rules (owner, 2026-11-13: "blazing fast, also on remote desktop sessions")

These hold for every change. A change that breaks one of them is a defect, even if all its tests pass.

1. **Nothing blocks the UI thread.** Git processes, gitoxide calls, file I/O, diffs, blame, search, network and AI requests all run on worker threads and report back through the event loop. The UI thread only handles input, layout and painting. A synchronous Git call on the UI thread is a P0 defect.
2. **Pixels answer input in the same frame.** A key press or click changes the screen in the next present, using data already in memory: the selection moves, the header of the selected commit appears, and the slower parts (files, diff) fill in behind it. Never show a blank pane while data loads. Show what is already known and mark it as loading. No spinner for anything under 150 ms.
3. **First frame first.** The window and the chrome are painted before any Git work finishes. History, refs and status stream in afterwards. The first batch of commits is sent as soon as it is parsed, never after the full log.
4. **Work scales with the viewport, not the repository.** The commit list, file lists, diffs and blame are virtualised: only visible rows are laid out and drawn. Graph lanes are computed incrementally as history streams in. Large diffs load in pages, never as one blob. The target repository size is 1M commits, 10k refs and 100k changed files.
5. **Event-driven, damage-only drawing** (same model as Kwil, `E:/Development/kwil/docs/decisions/0001-renderer.md`):
   - No frame loop. At idle: 0 presents, 0 wakeups, 0 % CPU.
   - Redraw only on input, a worker result, a window event or a timer the UI asked for.
   - Present only the damaged rectangles.
   - Scrolling is a pixel copy plus the newly exposed rows. On Windows this is a screen-to-screen `BitBlt` that RDP sends as a copy order.
6. **The CPU renderer is the remote path.** `softbuffer` with damage is a complete backend. In a remote session (Windows `SM_REMOTESESSION`, switched live on `WM_WTSSESSION_CHANGE`; a setting on macOS and Linux):
   - no animations, no smooth scrolling, no cursor blink;
   - no translucency, gradients or shadows in the chrome;
   - solid fills only, so the remote codec compresses them well.
   A GPU path may come later only if measurements prove it is faster, and never on the startup path.
7. **Never crash on user data.**
   - No `unwrap`/`expect` on I/O, Git output, repository content, settings or network data.
   - Git output is parsed defensively and bounded. Invalid UTF-8, huge lines and odd refs must not crash anything.
   - A panicking worker is caught and reported in the UI; the window stays alive.
   - Every crash is a P0 defect with a Pair Desk issue.
8. **Never lose user work.** Destructive operations (hard reset, discard, force push, deleting a branch or a stash, rewriting history) ask first, name exactly what will be lost, and offer an undo where Git allows one (reflog, a backup ref). ffwd never edits `.git` internals by hand; writes go through `git` or gitoxide with the same semantics as the CLI.
9. **Memory is bounded by the viewport.** History rows are compact (no per-row `String` for fields not yet shown where avoidable), diffs and blame stream in, and caches have caps.
10. **Measured, not assumed.**
    - Every budget in `docs/SPEC.md` §3 gets a benchmark or trace before it is claimed.
    - `FFWD_TRACE=1` prints startup and frame marks; `--snapshot` renders the real UI offscreen to a PNG.
    - A regression over 10 % against the recorded number is a defect.
    - Never claim a speed you didn't measure, and state the machine and commit.
11. **Small dependency graph.**
    - Every new dependency must earn its place. Its pull request states the crate count and the clean and incremental build time it adds.
    - No async runtime on the UI side. Workers use `std::thread` and channels; HTTP for AI goes through a small blocking client on a worker.
    - Heavy dependencies live in leaf crates so that UI edits don't recompile them.

## AI features

- **Off by default.** No data leaves the machine until the user configures an endpoint. Local endpoints come first: Ollama, llama.cpp, LM Studio, or anything OpenAI-compatible.
- Before anything is sent, the user sees exactly what will be sent (diff, files, messages). Secrets and ignored files are never included.
- AI calls never block the UI (rule 1). They stream, can be cancelled, and any result is a suggestion the user accepts, never an automatic write.

## Build (fastest iteration; details in `docs/BUILD.md`)

- **Toolchain:** pinned in `rust-toolchain.toml`. Use `rustup ... --no-self-update`; rustup self-updates belong to the owner.
- **Local Cargo config:** `.cargo/config.toml` is machine-local and gitignored; its template is `.cargo/config.example.toml`. A new worktree copies it before its first build. It sets:
  - the shared sccache at `E:/Development/_tools/sccache/0.19.0/.../sccache.exe`, with `SCCACHE_DIR=E:/sccache`;
  - `rust-lld` as the Windows linker;
  - build output in `E:/Development/_cache/cargo-target/ffwd`;
  - `jobs = 8`.
- **Profiles:** in `Cargo.toml`. Dependencies build optimised and cached by sccache; workspace crates stay incremental. Don't set `incremental = false`. With incremental on, sccache passes the workspace crates through uncached, which is fine: incremental rebuilds are the fast path for them.
- **Load:** no stress loops, full benchmarks or full suites on the owner PC. Use focused tests; full suites run on CI.
- **Agent-launched windows:**
  - Prefer `--snapshot <png>`, which renders offscreen and opens no window.
  - If a real window is needed, create it hidden and DWM-cloaked, without activation and without a taskbar entry (`--smoke`). Never put it on the owner's screen.
  - Record the PID and close only that process.

## Document map

- `docs/SPEC.md`: product scope, speed budgets, architecture. Read it before any work package.
- `docs/FEATURES.md`: the feature inventory, mapped to ffwd milestones.
- `docs/UI-DESIGN.md`: layout, interaction model, theme, keyboard map.
- `docs/BUILD.md`: build settings, sccache, linker, measured build times.
- `docs/RELEASING.md`, `docs/INSTALL.md`: releases (drafts only, the owner publishes), setup, updater. Never run `scripts/test-installer.ps1` or a built setup on the owner PC; they run on CI runners only.
- `docs/decisions/`: architecture decision records.
- `assets/logo/`: the SVG logo and its derived files.
- `scripts/`: owner test scripts, batch files on Windows (owner, 2026-11-13): `run.bat`, `snapshot.bat`, `bench-startup.bat`, `check.bat` (`.sh` twins for macOS/Linux). Agents set `FFWD_NO_OPEN=1` for `snapshot.bat` so no viewer opens on the owner's screen; never run `run.bat` (visible window).
