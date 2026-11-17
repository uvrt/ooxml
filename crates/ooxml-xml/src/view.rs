//! Owns: what generated typed views (design ADR-005) call to read and build elements --
//! children and attributes found by **local name**, whatever the namespace (as the Python
//! readers find them, so a strict-namespace part reads as a transitional one); numbers taken
//! from attribute values as Python's `float()` takes them; and a child inserted at the place
//! a schema sequence ranks it.
//!
//! Must not: know any OOXML name or namespace (a view passes them in), or decide what a value
//! means beyond its lexical form.

use crate::{Children, Element, Node};
use std::sync::Arc;

/// An element's local name.
pub fn local(element: &Element) -> &str {
    element.name().local()
}

/// Every child element, in order (text, comments and processing instructions skipped).
pub fn elements(element: &Element) -> impl Iterator<Item = &Element> {
    element.child_elements().map(|e| &**e)
}

/// Every child element with local name `name`, in order.
pub fn children<'a>(element: &'a Element, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
    elements(element).filter(move |e| local(e) == name)
}

/// The first child element with local name `name`.
pub fn child<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    elements(element).find(|e| local(e) == name)
}

/// An attribute by local name: one without a namespace first, then the first in document
/// order whose local name is `name` (ElementTree's `get`, then a scan).
pub fn attr<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    let attributes = element.attributes();
    attributes
        .iter()
        .find(|a| a.name.namespace().is_none() && a.name.local() == name)
        .or_else(|| attributes.iter().find(|a| a.name.local() == name))
        .map(|a| &*a.value)
}

/// An attribute in exactly the namespace `namespace`.
pub fn attr_ns<'a>(element: &'a Element, namespace: &str, name: &str) -> Option<&'a str> {
    element.attribute(Some(namespace), name)
}

/// A numeric attribute: `None` where absent, not a number, or not finite.
pub fn attr_num(element: &Element, name: &str) -> Option<f64> {
    attr(element, name).and_then(py_float).filter(|v| v.is_finite())
}

/// The element's text as ElementTree's `.text` holds it: the text before its first child
/// element (comments and processing instructions skipped).
pub fn text(element: &Element) -> String {
    let mut out = String::new();
    for node in element.children() {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Element(_) => break,
            _ => {}
        }
    }
    out
}

/// Whether `c` is whitespace to Python's `str.strip()`: Unicode's White_Space, plus the
/// four information separators (U+001C to U+001F) Python counts as well.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Whether `digits` is one or more ASCII digits with single underscores between them.
fn digit_part(digits: &str) -> bool {
    !digits.is_empty()
        && !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits.chars().all(|c| c.is_ascii_digit() || c == '_')
}

/// A number's body (after its sign) is `digits [. digits] [e [sign] digits]`, either side of
/// the point possibly empty but not both.
fn numeric_body(body: &str) -> bool {
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (body.get(..i).unwrap_or(""), Some(body.get(i + 1..).unwrap_or(""))),
        None => (body, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some((whole, fraction)) => {
            (!whole.is_empty() || !fraction.is_empty())
                && (whole.is_empty() || digit_part(whole))
                && (fraction.is_empty() || digit_part(fraction))
        }
        None => digit_part(mantissa),
    };
    let exponent_ok = exponent.is_none_or(|e| digit_part(e.strip_prefix(['+', '-']).unwrap_or(e)));
    mantissa_ok && exponent_ok
}

/// `text` as Python's `float(text)` reads it: surrounding whitespace, an optional sign, a
/// decimal number with optional exponent and single underscores between digits, or
/// `inf`, `infinity` or `nan` in any case. Correctly rounded, as both languages round.
pub fn py_float(text: &str) -> Option<f64> {
    let s = text.trim_matches(py_space);
    let (negative, body) = match s.as_bytes().first() {
        Some(b'-') => (true, s.get(1..)?),
        Some(b'+') => (false, s.get(1..)?),
        _ => (false, s),
    };
    let lower = body.to_ascii_lowercase();
    let magnitude = match lower.as_str() {
        "inf" | "infinity" => f64::INFINITY,
        "nan" => f64::NAN,
        _ if numeric_body(body) => body.replace('_', "").parse::<f64>().ok()?,
        _ => return None,
    };
    Some(if negative { -magnitude } else { magnitude })
}

/// The rank of the child named `name` in `order` (a schema sequence, each entry the local
/// names of one step: a choice lists several).
fn rank(order: &[&[&str]], name: &str) -> Option<usize> {
    order.iter().position(|step| step.contains(&name))
}

/// `parent` with `inserted` added where `order` puts it: after every sibling ranked at or
/// before it and before the first ranked after it; at the end when `order` does not name it
/// (an extension element). Siblings `order` does not name rank last, as an `extLst` does.
pub fn insert_ordered(parent: &Element, order: &[&[&str]], inserted: Element) -> Element {
    let nodes = parent.children();
    let index = match rank(order, local(&inserted)) {
        Some(own) => nodes
            .iter()
            .position(|node| {
                node.as_element()
                    .is_some_and(|e| rank(order, local(e)).is_none_or(|other| other > own))
            })
            .unwrap_or(nodes.len()),
        None => nodes.len(),
    };
    let node = Node::Element(Arc::new(inserted));
    // `index` is at most the length, where a splice cannot fail; the fallback is never taken.
    let spliced = nodes.splice(index..index, vec![node.clone()]).unwrap_or_else(|| {
        let mut all = nodes.to_vec();
        all.push(node);
        Children::from_vec(all)
    });
    parent.with_children(spliced)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_python_reads_them() {
        assert_eq!(py_float(" 12 "), Some(12.0));
        assert_eq!(py_float("1_000.5"), Some(1000.5));
        assert_eq!(py_float("-.5e1"), Some(-5.0));
        assert_eq!(py_float("1."), Some(1.0));
        assert_eq!(py_float("+1E+2"), Some(100.0));
        assert_eq!(py_float("Infinity"), Some(f64::INFINITY));
        assert!(py_float("nan").is_some_and(f64::is_nan));
        for bad in ["", ".", "1_", "_1", "1__0", "1._5", "e5", "1e", "0x10", "1 2", "--1"] {
            assert_eq!(py_float(bad), None, "{bad:?}");
        }
    }
}
