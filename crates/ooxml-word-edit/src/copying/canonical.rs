//! Owns: an element as lxml's inclusive canonical form writes it
//! (`etree.tostring(node, method="c14n")`, comments kept), which docx-agent's importer
//! compares formatting by (`_canonical`): every namespace in scope declared on the element
//! itself, sorted by prefix; a descendant's declarations where they differ from its
//! parent's; attributes sorted by namespace and name; text, comments and processing
//! instructions as they are, escaped as C14N escapes them.
//!
//! What is in scope depends on where the element is: in a part's tree, every declaration
//! above it ([`canonical`] with that scope); a detached copy as lxml's `copy.deepcopy`
//! makes it, only the namespaces its names use ([`canonical_copy`]).
//!
//! Must not: change anything; it reads elements only.

use crate::names::XML;
use ooxml_xml::{Element, NamespaceDecl, Node};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The namespaces in scope, by prefix (`""` for the default namespace).
type Scope = BTreeMap<String, String>;

/// `element`'s canonical form when the declarations `scope` (outermost first) are in scope
/// above it.
pub(crate) fn canonical(element: &Element, scope: &[NamespaceDecl]) -> String {
    let mut above = Scope::new();
    declare(&mut above, scope);
    let mut out = String::new();
    write_element(&mut out, element, &above, true);
    out
}

/// `element`'s canonical form as a detached copy of it has it (lxml's `copy.deepcopy`:
/// the namespaces its names use that nothing in it declares are declared on the copy).
pub(crate) fn canonical_copy(element: &Element) -> String {
    canonical(element, &used_undeclared(element))
}

/// Whether two detached copies are the same canonical XML.
pub(crate) fn same_copy(a: &Element, b: &Element) -> bool {
    canonical_copy(a) == canonical_copy(b)
}

fn declare(scope: &mut Scope, decls: &[NamespaceDecl]) {
    for d in decls {
        let prefix = d.prefix.as_deref().unwrap_or("").to_owned();
        scope.insert(prefix, d.uri.to_string());
    }
}

/// The namespaces `element`'s names (and its descendants') use that no declaration in it
/// makes, in the order first used.
fn used_undeclared(element: &Element) -> Vec<NamespaceDecl> {
    let mut declared: Vec<String> = Vec::new();
    let mut used: Vec<NamespaceDecl> = Vec::new();
    collect(element, &mut declared, &mut used);
    used.retain(|d| !declared.iter().any(|u| **u == *d.uri));
    let mut out: Vec<NamespaceDecl> = Vec::new();
    for d in used {
        if !out.iter().any(|o| o.prefix == d.prefix) {
            out.push(d);
        }
    }
    out
}

fn collect(element: &Element, declared: &mut Vec<String>, used: &mut Vec<NamespaceDecl>) {
    declared.extend(element.namespaces().iter().map(|d| d.uri.to_string()));
    let names = std::iter::once(element.name()).chain(element.attributes().iter().map(|a| &a.name));
    for name in names {
        let Some(uri) = name.namespace().filter(|u| *u != XML) else {
            continue;
        };
        let prefix = name.prefix();
        if !used.iter().any(|d| d.prefix.as_deref() == prefix && &*d.uri == uri) {
            used.push(NamespaceDecl {
                prefix: prefix.map(ooxml_xml::Atom::from),
                uri: ooxml_xml::Atom::from(uri),
            });
        }
    }
    for child in element.child_elements() {
        collect(child, declared, used);
    }
}

fn write_element(out: &mut String, element: &Element, parent: &Scope, apex: bool) {
    let mut scope = parent.clone();
    declare(&mut scope, element.namespaces());
    let name = element.name().qualified();
    out.push('<');
    out.push_str(name);
    for (prefix, uri) in &scope {
        let differs = apex || parent.get(prefix) != Some(uri);
        // the empty default namespace is written only to undeclare one in force above
        let undeclares = prefix.is_empty() && uri.is_empty();
        let render = if undeclares {
            !apex && parent.get(prefix).is_some_and(|u| !u.is_empty())
        } else {
            differs
        };
        if render {
            if prefix.is_empty() {
                let _ = write!(out, " xmlns=\"{}\"", escape_attribute(uri));
            } else {
                let _ = write!(out, " xmlns:{prefix}=\"{}\"", escape_attribute(uri));
            }
        }
    }
    let mut attributes: Vec<_> = element.attributes().iter().collect();
    attributes.sort_by(|a, b| {
        (a.name.namespace().unwrap_or(""), a.name.local()).cmp(&(b.name.namespace().unwrap_or(""), b.name.local()))
    });
    for attribute in attributes {
        let _ = write!(
            out,
            " {}=\"{}\"",
            attribute.name.qualified(),
            escape_attribute(&attribute.value)
        );
    }
    out.push('>');
    for child in element.children().iter() {
        match child {
            Node::Element(e) => write_element(out, e, &scope, false),
            Node::Text(t) => out.push_str(&escape_text(t)),
            Node::Comment(c) => {
                let _ = write!(out, "<!--{c}-->");
            }
            Node::Pi(p) if p.data.is_empty() => {
                let _ = write!(out, "<?{}?>", p.target);
            }
            Node::Pi(p) => {
                let _ = write!(out, "<?{} {}?>", p.target, p.data);
            }
            Node::EntityRef(r) => {
                let _ = write!(out, "&{r};");
            }
        }
    }
    let _ = write!(out, "</{name}>");
}

fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
    out
}

fn escape_attribute(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn parse(xml: &str) -> Arc<Element> {
        let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).expect("xml");
        Arc::clone(tree.root())
    }

    // Expected outputs written by lxml 6 (`etree.tostring(node, method="c14n")`).
    #[test]
    fn as_lxml_writes_an_element_in_its_tree() {
        let root = parse(
            "<w:styles xmlns:w=\"urn:w\" xmlns=\"urn:d\" xmlns:a=\"urn:a\"><w:x b=\"1\" w:a=\"&quot;2&#10;\" \
             a:c=\"3\"><!--c--><?pi d?>t&amp;&#13;<w:y xmlns:z=\"urn:z\" xmlns:a=\"urn:a\"/>tail</w:x>tl</w:styles>",
        );
        let x = root.child_elements().next().expect("x");
        assert_eq!(
            canonical(x, root.namespaces()),
            "<w:x xmlns=\"urn:d\" xmlns:a=\"urn:a\" xmlns:w=\"urn:w\" b=\"1\" a:c=\"3\" w:a=\"&quot;2&#xA;\">\
             <!--c--><?pi d?>t&amp;&#xD;<w:y xmlns:z=\"urn:z\"></w:y>tail</w:x>"
        );
    }

    #[test]
    fn a_copy_declares_what_it_uses() {
        let root = parse("<w:s xmlns:w=\"urn:w\" xmlns:a=\"urn:a\"><w:b w:val=\"0\"/></w:s>");
        let b = root.child_elements().next().expect("b");
        assert_eq!(canonical_copy(b), "<w:b xmlns:w=\"urn:w\" w:val=\"0\"></w:b>");
        assert!(canonical(b, root.namespaces()) != canonical_copy(b));
    }
}
