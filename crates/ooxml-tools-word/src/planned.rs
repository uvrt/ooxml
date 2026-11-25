//! Owns: [`Planned`], a command a handler works out on the version it is applied to: the
//! handler's plan -- the indices, ranges and names it reads off the document and turns into
//! a command -- runs in the command's `prepare`, on the transaction's version, the keys it
//! read recorded there ([`Plan::reads`]); a commit meanwhile that wrote one is a conflict,
//! and the store prepares the command again, planning it again on the newer version. A plan
//! worked out on the head before the command runs would be applied, stale, to a version it
//! was not worked out on.
//!
//! A plan that refuses writes nothing: its refusal is the command's out, said on the version
//! it was worked out on.
//!
//! Must not: write anything but through the planned command, or keep a plan past the run
//! that made it.

use ooxml_store::{Command, Txn};
use ooxml_word_edit::{EditError, Key, WordDoc};
use std::sync::{Mutex, PoisonError};

/// What a plan works out: the command, the keys the plan read off the version that the
/// command's own reads do not cover, and what the handler's reply needs of the plan.
pub(crate) struct Plan<C, S> {
    /// The command to run.
    pub(crate) command: C,
    /// The keys read in planning, read again in the transaction.
    pub(crate) reads: Vec<Key>,
    /// What the reply needs of the plan.
    pub(crate) said: S,
}

impl<C> Plan<C, ()> {
    /// `command`, its plan read by the keys `reads`, nothing kept for the reply.
    pub(crate) fn of(command: C, reads: Vec<Key>) -> Self {
        Plan {
            command,
            reads,
            said: (),
        }
    }
}

/// A command planned in its own `prepare` by `plan`, on the version it is applied to
/// ([`Planned::new`]); its out the planned command's, or the plan's refusal.
pub(crate) struct Planned<F, S> {
    plan: F,
    /// What the last prepare's plan kept for the reply: the committed one's, or the refused.
    said: Mutex<Option<S>>,
}

impl<F, S> Planned<F, S> {
    /// The command `plan` works out on the version it is applied to.
    pub(crate) fn new(plan: F) -> Self {
        Planned {
            plan,
            said: Mutex::new(None),
        }
    }

    /// What the last plan kept for the reply (that of the version the command was applied
    /// to); `None` where none was worked out, or it refused.
    pub(crate) fn said(&self) -> Option<S> {
        self.said.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

impl<F, C, S, R> Command<WordDoc> for Planned<F, S>
where
    F: Fn(&WordDoc) -> Result<Plan<C, S>, R>,
    C: Command<WordDoc, Error = EditError>,
{
    /// The planned command's out, or the plan's refusal (nothing written).
    type Out = Result<C::Out, R>;
    type Error = EditError;

    /// Nothing to check before the plan: the plan is the check, in `prepare`.
    fn validate(&self, _doc: &WordDoc) -> Result<(), EditError> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Self::Out, EditError> {
        let plan = match (self.plan)(tx.doc()) {
            Ok(plan) => plan,
            Err(refused) => {
                *self.said.lock().unwrap_or_else(PoisonError::into_inner) = None;
                return Ok(Err(refused));
            }
        };
        for key in plan.reads {
            tx.read(key);
        }
        *self.said.lock().unwrap_or_else(PoisonError::into_inner) = Some(plan.said);
        plan.command.validate(tx.doc())?;
        plan.command.prepare(tx).map(Ok)
    }
}
