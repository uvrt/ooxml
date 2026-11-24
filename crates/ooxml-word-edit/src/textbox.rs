//! Owns: keeping a text box's VML fallback with its drawing's content, as docx-agent does
//! after every edit (`Document._sync_text_boxes`, `drawings.sync_text_box`): a text box
//! (`mc:AlternateContent`) whose choice's content an edit changed gets the same content in
//! its fallback, paraIds and all, as Word writes it. The ids stamping gives (paraId,
//! textId, anchorId, editId) are no change on their own: Word reads `mc:Choice` and writes
//! the fallback again on saving, and older readers ignore them.
//!
//! Must not: look at blocks the edit did not replace (a text box in them did not change),
//! or write but through the sink.

use crate::names::{MC, W, W14, WP, WP14};
use crate::sink::EditSink;
use crate::work::{scope_above, Work};
use crate::{EditError, Result};
use ooxml_xml::{changed_runs, Element, Node};
use std::sync::Arc;

/// The place of the paragraph anchoring the text box the element at `path` of `part`
/// stands in (the outermost paragraph on its path), if it stands in one.
pub(crate) fn text_box_anchor(docx: &ooxml_word_model::docx::Docx, part: &str, path: &[u32]) -> Option<Vec<u32>> {
    let root = docx.part(part)?.root();
    let mut anchor = None;
    for n in 1..path.len() {
        let element = ooxml_word_model::docx::at_path(root, path.get(..n)?)?;
        if element.name().is(Some(W), "txbxContent") {
            return anchor;
        }
        if anchor.is_none() && element.name().is(Some(W), "p") {
            anchor = Some(path.get(..n)?.to_vec());
        }
    }
    None
}

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
/// it took the place of (none for one inserted). Found from the runs of children that
/// differ ([`changed_runs`], by identity), so the blocks the edit did not touch -- moved
/// past or not -- are not visited.
pub(crate) fn changed_blocks(old: &Arc<Element>, new: &Arc<Element>) -> Vec<(Vec<u32>, Option<Arc<Element>>)> {
    let ((base, old), (_, new)) = (story_blocks(old), story_blocks(new));
    let mut out = Vec::new();
    for run in changed_runs(old.children(), new.children()) {
        for k in run.new.clone() {
            let Some(Node::Element(block)) = new.children().get(k) else {
                continue;
            };
            let place = run.old.start + (k - run.new.start);
            let before = run
                .old
                .contains(&place)
                .then(|| old.children().get(place).and_then(Node::as_element))
                .flatten();
            if before.is_some_and(|b| Arc::ptr_eq(b, block)) {
                continue;
            }
            let mut path = base.clone();
            path.push(u32::try_from(k).unwrap_or(u32::MAX));
            out.push((path, before.cloned()));
        }
    }
    out
}

/// The text boxes of the runs of a story part's blocks that differ from the snapshot's:
/// the snapshot's, in order, and the working version's, in order with their paths.
type SpanBoxes = (Vec<Arc<Element>>, Vec<(Vec<u32>, Arc<Element>)>);

fn span_boxes(old: &Arc<Element>, new: &Arc<Element>) -> SpanBoxes {
    let ((base, old), (_, new)) = (story_blocks(old), story_blocks(new));
    let mut before = Vec::new();
    let mut now = Vec::new();
    for run in changed_runs(old.children(), new.children()) {
        for k in run.old {
            if let Some(Node::Element(block)) = old.children().get(k) {
                holders(block, &mut Vec::new(), &mut before);
            }
        }
        for k in run.new {
            if let Some(Node::Element(block)) = new.children().get(k) {
                let mut path = base.clone();
                path.push(u32::try_from(k).unwrap_or(u32::MAX));
                holders(block, &mut path, &mut now);
            }
        }
    }
    (before.into_iter().map(|(_, h)| h).collect(), now)
}

