//! Owns: [`EditError`], why a Word edit refused or could not be made (design section 10:
//! the tool layer maps it to a `ToolError` with its code and way out), and [`Internal`], the
//! one class of what is never the caller's fault: the edit core's own model and tree
//! disagreeing (a place that holds no element, a part the model names that the package
//! lacks, an edit copy that does not build, XML the core wrote that does not read back).
//! Refusals are for the caller's input; an [`Internal`] is a bug, answered as an `internal`
//! failure and counted, never worded as a refusal.
//!
//! Must not: carry anything but what the refusal says.

use crate::doc::{Key, ModelError};
use ooxml_chart::edit::GraphicError;
use ooxml_opc::OpcError;
use ooxml_store::StoreError;
use ooxml_word_model::address::AddressError;
use ooxml_word_model::ids::IdStateError;
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
    /// A table position refused as the table is accepted (past it, or in a cell Accept All
    /// removes), at the argument it names.
    #[error("{}", .0.error.message)]
    Position(ooxml_word_model::docx::tables::Misplaced),
    /// What the edit would have to do is not ported yet.
    #[error("not supported yet: {0}")]
    Unsupported(String),
    /// Laying a version out for the edit was stopped: the call's limit on laying out (the
    /// duration, where it has one) passed, or it was cancelled. Nothing changed.
    #[error("laying the document out did not finish{}", .0.map_or_else(String::new, |t| format!(" within {} s", t.as_secs_f64())))]
    LayoutStopped(Option<std::time::Duration>),
    /// The core's model and tree disagree: a bug, not the caller's input.
    #[error("{0}")]
    Internal(Box<Internal>),
    /// The document could not be read as a Word document.
    #[error(transparent)]
    Word(#[from] WordError),
    /// The package could not be read.
    #[error(transparent)]
    Package(#[from] OpcError),
    /// The id state given beside a document's bytes was refused (docx-agent's words).
    #[error(transparent)]
    IdState(#[from] IdStateError),
    /// The store refused a change.
    #[error(transparent)]
    Store(#[from] StoreError<Key, ModelError>),
    /// A change could not be made on the version.
    #[error(transparent)]
    Model(#[from] ModelError),
}

/// What disagreed, in an [`Internal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mismatch {
    /// A place found in a part holds no element there.
    NoElement,
    /// A part the model names is not in the package.
    NoPart,
    /// An edit copy of a part did not build back into a tree.
    Unbuilt,
    /// XML the core made itself did not read back.
    OwnXml,
}

/// The edit core's own model and tree disagreeing: never the caller's fault. Its message is
/// the one the tools have always answered with; `at` is where in the core it was found.
#[derive(Debug, Clone)]
pub struct Internal {
    /// What disagreed.
    pub what: Mismatch,
    /// The part it is about, or (for [`Mismatch::OwnXml`]) what the reader said.
    pub about: String,
    /// The place in the core's source that found it.
    pub at: &'static std::panic::Location<'static>,
}

impl std::fmt::Display for Internal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.what {
            Mismatch::NoElement | Mismatch::Unbuilt => write!(f, "no element at the place found in {}", self.about),
            Mismatch::NoPart => write!(f, "no part {}", self.about),
            Mismatch::OwnXml => f.write_str(&self.about),
        }
    }
}

/// A graphic edit's refusal (`ooxml_chart::edit`'s) as the edit core's, one to one: an
/// argument at no field, a form not supported yet, a part the graphic names that the package
/// lacks, a package that could not be read.
impl From<GraphicError> for EditError {
    #[track_caller]
    fn from(error: GraphicError) -> EditError {
        match error {
            GraphicError::Argument(message) => EditError::Argument {
                field: String::new(),
                message,
                options: Vec::new(),
            },
            GraphicError::Unsupported(what) => EditError::Unsupported(what),
            GraphicError::NoPart(part) => EditError::no_part(part),
            GraphicError::Package(error) => EditError::Package(error),
        }
    }
}

impl EditError {
    #[track_caller]
    fn internal(what: Mismatch, about: impl Into<String>) -> EditError {
        EditError::Internal(Box::new(Internal {
            what,
            about: about.into(),
            at: std::panic::Location::caller(),
        }))
    }

    /// A place found in `part` that holds no element.
    #[track_caller]
    pub fn no_element(part: impl Into<String>) -> EditError {
        EditError::internal(Mismatch::NoElement, part)
    }

    /// A part the model names that the package lacks.
    #[track_caller]
    pub fn no_part(part: impl Into<String>) -> EditError {
        EditError::internal(Mismatch::NoPart, part)
    }

    /// An edit copy of `part` that did not build.
    #[track_caller]
    pub fn unbuilt(part: impl Into<String>) -> EditError {
        EditError::internal(Mismatch::Unbuilt, part)
    }

    /// XML the core made that did not read back, with what the reader said.
    #[track_caller]
    pub fn own_xml(said: impl std::fmt::Display) -> EditError {
        EditError::internal(Mismatch::OwnXml, said.to_string())
    }

    /// The internal mismatch this is, if it is one.
    #[must_use]
    pub fn as_internal(&self) -> Option<&Internal> {
        match self {
            EditError::Internal(internal) => Some(internal),
            _ => None,
        }
    }
}

/// A result of this crate.
pub type Result<T> = std::result::Result<T, EditError>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn an_internal_mismatch_keeps_the_words_the_tools_answered_with_and_its_place() {
        let error = EditError::no_element("word/document.xml");
        assert_eq!(error.to_string(), "no element at the place found in word/document.xml");
        let internal = error.as_internal().unwrap();
        assert_eq!(internal.what, Mismatch::NoElement);
        assert!(internal.at.file().ends_with("error.rs"));
        assert_eq!(
            EditError::unbuilt("word/styles.xml").to_string(),
            "no element at the place found in word/styles.xml"
        );
        assert_eq!(
            EditError::no_part("word/numbering.xml").to_string(),
            "no part word/numbering.xml"
        );
        assert_eq!(EditError::own_xml("unclosed tag").to_string(), "unclosed tag");
        assert!(EditError::Refused("x".into()).as_internal().is_none());
    }
}
