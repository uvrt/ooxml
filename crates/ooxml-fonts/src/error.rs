//! Owns: [`FontError`], why a font could not be read or found.
//!
//! Must not: carry another crate's errors beyond what a port reports.

/// Why a face could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FontError {
    /// The bytes are not a font this reader understands (too short, a table missing).
    #[error("{source_name}: {message}")]
    Malformed {
        /// Where the bytes came from (a path, `embedded:<family>:<style>`).
        source_name: String,
        /// What is wrong.
        message: String,
    },
    /// The font store (the port the faces are read through) failed.
    #[error("font store: {0}")]
    Store(String),
}

impl FontError {
    pub(crate) fn malformed(source_name: &str, message: impl Into<String>) -> Self {
        FontError::Malformed {
            source_name: source_name.to_owned(),
            message: message.into(),
        }
    }
}
