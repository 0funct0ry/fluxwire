//! PNG helpers for screenshots (M9.6): dimensions, blank detection, masking and cropping.

use std::io::Cursor;

/// Standard base64 of `bytes`, for image content blocks.
pub fn to_base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Inverse of [`to_base64`].
pub fn from_base64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("bad base64: {e}"))
}

/// A decoded 8-bit RGBA image.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

/// A pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PxRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub fn decode(bytes: &[u8]) -> Result<Rgba, String> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| format!("not a PNG: {e}"))?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("image too large")?];
    let info = r
        .next_frame(&mut buf)
        .map_err(|e| format!("bad PNG: {e}"))?;
    buf.truncate(info.buffer_size());
    let data = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
    };
    Ok(Rgba {
        w: info.width,
        h: info.height,
        data,
    })
}

pub fn encode(img: &Rgba) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    let mut enc = png::Encoder::new(&mut out, img.w, img.h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&img.data))
        .map_err(|e| format!("could not encode PNG: {e}"))?;
    Ok(out)
}

/// Width and height from the IHDR chunk without decoding.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return Err("not a PNG".into());
    }
    let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Ok((be(16), be(20)))
}

/// Fraction of pixels that differ from the most common colour's first sample, sampled on a
/// grid. A capture that failed for lack of permission is one flat colour or a wallpaper-only
/// image; a real window has text and borders.
pub fn is_blank(img: &Rgba) -> bool {
    if img.w == 0 || img.h == 0 {
        return true;
    }
    let first = &img.data[..4];
    let step_x = (img.w / 64).max(1);
    let step_y = (img.h / 64).max(1);
    let mut other = 0usize;
    let mut total = 0usize;
    let mut y = 0;
    while y < img.h {
        let mut x = 0;
        while x < img.w {
            let i = ((y * img.w + x) * 4) as usize;
            total += 1;
            if img.data[i..i + 4] != *first {
                other += 1;
            }
            x += step_x;
        }
        y += step_y;
    }
    // Under 0.5% differing samples is a flat image.
    other * 200 < total
}

/// Fills `r` (clipped to the image) with opaque mid grey so masked captures compare equal.
pub fn fill(img: &mut Rgba, r: PxRect) {
    let x1 = (r.x + r.w).min(img.w);
    let y1 = (r.y + r.h).min(img.h);
    for y in r.y.min(img.h)..y1 {
        for x in r.x.min(img.w)..x1 {
            let i = ((y * img.w + x) * 4) as usize;
            img.data[i..i + 4].copy_from_slice(&[128, 128, 128, 255]);
        }
    }
}

/// Converts a rectangle in points relative to the window to pixels, clipped to `w`x`h` pixels.
pub fn to_px(x: f32, y: f32, pw: f32, ph: f32, scale: f32, w: u32, h: u32) -> Option<PxRect> {
    let x0 = (x * scale).floor().max(0.0) as u32;
    let y0 = (y * scale).floor().max(0.0) as u32;
    let x1 = (((x + pw) * scale).ceil().max(0.0) as u32).min(w);
    let y1 = (((y + ph) * scale).ceil().max(0.0) as u32).min(h);
    (x1 > x0 && y1 > y0).then(|| PxRect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Rgba {
        Rgba {
            w,
            h,
            data: c.repeat((w * h) as usize),
        }
    }

    #[test]
    fn roundtrips_and_reads_dimensions() {
        let img = solid(7, 5, [1, 2, 3, 255]);
        let bytes = encode(&img).unwrap();
        assert_eq!(dimensions(&bytes).unwrap(), (7, 5));
        assert_eq!(decode(&bytes).unwrap(), img);
        assert!(dimensions(b"nope").is_err());
        assert!(decode(b"nope").is_err());
    }

    #[test]
    fn base64_roundtrips() {
        let b = encode(&solid(2, 2, [1, 2, 3, 255])).unwrap();
        assert_eq!(from_base64(&to_base64(&b)).unwrap(), b);
        assert!(from_base64("***").is_err());
    }

    #[test]
    fn flat_images_are_blank_and_busy_ones_are_not() {
        let mut img = solid(200, 100, [10, 10, 10, 255]);
        assert!(is_blank(&img));
        fill(
            &mut img,
            PxRect {
                x: 20,
                y: 20,
                w: 100,
                h: 50,
            },
        );
        assert!(!is_blank(&img));
    }

    #[test]
    fn fill_clips_to_the_image() {
        let mut img = solid(4, 4, [0, 0, 0, 255]);
        fill(
            &mut img,
            PxRect {
                x: 2,
                y: 2,
                w: 100,
                h: 100,
            },
        );
        assert_eq!(&img.data[(3 * 4 + 3) * 4..][..4], &[128, 128, 128, 255]);
        assert_eq!(&img.data[..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn points_convert_to_clipped_pixels() {
        let r = to_px(10.0, 5.0, 20.0, 10.0, 2.0, 100, 100).unwrap();
        assert_eq!(
            r,
            PxRect {
                x: 20,
                y: 10,
                w: 40,
                h: 20
            }
        );
        let r = to_px(45.0, 0.0, 20.0, 10.0, 2.0, 100, 100).unwrap();
        assert_eq!(r.x + r.w, 100);
        assert!(to_px(60.0, 0.0, 5.0, 5.0, 2.0, 100, 100).is_none());
    }

    #[test]
    fn masked_images_compare_equal() {
        let mut a = solid(50, 50, [9, 9, 9, 255]);
        let mut b = a.clone();
        a.data[(10 * 50 + 10) * 4] = 200;
        b.data[(10 * 50 + 10) * 4] = 100;
        assert_ne!(a, b);
        let r = PxRect {
            x: 8,
            y: 8,
            w: 6,
            h: 6,
        };
        fill(&mut a, r);
        fill(&mut b, r);
        assert_eq!(a, b);
    }
}
