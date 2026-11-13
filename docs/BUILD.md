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

## CI

GitHub-hosted runners (Windows, macOS, Linux) run `cargo fmt --check`, `clippy -D warnings`, the tests and a release build, with sccache backed by the GitHub Actions cache. `cargo-deny` enforces the permissive licence list in `deny.toml`.

**Windows note:** release builds use the GUI subsystem, so a shell does not wait for them and their stderr is not shown. For `--snapshot`, `--smoke` and `--help` from a terminal, use the debug build or `Start-Process -Wait -RedirectStandardError`.
