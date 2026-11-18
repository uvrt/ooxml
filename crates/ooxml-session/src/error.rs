//! Owns: [`SessionError`], what the session's own operations (open, save, close, update)
//! report to their host, and its mapping to a tool's [`ToolError`] where a call meets it.
//!
//! Must not: lose which document or part an error is about.

use crate::model::ModelError;
use ooxml_base::BaseError;
use ooxml_opc::OpcError;
use ooxml_store::StoreError;
use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};

/// Why a session operation did nothing.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// No open document has the id.
    #[error("no open document {id:?}; open: {open:?}")]
    NotOpen {
        /// The id asked for.
        id: String,
        /// The ids open.
        open: Vec<String>,
    },
    /// A kind no session opens.
    #[error("no document kind {0:?}; one of docx, pptx, xlsx")]
    Kind(String),
    /// A part name that names no part.
    #[error("{0:?} is not a part name")]
    PartName(String),
    /// The package refused it.
    #[error(transparent)]
    Package(#[from] OpcError),
    /// The store refused it.
    #[error(transparent)]
    Store(#[from] StoreError<String, ModelError>),
    /// A Word document's store refused it.
    #[error(transparent)]
    WordStore(#[from] StoreError<ooxml_word_edit::Key, ooxml_word_edit::ModelError>),
    /// A Word document could not be read or edited.
    #[error(transparent)]
    Word(#[from] ooxml_word_edit::EditError),
    /// A caller's own latest change is not the document's latest: others changed it since.
    #[error("the change that made version {step} is not the latest: {} later change(s)", later.len())]
    Entangled {
        /// The version the caller's change made.
        step: u64,
        /// The later changes: the version each made and who made it.
        later: Vec<(u64, Option<String>)>,
    },
    /// Saving was refused: the document has validation problems it did not open with.
    #[error("{} new validation problem(s): {}", new.len(), new.join("; "))]
    NewProblems {
        /// The new problems, as text.
        new: Vec<String>,
    },
    /// A port failed, or the work was cancelled.
    #[error(transparent)]
    Base(#[from] BaseError),
}

impl SessionError {
    /// The error as a tool reply carries it.
    #[must_use]
    pub fn to_tool_error(&self) -> ToolError {
        match self {
            SessionError::NotOpen { id, open } => {
                let message = format!("no open document {}", ooxml_tools::pyfmt::repr_str(id));
                let way_out = WayOut::names(open.iter().cloned()).unwrap_or_else(|| {
                    // nothing is open: the way out is opening one
                    let mut arguments = serde_json::Map::new();
                    arguments.insert("blob".into(), "b1".into());
                    WayOut::call(TryCall {
                        tool: "open_document".into(),
                        arguments,
                        fill: vec!["blob".into()],
                    })
                });
                ToolError::refusal(Refusal::NotFound, message, way_out).at("doc")
            }
            SessionError::Base(BaseError::Cancelled) => ToolError::failure(
                Failure::Timeout,
                "the call stopped before it finished (cancelled, or past its deadline); nothing changed",
            ),
            other => ToolError::internal(other.to_string(), "session"),
        }
    }
}
