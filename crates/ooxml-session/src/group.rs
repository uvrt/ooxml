//! Owns: step groups through the session ([`Session::open_group`] and the rest): a host's
//! batch of calls on one document made one undo step of the batch's caller, the work of a
//! failed op (or of a whole batch that applies all or none) taken back without a redo, and
//! what a Word document keeps beside its versions -- its tracking mode, the ids its versions
//! retired -- as they were at the mark a revert goes back to.
//!
//! A Word document's batch is one transaction its owner holds open
//! ([`ooxml_store::Store::open_txn`]): the owner is the thread that opened it, whose calls on
//! the document run their edits in it (a savepoint each) and read the version its ops left,
//! while every other caller reads the head; it holds the document's gate from open to close,
//! so other writers publish after it, and it commits once as it closes. A second batch by the
//! same thread is refused; another thread's waits for it. Any other kind's group is the
//! store's step group.
//!
//! Must not: run a tool, or let a group outlive the batch that opened it: the host closes
//! it on every path (its guard; a dropped Python group takes its work back).

use crate::document::{Held, Word};
use crate::retired::Made;
use crate::{CallContext, DocId, Session, SessionError};
use ooxml_store::{DocModel, Error, Snapshot, Store, TxnMark, TxnOwner};
use std::sync::atomic::{AtomicU64, Ordering};

/// Where a document's step group was, for [`Session::revert_group`]; the mark
/// [`Session::open_group`] gives names the group itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMark {
    at: At,
    /// A Word document's tracking author then (`None` inside: untracked).
    tracking: Option<Option<String>>,
}

/// Where, by the kind of group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum At {
    /// In a package's step group.
    Package(ooxml_store::GroupMark),
    /// In the transaction `owner` holds on a Word document.
    Word { owner: TxnOwner, mark: TxnMark },
}

/// The owner a thread's batches hold their transactions as: its own, never another's.
pub(crate) fn this_thread() -> TxnOwner {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    thread_local! {
        static OWNER: TxnOwner = TxnOwner::new(NEXT.fetch_add(1, Ordering::Relaxed));
    }
    OWNER.with(|owner| *owner)
}

impl Session {
    /// Open a step group on `id`: the work done until [`Session::close_group`] is one step.
    /// Where it is now, which names the group to its other calls. A Word document's waits
    /// while another thread's batch holds it.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's when this thread's group is open already.
    pub fn open_group(&self, id: &DocId) -> Result<GroupMark, SessionError> {
        let document = self.document(id)?;
        let at = match &document.held {
            Held::Package(store) => At::Package(store.open_group()?),
            Held::Word(word) => {
                let store = word.editor.store();
                let owner = this_thread();
                store.open_txn(owner, &ooxml_word_edit::Setup::default())?;
                At::Word {
                    owner,
                    mark: store.txn_start(),
                }
            }
        };
        Ok(GroupMark {
            at,
            tracking: document.word().map(Word::tracking),
        })
    }

    /// Where `id`'s open group `group` (its [`Session::open_group`] mark) is now.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's without an open group.
    pub fn group_mark(&self, id: &DocId, group: &GroupMark) -> Result<GroupMark, SessionError> {
        let document = self.document(id)?;
        let at = match (&document.held, group.at) {
            (Held::Package(store), _) => At::Package(store.group_mark().ok_or_else(no_group)?),
            (Held::Word(word), At::Word { owner, .. }) => At::Word {
                owner,
                mark: word.editor.store().txn_mark(owner)?,
            },
            (Held::Word(_), At::Package(_)) => return Err(no_group()),
        };
        Ok(GroupMark {
            at,
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
        let reverted = match (&document.held, mark.at) {
            (Held::Package(store), At::Package(at)) => revert(store, at)?.map(|(_, version)| version.seq().0),
            (Held::Word(word), At::Word { owner, mark: at }) => {
                let found = word.editor.store().txn_revert(owner, at)?;
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
            _ => return Err(no_group()),
        };
        Ok(reverted)
    }

    /// Close `id`'s open group `group` (its [`Session::open_group`] mark): what it did one
    /// step of the context's caller, made by `tool` (the batch's; `None`: none named) -- the
    /// step's version number; `None` when it did nothing. A Word document's batch commits
    /// here, once.
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
        match (&document.held, group.at) {
            (Held::Package(store), _) => {
                let made = store.close_group(&context.caller)?;
                if let Some(seq) = made.filter(|_| !document.grouped()) {
                    // (the host holds the document alone while its group is open: the head
                    // is the step)
                    document.stepped(seq.0, document.head().number(), tool, context);
                }
                Ok(made.map(|seq| seq.0))
            }
            (Held::Word(word), At::Word { owner, .. }) => {
                let done = word.editor.store().commit_txn(owner, &context.caller)?;
                if !done.changed {
                    return Ok(None);
                }
                let version = &done.version;
                document.stepped(version.seq().0, version.number(), tool, context);
                Ok(Some(version.seq().0))
            }
            (Held::Word(_), At::Package(_)) => Err(no_group()),
        }
    }

    /// Let `id`'s open group `group` go without a step, what it did taken back (a Python
    /// group dropped unclosed); whether it was open.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's where a package's revert fails.
    pub fn abandon_group(&self, id: &DocId, group: &GroupMark, context: &CallContext) -> Result<bool, SessionError> {
        let document = self.document(id)?;
        match (&document.held, group.at) {
            (Held::Package(store), At::Package(at)) => {
                if store.group_mark().is_none() {
                    return Ok(false);
                }
                revert(store, at)?;
                store.close_group(&context.caller)?;
                Ok(true)
            }
            (Held::Word(word), At::Word { owner, .. }) => {
                let dropped = word.editor.store().drop_txn(owner);
                if let (true, Some(tracking)) = (dropped, &group.tracking) {
                    word.restore_tracking(tracking.clone());
                }
                Ok(dropped)
            }
            _ => Ok(false),
        }
    }
}

impl Session {
    /// The reflow of what `id`'s open step group `group` did, measured once (a batch's
    /// checks, after its last op): from the version the group started from to the version
    /// its ops left, laid out by the call's layout, else the document's own engine -- only
    /// where that engine laid out the version the group started from (`reflow_after_edit`);
    /// `None` where nothing is measured (no group, no change, no layout of the start).
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
        let (Some(word), At::Word { owner, .. }) = (document.word(), group.at) else {
            return Ok(None);
        };
        let store = word.editor.store();
        let (Some(start), Some(head)) = (store.txn_base(owner), store.txn_current(owner)) else {
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

/// No group of the document's kind.
fn no_group() -> SessionError {
    ooxml_store::StoreError::<String, crate::ModelError>::NoGroup.into()
}

/// A revert's head before and the version it made.
type Reverted<D> = (Snapshot<D>, Snapshot<D>);

/// The head before and the version a revert of `store` to `mark` made, if it made one.
fn revert<D: DocModel>(store: &Store<D>, mark: ooxml_store::GroupMark) -> Result<Option<Reverted<D>>, SessionError>
where
    SessionError: From<Error<D>>,
{
    let previous = store.snapshot();
    Ok(store.revert_group(mark)?.map(|version| (previous, version)))
}
