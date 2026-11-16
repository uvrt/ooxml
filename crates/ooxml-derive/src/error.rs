//! Owns: the derived-data errors.
//!
//! Must not: carry a format's names.

/// Why a derived structure refused an operation. Nothing changed when it is returned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeriveError {
    /// A position past the end of the sequence.
    #[error("position {at} is out of range for a sequence of {len}")]
    OutOfRange {
        /// The position asked for.
        at: usize,
        /// The sequence's length.
        len: usize,
    },
}
