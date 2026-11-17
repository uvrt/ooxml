//! Test support (docs/architecture/README.md, section 3: `ooxml-oracle`, the differential
//! harness): what checks the Rust core against the Python packages it replaces.
//!
//! Owns: the text engine's differential ([`text`]: docx2svg's line breaker's records in,
//! every difference out), and the adapters it runs the engine through -- the file-system
//! [`FontStore`](ooxml_fonts::FontStore) ([`disk::DiskFonts`]) and the fonts layer as the
//! text engine's [`Glyphs`](ooxml_text::Glyphs) port ([`glyphs::WordGlyphs`]), and recorded answers as that port ([`replay::ReplayGlyphs`]).
//!
//! Must not: be depended on by a shipped crate. The two adapters move to the layout and
//! session crates (L3, L4) when those exist.
#![forbid(unsafe_code)]

pub mod disk;
pub mod glyphs;
pub mod records;
pub mod replay;
pub mod text;
