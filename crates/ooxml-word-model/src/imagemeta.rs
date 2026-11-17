//! Owns: a picture's size in pixels from its header (PNG, JPEG, GIF, BMP), as ooxml-common's
//! `imagemeta.pixels_and_density` reads it: what sizes a picture bullet. Moves to the shared
//! image model when DrawingML's pictures do (phase P3).
//!
//! Must not: decode pixels.

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

fn png(data: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 8usize;
    let (mut width, mut height) = (0, 0);
    while offset + 8 <= data.len() {
        let length = usize::try_from(be32(data, offset)?).ok()?;
        let tag = data.get(offset + 4..offset + 8)?;
        let body = data.get(offset + 8..(offset + 8).saturating_add(length).min(data.len()))?;
        if tag == b"IHDR" && body.len() >= 8 {
            width = be32(body, 0)?;
            height = be32(body, 4)?;
        } else if tag == b"IDAT" || tag == b"IEND" {
            break;
        }
        offset = offset.checked_add(12)?.checked_add(length)?;
    }
    (width != 0 && height != 0).then_some((width, height))
}

const JPEG_FRAME: &[u8] = &[
    0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
];

fn jpeg(data: &[u8]) -> Option<(u32, u32)> {
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
        if JPEG_FRAME.contains(&marker) && body.len() >= 5 {
            let (height, width) = (be16(body, 1)?, be16(body, 3)?);
            return (width != 0 && height != 0).then_some((width, height));
        }
        if marker == 0xDA {
            break;
        }
        offset += 2 + length;
    }
    None
}

fn gif(data: &[u8]) -> Option<(u32, u32)> {
    let (width, height) = (le16(data, 6)?, le16(data, 8)?);
    (width != 0 && height != 0).then_some((width, height))
}

fn bmp(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 30 {
        return None;
    }
    let header = le32i(data, 14)?;
    if header < 40 || data.len() < 54 {
        if header == 12 && data.len() >= 26 {
            return Some((le16(data, 18)?, le16(data, 20)?));
        }
        return None;
    }
    let (width, height) = (le32i(data, 18)?.unsigned_abs(), le32i(data, 22)?.unsigned_abs());
    (width != 0 && height != 0).then_some((width, height))
}

/// A picture's width and height in pixels, or `None` for a format not read here.
pub fn pixels(data: &[u8]) -> Option<(u32, u32)> {
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
