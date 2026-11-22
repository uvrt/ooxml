//! Owns: step groups through the session ([`Session::open_group`] and the rest): a host's
//! batch of calls on one document made one undo step of the batch's caller, the commits of
//! a failed op (or of a whole batch that applies all or none) taken back without a redo, and
//! what a Word document keeps beside its versions -- its tracking mode, the ids its versions
//! retired -- as they were at the mark a revert goes back to.
//!
//! Must not: run a tool, or let a group outlive the batch that opened it: the host closes
//! it on every path (its guard), and holds the document alone while it is open.

use crate::document::{Held, Word};
use crate::retired::Made;
use crate::{CallContext, DocId, Session, SessionError};
use ooxml_store::{DocModel, Error, Snapshot, Store};

/// Where a document's step group was, for [`Session::revert_group`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMark {
    store: ooxml_store::GroupMark,
    /// A Word document's tracking author then (`None` inside: untracked).
    tracking: Option<Option<String>>,
}

impl Session {
    /// Open a step group on `id`: the commits made until [`Session::close_group`] are one
    /// step. Where it is now.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's when a group is open already.
    pub fn open_group(&self, id: &DocId) -> Result<GroupMark, SessionError> {
        let document = self.document(id)?;
        let store = match &document.held {
            Held::Package(store) => store.open_group()?,
            Held::Word(word) => word.editor.store().open_group()?,
        };
        Ok(GroupMark {
            store,
            tracking: document.word().map(Word::tracking),
        })
    }

    /// Where `id`'s open group is now.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's without an open group.
    pub fn group_mark(&self, id: &DocId) -> Result<GroupMark, SessionError> {
        let document = self.document(id)?;
        let store = match &document.held {
            Held::Package(store) => store.group_mark(),
            Held::Word(word) => word.editor.store().group_mark(),
        };
        let store = store.ok_or(ooxml_store::StoreError::<String, crate::ModelError>::NoGroup)?;
        Ok(GroupMark {
            store,
            tracking: document.word().map(Word::tracking),
        })
    }

    /// Take back what `id`'s open group committed after `mark`, as a new version (its
    /// number; `None` when nothing was committed since), its tracking mode as it was then.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's without an open group.
    pub fn revert_group(
        &self,
        id: &DocId,
        mark: &GroupMark,
        context: &CallContext,
    ) -> Result<Option<u64>, SessionError> {
        let document = self.document(id)?;
        let reverted = match &document.held {
            Held::Package(store) => revert(store, mark)?.map(|(_, version)| version.seq().0),
            Held::Word(word) => {
                let found = revert(word.editor.store(), mark)?;
                if let Some((previous, version)) = &found {
                    // what the revert removed and brought back, as an undo notes it
                    let delta = ooxml_word_edit::ids_delta(previous.doc(), version.doc())?;
                    let made = Made {
                        seq: version.seq().0,
                        caller: context.caller.name(),
                        tool: None,
                        history: true,
                    };
                    word.retired.note(&made, &delta);
                    word.retired.number(made.seq, version.number());
                }
                if let Some(tracking) = &mark.tracking {
                    word.restore_tracking(tracking.clone());
                }
                found.map(|(_, version)| version.seq().0)
            }
        };
        Ok(reverted)
    }

    /// Close `id`'s open group: what it committed one step of the context's caller, made
    /// by `tool` (the batch's; `None`: none named) -- the step's version number; `None` when
    /// it committed nothing.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's without an open group.
    pub fn close_group(
        &self,
        id: &DocId,
        context: &CallContext,
        tool: Option<&str>,
    ) -> Result<Option<u64>, SessionError> {
        let document = self.document(id)?;
        let made = match &document.held {
            Held::Package(store) => store.close_group(&context.caller)?,
            Held::Word(word) => word.editor.store().close_group(&context.caller)?,
        };
        if let Some(seq) = made.filter(|_| !document.grouped()) {
            // (the host holds the document alone while its group is open: the head is the step)
            document.stepped(seq.0, document.head().number(), tool, context);
        }
        Ok(made.map(|seq| seq.0))
    }
}

/// A revert's head before and the version it made.
type Reverted<D> = (Snapshot<D>, Snapshot<D>);

/// The head before and the version a revert of `store` to `mark` made, if it made one.
fn revert<D: DocModel>(store: &Store<D>, mark: &GroupMark) -> Result<Option<Reverted<D>>, SessionError>
where
    SessionError: From<Error<D>>,
{
    let previous = store.snapshot();
    Ok(store.revert_group(mark.store)?.map(|version| (previous, version)))
}
