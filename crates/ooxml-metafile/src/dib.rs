//! Owns: device-independent bitmaps -- the `BITMAPINFOHEADER` and its two supersets, colour
//! tables and bit-field masks, rows bottom-up or top-down, BGR order, 4-byte row padding --
//! decoded to RGBA and encoded as PNG; `BI_JPEG` and `BI_PNG` passed through; a packed DIB
//! split into header and pixels.
//!
//! The 32-bit "alpha" of a `BI_RGB` DIB is usually not alpha (Office writes zeros): it is
//! honoured only when some pixel sets it, or an explicit alpha mask says so. A truncated
//! pixel buffer is padded with zeros. Run-length encoded DIBs are not decoded.
//!
//! Must not: fail loudly -- malformed input is `None`, never a panic.

use crate::bytes::Bytes;
use crate::png;

const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_JPEG: u32 = 4;
const BI_PNG: u32 = 5;
const BI_ALPHABITFIELDS: u32 = 6;
const HEADER_INFO: u32 = 40;
const HEADER_V4: u32 = 108;
const HEADER_V5: u32 = 124;
/// The most pixels one DIB may declare.
const MAX_PIXELS: i64 = 64_000_000;

/// An encoded image: its bytes and media type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// The bytes (a PNG, or the JPEG or PNG the DIB carries).
    pub data: Vec<u8>,
    /// `image/png` or `image/jpeg`.
    pub mime: &'static str,
}

/// A DIB's pixels: tightly packed RGBA rows, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The pixels.
    pub rgba: Vec<u8>,
    /// Whether the fourth byte is real alpha.
    pub has_alpha: bool,
}

struct Info {
    width: u32,
    height: u32,
    bit_count: u16,
    compression: u32,
    palette_offset: usize,
    palette_entries: usize,
    masks: Option<[u32; 4]>,
    top_down: bool,
}

/// One DIB (its `BITMAPINFO` block and its separate pixels) as a PNG, or the JPEG or PNG
/// it carries; `None` when it cannot be read.
pub fn to_image(header: &[u8], pixels: &[u8]) -> Option<Encoded> {
    let info = parse_header(header)?;
    match info.compression {
        BI_JPEG => {
            return (!pixels.is_empty()).then(|| Encoded {
                data: pixels.to_vec(),
                mime: "image/jpeg",
            })
        }
        BI_PNG => {
            return (!pixels.is_empty()).then(|| Encoded {
                data: pixels.to_vec(),
                mime: "image/png",
            })
        }
        BI_RLE8 | BI_RLE4 => return None,
        BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS => {}
        _ => return None,
    }
    let d = decoded(&info, header, pixels)?;
    Some(Encoded {
        data: png::encode(d.width, d.height, &d.rgba, d.has_alpha),
        mime: "image/png",
    })
}

/// The pixels of an uncompressed DIB (`BI_RGB`, `BI_BITFIELDS`), or `None`.
pub fn decode(header: &[u8], pixels: &[u8]) -> Option<Decoded> {
    let info = parse_header(header)?;
    if !matches!(info.compression, BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS) {
        return None;
    }
    decoded(&info, header, pixels)
}

/// A packed DIB split into its header (with colour table or masks) and its pixels.
pub fn split_packed(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let b = Bytes(data);
    if data.len() < 40 {
        return None;
    }
    let size = b.u32(0).ok()?;
    let bit_count = b.u16(14).ok()?;
    let compression = b.u32(16).ok()?;
    if !matches!(size, HEADER_INFO | HEADER_V4 | HEADER_V5) || size as usize > data.len() {
        return None;
    }
    let clr_used = b.u32(32).ok()? as usize;
    let mut end = size as usize;
    if matches!(compression, BI_BITFIELDS | BI_ALPHABITFIELDS) && size == HEADER_INFO {
        end += if compression == BI_ALPHABITFIELDS { 16 } else { 12 };
    }
    if bit_count <= 8 {
        let entries = if clr_used != 0 { clr_used } else { 1usize << bit_count };
        end += 4 * entries.min(256);
    } else if clr_used != 0 {
        end += 4 * clr_used.min(256);
    }
    if end > data.len() {
        return None;
    }
    Some(data.split_at(end))
}

fn parse_header(header: &[u8]) -> Option<Info> {
    if header.len() < 40 {
        return None;
    }
    let b = Bytes(header);
    let size = b.u32(0).ok()?;
    let width = b.i32(4).ok()?;
    let height = b.i32(8).ok()?;
    let bit_count = b.u16(14).ok()?;
    let compression = b.u32(16).ok()?;
    let clr_used = b.u32(32).ok()? as usize;
    if !matches!(size, HEADER_INFO | HEADER_V4 | HEADER_V5) || size as usize > header.len() {
        return None;
    }
    if !matches!(bit_count, 1 | 4 | 8 | 16 | 24 | 32) || width <= 0 || height == 0 {
        return None;
    }
    let top_down = height < 0;
    let height = height.unsigned_abs();
    if i64::from(width) * i64::from(height) > MAX_PIXELS {
        return None;
    }
    let mut palette_offset = size as usize;
    let mut masks = None;
    if matches!(compression, BI_BITFIELDS | BI_ALPHABITFIELDS) {
        let count = if compression == BI_ALPHABITFIELDS { 4 } else { 3 };
        let at = if size == HEADER_INFO { palette_offset } else { 40 };
        let values = b.many(at, count, 4, Bytes::u32).ok()?;
        if size == HEADER_INFO {
            palette_offset += count * 4;
        }
        let get = |i: usize| values.get(i).copied().unwrap_or(0);
        masks = Some([get(0), get(1), get(2), if count == 4 { get(3) } else { 0 }]);
    }
    let mut palette_entries = 0;
    if bit_count <= 8 {
        let wanted = if clr_used != 0 { clr_used } else { 1usize << bit_count };
        let available = header.len().saturating_sub(palette_offset) / 4;
        palette_entries = wanted.min(available);
    }
    Some(Info {
        width: width.unsigned_abs(),
        height,
        bit_count,
        compression,
        palette_offset,
        palette_entries,
        masks,
        top_down,
    })
}

