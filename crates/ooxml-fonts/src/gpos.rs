//! Owns: `GPOS`'s `kern` feature, pair adjustments only -- the first glyph's advance change
//! (`PairPos` formats 1 and 2, through `Extension` lookups; each lookup's first subtable
//! that covers the pair) -- which Word applies to a watermark's letters. A port of
//! docx2svg's `outlines.PairKerning`.
//!
//! Must not: shape text, or apply any other lookup or feature (Word's body text is kerned
//! by the legacy `kern` table: [`crate::kern`]).

use crate::be::{i16_at, u16_at, u32_at};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// A coverage table as read (`_coverage`): a glyph's coverage index, where a glyph stated
/// twice takes its last statement. A range is kept as a range, never spread out glyph by
/// glyph: a malformed table's ranges may claim every glyph many times over.
enum Coverage {
    /// Format 1: the glyphs listed.
    Glyphs(HashMap<u16, usize>),
    /// Format 2: `(start, end, the start's index)`, in order.
    Ranges(Vec<(u16, u16, usize)>),
}

impl Coverage {
    fn get(&self, glyph: u16) -> Option<usize> {
        match self {
            Coverage::Glyphs(map) => map.get(&glyph).copied(),
            Coverage::Ranges(ranges) => ranges
                .iter()
                .rev()
                .find(|(start, end, _)| *start <= glyph && glyph <= *end)
                .map(|(start, _, index)| index + usize::from(glyph - start)),
        }
    }
}

/// A class definition as read (`_class_def`): a glyph's class, 0 where none is stated, a
/// glyph stated twice taking its last non-zero class. Ranges are kept as ranges.
enum ClassDef {
    /// Format 1: the glyphs from the first, with a class.
    Glyphs(HashMap<u16, u16>),
    /// Format 2: `(start, end, class)` of the ranges with a class, in order.
    Ranges(Vec<(u16, u16, u16)>),
}

impl ClassDef {
    fn get(&self, glyph: u16) -> u16 {
        match self {
            ClassDef::Glyphs(map) => map.get(&glyph).copied().unwrap_or(0),
            ClassDef::Ranges(ranges) => ranges
                .iter()
                .rev()
                .find(|(start, end, _)| *start <= glyph && glyph <= *end)
                .map_or(0, |(_, _, class)| *class),
        }
    }
}

/// A format 2 subtable's classes: the first glyph's, the second's, and their counts.
type Classes = (ClassDef, ClassDef, u16, u16);

/// One `PairPos` subtable read.
struct PairPos {
    format: u16,
    coverage: Coverage,
    first_value: u16,
    second_value: u16,
    classes: Option<Classes>,
}

/// A `GPOS` table's `kern` pair adjustments.
pub(crate) struct PairKerning {
    table: Vec<u8>,
    /// Per lookup, its pair subtables (a subtable two lookups share, read once).
    lookups: Vec<Vec<(usize, Arc<PairPos>)>>,
}

fn coverage(table: &[u8], at: usize) -> Option<Coverage> {
    let format = u16_at(table, at)?;
    let count = usize::from(u16_at(table, at + 2)?);
    if format == 1 {
        let mut out = HashMap::new();
        for i in 0..count {
            out.insert(u16_at(table, at + 4 + 2 * i)?, i);
        }
        return Some(Coverage::Glyphs(out));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = u16_at(table, at + 4 + 6 * i)?;
        let end = u16_at(table, at + 6 + 6 * i)?;
        let index = usize::from(u16_at(table, at + 8 + 6 * i)?);
        out.push((start, end, index));
    }
    Some(Coverage::Ranges(out))
}

fn class_def(table: &[u8], at: usize) -> Option<ClassDef> {
    let format = u16_at(table, at)?;
    if format == 1 {
        let mut out = HashMap::new();
        let first = u16_at(table, at + 2)?;
        let count = u16_at(table, at + 4)?;
        for i in 0..count {
            let class = u16_at(table, at + 6 + 2 * usize::from(i))?;
            if class != 0 {
                out.insert(first.wrapping_add(i), class);
            }
        }
        return Some(ClassDef::Glyphs(out));
    }
    let count = usize::from(u16_at(table, at + 2)?);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = u16_at(table, at + 4 + 6 * i)?;
        let end = u16_at(table, at + 6 + 6 * i)?;
        let class = u16_at(table, at + 8 + 6 * i)?;
        if class != 0 {
            out.push((start, end, class));
        }
    }
    Some(ClassDef::Ranges(out))
}

