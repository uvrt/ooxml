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

use crate::{CallContext, DocId, Session, SessionError};
use ooxml_store::{TxnMark, TxnOwner};
use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Where a document's step group was, for [`Session::revert_group`]; the mark
/// [`Session::open_group`] gives names the group itself.
#[derive(Clone)]
pub struct GroupMark {
    /// Who holds the group's transaction.
    owner: TxnOwner,
    /// Where in it.
    mark: TxnMark,
    /// The format's own state then ([`crate::FormatDoc::group_state`]: a Word document's
    /// tracking mode).
    state: Option<Arc<dyn Any + Send + Sync>>,
}

impl std::fmt::Debug for GroupMark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupMark")
            .field("owner", &self.owner)
            .field("mark", &self.mark)
            .finish_non_exhaustive()
    }
}

impl GroupMark {
    /// Who holds the group's transaction.
    #[must_use]
    pub fn owner(&self) -> TxnOwner {
        self.owner
    }
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
        let mark = document.stored().open_group(owner)?;
        Ok(GroupMark {
            owner,
            mark,
            state: document.doc.group_state(),
        })
    }

    /// Where `id`'s open group `group` (its [`Session::open_group`] mark) is now.
    ///
    /// # Errors
    /// [`SessionError::NotOpen`]; the store's without an open group.
    pub fn group_mark(&self, id: &DocId, group: &GroupMark) -> Result<GroupMark, SessionError> {
        let document = self.document(id)?;
        let mark = document.stored().group_mark(group.owner)?;
        Ok(GroupMark {
            owner: group.owner,
            mark,
            state: document.doc.group_state(),
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
        let reverted = document
            .stored()
            .revert_group(mark.owner, mark.mark, context.caller.name())?;
        if let Some(state) = &mark.state {
            document.doc.restore_group(&**state);
        }
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
        let made = document.stored().commit_group(group.owner, &context.caller)?;
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
        let dropped = document.stored().drop_group(group.owner);
        if let (true, Some(state)) = (dropped, &group.state) {
            document.doc.restore_group(&**state);
        }
        Ok(dropped)
    }
}
