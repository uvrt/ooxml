//! Owns: the two symbol faces Word lays out from integers recorded from its own copies
//! where the machine has neither the face nor an embedded copy -- Symbol and Wingdings
//! (ooxml-common's `text.recorded_symbol_faces`, written into `data/recorded-faces.tsv` by
//! `data/generate.py`): their metrics, decorations and advances.
//!
//! Must not: draw (a recorded face has no outlines), or record anything new: the data is
//! measured from Office by the generators, never transcribed.

use crate::sfnt::{Decorations, FaceMetrics};
use std::collections::HashMap;
use std::sync::OnceLock;

const TABLE: &str = include_str!("../data/recorded-faces.tsv");

/// A symbol face answered from recorded integers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedFace {
    /// The family, lowercased.
    pub family: String,
    /// Its line and script metrics.
    pub metrics: FaceMetrics,
    /// Its decorations' geometry.
    pub decorations: Decorations,
    advances: HashMap<u32, u16>,
}

impl RecordedFace {
    /// The advance of `ch`, font units, if recorded.
    pub fn advance(&self, ch: char) -> Option<u16> {
        self.advances.get(&u32::from(ch)).copied()
    }
}

fn ints(fields: &[&str]) -> Vec<i32> {
    fields.iter().map(|f| f.parse().unwrap_or_default()).collect()
}

fn parse() -> HashMap<String, RecordedFace> {
    let mut out: HashMap<String, RecordedFace> = HashMap::new();
    for line in TABLE.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        let (Some(kind), Some(family)) = (fields.first(), fields.get(1)) else {
            continue;
        };
        let rest = fields.get(2..).unwrap_or_default();
        let face = out.entry((*family).to_owned()).or_insert_with(|| RecordedFace {
            family: (*family).to_owned(),
            metrics: FaceMetrics {
                units_per_em: 0,
                ascent: 0,
                descent: 0,
                line_gap: 0,
                superscript_size: None,
                subscript_size: None,
                superscript_offset: None,
                subscript_offset: None,
            },
            decorations: Decorations::default(),
            advances: HashMap::new(),
        });
        let v = ints(rest);
        let at = |k: usize| v.get(k).copied().unwrap_or_default();
        let small = |k: usize| i16::try_from(at(k)).ok();
        match *kind {
            "metrics" => {
                face.metrics = FaceMetrics {
                    units_per_em: u16::try_from(at(0)).unwrap_or_default(),
                    ascent: at(1),
                    descent: at(2),
                    line_gap: at(3),
                    superscript_size: small(4),
                    subscript_size: small(5),
                    superscript_offset: small(6),
                    subscript_offset: small(7),
                }
            }
            "decorations" => {
                face.decorations = Decorations {
                    units_per_em: u16::try_from(at(0)).unwrap_or_default(),
                    superscript_offset: small(1),
                    subscript_offset: small(2),
                    strikeout_position: small(3),
                    strikeout_size: small(4),
                    underline_position: small(5),
                    underline_thickness: small(6),
                    hhea_ascent: at(7),
                    hhea_descent: at(8),
                }
            }
            "advances" => {
                for pair in rest.first().copied().unwrap_or_default().split_whitespace() {
                    if let Some((code, advance)) = pair.split_once(':') {
                        if let (Ok(code), Ok(advance)) = (u32::from_str_radix(code, 16), advance.parse()) {
                            face.advances.insert(code, advance);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The recorded face of `family` (any case), if there is one.
pub fn recorded_face(family: &str) -> Option<&'static RecordedFace> {
    static FACES: OnceLock<HashMap<String, RecordedFace>> = OnceLock::new();
    FACES.get_or_init(parse).get(&family.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_and_wingdings_are_recorded() {
        let symbol = recorded_face("Symbol").unwrap();
        assert_eq!(symbol.metrics.units_per_em, 2048);
        assert_eq!(symbol.metrics.ascent, 2059);
        assert_eq!(symbol.advance(' '), Some(512));
        assert_eq!(symbol.decorations.underline_position, Some(-223));
        assert!(recorded_face("wingdings").unwrap().advance('\u{f020}').is_some());
        assert!(recorded_face("Calibri").is_none());
    }
}
