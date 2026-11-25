//! Owns: step groups through the session ([`Session::open_group`] and the rest): a host's
//! batch of calls on one document made one undo step of the batch's caller, the work of a
//! failed op (or of a whole batch that applies all or none) taken back without a redo, and
//! what a Word document keeps beside its versions -- its tracking mode, the ids its versions
//! retired -- as they were at the mark a revert goes back to.
//!
//! A batch is one transaction its owner holds open on the document
//! ([`ooxml_store::Store::open_txn`]), of any kind: the owner's calls on the document run
//! their edits in it (a savepoint each) and read the version its ops left, while every other
//! caller reads the head; it holds the document's gate from open to close, so other writers
//! publish after it, and it commits once as it closes. A second batch by the same owner is
//! refused; another owner's waits for it.
//!
//! **The owner is the calling thread** ([`this_thread`]): the Python host's rule, which runs
//! each batch -- its open, its ops and its close -- on one thread, and whose interpreter lock
//! is let go during every call. In Phase 3 the Rust host passes an explicit owner instead
//! (docs/architecture/README.md, Phase 3: the Rust host re-runs a batch), and batches become
//! optimistic there; until then a batch holds the gate for its whole run.
//!
//! Must not: run a tool, or let a group outlive the batch that opened it: the host closes
//! it on every path (its guard; a dropped Python group takes its work back).

use crate::document::{Held, Word};
use crate::retired::Made;
use crate::{CallContext, DocId, Session, SessionError};
use ooxml_store::{DocModel, Error, Store, TxnMark, TxnOwner};
use std::sync::atomic::{AtomicU64, Ordering};

/// Where a document's step group was, for [`Session::revert_group`]; the mark
/// [`Session::open_group`] gives names the group itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMark {
    /// Who holds the group's transaction.
    owner: TxnOwner,
    /// Where in it.
    mark: TxnMark,
    /// A Word document's tracking author then (`None` inside: untracked).
    tracking: Option<Option<String>>,
}

/// The owner a thread's batches hold their transactions as: its own, never another's (the
/// Python host's rule, see the module's docs).
pub(crate) fn this_thread() -> TxnOwner {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    thread_local! {
        static OWNER: TxnOwner = TxnOwner::new(NEXT.fetch_add(1, Ordering::Relaxed));
    }
    OWNER.with(|owner| *owner)
}

impl Session {
    /// Open a step group on `id`, held by this thread: the work done until
    /// [`Session::close_group`] is one step. Where it is now, which names the group to its
    /// other calls. Waits while another thread's batch holds the document.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's when this thread's group is open already.
    pub fn open_group(&self, id: &DocId) -> Result<GroupMark, SessionError> {
        let document = self.document(id)?;
        let owner = this_thread();
        let mark = match &document.held {
            Held::Package(store) => open(store, owner)?,
            Held::Word(word) => open(word.editor.store(), owner)?,
        };
        Ok(GroupMark {
            owner,
            mark,
            tracking: document.word().map(Word::tracking),
        })
    }

    /// Where `id`'s open group `group` (its [`Session::open_group`] mark) is now.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's without an open group.
    pub fn group_mark(&self, id: &DocId, group: &GroupMark) -> Result<GroupMark, SessionError> {
        let document = self.document(id)?;
        let mark = match &document.held {
            Held::Package(store) => store.txn_mark(group.owner)?,
            Held::Word(word) => word.editor.store().txn_mark(group.owner)?,
        };
        Ok(GroupMark {
            owner: group.owner,
            mark,
            tracking: document.word().map(Word::tracking),
        })
    }

