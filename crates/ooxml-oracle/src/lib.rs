//! Test support (docs/architecture/README.md, section 3: `ooxml-oracle`, the differential
//! harness): what checks the Rust core against the Python packages it replaces.
//!
//! Owns: the text engine's differential ([`text`]: docx2svg's line breaker's records in,
//! every difference out), and the adapters it runs the engine through -- the file-system
//! [`FontStore`](ooxml_fonts::FontStore) ([`disk::DiskFonts`]) and the fonts layer as the
//! text engine's [`Glyphs`](ooxml_text::Glyphs) port ([`glyphs::WordGlyphs`]), and recorded answers as that port ([`replay::ReplayGlyphs`]);
//! the Word model's differential -- the model of a document as JSON, field by field in the
//! form `python/word_model_diff.py` writes docx2svg's model in ([`export`]) -- and the
//! model's open and reopen benchmark ([`bench`](mod@bench)); the Word layout's differential
//! ([`layout`]: docx2svg's layout records in, every difference out) and its benchmark
//! ([`layout_bench`]); and the DrawingML differential's Rust side ([`dml`]:
//! `python/drawingml_diff.py` compares it with ooxml-common).
//!
//! Must not: be depended on by a shipped crate. The two adapters move to the layout and
//! session crates (L3, L4) when those exist.
#![forbid(unsafe_code)]

pub mod bench;
pub mod disk;
pub mod dml;
pub mod export;
mod export_parts;
pub mod glyphs;
pub mod layout;
pub mod layout_bench;
pub mod metafile;
pub mod records;
pub mod replay;
pub mod text;
