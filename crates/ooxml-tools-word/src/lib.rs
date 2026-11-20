//! Word's tools (docs/architecture/README.md, section 3 `ooxml-tools-*`, section 9; layer 4).
//!
//! Owns: each Word tool's definition (a `#[derive(ToolArgs)]` struct with its description
//! as a Markdown file beside it) and its handler: a reading tool's, a function of the
//! typed arguments and a version of the document; an editing tool's, of the typed
//! arguments and the document's editor ([`Editing`]).
//!
//! Must not: hold sessions, documents by id, locks or dispatch (`ooxml-session` does, and
//! hands a handler the version it reads), know providers, or keep state between calls.
//! Every handler is a pure function of its inputs and checks the [`ooxml_base::Cancel`]
//! port in its loops.
#![forbid(unsafe_code)]

mod blobs;
mod describe;
mod find_text;
mod objects;
mod originals;
mod pages;
mod paging;
mod placements;
mod refusal;
mod review;
mod structure;
mod style;
mod table_read;
mod text;
mod version;
mod word_inspect;
mod word_read;
mod written;

pub use blobs::{Blob, BlobList, Blobs};
pub use describe::{describe, Describe, DESCRIBED, NOT_DESCRIBED};
pub use find_text::{find_text, FindStories, FindText};
pub use pages::EnginePages;
pub use placements::Placer;
// A tool group's module exports its tools (`pub use` in its `mod.rs`); the crate exports
// each group whole, so a new tool in a group is named only there.
pub use objects::*;
pub use review::*;
pub use structure::*;
pub use style::*;
pub use text::*;
pub use version::{Others, Version};
pub use word_inspect::{word_inspect, WordInspect};
pub use word_read::{word_read, Story, View, WordRead};
pub use written::Written;

/// What `unported` says of a reply holding a chart's or SmartArt diagram's summary or JSON,
/// which docx-agent reads from the chart model (phase P3).
pub const GRAPHICS: &str = "chart or SmartArt summary";

/// What `unported` says of a final view where a cell's end is one Accept All takes away.
pub const CELL_ENDS: &str = "Accept All's cell ends";

/// The document kind these tools serve.
pub const KIND: &str = "docx";
