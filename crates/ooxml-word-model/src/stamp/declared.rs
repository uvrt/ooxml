//! Owns: the namespaces a part's root declares for Word 2010's ids, as docx-agent's
//! `ensure_w14` declares them: `w14` (and `wp14`, and any `extra`) declared at the root where
//! the tree uses them, `mc` where it is missing, and each prefix listed in `mc:Ignorable`
//! ([`ensured_head`]).
//!
//! Must not: write a part, or declare a namespace the tree does not use.

use crate::ids::{MC, W14, WP14};
use ooxml_xml::{Attribute, Element, Head, NamespaceDecl, QName};
use std::sync::Arc;

/// `root`'s head with `w14` (`wp14`, `extra`) declared and listed as ignorable; `None` when
/// it has them already. New declarations go after the root's own, `mc` last when it is
/// added; the prefixes are added to the end of `mc:Ignorable`.
pub fn ensured_head(root: &Element, drawings: bool, extra: &[(&str, &str)]) -> Option<(Head, bool)> {
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
