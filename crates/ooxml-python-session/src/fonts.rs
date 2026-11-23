//! Owns: `ooxml_core.WordFonts` (lane L4): a machine's fonts for one set of folders, scanned
//! once and shared by every engine made with them; which installed face a family is laid out
//! with (`face`), and Word's measured substitutes (`default_substitutes`).
//!
//! Must not: lay out, or read a font file beyond the scan (ooxml-fonts does).

use ooxml_fonts::substitutes::WORD_SUBSTITUTES;
use ooxml_fonts::{Folder, Location, WordFontOptions};
use ooxml_word_pages::{DiskFonts, FontSetup};
use pyo3::prelude::*;
use std::sync::Arc;

/// A machine's fonts for one set of folders (Word's order) and options, scanned once and
/// shared by every document laid out with them.
#[pyclass(frozen, name = "WordFonts", module = "ooxml_core")]
pub struct PyWordFonts {
    inner: Arc<FontSetup>,
}

impl PyWordFonts {
    /// The fonts, as a session's engines share them.
    pub(crate) fn setup(&self) -> Arc<FontSetup> {
        Arc::clone(&self.inner)
    }
}

#[pymethods]
impl PyWordFonts {
    /// `folders`: `(location, path)` pairs, `location` `"bundle"` for Word's own bundle, else
    /// `"folder"`; `substitutes`: face (lowercased) -> the face used in its place.
    #[new]
    #[pyo3(signature = (folders, *, substitutes = None, recorded_faces = true, fallbacks = true))]
    fn new(
        py: Python<'_>,
        folders: Vec<(String, String)>,
        substitutes: Option<Vec<(String, String)>>,
        recorded_faces: bool,
        fallbacks: bool,
    ) -> Self {
        let folders: Vec<Folder> = folders
            .into_iter()
            .map(|(location, path)| Folder {
                location: if location == "bundle" {
                    Location::WordBundle
                } else {
                    Location::Folder
                },
                path,
            })
            .collect();
        let options = WordFontOptions {
            substitutes: substitutes.unwrap_or_else(|| WordFontOptions::default().substitutes),
            recorded_faces,
            fallbacks,
        };
        let inner = py.detach(|| Arc::new(FontSetup::new(Arc::new(DiskFonts), &folders, options)));
        Self { inner }
    }

    /// The file of the installed face `family` is laid out with in a style (the style asked
    /// for, else bold, else regular), or `None` where no folder holds one: a substitute or a
    /// recorded face does not count.
    #[pyo3(signature = (family, bold = false, italic = false))]
    fn face(&self, family: &str, bold: bool, italic: bool) -> Option<String> {
        self.inner.installed(family, bold, italic).map(str::to_owned)
    }

    /// Word's faces (lowercased) and the open faces measured to their metrics, laid out in
    /// their place where they are absent: what `substitutes` holds when it is not given.
    #[staticmethod]
    fn default_substitutes() -> Vec<(&'static str, &'static str)> {
        WORD_SUBSTITUTES.to_vec()
    }
}