/// After an edit: sync every text box of every story part, in the blocks the edit
/// replaced, whose content is not the one the transaction's snapshot had -- the ids
/// stamping gave it aside (rule of #94: only a change of a box's content syncs its
/// fallback; stamping it or moving its block does not).
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
        let kept = unchanged_boxes(&before, &now);
        for ((path, holder), kept) in now.into_iter().zip(kept) {
            if !kept {
                sync_holder(sink, part, &path, &holder)?;
            }
        }
    }
    Ok(())
}

/// Which of the changed stretch's boxes `now` are boxes of the snapshot's stretch `before`
/// with their content unchanged, each snapshot box standing for one box at most. A box is
/// its snapshot's when it is the same element (untouched, wherever its block went). When
/// the edit added or removed no box, one only stamped is its snapshot's too: the box in its
/// place first, else the first one not yet taken with the same content (the box of a moved
/// block, which docx-agent knows by the element's identity). When the edit added or removed
/// boxes, a box is its snapshot's when it is the same element or holds the same drawing
/// (by number) with the same content; any other counts as changed, as docx-agent's does when
/// an edit gives a box a new element (a moved block's copy among them).
fn unchanged_boxes(before: &[Arc<Element>], now: &[(Vec<u32>, Arc<Element>)]) -> Vec<bool> {
    let mut taken = vec![false; before.len()];
    let mut kept: Vec<bool> = now
        .iter()
        .map(|(_, holder)| take(&mut taken, before, 0..before.len(), |was| Arc::ptr_eq(was, holder)))
        .collect();
    if before.len() != now.len() {
        // boxes added or removed: a box is its snapshot's by its drawing's number, unique
        // in the document (a copy's drawings are numbered anew), its content unchanged
        for ((_, holder), kept) in now.iter().zip(kept.iter_mut()) {
            if !*kept {
                let number = drawing_number(holder);
                *kept = number.is_some()
                    && take(&mut taken, before, 0..before.len(), |was| {
                        drawing_number(was) == number && unchanged_box(was, holder)
                    });
            }
        }
        return kept;
    }
    for (k, ((_, holder), kept)) in now.iter().zip(kept.iter_mut()).enumerate() {
        if !*kept {
            let order = std::iter::once(k).chain((0..before.len()).filter(move |&j| j != k));
            *kept = take(&mut taken, before, order, |was| unchanged_box(was, holder));
        }
    }
    kept
}

/// Take the first snapshot box of `order` not yet `taken` that `matches`; whether one was.
fn take(
    taken: &mut [bool],
    before: &[Arc<Element>],
    mut order: impl Iterator<Item = usize>,
    matches: impl Fn(&Arc<Element>) -> bool,
) -> bool {
    let found = order.find(|&j| taken.get(j) == Some(&false) && before.get(j).is_some_and(&matches));
    found.and_then(|j| taken.get_mut(j)).map(|t| *t = true).is_some()
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

/// The number (`wp:docPr/@id`) of the drawing a box's choice holds.
fn drawing_number(holder: &Element) -> Option<String> {
    fn find(element: &Element) -> Option<String> {
        if element.name().is(Some(WP), "docPr") {
            return element.attribute(None, "id").map(str::to_owned);
        }
        element.child_elements().find_map(|c| find(c))
    }
    holder.find(Some(MC), "Choice").and_then(|choice| find(choice))
}

/// Whether the box `holder` holds what the snapshot's box `was` held, but for the ids
/// stamping gave.
fn unchanged_box(was: &Arc<Element>, holder: &Arc<Element>) -> bool {
    choice_content(was)
        .zip(choice_content(holder))
        .is_some_and(|(was, now)| same_but_ids(&was, &now))
}

/// A text box's content as Word reads it (its choice's `w:txbxContent`).
fn choice_content(holder: &Arc<Element>) -> Option<Arc<Element>> {
    first_content(holder.find(Some(MC), "Choice")?)
}
