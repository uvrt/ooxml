//! Owns: [`Cursor`], a red node -- a position in one version of a tree that knows its parent
//! and its index, made on demand while walking (green nodes do not know their parents, so
//! that versions can share them).
//!
//! Must not: outlive the meaning of its version (a cursor is into the version it was made
//! from; patches it makes are checked against that version's nodes).

use crate::node::{Element, Node, Tree};
use crate::patch::{NodePath, Patch};
use std::ops::Range;
use std::sync::Arc;

/// A node in a tree, with the way back to the root.
#[derive(Debug, Clone)]
pub struct Cursor {
    node: Node,
    parent: Option<Arc<Cursor>>,
    index: usize,
}

impl Cursor {
    /// The root element of `tree`.
    pub fn root(tree: &Tree) -> Cursor {
        Cursor { node: Node::Element(Arc::clone(tree.root())), parent: None, index: 0 }
    }

    /// The node.
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// The node as an element, if it is one.
    pub fn element(&self) -> Option<&Arc<Element>> {
        self.node.as_element()
    }

    /// The parent element's cursor; `None` at the root.
    pub fn parent(&self) -> Option<&Cursor> {
        self.parent.as_deref()
    }

    /// The position among the parent's children (0 for the root).
    pub fn index(&self) -> usize {
        self.index
    }

    /// The `index`th child.
    pub fn child(&self, index: usize) -> Option<Cursor> {
        let node = self.element()?.children().get(index)?.clone();
        Some(Cursor { node, parent: Some(Arc::new(self.clone())), index })
    }

    /// The children, in order.
    pub fn children(&self) -> ChildCursors {
        ChildCursors { parent: Arc::new(self.clone()), next: 0 }
    }

    /// The next sibling.
    pub fn next_sibling(&self) -> Option<Cursor> {
        self.parent()?.child(self.index + 1)
    }

    /// The previous sibling.
    pub fn prev_sibling(&self) -> Option<Cursor> {
        self.parent()?.child(self.index.checked_sub(1)?)
    }

    /// The path from the document to this node.
    pub fn path(&self) -> NodePath {
        let mut indices = vec![self.index];
        let mut at = self.parent();
        while let Some(c) = at {
            indices.push(c.index);
            at = c.parent();
        }
        indices.reverse();
        NodePath(indices)
    }

    /// A patch replacing this element's children in `range` by `inserted`; `None` when this
    /// is not an element or `range` is not within its children.
    pub fn splice(&self, range: Range<usize>, inserted: Vec<Node>) -> Option<Patch> {
        let children = self.element()?.children();
        let removed: Option<Vec<Node>> = range.clone().map(|i| children.get(i).cloned()).collect();
        Some(Patch::splice(self.path(), range.start, removed?, inserted))
    }

    /// A patch replacing this node by `with` (the root only by an element).
    pub fn replace(&self, with: Node) -> Option<Patch> {
        match &self.parent {
            Some(parent) => parent.splice(self.index..self.index + 1, vec![with]),
            None => match (&self.node, with) {
                (Node::Element(old), Node::Element(new)) => Some(Patch::replace_root(old, new)),
                _ => None,
            },
        }
    }
}

/// The children of a [`Cursor`], as cursors.
#[derive(Debug, Clone)]
pub struct ChildCursors {
    parent: Arc<Cursor>,
    next: usize,
}

impl Iterator for ChildCursors {
    type Item = Cursor;
    fn next(&mut self) -> Option<Cursor> {
        let node = self.parent.element()?.children().get(self.next)?.clone();
        let index = self.next;
        self.next += 1;
        Some(Cursor { node, parent: Some(Arc::clone(&self.parent)), index })
    }
}
