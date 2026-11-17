//! Owns: the measured tables PowerPoint's text is measured with (`data/office-faces.tsv`,
//! ooxml-common's `text.metrics.METRICS` and its OpenType kern classes) -- each face's
//! units per em, ascender, descender and line gap, advance widths upright and bold, and kern
//! class matrix -- and which table a family is measured with ([`office_metrics_for`],
//! ooxml-common's `fontmap.metrics_for`).
//!
//! Must not: sum advances into a width (the text engine does, through its port), or read a
//! font file.

use crate::substitutes::substitution_for;
use std::collections::HashMap;
use std::sync::OnceLock;

/// One face's kern classes and cells.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct KernClasses {
    left: HashMap<char, u32>,
    right: HashMap<char, u32>,
    cells: HashMap<(u32, u32), i32>,
}

impl KernClasses {
    fn adjustment(&self, left: char, right: char) -> i32 {
        let (Some(l), Some(r)) = (self.left.get(&left), self.right.get(&right)) else {
            return 0;
        };
        self.cells.get(&(*l, *r)).copied().unwrap_or(0)
    }
}

/// One measured face.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OfficeFace {
    /// The table's name (`Carlito`, `Aptos`).
    pub name: String,
    /// Units per em.
    pub units_per_em: u32,
    /// `hhea` ascender, font units.
    pub ascender: i32,
    /// `hhea` descender, font units (negative).
    pub descender: i32,
    /// The Office face's line gap, font units, where it was measured.
    pub line_gap: Option<i32>,
    /// The advance of a character with no entry.
    pub default_width: u32,
    /// The advance of an East Asian character.
    pub cjk_width: u32,
    /// Bold equivalents.
    pub bold_default_width: u32,
    /// Bold equivalents.
    pub bold_cjk_width: u32,
    /// Whether the face is a variable one.
    pub variable: bool,
    widths: HashMap<char, u32>,
    bold_widths: HashMap<char, u32>,
    kerning: Option<KernClasses>,
    bold_kerning: Option<KernClasses>,
    legacy_kerning: Option<KernClasses>,
    legacy_bold_kerning: Option<KernClasses>,
}

/// One of a face's kern tables (ooxml-common's `kerning.KERNING_TABLES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernTable {
    /// The OpenType `kern` feature (`GPOS`).
    Feature,
    /// The legacy `kern` table of the face the table stands for.
    Legacy,
    /// No kerning.
    None,
}

/// Which of a face's kern pairs an application lays a line out with
/// (ooxml-common's `kerning.KerningSource`): one table for a static face, one for a
/// variable one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KerningSource {
    /// The table a static face is kerned from.
    pub static_face: KernTable,
    /// The table a variable face is kerned from.
    pub variable_face: KernTable,
}

impl KerningSource {
    /// The OpenType feature for every face: the shared default.
    pub const FEATURE: KerningSource = KerningSource {
        static_face: KernTable::Feature,
        variable_face: KernTable::Feature,
    };
    /// PowerPoint's (measured): a static face's legacy table, a variable face's feature.
    pub const POWERPOINT: KerningSource = KerningSource {
        static_face: KernTable::Legacy,
        variable_face: KernTable::Feature,
    };
    /// Word's, where a run kerns at all: the legacy table, nothing for a variable face.
    pub const WORD: KerningSource = KerningSource {
        static_face: KernTable::Legacy,
        variable_face: KernTable::None,
    };
    /// No kerning.
    pub const NONE: KerningSource = KerningSource {
        static_face: KernTable::None,
        variable_face: KernTable::None,
    };
}

impl OfficeFace {
    /// The measured advance of `ch`, upright or bold, where the table has one.
    pub fn advance(&self, ch: char, bold: bool) -> Option<u32> {
        if bold { &self.bold_widths } else { &self.widths }.get(&ch).copied()
    }

    /// Whether the table has bold advances.
    pub fn has_bold_widths(&self) -> bool {
        !self.bold_widths.is_empty()
    }

    /// Whether the bold table says nothing the upright one does not (no bold cut).
    pub fn bold_is_indistinguishable(&self) -> bool {
        self.bold_widths == self.widths && self.bold_cjk_width == self.cjk_width
    }

    /// Whether the face kerns (its OpenType kern feature).
    pub fn kerns(&self) -> bool {
        self.kerning.is_some()
    }

    /// The OpenType kern adjustment between two characters, font units; the bold classes
    /// when `bold` and the face has them.
    pub fn kern(&self, left: char, right: char, bold: bool) -> i32 {
        let table = match (&self.bold_kerning, &self.kerning) {
            (Some(b), _) if bold => Some(b),
            (_, upright) => upright.as_ref(),
        };
        table.map_or(0, |t| t.adjustment(left, right))
    }

    /// Whether `source` names a table this face has (`kern_table(source) is not None`).
    pub fn kerns_from(&self, source: KerningSource) -> bool {
        self.tables(source).0.is_some()
    }

