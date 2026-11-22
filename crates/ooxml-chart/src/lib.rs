//! Charts (docs/architecture/README.md, section 3 `ooxml-chart`; phase P3, lane P3b2),
//! equal to ooxml-common's `chart/` package.
//!
//! Owns:
//! - the chart model, a `c:chartSpace` as data plus styling ([`model`]), and its reader
//!   ([`read`]);
//! - a chart's data and labels as an editor shows them, and their one-line summary
//!   ([`data`]), and what `edit_chart`'s `read` gives of its look (`data::look`,
//!   `data::labels`);
//! - a chart's data changed in its caches, point by point, for `edit_chart`'s data actions
//!   ([`edit`]);
//! - where Word and PowerPoint differ ([`rules`]);
//! - the layout, lowered to drawable elements ([`layout`]);
//! - Python's number writing and arithmetic where the layout's text, paths and positions
//!   depend on its digits ([`pyfmt`], [`pymath`]).
//!
//! Must not: write SVG, resolve a theme (the consumer supplies colours and faces), or know
//! which document format holds the chart. Every value is immutable and `Send + Sync`;
//! nothing here depends on an interpreter lock.
#![forbid(unsafe_code)]

pub mod data;
pub mod edit;
pub mod layout;
pub mod model;
pub mod pyfmt;
pub mod pymath;
pub mod read;
pub mod rules;

pub use read::chart_space;
