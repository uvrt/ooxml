//! Owns: keeping revision ids unique after an edit, as docx-agent's `_unique_revision_ids`
//! does: an edit that splits a run copies its `w:rPrChange` with the rest of its
//! properties, so a revision id it repeated gets a new one (one above the largest
//! annotation id), the first occurrence in document order keeping its own. Repeats the
//! document had before are left alone.
//!
//! Must not: read the document unless the edited paragraph holds more of an id than before
//! (the only way an edit repeats one), or write but through the sink.

use crate::doc::Key;
use crate::names::W;
use crate::sink::EditSink;
use crate::stamping::with_attribute;
use crate::validate::annotation_ceiling;
use crate::Result;
use ooxml_word_model::docx::at_path;
use ooxml_xml::{Element, HeadPatch, Node, NodePath, QName};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Every revision record's local name (docx-agent's `REVISION_KINDS_ALL`).
pub(crate) const KINDS: &[&str] = &[
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    "rPrChange",
    "pPrChange",
    "sectPrChange",
    "tblPrChange",
    "trPrChange",
    "tcPrChange",
    "tblGridChange",
    "numberingChange",
    "cellIns",
    "cellDel",
    "cellMerge",
];

pub(crate) fn is_record(e: &Element) -> bool {
    e.name().namespace() == Some(W) && KINDS.contains(&e.name().local())
}

/// Each revision record's `w:id` below `element` (itself included), counted.
fn counts(element: &Element) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    let mut stack = vec![element];
    while let Some(e) = stack.pop() {
        if is_record(e) {
            if let Some(id) = e.attribute(Some(W), "id") {
                *out.entry(id.to_owned()).or_insert(0) += 1;
            }
        }
        stack.extend(e.child_elements().map(|c| &**c));
    }
    out
}

fn add(into: &mut HashMap<String, usize>, more: HashMap<String, usize>) {
    for (id, n) in more {
        *into.entry(id).or_insert(0) += n;
    }
}

/// Every revision record with one of `wanted`'s ids in `root`, in document order, with its
/// path (child indexes from the root).
fn occurrences(root: &Element, wanted: &[String], path: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, Arc<Element>)>) {
    for (k, node) in root.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        if is_record(child)
            && child
                .attribute(Some(W), "id")
                .is_some_and(|id| wanted.iter().any(|w| w == id))
        {
            out.push((path.clone(), Arc::clone(child)));
        }
        occurrences(child, wanted, path, out);
        path.pop();
    }
}

/// Give each revision id the edit of the paragraph at `path` of `part` repeated (it was
/// `old` before) an id of its own, after the first occurrence.
///
/// # Errors
/// When a part is not XML.
pub(crate) fn make_unique(sink: &mut EditSink<'_, '_>) -> Result<()> {
    // the ids in the blocks the edit replaced (its paragraph's, text boxes it synced),
    // before and after
    let (mut before, mut after) = (HashMap::new(), HashMap::new());
    let stories: Vec<String> = sink.base().docx()?.parts.iter().map(|p| p.part.clone()).collect();
    for part in &stories {
        let (Some(old), Some(new)) = (sink.base().tree(part)?, sink.doc().tree(part)?) else {
            continue;
        };
        if old.ptr_eq(&new) {
            continue;
        }
        for (path, was) in crate::textbox::changed_blocks(old.root(), new.root()) {
            if let Some(was) = was {
                add(&mut before, counts(&was));
            }
            if let Some(now) = at_path(new.root(), &path) {
                add(&mut after, counts(now));
            }
        }
    }
    // an id this edit gave is new (above every id the snapshot had): it repeats nothing
    let ceiling = annotation_ceiling(sink.base())?;
    let given = |id: &str| id.trim().parse::<i64>().is_ok_and(|v| v > ceiling);
    let candidates: Vec<String> = after
        .iter()
        .filter(|(id, n)| **n > before.get(*id).copied().unwrap_or(0) && (**n > 1 || !given(id)))
        .map(|(id, _)| id.clone())
        .collect();
    renumber(sink, &candidates)
}

/// Every revision record's `w:id` in every story part of `docx`, counted
/// (`_revision_id_counts`).
pub(crate) fn document_counts(docx: &ooxml_word_model::docx::Docx) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for story in &docx.parts {
        add(&mut out, counts(story.root()));
    }
    out
}

/// Give each revision id the document holds more of than `before` counted, more than
/// once, an id of its own after its first occurrence (`_unique_revision_ids(before)` over
/// the whole document, as docx-agent does after a review).
///
/// # Errors
/// When a part is not XML.
pub(crate) fn unique_since(sink: &mut EditSink<'_, '_>, before: &HashMap<String, usize>) -> Result<()> {
    let after = document_counts(&*sink.docx()?);
    let grown: Vec<String> = after
        .iter()
        .filter(|(id, n)| **n > 1 && **n > before.get(*id).copied().unwrap_or(0))
        .map(|(id, _)| id.clone())
        .collect();
    renumber(sink, &grown)
}

/// The later occurrences of each of `candidates` that occurs more than once, numbered
/// from one above the largest annotation id, in story and document order.
fn renumber(sink: &mut EditSink<'_, '_>, candidates: &[String]) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let docx = sink.docx()?;
    let mut found: Vec<(String, Vec<u32>, Arc<Element>)> = Vec::new();
    for story in &docx.parts {
        let mut places = Vec::new();
        occurrences(story.root(), candidates, &mut Vec::new(), &mut places);
        found.extend(places.into_iter().map(|(p, e)| (story.part.clone(), p, e)));
    }
    let mut total: HashMap<&str, usize> = HashMap::new();
    for (_, _, e) in &found {
        *total.entry(e.attribute(Some(W), "id").unwrap_or("")).or_insert(0) += 1;
    }
    let grown: Vec<&String> = candidates
        .iter()
        .filter(|c| total.get(c.as_str()).copied().unwrap_or(0) > 1)
        .collect();
    if grown.is_empty() {
        return Ok(());
    }
    let mut largest = annotation_ceiling(sink.doc())?;
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for (story, place, element) in found {
        let id = element.attribute(Some(W), "id").unwrap_or("").to_owned();
        if !grown.iter().any(|g| **g == id) {
            continue;
        }
        if seen.insert(id, ()).is_none() {
            continue;
        }
        largest += 1;
        let name = element
            .attributes()
            .iter()
            .find(|a| a.name.is(Some(W), "id"))
            .map_or_else(
                || Arc::new(QName::new(Some("w"), "id", Some(W))),
                |a| Arc::clone(&a.name),
            );
        let head = with_attribute(element.head(), &name, &largest.to_string());
        let mut node_path = vec![0usize];
        node_path.extend(place.iter().map(|k| *k as usize));
        sink.read(Key::Annotations);
        sink.set_head_keyed(
            &story,
            HeadPatch::new(NodePath(node_path), Arc::clone(element.head()), Arc::new(head)),
            vec![Key::part(story.as_str()), Key::Annotations],
        )?;
    }
    Ok(())
}