fn decoded(info: &Info, header: &[u8], pixels: &[u8]) -> Option<Decoded> {
    let (w, h) = (info.width as usize, info.height as usize);
    let stride = (w * usize::from(info.bit_count)).div_ceil(32) * 4;
    let needed = stride * h;
    let padded;
    let pixels = if pixels.len() < needed {
        padded = [pixels, &vec![0u8; needed - pixels.len()]].concat();
        &padded[..]
    } else {
        pixels
    };
    let palette: Vec<[u8; 3]> = (0..info.palette_entries)
        .filter_map(|i| {
            let base = info.palette_offset + i * 4;
            Some([*header.get(base + 2)?, *header.get(base + 1)?, *header.get(base)?])
        })
        .collect();
    if info.bit_count <= 8 && palette.is_empty() {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let source = if info.top_down { y } else { h - 1 - y };
        let row = pixels.get(source * stride..source * stride + stride).unwrap_or(&[]);
        let target = out.get_mut(y * w * 4..(y + 1) * w * 4)?;
        decode_row(info, row, &palette, target);
    }
    let has_alpha = info.bit_count == 32 && alpha_is_meaningful(info, &out);
    Some(Decoded {
        width: info.width,
        height: info.height,
        rgba: out,
        has_alpha,
    })
}

fn put(target: &mut [u8], x: usize, px: [u8; 4]) {
    if let Some(slot) = target.get_mut(x * 4..x * 4 + 4) {
        slot.copy_from_slice(&px);
    }
}

fn decode_row(info: &Info, row: &[u8], palette: &[[u8; 3]], target: &mut [u8]) {
    let width = info.width as usize;
    let at = |i: usize| row.get(i).copied();
    match info.bit_count {
        1 | 4 | 8 => {
            let bpp = usize::from(info.bit_count);
            let per_byte = 8 / bpp;
            let mask = (1u16 << bpp) - 1;
            for x in 0..width {
                let byte = at(x / per_byte).unwrap_or(0);
                let shift = 8 - bpp * (x % per_byte + 1);
                let index = usize::from((u16::from(byte) >> shift) & mask);
                let [r, g, b] = palette.get(index).copied().unwrap_or([0, 0, 0]);
                put(target, x, [r, g, b, 255]);
            }
        }
        16 => {
            let masks = info.masks.unwrap_or([0x7C00, 0x03E0, 0x001F, 0]);
            for x in 0..width {
                let (Some(lo), Some(hi)) = (at(x * 2), at(x * 2 + 1)) else {
                    break;
                };
                let v = u32::from(lo) | (u32::from(hi) << 8);
                put(
                    target,
                    x,
                    [scale(v, masks[0]), scale(v, masks[1]), scale(v, masks[2]), 255],
                );
            }
        }
        24 => {
            for x in 0..width {
                let (Some(b), Some(g), Some(r)) = (at(x * 3), at(x * 3 + 1), at(x * 3 + 2)) else {
                    break;
                };
                put(target, x, [r, g, b, 255]);
            }
        }
        _ => {
            for x in 0..width {
                let Some(&[b, g, r, a]) = row.get(x * 4..x * 4 + 4) else {
                    break;
                };
                match info.masks {
                    None => put(target, x, [r, g, b, a]),
                    Some(m) => {
                        let v = u32::from_le_bytes([b, g, r, a]);
                        let alpha = if m[3] != 0 { scale(v, m[3]) } else { 255 };
                        put(target, x, [scale(v, m[0]), scale(v, m[1]), scale(v, m[2]), alpha]);
                    }
                }
            }
        }
    }
}

/// A masked field stretched to 0..255.
fn scale(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let width = 32 - (mask >> shift).leading_zeros();
    let field = u64::from((value & mask) >> shift);
    let maximum = (1u64 << width) - 1;
    if maximum == 0 {
        return 0;
    }
    ((field * 255 + maximum / 2) / maximum) as u8
}

fn alpha_is_meaningful(info: &Info, rgba: &[u8]) -> bool {
    if info.compression == BI_ALPHABITFIELDS && info.masks.is_some_and(|m| m[3] != 0) {
        return true;
    }
    rgba.iter().skip(3).step_by(4).any(|&a| a != 0)
}
