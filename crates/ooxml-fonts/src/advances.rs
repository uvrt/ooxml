//! Owns: the upright advance tables PowerPoint's text and charts are measured with
//! (ooxml-common's `text.metrics.METRICS`, written into `data/advances.tsv` by
//! `data/generate.py`): per table the units per em, ascender, descender, the default and
//! East Asian widths, the Office face's line gap where measured, and each character's
//! advance; and which table a family is measured with ([`metrics_for`], ooxml-common's
//! `fontmap.metrics_for`) and whether it draws East Asian text ([`covers_east_asian`]).
//!
//! Must not: kern, shape or read a font file; record anything new (the data is generated
//! from the tables ooxml-common extracted from the faces it draws with).

use crate::substitutes::substitution_for;
use std::collections::HashMap;
use std::sync::OnceLock;

/// One face's upright advances, in font units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceTable {
    /// The table's name (the face it was extracted from).
    pub name: String,
    /// Units per em.
    pub units_per_em: u32,
    /// The ascender.
    pub ascender: i32,
    /// The descender (negative below the baseline).
    pub descender: i32,
    /// The width of a character with no entry.
    pub default_width: u32,
    /// The width of an East Asian character with no entry.
    pub cjk_width: u32,
    /// The Office face's `hhea` line gap; `None` where unmeasured.
    pub line_gap: Option<u32>,
    widths: HashMap<char, u32>,
}

impl AdvanceTable {
    /// The advance of `ch`, if the table has one.
    pub fn width(&self, ch: char) -> Option<u32> {
        self.widths.get(&ch).copied()
    }
}

fn parse(line: &str) -> Option<AdvanceTable> {
    let mut fields = line.split('\t');
    let name = fields.next()?.to_owned();
    let units_per_em = fields.next()?.parse().ok()?;
    let ascender = fields.next()?.parse().ok()?;
    let descender = fields.next()?.parse().ok()?;
    let default_width = fields.next()?.parse().ok()?;
    let cjk_width = fields.next()?.parse().ok()?;
    let line_gap = match fields.next()? {
        "-" => None,
        gap => Some(gap.parse().ok()?),
    };
    let widths = fields
        .next()
        .unwrap_or("")
        .split(' ')
        .filter(|p| !p.is_empty())
        .filter_map(|pair| {
            let (code, advance) = pair.split_once(':')?;
            Some((
                char::from_u32(u32::from_str_radix(code, 16).ok()?)?,
                advance.parse().ok()?,
            ))
        })
        .collect();
    Some(AdvanceTable {
        name,
        units_per_em,
        ascender,
        descender,
        default_width,
        cjk_width,
        line_gap,
        widths,
    })
}

fn tables() -> &'static HashMap<String, AdvanceTable> {
    static TABLES: OnceLock<HashMap<String, AdvanceTable>> = OnceLock::new();
    TABLES.get_or_init(|| {
        include_str!("../data/advances.tsv")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(parse)
            .map(|t| (t.name.clone(), t))
            .collect()
    })
}

/// The table named `name`.
pub fn advance_table(name: &str) -> Option<&'static AdvanceTable> {
    tables().get(name)
}

/// The table a family is measured with (its substitution row's), or `None` without data.
pub fn metrics_for(family: Option<&str>) -> Option<&'static AdvanceTable> {
    advance_table(&substitution_for(family?)?.metrics)
}

/// Whether a family can draw Japanese, Chinese or Korean text (its row says so).
pub fn covers_east_asian(family: &str) -> bool {
    substitution_for(family).is_some_and(|row| row.east_asian)
}

/// Whether a code point is in the East Asian ranges every face here draws full width
/// (ooxml-common's `text.measure.is_cjk`).
pub fn is_cjk(ch: char) -> bool {
    matches!(u32::from(ch), 0x3000..=0x9FFF | 0xF900..=0xFAFF | 0xFF01..=0xFF60 | 0x20000..=0x2A6DF)
}

#[cfg(test)]
mod tests {
    use super::{is_cjk, metrics_for};

    #[test]
    fn arial_is_measured_with_arimo() {
        let table = metrics_for(Some("Arial")).unwrap();
        assert_eq!(
            (table.units_per_em, table.ascender, table.descender),
            (2048, 1854, -434)
        );
        assert_eq!(table.line_gap, Some(67));
        assert!(metrics_for(Some("+mn-lt")).is_none());
        assert!(is_cjk('\u{3042}') && !is_cjk('a'));
    }
}