    /// The adjustment between two characters from the table `source` names for this face,
    /// font units; its bold classes when `bold` and the table has them. 0 where it names
    /// none.
    pub fn kern_from(&self, source: KerningSource, left: char, right: char, bold: bool) -> i32 {
        let (upright, bold_table) = self.tables(source);
        let table = match (bold_table, upright) {
            (Some(b), _) if bold => Some(b),
            (_, upright) => upright,
        };
        table.map_or(0, |t| t.adjustment(left, right))
    }

    fn tables(&self, source: KerningSource) -> (Option<&KernClasses>, Option<&KernClasses>) {
        let which = if self.variable {
            source.variable_face
        } else {
            source.static_face
        };
        match which {
            KernTable::Feature => (self.kerning.as_ref(), self.bold_kerning.as_ref()),
            KernTable::Legacy => (self.legacy_kerning.as_ref(), self.legacy_bold_kerning.as_ref()),
            KernTable::None => (None, None),
        }
    }
}

fn faces() -> &'static HashMap<String, OfficeFace> {
    static FACES: OnceLock<HashMap<String, OfficeFace>> = OnceLock::new();
    FACES.get_or_init(|| parse(include_str!("../data/office-faces.tsv")))
}

fn parse(text: &str) -> HashMap<String, OfficeFace> {
    let mut out: HashMap<String, OfficeFace> = HashMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        let (Some(&kind), Some(&name)) = (fields.first(), fields.get(1)) else {
            continue;
        };
        let rest = fields.get(2..).unwrap_or(&[]);
        let face = out.entry(name.to_owned()).or_insert_with(|| OfficeFace {
            name: name.to_owned(),
            ..OfficeFace::default()
        });
        match kind {
            "face" => read_face(face, rest),
            "widths" => face.widths = advances(rest.first().copied().unwrap_or("")),
            "bold-widths" => face.bold_widths = advances(rest.first().copied().unwrap_or("")),
            _ => read_kern(face, kind, rest),
        }
    }
    out
}

fn read_face(face: &mut OfficeFace, fields: &[&str]) {
    let n = |i: usize| fields.get(i).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let u = |i: usize| u32::try_from(n(i)).unwrap_or(0);
    face.units_per_em = u(0);
    face.ascender = i32::try_from(n(1)).unwrap_or(0);
    face.descender = i32::try_from(n(2)).unwrap_or(0);
    face.line_gap = fields.get(3).and_then(|v| v.parse().ok());
    face.default_width = u(4);
    face.cjk_width = u(5);
    face.bold_default_width = u(6);
    face.bold_cjk_width = u(7);
    face.variable = n(8) == 1;
}

fn code(hex: &str) -> Option<char> {
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
}

fn advances(field: &str) -> HashMap<char, u32> {
    field
        .split(' ')
        .filter_map(|pair| {
            let (c, a) = pair.split_once(':')?;
            Some((code(c)?, a.parse().ok()?))
        })
        .collect()
}

fn classes(fields: &[&str]) -> HashMap<char, u32> {
    let mut out = HashMap::new();
    for (i, group) in fields.iter().enumerate() {
        let class = u32::try_from(i + 1).unwrap_or(u32::MAX);
        for ch in group.split(',').filter_map(code) {
            out.insert(ch, class);
        }
    }
    out
}

fn read_kern(face: &mut OfficeFace, kind: &str, fields: &[&str]) {
    let (legacy, kind) = match kind.strip_prefix("legacy-") {
        Some(rest) => (true, rest),
        None => (false, kind),
    };
    let (bold, what) = match kind.strip_prefix("bold-") {
        Some(rest) => (true, rest),
        None => (false, kind),
    };
    let table = match (legacy, bold) {
        (false, false) => &mut face.kerning,
        (false, true) => &mut face.bold_kerning,
        (true, false) => &mut face.legacy_kerning,
        (true, true) => &mut face.legacy_bold_kerning,
    };
    let table = table.get_or_insert_with(KernClasses::default);
    match what {
        "kern-left" => table.left = classes(fields),
        "kern-right" => table.right = classes(fields),
        "kern-cells" => {
            table.cells = fields
                .first()
                .copied()
                .unwrap_or("")
                .split(' ')
                .filter_map(|cell| {
                    let mut parts = cell.split(':');
                    let l = parts.next()?.parse().ok()?;
                    let r = parts.next()?.parse().ok()?;
                    Some(((l, r), parts.next()?.parse().ok()?))
                })
                .collect();
        }
        _ => {}
    }
}

/// A measured table by its name.
pub fn office_face(name: &str) -> Option<&'static OfficeFace> {
    faces().get(name)
}

/// The table a family is measured with (its substitution row's), or `None` when nothing
/// is known of it.
pub fn office_metrics_for(family: &str) -> Option<&'static OfficeFace> {
    substitution_for(family).and_then(|row| office_face(&row.metrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_read() {
        let carlito = office_face("Carlito").unwrap();
        assert_eq!(
            (carlito.units_per_em, carlito.ascender, carlito.descender),
            (2048, 1950, -550)
        );
        assert_eq!(carlito.advance(' ', false), Some(463));
        assert!(carlito.kerns());
        assert_eq!(office_metrics_for("Calibri").unwrap().name, "Carlito");
        assert!(office_metrics_for("No Such Face").is_none());
    }
}
