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
///
/// A well-formed table states each pair once, in 6 bytes of its own: more pairs than the
/// table has 6-byte runs can only be the same bytes read again (subtables whose lengths
/// lay them over each other), and the table is read no further than that many pairs.
pub(crate) fn read_legacy_kern(table: &[u8]) -> KernPairs {
    let mut out = KernPairs::new();
    if table.len() < 4 {
        return out;
    }
    let mut budget = table.len() / 6;
    if u16_at(table, 0) == Some(0) {
        let count = u16_at(table, 2).unwrap_or_default();
        let mut at = 4_usize;
        for _ in 0..count {
            let (Some(length), Some(coverage)) = (u16_at(table, at + 2), u16_at(table, at + 4)) else {
                break;
            };
            if coverage >> 8 == 0 && !pairs(table, at + 6, at + 14, &mut budget, &mut out) {
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
            if coverage & 0xFF == 0 && !pairs(table, at + 8, at + 16, &mut budget, &mut out) {
                break;
            }
            at += length as usize;
        }
    }
    out
}

/// Read a format 0 subtable's pairs (`count_at`: its `nPairs`; `first`: its first pair)
/// into `out`, each charged to `budget`; `false` where the table ends before them or the
/// budget does.
fn pairs(table: &[u8], count_at: usize, first: usize, budget: &mut usize, out: &mut KernPairs) -> bool {
    let Some(count) = u16_at(table, count_at) else {
        return false;
    };
    for i in 0..usize::from(count) {
        let Some(rest) = budget.checked_sub(1) else {
            return false;
        };
        *budget = rest;
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

    #[test]
    fn subtables_laid_over_each_other_are_read_no_further_than_the_table_holds() {
        // 65,535 subtables of length 0, each the same one of 10,000 pairs: 655 million
        // pairs, were they read. The table's bytes hold 10,003 pairs: the first
        // subtable's are read, and 3 of the second's before the reading stops.
        let pairs: Vec<(u16, u16, i16)> = (0..10_000).map(|k| (k, k, 1)).collect();
        let mut table = version0(&[(1, &pairs)]);
        table.splice(2..4, u16::MAX.to_be_bytes());
        table.splice(6..8, 0_u16.to_be_bytes());
        let started = std::time::Instant::now();
        let read = read_legacy_kern(&table);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(read.len(), 10_000);
    }
}
