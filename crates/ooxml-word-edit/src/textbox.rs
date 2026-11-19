//! Owns: keeping a text box's VML fallback with its drawing's content, as docx-agent does
//! after every edit (`Document._sync_text_boxes`, `drawings.sync_text_box`): a text box
//! (`mc:AlternateContent`) whose choice's content an edit changed gets the same content in
//! its fallback, paraIds and all, as Word writes it. The ids stamping gives (paraId,
//! textId, anchorId, editId) are no change on their own: Word reads `mc:Choice` and writes
//! the fallback again on saving, and older readers ignore them.
//!
//! Must not: look at blocks the edit did not replace (a text box in them did not change),
//! or write but through the sink.

use crate::names::{MC, W, W14, WP14};
use crate::sink::EditSink;
use crate::work::{scope_above, Work};
use crate::{EditError, Result};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// The first `w:txbxContent` at or below `element`, in document order.
fn first_content(element: &Arc<Element>) -> Option<Arc<Element>> {
    if element.name().is(Some(W), "txbxContent") {
        return Some(Arc::clone(element));
    }
    element.child_elements().find_map(first_content)
}

/// Every `mc:AlternateContent` at or below `element`, with its path (from `base`).
fn holders(element: &Arc<Element>, path: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, Arc<Element>)>) {
    if element.name().is(Some(MC), "AlternateContent") {
        out.push((path.clone(), Arc::clone(element)));
    }
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        holders(child, path, out);
        path.pop();
    }
}

/// A story part's block holder (the main part's body, another part's root) and its path.
fn story_blocks(root: &Arc<Element>) -> (Vec<u32>, Arc<Element>) {
    root.children()
        .iter()
        .enumerate()
        .find_map(|(k, n)| {
            let e = n.as_element().filter(|e| e.name().is(Some(W), "body"))?;
            Some((vec![u32::try_from(k).ok()?], Arc::clone(e)))
        })
        .unwrap_or_else(|| (Vec::new(), Arc::clone(root)))
}

/// The top-level blocks of a story part (the main part's body's children, another
/// part's root's) that are not the snapshot's: each one's path and the snapshot's block
/// it took the place of (none for one inserted). Found from the stretch of children that
/// differs (pointer comparisons), so the blocks the edit did not touch are not visited.
pub(crate) fn changed_blocks(old: &Arc<Element>, new: &Arc<Element>) -> Vec<(Vec<u32>, Option<Arc<Element>>)> {
    let ((base, old), (_, new)) = (story_blocks(old), story_blocks(new));
    let Some((start, old_end, new_end)) = old.children().changed_span(new.children()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, node) in new.children().iter().enumerate().skip(start).take(new_end - start) {
        let Node::Element(block) = node else { continue };
        let before = (k < old_end)
            .then(|| old.children().get(k).and_then(Node::as_element))
            .flatten();
        if before.is_some_and(|b| Arc::ptr_eq(b, block)) {
            continue;
        }
        let mut path = base.clone();
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        out.push((path, before.cloned()));
    }
    out
}

/// The text boxes of the stretch of a story part's blocks that differs from the
/// snapshot's: the snapshot's, in order, and the working version's, in order with their
/// paths.
type SpanBoxes = (Vec<Arc<Element>>, Vec<(Vec<u32>, Arc<Element>)>);

fn span_boxes(old: &Arc<Element>, new: &Arc<Element>) -> SpanBoxes {
    let ((base, old), (_, new)) = (story_blocks(old), story_blocks(new));
    let Some((start, old_end, new_end)) = old.children().changed_span(new.children()) else {
        return (Vec::new(), Vec::new());
    };
    let mut before = Vec::new();
    for node in old.children().iter().skip(start).take(old_end - start) {
        if let Node::Element(block) = node {
            holders(block, &mut Vec::new(), &mut before);
        }
    }
    let mut now = Vec::new();
    for (k, node) in new.children().iter().enumerate().skip(start).take(new_end - start) {
        if let Node::Element(block) = node {
            let mut path = base.clone();
            path.push(u32::try_from(k).unwrap_or(u32::MAX));
            holders(block, &mut path, &mut now);
        }
    }
    (before.into_iter().map(|(_, h)| h).collect(), now)
}

