//! Owns: locating elements in a part as docx-agent's `live_elements` does -- by name, in
//! document order, `mc:Fallback` subtrees left out -- each with its place (a path of child
//! indexes, as [`crate::ids::PartIds`] keys blocks), and locating an element by its place.
//!
//! Must not: know what the elements mean.

use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// An element found, with its place in the part.
#[derive(Debug, Clone)]
pub struct Found {
    /// The element.
    pub element: Arc<Element>,
    /// Its place: child indexes from the root.
    pub path: Box<[u32]>,
}

/// The live elements below `root` (not `root` itself) that `wanted` accepts, in document
/// order, with their places; the branches of an `mc:AlternateContent` the readers do not
/// read skipped ([`crate::mc::chosen`]). `base` is `root`'s own place.
pub fn live_paths(root: &Element, base: &[u32], wanted: &dyn Fn(&Element) -> bool) -> Vec<Found> {
    let mut out = Vec::new();
    let mut path = base.to_vec();
    walk(root, &mut path, wanted, &mut out);
    out
}

fn walk(element: &Element, path: &mut Vec<u32>, wanted: &dyn Fn(&Element) -> bool, out: &mut Vec<Found>) {
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        if crate::mc::unchosen(element, child) {
            continue;
        }
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        if wanted(child) {
            out.push(Found {
                element: Arc::clone(child),
                path: path.clone().into_boxed_slice(),
            });
        }
        walk(child, path, wanted, out);
        path.pop();
    }
}

/// The element at `path` below `root`.
#[must_use]
pub fn at_path<'a>(root: &'a Arc<Element>, path: &[u32]) -> Option<&'a Arc<Element>> {
    let mut node = root;
    for k in path {
        node = node.children().get(*k as usize)?.as_element()?;
    }
    Some(node)
}
