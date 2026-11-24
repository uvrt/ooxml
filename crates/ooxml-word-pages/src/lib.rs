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
//!   ([`support::unsupported`]): the answer is why ([`Fallback`]: a reason and where), which
//!   the tools refuse as not supported yet, with a way out -- never laid out another way;
//! - one document's engine ([`PageEngine`]): the Word model's and the layout's memos, kept
//!   between versions, and the last version laid out, which the next is laid out from;
//! - one version laid out ([`LaidVersion`]): its facts (made once, when first asked), its
//!   pages' views (lines with their spans, rules, pictures and drawing slots) and SVG,
//!   painted when asked;
//! - how much of a document a version covers, and every warning it says, as docx2svg says
//!   them ([`coverage`]);
//! - a version's layout summary ([`summary`]) and the comparison of two versions page by
//!   page ([`PageEngine::compare`], [`summarised`]): only the pages they do not share
//!   painted, each page's digests kept across versions within a bound;
//! - `check`'s layout facts of a version laid out ([`layout_facts`]: the text sizes, the
//!   spacers, the keeps broken at a page's foot, the tables kept together and the tables
//!   parted from their lead-in or the line after them), read with the version's reading;
//! - what a version alone holds, in bytes, roughly ([`LaidVersion::weight`]), and what the
//!   engine keeps within its budgets ([`EngineBudgets`]: its memos, swept once a version is
//!   laid out; its pages' paint, least recently used let go first).
//!
//! Must not: edit a document, read anything of the machine but font files, hold global
//! state, or rely on an interpreter lock: an engine is used by one caller at a time
//! (its owner wraps it in a lock), and a laid version is immutable and `Send + Sync`.
#![forbid(unsafe_code)]

mod causes;
pub mod coverage;
mod digest_cache;
mod digests;
mod disk;
mod drawing_said;
pub mod drawings;
mod engine;
mod error;
mod fonts;
pub mod ids;
pub mod laid_entry;
pub mod layout_facts;
pub mod line_text;
mod page_context;
pub mod page_entry;
pub mod page_parts;
pub mod page_read;
mod page_reads;
pub mod page_said;
mod pictures;
pub mod png;
mod said;
mod stats;
pub mod summarised;
pub mod summary;
pub mod support;
mod text_boxes;
mod version;
pub mod warnings;
mod weight;

pub use causes::RelaidCause;
pub use disk::DiskFonts;
pub use engine::{lock_engine, EngineBudgets, EngineOptions, EngineSeed, Laid, PageDigests, PageEngine, Trial};
pub use error::{Fallback, PagesError};
pub use fonts::{embedded_parts, font_table, DocumentFonts, FontSetup};
pub use pictures::PackagePictures;
pub use stats::EngineStats;
pub use version::{LaidVersion, PageView};
