//! CPU canvas for the ffwd UI: solid fills, anti-aliased vector shapes and
//! text, drawn into a `0x00RRGGBB` pixel buffer. The same buffer feeds the
//! window (softbuffer, damage-only presents) and offscreen snapshots.

mod text;

pub use text::{FontId, TextEngine};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(self) -> i32 {
        self.y + self.h
    }
    pub fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }
    pub fn intersect(self, o: Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect { x, y, w: (r - x).max(0), h: (b - y).max(0) }
    }
    pub fn inset(self, dx: i32, dy: i32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, w: self.w - 2 * dx, h: self.h - 2 * dy }
    }
    pub fn area(self) -> u64 {
        if self.is_empty() { 0 } else { self.w as u64 * self.h as u64 }
    }
}

/// A view into a pixel buffer with a clip rectangle. Every primitive clips,
/// so a paint routine can never write outside its damage rectangle.
pub struct Canvas<'a> {
    px: &'a mut [u32],
    stride: usize,
    height: usize,
    clip: Rect,
}

#[inline]
pub fn blend(dst: u32, src: u32, a: u32) -> u32 {
    if a >= 255 {
        return src;
    }
    if a == 0 {
        return dst;
    }
    let inv = 255 - a;
    let rb = ((dst & 0xff00ff) * inv + (src & 0xff00ff) * a + 0x800080) >> 8 & 0xff00ff;
    let g = ((dst & 0x00ff00) * inv + (src & 0x00ff00) * a + 0x008000) >> 8 & 0x00ff00;
    rb | g
}

/// Mixes two colours, `t` in 0..=255 (0 = a, 255 = b). Used for theme
/// derivation at startup, not per pixel.
pub fn mix(a: u32, b: u32, t: u32) -> u32 {
    blend(a, b, t.min(255))
}

impl<'a> Canvas<'a> {
    pub fn new(px: &'a mut [u32], width: usize, height: usize) -> Canvas<'a> {
        assert!(px.len() >= width * height, "canvas buffer too small");
        Canvas { px, stride: width, height, clip: Rect::new(0, 0, width as i32, height as i32) }
    }

    pub fn width(&self) -> usize {
        self.stride
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.stride as i32, self.height as i32)
    }
    pub fn clip(&self) -> Rect {
        self.clip
    }
    /// Restricts drawing to `r` (within the buffer) and returns the previous clip.
    pub fn set_clip(&mut self, r: Rect) -> Rect {
        let old = self.clip;
        self.clip = r.intersect(self.bounds());
        old
    }
    pub fn pixels(&self) -> &[u32] {
        self.px
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, color: u32, a: u32) {
        if self.clip.contains(x, y) {
            let i = y as usize * self.stride + x as usize;
            self.px[i] = blend(self.px[i], color, a);
        }
    }

    /// Draws an RGBA image (`w`×`h`, 8 bits per channel) scaled into `dst`
    /// with nearest-neighbour sampling, blended over what is there.
    pub fn draw_rgba_scaled(&mut self, dst: Rect, w: usize, h: usize, rgba: &[u8]) {
        if w == 0 || h == 0 || rgba.len() < w * h * 4 || dst.is_empty() {
            return;
        }
        let r = dst.intersect(self.clip);
        if r.is_empty() {
            return;
        }
        for y in r.y..r.bottom() {
            let sy = ((y - dst.y) as usize * h / dst.h as usize).min(h - 1);
            let o = y as usize * self.stride;
            for x in r.x..r.right() {
                let sx = ((x - dst.x) as usize * w / dst.w as usize).min(w - 1);
                let p = (sy * w + sx) * 4;
                let (cr, cg, cb, ca) = (rgba[p] as u32, rgba[p + 1] as u32, rgba[p + 2] as u32, rgba[p + 3] as u32);
                let i = o + x as usize;
                self.px[i] = blend(self.px[i], cr << 16 | cg << 8 | cb, ca);
            }
        }
    }

    pub fn fill(&mut self, r: Rect, color: u32) {
        let r = r.intersect(self.clip);
        if r.is_empty() {
            return;
        }
        for y in r.y..r.bottom() {
            let o = y as usize * self.stride;
            self.px[o + r.x as usize..o + r.right() as usize].fill(color);
        }
    }

