//! A Word document's pages held in Rust (docs/architecture/README.md, sections 6, 8 and 13;
//! layer 3, above the layout; phase PL, lane L4): one layout per version of a document,
//! each laid out again from the version before, read through its facts, its pages' paint
//! and its pages' SVG, made on demand.
//!
//! Owns:
//! - the fonts a document is laid out and drawn with: the machine's, scanned once per
//!   set of folders ([`FontSetup`], over [`DiskFonts`]), with a package's own font table and
//!   embedded faces ([`DocumentFonts`]);
//! - a package's pictures as the SVG writer reads them ([`PackagePictures`]), and the
//!   painter of its drawings ([`drawings::WordDrawings`]): pictures, shapes, groups and
//!   charts, as docx2svg paints them;
//! - what the layout cannot lay out yet, found before a version is laid out
//!   ([`support::unsupported`]): the caller lays such a document out another way
//!   ([`Fallback`]), counted, never silently;
//! - one document's engine ([`PageEngine`]): the Word model's and the layout's memos, kept
//!   between versions, and the last version laid out, which the next is laid out from;
//! - one version laid out ([`LaidVersion`]): its facts (made once, when first asked), its
//!   pages' views (lines with their spans, rules, pictures and drawing slots) and SVG,
//!   painted when asked;
//! - what it holds, in bytes, roughly ([`LaidVersion::weight`]).
//!
//! Must not: edit a document, read anything of the machine but font files, hold global
//! state, or rely on an interpreter lock: an engine is used by one caller at a time
//! (its owner wraps it in a lock), and a laid version is immutable and `Send + Sync`.
#![forbid(unsafe_code)]

mod disk;
pub mod drawings;
mod engine;
mod error;
mod fonts;
mod pictures;
pub mod png;
pub mod support;
mod version;
pub mod warnings;

pub use disk::DiskFonts;
pub use engine::{EngineOptions, EngineStats, PageDigests, PageEngine, RelaidCause};
pub use error::{Fallback, PagesError};
pub use fonts::{embedded_parts, font_table, DocumentFonts, FontSetup};
pub use pictures::PackagePictures;
pub use version::{LaidVersion, PageView};
