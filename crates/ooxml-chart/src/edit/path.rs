//! Owns: path copies of an immutable tree -- an element found by the places of its
//! ancestors among their parents' children, replaced there with every ancestor copied and
//! every other subtree shared -- and the one-child changes an edit makes (a child replaced,
//! put in, appended or taken out).
//!
//! Must not: know what a chart is.

use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// `parent`'s element children with their places among all its children.
pub(crate) fn indexed(parent: &Element) -> impl Iterator<Item = (usize, &Element)> {
    parent
        .children()
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.as_element().map(|e| (i, &**e)))
}

/// The element at `path` below `root`.
pub(crate) fn at<'a>(root: &'a Element, path: &[usize]) -> Option<&'a Element> {
    let mut element = root;
    for &place in path {
        element = element.children().get(place).and_then(Node::as_element)?;
    }
    Some(element)
}

/// `root` with the element at `path` replaced by what `change` makes of it (`root` as it is
/// where the path leads nowhere: the callers found the path in this tree).
pub(crate) fn update(root: &Element, path: &[usize], change: impl FnOnce(&Element) -> Element) -> Element {
    let Some((&first, rest)) = path.split_first() else {
        return change(root);
    };
    let Some(child) = root.children().get(first).and_then(Node::as_element) else {
        return root.clone();
    };
    replaced(root, first, update(child, rest, change))
}

/// `parent` with its child at `place` replaced by `child`.
pub(crate) fn replaced(parent: &Element, place: usize, child: Element) -> Element {
    splice(parent, place..place + 1, vec![Node::Element(Arc::new(child))])
}

/// `parent` with `child` put in before its child at `place`.
pub(crate) fn inserted(parent: &Element, place: usize, child: Element) -> Element {
    splice(parent, place..place, vec![Node::Element(Arc::new(child))])
}

/// `parent` with `child` last.
pub(crate) fn appended(parent: &Element, child: Element) -> Element {
    let end = parent.children().len();
    inserted(parent, end, child)
}

/// `parent` without its child at `place` (the text around it kept, as ooxml-edit's
/// `remove` keeps a tail).
pub(crate) fn without(parent: &Element, place: usize) -> Element {
    splice(parent, place..place + 1, Vec::new())
}

fn splice(parent: &Element, range: std::ops::Range<usize>, nodes: Vec<Node>) -> Element {
    let children = parent.children();
    // the places come from this parent's children; a place past them changes nothing
    let spliced = children.splice(range, nodes).unwrap_or_else(|| children.clone());
    parent.with_children(spliced)
}
