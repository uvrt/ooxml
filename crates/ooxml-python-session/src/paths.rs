//! Owns: `ooxml_core.element_path` -- where the element a layout path names (an SVG
//! element's `data-docx-path`) is in a part's XML, as the Word model resolves it
//! (`ooxml_word_model::read::paths`): docx2svg's `resolve_path`, which walks a Python tree by
//! the indexes it returns.
//!
//! Must not: hold the part, or answer for a path the part does not hold.

use ooxml_word_model::read::paths::resolve_path;
use ooxml_xml::{Element, Limits, Node};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::sync::Arc;

/// The element `path` names in the part `xml`, as element-child indexes from its root (each
/// the place among its parent's child elements, every other node left out), or `None` for a
/// path the part does not hold (an edited tree, a synthetic step).
///
/// # Errors
/// `ValueError` where `xml` is not well-formed.
#[pyfunction]
pub(crate) fn element_path(py: Python<'_>, xml: &[u8], path: &str) -> PyResult<Option<Vec<usize>>> {
    let xml = xml.to_owned();
    let path = path.to_owned();
    let found = py.detach(|| {
        let tree = ooxml_xml::parse(&xml, &Limits::default()).map_err(|e| e.to_string())?;
        let root = Arc::clone(tree.root());
        Ok::<_, String>(resolve_path(&root, &path).map(|(_, place)| element_indexes(&root, &place)))
    });
    found.map(Option::flatten).map_err(PyValueError::new_err)
}

/// `place` (child indexes from `root`, every node counted) as element-child indexes; `None`
/// where a step is not an element's.
fn element_indexes(root: &Arc<Element>, place: &[u32]) -> Option<Vec<usize>> {
    let mut node = root;
    let mut out = Vec::with_capacity(place.len());
    for &k in place {
        let k = usize::try_from(k).ok()?;
        let children = node.children();
        let before = children
            .iter()
            .take(k)
            .filter(|c| matches!(c, Node::Element(_)))
            .count();
        let Some(Node::Element(child)) = children.iter().nth(k) else {
            return None;
        };
        out.push(before);
        node = child;
    }
    Some(out)
}
