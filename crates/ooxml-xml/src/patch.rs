//! Owns: [`Patch`] -- one splice of an element's children (or the root's replacement),
//! addressed by a [`NodePath`] and checked by node (pointer, else equal content): applying it
//! makes a new tree by path copying and refuses when the nodes it removes are not the ones at
//! its position.
//! [`Patch::rebase`] moves a patch made on one version past a patch committed since, the way
//! the store re-applies a transaction's change set to a newer head (design section 5).
//!
//! Must not: search a tree for a node (a patch is exact or it is stale), or change a node.

use crate::node::{Element, Node, Tree};
use crate::PatchError;
use std::sync::Arc;

/// A node's address from the document: `[]` is the document itself, `[0]` its root element,
/// and each further index a child's position among its parent's children (all node kinds
/// counted).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct NodePath(pub Vec<usize>);

impl NodePath {
    /// The document.
    pub fn document() -> NodePath {
        NodePath(Vec::new())
    }

    /// The root element.
    pub fn root() -> NodePath {
        NodePath(vec![0])
    }

    /// This path and then `index`.
    pub fn child(&self, index: usize) -> NodePath {
        let mut v = self.0.clone();
        v.push(index);
        NodePath(v)
    }

    /// Whether `self` is a proper prefix of `other` (an ancestor's path).
    pub fn is_ancestor_of(&self, other: &NodePath) -> bool {
        self.0.len() < other.0.len() && other.0.starts_with(&self.0)
    }
}

/// Replace the children `start .. start + removed.len()` of the element at `parent` by
/// `inserted`. At the document level (`parent` empty) the one child is the root element,
/// replaced by one element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    parent: NodePath,
    start: usize,
    removed: Vec<Node>,
    inserted: Vec<Node>,
}

impl Patch {
    /// A splice of `parent`'s children.
    pub fn splice(parent: NodePath, start: usize, removed: Vec<Node>, inserted: Vec<Node>) -> Patch {
        Patch {
            parent,
            start,
            removed,
            inserted,
        }
    }

    /// The root element replaced.
    pub fn replace_root(old: &Arc<Element>, new: Arc<Element>) -> Patch {
        Patch::splice(
            NodePath::document(),
            0,
            vec![Node::Element(Arc::clone(old))],
            vec![Node::Element(new)],
        )
    }

    /// The parent's path.
    pub fn parent(&self) -> &NodePath {
        &self.parent
    }

    /// The first child replaced.
    pub fn start(&self) -> usize {
        self.start
    }

    /// The nodes removed (as they are in the version the patch was made on).
    pub fn removed(&self) -> &[Node] {
        &self.removed
    }

    /// The nodes put in their place.
    pub fn inserted(&self) -> &[Node] {
        &self.inserted
    }

    /// The patch that undoes this one, applied to this one's result.
    pub fn inverse(&self) -> Patch {
        Patch::splice(
            self.parent.clone(),
            self.start,
            self.inserted.clone(),
            self.removed.clone(),
        )
    }

    /// The tree with this patch applied; the nodes it removes must be at its position.
    pub fn apply(&self, tree: &Tree) -> Result<Tree, PatchError> {
        let Some((first, below)) = self.parent.0.split_first() else {
            return self.apply_to_document(tree);
        };
        if *first != 0 {
            return Err(PatchError::NoElement(self.parent.0.clone()));
        }
        let root = self.edit(tree.root(), below)?;
        Ok(tree.with_root(root))
    }

    fn apply_to_document(&self, tree: &Tree) -> Result<Tree, PatchError> {
        match (self.start, self.removed.as_slice(), self.inserted.as_slice()) {
            (0, [Node::Element(old)], [Node::Element(new)]) => {
                if !Arc::ptr_eq(old, tree.root()) {
                    return Err(PatchError::Stale {
                        parent: Vec::new(),
                        start: 0,
                    });
                }
                Ok(tree.with_root(Arc::clone(new)))
            }
            _ => Err(PatchError::BadRoot),
        }
    }

