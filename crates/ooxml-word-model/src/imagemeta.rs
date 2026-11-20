//! Owns: a picture's size in pixels and its density from its header (PNG, JPEG, GIF, BMP),
//! as ooxml-common's `imagemeta.pixels_and_density` reads them, and whether the file states
//! a density at all (docx-agent's `_states_density`): what sizes a picture bullet and an
//! inserted picture. Moves to the shared image model when DrawingML's pictures do (phase
//! P3; ooxml-render's `image.rs` reads the same headers for the painter).
//!
//! Must not: decode pixels.

/// The density ooxml-common assumes where a header states none, pixels an inch.
pub const DEFAULT_DENSITY_DPI: f64 = 144.0;
const INCHES_PER_METRE: f64 = 0.0254;

fn be16(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at + 2)?;
    Some(u32::from(u16::from_be_bytes([*b.first()?, *b.get(1)?])))
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    let b: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(b))
}

fn le16(data: &[u8], at: usize) -> Option<u32> {
    let b: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
    Some(u32::from(u16::from_le_bytes(b)))
}

fn le32i(data: &[u8], at: usize) -> Option<i32> {
    let b: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(i32::from_le_bytes(b))
}

/// Pixels and pixels an inch.
type Measured = (u32, u32, f64, f64);

fn png(data: &[u8]) -> Option<Measured> {
    let mut offset = 8usize;
    let (mut width, mut height) = (0, 0);
    let (mut dx, mut dy) = (DEFAULT_DENSITY_DPI, DEFAULT_DENSITY_DPI);
    while offset + 8 <= data.len() {
        let length = usize::try_from(be32(data, offset)?).ok()?;
        let tag = data.get(offset + 4..offset + 8)?;
        let body = data.get(offset + 8..(offset + 8).saturating_add(length).min(data.len()))?;
        if tag == b"IHDR" && body.len() >= 8 {
            width = be32(body, 0)?;
            height = be32(body, 4)?;
        } else if tag == b"pHYs" && body.len() >= 9 {
            // unit 1 is metres; unit 0 means "aspect ratio only", which states no size
            let (px, py) = (be32(body, 0)?, be32(body, 4)?);
            if body.get(8) == Some(&1) && px != 0 && py != 0 {
                dx = f64::from(px) * INCHES_PER_METRE;
                dy = f64::from(py) * INCHES_PER_METRE;
            }
        } else if tag == b"IDAT" || tag == b"IEND" {
            break;
        }
        offset = offset.checked_add(12)?.checked_add(length)?;
    }
    (width != 0 && height != 0).then_some((width, height, dx, dy))
}

const JPEG_FRAME: &[u8] = &[
    0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
];

fn jpeg(data: &[u8]) -> Option<Measured> {
    let (mut dx, mut dy) = (DEFAULT_DENSITY_DPI, DEFAULT_DENSITY_DPI);
    let mut offset = 2usize;
    while offset + 4 <= data.len() {
        if *data.get(offset)? != 0xFF {
            offset += 1;
            continue;
        }
        let marker = *data.get(offset + 1)?;
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            offset += 2;
            continue;
        }
        let length = usize::try_from(be16(data, offset + 2)?).ok()?;
        let end = (offset + 2 + length).min(data.len());
        let body = data.get((offset + 4).min(end)..end)?;
        if marker == 0xE0 && body.starts_with(b"JFIF\x00") && body.len() >= 12 {
            // units 0 is "pixel aspect ratio only": no size at all
            let (px, py) = (be16(body, 8)?, be16(body, 10)?);
            if px != 0 && py != 0 {
                match body.get(7) {
                    Some(1) => (dx, dy) = (f64::from(px), f64::from(py)),
                    Some(2) => (dx, dy) = (f64::from(px) * 2.54, f64::from(py) * 2.54),
                    _ => {}
                }
            }
        } else if JPEG_FRAME.contains(&marker) && body.len() >= 5 {
            let (height, width) = (be16(body, 1)?, be16(body, 3)?);
            return (width != 0 && height != 0).then_some((width, height, dx, dy));
        } else if marker == 0xDA {
            break;
        }
        offset += 2 + length;
    }
    None
}

