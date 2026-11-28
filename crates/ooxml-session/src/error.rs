//! Owns: [`SessionError`], what the session's own operations (open, save, close, refresh)
//! report to their host, and its mapping to a tool's [`ToolError`] where a call meets it.
//!
//! Must not: lose which document or part an error is about.

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
    /// A document's store refused it (any format's).
    #[error("{0}")]
    Store(Box<dyn std::error::Error + Send + Sync>),
    /// The document's format could not read or change it (a Word document that cannot be
    /// read or edited: the format's own error, which its code reads back by downcast).
    #[error("{0}")]
    Format(Box<dyn std::error::Error + Send + Sync>),
    /// A caller's own latest change is not the document's latest: others changed it since.
    #[error("the change that made state {step} is not the latest: {} later change(s)", later.len())]
    Entangled {
        /// The state number the caller's change made.
        step: u64,
        /// The later steps, oldest first: the state number each made.
        later: Vec<u64>,
    },
    /// A caller's own latest undo is not the next redo brings back: another caller undid a
    /// change since.
    #[error("the undone change that made state {step} is not the next to redo")]
    RedoNotNext {
        /// The state number the undone change made.
        step: u64,
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
    /// The state given to open a document with was refused: its format keeps none
    /// (`reason` `None`), or it is not the state of these bytes (the format's words).
    #[error("{}", reason.clone().unwrap_or_else(|| format!("{} documents keep no state beside their bytes", kind.as_str())))]
    StateRefused {
        /// The document's kind.
        kind: crate::DocKind,
        /// Why, where the format keeps a state and refused this one.
        reason: Option<String>,
    },
    /// An id the application gave a document to open is open already.
    #[error("doc_id {0:?} is in use")]
    IdTaken(String),
    /// Another editor's version ([`crate::Session::refresh`]) is in the way: it changed what
    /// an edit or batch prepared before it read (not run again over it), or what an undo or
    /// redo would take back.
    #[error("{editor} changed {} (version {version})", changed.join(", "))]
    ChangedMeanwhile {
        /// An edit's or an undo's.
        meanwhile: ooxml_tools::Meanwhile,
        /// Who.
        editor: String,
        /// The document's version their change made.
        version: u64,
        /// What it changed that is in the way: block ids, part names.
        changed: Vec<String>,
    },
    /// Another editor's version was asked to come in on a base this session does not hold:
    /// a token of another session's, one past the head, or a version no longer kept with
    /// every step after it.
    #[error("{0}")]
    RefreshBase(String),
    /// The document's format takes no other editor's versions.
    #[error("another editor's version of a {} document", .0.as_str())]
    NoRefresh(crate::DocKind),
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
            SessionError::ChangedMeanwhile {
                meanwhile,
                editor,
                version,
                changed,
            } => ToolError::changed_meanwhile(*meanwhile, editor, *version, changed),
            SessionError::RefreshBase(message) => ToolError::refusal(
                Refusal::NotFound,
                message.clone(),
                WayOut::one(serde_json::Value::from(
                    "reopen the document from the server's file, then refresh from that version",
                )),
            ),
            SessionError::NoRefresh(kind) => ToolError::not_supported(
                &format!("another editor's version of a {} document", kind.as_str()),
                WayOut::one(serde_json::Value::from("docx")),
            ),
            other => ToolError::internal(other.to_string(), "session"),
        }
    }
}

impl SessionError {
    /// A format's error.
    #[must_use]
    pub fn format(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        SessionError::Format(Box::new(error))
    }
}

/// The store's refusal for the model `D`: another editor's change in the way as
/// [`SessionError::ChangedMeanwhile`] (its keys named as `D` names them, `meanwhile` saying
/// what met it), any other as the store's.
pub fn store_error<D: ooxml_store::DocModel>(
    error: ooxml_store::Error<D>,
    meanwhile: ooxml_tools::Meanwhile,
) -> SessionError
where
    D::Key: Send + Sync + 'static,
    D::Error: Send + Sync + 'static,
{
    match error {
        StoreError::ForeignChange { step, editor, overlap } => SessionError::ChangedMeanwhile {
            meanwhile,
            editor: editor.to_string(),
            version: step.0,
            changed: overlap
                .keys()
                .map_or_else(Vec::new, |keys| keys.iter().filter_map(D::key_name).collect()),
        },
        StoreError::NotKept { seq } => SessionError::RefreshBase(format!(
            "version {seq} is not one this session keeps with every step after it"
        )),
        other => SessionError::from(other),
    }
}

/// Any model's store refusal: a format's store errors convert with `?` too.
impl<K, E> From<StoreError<K, E>> for SessionError
where
    K: Ord + std::fmt::Debug + Send + Sync + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(error: StoreError<K, E>) -> Self {
        SessionError::Store(Box::new(error))
    }
}
