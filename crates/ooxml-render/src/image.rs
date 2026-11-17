//! Owns: what the painter reads of a picture -- its bytes from base64 (strict, as Python's
//! `b64decode(validate=True)`), and its natural size in points from its pixels and stated
//! density (ooxml-common's `imagemeta.natural_size_pt`: PNG, JPEG, GIF and BMP) -- and an
//! `a:pattFill` preset's measured cell as rectangles (`pattern.cell_rectangles`).
//!
//! Must not: decode pixels, or guess a size for a format it does not read.

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

/// The density assumed where a picture states none, pixels an inch.
const DEFAULT_DENSITY_DPI: f64 = 144.0;
const INCHES_PER_METRE: f64 = 0.0254;
/// A pattern cell's side, bits.
pub const PATTERN_CELL_BITS: u32 = 8;
/// A pattern cell's side, points.
pub const PATTERN_CELL_PT: f64 = 8.0;

/// `text` decoded from padded standard base64, or `None` for anything that is not.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let value = |b: u8| -> Option<u32> {
        match b {
            b'A'..=b'Z' => Some(u32::from(b - b'A')),
            b'a'..=b'z' => Some(u32::from(b - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(b - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let chunks = bytes.len() / 4;
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let pad = chunk.iter().rev().take_while(|b| **b == b'=').count();
        if pad > 2 || (pad > 0 && index + 1 != chunks) {
            return None;
        }
        let mut n = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            let v = if i >= 4 - pad { 0 } else { value(*b)? };
            n = (n << 6) | v;
        }
        let [_, a, b, c] = n.to_be_bytes();
        out.push(a);
        if pad < 2 {
            out.push(b);
        }
        if pad < 1 {
            out.push(c);
        }
    }
    Some(out)
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn be16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn le16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn le32(data: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// A picture's size in points, or `None` for a format not read here.
pub fn natural_size_pt(data: &[u8]) -> Option<(f64, f64)> {
    let (w, h, dx, dy) = pixels_and_density(data)?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((f64::from(w) * 72.0 / dx, f64::from(h) * 72.0 / dy))
}

fn pixels_and_density(data: &[u8]) -> Option<(u32, u32, f64, f64)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(data);
    }
    if data.starts_with(b"\xff\xd8") {
        return jpeg(data);
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        let (w, h) = (le16(data, 6)?, le16(data, 8)?);
        return (w != 0 && h != 0).then_some((u32::from(w), u32::from(h), DEFAULT_DENSITY_DPI, DEFAULT_DENSITY_DPI));
    }
    if data.starts_with(b"BM") {
        return bmp(data);
    }
    None
}

fn png(data: &[u8]) -> Option<(u32, u32, f64, f64)> {
    let (mut w, mut h) = (0, 0);
    let (mut dx, mut dy) = (DEFAULT_DENSITY_DPI, DEFAULT_DENSITY_DPI);
    let mut offset = 8usize;
    while offset + 8 <= data.len() {
        let length = usize::try_from(be32(data, offset)?).ok()?;
        let tag = data.get(offset + 4..offset + 8)?;
        let end = (offset + 8).saturating_add(length).min(data.len());
        let body = data.get(offset + 8..end)?;
        if tag == b"IHDR" && body.len() >= 8 {
            w = be32(body, 0)?;
            h = be32(body, 4)?;
        } else if tag == b"pHYs" && body.len() >= 9 {
            let (px, py, unit) = (be32(body, 0)?, be32(body, 4)?, body.get(8).copied()?);
            if unit == 1 && px != 0 && py != 0 {
                dx = f64::from(px) * INCHES_PER_METRE;
                dy = f64::from(py) * INCHES_PER_METRE;
            }
        } else if tag == b"IDAT" || tag == b"IEND" {
            break;
        }
        offset = offset.checked_add(12)?.checked_add(length)?;
    }
    (w != 0 && h != 0).then_some((w, h, dx, dy))
}

const JPEG_FRAME: [u8; 13] = [
    0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
];

fn jpeg(data: &[u8]) -> Option<(u32, u32, f64, f64)> {
    let (mut dx, mut dy) = (DEFAULT_DENSITY_DPI, DEFAULT_DENSITY_DPI);
    let mut offset = 2usize;
    while offset + 4 <= data.len() {
        if data.get(offset) != Some(&0xFF) {
            offset += 1;
            continue;
        }
        let marker = *data.get(offset + 1)?;
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            offset += 2;
            continue;
        }
        let length = usize::from(be16(data, offset + 2)?);
        let end = (offset + 2 + length).min(data.len());
        let body = data.get((offset + 4).min(end)..end)?;
        if marker == 0xE0 && body.starts_with(b"JFIF\x00") && body.len() >= 12 {
            let units = body.get(7).copied()?;
            let (px, py) = (be16(body, 8)?, be16(body, 10)?);
            if px != 0 && py != 0 {
                if units == 1 {
                    dx = f64::from(px);
                    dy = f64::from(py);
                } else if units == 2 {
                    dx = f64::from(px) * 2.54;
                    dy = f64::from(py) * 2.54;
                }
            }
        } else if JPEG_FRAME.contains(&marker) && body.len() >= 5 {
            let (h, w) = (be16(body, 1)?, be16(body, 3)?);
            return (w != 0 && h != 0).then_some((u32::from(w), u32::from(h), dx, dy));
        } else if marker == 0xDA {
            break;
        }
        offset += 2 + length;
    }
    None
}

