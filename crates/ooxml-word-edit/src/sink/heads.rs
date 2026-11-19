//! Owns: what the sink's writes compute from a tree without writing: a root's head with
//! the namespaces an edit needs declared ([`ensured_head`]), and the draws that number a
//! write's revision records again where its change is applied ([`annotation_draws`]).
//!
//! Must not: write (the sink does).

use super::Numbered;
use crate::doc::{Draw, DrawFrom};
use crate::names::{MC, W14, WP14};
use ooxml_xml::{Attribute, Element, Head, NamespaceDecl, Node, QName};
use std::sync::Arc;

/// `root`'s head with `w14` (`wp14`, `extra`) declared and listed as ignorable; `None` when
/// it has them already. New declarations go after the root's own, `mc` last when it is
/// added; the prefixes are added to the end of `mc:Ignorable`.
pub(crate) fn ensured_head(root: &Element, drawings: bool, extra: &[(&str, &str)]) -> Option<(Head, bool)> {
    let mut wanted: Vec<(&str, &str)> = vec![("w14", W14)];
    if drawings {
        wanted.push(("wp14", WP14));
    }
    wanted.extend_from_slice(extra);
    let declared = |namespaces: &[NamespaceDecl], uri: &str| {
        namespaces
            .iter()
            .find(|d| &*d.uri == uri && d.prefix.as_deref().is_some_and(|p| !p.is_empty()))
            .and_then(|d| d.prefix.as_deref().map(str::to_owned))
    };
    let mut namespaces = root.namespaces().to_vec();
    let mut changed = false;
    let mut found: Vec<String> = Vec::new();
    let missing_any = wanted.iter().any(|(_, uri)| declared(&namespaces, uri).is_none());
    let mc_missing = declared(&namespaces, MC).is_none();
    if missing_any || mc_missing {
        for (name, uri) in &wanted {
            // lxml's cleanup declares at the top only what the tree uses
            if declared(&namespaces, uri).is_none() && uses_namespace(root, uri) {
                namespaces.push(NamespaceDecl {
                    prefix: Some(Arc::from(*name)),
                    uri: Arc::from(*uri),
                });
                changed = true;
            }
        }
        if mc_missing && root.child_elements().next().is_some() {
            namespaces.push(NamespaceDecl {
                prefix: Some(Arc::from("mc")),
                uri: Arc::from(MC),
            });
            changed = true;
        }
    }
    for (name, uri) in &wanted {
        found.push(declared(&namespaces, uri).unwrap_or_else(|| (*name).to_owned()));
    }
    let mc = declared(&namespaces, MC).unwrap_or_else(|| "mc".to_owned());
    let mut attributes = root.attributes().to_vec();
    let listed: Vec<String> = root
        .attribute(Some(MC), "Ignorable")
        .map(|v| v.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    let missing: Vec<&String> = found.iter().filter(|p| !listed.contains(p)).collect();
    if !missing.is_empty() {
        let value: Vec<&str> = listed
            .iter()
            .map(String::as_str)
            .chain(missing.iter().map(|s| s.as_str()))
            .collect();
        let value: Arc<str> = Arc::from(value.join(" "));
        match attributes.iter_mut().find(|a| a.name.is(Some(MC), "Ignorable")) {
            Some(a) => a.value = value,
            None => attributes.push(Attribute {
                name: Arc::new(QName::new(Some(&mc), "Ignorable", Some(MC))),
                value,
            }),
        }
        changed = true;
    }
    let declared_new = namespaces.len() != root.namespaces().len();
    changed.then(|| {
        let head = Head {
            name: root.name().clone(),
            namespaces: namespaces.into(),
            attributes: attributes.into(),
        };
        (head, declared_new)
    })
}

/// Whether an element or attribute at or below `element` is in the namespace `uri`.
fn uses_namespace(element: &Element, uri: &str) -> bool {
    element.name().namespace() == Some(uri)
        || element.attributes().iter().any(|a| a.name.namespace() == Some(uri))
        || element.child_elements().any(|c| uses_namespace(c, uri))
}

/// The draws that number again, where the change is applied, the revision records a
/// write made in `element` (`w:id` among the stand-ins it gave), each by its offset from
/// the edit's first id; paths from `element`.
pub(crate) fn annotation_draws(element: &Element, numbered: Numbered, node: u32) -> Vec<Draw> {
    let mut out = Vec::new();
    let mut stack: Vec<(&Element, Vec<u32>)> = vec![(element, Vec::new())];
    while let Some((e, path)) = stack.pop() {
        if e.name().namespace() == Some(crate::names::W) && crate::revision_ids::is_record(e) {
            let value = e
                .attribute(Some(crate::names::W), "id")
                .and_then(|v| v.trim().parse::<i64>().ok())
                .filter(|v| (numbered.from..numbered.to).contains(v));
            if let Some(value) = value {
                if let (Ok(offset), Some(attribute)) = (
                    u32::try_from(value - numbered.base),
                    e.attributes().iter().find(|a| a.name.is(Some(crate::names::W), "id")),
                ) {
                    out.push(Draw {
                        from: DrawFrom::Annotation(offset),
                        node,
                        at: path.clone(),
                        attribute: Arc::clone(&attribute.name),
                    });
                }
            }
        }
        for (k, node) in e.children().iter().enumerate() {
            if let Node::Element(child) = node {
                let mut below = path.clone();
                below.push(u32::try_from(k).unwrap_or(u32::MAX));
                stack.push((child, below));
            }
        }
    }
    out
}