    /// `element` with the patch applied at `path` below it, by path copying.
    fn edit(&self, element: &Arc<Element>, path: &[usize]) -> Result<Arc<Element>, PatchError> {
        let Some((&i, below)) = path.split_first() else {
            return self.splice_here(element).map(Arc::new);
        };
        let child = element
            .children()
            .get(i)
            .and_then(Node::as_element)
            .ok_or_else(|| PatchError::NoElement(self.parent.0.clone()))?;
        let edited = self.edit(child, below)?;
        let children = element
            .children()
            .splice(i..i + 1, vec![Node::Element(edited)])
            .ok_or_else(|| PatchError::NoElement(self.parent.0.clone()))?;
        Ok(Arc::new(element.with_children(children)))
    }

    fn splice_here(&self, element: &Element) -> Result<Element, PatchError> {
        let stale = || PatchError::Stale {
            parent: self.parent.0.clone(),
            start: self.start,
        };
        let children = element.children();
        for (k, node) in self.removed.iter().enumerate() {
            if !children.get(self.start + k).is_some_and(|have| have.removable_as(node)) {
                return Err(stale());
            }
        }
        let end = self.start + self.removed.len();
        let spliced = children
            .splice(self.start..end, self.inserted.clone())
            .ok_or_else(stale)?;
        Ok(element.with_children(spliced))
    }

    /// This patch, made on the version `earlier` was applied to, moved to apply after it.
    /// Fails with [`PatchError::Conflict`] when the two touch the same nodes. Two inserts at
    /// one position keep publish order: `earlier`, published first, has its nodes first.
    pub fn rebase(&self, earlier: &Patch) -> Result<Patch, PatchError> {
        self.moved_past(earlier, false)
    }

    /// This patch, published before `later` but moved to apply after it (both made on one
    /// version): the other side of a sequence's rebase, where a change published first is
    /// carried past a later commit's earlier changes. As [`Patch::rebase`], but two inserts
    /// at one position keep this patch's nodes first, so moving either past the other gives
    /// one result: `later.rebase(self)` after `self` equals `self.rebase_first(later)` after
    /// `later`.
    pub fn rebase_first(&self, later: &Patch) -> Result<Patch, PatchError> {
        self.moved_past(later, true)
    }

    /// This patch moved past `earlier`; `first`: at one insertion point this patch's nodes
    /// stay first (it was published first), else `earlier`'s come first.
    ///
    /// The cases, the same either way round but the first:
    /// 1. both insert at one point: the one published first comes first;
    /// 2. an insert where a range the other replaces starts: before its replacement;
    /// 3. an insert where that range ends: after its replacement;
    /// 4. two ranges that meet at one point: in document order;
    /// 5. ranges that overlap, or a change inside a node the other replaces: a conflict.
    fn moved_past(&self, earlier: &Patch, first: bool) -> Result<Patch, PatchError> {
        let conflict = || PatchError::Conflict(self.parent.0.clone());
        if self.parent.0.is_empty() || earlier.parent.0.is_empty() {
            return Err(conflict());
        }
        let delta = earlier.inserted.len() as isize - earlier.removed.len() as isize;
        let (sa, ea) = (earlier.start, earlier.start + earlier.removed.len());
        let mut moved = self.clone();
        if earlier.parent == self.parent {
            let (sb, eb) = (self.start, self.start + self.removed.len());
            let tie = sa == ea && sb == eb && sa == sb;
            if (tie && !first) || (sb >= ea && !(sb == eb && sb == sa)) {
                // after the earlier change (or an insert at the same point, published after it)
                moved.start = shift(sb, delta).ok_or_else(conflict)?;
            } else if eb > sa {
                return Err(conflict());
            }
        } else if earlier.parent.is_ancestor_of(&self.parent) {
            let level = earlier.parent.0.len();
            let index = self.parent.0.get(level).copied().ok_or_else(conflict)?;
            if (sa..ea).contains(&index) {
                return Err(conflict());
            }
            if index >= ea {
                if let Some(slot) = moved.parent.0.get_mut(level) {
                    *slot = shift(index, delta).ok_or_else(conflict)?;
                }
            }
        } else if self.parent.is_ancestor_of(&earlier.parent) {
            let index = earlier
                .parent
                .0
                .get(self.parent.0.len())
                .copied()
                .ok_or_else(conflict)?;
            if (self.start..self.start + self.removed.len()).contains(&index) {
                return Err(conflict());
            }
        }
        Ok(moved)
    }
}

fn shift(index: usize, delta: isize) -> Option<usize> {
    index.checked_add_signed(delta)
}