fn bmp(data: &[u8]) -> Option<(u32, u32, f64, f64)> {
    if data.len() < 30 {
        return None;
    }
    let header = le32(data, 14)?;
    if header < 40 || data.len() < 54 {
        if header == 12 && data.len() >= 26 {
            let (w, h) = (le16(data, 18)?, le16(data, 20)?);
            return Some((u32::from(w), u32::from(h), DEFAULT_DENSITY_DPI, DEFAULT_DENSITY_DPI));
        }
        return None;
    }
    let (w, h) = (le32(data, 18)?, le32(data, 22)?);
    let (px, py) = (le32(data, 38)?, le32(data, 42)?);
    let dx = if px > 0 {
        f64::from(px) * INCHES_PER_METRE
    } else {
        DEFAULT_DENSITY_DPI
    };
    let dy = if py > 0 {
        f64::from(py) * INCHES_PER_METRE
    } else {
        DEFAULT_DENSITY_DPI
    };
    let (w, h) = (w.unsigned_abs(), h.unsigned_abs());
    (w != 0 && h != 0).then_some((w, h, dx, dy))
}

fn cells() -> &'static HashMap<String, [u8; 8]> {
    static CELLS: OnceLock<HashMap<String, [u8; 8]>> = OnceLock::new();
    CELLS.get_or_init(|| {
        include_str!("../data/patterns.tsv")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(|line| {
                let (name, rows) = line.split_once('\t')?;
                let mut out = [0u8; 8];
                for (slot, row) in out.iter_mut().zip(rows.split(' ')) {
                    *slot = u8::from_str_radix(row, 16).ok()?;
                }
                Some((name.to_owned(), out))
            })
            .collect()
    })
}

/// `(x, y, width, height)` boxes, in bits, covering the preset's set bits -- runs along each
/// row merged down the rows -- in sorted order; `None` for an unknown preset.
pub fn cell_rectangles(preset: &str) -> Option<Vec<(u32, u32, u32, u32)>> {
    let rows = cells().get(preset)?;
    let mut out = Vec::new();
    // Open runs (start, length) and the row each opened on, in insertion order.
    let mut open: Vec<((u32, u32), u32)> = Vec::new();
    for (y, bits) in (0u32..).zip(rows.iter()) {
        let set = |x: u32| bits & (1u8 << (PATTERN_CELL_BITS - 1 - x)) != 0;
        let mut runs = BTreeSet::new();
        let mut x = 0;
        while x < PATTERN_CELL_BITS {
            if set(x) {
                let start = x;
                while x < PATTERN_CELL_BITS && set(x) {
                    x += 1;
                }
                runs.insert((start, x - start));
            } else {
                x += 1;
            }
        }
        open.retain(|(run, top)| {
            if runs.contains(run) {
                return true;
            }
            out.push((run.0, *top, run.1, y - top));
            false
        });
        for run in runs {
            if !open.iter().any(|(r, _)| *r == run) {
                open.push((run, y));
            }
        }
    }
    for (run, top) in open {
        out.push((run.0, top, run.1, PATTERN_CELL_BITS - top));
    }
    out.sort_unstable();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{base64_decode, cell_rectangles};

    #[test]
    fn cells_and_base64_read_as_the_python_reads_them() {
        assert_eq!(cell_rectangles("horz"), Some(vec![(0, 0, 8, 1)]));
        assert_eq!(cell_rectangles("vert"), Some(vec![(0, 0, 1, 8)]));
        assert_eq!(cell_rectangles("nope"), None);
        assert_eq!(base64_decode("aGk="), Some(b"hi".to_vec()));
        assert_eq!(base64_decode("aGk"), None);
    }
}
