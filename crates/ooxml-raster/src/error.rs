//! Owns: why a drawing cannot be rasterised ([`RasterError`]).
//!
//! Must not: carry anything but the reason (no document, no pixels).

/// Why an SVG could not be drawn.
#[derive(Debug, thiserror::Error)]
pub enum RasterError {
    /// The document is not well-formed XML.
    #[error("the SVG is not well-formed: {0}")]
    Xml(String),
    /// usvg refused the document (no `svg` root, a size it cannot read).
    #[error("usvg could not read the SVG: {0}")]
    Svg(String),
    /// A size of zero, or one the fit cannot be computed for.
    #[error("no size to draw at: {0}")]
    Size(&'static str),
    /// The PNG could not be encoded.
    #[error("the PNG could not be encoded: {0}")]
    Png(String),
}
