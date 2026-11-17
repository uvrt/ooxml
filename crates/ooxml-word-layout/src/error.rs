//! Owns: the crate's error type.
//!
//! Must not: carry a document's content beyond a block's place in it.

/// Why a document could not be laid out at all (a block the layout cannot lay out is not
/// an error: the layout stops there and says so in its pages).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// The document has no section.
    #[error("the document has no section")]
    NoSection,
}
