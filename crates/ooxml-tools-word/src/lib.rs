//! Word's tools (docs/architecture/README.md, section 3 `ooxml-tools-*`, section 9; layer 4).
//!
//! Owns: each Word tool's definition (a `#[derive(ToolArgs)]` struct with its description
//! as a Markdown file beside it) and its handler, a function of the typed arguments and a
//! version of the document.
//!
//! Must not: hold sessions, documents by id, locks or dispatch (`ooxml-session` does, and
//! hands a handler the version it reads), know providers, or keep state between calls.
//! Every handler is a pure function of its inputs and checks the [`ooxml_base::Cancel`]
//! port in its loops.
#![forbid(unsafe_code)]

mod describe;

pub use describe::{describe, Describe, DESCRIBED, NOT_DESCRIBED};

/// The document kind these tools serve.
pub const KIND: &str = "docx";
