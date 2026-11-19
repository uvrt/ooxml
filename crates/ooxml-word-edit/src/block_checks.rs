//! Owns: what deleting a block reads of the part before anything changes, as docx-agent's
//! `delete_block` reads it: whether a paragraph ends a section, holds a note or comment
//! reference -- and whether those references are a block comment's, which move on
//! (`comments.block_references`) -- whether its container keeps a paragraph to end on
//! (`_check_container_survives`), whether range markers in it have a partner outside it
//! (`_rehome_markers`), and the paragraphs around it in document order.
//!
//! Must not: change anything, or read beyond the part it is given.

use crate::across::PAIRS;
use crate::names::W;
use crate::{EditError, Result};
use ooxml_word_model::docx::at_path;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// Whether `element` is `w:{local}`.
fn is(element: &Element, local: &str) -> bool {
    element.name().is(Some(W), local)
}

/// Whether a paragraph's properties hold a section break.
pub(crate) fn ends_section(paragraph: &Element) -> bool {
    paragraph
        .child_elements()
        .find(|c| is(c, "pPr"))
        .is_some_and(|p| p.child_elements().any(|c| is(c, "sectPr")))
}

/// The elements at or below `element`, in document order, each with its parent.
fn walk<'e>(element: &'e Element, parent: Option<&'e Element>, out: &mut Vec<(&'e Element, Option<&'e Element>)>) {
    out.push((element, parent));
    for child in element.child_elements() {
        walk(child, Some(element), out);
    }
}

/// The footnote, endnote or comment reference `element` holds, by its local name: the first
/// of those kinds, in that order, found anywhere in it (docx-agent's `_REFERENCES` loop).
pub(crate) fn held_reference(element: &Element) -> Option<&'static str> {
    let mut all = Vec::new();
    walk(element, None, &mut all);
    ["footnoteReference", "endnoteReference", "commentReference"]
        .into_iter()
        .find(|local| all.iter().any(|(e, _)| is(e, local)))
}

/// The elements at or below `element`, in document order, each with its place below
/// `element` (equal elements may share one allocation: places tell them apart).
fn placed<'e>(element: &'e Element, at: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, &'e Element)>) {
    out.push((at.clone(), element));
    for (k, node) in element.children().iter().enumerate() {
        if let (Node::Element(child), Ok(k)) = (node, u32::try_from(k)) {
            at.push(k);
            placed(child, at, out);
            at.pop();
        }
    }
}

/// The element at `path` below `element`.
fn below<'e>(element: &'e Element, path: &[u32]) -> Option<&'e Element> {
    let mut node = element;
    for &k in path {
        node = node.children().get(k as usize)?.as_element()?;
    }
    Some(node)
}

/// docx-agent's `block_references`: whether the references `element` holds are all a
/// block comment's (the comment's range ending between blocks, the reference run opening
/// its paragraph), which move to the paragraph after it -- `Some(true)` with some,
/// `Some(false)` with none -- or `None` when it holds any other.
pub(crate) fn block_references(root: &Element, element: &Element) -> Option<bool> {
    let mut inside = Vec::new();
    placed(element, &mut Vec::new(), &mut inside);
    let references: Vec<&(Vec<u32>, &Element)> = inside
        .iter()
        .filter(|(_, e)| is(e, "footnoteReference") || is(e, "endnoteReference") || is(e, "commentReference"))
        .collect();
    if references.is_empty() {
        return Some(false);
    }
    let mut everything = Vec::new();
    walk(root, None, &mut everything);
    for (path, node) in references {
        if !is(node, "commentReference") {
            return None;
        }
        let id = node.attribute(Some(W), "id");
        let ends: Vec<_> = everything
            .iter()
            .filter(|(e, _)| is(e, "commentRangeEnd") && e.attribute(Some(W), "id") == id)
            .collect();
        let (_, run_path) = path.split_last()?;
        let (&run_index, paragraph_path) = run_path.split_last()?;
        let paragraph = below(element, paragraph_path)?;
        let [(_, end_parent)] = ends.as_slice() else {
            return None;
        };
        if !is(paragraph, "p") || end_parent.is_some_and(|p| is(p, "p")) {
            return None;
        }
        let before = paragraph
            .children()
            .iter()
            .take(run_index as usize)
            .filter_map(Node::as_element)
            .filter(|c| is(c, "r"));
        if before
            .into_iter()
            .any(|c| !c.child_elements().any(|g| is(g, "commentReference")))
        {
            return None;
        }
    }
    Some(true)
}

