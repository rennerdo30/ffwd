//! Images for the diff (FF-41): a changed PNG's old and new version,
//! decoded on a worker. Decoding is bounded (32 MB of file, 64 MB of
//! pixels, the png crate's own limits) and large images are shrunk to at
//! most 1,600 px on their long side, so memory follows the view.

use ffwd_core::Git;

/// The longest side kept after decoding.
const MAX_SIDE: usize = 1600;
/// The most file bytes read.
const MAX_FILE: usize = 32 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Img {
    pub w: usize,
    pub h: usize,
    /// 8-bit RGBA, `w`×`h`.
    pub rgba: Vec<u8>,
    /// The size in the file.
    pub orig: (u32, u32),
    pub bytes: usize,
}

/// Whether ffwd shows `path` as an image.
pub fn is_image(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".png")
}

/// Decodes a PNG to RGBA, shrunk to fit [`MAX_SIDE`].
pub fn decode_png(bytes: &[u8]) -> Result<Img, String> {
    let mut dec = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: 64 << 20 });
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| format!("not a readable PNG: {e}"))?;
    let size = reader.output_buffer_size().ok_or("the image is too large")?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("not a readable PNG: {e}"))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("an indexed PNG was not expanded".into()),
    };
    let line = info.line_size;
    // Shrink: sample every `step`-th pixel (nearest) so the long side fits.
    let step = w.max(h).div_ceil(MAX_SIDE).max(1);
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut rgba = Vec::with_capacity(ow * oh * 4);
    for oy in 0..oh {
        let row = &buf[(oy * step) * line..];
        for ox in 0..ow {
            let p = &row[(ox * step) * channels..];
            let px = match channels {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            };
            rgba.extend_from_slice(&px);
        }
    }
    Ok(Img { w: ow, h: oh, rgba, orig: (info.width, info.height), bytes: bytes.len() })
}

/// Reads `rev:path` from the repository and decodes it.
pub fn load(git: &Git, rev: &str, path: &str) -> Result<Img, String> {
    let spec = format!("{rev}:{path}");
    let b = git.output_limited(&["cat-file", "blob", &spec], MAX_FILE).map_err(|e| e.to_string())?;
    if b.len() >= MAX_FILE {
        return Err("the image file is larger than 32 MB".into());
    }
    decode_png(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(color);
            e.set_depth(png::BitDepth::Eight);
            let mut wr = e.write_header().unwrap();
            wr.write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn decodes_and_shrinks() {
        let rgb = png_bytes(2, 1, png::ColorType::Rgb, &[255, 0, 0, 0, 0, 255]);
        let i = decode_png(&rgb).unwrap();
        assert_eq!((i.w, i.h, i.orig), (2, 1, (2, 1)));
        assert_eq!(i.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        let gray = png_bytes(1, 1, png::ColorType::GrayscaleAlpha, &[100, 50]);
        assert_eq!(decode_png(&gray).unwrap().rgba, [100, 100, 100, 50]);
        // 4000 px wide shrinks to at most 1,600.
        let wide = png_bytes(4000, 2, png::ColorType::Grayscale, &vec![7u8; 8000]);
        let i = decode_png(&wide).unwrap();
        assert!(i.w <= MAX_SIDE && i.orig == (4000, 2), "{} {:?}", i.w, i.orig);
        assert!(decode_png(b"not a png").is_err());
        assert!(is_image("a/B.PNG") && !is_image("a.txt"));
    }
}
