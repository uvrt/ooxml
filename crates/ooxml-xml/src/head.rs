//! Owns: [`HeadPatch`] -- one element's head (name, namespace declarations, attributes)
//! replaced in place, its children kept, addressed by a [`NodePath`] and checked by value:
//! applying it makes a new tree by path copying and refuses when the element's head is not
//! the one it replaces. Rebasing moves it past a splice ([`Patch`]) committed since, and a
//! splice past it, the way the store re-applies a transaction's changes to a newer head
//! (design section 5).
//!
//! Must not: search a tree for a node, touch an element's children, or change a node.

use crate::node::{Element, Head, Tree};
use crate::patch::{NodePath, Patch};
use crate::PatchError;
use std::sync::Arc;

/// The element at `path` gets the head `new` in place of `old`; its children stay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadPatch {
    path: NodePath,
    old: Arc<Head>,
    new: Arc<Head>,
}

impl HeadPatch {
    /// The head of the element at `path` (the root is `[0]`) changed from `old` to `new`.
    pub fn new(path: NodePath, old: Arc<Head>, new: Arc<Head>) -> HeadPatch {
        HeadPatch { path, old, new }
    }

    /// The element's path.
    pub fn path(&self) -> &NodePath {
        &self.path
    }

    /// The head it replaces.
    pub fn old(&self) -> &Arc<Head> {
        &self.old
    }

    /// The head it puts in place.
    pub fn new_head(&self) -> &Arc<Head> {
        &self.new
    }

    /// The patch that undoes this one, applied to this one's result.
    pub fn inverse(&self) -> HeadPatch {
        HeadPatch::new(self.path.clone(), Arc::clone(&self.new), Arc::clone(&self.old))
    }

    /// The tree with this patch applied; the element at its path must have its old head.
    pub fn apply(&self, tree: &Tree) -> Result<Tree, PatchError> {
        match self.path.0.split_first() {
            Some((0, below)) => Ok(tree.with_root(self.edit(tree.root(), below)?)),
            _ => Err(PatchError::NoElement(self.path.0.clone())),
        }
    }

    fn edit(&self, element: &Arc<Element>, path: &[usize]) -> Result<Arc<Element>, PatchError> {
        let Some((&i, below)) = path.split_first() else {
            if !(Arc::ptr_eq(element.head(), &self.old) || **element.head() == *self.old) {
                return Err(PatchError::Stale {
                    parent: self.path.0.clone(),
                    start: 0,
                });
            }
            return Ok(Arc::new(Element::from_head(
                Arc::clone(&self.new),
                element.children().clone(),
            )));
        };
        let missing = || PatchError::NoElement(self.path.0.clone());
        let child = element
            .children()
            .get(i)
            .and_then(|n| n.as_element())
            .ok_or_else(missing)?;
        let edited = self.edit(child, below)?;
        let children = element
            .children()
            .splice(i..i + 1, vec![crate::Node::Element(edited)])
            .ok_or_else(missing)?;
        Ok(Arc::new(element.with_children(children)))
    }

    /// This patch, made on the version `earlier` was applied to, moved to apply after it.
    /// Fails with [`PatchError::Conflict`] when `earlier` removed or replaced the element.
    pub fn rebase(&self, earlier: &Patch) -> Result<HeadPatch, PatchError> {
        let conflict = || PatchError::Conflict(self.path.0.clone());
        if earlier.parent().0.is_empty() {
            return Err(conflict());
        }
        let mut moved = self.clone();
        if earlier.parent().is_ancestor_of(&self.path) {
            let level = earlier.parent().0.len();
            let index = self.path.0.get(level).copied().ok_or_else(conflict)?;
            let (start, end) = (earlier.start(), earlier.start() + earlier.removed().len());
            if (start..end).contains(&index) {
                return Err(conflict());
            }
            if index >= end {
                let delta = earlier.inserted().len() as isize - earlier.removed().len() as isize;
                if let Some(slot) = moved.path.0.get_mut(level) {
                    *slot = index.checked_add_signed(delta).ok_or_else(conflict)?;
                }
            }
        }
        Ok(moved)
    }

    /// This patch moved past `earlier`, another head patch: two changes of one element's
    /// head conflict; any other pair commutes.
    pub fn rebase_head(&self, earlier: &HeadPatch) -> Result<HeadPatch, PatchError> {
        if earlier.path == self.path {
            return Err(PatchError::Conflict(self.path.0.clone()));
        }
        Ok(self.clone())
    }
}

impl Patch {
    /// This splice moved past `earlier`, a head patch committed since: it conflicts when it
    /// removes the element whose head changed (the node it removes is no longer there);
    /// otherwise paths are unchanged.
    pub fn rebase_head(&self, earlier: &HeadPatch) -> Result<Patch, PatchError> {
        let conflict = || PatchError::Conflict(self.parent().0.clone());
        if self.parent().0.is_empty() {
            return Err(conflict());
        }
        if self.parent().is_ancestor_of(earlier.path()) {
            let index = earlier
                .path()
                .0
                .get(self.parent().0.len())
                .copied()
                .ok_or_else(conflict)?;
            if (self.start()..self.start() + self.removed().len()).contains(&index) {
                return Err(conflict());
            }
        }
        Ok(self.clone())
    }
}
