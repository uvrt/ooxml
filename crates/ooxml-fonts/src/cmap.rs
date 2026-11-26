//! Owns: a face's character map as Word's layout reads it -- the best Unicode subtable in
//! fontTools' `getBestCmap` order, then a symbol face's (3, 0) subtable under it (Symbol
//! and Wingdings map their glyphs at U+F0xx there, which is what a list label's
//! `w:lvlText` names). A port of ooxml-common's `office.read_cmap`, subtable formats 0, 4,
//! 6, 12 and 13, glyph 0 left out, a later mapping of one code winning within a subtable.
//! A subtable whose ranges claim more codes than its codespace holds is refused (ranges
//! laid over each other: an untrusted font's way to cost billions of insertions).
//!
//! Must not: shape, or map anything but single code points.

use crate::be::{i16_at, u16_at, u32_at};
use std::collections::HashMap;

/// `getBestCmap`'s order of (platform, encoding).
const PREFERENCE: [(u16, u16); 8] = [(3, 10), (0, 6), (0, 4), (3, 1), (0, 3), (0, 2), (0, 1), (0, 0)];

/// The most code points one subtable maps: a range wider than this in a format 12 or 13
/// group is malformed (Unicode has 0x110000 code points), and so is a format 12 or 13
/// subtable whose groups claim more codes than this in all.
const MAX_CODES: u32 = 0x11_0000;

/// The most codes a format 4 subtable maps (its codes are 16-bit): one whose segments
/// claim more in all is malformed.
const MAX_CODES_16: u32 = 0x1_0000;

/// Code point -> glyph id.
pub(crate) type CharMap = HashMap<u32, u32>;

/// The character map of a face's `cmap` table.
pub(crate) fn read_cmap(table: &[u8]) -> CharMap {
    let count = u16_at(table, 2).unwrap_or_default();
    // The first record of each (platform, encoding) wins.
    let mut subtables: HashMap<(u16, u16), usize> = HashMap::new();
    for i in 0..usize::from(count) {
        let at = 4 + 8 * i;
        let (Some(platform), Some(encoding), Some(offset)) =
            (u16_at(table, at), u16_at(table, at + 2), u32_at(table, at + 4))
        else {
            break;
        };
        subtables.entry((platform, encoding)).or_insert(offset as usize);
    }
    let find = |key: (u16, u16)| subtables.get(&key).copied();
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
    // The format states its segments sorted and apart, so they claim at most the codespace;
    // more can only be segments laid over each other again and again, each claim a map
    // insertion, and such a subtable is refused before any is made.
    let segment = |k: usize| {
        Some((
            u16_at(table, ends_at + 2 * k)?,
            u16_at(table, starts_at + 2 * k)?,
            i16_at(table, deltas_at + 2 * k)?,
            u16_at(table, ranges_at + 2 * k)?,
        ))
    };
    let claimed: u64 = (0..segments)
        .map_while(segment)
        .filter(|&(end, start, _, _)| start != 0xFFFF && end >= start)
        .map(|(end, start, _, _)| u64::from(end - start) + 1)
        .sum();
    if claimed > u64::from(MAX_CODES_16) {
        return;
    }
    for k in 0..segments {
        let Some((end, start, delta, range)) = segment(k) else {
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
    let group = |i: usize| {
        let at = offset + 16 + 12 * i;
        Some((u32_at(table, at)?, u32_at(table, at + 4)?, u32_at(table, at + 8)?))
    };
    let wide = |start: u32, end: u32| end < start || end - start >= MAX_CODES;
    // As format 4's segments: groups claiming more than the codespace in all are refused
    // before any insertion (the loop below reads as far as this one).
    let claimed: u64 = (0..groups)
        .map_while(group)
        .filter(|&(start, end, _)| !wide(start, end))
        .map(|(start, end, _)| u64::from(end - start) + 1)
        .sum();
    if claimed > u64::from(MAX_CODES) {
        return;
    }
    for i in 0..groups {
        let Some((start, end, glyph)) = group(i) else {
            return;
        };
        if wide(start, end) {
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

    fn format12_body(groups: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut out = vec![0, 12, 0, 0];
        out.extend((16 + 12 * groups.len() as u32).to_be_bytes());
        out.extend(0_u32.to_be_bytes());
        out.extend((groups.len() as u32).to_be_bytes());
        for (start, end, glyph) in groups {
            out.extend(start.to_be_bytes());
            out.extend(end.to_be_bytes());
            out.extend(glyph.to_be_bytes());
        }
        out
    }

    fn format4_body(segments: &[(u16, u16)]) -> Vec<u8> {
        let mut out = vec![0, 4, 0, 0, 0, 0];
        out.extend((2 * segments.len() as u16).to_be_bytes());
        out.extend([0; 6]);
        for (_, end) in segments {
            out.extend(end.to_be_bytes());
        }
        out.extend([0, 0]);
        for (start, _) in segments {
            out.extend(start.to_be_bytes());
        }
        out.extend(std::iter::repeat_n([0, 1], segments.len()).flatten()); // delta 1
        out.extend(std::iter::repeat_n([0, 0], segments.len()).flatten()); // no range offsets
        out
    }

    #[test]
    fn groups_claiming_more_than_the_codespace_in_all_are_refused() {
        let every = (0, 0x10_FFFF, 1);
        let started = std::time::Instant::now();
        assert!(read_cmap(&table(&[((3, 1), format12_body(&[every; 300]))])).is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        // A group wider than the codespace is skipped, the rest read.
        let map = read_cmap(&table(&[(
            (3, 10),
            format12_body(&[(0, 0x11_0000, 9), (0x41, 0x42, 5)]),
        )]));
        assert_eq!(
            (map.get(&0x41), map.get(&0x42), map.get(&0x43)),
            (Some(&5), Some(&6), None)
        );
    }

    #[test]
    fn segments_claiming_more_than_sixteen_bits_in_all_are_refused() {
        let started = std::time::Instant::now();
        assert!(read_cmap(&table(&[((3, 1), format4_body(&[(0, 0xFFFE); 2000]))])).is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        let map = read_cmap(&table(&[((3, 1), format4_body(&[(0x41, 0x42), (0xFFFF, 0xFFFF)]))]));
        assert_eq!((map.get(&0x41), map.get(&0x42)), (Some(&0x42), Some(&0x43)));
    }

    #[test]
    fn many_records_are_read_in_one_pass() {
        // Records of every (platform, encoding): each was checked against all before it.
        let mut bytes = vec![0, 0, 0xFF, 0xFF];
        for k in 0..0xFFFF_u32 {
            bytes.extend(((k >> 8) as u16 + 4).to_be_bytes());
            bytes.extend(((k & 0xFF) as u16).to_be_bytes());
            bytes.extend(0_u32.to_be_bytes());
        }
        let started = std::time::Instant::now();
        assert!(read_cmap(&bytes).is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
