//! Images for the diff (FF-41): a changed PNG's or JPEG's old and new
//! version, decoded on a worker. Decoding is bounded (32 MB of file,
//! 64 MB of pixels, the decoders' own limits) and large images are shrunk
//! to at most 1,600 px on their long side, so memory follows the view.

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
    let p = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".jpe", ".jfif"].iter().any(|e| p.ends_with(e))
}

/// Decodes a PNG or a JPEG, told apart by their first bytes.
pub fn decode(bytes: &[u8]) -> Result<Img, String> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) { decode_jpeg(bytes) } else { decode_png(bytes) }
}

/// The pixels of a decoded image, `channels` bytes each, rows `line`
/// bytes apart, sampled down (nearest) so the long side fits
/// [`MAX_SIDE`]; `px` turns one pixel's bytes into RGBA.
fn shrink(
    w: usize,
    h: usize,
    channels: usize,
    line: usize,
    buf: &[u8],
    px: impl Fn(&[u8]) -> [u8; 4],
) -> Result<(usize, usize, Vec<u8>), String> {
    if w == 0 || h == 0 || buf.len() < (h - 1) * line + w * channels {
        return Err("the image data is incomplete".into());
    }
    let step = w.max(h).div_ceil(MAX_SIDE).max(1);
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut rgba = Vec::with_capacity(ow * oh * 4);
    for oy in 0..oh {
        let row = &buf[(oy * step) * line..];
        for ox in 0..ow {
            rgba.extend_from_slice(&px(&row[(ox * step) * channels..]));
        }
    }
    Ok((ow, oh, rgba))
}

/// Decodes a JPEG (baseline or progressive; gray, colour or CMYK) to
/// RGBA, shrunk to fit [`MAX_SIDE`].
pub fn decode_jpeg(bytes: &[u8]) -> Result<Img, String> {
    use jpeg_decoder::PixelFormat;
    let mut dec = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_max_decoding_buffer_size(64 << 20);
    let buf = dec.decode().map_err(|e| format!("not a readable JPEG: {e}"))?;
    let info = dec.info().ok_or("not a readable JPEG")?;
    let (w, h) = (info.width as usize, info.height as usize);
    let (ow, oh, rgba) = match info.pixel_format {
        PixelFormat::L8 => shrink(w, h, 1, w, &buf, |p| [p[0], p[0], p[0], 255]),
        // 16-bit gray, big-endian: the high byte.
        PixelFormat::L16 => shrink(w, h, 2, w * 2, &buf, |p| [p[0], p[0], p[0], 255]),
        PixelFormat::RGB24 => shrink(w, h, 3, w * 3, &buf, |p| [p[0], p[1], p[2], 255]),
        PixelFormat::CMYK32 => shrink(w, h, 4, w * 4, &buf, |p| {
            let k = 255 - p[3] as u32;
            let c = |v: u8| ((255 - v as u32) * k / 255) as u8;
            [c(p[0]), c(p[1]), c(p[2]), 255]
        }),
    }?;
    Ok(Img { w: ow, h: oh, rgba, orig: (info.width as u32, info.height as u32), bytes: bytes.len() })
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
    let (ow, oh, rgba) = shrink(w, h, channels, info.line_size, &buf, |p| match channels {
        1 => [p[0], p[0], p[0], 255],
        2 => [p[0], p[0], p[0], p[1]],
        3 => [p[0], p[1], p[2], 255],
        _ => [p[0], p[1], p[2], p[3]],
    })?;
    Ok(Img { w: ow, h: oh, rgba, orig: (info.width, info.height), bytes: bytes.len() })
}

/// Reads `rev:path` from the repository and decodes it.
pub fn load(git: &Git, rev: &str, path: &str) -> Result<Img, String> {
    let spec = format!("{rev}:{path}");
    let b = git.output_limited(&["cat-file", "blob", &spec], MAX_FILE).map_err(|e| e.to_string())?;
    if b.len() >= MAX_FILE {
        return Err("the image file is larger than 32 MB".into());
    }
    decode(&b)
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

    fn jpeg_bytes(w: u16, h: u16, rgb: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 100).encode(rgb, w, h, jpeg_encoder::ColorType::Rgb).unwrap();
        out
    }

    #[test]
    fn decodes_jpeg_and_tells_formats_apart() {
        // 16×8: left half red, right half blue (lossy, so roughly).
        let mut rgb = Vec::new();
        for _ in 0..8 {
            for x in 0..16 {
                rgb.extend_from_slice(if x < 8 { &[250, 10, 10] } else { &[10, 10, 250] });
            }
        }
        let j = jpeg_bytes(16, 8, &rgb);
        let i = decode(&j).unwrap();
        assert_eq!((i.w, i.h, i.orig, i.bytes), (16, 8, (16, 8), j.len()));
        let left = &i.rgba[(4 * 16 + 2) * 4..][..4];
        let right = &i.rgba[(4 * 16 + 13) * 4..][..4];
        assert!(left[0] > 200 && left[2] < 60 && left[3] == 255, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
        // A wide JPEG shrinks like a PNG.
        let wide = jpeg_bytes(3300, 8, &vec![128u8; 3300 * 8 * 3]);
        let i = decode(&wide).unwrap();
        assert!(i.w <= MAX_SIDE && i.orig == (3300, 8));
        // Broken data is an error, never a panic.
        assert!(decode(&j[..j.len() / 2]).is_err());
        assert!(decode(&[0xff, 0xd8, 0xff, 0x00]).is_err());
        assert!(is_image("photo.JPG") && is_image("a.jpeg") && !is_image("a.jpg.txt"));
    }
}
