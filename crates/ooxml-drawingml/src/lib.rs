//! DrawingML's model and resolution (docs/architecture/README.md, section 3
//! `ooxml-drawingml`; phase P3, lane P3a), equal to ooxml-common's.
//!
//! Owns:
//! - the source model: fills, outlines, effects, transforms, geometry and the format
//!   scheme ([`source`]); text bodies ([`text`]); shape trees ([`tree`]);
//! - reading them from a part's tree through `ooxml_schema::a`'s views ([`read`]);
//! - the guide-formula language ([`guides`]) and the preset geometries ([`presets`]);
//! - resolution: a theme and colour maps to a colour context, and fills, outlines and
//!   effects -- with a style reference's `phClr` -- to concrete colours ([`resolve`]);
//! - the drawable elements a chart is lowered to ([`drawable`]).
//!
//! Must not: write SVG or paint (phase P3c: `ooxml-render`), lay text out, or resolve a
//! slide's or a document's own inheritance (`p:ph` shapes, list styles through a master: the
//! format crates'). Every value is immutable and `Send + Sync`; nothing here depends on an
//! interpreter lock.
#![forbid(unsafe_code)]

pub mod drawable;
pub mod guides;
pub mod presets;
mod pyfmt;
pub mod read;
pub mod resolve;
pub mod source;
pub mod text;
pub mod tree;

pub use pyfmt::coordinate;
