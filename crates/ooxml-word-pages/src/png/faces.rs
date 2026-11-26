//! Owns: the faces a rasteriser is handed to draw some pages of a version as Word draws
//! them (docx2svg's `drawn_faces` and `rasteriser_files`, as `convert_docx_to_png` hands
//! them over): every `(face, bold, italic)` the pages draw text in -- their lines' and
//! their drawings' (a text path's, a chart's or a diagram's text) -- each as the file Word
//! draws it with, read in place, or the face the document embeds, as its bytes (never a
//! Microsoft face: then it counts as missing); the face the generic `sans-serif` names (a
//! placeholder's label: Arial, Helvetica or Calibri, the first installed); and whether
//! any face is missing (then the system's fonts are handed over too).
//!
//! Must not: read a font file (the faces are the version's, resolved when it was laid out)
//! or draw.

use crate::{LaidVersion, PageEngine};
use ooxml_base::{Digest, DigestKey, Digester};
use ooxml_fonts::Face;
use ooxml_word_layout::paint::names::PageNames;
use std::collections::BTreeSet;
use std::sync::Arc;

/// One file a rasteriser is handed.
#[derive(Debug, Clone)]
pub enum RasterFile {
    /// A font file, read where it is installed.
    Path(String),
    /// A face the document embeds: its bytes, de-obfuscated.
    Embedded(Arc<[u8]>),
}

impl PartialEq for RasterFile {
    fn eq(&self, other: &RasterFile) -> bool {
        match (self, other) {
            (RasterFile::Path(a), RasterFile::Path(b)) => a == b,
            (RasterFile::Embedded(a), RasterFile::Embedded(b)) => Arc::ptr_eq(a, b) || a == b,
            _ => false,
        }
    }
}

/// The faces a rasteriser is handed for some pages.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RasterFaces {
    /// The files, in the order docx2svg hands them over.
    pub files: Vec<RasterFile>,
    /// The families no file holds here (the system's fonts are handed over then).
    pub missing: Vec<String>,
    /// The family the generic `sans-serif` names, where one of Arial, Helvetica and Calibri
    /// is installed.
    pub sans_serif: Option<String>,
    /// Whether the system's fonts are handed over, where the caller says (docx2svg's
    /// `skip_system_fonts` given); `None`: only where a face is missing.
    pub system: Option<bool>,
}

impl RasterFaces {
    /// Whether the system's fonts are left out: as the caller says, else where every face
    /// drawn was found.
    #[must_use]
    pub fn skip_system(&self) -> bool {
        self.system.map_or(self.missing.is_empty(), |system| !system)
    }

    /// What tells two sets apart: the files (an embedded face by its bytes' digest), the
    /// missing families' presence, the `sans-serif` face.
    #[must_use]
    pub fn key(&self) -> Digest {
        let mut hasher = Digester::new(KEY);
        for file in &self.files {
            match file {
                RasterFile::Path(path) => {
                    hasher.update(b"p");
                    hasher.update(path.as_bytes());
                }
                RasterFile::Embedded(bytes) => {
                    hasher.update(b"e");
                    hasher.update(&Digest::of(KEY, bytes).0);
                }
            }
            hasher.update(b"\0");
        }
        hasher.update(if self.skip_system() { b"skip" } else { b"system" });
        hasher.update(self.sans_serif.as_deref().unwrap_or_default().as_bytes());
        hasher.finish()
    }
}

/// What [`RasterFaces::key`] is keyed with.
const KEY: DigestKey = DigestKey::new(0x6f6f_786d_6c2d_7261, 0x7374_6572_2d52_4431);

/// Families tried, in order, for the generic `sans-serif` face.
const SANS_SERIF: [&str; 3] = ["Arial", "Helvetica", "Calibri"];

/// Whether `source` names a face the document embeds.
fn embedded(face: &Face) -> bool {
    face.source().starts_with("embedded:")
}

/// A face whose copyright, trademark or manufacturer names Microsoft: never handed over
/// from a document.
fn microsoft(face: &Face) -> bool {
    face.names()
        .iter()
        .any(|n| matches!(n.name_id, 0 | 7 | 8) && n.text.to_lowercase().contains("microsoft"))
}

impl PageEngine {
    /// Every `(face, bold, italic)` `pages` (0-based, as [`PageEngine::paint`] counts them)
    /// of `version` draw text in, sorted.
    pub fn drawn_faces(&mut self, version: &LaidVersion, pages: &[usize]) -> Vec<(String, bool, bool)> {
        let mut faces: BTreeSet<(String, bool, bool)> = BTreeSet::new();
        for &page in pages {
            let Some(paint) = self.paint(version, page) else {
                continue;
            };
            for line in &paint.lines {
                for span in line.spans.iter().filter(|s| !s.chars.is_empty()) {
                    faces.insert((span.face.to_string(), span.bold, span.italic));
                }
            }
            if let Some(drawings) = &version.drawings {
                let names = PageNames::of_page(&version.layout().document.names, &paint);
                for slot in &paint.drawings {
                    faces.extend(drawings.faces(slot, &names.slot_paths(slot)));
                }
            }
        }
        faces.into_iter().collect()
    }

    /// The faces a rasteriser is handed to draw `pages` (0-based) of `version` as Word draws
    /// them.
    pub fn raster_faces(&mut self, version: &LaidVersion, pages: &[usize]) -> RasterFaces {
        let drawing = Arc::clone(&version.drawing);
        let mut out = RasterFaces::default();
        for (family, bold, italic) in self.drawn_faces(version, pages) {
            let Some(face) = drawing.drawing_face(&family, bold, italic) else {
                out.missing.push(family);
                continue;
            };
            let file = if embedded(&face) {
                if microsoft(&face) {
                    out.missing.push(family);
                    continue;
                }
                RasterFile::Embedded(Arc::clone(face.data()))
            } else {
                RasterFile::Path(face.source().to_owned())
            };
            if !out.files.contains(&file) {
                out.files.push(file);
            }
        }
        for family in SANS_SERIF {
            let Some(face) = drawing.drawing_face(family, false, false).filter(|f| !embedded(f)) else {
                continue;
            };
            out.sans_serif = Some(family.to_owned());
            let file = RasterFile::Path(face.source().to_owned());
            if !out.files.contains(&file) {
                out.files.push(file);
            }
            break;
        }
        out
    }
}
