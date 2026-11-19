//! Owns: [`EditError`], why a Word edit refused or could not be made (design section 10:
//! the tool layer maps it to a `ToolError` with its code and way out).
//!
//! Must not: carry anything but what the refusal says.

use crate::doc::{Key, ModelError};
use ooxml_opc::OpcError;
use ooxml_store::StoreError;
use ooxml_word_model::address::AddressError;
use ooxml_word_model::WordError;

/// Why an edit refused or failed. Nothing changed when it is returned.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    /// Text a paragraph cannot hold (`\n`, a new object).
    #[error("{0}")]
    Text(String),
    /// A span that cuts a field or would orphan a note.
    #[error("{0}")]
    Span(String),
    /// An address that names nothing, or something of another kind.
    #[error("{0}")]
    Address(AddressError),
    /// The target is not what the edit takes.
    #[error("{0}")]
    Refused(String),
    /// An argument the edit cannot take as given, at the field it names (docx-agent's
    /// `invalid_arguments` there).
    #[error("{message}")]
    Argument {
        /// The argument, as the tool names it (`cells[].row or row_label`).
        field: String,
        /// What is wrong, and what to give.
        message: String,
        /// The values that would do, where there is a set of them (none: the call itself
        /// is the way out).
        options: Vec<String>,
    },
    /// A style edit refused, with the field it names and the options that would do.
    #[error("{0}")]
    Style(crate::styles::StyleRefusal),
    /// What the edit would have to do is not ported yet.
    #[error("not supported yet: {0}")]
    Unsupported(String),
    /// A part the edit needs is missing.
    #[error("no part {0}")]
    NoPart(String),
    /// A place in a part that holds no element.
    #[error("no element at the place found in {0}")]
    Path(String),
    /// The document could not be read as a Word document.
    #[error(transparent)]
    Word(#[from] WordError),
    /// The package could not be read.
    #[error(transparent)]
    Package(#[from] OpcError),
    /// The store refused a change.
    #[error(transparent)]
    Store(#[from] StoreError<Key, ModelError>),
    /// A change could not be made on the version.
    #[error(transparent)]
    Model(#[from] ModelError),
}

/// A result of this crate.
pub type Result<T> = std::result::Result<T, EditError>;