/// A ValueRecord's size in bytes.
fn value_size(format: u16) -> usize {
    2 * (format & 0xFF).count_ones() as usize
}

/// A ValueRecord's XAdvance (0 where it states none).
fn x_advance(table: &[u8], at: usize, format: u16) -> i64 {
    if format & 4 == 0 {
        return 0;
    }
    i16_at(table, at + 2 * (format & 3).count_ones() as usize).map_or(0, i64::from)
}

fn pair_pos(table: &[u8], at: usize) -> Option<PairPos> {
    let format = u16_at(table, at)?;
    let covered = coverage(table, at + usize::from(u16_at(table, at + 2)?))?;
    let (first_value, second_value) = (u16_at(table, at + 4)?, u16_at(table, at + 6)?);
    let classes = if format == 1 {
        None
    } else {
        let first = class_def(table, at + usize::from(u16_at(table, at + 8)?))?;
        let second = class_def(table, at + usize::from(u16_at(table, at + 10)?))?;
        Some((first, second, u16_at(table, at + 12)?, u16_at(table, at + 14)?))
    };
    Some(PairPos {
        format,
        coverage: covered,
        first_value,
        second_value,
        classes,
    })
}

impl PairKerning {
    /// The `kern` feature's pair lookups of `table`; `None` where it has none or cannot be
    /// read.
    pub fn new(table: &[u8]) -> Option<PairKerning> {
        // A well-formed table states each reference once, in bytes of its own: more
        // references than the table has pairs of bytes can only be the same bytes read
        // again and again (records pointing at one list), and such a table is refused.
        let budget = table.len() / 2;
        let features = usize::from(u16_at(table, 6)?);
        let lookups_at = usize::from(u16_at(table, 8)?);
        let count = usize::from(u16_at(table, features)?);
        let mut indices: BTreeSet<u16> = BTreeSet::new();
        let mut read: BTreeSet<usize> = BTreeSet::new();
        for i in 0..count {
            let record = features + 2 + 6 * i;
            let tag = table.get(record..record + 4)?;
            let offset = usize::from(u16_at(table, record + 4)?);
            // Two records of one feature table give the same lookups: read it once.
            if tag == b"kern" && read.insert(offset) {
                let at = features + offset;
                let n = usize::from(u16_at(table, at + 2)?);
                for k in 0..n {
                    indices.insert(u16_at(table, at + 4 + 2 * k)?);
                }
            }
        }
        let lookup_count = u16_at(table, lookups_at)?;
        let mut parsed: HashMap<usize, Arc<PairPos>> = HashMap::new();
        let mut references = 0_usize;
        let mut lookups = Vec::new();
        for index in indices {
            if index >= lookup_count {
                continue;
            }
            let at = lookups_at + usize::from(u16_at(table, lookups_at + 2 + 2 * usize::from(index))?);
            let kind = u16_at(table, at)?;
            let n = usize::from(u16_at(table, at + 4)?);
            references += n;
            if references > budget {
                return None;
            }
            let mut found = Vec::new();
            for j in 0..n {
                let mut sub = at + usize::from(u16_at(table, at + 6 + 2 * j)?);
                let mut sub_kind = kind;
                if kind == 9 {
                    sub_kind = u16_at(table, sub + 2)?;
                    sub += usize::try_from(u32_at(table, sub + 4)?).ok()?;
                }
                if sub_kind == 2 {
                    let pos = match parsed.get(&sub) {
                        Some(pos) => Arc::clone(pos),
                        None => {
                            let pos = Arc::new(pair_pos(table, sub)?);
                            parsed.insert(sub, Arc::clone(&pos));
                            pos
                        }
                    };
                    found.push((sub, pos));
                }
            }
            if !found.is_empty() {
                lookups.push(found);
            }
        }
        if lookups.is_empty() {
            return None;
        }
        Some(PairKerning {
            table: table.to_vec(),
            lookups,
        })
    }

