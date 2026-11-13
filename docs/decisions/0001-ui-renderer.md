# 0001: UI renderer: Kwil's path B (winit + softbuffer, own drawing)

Status: proposed 2026-11-13, carried over from Kwil, where the owner accepted it on 2026-10-23 based on measurements.

## Decision

ffwd draws its UI itself, on the CPU:

- `winit` provides windows and input; `softbuffer` presents pixels with damage rectangles.
- `ffwd-render` provides fills, anti-aliased shapes and text (fontdue for now).
- There is no frame loop. Each change records damage, and a present sends only those rectangles.
- Scrolling moves the pixels already on screen (on Windows, a screen-to-screen `BitBlt`) and draws only the newly exposed rows.

## Why

- **Kwil measured both paths on the owner's PC** ([Kwil 0001](https://github.com/rennerdo30/kwil/blob/main/docs/decisions/0001-renderer.md)). Compared with gpui (Zed's GPU UI), path B reached the first frame in 37 ms against 173 ms, had 0 idle wakeups against about 160 per second, and presented one line per key press instead of the whole window. It also builds in seconds instead of minutes.
- **GPU device creation alone takes 175 ms or more** on Windows, even for DX12 only, so the first frame has to come from the CPU anyway.
- **Remote desktop is a core use case** (owner, 2026-11-13). Over RDP without GPU policy, Direct3D runs in software (WARP). Damage-only CPU presents and copy-scrolling are what RDP and VNC encode cheaply.
- **Slowness and late rendering in existing Git clients is the problem ffwd exists to solve**, so the renderer must have no frame loop and no full-window repaint per input.

## PoC evidence (2026-11-13, owner PC, release build)

| Measure | Result |
|---|---|
| Launch to first frame, 2160×1350 px window | 23 to 34 ms over 10 runs (`ffwd --smoke`) |
| Full repaint | 3.1 to 4.6 ms |
| Key Down in the history | 2 rows, the details pane and the status bar, 0.4 ms paint |
| Idle | no presents (the event loop waits) |
| Crates in the dependency graph (Windows) | about 35 |
| Release binary | 1.3 MB |

**Trace pitfall:** writing each `FFWD_TRACE` mark straight to a redirected stderr cost about 17 ms per line and doubled the measured startup. Marks are now buffered and printed at exit.

## Costs

The following are ours to build:

- text input and IME;
- accessibility (`accesskit`);
- shaping beyond simple left-to-right text;
- widgets and layout.

fontdue parses every outline when a font loads. The three faces load in parallel and are ready 12 to 18 ms after launch, about 6 ms after the window. Swapping it for a lazy rasteriser is an option if startup needs it.

## Not measured yet

- real RDP and VNC sessions (bytes on the wire per action);
- macOS and Linux (Wayland damage);
- a 1M-commit repository.
