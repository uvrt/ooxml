//! Owns: a document's changes feed as the session hands it on (phase 3 changes-feed design,
//! section 2), format-neutral: what a call asks ([`FeedAsk`]), what a format answers
//! ([`FeedAnswer`]: the changes in their wire shape, and the pictures new since `since` by
//! content address with their bytes), and [`Session::feed`], which asks the document's
//! format and says where its format keeps no feed ([`FeedError::NoFeed`]: pptx and xlsx
//! until Phase 4).
//!
//! Must not: lay out or paint (the format does), or know a format's records.

use crate::{CallContext, DocId, DocKind, Session, SessionError};
use ooxml_base::Context;
use ooxml_tools::ToolError;
use serde_json::{Map, Value};
use std::sync::Arc;

/// What a changes-feed call asks of a document.
pub struct FeedAsk<'a> {
    /// The document.
    pub doc: &'a DocId,
    /// The token of the answer the caller has (`<session nonce>-<seq>`); `None`: everything.
    pub since: Option<&'a str>,
    /// Whether the pages are asked too (`false`: the version, parts and blocks only, never
    /// laid out).
    pub pages: bool,
    /// The session's nonce: a `since` of another session's is stale.
    pub nonce: &'a str,
    /// The session's ports (its clock and cancellation, for the limit on laying out).
    pub context: &'a Context,
    /// The call's context (its limit on laying out).
    pub call: &'a CallContext,
}

/// A picture the feed draws by content address, with its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedBytes {
    /// Its address (`href="picture:<hash>"`).
    pub hash: String,
    /// Its media type.
    pub mime: String,
    /// Its bytes.
    pub bytes: Arc<[u8]>,
}

/// A changes-feed answer.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedAnswer {
    /// The changes in their wire shape: `version`, `state`, `session`, `token`, `since`,
    /// `full`, `behind`, `parts`, `blocks`, and with pages `pages`.
    pub changes: Map<String, Value>,
    /// The pictures the answer's groups draw that were not drawn at `since`.
    pub pictures: Vec<FeedBytes>,
}

/// Why a changes-feed call was not answered.
#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    /// The session's own: the document is not open.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// The document's format keeps no feed.
    #[error("the changes feed does not draw {0} documents yet")]
    NoFeed(DocKind),
    /// The format's refusal or failure (a version it does not lay out, past the limit).
    #[error("{0}")]
    Format(ToolError),
}

impl Session {
    /// What changed in the document `id` after `since` ([`FeedAsk`] says what is asked),
    /// as its format's feed answers it.
    ///
    /// # Errors
    /// [`FeedError::Session`] for a document not open, [`FeedError::NoFeed`] where its
    /// format keeps none, [`FeedError::Format`] for the format's refusal.
    pub fn feed(
        &self,
        id: &DocId,
        since: Option<&str>,
        pages: bool,
        call: &CallContext,
    ) -> Result<FeedAnswer, FeedError> {
        let document = self.document(id)?;
        let ask = FeedAsk {
            doc: id,
            since,
            pages,
            nonce: self.nonce(),
            context: self.context(),
            call,
        };
        match document.doc.changes(&ask) {
            Some(answer) => answer.map_err(FeedError::Format),
            None => Err(FeedError::NoFeed(document.kind)),
        }
    }
}
