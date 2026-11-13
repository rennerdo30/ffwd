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

## Try the proof of concept

You need Rust (the toolchain installs itself from `rust-toolchain.toml`) and Git.

```sh
cargo run --release -- path/to/a/repo
```

| Key | Action |
|---|---|
| `↑ ↓` / `j k` | Move through history, files or diff |
| `Tab` / `Shift+Tab` | Switch between history, files and diff |
| `Ctrl+K` / `Ctrl+P` / `/` | Jump to a branch, tag or SHA, or find a command |
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
- [Architecture decisions](docs/decisions)
- [Contributor and agent rules, including the performance rules](AGENTS.md)

License: [MIT](LICENSE).
