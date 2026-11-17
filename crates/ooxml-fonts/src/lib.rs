//! The fonts layer of the shared text engine (docs/architecture/README.md, sections 3 and
//! 8; layer 1).
//!
//! Owns: the one sfnt reader ([`Face`], on read-fonts), Office's measured face data (the
//! recorded symbol faces, the substitution table), Word's font lookup order and
//! substitution grades ([`WordFonts`], [`Grade`]), embedded faces, and the fonts epoch a
//! cached measure is keyed by ([`WordFonts::epoch`]).
//!
//! Must not: know about paragraphs, runs or styles; do I/O itself (files are read through
//! the [`FontStore`] port, whose adapters live in leaf crates); hold global mutable state
//! (its only statics are tables parsed once from data compiled in).
//!
//! Not here yet: PowerPoint's lookup order, EOT/MTX (PowerPoint's embedded fonts) and the
//! names and CSS keys a rasteriser files faces under -- they come with the slides and
//! render phases, from ooxml-common's `fonts.office`, `fonts.eot` and `fonts.mtx`.
#![forbid(unsafe_code)]

mod be;
pub mod catalogue;
mod cmap;
pub mod embedded;
mod error;
mod kern;
pub mod names;
pub mod recorded;
pub mod scan;
pub mod sfnt;
pub mod store;
pub mod substitutes;
pub mod word;

pub use catalogue::{Catalogue, FaceLoc, Folder, Location, WORD_PREFER_BUNDLE};
pub use embedded::{deobfuscate, embedded_face, EmbeddedPart};
pub use error::FontError;
pub use recorded::{recorded_face, RecordedFace};
pub use sfnt::{face_offsets, Decorations, Face, FaceMetrics};
pub use store::{FileStamp, FontFile, FontStore};
pub use substitutes::{FaceClass, FontTable, FontTableEntry, SubstitutionKind};
pub use word::{GlyphAdvance, Grade, LayoutFace, Resolution, Substitution, WordFontOptions, WordFonts};
