//! The display list (docs/architecture/README.md, sections 3 and 8; layer 2): what charts and
//! metafiles lower to, and the typed SVG writer that is both today's equality oracle and the
//! future render path's input.
//!
//! Owns: the scene model ([`model`]: paths, rectangles, text runs with positioned characters,
//! images, groups, nested viewports; clip paths, patterns, gradients and masks as resources
//! with ids), its geometry ([`geometry`]: matrices, figures of line and cubic segments), how
//! a number is written ([`num`]), and the SVG writer ([`svg`]); the DrawingML painter's
//! display list ([`markup`]: elements in SVG's vocabulary with typed attribute values in their
//! producer's order) with the numbers it writes ([`drawing_num`]); and the Word layout's page
//! display list with exact lengths and live-view ids, and its writer ([`exact`]).
//!
//! Must not: know any OOXML or metafile record, parse SVG, or hold state between calls.
#![forbid(unsafe_code)]

pub mod base64;
mod drawnum;
pub mod exact;
pub mod geometry;
pub mod markup;
pub mod model;
mod number;
pub mod svg;

pub use drawnum::{drawing_num, factor, fixed2, python_g, written};
pub use geometry::{Figure, Matrix, PathData, Segment};
pub use model::{
    Anchor, Bounds, Cells, Content, Decoration, Def, Fill, Font, Group, GroupTransform, Image, ImageData, LineCap,
    LineJoin, Node, Origin, Paint, Path, Rect, ResourceId, ResourceKind, Rgb, Root, Scene, Stop, StrokeAttr, Text,
    Viewport,
};
pub use number::num;
pub use svg::{write_scene, CharCount, Sink};
