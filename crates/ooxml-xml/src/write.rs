//! Owns: the write contract for a touched part (ADR-004) -- the bytes libxml2's xmlsave
//! writes for `etree.tostring(tree, xml_declaration=True, encoding="UTF-8",
//! standalone=True)`, which is what `ooxml_edit.xml.serialize` writes. The algorithm, so
//! that no library defines it:
//!
//! 1. The declaration `<?xml version='1.0' encoding='UTF-8' standalone='yes'?>` and a line
//!    feed, whatever the part's own declaration said (or if it had none).
//! 2. A document type declaration, if any: `<!DOCTYPE name`, then ` PUBLIC "p" "s"` or
//!    ` SYSTEM "s"` (a literal holding `"` and no `'` in single quotes), then `>` and a line
//!    feed.
//! 3. Comments and processing instructions before the root, with nothing between them; the
//!    root; those after the root, with nothing between them. No trailing line feed.
//! 4. An element: `<` and its name as written; each namespace declaration
//!    (` xmlns="u"` / ` xmlns:p="u"`), then each attribute (` n="v"`), in order; `/>` when
//!    it has no children, else `>`, the children and `</name>`.
//! 5. Text with `&`, `<`, `>` and carriage return escaped (`&amp;` `&lt;` `&gt;` `&#13;`);
//!    attribute values and namespace URIs with `&` `<` `>` `"` and line feed, carriage
//!    return and tab escaped (`&quot;` `&#10;` `&#13;` `&#9;`). Everything else, non-ASCII
//!    included, as UTF-8.
//! 6. A comment `<!--c-->`; a processing instruction `<?target data?>` (`<?target?>` with no
//!    data); an entity reference `&name;`.
//!
//! Must not: write any other form, pretty-print, or depend on how the part was read.

use crate::node::{Doctype, Element, ExternalId, Node, Tree};
use crate::rope::Iter;

const DECLARATION: &[u8] = b"<?xml version='1.0' encoding='UTF-8' standalone='yes'?>\n";

impl Tree {
    /// The part's bytes in the write contract's form.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write_to(&mut out);
        out
    }

    /// Append the part's bytes to `out`.
    pub fn write_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(DECLARATION);
        if let Some(d) = self.doctype() {
            write_doctype(d, out);
        }
        self.prolog().iter().for_each(|n| write_leaf(n, out));
        write_element(self.root(), out);
        self.epilog().iter().for_each(|n| write_leaf(n, out));
    }
}

fn write_doctype(d: &Doctype, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<!DOCTYPE ");
    out.extend_from_slice(d.name.as_bytes());
    match &d.external {
        Some(ExternalId::Public(public, system)) => {
            out.extend_from_slice(b" PUBLIC ");
            write_quoted(public, out);
            out.push(b' ');
            write_quoted(system, out);
        }
        Some(ExternalId::System(system)) => {
            out.extend_from_slice(b" SYSTEM ");
            write_quoted(system, out);
        }
        None => {}
    }
    out.extend_from_slice(b">\n");
}

/// libxml2's `xmlBufWriteQuotedString`.
fn write_quoted(s: &str, out: &mut Vec<u8>) {
    if s.contains('"') {
        if s.contains('\'') {
            out.push(b'"');
            out.extend_from_slice(s.replace('"', "&quot;").as_bytes());
            out.push(b'"');
        } else {
            out.push(b'\'');
            out.extend_from_slice(s.as_bytes());
            out.push(b'\'');
        }
    } else {
        out.push(b'"');
        out.extend_from_slice(s.as_bytes());
        out.push(b'"');
    }
}

/// Write `s` with the bytes `escape` maps replaced.
fn write_escaped(s: &str, out: &mut Vec<u8>, escape: fn(u8) -> Option<&'static [u8]>) {
    let bytes = s.as_bytes();
    let mut last = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if let Some(r) = escape(b) {
            out.extend_from_slice(bytes.get(last..i).unwrap_or_default());
            out.extend_from_slice(r);
            last = i + 1;
        }
    }
    out.extend_from_slice(bytes.get(last..).unwrap_or_default());
}

fn text_escape(b: u8) -> Option<&'static [u8]> {
    match b {
        b'&' => Some(b"&amp;"),
        b'<' => Some(b"&lt;"),
        b'>' => Some(b"&gt;"),
        b'\r' => Some(b"&#13;"),
        _ => None,
    }
}

fn attribute_escape(b: u8) -> Option<&'static [u8]> {
    match b {
        b'&' => Some(b"&amp;"),
        b'<' => Some(b"&lt;"),
        b'>' => Some(b"&gt;"),
        b'"' => Some(b"&quot;"),
        b'\n' => Some(b"&#10;"),
        b'\r' => Some(b"&#13;"),
        b'\t' => Some(b"&#9;"),
        _ => None,
    }
}

fn write_attribute(name: &[u8], value: &str, out: &mut Vec<u8>) {
    out.push(b' ');
    out.extend_from_slice(name);
    out.extend_from_slice(b"=\"");
    write_escaped(value, out, attribute_escape);
    out.push(b'"');
}

/// A node that is not an element.
fn write_leaf(node: &Node, out: &mut Vec<u8>) {
    match node {
        Node::Element(e) => write_element(e, out),
        Node::Text(t) => write_escaped(t, out, text_escape),
        Node::Comment(c) => {
            out.extend_from_slice(b"<!--");
            out.extend_from_slice(c.as_bytes());
            out.extend_from_slice(b"-->");
        }
        Node::Pi(p) => {
            out.extend_from_slice(b"<?");
            out.extend_from_slice(p.target.as_bytes());
            if !p.data.is_empty() {
                out.push(b' ');
                out.extend_from_slice(p.data.as_bytes());
            }
            out.extend_from_slice(b"?>");
        }
        Node::EntityRef(name) => {
            out.push(b'&');
            out.extend_from_slice(name.as_bytes());
            out.push(b';');
        }
    }
}

/// Write the start tag; whether the element has children (then the end tag is owed).
fn write_start(e: &Element, out: &mut Vec<u8>) -> bool {
    out.push(b'<');
    out.extend_from_slice(e.name().qualified().as_bytes());
    for ns in e.namespaces() {
        match &ns.prefix {
            Some(p) => {
                out.extend_from_slice(b" xmlns:");
                out.extend_from_slice(p.as_bytes());
            }
            None => out.extend_from_slice(b" xmlns"),
        }
        out.extend_from_slice(b"=\"");
        write_escaped(&ns.uri, out, attribute_escape);
        out.push(b'"');
    }
    for a in e.attributes() {
        write_attribute(a.name.qualified().as_bytes(), &a.value, out);
    }
    if e.children().is_empty() {
        out.extend_from_slice(b"/>");
        false
    } else {
        out.push(b'>');
        true
    }
}

fn write_end(e: &Element, out: &mut Vec<u8>) {
    out.extend_from_slice(b"</");
    out.extend_from_slice(e.name().qualified().as_bytes());
    out.push(b'>');
}

/// An element and everything in it, with an explicit stack (no recursion).
fn write_element(root: &Element, out: &mut Vec<u8>) {
    let mut stack: Vec<(&Element, Iter<'_>)> = Vec::new();
    if write_start(root, out) {
        stack.push((root, root.children().iter()));
    }
    while let Some((element, children)) = stack.last_mut() {
        match children.next() {
            Some(Node::Element(child)) => {
                if write_start(child, out) {
                    stack.push((child, child.children().iter()));
                }
            }
            Some(leaf) => write_leaf(leaf, out),
            None => {
                write_end(element, out);
                stack.pop();
            }
        }
    }
}
