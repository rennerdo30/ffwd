<p align="center"><img src="assets/logo/ffwd-wordmark.svg" alt="ffwd" width="380"></p>

<p align="center"><b>A Git client that is fast at everything, everywhere, including over remote desktop.</b><br>
Windows · macOS · Linux · written in Rust</p>

---

**Status: proof of concept.** ffwd is at the start. The history view below works and is measured; staging, committing and everything else are still ahead. See the [specification](docs/SPEC.md) and the [feature plan](docs/FEATURES.md).

![ffwd proof of concept: commit graph, details and diff](docs/images/poc-history.png)

*The real UI, rendered offscreen by `ffwd --snapshot` from a public repository. It is not a mock-up.*

## Why

Desktop Git clients are often slow. They render late, stutter on big repositories and turn sluggish over remote desktop. ffwd is built the other way round:

- **The first frame comes first.** In the proof of concept the window and chrome are on screen in about 25 ms, and history streams in behind them.
- **Input answers in the same frame.** Selecting a commit shows its header immediately; files and diff follow.
- **Nothing blocks the UI.** Every Git call runs on a worker thread.
- **It is built for remote desktop.**
  - There is no frame loop and nothing is drawn at idle.
  - Each present sends only what changed.
  - Scrolling moves pixels instead of repainting them.
  - The CPU renderer is a complete backend, so sessions without a GPU stay fast.
- **Your work is safe.** Destructive actions will ask first and be undoable.
- **AI on your terms (planned).** Point ffwd at any OpenAI-compatible endpoint, preferably a local one (Ollama, llama.cpp, LM Studio), for commit messages, explanations and reviews. It is off until you configure it, and you see exactly what is sent.

## Install

No release is published yet. Once one is, Windows gets a per-user setup that updates itself, and macOS and Linux get archives: see [Installing ffwd](docs/INSTALL.md).

## Try the proof of concept

You need Rust (the toolchain installs itself from `rust-toolchain.toml`) and Git. On Windows, double-click or run the scripts:

| Script | What it does |
|---|---|
| `scripts\run.bat [repo] [--release] [--pull] [--trace]` | Build and open ffwd on a repository (this one by default) |
| `scripts\snapshot.bat [repo] [--select N] [--focus diff] [--palette Q]` | Render the UI offscreen to `artifacts\snapshot.png` and open it |
| `scripts\bench-startup.bat [repo] [runs]` | Measure startup in a hidden window, one JSON line per run |
| `scripts\check.bat` | The CI checks: format, clippy, tests |
| `scripts\package.bat` | Build the Windows setup, the zip and `SHA256SUMS.txt` into `artifacts\installer` |

On macOS and Linux: `scripts/run.sh` and `scripts/check.sh`, or `cargo run --release -- path/to/a/repo`.

| Key | Action |
|---|---|
| `↑ ↓` / `j k` | Move through history, files or diff |
| `Tab` / `Shift+Tab` | Switch between history, files and diff |
| `Ctrl+K` / `Ctrl+P` / `/` | Jump to a branch, tag or SHA, or find a command |
| `Ctrl+O` / `Ctrl+T` | Open another repository in a tab |
| `Ctrl+Tab` / `Ctrl+W` | Next tab / close tab |
| `Ctrl+Shift+F` / `L` / `K` | Fetch / Pull / Push |
| `Esc` | Back to history |

Offscreen rendering and startup traces:

```sh
ffwd path/to/repo --snapshot shot.png --size 1600x1000 --select 3
FFWD_TRACE=1 ffwd path/to/repo
```

## Project

- [Specification and speed budgets](docs/SPEC.md)
- [UI design](docs/UI-DESIGN.md)
- [Feature plan, the feature plan](docs/FEATURES.md)
- [Building and fast iteration](docs/BUILD.md)
- [Releasing and updates](docs/RELEASING.md)
- [Architecture decisions](docs/decisions)
- [Contributor and agent rules, including the performance rules](AGENTS.md)

License: [MIT](LICENSE).
