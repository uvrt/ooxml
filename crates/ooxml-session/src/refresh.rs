//! Owns: [`Session::refresh`], another editor's version of an open document coming in
//! (co-authoring refresh, phase 3 CF2): its file, made from a version this session gave (the
//! feed's token, `<session nonce>-<seq>`), committed as one step by that editor, so the
//! changes feed shows it as an ordinary version. This session's steps since that version
//! that touched what it changed are taken back first and named ([`Refreshed::handed_back`]),
//! with who made each, so the host tells those callers; the rest stay. What the format does
//! with the file is its own ([`crate::FormatDoc::refresh`]).
//!
//! Must not: refuse a version because of this session's steps (the server's accepted version
//! is the truth; main's ruling for CF2), or read the file itself.

use crate::format::AnyVersion;
use crate::owners::StepOwner;
use crate::{CallContext, DocId, DocKind, Session, SessionError};
use ooxml_base::Limits;
use serde_json::{Map, Value};
use std::sync::Arc;

/// The tool name another editor's steps are owned by.
pub const REFRESH: &str = "refresh";

/// Another editor's version, as a format is asked to take it in.
#[derive(Debug, Clone)]
pub struct RefreshAsk<'a> {
    /// The document's kind.
    pub kind: DocKind,
    /// The other editor, as its steps are named.
    pub editor: &'a str,
    /// The version of this document their save was made from (the store's seq).
    pub base: u64,
    /// Their file.
    pub file: Arc<[u8]>,
    /// The limits to read it within.
    pub limits: Limits,
}

/// A step of this session's taken back first, as the format reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenBack {
    /// The state number the step made.
    pub number: u64,
    /// The caller that made it (the store's key), if any.
    pub caller: Option<String>,
    /// The blocks it wrote, by id.
    pub blocks: Vec<String>,
    /// The version the undo that took it back made, and its state number.
    pub undo: (u64, u64),
}

/// What a format made of another editor's version.
#[derive(Clone)]
pub struct FormatRefreshed {
    /// The version it made (the head it found, where it changed nothing).
    pub version: AnyVersion,
    /// Whether it changed the document.
    pub changed: bool,
    /// The parts it changed.
    pub parts: Vec<String>,
    /// The blocks it changed, by id.
    pub blocks: Vec<String>,
    /// The steps taken back first, newest first.
    pub handed_back: Vec<TakenBack>,
}

/// A step of this session's taken back because another editor's version changed what it
/// touched (or because it was entangled with one that did).
#[derive(Debug, Clone, PartialEq)]
pub struct HandedBackStep {
    /// The state number the step made.
    pub step: u64,
    /// The caller that made it (the store's key), if any.
    pub caller: Option<String>,
    /// The tool that made it, where a tool did.
    pub tool: Option<String>,
    /// The caller's context, as the step's owner keeps it (empty: none).
    pub context: Map<String, Value>,
    /// The blocks it wrote, by id.
    pub blocks: Vec<String>,
    /// The state number of the undo that took it back.
    pub undo: u64,
}

/// What another editor's version did.
#[derive(Debug, Clone, PartialEq)]
pub struct Refreshed {
    /// The version after it (the store's seq): the feed's `version`.
    pub version: u64,
    /// Its state number.
    pub number: u64,
    /// The feed's token for it (`<session nonce>-<seq>`): the base of the next refresh.
    pub token: String,
    /// Whether it changed the document.
    pub changed: bool,
    /// The parts it changed.
    pub parts: Vec<String>,
    /// The blocks it changed, by id.
    pub blocks: Vec<String>,
    /// This session's steps taken back first, newest first.
    pub handed_back: Vec<HandedBackStep>,
}

impl Session {
    /// Take in another editor's version of the open document `id`: `file`, their file, made
    /// from the version `base` names (a token of this session's feed, `<nonce>-<seq>`),
    /// committed as one step by `editor`. The steps of this session's since `base` that
    /// touched what it changed are taken back first and named.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; [`SessionError::RefreshBase`] for a token of another
    /// session's, one past the head, or a version no longer kept with every step after it;
    /// [`SessionError::NoRefresh`] where the document's format takes no other editor's
    /// versions; the format's refusal of the file.
    pub fn refresh(
        &self,
        id: &DocId,
        base: &str,
        file: Arc<[u8]>,
        editor: &str,
        context: &CallContext,
    ) -> Result<Refreshed, SessionError> {
        let document = self.document(id)?;
        let cancel = crate::CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        if let Err(error) = ooxml_base::Cancel::check(&cancel) {
            self.counts.cancelled.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Err(error.into());
        }
        let base = self.base_of(base, document.head().seq())?;
        let asked = RefreshAsk {
            kind: document.kind,
            editor,
            base,
            file,
            limits: self.context.limits,
        };
        let made = document.doc.refresh(&asked)?;
        let stored = document.stored();
        let handed_back = made
            .handed_back
            .into_iter()
            .map(|taken| {
                let owner = stored.owners().of(taken.number);
                HandedBackStep {
                    step: taken.number,
                    caller: taken.caller,
                    tool: owner.as_ref().map(|o| o.tool.to_string()),
                    context: owner.map(|o| (*o.context).clone()).unwrap_or_default(),
                    blocks: taken.blocks,
                    undo: taken.undo.1,
                }
            })
            .collect();
        let (version, number) = (made.version.seq(), made.version.number());
        if made.changed {
            let owner = StepOwner::of(REFRESH, &another_editor(editor, context));
            stored.arrived_step(version, number, owner);
        }
        Ok(Refreshed {
            version,
            number,
            token: format!("{}-{version}", self.nonce),
            changed: made.changed,
            parts: made.parts,
            blocks: made.blocks,
            handed_back,
        })
    }

    /// The seq a refresh's `base` token names: this session's, at or below `head`.
    fn base_of(&self, base: &str, head: u64) -> Result<u64, SessionError> {
        let stale = || SessionError::RefreshBase(format!("{base:?} is not a version this session gave"));
        let (nonce, seq) = base.rsplit_once('-').ok_or_else(stale)?;
        let seq: u64 = seq.parse().map_err(|_| stale())?;
        if nonce != self.nonce || seq > head {
            return Err(stale());
        }
        Ok(seq)
    }
}

/// The context another editor's step is owned with: the editor as its caller.
fn another_editor(editor: &str, context: &CallContext) -> CallContext {
    let mut fields = Map::new();
    fields.insert("editor".to_owned(), Value::from(editor));
    CallContext {
        caller: ooxml_store::Caller::foreign(editor),
        caller_context: Some(Arc::new(fields)),
        ..context.clone()
    }
}
