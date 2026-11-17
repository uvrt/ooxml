//! Word's layout (docs/architecture/README.md, sections 3 and 8; layer 3; phase PL, lane
//! L3a): from a version of the Word model (`ooxml-word-model`) and the text engine
//! (`ooxml-text`) to pages of placed lines, exact -- docx2svg's `lines`, `vertical`,
//! `paginate` and the layout's field passes.
//!
//! Owns:
//! - the faces port and Word's fonts adapted to it ([`faces`]);
//! - exact device px ([`px`]); Word's vertical model of a line ([`vertical`]) and a
//!   paragraph's lines' heights ([`heights`]);
//! - what the text engine is given for a paragraph ([`input`]), and a paragraph measured
//!   ([`paragraph`]);
//! - sections, columns and page edges ([`sections`]); headers' and footers' heights
//!   ([`stories`]);
//! - the body's flow ([`flow`]) and its pagination ([`paginate`]); its page-number
//!   fields settled against its pages ([`fields`]);
//! - tables: the extension point the paginator lays them out through ([`table`]) and
//!   Word's tables laid out ([`tables`], lane L3b);
//! - the extension points of what later lanes lay out: stories' contents, notes and
//!   floating drawings (lane L3c);
//! - relayout from the version before ([`incremental`]), and the engine that keeps
//!   measures between versions ([`engine`]).
//!
//! Must not: edit a document, draw (SVG and paint are lane L3d's), read XML, or hold
//! global state. Nothing here relies on an interpreter lock: the engine is owned by one
//! task at a time and its values are `Send + Sync`.
#![forbid(unsafe_code)]

pub mod engine;
mod error;
pub mod faces;
pub mod fields;
pub mod flow;
pub mod heights;
pub mod incremental;
pub mod input;
pub mod paginate;
pub mod paragraph;
pub mod px;
pub mod sections;
pub mod stories;
pub mod table;
pub mod tables;
pub mod vertical;

pub use engine::{DocumentLayout, WordLayout};
pub use error::LayoutError;
pub use faces::{Coverage, Faces, WordGlyphs};
pub use px::Px;
