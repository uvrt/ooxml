//! Owns: the legacy `kern` table's pairs, the kerning Word charges a run with `w:kern`
//! (and PowerPoint a static face): the horizontal format 0 subtables of the OpenType
//! (version 0) and Apple (version 1.0) headers, glyph id to glyph id, the first
//! subtable's value winning. A port of ooxml-common's `office.read_legacy_kern`, whose
//! reading of each subtable's declared length (not read-fonts' "the last runs to the
//! table's end") and of the coverage's format byte alone it keeps.
//!
//! Must not: read `GPOS` (DrawingML's kerning, not Word's).

use crate::be::{i16_at, u16_at, u32_at};
use std::collections::HashMap;

/// (left glyph, right glyph) -> adjustment in font units.
pub(crate) type KernPairs = HashMap<(u16, u16), i16>;

/// The pairs of a `kern` table. A subtable whose length runs past the table keeps what
/// was read before it.
pub(crate) fn read_legacy_kern(table: &[u8]) -> KernPairs {
    let mut out = KernPairs::new();
    if table.len() < 4 {
        return out;
    }
    if u16_at(table, 0) == Some(0) {
        let count = u16_at(table, 2).unwrap_or_default();
        let mut at = 4_usize;
        for _ in 0..count {
            let (Some(length), Some(coverage)) = (u16_at(table, at + 2), u16_at(table, at + 4)) else {
                break;
            };
            if coverage >> 8 == 0 && !pairs(table, at + 6, at + 14, &mut out) {
                break;
            }
            at += usize::from(length);
        }
    } else {
        let count = u32_at(table, 4).unwrap_or_default();
        let mut at = 8_usize;
        for _ in 0..count {
            let (Some(length), Some(coverage)) = (u32_at(table, at), u16_at(table, at + 4)) else {
                break;
            };
            if coverage & 0xFF == 0 && !pairs(table, at + 8, at + 16, &mut out) {
                break;
            }
            at += length as usize;
        }
    }
    out
}

/// Read a format 0 subtable's pairs (`count_at`: its `nPairs`; `first`: its first pair)
/// into `out`; `false` where the table ends before them.
fn pairs(table: &[u8], count_at: usize, first: usize, out: &mut KernPairs) -> bool {
    let Some(count) = u16_at(table, count_at) else {
        return false;
    };
    for i in 0..usize::from(count) {
        let at = first + 6 * i;
        let (Some(left), Some(right), Some(value)) = (u16_at(table, at), u16_at(table, at + 2), i16_at(table, at + 4))
        else {
            return false;
        };
        out.entry((left, right)).or_insert(value);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    type Pairs<'a> = &'a [(u16, u16, i16)];

    fn version0(subtables: &[(u16, Pairs<'_>)]) -> Vec<u8> {
        let mut out = vec![0, 0];
        out.extend((subtables.len() as u16).to_be_bytes());
        for (coverage, pairs) in subtables {
            let length = 14 + 6 * pairs.len() as u16;
            out.extend([0, 0]);
            out.extend(length.to_be_bytes());
            out.extend(coverage.to_be_bytes());
            out.extend((pairs.len() as u16).to_be_bytes());
            out.extend([0; 6]);
            for (l, r, v) in *pairs {
                out.extend(l.to_be_bytes());
                out.extend(r.to_be_bytes());
                out.extend(v.to_be_bytes());
            }
        }
        out
    }

    #[test]
    fn the_first_subtable_wins_and_other_formats_are_skipped() {
        let table = version0(&[
            (1, &[(3, 4, -50)]),
            (0x0201, &[(3, 5, 9)]),
            (1, &[(3, 4, 20), (6, 7, 1)]),
        ]);
        let pairs = read_legacy_kern(&table);
        assert_eq!(pairs.get(&(3, 4)), Some(&-50));
        assert_eq!(pairs.get(&(3, 5)), None);
        assert_eq!(pairs.get(&(6, 7)), Some(&1));
    }
}
