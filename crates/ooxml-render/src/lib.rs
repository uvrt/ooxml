//! The DrawingML painter (docs/architecture/README.md, sections 3 and 8; layer 2): drawable
//! elements -- a chart's, a slide's, a document's -- lowered onto the scene's display list
//! ([`ooxml_scene::markup`]) under one application's drawing rules, equal to ooxml-common's
//! `drawingml.elements.render_element` element for element.
//!
//! Owns: the painting pass ([`context`]), the rules as data ([`rules`]), the text measure
//! port and its table measure ([`measure`]), geometry ([`geometry`]) and as path data
//! ([`paths`]), fills and outlines ([`fill`]), Word's drawn arrowheads ([`arrowhead`]),
//! effects ([`effect`]), text bodies ([`text`]), shapes, connectors, pictures and tables
//! ([`shape`]), and elements and groups ([`element`]).
//!
//! Must not: parse SVG, read a part, resolve a colour or a style (the model crates do), or
//! hold state between passes. Every value is `Send + Sync`; nothing depends on an
//! interpreter lock.
#![forbid(unsafe_code)]

pub mod arrowhead;
pub mod context;
pub mod effect;
pub mod element;
pub mod fill;
pub mod geometry;
pub mod image;
pub mod measure;
pub mod paths;
pub mod rules;
pub mod shape;
pub mod text;
pub mod units;

pub use context::{table_measurer, Painter};
pub use element::{render_element, render_group};
pub use measure::{Faces, TableMeasurer, TextMeasurer};
pub use rules::{DrawingRules, POWERPOINT, WORD};