/// After an edit: sync every text box of every story part, in the blocks the edit
/// replaced, whose content is not the one the transaction's snapshot had -- the ids
/// stamping gave it aside. The boxes of the stretch that changed are paired in order with
/// the snapshot's when the edit added or removed none (an inserted paragraph shifts the
/// blocks after it); otherwise every one not the snapshot's own is synced.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn sync(sink: &mut EditSink<'_, '_>) -> Result<()> {
    // the story parts are the snapshot's (an edit of text adds or removes none); their
    // trees, the working version's
    let stories: Vec<String> = sink.base().docx()?.parts.iter().map(|p| p.part.clone()).collect();
    for part in &stories {
        let part = part.as_str();
        let (Some(old), Some(new)) = (sink.base().tree(part)?, sink.doc().tree(part)?) else {
            continue;
        };
        if old.ptr_eq(&new) {
            continue;
        }
        let (before, now) = span_boxes(old.root(), new.root());
        let paired = before.len() == now.len();
        for (k, (path, holder)) in now.into_iter().enumerate() {
            let was = before.get(k).filter(|_| paired);
            if was.is_some_and(|was| Arc::ptr_eq(was, &holder)) {
                continue; // the same text box as before the edit
            }
            let contents = was.and_then(choice_content).zip(choice_content(&holder));
            if contents.is_some_and(|(was, now)| same_but_ids(&was, &now)) {
                continue; // only stamped
            }
            sync_holder(sink, part, &path, &holder)?;
        }
    }
    Ok(())
}

/// Every text box of every story part whose fallback does not hold what its choice holds
/// gets its choice's content (docx-agent's `_sync_text_boxes(None)`, after a review: the
/// boxes an edit did not touch too).
///
/// # Errors
/// When a part is not XML.
pub(crate) fn sync_all(sink: &mut EditSink<'_, '_>) -> Result<()> {
    let stories: Vec<String> = sink.docx()?.parts.iter().map(|p| p.part.clone()).collect();
    for part in &stories {
        let Some(tree) = sink.doc().tree(part)? else { continue };
        let mut found = Vec::new();
        holders(tree.root(), &mut Vec::new(), &mut found);
        let mut synced: Vec<Vec<u32>> = Vec::new();
        for (path, holder) in found {
            if synced.iter().any(|s| path.starts_with(s)) {
                continue; // inside a text box written whole just now
            }
            if sync_holder(sink, part, &path, &holder)? {
                synced.push(path);
            }
        }
    }
    Ok(())
}

/// The text box `holder` at `path` of `part` given its choice's content in its fallback,
/// when the two differ; whether it was.
pub(crate) fn sync_holder(
    sink: &mut EditSink<'_, '_>,
    part: &str,
    path: &[u32],
    holder: &Arc<Element>,
) -> Result<bool> {
    let (Some(choice), Some(fallback)) = (holder.find(Some(MC), "Choice"), holder.find(Some(MC), "Fallback")) else {
        return Ok(false);
    };
    let (Some(source), Some(target)) = (first_content(choice), first_content(fallback)) else {
        return Ok(false);
    };
    if *source == *target {
        return Ok(false);
    }
    let Some(tree) = sink.doc().tree(part)? else {
        return Ok(false);
    };
    let mut work = Work::of(holder, scope_above(tree.root(), path));
    let root = work.root();
    let content = |work: &Work, side: &str| {
        let at = work.find(root, MC, side)?;
        work.iter(at, W, &["txbxContent"]).into_iter().next()
    };
    let (Some(from), Some(into)) = (content(&work, "Choice"), content(&work, "Fallback")) else {
        return Ok(false);
    };
    for child in work.children(into).to_vec() {
        work.remove(child);
    }
    for child in work.children(from).to_vec() {
        let copy = work.deep_copy(child);
        work.append(into, copy);
    }
    let synced = work.build().ok_or_else(|| EditError::Path(part.to_owned()))?;
    sink.replace(part, path, synced, vec![crate::doc::Key::Part(Arc::from(part))])?;
    Ok(true)
}

/// Whether an attribute is an id stamping gives.
fn stamped_id(attribute: &ooxml_xml::Attribute) -> bool {
    let name = &attribute.name;
    (name.namespace() == Some(W14) && matches!(name.local(), "paraId" | "textId"))
        || (name.namespace() == Some(WP14) && matches!(name.local(), "anchorId" | "editId"))
}

/// Whether two elements hold the same but for the ids stamping gives (and namespace
/// declarations, which stamping adds for them).
fn same_but_ids(a: &Element, b: &Element) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    let attributes = |e: &Element| -> Vec<(Option<String>, String, String)> {
        e.attributes()
            .iter()
            .filter(|a| !stamped_id(a))
            .map(|a| {
                (
                    a.name.namespace().map(str::to_owned),
                    a.name.local().to_owned(),
                    a.value.to_string(),
                )
            })
            .collect()
    };
    a.name().namespace() == b.name().namespace()
        && a.name().local() == b.name().local()
        && attributes(a) == attributes(b)
        && a.children().len() == b.children().len()
        && a.children().iter().zip(b.children().iter()).all(|(x, y)| match (x, y) {
            (Node::Element(x), Node::Element(y)) => same_but_ids(x, y),
            _ => x.same(y),
        })
}

/// A text box's content as Word reads it (its choice's `w:txbxContent`).
fn choice_content(holder: &Arc<Element>) -> Option<Arc<Element>> {
    first_content(holder.find(Some(MC), "Choice")?)
}
