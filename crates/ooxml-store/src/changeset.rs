//! Owns: [`ChangeSet`], the changes that made a version from the one before: a commit's, as
//! prepared, or an exact undo's, the inverses of its step's changes -- worked out only when
//! something needs them (a commit or a scoped undo moved past the undo), so a plain undo
//! costs a pointer.
//!
//! Must not: apply anything to a published version, or lock.

use crate::{DocModel, Error, StoreError};
use std::sync::{Arc, OnceLock};

/// The changes that made a version from the one before. Cloning shares.
pub(crate) struct ChangeSet<D: DocModel>(Arc<Inner<D>>);

enum Inner<D: DocModel> {
    /// As made.
    Made(Arc<[D::Change]>),
    /// The inverses of `of`, made on `on`, in the order they apply: worked out once, when
    /// first asked (`None` when one cannot be).
    Inverse {
        on: D,
        of: ChangeSet<D>,
        worked: OnceLock<Option<Arc<[D::Change]>>>,
    },
}

impl<D: DocModel> Clone for ChangeSet<D> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<D: DocModel> ChangeSet<D> {
    /// These changes.
    pub(crate) fn made(changes: Arc<[D::Change]>) -> Self {
        Self(Arc::new(Inner::Made(changes)))
    }

    /// What takes back `of`, which was applied to `on`.
    pub(crate) fn inverse_of(on: D, of: ChangeSet<D>) -> Self {
        Self(Arc::new(Inner::Inverse {
            on,
            of,
            worked: OnceLock::new(),
        }))
    }

    /// The changes, in order; `None` when an inverse could not be worked out.
    pub(crate) fn changes(&self) -> Option<Arc<[D::Change]>> {
        match &*self.0 {
            Inner::Made(changes) => Some(Arc::clone(changes)),
            Inner::Inverse { on, of, worked } => worked
                .get_or_init(|| {
                    let of = of.changes()?;
                    inverses(on, &of).ok().map(Arc::from)
                })
                .clone(),
        }
    }
}

/// The inverses of `changes`, made on `on`, in the order they apply (newest first).
pub(crate) fn inverses<D: DocModel>(on: &D, changes: &[D::Change]) -> Result<Vec<D::Change>, Error<D>> {
    let mut at = on.clone();
    let mut inverses = Vec::with_capacity(changes.len());
    for change in changes {
        inverses.push(at.inverse(change).map_err(StoreError::Model)?);
        at.apply(change).map_err(StoreError::Model)?;
    }
    inverses.reverse();
    Ok(inverses)
}
