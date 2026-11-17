//! Owns: [`ZipError`].
//! Must not: carry other crates' errors.

use thiserror::Error;

/// What went wrong reading or writing an archive.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ZipError {
    /// The bytes are not a ZIP archive as the specification lays one out.
    #[error("not a valid ZIP archive: {0}")]
    Malformed(String),
    /// A limit on untrusted input (design section 11) was exceeded.
    #[error("limit exceeded: {what} (limit {limit})")]
    Limit {
        /// Which limit.
        what: &'static str,
        /// Its value.
        limit: u64,
    },
    /// An entry's data does not inflate to its stated size and CRC-32.
    #[error("entry {name:?} is corrupt: {message}")]
    Corrupt {
        /// The entry.
        name: String,
        /// What is wrong.
        message: String,
    },
    /// A ZIP feature this crate does not read or write (encryption, a method other than
    /// stored and deflate, an archive that needs ZIP64 to be written).
    #[error("not supported: {0}")]
    Unsupported(String),
}
