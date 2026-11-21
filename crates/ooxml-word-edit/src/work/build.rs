//! Owns: a [`Work`] given back as green nodes ([`Work::build`], [`Work::build_content`]):
//! every node that did not change is the version's own; a sealed one changed is no node.
//!
//! Must not: change the work.

use super::{Id, Kind, Work};
use ooxml_xml::{Atom, Element, Node};
use std::sync::Arc;

impl Work {
    /// Whether anything at or below the root changed.
    pub(crate) fn changed(&self) -> bool {
        self.slot(self.root).is_some_and(|s| s.changed)
    }

    /// The root as a green element: unchanged subtrees are the version's own nodes.
    pub(crate) fn build(&self) -> Option<Arc<Element>> {
        match self.build_node(self.root) {
            Some(Node::Element(e)) => Some(e),
            _ => None,
        }
    }

    /// The element `id` built, detached (its tail left out).
    pub(crate) fn build_element(&self, id: Id) -> Option<Arc<Element>> {
        match self.build_node(id) {
            Some(Node::Element(e)) => Some(e),
            _ => None,
        }
    }

    /// The element's content as green nodes (its text, its children, their tails): what
    /// replaces a stretch of a parent's children when the root stands for that stretch.
    pub(crate) fn build_content(&self, id: Id) -> Option<Vec<Node>> {
        let slot = self.slot(id)?;
        let mut out = Vec::new();
        if !self.text(id).is_empty() {
            out.push(Node::Text(Atom::from(self.text(id))));
        }
        for child in &slot.children {
            out.push(self.build_node(*child)?);
            let tail = self.tail(*child);
            if !tail.is_empty() {
                out.push(Node::Text(Atom::from(tail)));
            }
        }
        Some(out)
    }

    fn build_node(&self, id: Id) -> Option<Node> {
        let slot = self.slot(id)?;
        if !slot.changed {
            if let Some(origin) = &slot.origin {
                return Some(origin.clone());
            }
        }
        if slot.sealed {
            // its content was never read: built changed, it would lose it
            return None;
        }
        Some(match &slot.kind {
            Kind::Element {
                name,
                namespaces,
                attributes,
                text,
            } => {
                let mut children = Vec::with_capacity(slot.children.len() * 2 + 1);
                if !text.is_empty() {
                    children.push(Node::Text(Atom::from(text.as_str())));
                }
                for child in &slot.children {
                    children.push(self.build_node(*child)?);
                    let tail = self.tail(*child);
                    if !tail.is_empty() {
                        children.push(Node::Text(Atom::from(tail)));
                    }
                }
                Node::Element(Arc::new(Element::new(
                    name.clone(),
                    namespaces.clone(),
                    attributes.clone(),
                    children,
                )))
            }
            Kind::Comment(c) => Node::Comment(Arc::clone(c)),
            Kind::Pi(p) => Node::Pi(Arc::clone(p)),
            Kind::Entity(r) => Node::EntityRef(Arc::clone(r)),
        })
    }
}
