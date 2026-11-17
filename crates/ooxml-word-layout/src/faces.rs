//! Owns: the port the layout reads faces through ([`Faces`]: the text engine's
//! [`Glyphs`] and a face's vertical metrics), and [`WordGlyphs`], Word's fonts
//! ([`WordFonts`], `ooxml-fonts`) adapted to it -- design section 3, `ooxml-text`: "the
//! Word layout (L3) adapts `ooxml-fonts` to it". Each family's first grade and every
//! glyph fallback are recorded for the coverage report.
//!
//! Must not: decide anything the fonts layer or the text engine decides, or find faces
//! itself.

use ooxml_base::Digest;
use ooxml_fonts::{Decorations, FaceMetrics, Grade, WordFonts};
use ooxml_text::{Advance, FaceKey, Glyphs, ScriptMetrics};
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

/// What the layout reads of faces: the text engine's port, and each face's vertical
/// metrics (`hhea`, or `OS/2`'s typographic ones), which line heights are made of.
pub trait Faces: Glyphs + Send + Sync {
    /// The face's vertical metrics, or `None` where the face is unknown.
    fn metrics(&self, face: &FaceKey) -> Option<FaceMetrics>;
    /// Where the face draws its underline and strikeout, or `None` where it is unknown or
    /// says nothing of them.
    fn decorations(&self, face: &FaceKey) -> Option<Decorations>;
}

/// What the layout's faces were found as: per family its first grade; every glyph taken
/// from another face.
#[derive(Debug, Default, Clone)]
pub struct Coverage {
    /// Family -> the grade of its first lookup.
    pub grades: BTreeMap<String, Grade>,
    /// The families in the order they were first looked up (docx2svg's record order).
    pub order: Vec<String>,
    /// (family, character) -> the face its advance came from.
    pub glyph_fallbacks: BTreeMap<(String, char), String>,
    /// The `(family, character)` of [`Coverage::glyph_fallbacks`] in the order first met.
    pub fallback_order: Vec<(String, char)>,
}

/// Word's fonts, as the layout and the text engine read them.
pub struct WordGlyphs {
    fonts: WordFonts,
    coverage: Mutex<Coverage>,
}

impl std::fmt::Debug for WordGlyphs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WordGlyphs").finish_non_exhaustive()
    }
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
            coverage.order.push(face.family.to_string());
        }
    }
}

impl Glyphs for WordGlyphs {
    fn advance(&self, face: &FaceKey, ch: char) -> Option<Advance> {
        self.note(face);
        let found = self.fonts.advance(&face.family, face.bold, face.italic, ch)?;
        if let Some(fallback) = &found.fallback {
            let mut coverage = self.coverage.lock().unwrap_or_else(PoisonError::into_inner);
            let key = (face.family.to_string(), ch);
            if !coverage.glyph_fallbacks.contains_key(&key) {
                coverage.fallback_order.push(key.clone());
                coverage.glyph_fallbacks.insert(key, fallback.clone());
            }
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

impl Faces for WordGlyphs {
    fn metrics(&self, face: &FaceKey) -> Option<FaceMetrics> {
        self.note(face);
        self.fonts.metrics(&face.family, face.bold, face.italic)
    }

    fn decorations(&self, face: &FaceKey) -> Option<Decorations> {
        self.fonts.decorations(&face.family, face.bold, face.italic)
    }
}
