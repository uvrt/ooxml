//! Owns: which face an absent face is laid out with, and how good that substitute is --
//! Word's measured metric compatible pairs ([`WORD_SUBSTITUTES`]), the Office substitution
//! table shared with PowerPoint (`data/substitutions.tsv`, ooxml-common's
//! `fontmap.SUBSTITUTIONS`), the generic class of a face and the faces of each class
//! ([`GENERIC_FALLBACKS`]); docx2svg's `default_substitutes`, `face_class` and ooxml-common's
//! `family_key`, `substitution_for` and `generic_family`.
//!
//! Must not: look a face up (that is [`crate::word`]), or read a document part.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Word's face (lowercased) -> the open face laid out and drawn in its place where Word's
/// is neither installed nor embedded: the pairs measured metric compatible (docx2svg's
/// `fonts.SUBSTITUTES`).
pub const WORD_SUBSTITUTES: [(&str, &str); 4] = [
    ("calibri", "Carlito"),
    ("arial", "Liberation Sans"),
    ("times new roman", "Liberation Serif"),
    ("courier new", "Liberation Mono"),
];

/// A face's generic class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaceClass {
    /// `sans-serif`.
    SansSerif,
    /// `serif`.
    Serif,
    /// `monospace` (fixed pitch).
    Monospace,
}

impl FaceClass {
    /// The CSS generic family's name.
    pub fn name(self) -> &'static str {
        match self {
            FaceClass::SansSerif => "sans-serif",
            FaceClass::Serif => "serif",
            FaceClass::Monospace => "monospace",
        }
    }
}

/// The faces an absent face with no substitute of its own is laid out with, by class,
/// first installed winning (docx2svg's `GENERIC_FALLBACKS`), in its order.
pub const GENERIC_FALLBACKS: [(FaceClass, &[&str]); 3] = [
    (
        FaceClass::SansSerif,
        &["Liberation Sans", "Arimo", "Arial", "Helvetica", "Carlito", "Calibri"],
    ),
    (
        FaceClass::Serif,
        &[
            "Liberation Serif",
            "Tinos",
            "Times New Roman",
            "Times",
            "Caladea",
            "Cambria",
        ],
    ),
    (
        FaceClass::Monospace,
        &["Liberation Mono", "Cousine", "Courier New", "Courier"],
    ),
];

/// The faces of a class.
pub fn generic_fallbacks(class: FaceClass) -> &'static [&'static str] {
    GENERIC_FALLBACKS
        .iter()
        .find(|(c, _)| *c == class)
        .map_or(&[], |(_, faces)| faces)
}

/// One row of the Office substitution table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficeSubstitution {
    /// The family a document asks for.
    pub office: String,
    /// The family it is measured and drawn with.
    pub substitute: String,
    /// Whether the two were measured metric compatible.
    pub metric_compatible: bool,
}

struct Table {
    rows: Vec<OfficeSubstitution>,
    by_key: HashMap<String, usize>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let rows: Vec<OfficeSubstitution> = include_str!("../data/substitutions.tsv")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(|line| {
                let mut fields = line.split('\t');
                Some(OfficeSubstitution {
                    office: fields.next()?.to_owned(),
                    substitute: fields.next()?.to_owned(),
                    metric_compatible: fields.next()? == "1",
                })
            })
            .collect();
        let mut by_key = HashMap::new();
        for (i, row) in rows.iter().enumerate() {
            by_key.entry(family_key(&row.office)).or_insert(i);
        }
        Table { rows, by_key }
    })
}

/// Every row of the Office substitution table, in its order.
pub fn office_substitutions() -> &'static [OfficeSubstitution] {
    &table().rows
}

/// A family name as the substitution table keys it: full-width forms made half-width,
/// outer whitespace stripped, lowercased (ooxml-common's `family_key`).
pub fn family_key(value: &str) -> String {
    let half: String = value
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(u32::from(c) - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect();
    half.trim().to_lowercase()
}

/// Weight and optical-size qualifiers peeled off a family name to find its row.
const WEIGHT_SUFFIXES: [&str; 14] = [
    " light",
    " semilight",
    " semibold",
    " demibold",
    " medium",
    " black",
    " heavy",
    " thin",
    " extralight",
    " extrabold",
    " ultralight",
    " bold",
    " italic",
    " ui",
];

/// The Office substitution row for a family, peeling weight qualifiers off its end
/// (ooxml-common's `substitution_for`).
pub fn substitution_for(family: &str) -> Option<&'static OfficeSubstitution> {
    if family.is_empty() || family.starts_with('+') {
        return None;
    }
    let table = table();
    let mut key = family_key(family);
    loop {
        if let Some(&i) = table.by_key.get(&key) {
            return table.rows.get(i);
        }
        let suffix = WEIGHT_SUFFIXES
            .iter()
            .find(|s| key.ends_with(*s) && key.len() > s.len())?;
        key = key
            .get(..key.len() - suffix.len())
            .unwrap_or_default()
            .trim()
            .to_owned();
    }
}

const BUNDLED_GENERICS: [(&str, FaceClass); 8] = [
    ("arimo", FaceClass::SansSerif),
    ("caladea", FaceClass::Serif),
    ("carlito", FaceClass::SansSerif),
    ("cousine", FaceClass::Monospace),
    ("lato", FaceClass::SansSerif),
    ("noto sans jp", FaceClass::SansSerif),
    ("raleway", FaceClass::SansSerif),
    ("tinos", FaceClass::Serif),
];

