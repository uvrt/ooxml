//! Test support (docs/architecture/README.md, section 3: `ooxml-oracle`, the differential
//! harness): what checks the Rust core against the Python packages it replaces.
//!
//! Owns: the adapters the harnesses run the engines through -- the file-system
//! [`FontStore`](ooxml_fonts::FontStore) ([`disk::DiskFonts`]), the fonts layer as the text
//! engine's [`Glyphs`](ooxml_text::Glyphs) port ([`glyphs::WordGlyphs`]) and a fonts record
//! read as Word's fonts ([`fonts_record`]); the Word model as JSON ([`export`]) and the
//! model's open and reopen benchmark ([`bench`](mod@bench)); the Word layout's benchmark
//! ([`layout_bench`]); the DrawingML differential's Rust side ([`dml`]:
//! `python/drawingml_diff.py` compares it with ooxml-common); and the charts' ([`chart`]:
//! `python/chart_diff.py`); the Word reads' ([`ids`]: block ids and the id ledger;
//! `python/word_reads_diff.py` compares them with docx-agent's); and the PNG differential's
//! ([`raster`]: `python/raster_record.py`'s calls to resvg-py drawn again by
//! `ooxml-raster`). The differentials against docx2svg's Python layout, text engine, model
//! and painter were retired with it (lane svg-thin).
//!
//! Must not: be depended on by a shipped crate. The two adapters move to the layout and
//! session crates (L3, L4) when those exist.
#![forbid(unsafe_code)]

pub mod bench;
mod case_values;
pub mod chart;
pub mod disk;
pub mod dml;
pub mod drawing;
pub mod edits;
pub mod edits_bench;
pub mod edits_markdown;
pub mod edits_styled;
pub mod export;
mod export_parts;
pub mod facts;
pub mod find_regex;
pub mod fonts_record;
pub mod frozen;
pub mod frozen_numbers;
pub mod glyphs;
pub mod host_replay;
pub mod ids;
pub mod laid_facts;
pub mod layout_bench;
pub mod markdown;
pub mod metafile;
pub mod object_bench;
mod panics;
mod part_texts;
pub mod pixels;
pub mod raster;
pub mod reads;
pub mod reads_bench;
pub mod review_bench;
pub mod review_tools;
pub mod scene_data;
pub mod structure_tools;
