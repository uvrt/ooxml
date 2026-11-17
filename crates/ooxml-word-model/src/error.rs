//! Owns: the crate's error type.
//!
//! Must not: carry a document's content beyond the part name.

use ooxml_opc::OpcError;

/// Why a document could not be read.
#[derive(Debug, thiserror::Error)]
pub enum WordError {
    /// The package could not be read.
    #[error(transparent)]
    Package(#[from] OpcError),
    /// The main document part is missing.
    #[error("the package has no main document part ({0})")]
    NoMainPart(String),
}
