//! Word's read-side model for layout (docs/architecture/README.md, section 3 `ooxml-word`,
//! section 8 "resolve"; phase PL step 2): a version of a document read from its parts as
//! docx2svg reads it, with what layout needs resolved.
//!
//! Owns:
//! - generated `w:` views ([`w`], from `schema/w.views` by `cargo xtask codegen`);
//! - declared properties per level ([`props`], [`value`]), the styles part ([`styles`]),
//!   numbering ([`numbering`]), settings ([`settings`]) and the theme ([`theme`]);
//! - the source model ([`model`]) read in the final view of tracked changes, with fields
//!   threaded through each part ([`read`]);
//! - the styles cascade, tables' resolved formats and faces per character slot
//!   ([`resolve`]);
//! - list labels counted as Word counts them, kept incrementally ([`labels`]);
//! - opening versions of one document, reusing what is unchanged ([`Model`],
//!   [`WordDocument`]).
//!
//! Must not: lay out pages or compute fields' values (`ooxml-word-layout`, L3), edit, or
//! read another library's internals. Nothing here depends on an interpreter lock: every
//! value is immutable and `Send + Sync`; a [`Model`] is owned by one task at a time.
//!
//! The package is named `ooxml-word-model` while the prototype `crates/word` holds the name
//! `ooxml-word` (its `word-check` binary is built by name in CI); it takes the design's name
//! when the prototype is retired.
#![forbid(unsafe_code)]

mod digest;
pub mod document;
mod error;
mod imagemeta;
pub mod labels;
pub mod model;
pub mod numbering;
mod open;
pub mod props;
pub mod read;
pub mod resolve;
pub mod settings;
pub mod styles;
pub mod theme;
pub mod value;
#[rustfmt::skip]
pub mod w;
pub mod xmlread;

pub use document::{Context, Notes, ResolvedParagraph, WordDocument};
pub use error::WordError;
pub use open::{Model, OpenStats};
