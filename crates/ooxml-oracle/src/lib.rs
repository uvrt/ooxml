//! Test support (docs/architecture/README.md, section 3: `ooxml-oracle`): the Rust side of
//! the differential harness -- the Word model of a document as JSON, field by field in the
//! form `tools/word_model_diff.py` writes docx2svg's model in ([`export`]) -- and the
//! model's open and reopen benchmark ([`bench`]).
//!
//! Must not: be a dependency of any shipped crate.
#![forbid(unsafe_code)]

pub mod bench;
pub mod export;
mod export_parts;
