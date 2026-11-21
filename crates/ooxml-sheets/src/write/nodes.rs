//! Owns: the element changes the writer makes on immutable trees -- an attribute set or
//! dropped (a new one goes last, an existing one keeps its place, as lxml's `set` does), a
//! child put in, replaced or taken out, and a new SpreadsheetML element named in the
//! namespace and prefix its parent uses.
//!
//! Must not: know what a cell, a row or a sheet is.

use crate::SML_NS;
use ooxml_xml::{Atom, Attribute, Element, Head, Node, QName};
use std::sync::Arc;

/// `xml:space`'s namespace.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// `element` with the unqualified attribute `local` set to `value`.
pub(crate) fn with_attr(element: &Element, local: &str, value: &str) -> Element {
    with_attr_ns(element, None, local, value)
}

fn with_attr_ns(element: &Element, namespace: Option<(&str, &str)>, local: &str, value: &str) -> Element {
    let uri = namespace.map(|(uri, _)| uri);
    let mut attributes: Vec<Attribute> = element.attributes().to_vec();
    match attributes.iter_mut().find(|a| a.name.is(uri, local)) {
        Some(found) if &*found.value == value => return element.clone(),
        Some(found) => found.value = Atom::from(value),
        None => attributes.push(Attribute {
            name: Arc::new(QName::new(namespace.map(|(_, prefix)| prefix), local, uri)),
            value: Atom::from(value),
        }),
    }
    with_attributes(element, attributes)
}

/// `element` without the unqualified attribute `local`.
pub(crate) fn without_attr(element: &Element, local: &str) -> Element {
    without_attr_ns(element, None, local)
}

fn without_attr_ns(element: &Element, namespace: Option<&str>, local: &str) -> Element {
    if element.attribute(namespace, local).is_none() {
        return element.clone();
    }
    let attributes = element
        .attributes()
        .iter()
        .filter(|a| !a.name.is(namespace, local))
        .cloned()
        .collect();
    with_attributes(element, attributes)
}

fn with_attributes(element: &Element, attributes: Vec<Attribute>) -> Element {
    let head = element.head();
    element.with_head(Head {
        name: head.name.clone(),
        namespaces: head.namespaces.clone(),
        attributes: attributes.into(),
    })
}

/// `parent` with its child node at `index` replaced by `node`.
pub(crate) fn replace_child(parent: &Element, index: usize, node: Node) -> Element {
    splice(parent, index..index + 1, vec![node])
}

/// `parent` with `node` put in before its child node at `index` (at the end for its length).
pub(crate) fn insert_child(parent: &Element, index: usize, node: Node) -> Element {
    splice(parent, index..index, vec![node])
}

/// `parent` without its child node at `index`.
pub(crate) fn remove_child(parent: &Element, index: usize) -> Element {
    splice(parent, index..index + 1, Vec::new())
}

fn splice(parent: &Element, range: std::ops::Range<usize>, nodes: Vec<Node>) -> Element {
    let children = parent.children();
    // the callers give places among the children; a place past them changes nothing
    let spliced = children.splice(range, nodes).unwrap_or_else(|| children.clone());
    parent.with_children(spliced)
}

/// `parent` with `child` appended.
pub(crate) fn append_child(parent: &Element, child: Element) -> Element {
    insert_child(parent, parent.children().len(), Node::Element(Arc::new(child)))
}

/// A new SpreadsheetML element `local`, named with the prefix `parent` uses for the
/// namespace (an element lxml makes and then places takes the declaration in scope).
pub(crate) fn element(parent: &Element, local: &str, attributes: &[(&str, &str)], children: Vec<Node>) -> Element {
    let prefix = parent
        .name()
        .prefix()
        .filter(|_| parent.name().namespace() == Some(SML_NS));
    let attributes = attributes
        .iter()
        .map(|(name, value)| Attribute {
            name: Arc::new(QName::new(None, name, None)),
            value: Atom::from(*value),
        })
        .collect();
    Element::new(
        Arc::new(QName::new(prefix, local, Some(SML_NS))),
        Vec::new(),
        attributes,
        children,
    )
}

/// A text `t` under `parent`, holding `text`: `xml:space="preserve"` where Python's
/// `strip()` would change the text or it holds a line break (ooxml-edit's `_set_text`).
pub(crate) fn text_element(parent: &Element, text: &str) -> Element {
    let node = element(parent, "t", &[], text_nodes(text));
    let kept = text != text.trim_matches(ooxml_xml::view::py_space) || text.contains('\n');
    if kept {
        with_attr_ns(&node, Some((XML_NS, "xml")), "space", "preserve")
    } else {
        without_attr_ns(&node, Some(XML_NS), "space")
    }
}

/// An element's text as its children: one text node, empty or not (lxml writes an element
/// whose text was set to `""` as `<t></t>`, not `<t/>`).
pub(crate) fn text_nodes(text: &str) -> Vec<Node> {
    vec![Node::Text(Atom::from(text))]
}

/// The node of `parent` at `index` as an element.
pub(crate) fn element_at(parent: &Element, index: usize) -> Option<&Arc<Element>> {
    parent.children().get(index).and_then(Node::as_element)
}

/// The places of `parent`'s element children named SpreadsheetML's `local`, with them.
pub(crate) fn places<'a>(parent: &'a Element, local: &'a str) -> impl Iterator<Item = (usize, &'a Arc<Element>)> + 'a {
    parent
        .children()
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.as_element().map(|e| (i, e)))
        .filter(move |(_, e)| e.name().is(Some(SML_NS), local))
}
