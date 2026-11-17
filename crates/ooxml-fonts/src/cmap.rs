//! Owns: a face's character map as Word's layout reads it -- the best Unicode subtable in
//! fontTools' `getBestCmap` order, then a symbol face's (3, 0) subtable under it (Symbol
//! and Wingdings map their glyphs at U+F0xx there, which is what a list label's
//! `w:lvlText` names). A port of ooxml-common's `office.read_cmap`, subtable formats 0, 4,
//! 6, 12 and 13, glyph 0 left out, a later mapping of one code winning within a subtable.
//!
//! Must not: shape, or map anything but single code points.

use crate::be::{i16_at, u16_at, u32_at};
use std::collections::HashMap;

/// `getBestCmap`'s order of (platform, encoding).
const PREFERENCE: [(u16, u16); 8] = [(3, 10), (0, 6), (0, 4), (3, 1), (0, 3), (0, 2), (0, 1), (0, 0)];

/// The most code points one subtable maps: a range wider than this in a format 12 or 13
/// group is malformed (Unicode has 0x110000 code points).
const MAX_CODES: u32 = 0x11_0000;

/// Code point -> glyph id.
pub(crate) type CharMap = HashMap<u32, u32>;

/// The character map of a face's `cmap` table.
pub(crate) fn read_cmap(table: &[u8]) -> CharMap {
    let count = u16_at(table, 2).unwrap_or_default();
    let mut subtables: Vec<((u16, u16), usize)> = Vec::new();
    for i in 0..usize::from(count) {
        let at = 4 + 8 * i;
        let (Some(platform), Some(encoding), Some(offset)) =
            (u16_at(table, at), u16_at(table, at + 2), u32_at(table, at + 4))
        else {
            break;
        };
        let key = (platform, encoding);
        if !subtables.iter().any(|(k, _)| *k == key) {
            subtables.push((key, offset as usize));
        }
    }
    let find = |key: (u16, u16)| subtables.iter().find(|(k, _)| *k == key).map(|(_, offset)| *offset);
    let mut out = PREFERENCE
        .iter()
        .find_map(|key| find(*key))
        .map(|offset| subtable(table, offset))
        .unwrap_or_default();
    if let Some(offset) = find((3, 0)) {
        for (code, glyph) in subtable(table, offset) {
            out.entry(code).or_insert(glyph);
        }
    }
    out
}

fn subtable(table: &[u8], offset: usize) -> CharMap {
    let mut out = CharMap::new();
    match u16_at(table, offset) {
        Some(0) => format0(table, offset, &mut out),
        Some(4) => format4(table, offset, &mut out),
        Some(6) => format6(table, offset, &mut out),
        Some(format @ (12 | 13)) => format12(table, offset, format == 13, &mut out),
        _ => {}
    }
    out
}

fn format0(table: &[u8], offset: usize, out: &mut CharMap) {
    for code in 0..256_usize {
        match table.get(offset + 6 + code) {
            Some(&glyph) if glyph != 0 => {
                out.insert(code as u32, u32::from(glyph));
            }
            Some(_) => {}
            None => break,
        }
    }
}

fn format4(table: &[u8], offset: usize, out: &mut CharMap) {
    let segments = usize::from(u16_at(table, offset + 6).unwrap_or_default() / 2);
    let ends_at = offset + 14;
    let starts_at = offset + 16 + 2 * segments;
    let deltas_at = starts_at + 2 * segments;
    let ranges_at = starts_at + 4 * segments;
    for k in 0..segments {
        let (Some(end), Some(start), Some(delta), Some(range)) = (
            u16_at(table, ends_at + 2 * k),
            u16_at(table, starts_at + 2 * k),
            i16_at(table, deltas_at + 2 * k),
            u16_at(table, ranges_at + 2 * k),
        ) else {
            return;
        };
        if start == 0xFFFF {
            continue;
        }
        for code in start..=end {
            let glyph = if range == 0 {
                (i32::from(code) + i32::from(delta)) & 0xFFFF
            } else {
                let at = ranges_at + 2 * k + usize::from(range) + 2 * usize::from(code - start);
                match u16_at(table, at) {
                    None => continue,
                    Some(0) => 0,
                    Some(glyph) => (i32::from(glyph) + i32::from(delta)) & 0xFFFF,
                }
            };
            if glyph != 0 {
                out.insert(u32::from(code), glyph as u32);
            }
        }
    }
}

fn format6(table: &[u8], offset: usize, out: &mut CharMap) {
    let (Some(first), Some(count)) = (u16_at(table, offset + 6), u16_at(table, offset + 8)) else {
        return;
    };
    for i in 0..usize::from(count) {
        match u16_at(table, offset + 10 + 2 * i) {
            Some(0) => {}
            Some(glyph) => {
                out.insert(u32::from(first) + i as u32, u32::from(glyph));
            }
            None => return,
        }
    }
}

fn format12(table: &[u8], offset: usize, constant: bool, out: &mut CharMap) {
    let groups = u32_at(table, offset + 12).unwrap_or_default() as usize;
    for i in 0..groups {
        let at = offset + 16 + 12 * i;
        let (Some(start), Some(end), Some(glyph)) = (u32_at(table, at), u32_at(table, at + 4), u32_at(table, at + 8))
        else {
            return;
        };
        if end < start || end - start >= MAX_CODES {
            continue;
        }
        for code in start..=end {
            let value = if constant {
                glyph
            } else {
                glyph.wrapping_add(code - start)
            };
            if value != 0 {
                out.insert(code, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(subtables: &[((u16, u16), Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0, 0];
        out.extend((subtables.len() as u16).to_be_bytes());
        let mut offset = 4 + 8 * subtables.len();
        for ((platform, encoding), body) in subtables {
            out.extend(platform.to_be_bytes());
            out.extend(encoding.to_be_bytes());
            out.extend((offset as u32).to_be_bytes());
            offset += body.len();
        }
        for (_, body) in subtables {
            out.extend(body);
        }
        out
    }

    fn format6_body(first: u16, glyphs: &[u16]) -> Vec<u8> {
        let mut out = vec![0, 6, 0, 0, 0, 0];
        out.extend(first.to_be_bytes());
        out.extend((glyphs.len() as u16).to_be_bytes());
        for glyph in glyphs {
            out.extend(glyph.to_be_bytes());
        }
        out
    }

    #[test]
    fn the_best_unicode_subtable_then_the_symbol_one_under_it() {
        let bytes = table(&[
            ((3, 0), format6_body(0x41, &[7, 8, 9])),
            ((3, 1), format6_body(0x42, &[0, 5])),
        ]);
        let map = read_cmap(&bytes);
        assert_eq!(map.get(&0x41), Some(&7));
        assert_eq!(map.get(&0x42), Some(&8));
        assert_eq!(map.get(&0x43), Some(&5));
    }
}
