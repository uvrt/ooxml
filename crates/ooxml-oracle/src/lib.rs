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
//! ([`layout`]: docx2svg's layout records in, every difference out; tables in
//! [`layout_tables`]) and its benchmark ([`layout_bench`]); the DrawingML differential's
//! Rust side ([`dml`]: `python/drawingml_diff.py` compares it with ooxml-common); and the
//! charts' ([`chart`]: `python/chart_diff.py`); the Word reads' ([`ids`]: block ids and
//! the id ledger; `python/word_reads_diff.py` compares them with docx-agent's); the PNG
//! differential's ([`raster`]: `python/raster_record.py`'s calls to resvg-py drawn again by
//! `ooxml-raster`); and the painter pieces' ([`painter`]: `python/painter_record.py`'s calls
//! from docx2svg).
//!
//! Must not: be depended on by a shipped crate. The two adapters move to the layout and
//! session crates (L3, L4) when those exist.
#![forbid(unsafe_code)]

pub mod bench;
pub mod chart;
pub mod disk;
pub mod dml;
pub mod drawing;
pub mod edits;
pub mod edits_bench;
pub mod export;
mod export_parts;
pub mod glyphs;
pub mod ids;
pub mod layout;
pub mod layout_bench;
pub mod layout_svg;
pub mod layout_tables;
pub mod layout_updates;
pub mod metafile;
pub mod painter;
pub mod pixels;
pub mod raster;
pub mod reads;
pub mod reads_bench;
pub mod records;
pub mod replay;
pub mod review_bench;
pub mod review_tools;
pub mod scene_data;
pub mod text;
