//! Owns: namespace declarations as docx-agent leaves them when it declares Word 2010's
//! namespaces on a part (`ids.ensure_w14`, lxml's `cleanup_namespaces` with a top map):
//! the declarations added at the root, and every declaration below that repeats one in
//! scope removed, its names taking the declaration in scope ([`strip_redundant`], what
//! lxml does to a subtree it moves).
//!
//! Must not: remove a declaration a name below needs, or know any Word rule.

use ooxml_xml::{Element, Name, NamespaceDecl, Node, QName};
use std::sync::Arc;

/// A prefix that goes, the prefix its names take instead, and their namespace.
type Remap = (Option<Arc<str>>, Option<Arc<str>>, Arc<str>);

/// `element` (whose ancestors declare `scope`, outermost first) with every declaration
/// that repeats one in scope removed; unchanged subtrees shared.
#[must_use]
pub fn strip_redundant(element: &Arc<Element>, scope: &mut Vec<NamespaceDecl>) -> Arc<Element> {
    let found =
        |scope: &[NamespaceDecl], uri: &str| scope.iter().rev().find(|d| &*d.uri == uri).map(|d| d.prefix.clone());
    let mut keep = Vec::new();
    let mut remap: Vec<Remap> = Vec::new();
    for d in element.namespaces() {
        match found(scope, &d.uri) {
            Some(prefix) if prefix.is_some() || d.prefix.is_none() => {
                if prefix != d.prefix {
                    remap.push((d.prefix.clone(), prefix, Arc::clone(&d.uri)));
                }
            }
            _ => keep.push(d.clone()),
        }
    }
    let pushed = keep.len();
    scope.extend(keep.iter().cloned());
    let mut changed = keep.len() != element.namespaces().len();
    let mut children = Vec::with_capacity(element.children().len());
    for node in element.children().iter() {
        match node {
            Node::Element(child) => {
                let new = strip_redundant(child, scope);
                changed |= !Arc::ptr_eq(&new, child);
                children.push(Node::Element(new));
            }
            other => children.push(other.clone()),
        }
    }
    scope.truncate(scope.len() - pushed);
    if !changed {
        return Arc::clone(element);
    }
    let rename = |n: &Name| -> Name {
        remap
            .iter()
            .find(|(from, _, uri)| n.namespace() == Some(&**uri) && n.prefix() == from.as_deref())
            .map_or_else(
                || Arc::clone(n),
                |(_, to, uri)| Arc::new(QName::new(to.as_deref(), n.local(), Some(uri))),
            )
    };
    let attributes = element
        .attributes()
        .iter()
        .map(|a| ooxml_xml::Attribute {
            name: rename(&a.name),
            value: Arc::clone(&a.value),
        })
        .collect();
    let mut out = Element::new(rename(element.name()), keep, attributes, children);
    if remap.is_empty() {
        out = Element::from_head(
            Arc::new(ooxml_xml::Head {
                name: element.name().clone(),
                namespaces: out.namespaces().into(),
                attributes: element.attributes().into(),
            }),
            out.children().clone(),
        );
    }
    Arc::new(out)
}

/// A root whose head `ensure_w14` changed by declaring namespaces: every declaration below
/// that repeats one in scope removed (lxml's `cleanup_namespaces` with a top map).
#[must_use]
pub fn cleaned(root: &Arc<Element>) -> Arc<Element> {
    let mut scope = root.namespaces().to_vec();
    let mut children = Vec::with_capacity(root.children().len());
    let mut changed = false;
    for node in root.children().iter() {
        match node {
            Node::Element(child) => {
                let new = strip_redundant(child, &mut scope);
                changed |= !Arc::ptr_eq(&new, child);
                children.push(Node::Element(new));
            }
            other => children.push(other.clone()),
        }
    }
    if !changed {
        return Arc::clone(root);
    }
    Arc::new(root.with_children(ooxml_xml::Children::from_vec(children)))
}
