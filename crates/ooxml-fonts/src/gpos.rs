//! Owns: `GPOS`'s `kern` feature, pair adjustments only -- the first glyph's advance change
//! (`PairPos` formats 1 and 2, through `Extension` lookups; each lookup's first subtable
//! that covers the pair) -- which Word applies to a watermark's letters. A port of
//! docx2svg's `outlines.PairKerning`.
//!
//! Must not: shape text, or apply any other lookup or feature (Word's body text is kerned
//! by the legacy `kern` table: [`crate::kern`]).

use crate::be::{i16_at, u16_at, u32_at};
use std::collections::HashMap;

/// A format 2 subtable's classes: the first glyph's, the second's, and their counts.
type Classes = (HashMap<u16, u16>, HashMap<u16, u16>, u16, u16);

/// One `PairPos` subtable read.
struct PairPos {
    format: u16,
    coverage: HashMap<u16, usize>,
    first_value: u16,
    second_value: u16,
    classes: Option<Classes>,
}

/// A `GPOS` table's `kern` pair adjustments.
pub(crate) struct PairKerning {
    table: Vec<u8>,
    /// Per lookup, its pair subtables.
    lookups: Vec<Vec<(usize, PairPos)>>,
}

fn coverage(table: &[u8], at: usize) -> Option<HashMap<u16, usize>> {
    let format = u16_at(table, at)?;
    let count = usize::from(u16_at(table, at + 2)?);
    let mut out = HashMap::new();
    if format == 1 {
        for i in 0..count {
            out.insert(u16_at(table, at + 4 + 2 * i)?, i);
        }
        return Some(out);
    }
    for i in 0..count {
        let start = u16_at(table, at + 4 + 6 * i)?;
        let end = u16_at(table, at + 6 + 6 * i)?;
        let index = usize::from(u16_at(table, at + 8 + 6 * i)?);
        for glyph in start..=end {
            out.insert(glyph, index + usize::from(glyph - start));
        }
    }
    Some(out)
}

fn class_def(table: &[u8], at: usize) -> Option<HashMap<u16, u16>> {
    let format = u16_at(table, at)?;
    let mut out = HashMap::new();
    if format == 1 {
        let first = u16_at(table, at + 2)?;
        let count = u16_at(table, at + 4)?;
        for i in 0..count {
            let class = u16_at(table, at + 6 + 2 * usize::from(i))?;
            if class != 0 {
                out.insert(first.wrapping_add(i), class);
            }
        }
        return Some(out);
    }
    let count = usize::from(u16_at(table, at + 2)?);
    for i in 0..count {
        let start = u16_at(table, at + 4 + 6 * i)?;
        let end = u16_at(table, at + 6 + 6 * i)?;
        let class = u16_at(table, at + 8 + 6 * i)?;
        if class != 0 {
            for glyph in start..=end {
                out.insert(glyph, class);
            }
        }
    }
    Some(out)
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
        let features = usize::from(u16_at(table, 6)?);
        let lookups_at = usize::from(u16_at(table, 8)?);
        let count = usize::from(u16_at(table, features)?);
        let mut indices: Vec<u16> = Vec::new();
        for i in 0..count {
            let record = features + 2 + 6 * i;
            let tag = table.get(record..record + 4)?;
            let offset = usize::from(u16_at(table, record + 4)?);
            if tag == b"kern" {
                let at = features + offset;
                let n = usize::from(u16_at(table, at + 2)?);
                for k in 0..n {
                    let index = u16_at(table, at + 4 + 2 * k)?;
                    if !indices.contains(&index) {
                        indices.push(index);
                    }
                }
            }
        }
        indices.sort_unstable();
        let lookup_count = u16_at(table, lookups_at)?;
        let mut lookups = Vec::new();
        for index in indices {
            if index >= lookup_count {
                continue;
            }
            let at = lookups_at + usize::from(u16_at(table, lookups_at + 2 + 2 * usize::from(index))?);
            let kind = u16_at(table, at)?;
            let n = usize::from(u16_at(table, at + 4)?);
            let mut found = Vec::new();
            for j in 0..n {
                let mut sub = at + usize::from(u16_at(table, at + 6 + 2 * j)?);
                let mut sub_kind = kind;
                if kind == 9 {
                    sub_kind = u16_at(table, sub + 2)?;
                    sub += usize::try_from(u32_at(table, sub + 4)?).ok()?;
                }
                if sub_kind == 2 {
                    found.push(sub);
                }
            }
            if !found.is_empty() {
                lookups.push(found);
            }
        }
        if lookups.is_empty() {
            return None;
        }
        let lookups = lookups
            .into_iter()
            .map(|subs| subs.into_iter().map(|at| Some((at, pair_pos(table, at)?))).collect())
            .collect::<Option<Vec<Vec<_>>>>()?;
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
                let Some(&index) = sub.coverage.get(&left) else {
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
                        let c1 = first.get(&left).copied().unwrap_or(0);
                        let c2 = second.get(&right).copied().unwrap_or(0);
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
