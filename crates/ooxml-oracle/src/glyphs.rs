//! Owns: [`WordGlyphs`], Word's fonts ([`WordFonts`]) as the text engine's [`Glyphs`]
//! port, recording each family's first grade and every glyph fallback, for the report.
//!
//! Must not: decide anything the fonts layer or the engine decides.

use ooxml_base::Digest;
use ooxml_fonts::{Grade, WordFonts};
use ooxml_text::{Advance, FaceKey, Glyphs, ScriptMetrics};
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

/// What the layout's faces were found as: per family its first grade; every glyph taken
/// from another face.
#[derive(Debug, Default, Clone)]
pub struct Coverage {
    /// Family -> the grade of its first lookup.
    pub grades: BTreeMap<String, Grade>,
    /// (family, character) -> the face its advance came from.
    pub glyph_fallbacks: BTreeMap<(String, char), String>,
}

/// Word's fonts, as the text engine reads them.
pub struct WordGlyphs {
    fonts: WordFonts,
    coverage: Mutex<Coverage>,
}

impl WordGlyphs {
    /// The adapter over `fonts`.
    pub fn new(fonts: WordFonts) -> WordGlyphs {
        WordGlyphs {
            fonts,
            coverage: Mutex::new(Coverage::default()),
        }
    }

    /// What the lookups found so far.
    pub fn coverage(&self) -> Coverage {
        self.coverage.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn note(&self, face: &FaceKey) {
        let mut coverage = self.coverage.lock().unwrap_or_else(PoisonError::into_inner);
        if !coverage.grades.contains_key(face.family.as_ref()) {
            let grade = self.fonts.resolve(&face.family, face.bold, face.italic).grade;
            coverage.grades.insert(face.family.to_string(), grade);
        }
    }
}

impl Glyphs for WordGlyphs {
    fn advance(&self, face: &FaceKey, ch: char) -> Option<Advance> {
        self.note(face);
        let found = self.fonts.advance(&face.family, face.bold, face.italic, ch)?;
        if let Some(fallback) = &found.fallback {
            let mut coverage = self.coverage.lock().unwrap_or_else(PoisonError::into_inner);
            coverage
                .glyph_fallbacks
                .entry((face.family.to_string(), ch))
                .or_insert_with(|| fallback.clone());
        }
        Some(Advance {
            advance: u32::from(found.advance),
            units_per_em: u32::from(found.units_per_em),
        })
    }

    fn kern(&self, face: &FaceKey, left: char, right: char) -> Option<i32> {
        self.fonts
            .kern(&face.family, face.bold, face.italic, left, right)
            .map(i32::from)
    }

    fn script_metrics(&self, face: &FaceKey) -> Option<ScriptMetrics> {
        self.note(face);
        let m = self.fonts.metrics(&face.family, face.bold, face.italic)?;
        Some(ScriptMetrics {
            units_per_em: u32::from(m.units_per_em),
            superscript_size: m.superscript_size.map(i32::from),
            subscript_size: m.subscript_size.map(i32::from),
            superscript_offset: m.superscript_offset.map(i32::from),
            subscript_offset: m.subscript_offset.map(i32::from),
        })
    }

    fn epoch(&self) -> Digest {
        self.fonts.epoch()
    }
}
