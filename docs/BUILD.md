# Building ffwd

```sh
cargo run --release -- path/to/repo
cargo run --release -- path/to/repo --snapshot shot.png   # offscreen render, no window
FFWD_TRACE=1 cargo run --release -- path/to/repo           # startup and present marks
```

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.100.0, the stable release from 2026-11-12). `rustup` installs it automatically.

## Fastest iteration

| Setting | Where | Why |
|---|---|---|
| `[profile.dev] debug = "line-tables-only"` | `Cargo.toml` | Small debug info: faster codegen and linking, a smaller target folder |
| `[profile.dev.package."*"] opt-level = 2, debug = false` | `Cargo.toml` | Dependencies run fast in dev and build once (then come from sccache) |
| `[profile.dev.package.ffwd-render] opt-level = 2` | `Cargo.toml` | The CPU renderer paints every pixel; unoptimised, it would distort interactive checks |
| Incremental stays on for workspace crates | default | sccache passes these through uncached, which is fine: incremental is the fast path for them |
| `rustc-wrapper` = shared sccache | `.cargo/config.toml` (local) | Dependencies compile once per machine, shared across worktrees and projects |
| `linker = "rust-lld.exe"` (Windows) | `.cargo/config.toml` (local) | Several times faster linking than `link.exe`. Linux already uses `rust-lld` by default; macOS's `ld-prime` is fast |
| `target-dir` outside the repository | `.cargo/config.toml` (local) | One build folder per project, outside the source tree |
| `jobs = 8` | `.cargo/config.toml` (local) | Leaves headroom on a shared PC |
| `[profile.release] lto = "thin", codegen-units = 1` | `Cargo.toml` | Fast, small release binaries; unwinding stays on so worker panics are caught |

`.cargo/config.toml` is machine-local and gitignored. Copy `.cargo/config.example.toml` and adjust its paths. Without it, a plain `cargo build` works too.

**Optional on nightly:** the Cranelift backend (`-Zcodegen-backend=cranelift`) for even faster debug builds. It is never required.

## Measured (owner PC, 2026-11-13)

| Build | Time |
|---|---|
| Incremental dev rebuild after a `ffwd-app` edit | 1.0 s |
| Incremental dev rebuild after a `ffwd-core` edit | 0.9 s |
| Clean dev build into a fresh target folder, sccache warm | 2.8 s |
| Release build (dependencies already built) | 6 to 11 s |
| Release binary (Windows x64) | 1.3 MB |
| Crates in the graph (Windows) | about 35 |

## Fast UI iteration (FF-63)

Two speed tiers for UI work, measured on the owner PC (2026-11-27, Rust 1.100.0):

- **Tier 0 — Live design tokens over `--serve` (~20–36 ms, 0 ms compile):**
  ```sh
  ffwd --dump-tokens > tokens.toml
  pwsh scripts/ui-snap.ps1 -Serve -Port 17321
  pwsh scripts/ui-snap.ps1 -Send "tokens tokens.toml; screen history; snap out.png; audit" -Port 17321
  ```
  Keeps parsed fonts and the synthetic in-memory repository hot in memory on `127.0.0.1`. Token reloads take ~0.3 ms; full 1600×1000 PNG capture takes ~20–35 ms; programmatic `--audit` (WCAG contrast, pane/hit-rect overlaps, ellipsis count, partial-paint parity) takes < 1 ms.
- **Tier 1 — Sub-second Rust UI code → multi-screen batch capture:**
  ```sh
  pwsh scripts/ui-snap.ps1                         # dev-ui build + all 12 screens × dark/light (24 PNGs)
  pwsh scripts/ui-snap.ps1 -Only sheet -Audit      # component reference sheet + JSON audit
  pwsh scripts/ui-snap.ps1 -Cranelift -Only history # optional nightly Cranelift codegen
  ```
  `[profile.dev-ui]` disables PDB emission (`debug = 0`) while keeping dependencies and `ffwd-render` at `opt-level = 2`. Colour getters in `theme.rs` are `inline(never)` in debug builds so colour tweaks compile in **~0.99 s** (down from 3.08 s; **0.95 s** on Cranelift, `ui.rs` edits **1.13 s** on Cranelift). `--snapshot-ui` renders all **24 demo screens (dark + light) + audit in ~530 ms** (~22 ms/screen, 1.2 ms paint, zero `git.exe` spawns).

## CI

GitHub-hosted runners (Windows, macOS, Linux) run `cargo fmt --check`, `clippy -D warnings`, the tests and a release build, with sccache backed by the GitHub Actions cache. `cargo-deny` enforces the permissive licence list in `deny.toml`.

**Windows note:** release builds use the GUI subsystem, so a shell does not wait for them and their stderr is not shown. For `--snapshot`, `--smoke` and `--help` from a terminal, use the debug build or `Start-Process -Wait -RedirectStandardError`.
