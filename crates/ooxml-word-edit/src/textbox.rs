//! Owns: keeping a text box's VML fallback with its drawing's content, as docx-agent does
//! after every edit (`Document._sync_text_boxes`, `drawings.sync_text_box`): a text box
//! (`mc:AlternateContent`) whose choice's content an edit changed -- its text, or the ids
//! stamping gave its paragraphs -- gets the same content in its fallback, paraIds and all,
//! as Word writes it.
//!
//! Must not: look at blocks the edit did not replace (a text box in them did not change),
//! or write but through the sink.

use crate::names::{MC, W};
use crate::sink::EditSink;
use crate::work::{scope_above, Work};
use crate::{EditError, Result};
use ooxml_xml::{Element, Node};
use std::collections::HashSet;
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

/// The top-level blocks of a story part (the main part's body's children, another
/// part's root's) that are not the snapshot's: each one's path and the snapshot's block
/// at that place.
fn changed_blocks(old: &Arc<Element>, new: &Arc<Element>) -> Vec<(Vec<u32>, Option<Arc<Element>>)> {
    let holder = |root: &Arc<Element>| -> (Vec<u32>, Arc<Element>) {
        root.children()
            .iter()
            .enumerate()
            .find_map(|(k, n)| {
                let e = n.as_element().filter(|e| e.name().is(Some(W), "body"))?;
                Some((vec![u32::try_from(k).ok()?], Arc::clone(e)))
            })
            .unwrap_or_else(|| (Vec::new(), Arc::clone(root)))
    };
    let ((base, old), (_, new)) = (holder(old), holder(new));
    if Arc::ptr_eq(&old, &new) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (k, node) in new.children().iter().enumerate() {
        let Node::Element(block) = node else { continue };
        let before = old.children().get(k).and_then(Node::as_element);
        if before.is_some_and(|b| Arc::ptr_eq(b, block)) {
            continue;
        }
        let mut path = base.clone();
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        out.push((path, before.cloned()));
    }
    out
}

/// After an edit: sync every text box of every story part, in the blocks the edit
/// replaced, whose content is not the one the transaction's snapshot had.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn sync(sink: &mut EditSink<'_, '_>) -> Result<()> {
    let docx = sink.docx()?;
    for story in &docx.parts {
        let part = story.part.as_str();
        let Some(old) = sink.base().tree(part)? else { continue };
        if old.ptr_eq(&story.tree) {
            continue;
        }
        for (block, before) in changed_blocks(old.root(), story.root()) {
            sync_block(sink, part, &block, before.as_ref())?;
        }
    }
    Ok(())
}

fn sync_block(sink: &mut EditSink<'_, '_>, part: &str, block: &[u32], old: Option<&Arc<Element>>) -> Result<()> {
    let before: HashSet<usize> = old.map_or_else(HashSet::new, |old| {
        let mut found = Vec::new();
        holders(old, &mut Vec::new(), &mut found);
        found.iter().map(|(_, h)| Arc::as_ptr(h) as usize).collect()
    });
    {
        let Some(tree) = sink.doc().tree(part)? else {
            return Ok(());
        };
        let Some(element) = ooxml_word_model::docx::at_path(tree.root(), block) else {
            return Ok(());
        };
        let mut found = Vec::new();
        holders(element, &mut block.to_vec(), &mut found);
        for (path, holder) in found {
            if before.contains(&(Arc::as_ptr(&holder) as usize)) {
                continue; // the same text box as before the edit
            }
            let (Some(choice), Some(fallback)) = (holder.find(Some(MC), "Choice"), holder.find(Some(MC), "Fallback"))
            else {
                continue;
            };
            let (Some(source), Some(target)) = (first_content(choice), first_content(fallback)) else {
                continue;
            };
            if *source == *target {
                continue;
            }
            let mut work = Work::of(&holder, scope_above(tree.root(), &path));
            let root = work.root();
            let content = |work: &Work, side: &str| {
                let at = work.find(root, MC, side)?;
                work.iter(at, W, &["txbxContent"]).into_iter().next()
            };
            let (Some(from), Some(into)) = (content(&work, "Choice"), content(&work, "Fallback")) else {
                continue;
            };
            for child in work.children(into).to_vec() {
                work.remove(child);
            }
            for child in work.children(from).to_vec() {
                let copy = work.deep_copy(child);
                work.append(into, copy);
            }
            let synced = work.build().ok_or_else(|| EditError::Path(part.to_owned()))?;
            sink.replace(part, &path, synced, vec![crate::doc::Key::Part(Arc::from(part))])?;
        }
    }
    Ok(())
}
