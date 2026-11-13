//! The ffwd mark, drawn from the logo's geometry (`assets/logo/ffwd-mark.svg`):
//! a commit, its branch line and the fast-forward it takes, dark on a flat
//! amber tile. The same code draws the window icon, the installer icon
//! (`--icon-png`) and the mark in the top bar, so they can never differ.

use crate::theme::{ACCENT, ACCENT_TEXT};
use ffwd_render::{Canvas, Rect, mix};

/// The glyph of an `n`-pixel mark whose top-left corner is at (`x0`, `y0`).
fn glyph(g: &mut Canvas, x0: f32, y0: f32, n: f32, color: u32) {
    let s = n / 512.0;
    // Same geometry as the SVG: scaled 1.12 around the centre.
    let t = |x: f32, y: f32| (x0 + (256.0 + (x - 260.0) * 1.12) * s, y0 + (256.0 + (y - 256.0) * 1.12) * s);
    let (cx, cy) = t(116.0, 256.0);
    g.fill_circle(cx, cy, 30.0 * 1.12 * s, color);
    g.stroke(&[t(146.0, 256.0), t(190.0, 256.0)], 26.0 * 1.12 * s, color);
    for xa in [196.0, 306.0] {
        let (a, b, c) = (t(xa, 162.0), t(xa + 98.0, 256.0), t(xa, 350.0));
        g.fill_triangle(a, b, c, color);
        // The SVG's round stroke joins: an outline of the same width.
        g.stroke(&[a, b, c, a], 30.0 * 1.12 * s, color);
    }
}

/// Draws the mark into `r` (square) on the canvas.
pub fn draw_mark(cv: &mut Canvas, r: Rect) {
    let n = r.w.min(r.h) as f32;
    cv.fill_round(Rect::new(r.x, r.y, n as i32, n as i32), 116.0 / 512.0 * n, ACCENT);
    glyph(cv, r.x as f32, r.y as f32, n, ACCENT_TEXT);
}

/// RGBA pixels of the mark at `n`×`n` (transparent outside the tile).
pub fn mark_rgba(n: usize) -> Vec<u8> {
    let mut tile = vec![0u32; n * n];
    Canvas::new(&mut tile, n, n).fill_round(Rect::new(0, 0, n as i32, n as i32), 116.0 / 512.0 * n as f32, 0xffffff);
    let mut cover = vec![0u32; n * n];
    glyph(&mut Canvas::new(&mut cover, n, n), 0.0, 0.0, n as f32, 0xffffff);
    let mut out = Vec::with_capacity(n * n * 4);
    for i in 0..n * n {
        let px = mix(ACCENT, ACCENT_TEXT, cover[i] & 0xff);
        out.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8, (tile[i] & 0xff) as u8]);
    }
    out
}

fn icon(n: usize) -> Option<winit::window::Icon> {
    winit::window::Icon::from_rgba(mark_rgba(n), n as u32, n as u32).ok()
}

/// The pixel sizes of the small (title bar) and big (taskbar, Alt+Tab)
/// icons at `scale`: 16 and 32 logical pixels, drawn at the real size so
/// Windows never scales a bitmap.
pub fn sizes(scale: f64) -> (usize, usize) {
    let s = if scale.is_finite() { scale.clamp(1.0, 4.0) } else { 1.0 };
    ((16.0 * s).round() as usize, (32.0 * s).round() as usize)
}

/// Gives the window its icons for `scale`. Without the big one, Windows
/// shows the taskbar button with a scaled small icon or the exe default.
pub fn apply(window: &winit::window::Window, scale: f64) {
    let (small, big) = sizes(scale);
    window.set_window_icon(icon(small));
    #[cfg(windows)]
    {
        use winit::platform::windows::WindowExtWindows;
        window.set_taskbar_icon(icon(big));
    }
    #[cfg(not(windows))]
    let _ = big;
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_sizes_follow_the_scale() {
        assert_eq!(super::sizes(1.0), (16, 32));
        assert_eq!(super::sizes(1.5), (24, 48));
        assert_eq!(super::sizes(2.0), (32, 64));
        assert_eq!(super::sizes(f64::NAN), (16, 32));
    }
}
