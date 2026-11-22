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
//!   ([`stories`]), and per page where a `STYLEREF` changes them ([`page_stories`]);
//! - the body's flow ([`flow`]) and its pagination ([`paginate`]); its page-number
//!   fields settled against its pages ([`fields`]);
//! - tables: the extension point the paginator lays them out through ([`table`]) and
//!   Word's tables laid out ([`tables`], lane L3b);
//! - the extension points of what later lanes lay out: stories' contents, notes and
//!   floating drawings (lane L3c);
//! - relayout from the version before ([`incremental`]), and the engine that keeps
//!   measures between versions ([`engine`]).
//!
//! - a text box's text laid out as a story of its own (`geometry::text_box`) and painted in
//!   its drawing (`paint::text_box`), where the drawing's painter finds the box (lane L3g);
//! - a page's geometry: its frame and its lines on whole device pixels ([`geometry`]); the
//!   layout facts a check reads ([`facts`]); the page numbers a field update writes
//!   ([`fields::update`]); a page's paint and its SVG ([`paint`]) (lane L3d).
//!
//! Must not: edit a document, read XML or a part's bytes, or hold global state. Nothing here relies on an interpreter lock: the engine is owned by one
//! task at a time and its values are `Send + Sync`.
#![forbid(unsafe_code)]

pub mod borders;
pub mod engine;
mod error;
pub mod faces;
pub mod facts;
pub mod fields;
pub mod floating;
pub mod flow;
pub mod geometry;
pub mod heights;
pub mod incremental;
pub mod input;
pub mod notes;
pub mod page_stories;
pub mod paginate;
pub mod paint;
pub mod paragraph;
pub mod placeholder;
pub mod px;
pub mod rebreak;
pub mod rooms;
pub mod sections;
pub mod stop;
pub mod stories;
pub mod table;
pub mod tables;
pub mod vertical;
pub mod wrap;

pub use engine::{DocumentLayout, WordLayout};
pub use error::LayoutError;
pub use faces::{Coverage, Faces, WordGlyphs};
pub use px::{CompactPx, Px};
pub use stop::Stop;