/// The blocks of a container in order, each with its place, through block-level content
/// controls and custom XML (docx-agent's `_direct_blocks`). Places, not addresses, tell
/// blocks apart: equal elements may share one allocation.
fn direct_blocks<'e>(container: &'e Element, at: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, &'e Element)>) {
    for (k, node) in container.children().iter().enumerate() {
        let (Node::Element(child), Ok(k)) = (node, u32::try_from(k)) else {
            continue;
        };
        at.push(k);
        if is(child, "p") || is(child, "tbl") {
            out.push((at.clone(), child));
        } else if is(child, "sdt") {
            if let Some((c, content)) = child
                .children()
                .iter()
                .enumerate()
                .find_map(|(c, n)| n.as_element().filter(|e| is(e, "sdtContent")).map(|e| (c, e)))
            {
                at.push(u32::try_from(c).unwrap_or(u32::MAX));
                direct_blocks(content, at, out);
                at.pop();
            }
        } else if is(child, "customXml") {
            direct_blocks(child, at, out);
        }
        at.pop();
    }
}

/// Refuse a deletion that would leave a cell, the body or a story without a paragraph to
/// end on (docx-agent's `_check_container_survives`).
///
/// # Errors
/// [`EditError::Refused`] when the container would be left empty or ending on a table.
pub(crate) fn check_container_survives(root: &Arc<Element>, path: &[u32]) -> Result<()> {
    let mut at = path.split_last().map(|(_, above)| above.to_vec()).unwrap_or_default();
    let container = loop {
        let node = if at.is_empty() { Some(root) } else { at_path(root, &at) };
        let Some(node) = node else {
            return Err(EditError::Refused("the block has no container".to_owned()));
        };
        if (is(node, "sdtContent") || is(node, "sdt") || is(node, "customXml")) && at.pop().is_some() {
            continue;
        }
        break node;
    };
    let mut blocks = Vec::new();
    direct_blocks(container, &mut at, &mut blocks);
    blocks.retain(|(at, _)| at.as_slice() != path);
    match blocks.last() {
        None => Err(EditError::Refused(
            "deleting it would leave its story or cell with no paragraph".to_owned(),
        )),
        Some((_, last)) if is(last, "tbl") => Err(EditError::Refused(
            "deleting it would leave a table last in its story or cell; Word needs a paragraph after it".to_owned(),
        )),
        Some(_) => Ok(()),
    }
}

/// Whether a range marker in `element` has its partner outside it.
pub(crate) fn has_lonely_markers(element: &Element) -> bool {
    let mut all = Vec::new();
    walk(element, None, &mut all);
    let markers: Vec<(&str, &str)> = all
        .iter()
        .filter(|(e, _)| e.name().namespace() == Some(W))
        .filter_map(|(e, _)| {
            let local = e.name().local();
            PAIRS
                .iter()
                .any(|(a, b)| *a == local || *b == local)
                .then(|| (local, e.attribute(Some(W), "id").unwrap_or("")))
        })
        .collect();
    markers.iter().any(|(local, id)| {
        let partner = PAIRS
            .iter()
            .find_map(|(a, b)| (a == local).then_some(*b).or_else(|| (b == local).then_some(*a)));
        !markers.iter().any(|(l, i)| Some(*l) == partner && i == id)
    })
}

/// The places of the part's paragraphs in document order (every `w:p`, text boxes'
/// included, as lxml's `iter` finds them).
fn paragraph_paths(element: &Element, at: &mut Vec<u32>, out: &mut Vec<Vec<u32>>) {
    for (k, child) in element.children().iter().enumerate() {
        if let (Node::Element(e), Ok(k)) = (child, u32::try_from(k)) {
            at.push(k);
            if is(e, "p") {
                out.push(at.clone());
            }
            paragraph_paths(e, at, out);
            at.pop();
        }
    }
}

/// The paragraph before (or after) the block at `path` in document order, outside it:
/// docx-agent's `_neighbour_paragraph`.
pub(crate) fn neighbour_path(root: &Element, path: &[u32], forward: bool) -> Option<Vec<u32>> {
    let mut all = Vec::new();
    paragraph_paths(root, &mut Vec::new(), &mut all);
    let inside = |p: &Vec<u32>| p.starts_with(path);
    let first = all.iter().position(inside)?;
    let last = all.iter().rposition(inside)?;
    if forward {
        all.get(last + 1..)?.iter().find(|p| !inside(p)).cloned()
    } else {
        all.get(..first)?.iter().rev().find(|p| !inside(p)).cloned()
    }
}
