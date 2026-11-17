//! Word's tools (docs/architecture/README.md, section 3 `ooxml-tools-*`, section 9; layer 4).
//!
//! Owns: each Word tool's definition (a `#[derive(ToolArgs)]` struct with its description
//! as a Markdown file beside it) and its handler, a function of the typed arguments and a
//! version of the document. So far one read-only tool, [`describe`], over package parts
//! (lane P0d), and the reading of a Word package that it needs ([`package`]): the main
//! part, its story parts and its settings as docx-agent finds them.
//!
//! Must not: hold sessions, documents by id, locks or dispatch (`ooxml-session` does, and
//! hands a handler the version it reads), know providers, or keep state between calls.
//! Every handler is a pure function of its inputs and checks the [`ooxml_base::Cancel`]
//! port in its loops.
//!
//! The package reading here moves into `ooxml-word` (layer 3) when that crate exists (phase
//! P1); it follows docx-agent's rules, which in places differ from the layout model's
//! (`ooxml-word-model`, docx2svg's rules).
#![forbid(unsafe_code)]

mod describe;
pub mod package;

pub use describe::{describe, Describe, DESCRIBED, NOT_DESCRIBED};

/// The document kind these tools serve.
pub const KIND: &str = "docx";