    /// Take back what `id`'s open group did after `mark`, as a new version (its number;
    /// `None` when nothing was done since), its tracking mode as it was then.
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
            Held::Package(store) => store
                .txn_revert(mark.owner, mark.mark)?
                .map(|(_, version)| version.seq().0),
            Held::Word(word) => {
                let found = word.editor.store().txn_revert(mark.owner, mark.mark)?;
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

    /// Close `id`'s open group `group` (its [`Session::open_group`] mark): its transaction
    /// committed once, what it did one step of the context's caller, made by `tool` (the
    /// batch's; `None`: none named) -- the step's version number; `None` when it did nothing.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's without an open group.
    pub fn close_group(
        &self,
        id: &DocId,
        group: &GroupMark,
        context: &CallContext,
        tool: Option<&str>,
    ) -> Result<Option<u64>, SessionError> {
        let document = self.document(id)?;
        let made = match &document.held {
            Held::Package(store) => commit(store, group.owner, context)?,
            Held::Word(word) => commit(word.editor.store(), group.owner, context)?,
        };
        if let Some((seq, number)) = made {
            document.stepped(seq, number, tool, context);
        }
        Ok(made.map(|(seq, _)| seq))
    }

    /// Let `id`'s open group `group` go without a step, what it did taken back (a Python
    /// group dropped unclosed); whether it was open.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn abandon_group(&self, id: &DocId, group: &GroupMark) -> Result<bool, SessionError> {
        let document = self.document(id)?;
        Ok(match &document.held {
            Held::Package(store) => store.drop_txn(group.owner),
            Held::Word(word) => {
                let dropped = word.editor.store().drop_txn(group.owner);
                if let (true, Some(tracking)) = (dropped, &group.tracking) {
                    word.restore_tracking(tracking.clone());
                }
                dropped
            }
        })
    }
}

impl Session {
    /// The reflow of what `id`'s open step group `group` did, measured once (a batch's
    /// checks, after its last op): from the version the group started from to the version
    /// its ops left, laid out by the call's layout, else the document's own engine -- only
    /// where that engine laid out the version the group started from (`reflow_after_edit`);
    /// `None` where nothing is measured (not a Word document, no change, no layout of the
    /// start).
    ///
    /// # Errors
    /// [`SessionError::NotOpen`].
    pub fn group_reflow(
        &self,
        id: &DocId,
        group: &GroupMark,
        context: &CallContext,
    ) -> Result<Option<serde_json::Value>, SessionError> {
        let document = self.document(id)?;
        let Some(word) = document.word() else {
            return Ok(None);
        };
        let store = word.editor.store();
        let (Some(start), Some(head)) = (store.txn_base(group.owner), store.txn_current(group.owner)) else {
            return Ok(None);
        };
        if head.seq() == start.seq() {
            return Ok(None);
        }
        let Some(layout) = context.layout.clone().or_else(|| self.own_pages(id)) else {
            return Ok(None);
        };
        let cancel = crate::context::CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        let layout_cancel = cancel.layout();
        let limit = layout_cancel.as_ref().map(crate::context::LayoutCancel::limit);
        let from = ooxml_tools_word::EditedFrom {
            version: start.seq().0,
            package: start.doc().package(),
        };
        Ok(ooxml_tools_word::reflow_after_edit(
            &*layout,
            &word.last_check,
            from,
            head.doc().package(),
            head.doc().ids().aliases(),
            (limit, &cancel),
        ))
    }
}

/// `owner`'s transaction held open on `store`: the mark it opened at.
fn open<D: DocModel>(store: &Store<D>, owner: TxnOwner) -> Result<TxnMark, SessionError>
where
    D::Setup: Default,
    SessionError: From<Error<D>>,
{
    store.open_txn(owner, &D::Setup::default())?;
    Ok(store.txn_start())
}

/// `owner`'s transaction on `store` committed as the context's caller: the version it
/// published and its state number, `None` when it changed nothing.
fn commit<D: DocModel>(
    store: &Store<D>,
    owner: TxnOwner,
    context: &CallContext,
) -> Result<Option<(u64, u64)>, SessionError>
where
    SessionError: From<Error<D>>,
{
    let done = store.commit_txn(owner, &context.caller)?;
    Ok(done.changed.then(|| (done.version.seq().0, done.version.number())))
}