fn gif(data: &[u8]) -> Option<Measured> {
    // GIF states no physical size: the default density is the whole answer
    let (width, height) = (le16(data, 6)?, le16(data, 8)?);
    (width != 0 && height != 0).then_some((width, height, DEFAULT_DENSITY_DPI, DEFAULT_DENSITY_DPI))
}

fn bmp(data: &[u8]) -> Option<Measured> {
    if data.len() < 30 {
        return None;
    }
    let header = le32i(data, 14)?;
    if header < 40 || data.len() < 54 {
        // a BITMAPCOREHEADER: 16-bit extents and no density
        if header == 12 && data.len() >= 26 {
            return Some((
                le16(data, 18)?,
                le16(data, 20)?,
                DEFAULT_DENSITY_DPI,
                DEFAULT_DENSITY_DPI,
            ));
        }
        return None;
    }
    let (width, height) = (le32i(data, 18)?.unsigned_abs(), le32i(data, 22)?.unsigned_abs());
    let density = |per: i32| {
        if per > 0 {
            f64::from(per) * INCHES_PER_METRE
        } else {
            DEFAULT_DENSITY_DPI
        }
    };
    let (dx, dy) = (density(le32i(data, 38)?), density(le32i(data, 42)?));
    (width != 0 && height != 0).then_some((width, height, dx, dy))
}

/// A picture's width and height in pixels and its density in pixels an inch (the stated
/// one, else [`DEFAULT_DENSITY_DPI`]), or `None` for a format not read here.
pub fn pixels_and_density(data: &[u8]) -> Option<(u32, u32, f64, f64)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(data)
    } else if data.starts_with(b"\xff\xd8") {
        jpeg(data)
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        gif(data)
    } else if data.starts_with(b"BM") {
        bmp(data)
    } else {
        None
    }
}

/// A picture's width and height in pixels, or `None` for a format not read here.
pub fn pixels(data: &[u8]) -> Option<(u32, u32)> {
    pixels_and_density(data).map(|(w, h, _, _)| (w, h))
}

/// Whether the file states a density (docx-agent's `_states_density`): a PNG's `pHYs` in
/// metres (before its first `IDAT`), a JFIF header with units, any BMP.
pub fn states_density(data: &[u8]) -> bool {
    if data.starts_with(b"\x89PNG") {
        let mut position = 8usize;
        while position + 8 <= data.len() {
            let Some(length) = be32(data, position).and_then(|l| usize::try_from(l).ok()) else {
                return false;
            };
            match data.get(position + 4..position + 8) {
                Some(b"pHYs") => return position + 17 <= data.len() && data.get(position + 16) == Some(&1),
                Some(b"IDAT") => return false,
                _ => {}
            }
            let Some(next) = position.checked_add(12).and_then(|p| p.checked_add(length)) else {
                return false;
            };
            position = next;
        }
        return false;
    }
    if data.starts_with(b"\xff\xd8") {
        return data.len() > 13 && data.get(6..11) == Some(b"JFIF\x00") && matches!(data.get(13), Some(1 | 2));
    }
    data.starts_with(b"BM")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_with(phys: Option<(u32, u8)>) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8], body: &[u8]| {
            out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(0).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out.extend_from_slice(&[0, 0, 0, 0]);
        };
        let mut header = Vec::new();
        header.extend_from_slice(&40u32.to_be_bytes());
        header.extend_from_slice(&20u32.to_be_bytes());
        header.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(b"IHDR", &header);
        if let Some((per, unit)) = phys {
            let mut body = Vec::new();
            body.extend_from_slice(&per.to_be_bytes());
            body.extend_from_slice(&per.to_be_bytes());
            body.push(unit);
            chunk(b"pHYs", &body);
        }
        chunk(b"IDAT", &[]);
        out
    }

    #[test]
    fn png_density_stated_or_not() {
        let plain = png_with(None);
        assert_eq!(pixels(&plain), Some((40, 20)));
        assert!(!states_density(&plain));
        let stated = png_with(Some((11811, 1)));
        assert!(states_density(&stated));
        let (_, _, dx, _) = pixels_and_density(&stated).unwrap_or_default();
        assert!((dx - 299.9994).abs() < 1e-3);
        assert!(!states_density(&png_with(Some((11811, 0)))));
    }
}
