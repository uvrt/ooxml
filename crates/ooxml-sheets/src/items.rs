//! Owns: the element reading every part of a workbook shares -- SpreadsheetML children by
//! exact name, an element's text as lxml's `.text` holds it, and a string item's text (a
//! shared string's `si` or a cell's inline `is`).
//!
//! Must not: know what a cell, a sheet or a workbook is.

use crate::SML_NS;
use ooxml_xml::{Element, Node};

/// Whether `element` is SpreadsheetML's `local`.
pub(crate) fn is(element: &Element, local: &str) -> bool {
    element.name().is(Some(SML_NS), local)
}

/// The first child that is SpreadsheetML's `local`.
pub(crate) fn child<'a>(element: &'a Element, local: &str) -> Option<&'a Element> {
    element.child_elements().map(|e| &**e).find(|e| is(e, local))
}

/// Every child that is SpreadsheetML's `local`, in order.
pub(crate) fn children<'a>(element: &'a Element, local: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
    element.child_elements().map(|e| &**e).filter(move |e| is(e, local))
}

/// Every element below `element` that is SpreadsheetML's `local`, in document order (lxml's
/// `iter`, `element` itself left out).
pub(crate) fn descendants<'a>(element: &'a Element, local: &'a str) -> Vec<&'a Element> {
    let mut found = Vec::new();
    let mut stack = vec![element.child_elements()];
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(e) => {
                if is(e, local) {
                    found.push(&**e);
                }
                stack.push(e.child_elements());
            }
            None => {
                stack.pop();
            }
        }
    }
    found
}

/// The element's text as lxml's `.text` holds it: the text right after its start tag, before
/// any child node (element, comment or processing instruction); `None` when there is none.
pub(crate) fn leading_text(element: &Element) -> Option<String> {
    let mut out = String::new();
    for node in element.children() {
        match node {
            Node::Text(text) => out.push_str(text),
            _ => break,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// A string item's text: its plain `t`, or the `t` of its runs joined (a phonetic reading's
/// `t` left out).
pub(crate) fn item_text(item: &Element) -> String {
    if let Some(plain) = child(item, "t") {
        if child(item, "r").is_none() {
            return leading_text(plain).unwrap_or_default();
        }
    }
    let mut out = String::new();
    let mut stack = vec![(false, item.child_elements())];
    while let Some((in_run, top)) = stack.last_mut() {
        let in_run = *in_run;
        match top.next() {
            Some(e) => {
                if in_run && is(e, "t") {
                    out.push_str(&leading_text(e).unwrap_or_default());
                }
                stack.push((is(e, "r"), e.child_elements()));
            }
            None => {
                stack.pop();
            }
        }
    }
    out
}
