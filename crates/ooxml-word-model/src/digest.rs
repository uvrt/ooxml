//! Owns: content digests of what the model reads -- an element subtree (its names by
//! namespace and local name, attributes, text) and a part's tree -- so that a reopened
//! version finds the blocks and parts it read before by content, whatever parse made them.
//!
//! Must not: be used as a cryptographic hash; the key comes from the caller.

use ooxml_base::{Digest, DigestKey, Digester};
use ooxml_xml::{Element, Node};

/// A length-prefixed field.
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn element_head(out: &mut Vec<u8>, e: &Element) {
    out.push(b'<');
    field(out, e.name().namespace().unwrap_or("").as_bytes());
    field(out, e.name().local().as_bytes());
    for a in e.attributes() {
        out.push(b'@');
        field(out, a.name.namespace().unwrap_or("").as_bytes());
        field(out, a.name.local().as_bytes());
        field(out, a.value.as_bytes());
    }
    out.push(b'>');
}

/// `element`'s subtree as the bytes its digest is taken of: what the model can read of it
/// (comments, processing instructions and unexpanded references are not read; every field is length-prefixed, so
/// no two subtrees run together).
fn encode(out: &mut Vec<u8>, element: &Element) {
    let mut stack: Vec<ooxml_xml::Iter<'_>> = Vec::new();
    element_head(out, element);
    stack.push(element.children().iter());
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(Node::Element(child)) => {
                element_head(out, child);
                stack.push(child.children().iter());
            }
            Some(Node::Text(t)) => {
                out.push(b't');
                field(out, t.as_bytes());
            }
            Some(_) => {}
            None => {
                out.push(b'/');
                stack.pop();
            }
        }
    }
}

/// Feed `element`'s subtree to a digester.
pub fn feed(d: &mut Digester, element: &Element) {
    let mut buffer = Vec::with_capacity(1024);
    encode(&mut buffer, element);
    d.update(&buffer);
}

/// The digest of an element's subtree.
pub fn element(key: DigestKey, element: &Element) -> Digest {
    let mut d = Digester::new(key);
    feed(&mut d, element);
    d.finish()
}
