//! The window icon, drawn from the logo's geometry at startup
//! (`assets/logo/ffwd-mark.svg`), so the binary needs no image files.

use ffwd_render::{Canvas, Rect, mix};

/// RGBA pixels of the mark at `n`×`n`.
pub fn mark_rgba(n: usize) -> Vec<u8> {
    let s = n as f32 / 512.0;
    let mut tile = vec![0u32; n * n];
    Canvas::new(&mut tile, n, n).fill_round(Rect::new(0, 0, n as i32, n as i32), 116.0 * s, 0xffffff);
    let mut glyph = vec![0u32; n * n];
    {
        let mut g = Canvas::new(&mut glyph, n, n);
        // Same geometry as the SVG: scaled 1.12 around the centre.
        let t = |x: f32, y: f32| ((256.0 + (x - 260.0) * 1.12) * s, (256.0 + (y - 256.0) * 1.12) * s);
        let (cx, cy) = t(116.0, 256.0);
        g.fill_circle(cx, cy, 30.0 * 1.12 * s, 0xffffff);
        g.stroke(&[t(146.0, 256.0), t(190.0, 256.0)], 26.0 * 1.12 * s, 0xffffff);
        for x0 in [196.0, 306.0] {
            let (a, b, c) = (t(x0, 162.0), t(x0 + 98.0, 256.0), t(x0, 350.0));
            g.fill_triangle(a, b, c, 0xffffff);
            // The SVG's round stroke joins: an outline of the same width.
            g.stroke(&[a, b, c, a], 30.0 * 1.12 * s, 0xffffff);
        }
    }
    let mut out = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            // Diagonal gradient #FF8A3D → #FF5A4E → #FF2E7E.
            let t = (x + y) as f32 / (2 * n) as f32;
            let base = if t < 0.55 {
                mix(0xff8a3d, 0xff5a4e, (t / 0.55 * 255.0) as u32)
            } else {
                mix(0xff5a4e, 0xff2e7e, ((t - 0.55) / 0.45 * 255.0) as u32)
            };
            let px = mix(base, 0xffffff, glyph[i] & 0xff);
            out.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8, (tile[i] & 0xff) as u8]);
        }
    }
    out
}

pub fn window_icon() -> Option<winit::window::Icon> {
    const N: usize = 64;
    winit::window::Icon::from_rgba(mark_rgba(N), N as u32, N as u32).ok()
}