const SERIF_HINTS: [&str; 24] = [
    "mincho",
    "明朝",
    "times new roman",
    "georgia",
    "cambria",
    "garamond",
    "book antiqua",
    "palatino",
    "caslon",
    "baskerville",
    "constantia",
    "simsun",
    "nsimsun",
    "宋体",
    "mingliu",
    "細明體",
    "batang",
    "바탕",
    "gungsuh",
    "궁서",
    "kaiti",
    "楷体",
    "fangsong",
    "仿宋",
];

const MONOSPACE_HINTS: [&str; 5] = ["mono", "courier", "consolas", "console", "typewriter"];

/// The class a family's name says it is (ooxml-common's `generic_family`).
pub fn generic_family(family: &str) -> FaceClass {
    let key = family_key(family);
    if let Some((_, class)) = BUNDLED_GENERICS.iter().find(|(name, _)| *name == key) {
        return *class;
    }
    let lowered = family.to_lowercase();
    if SERIF_HINTS.iter().any(|h| lowered.contains(h)) || (lowered.contains("serif") && !lowered.contains("sans")) {
        return FaceClass::Serif;
    }
    if MONOSPACE_HINTS.iter().any(|h| lowered.contains(h)) {
        return FaceClass::Monospace;
    }
    FaceClass::SansSerif
}

/// A face's entry in the document's font table: `w:family` and `w:pitch`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontTableEntry {
    /// `w:family/@w:val` (`roman`, `swiss`, `modern`, ...).
    pub family: Option<String>,
    /// `w:pitch/@w:val` (`fixed`, `variable`, ...).
    pub pitch: Option<String>,
}

/// The document's font table classes, by face name lowercased.
pub type FontTable = HashMap<String, FontTableEntry>;

/// A face's class: what the document's font table says of it, else what its name says
/// (docx2svg's `face_class`).
pub fn face_class(family: &str, font_table: &FontTable) -> FaceClass {
    if let Some(entry) = font_table.get(&family.to_lowercase()) {
        if entry.pitch.as_deref() == Some("fixed") {
            return FaceClass::Monospace;
        }
        match entry.family.as_deref() {
            Some("roman") => return FaceClass::Serif,
            Some("swiss") => return FaceClass::SansSerif,
            Some("modern") => return FaceClass::Monospace,
            _ => {}
        }
    }
    generic_family(family)
}

/// How a substitute was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubstitutionKind {
    /// Named by the caller, or by [`WORD_SUBSTITUTES`].
    Named,
    /// The face PowerPoint rendering uses for it, or the Office face that one clones.
    Default,
    /// The first installed face of its class.
    Generic,
}

/// The faces an absent `family` is laid out with when no substitute is named for it, best
/// first, each with its kind (docx2svg's `default_substitutes`).
pub fn default_substitutes(family: &str, font_table: &FontTable) -> Vec<(String, SubstitutionKind)> {
    let mut out: Vec<(String, SubstitutionKind)> = Vec::new();
    if let Some(row) = substitution_for(family) {
        if row.substitute.to_lowercase() != family.to_lowercase() {
            out.push((row.substitute.clone(), SubstitutionKind::Default));
            for other in office_substitutions() {
                let named = WORD_SUBSTITUTES
                    .iter()
                    .find(|(office, _)| *office == other.office.to_lowercase());
                if other.substitute == row.substitute && named.is_some_and(|(_, s)| *s == row.substitute) {
                    out.push((other.office.clone(), SubstitutionKind::Default));
                }
            }
        }
    }
    for name in generic_fallbacks(face_class(family, font_table)) {
        out.push(((*name).to_owned(), SubstitutionKind::Generic));
    }
    let mut seen = vec![family.to_lowercase()];
    out.retain(|(name, _)| {
        let key = name.to_lowercase();
        let fresh = !seen.contains(&key);
        seen.push(key);
        fresh
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_found_by_key_and_by_peeling_weights() {
        assert_eq!(substitution_for("Calibri").unwrap().substitute, "Carlito");
        assert_eq!(substitution_for(" CALIBRI ").unwrap().substitute, "Carlito");
        assert_eq!(substitution_for("Yu Gothic UI Light").unwrap().office, "Yu Gothic UI");
        assert!(substitution_for("+mj-lt").is_none());
        assert_eq!(family_key("ＭＳ Ｐゴシック"), "ms pゴシック");
    }

    #[test]
    fn classes_and_defaults() {
        let table = FontTable::new();
        assert_eq!(generic_family("Georgia"), FaceClass::Serif);
        assert_eq!(generic_family("Fira Mono"), FaceClass::Monospace);
        assert_eq!(generic_family("PT Serif"), FaceClass::Serif);
        assert_eq!(generic_family("PT Sans Serif"), FaceClass::SansSerif);
        let names: Vec<String> = default_substitutes("Aptos", &table)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names.first().map(String::as_str), Some("Carlito"));
        assert_eq!(names.get(1).map(String::as_str), Some("Calibri"));
        let mut fixed = FontTable::new();
        fixed.insert(
            "odd".into(),
            FontTableEntry {
                family: Some("swiss".into()),
                pitch: Some("fixed".into()),
            },
        );
        assert_eq!(face_class("Odd", &fixed), FaceClass::Monospace);
    }
}