    /// The kerning between glyphs `left` and `right`, font units.
    pub fn pair(&self, left: u16, right: u16) -> i64 {
        let t = &self.table;
        let mut total = 0;
        for lookup in &self.lookups {
            for (at, sub) in lookup {
                let Some(index) = sub.coverage.get(left) else {
                    continue;
                };
                let record = value_size(sub.first_value) + value_size(sub.second_value);
                match &sub.classes {
                    None if sub.format == 1 => {
                        let Some(hit) = pair_set_hit(t, *at, index, record, right) else {
                            continue;
                        };
                        total += x_advance(t, hit, sub.first_value);
                    }
                    Some((first, second, count1, count2)) => {
                        let (c1, c2) = (first.get(left), second.get(right));
                        if c1 < *count1 && c2 < *count2 {
                            let cell = usize::from(c1) * usize::from(*count2) + usize::from(c2);
                            total += x_advance(t, at + 16 + cell * record, sub.first_value);
                        }
                    }
                    None => {}
                }
                break;
            }
        }
        total
    }
}

/// Where a format 1 subtable's pair set holds `right`'s value records (the first's).
fn pair_set_hit(t: &[u8], at: usize, index: usize, record: usize, right: u16) -> Option<usize> {
    let sets = at + usize::from(u16_at(t, at + 10 + 2 * index)?);
    let n = i64::from(u16_at(t, sets)?);
    let (mut lo, mut hi) = (0_i64, n - 1);
    while lo <= hi {
        let mid = (lo + hi).div_euclid(2);
        let offset = sets + 2 + usize::try_from(mid).ok()? * (2 + record);
        let second = u16_at(t, offset)?;
        if second == right {
            return Some(offset + 2);
        }
        if second < right {
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(out: &mut Vec<u8>, values: &[u16]) {
        for v in values {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }

    /// A `GPOS` table: a `kern` feature, one pair lookup of one format 1 subtable whose
    /// coverage is `ranges` ranges each claiming every glyph, glyph 0 kerned -50 before 5.
    fn table(ranges: u16) -> Vec<u8> {
        let mut t = Vec::new();
        put(&mut t, &[1, 0, 0, 10, 24]); // version, script list, feature list, lookup list
        put(&mut t, &[1]); // feature list at 10: one record
        t.extend_from_slice(b"kern");
        put(&mut t, &[8, 0, 1, 0]); // its offset (18 - 10), feature: no params, one lookup: 0
        put(&mut t, &[1, 4]); // lookup list at 24: one lookup, at 28
        put(&mut t, &[2, 0, 1, 8]); // lookup at 28: pair, no flags, one subtable, at 36
        put(&mut t, &[1, 18, 4, 0, 1, 12]); // pair pos at 36: format 1, coverage at 54, XAdvance
        put(&mut t, &[1, 5, (-50_i16).cast_unsigned()]); // pair set at 48: glyph 5, -50
        put(&mut t, &[2, ranges]); // coverage at 54: ranges
        for _ in 0..ranges {
            put(&mut t, &[0, u16::MAX, 0]);
        }
        t
    }

    #[test]
    fn a_coverage_of_ranges_claiming_every_glyph_again_and_again_is_read_quickly() {
        // The fuzzer's `glyph_ink` timeout: ranges spread out glyph by glyph were billions of
        // insertions; kept as ranges, the table reads in a moment and answers the same.
        let started = std::time::Instant::now();
        let kerning = PairKerning::new(&table(4000)).expect("a pair lookup");
        assert_eq!(kerning.pair(0, 5), -50);
        assert_eq!(kerning.pair(0, 6), 0);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn a_feature_read_from_more_references_than_the_table_holds_is_refused() {
        let mut t = table(1);
        // The lookup states 40,000 subtables, all the same offset: more references than
        // the table has pairs of bytes.
        let at = 28 + 4;
        t.splice(at..at + 2, 40_000_u16.to_be_bytes());
        assert!(PairKerning::new(&t).is_none());
    }
}
