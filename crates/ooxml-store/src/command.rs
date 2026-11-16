//! Owns: [`Command`], an edit as the store runs it (validate, then prepare on a snapshot),
//! and [`Batch`], several commands as one transaction and one history step.
//!
//! Must not: know any tool; the tool layer's handlers implement [`Command`].

use crate::{DocModel, Error, Txn};

/// One edit: checked against a document, then prepared in a transaction. Both may run more
/// than once (a conflict redoes them on a newer version), so neither may have effects
/// outside the transaction.
pub trait Command<D: DocModel> {
    /// What the edit returns.
    type Out;
    /// Why it refused; the store's own errors convert into it.
    type Error: From<Error<D>>;

    /// Check the arguments against `doc`. What the outcome depends on must be read again in
    /// [`Command::prepare`] through the transaction, which records it.
    fn validate(&self, doc: &D) -> Result<(), Self::Error>;

    /// Make the edit's changes in `tx`, recording what they depend on.
    fn prepare(&self, tx: &mut Txn<'_, D>) -> Result<Self::Out, Self::Error>;
}

/// Commands applied in order in one transaction: one commit and one history step, costing
/// the sum of its ops. A failing op returns the transaction to the savepoint taken before
/// it and the others go on, unless the batch is atomic: then the first failure refuses the
/// whole batch and nothing changes.
#[derive(Debug, Clone)]
pub struct Batch<C> {
    ops: Vec<C>,
    atomic: bool,
}

impl<C> Batch<C> {
    /// A batch where a failing op is skipped.
    pub fn new(ops: Vec<C>) -> Self {
        Self { ops, atomic: false }
    }

    /// A batch where a failing op refuses the whole batch.
    pub fn atomic(ops: Vec<C>) -> Self {
        Self { ops, atomic: true }
    }
}

impl<D: DocModel, C: Command<D>> Command<D> for Batch<C> {
    /// Each op's result, in order.
    type Out = Vec<Result<C::Out, C::Error>>;
    type Error = C::Error;

    /// Each op is checked against the working version when its turn comes.
    fn validate(&self, _doc: &D) -> Result<(), C::Error> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, D>) -> Result<Self::Out, C::Error> {
        let mut outs = Vec::with_capacity(self.ops.len());
        for op in &self.ops {
            let point = tx.savepoint();
            match op.validate(tx.doc()).and_then(|()| op.prepare(tx)) {
                Ok(out) => outs.push(Ok(out)),
                Err(error) if self.atomic => return Err(error),
                Err(error) => {
                    tx.rollback(point);
                    outs.push(Err(error));
                }
            }
        }
        Ok(outs)
    }
}
