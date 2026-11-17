//! Owns: the PNG the drawings embed -- one `IHDR`, one `IDAT` (zlib at level 9, filter type 0
//! on every row), one `IEND` -- and reading back an 8-bit RGB or RGBA PNG's pixels.
//!
//! The deflate stream is zlib-rs's, which is not always the C zlib's byte for byte: the
//! pixels are the same, the compressed bytes may differ (the differential compares embedded
//! PNGs by their pixels, as ooxml-common's goldens do).
//!
//! Must not: decode more than its caller's cap allows.

use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read, Write};

/// RGBA pixels (top row first) as a PNG: truecolour with alpha, or without when
/// `has_alpha` is false (the fourth byte dropped).
pub fn encode(width: u32, height: u32, rgba: &[u8], has_alpha: bool) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut raw = Vec::with_capacity(h * (1 + w * if has_alpha { 4 } else { 3 }));
    for y in 0..h {
        raw.push(0);
        let row = rgba.get(y * w * 4..(y + 1) * w * 4).unwrap_or(&[]);
        if has_alpha {
            raw.extend_from_slice(row);
        } else {
            for [r, g, b, _] in row.as_chunks::<4>().0 {
                raw.extend_from_slice(&[*r, *g, *b]);
            }
        }
    }
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, if has_alpha { 6 } else { 2 }, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &deflate(&raw));
    chunk(&mut out, b"IEND", b"");
    out
}

fn deflate(raw: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(9));
    // Writing to a vector cannot fail.
    if encoder.write_all(raw).is_err() {
        return Vec::new();
    }
    encoder.finish().unwrap_or_default()
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(payload);
    let mut crc = flate2::Crc::new();
    crc.update(tag);
    crc.update(payload);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

/// An 8-bit RGB or RGBA, non-interlaced PNG's pixels as RGBA rows; `None` for any other PNG,
/// one of more than `max_pixels`, or one that cannot be read.
pub fn decode_rgba(data: &[u8], max_pixels: u64) -> Option<(u32, u32, Vec<u8>)> {
    if data.get(12..16)? != b"IHDR" {
        return None;
    }
    let ihdr = data.get(16..29)?;
    let width = u32::from_be_bytes(ihdr.get(0..4)?.try_into().ok()?);
    let height = u32::from_be_bytes(ihdr.get(4..8)?.try_into().ok()?);
    let (depth, colour, interlace) = (*ihdr.get(8)?, *ihdr.get(9)?, *ihdr.get(12)?);
    if depth != 8
        || !(colour == 2 || colour == 6)
        || interlace != 0
        || u64::from(width) * u64::from(height) > max_pixels
    {
        return None;
    }
    let channels = if colour == 6 { 4 } else { 3 };
    let compressed = idat(data);
    let stride = width as usize * channels;
    let want = (stride + 1) * height as usize;
    let mut raw = Vec::with_capacity(want);
    let mut decoder = flate2::read::ZlibDecoder::new(&compressed[..]).take(want as u64);
    // A stream cut short leaves fewer bytes; a corrupt one stops where it fails.
    let _ = decoder.read_to_end(&mut raw);
    if raw.len() < want {
        return None;
    }
    Some((
        width,
        height,
        unfilter(&raw, stride, channels, width as usize, height as usize),
    ))
}

fn idat(data: &[u8]) -> Vec<u8> {
    let mut at = 8usize;
    let mut out = Vec::new();
    while at + 8 <= data.len() {
        let Some(len) = data
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
        else {
            break;
        };
        let tag = data.get(at + 4..at + 8).unwrap_or(&[]);
        if tag == b"IDAT" {
            let end = (at + 8).saturating_add(len as usize);
            out.extend_from_slice(data.get(at + 8..end.min(data.len())).unwrap_or(&[]));
        }
        if tag == b"IEND" {
            break;
        }
        at = at.saturating_add(12).saturating_add(len as usize);
    }
    out
}

fn unfilter(raw: &[u8], stride: usize, channels: usize, width: usize, height: usize) -> Vec<u8> {
    let mut previous = vec![0u8; stride];
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let kind = raw.get(y * (stride + 1)).copied().unwrap_or(0);
        let mut line = raw
            .get(y * (stride + 1) + 1..(y + 1) * (stride + 1))
            .unwrap_or(&[])
            .to_vec();
        for i in 0..line.len() {
            let a = if i >= channels {
                line.get(i - channels).copied().unwrap_or(0)
            } else {
                0
            };
            let b = previous.get(i).copied().unwrap_or(0);
            let c = if i >= channels {
                previous.get(i - channels).copied().unwrap_or(0)
            } else {
                0
            };
            let add = match kind {
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => paeth(a, b, c),
                _ => 0,
            };
            if let Some(v) = line.get_mut(i) {
                *v = v.wrapping_add(add);
            }
        }
        if channels == 4 {
            rgba.extend_from_slice(&line);
        } else {
            for [r, g, b] in line.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[*r, *g, *b, 0xFF]);
            }
        }
        previous = line;
    }
    rgba
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i16::from(a), i16::from(b), i16::from(c));
    let (pa, pb, pc) = ((ib - ic).abs(), (ia - ic).abs(), (ia + ib - 2 * ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}
