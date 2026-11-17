//! Owns: how the model reads an element -- children and attributes by **local name**,
//! whatever the namespace (transitional and strict WordprocessingML alike, and the prefixes
//! a generator chose), integers as docx2svg reads them, and WordprocessingML's on/off
//! values. The generated views (`crate::w`) and every reader call these.
//!
//! Must not: know an element's meaning, or allocate where a borrow does.

use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// An element's local name.
pub fn local(element: &Element) -> &str {
    element.name().local()
}

/// Every child element, in order (text, comments and processing instructions skipped).
pub fn elements(element: &Element) -> impl Iterator<Item = &Arc<Element>> {
    element.child_elements()
}

/// Every child element with local name `name`, in order.
pub fn children<'a>(element: &'a Element, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
    element.child_elements().filter(move |e| local(e) == name).map(|e| &**e)
}

/// The first child element with local name `name`.
pub fn child<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.child_elements().find(|e| local(e) == name).map(|e| &**e)
}

/// An attribute by local name: one without a namespace first, then the first in document
/// order whose local name is `name` (ElementTree's `get` then a scan, as docx2svg reads).
pub fn attr<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    let attributes = element.attributes();
    attributes
        .iter()
        .find(|a| a.name.namespace().is_none() && a.name.local() == name)
        .or_else(|| attributes.iter().find(|a| a.name.local() == name))
        .map(|a| &*a.value)
}

/// An attribute in exactly namespace `namespace`.
pub fn attr_ns<'a>(element: &'a Element, namespace: &str, name: &str) -> Option<&'a str> {
    element.attribute(Some(namespace), name)
}

/// An integer as Python's `int()` reads a string: surrounding whitespace, an optional
/// sign, ASCII digits with single underscores between them. `None` for anything else or
/// a value outside `i64`.
pub fn py_int(text: &str) -> Option<i64> {
    let s = text.trim_matches(|c: char| c.is_whitespace());
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, s.get(1..)?),
        Some(b'+') => (false, s.get(1..)?),
        _ => (false, s),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let mut value: i64 = 0;
    for c in digits.chars() {
        if c == '_' {
            continue;
        }
        let d = i64::from(c.to_digit(10)?);
        value = value.checked_mul(10)?.checked_add(d)?;
    }
    Some(if negative { -value } else { value })
}

/// An integer attribute (docx2svg's `attr_int`): `None` where absent or not an integer.
pub fn attr_int(element: &Element, name: &str) -> Option<i64> {
    attr(element, name).and_then(py_int)
}

/// An `ST_OnOff` attribute (docx2svg's `attr_bool`): absent is on; else on for `1`,
/// `true`, `on` in any case.
pub fn attr_on(element: &Element, name: &str) -> bool {
    attr(element, name).is_none_or(is_on)
}

/// Whether an `ST_OnOff` value is on: `1`, `true` or `on`, in any case.
pub fn is_on(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "1" | "true" | "on")
}

/// A DrawingML boolean as docx2svg reads one: `None` where absent, else whether the value
/// is not `0`, `false` or `off` (as written).
pub fn attr_flag(element: &Element, name: &str) -> Option<bool> {
    attr(element, name).map(|v| !matches!(v, "0" | "false" | "off"))
}

/// An element's text as ElementTree's `.text` holds it: the text before its first child
/// element (comments and processing instructions, which ElementTree drops, skipped).
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

/// `text`, or `None` where the element holds none (ElementTree's `None`).
pub fn text_opt(element: &Element) -> Option<String> {
    let t = text(element);
    (!t.is_empty()).then_some(t)
}
