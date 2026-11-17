//! Charts (docs/architecture/README.md, section 3 `ooxml-chart`; phase P3, lane P3b2),
//! equal to ooxml-common's `chart/` package.
//!
//! Owns:
//! - the chart model, a `c:chartSpace` as data plus styling ([`model`]), and its reader
//!   ([`read`]);
//! - where Word and PowerPoint differ ([`rules`]);
//! - Python's number writing where the layout's text and paths depend on its digits
//!   ([`pyfmt`]).
//!
//! Must not: write SVG, resolve a theme (the consumer supplies colours and faces), or know
//! which document format holds the chart. Every value is immutable and `Send + Sync`;
//! nothing here depends on an interpreter lock.
#![forbid(unsafe_code)]

pub mod model;
pub mod pyfmt;
pub mod read;
pub mod rules;

pub use read::chart_space;
