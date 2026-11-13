//! Text: fontdue rasterisation with a coverage cache per (font, size, char).
//! Glyphs blend onto whatever is already in the canvas, so one cached
//! bitmap serves every background colour.

use crate::{Canvas, Rect};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontId(pub u8);

struct Glyph {
    xmin: i32,
    ymin: i32,
    w: usize,
    h: usize,
    advance: f32,
    cov: Vec<u8>,
}

pub struct TextEngine {
    fonts: Vec<fontdue::Font>,
    cache: HashMap<(u8, u32, char), Glyph>,
    /// Coverage lookup: a slight boost so light text on dark backgrounds
    /// keeps its weight with linear blending.
    lut: [u8; 256],
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    pub fn new() -> TextEngine {
        let mut lut = [0u8; 256];
        for (i, v) in lut.iter_mut().enumerate() {
            *v = ((i as f32 / 255.0).powf(0.82) * 255.0).round() as u8;
        }
        TextEngine { fonts: Vec::new(), cache: HashMap::new(), lut }
    }

    /// Loads the first readable font among `paths`. Returns `None` when
    /// none of them can be read or parsed.
    pub fn load_first(&mut self, paths: &[&str]) -> Option<FontId> {
        for p in paths {
            if let Ok(bytes) = std::fs::read(p)
                && let Some(id) = self.load_bytes(bytes)
            {
                return Some(id);
            }
        }
        None
    }

    pub fn load_bytes(&mut self, bytes: Vec<u8>) -> Option<FontId> {
        Some(self.add(Self::parse(bytes)?))
    }

    /// Parses a font without an engine, so faces can load on worker threads.
    pub fn parse(bytes: Vec<u8>) -> Option<fontdue::Font> {
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
    }

    pub fn add(&mut self, font: fontdue::Font) -> FontId {
        self.fonts.push(font);
        FontId((self.fonts.len() - 1) as u8)
    }

    fn glyph(&mut self, f: FontId, size: f32, c: char) -> &Glyph {
        let key = (f.0, (size * 64.0) as u32, c);
        let fonts = &self.fonts;
        self.cache.entry(key).or_insert_with(|| {
            let font = &fonts[f.0 as usize];
            let (m, cov) = font.rasterize(c, size);
            Glyph { xmin: m.xmin, ymin: m.ymin, w: m.width, h: m.height, advance: m.advance_width, cov }
        })
    }

    pub fn ascent(&self, f: FontId, size: f32) -> f32 {
        self.fonts[f.0 as usize].horizontal_line_metrics(size).map_or(size * 0.8, |m| m.ascent)
    }

    pub fn measure(&mut self, f: FontId, size: f32, text: &str) -> f32 {
        text.chars().map(|c| self.glyph(f, size, map_char(c)).advance).sum()
    }

    /// Draws `text` with its baseline at `y`, clipped to `max_w` pixels
    /// (an ellipsis replaces the overflow). Returns the end x.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        cv: &mut Canvas,
        f: FontId,
        size: f32,
        x: f32,
        y: f32,
        text: &str,
        color: u32,
        max_w: f32,
    ) -> f32 {
        let full = self.measure(f, size, text);
        if full <= max_w {
            return self.draw_run(cv, f, size, x, y, text.chars(), color);
        }
        let ell = self.glyph(f, size, '…').advance;
        let budget = max_w - ell;
        if budget <= 0.0 {
            return x;
        }
        let mut w = 0.0;
        let mut n = 0;
        for c in text.chars() {
            let a = self.glyph(f, size, map_char(c)).advance;
            if w + a > budget {
                break;
            }
            w += a;
            n += 1;
        }
        let end = self.draw_run(cv, f, size, x, y, text.chars().take(n), color);
        self.draw_run(cv, f, size, end, y, "…".chars(), color)
    }

    /// Draws text centred vertically in `r`, left aligned with `pad`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_in(&mut self, cv: &mut Canvas, f: FontId, size: f32, r: Rect, pad: f32, text: &str, color: u32) -> f32 {
        let base = r.y as f32 + (r.h as f32 + self.ascent(f, size) * 0.72) / 2.0;
        self.draw(cv, f, size, r.x as f32 + pad, base.round(), text, color, r.w as f32 - 2.0 * pad)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_run(
        &mut self,
        cv: &mut Canvas,
        f: FontId,
        size: f32,
        x: f32,
        y: f32,
        chars: impl Iterator<Item = char>,
        color: u32,
    ) -> f32 {
        let mut pen = x;
        let clip = cv.clip();
        for c in chars {
            let c = map_char(c);
            let lut = self.lut;
            let g = self.glyph(f, size, c);
            let gx = pen.round() as i32 + g.xmin;
            let gy = y as i32 - g.h as i32 - g.ymin;
            if gx > clip.right() {
                break;
            }
            for row in 0..g.h {
                let py = gy + row as i32;
                for col in 0..g.w {
                    let a = g.cov[row * g.w + col];
                    if a != 0 {
                        cv.put(gx + col as i32, py, color, lut[a as usize] as u32);
                    }
                }
            }
            pen += g.advance;
        }
        pen
    }
}

#[inline]
fn map_char(c: char) -> char {
    match c {
        '\t' => ' ',
        c if c.is_control() => '\u{FFFD}',
        c => c,
    }
}
