//! The shared text engine (docs/architecture/README.md, sections 3 and 8; layer 1).
//!
//! Owns: Word's advances in 1/4096 pt with the legacy kern charged to the left glyph
//! ([`mod@pieces`]), break opportunities and the one break loop with Word's compatibility
//! rules as data ([`breaks`], [`geometry`]), hyphenation ([`hyphenate`], automatic
//! hyphens), and measures memoised by content ([`Measurer`]). A port of docx2svg's
//! `linebreak`, `measure` and `hyphenate`, equal to them on every corpus paragraph (the
//! differential, `ooxml-oracle`'s `text-diff`).
//!
//! Must not: know about paragraphs' properties or styles -- its input is runs of text with
//! resolved character formats ([`input`]) and a geometry -- or find faces: it reads them
//! through the [`Glyphs`] port, which the layout adapts the fonts layer (`ooxml-fonts`) to.
//! It holds no global mutable state (the hyphenation tables are parsed once from data
//! compiled in) and relies on no interpreter lock.
//!
//! Not here yet: DrawingML's rule set (GPOS kerning, grid rounding, PowerPoint's breaks):
//! it comes with the slides phase (P3/P5), as a second rule set of the same loop.
#![forbid(unsafe_code)]

mod auto;
pub mod breaks;
mod error;
pub mod geometry;
pub mod glyphs;
pub mod hyphenate;
pub mod input;
pub mod measure;
pub mod pieces;
pub mod units;

pub use breaks::{break_from, break_line, break_pieces, Line};
pub use error::TextError;
pub use geometry::{next_tab, Geometry, TabAlign, WordRules};
pub use glyphs::{Advance, Glyphs, ScriptMetrics};
pub use input::{
    BreakKind, CharFormat, FaceKey, Hyphenation, Label, LabelAlign, LabelBody, LabelSuffix, Mark, Paragraph, Run,
    RunFormats, VerticalAlign,
};
pub use measure::{measure, pieces, MeasureKey, Measured, Measurer};
pub use pieces::{Piece, PieceKind};
pub use units::{twips, twips_to_units, Units};
