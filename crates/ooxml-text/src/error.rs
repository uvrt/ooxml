//! Owns: [`TextError`], why a paragraph's text cannot be measured.
//!
//! Must not: carry another crate's errors.

/// Why the engine refused a paragraph.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    /// A piece has no known width: a face or character without an advance, a drawing not
    /// laid out, a script size the face does not give (docx2svg's `Unmeasurable`).
    #[error("unmeasurable: {0}")]
    Unmeasurable(String),
}
