//! Owns: the DrawingML differential's Rust side (phase P3, lane P3a): the source model
//! ([`model`], [`tree`]) and resolved values ([`resolved`]) as JSON, the walk over a
//! package's parts ([`package`]), and the preset and colour-swatch checks ([`presets`]), in
//! the form `python/drawingml_diff.py` writes ooxml-common's answers in.
//!
//! Must not: be used by any library; it is a test instrument.

pub mod model;
pub mod package;
pub mod presets;
pub mod resolved;
pub mod tree;
