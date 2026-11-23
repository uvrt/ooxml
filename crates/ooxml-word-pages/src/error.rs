//! Owns: what laying out a version can answer instead of a layout -- a package that does
//! not open or a layout stopped ([`PagesError`]), or a document the layout cannot lay out
//! yet ([`Fallback`]), which the tools refuse as not supported yet, with a way out.
//!
//! Must not: decide what the caller does with either.

/// A version that could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum PagesError {
    /// The package's parts.
    #[error("package: {0}")]
    Package(#[from] ooxml_opc::OpcError),
    /// The Word model's reading of them.
    #[error("word: {0}")]
    Word(#[from] ooxml_word_model::WordError),
    /// Laying it out was stopped (cancelled, or past its caller's deadline): nothing was
    /// laid out, the version laid out before is still the one the next is laid out from.
    #[error("laying the version out was stopped")]
    Stopped,
}

/// A document the layout does not lay out yet: the reason, stable and short (the form a
/// tool refuses it in), and where it was found. Nothing lays it out another way.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason} ({at})")]
pub struct Fallback {
    /// Why, one of [`crate::support::REASONS`].
    pub reason: &'static str,
    /// Where: a path, a part, or a setting.
    pub at: String,
}