    pub fn fill_alpha(&mut self, r: Rect, color: u32, a: u32) {
        let r = r.intersect(self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                self.put(x, y, color, a);
            }
        }
    }

    /// 1 px outline inside `r`.
    pub fn stroke_rect(&mut self, r: Rect, color: u32) {
        self.fill(Rect::new(r.x, r.y, r.w, 1), color);
        self.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1), color);
        self.fill(Rect::new(r.x, r.y, 1, r.h), color);
        self.fill(Rect::new(r.right() - 1, r.y, 1, r.h), color);
    }

    /// Filled rounded rectangle with anti-aliased corners.
    pub fn fill_round(&mut self, r: Rect, radius: f32, color: u32) {
        let rad = radius.min(r.w as f32 / 2.0).min(r.h as f32 / 2.0).max(0.0);
        let ri = rad.ceil() as i32;
        // Straight middle band and the two side bands without corners.
        self.fill(Rect::new(r.x, r.y + ri, r.w, r.h - 2 * ri), color);
        self.fill(Rect::new(r.x + ri, r.y, r.w - 2 * ri, ri), color);
        self.fill(Rect::new(r.x + ri, r.bottom() - ri, r.w - 2 * ri, ri), color);
        let corners = [
            (r.x, r.y, r.x as f32 + rad, r.y as f32 + rad),
            (r.right() - ri, r.y, r.right() as f32 - rad, r.y as f32 + rad),
            (r.x, r.bottom() - ri, r.x as f32 + rad, r.bottom() as f32 - rad),
            (r.right() - ri, r.bottom() - ri, r.right() as f32 - rad, r.bottom() as f32 - rad),
        ];
        for (x0, y0, cx, cy) in corners {
            for y in y0..y0 + ri {
                for x in x0..x0 + ri {
                    let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    let cov = (rad - d + 0.5).clamp(0.0, 1.0);
                    self.put(x, y, color, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Rounded rectangle outline: a filled outer shape with the inner shape
    /// filled in `inner` (cheap and exact for solid backgrounds).
    pub fn round_border(&mut self, r: Rect, radius: f32, border: u32, inner: u32) {
        self.fill_round(r, radius, border);
        self.fill_round(r.inset(1, 1), (radius - 1.0).max(0.0), inner);
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: u32) {
        let x0 = (cx - radius - 1.0).floor() as i32;
        let x1 = (cx + radius + 1.0).ceil() as i32;
        let y0 = (cy - radius - 1.0).floor() as i32;
        let y1 = (cy + radius + 1.0).ceil() as i32;
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let cov = (radius - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.put(x, y, color, (cov * 255.0) as u32);
                }
            }
        }
    }

    pub fn ring(&mut self, cx: f32, cy: f32, radius: f32, width: f32, color: u32) {
        let x0 = (cx - radius - 1.0).floor() as i32;
        let x1 = (cx + radius + 1.0).ceil() as i32;
        let y0 = (cy - radius - 1.0).floor() as i32;
        let y1 = (cy + radius + 1.0).ceil() as i32;
        let mid = radius - width / 2.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let cov = (width / 2.0 - (d - mid).abs() + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.put(x, y, color, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Anti-aliased stroke along a polyline. Coverage is the minimum distance
    /// to any segment, so joints never double-blend.
    pub fn stroke(&mut self, pts: &[(f32, f32)], width: f32, color: u32) {
        if pts.len() < 2 {
            return;
        }
        let hw = width / 2.0;
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in pts {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        let bb = Rect::new(
            (minx - hw - 1.0).floor() as i32,
            (miny - hw - 1.0).floor() as i32,
            ((maxx - minx) + width + 3.0).ceil() as i32,
            ((maxy - miny) + width + 3.0).ceil() as i32,
        )
        .intersect(self.clip);
        for y in bb.y..bb.bottom() {
            let py = y as f32 + 0.5;
            for x in bb.x..bb.right() {
                let px = x as f32 + 0.5;
                let mut d = f32::MAX;
                for s in pts.windows(2) {
                    d = d.min(seg_dist(px, py, s[0], s[1]));
                }
                let cov = (hw - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.put(x, y, color, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Anti-aliased filled triangle (any winding).
    pub fn fill_triangle(&mut self, a: (f32, f32), b: (f32, f32), c: (f32, f32), color: u32) {
        let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if area.abs() < f32::EPSILON {
            return;
        }
        let s = area.signum();
        let edges = [(a, b), (b, c), (c, a)];
        let bb = Rect::new(
            a.0.min(b.0).min(c.0).floor() as i32 - 1,
            a.1.min(b.1).min(c.1).floor() as i32 - 1,
            (a.0.max(b.0).max(c.0) - a.0.min(b.0).min(c.0)).ceil() as i32 + 3,
            (a.1.max(b.1).max(c.1) - a.1.min(b.1).min(c.1)).ceil() as i32 + 3,
        )
        .intersect(self.clip);
        for y in bb.y..bb.bottom() {
            let py = y as f32 + 0.5;
            for x in bb.x..bb.right() {
                let px = x as f32 + 0.5;
                // Signed distance to the nearest edge, positive inside.
                let d = edges
                    .iter()
                    .map(|&(p, q)| {
                        let (ex, ey) = (q.0 - p.0, q.1 - p.1);
                        s * (ex * (py - p.1) - ey * (px - p.0)) / (ex * ex + ey * ey).sqrt()
                    })
                    .fold(f32::MAX, f32::min);
                let cov = (d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.put(x, y, color, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Cubic Bézier stroke, flattened to a short polyline.
    pub fn bezier(&mut self, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), width: f32, color: u32) {
        const N: usize = 12;
        let mut pts = [(0.0f32, 0.0f32); N + 1];
        for (i, p) in pts.iter_mut().enumerate() {
            let t = i as f32 / N as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            *p = (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1);
        }
        self.stroke(&pts, width, color);
    }

    /// Copies a horizontal band of rows inside `area` by `dy` pixels
    /// (negative = content moves up). Used for scrolling by copy.
    pub fn scroll(&mut self, area: Rect, dy: i32) {
        let area = area.intersect(self.bounds());
        if dy == 0 || dy.abs() >= area.h {
            return;
        }
        let (x0, x1) = (area.x as usize, area.right() as usize);
        let rows: Box<dyn Iterator<Item = i32>> =
            if dy < 0 { Box::new(area.y..area.bottom() + dy) } else { Box::new((area.y + dy..area.bottom()).rev()) };
        for y in rows {
            let src = (y - dy) as usize * self.stride;
            let dst = y as usize * self.stride;
            self.px.copy_within(src + x0..src + x1, dst + x0);
        }
    }
}

fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_by_copy_equals_redraw() {
        let (w, h) = (8usize, 10usize);
        let draw = |px: &mut Vec<u32>, top: u32| {
            for y in 0..h {
                px[y * w..y * w + w].fill(top + y as u32);
            }
        };
        let mut a = vec![0; w * h];
        draw(&mut a, 0);
        let mut c = Canvas::new(&mut a, w, h);
        c.scroll(Rect::new(0, 0, w as i32, h as i32), -3);
        for y in 7..h {
            c.fill(Rect::new(0, y as i32, w as i32, 1), 3 + y as u32);
        }
        let mut b = vec![0; w * h];
        draw(&mut b, 3);
        assert_eq!(a, b);
    }

    #[test]
    fn primitives_respect_clip() {
        let mut px = vec![0u32; 16 * 16];
        let mut c = Canvas::new(&mut px, 16, 16);
        c.set_clip(Rect::new(4, 4, 4, 4));
        c.fill(Rect::new(0, 0, 16, 16), 0xffffff);
        c.fill_circle(8.0, 8.0, 7.0, 0xff0000);
        c.stroke(&[(0.0, 0.0), (16.0, 16.0)], 3.0, 0x00ff00);
        for y in 0..16 {
            for x in 0..16 {
                let inside = (4..8).contains(&x) && (4..8).contains(&y);
                assert_eq!(px[y * 16 + x] != 0, inside, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn images_scale_and_blend() {
        let mut px = vec![0u32; 4 * 4];
        let mut cv = Canvas::new(&mut px, 4, 4);
        // 2x2: red, transparent / green, blue (half alpha).
        let img = [255, 0, 0, 255, 0, 0, 0, 0, 0, 255, 0, 255, 0, 0, 255, 128];
        cv.draw_rgba_scaled(Rect::new(0, 0, 4, 4), 2, 2, &img);
        assert_eq!(cv.pixels()[0], 0xff0000);
        assert_eq!(cv.pixels()[3], 0x000000, "transparent leaves the background");
        assert_eq!(cv.pixels()[12], 0x00ff00);
        assert_eq!(cv.pixels()[15] & 0xff, 0x80, "half alpha blends");
    }
}
