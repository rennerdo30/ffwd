# ffwd-render

A small CPU canvas, written for the [ffwd](https://github.com/rennerdo30/ffwd) Git client:

- solid fills and anti-aliased vector shapes (lines, rounded rectangles, circles);
- text through [fontdue](https://crates.io/crates/fontdue), with a coverage cache per font, size and character. It ships no fonts: load them from a path or from bytes;
- everything is drawn into a plain `0x00RRGGBB` `u32` buffer, so it works with any present path (softbuffer, an offscreen PNG, a remote framebuffer).

It has no windowing and no GPU code. Its only dependency is fontdue.

Licence: MIT.
