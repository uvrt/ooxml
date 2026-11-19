//! Owns: where insert_markdown's blocks go and how they are put there (docx-agent's
//! `_locate`, `_lone_empty_paragraph`, `_insert`, `Writer.stamp`, `Writer.track`): the
//! container and the place in it, a body that is one empty paragraph typed into, the
//! stretch of the container the edit reads (the blocks beside the place), separators and a
//! closing paragraph, paraIds and textIds drawn in document order (rows from their own
//! sequence), and the insertion tracked as one revision group in Word's typing form.
//!
//! Must not: write but through the sink.

use super::writer::Writer;
use super::{InsertMarkdown, MarkdownInserted};
use crate::doc::Key;
use crate::marks::{clean_properties, insert_in_order, mark, mark_record, pass_mark};
use crate::names::{W, W14, W16DU};
use crate::policy::{RevisionPolicy, Stamp};
use crate::set_text::{block_key, paragraph_has_annotations};
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::{stamp_document, w14_prefix};
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::valid_long_hex;
use ooxml_word_model::markdown::model::Block;
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// The block elements a container holds.
const BLOCKS: [&str; 4] = ["p", "tbl", "sdt", "customXml"];

/// Where the blocks go.
pub(crate) struct Place {
    part: String,
    story: String,
    /// The container's path from the part's root.
    container: Vec<u32>,
    /// The child (node index) they go before; none: the end.
    before: Option<u32>,
    /// The body's one empty paragraph typed into: its id and node index.
    lone: Option<(String, u32)>,
}

fn is_block(e: &Element) -> bool {
    e.name().namespace() == Some(W) && BLOCKS.contains(&e.name().local())
}

/// `_lone_empty_paragraph`: the id of the body's only block when it is an empty paragraph
/// (no content, no section break or numbering), with its node index.
fn lone_empty_paragraph(docx: &Docx) -> Option<(String, Vec<u32>)> {
    let body_part = docx.body()?;
    let root = body_part.root();
    let body = root.child_elements().find(|c| c.name().is(Some(W), "body"))?;
    let blocks: Vec<&Arc<Element>> = body
        .child_elements()
        .filter(|c| !c.name().is(Some(W), "sectPr"))
        .collect();
    let [paragraph] = blocks.as_slice() else { return None };
    if !paragraph.name().is(Some(W), "p") || paragraph.child_elements().any(|c| !c.name().is(Some(W), "pPr")) {
        return None;
    }
    let properties = paragraph.child_elements().find(|c| c.name().is(Some(W), "pPr"));
    if properties.is_some_and(|p| {
        p.child_elements()
            .any(|c| c.name().is(Some(W), "sectPr") || c.name().is(Some(W), "numPr"))
    }) {
        return None;
    }
    let entries = body_part.ids.paragraphs();
    let entry = entries.first().filter(|_| entries.len() == 1)?;
    (entry.path.len() == 2).then(|| (entry.id.clone(), entry.path.to_vec()))
}

/// The place `at` names, with the body's empty paragraph typed into where the Markdown
/// writes at the body's end and does not start with a table.
///
/// # Errors
/// A place that does not exist, `replace:` (not ported yet), a story of notes or comments.
pub(crate) fn locate(doc: &WordDoc, docx: &Docx, at: &str, blocks: &[Block]) -> Result<Place> {
    let lone = if matches!(at, "end" | "end:body") && !matches!(blocks.first(), Some(Block::Table(_))) {
        lone_empty_paragraph(docx)
    } else {
        None
    };
    if let Some((id, path)) = lone {
        let index = path.last().copied().unwrap_or_default();
        return Ok(Place {
            part: docx.main.clone(),
            story: "body".to_owned(),
            container: path.get(..1).unwrap_or_default().to_vec(),
            before: Some(index),
            lone: (!matches!(blocks.last(), Some(Block::Table(_)))).then_some((id, index)),
        });
    }
    let (kind, rest) = match at.split_once(':') {
        Some((k, r)) if matches!(k, "after" | "before" | "replace" | "end") => (k, r),
        _ if at == "end" => ("end", ""),
        _ => ("after", at),
    };
    match kind {
        "end" => end_of(docx, if rest.is_empty() { "body" } else { rest }),
        "replace" => Err(EditError::Unsupported("insert_markdown at replace:".to_owned())),
        _ => {
            let (part, found) = crate::blocks::resolve_block(doc, docx, rest)?;
            let story = docx.part(&part).map(|p| p.story.clone()).unwrap_or_default();
            let (&index, container) = found.path.split_last().ok_or_else(|| EditError::Path(part.clone()))?;
            Ok(Place {
                part,
                story,
                container: container.to_vec(),
                before: Some(if kind == "after" { index + 1 } else { index }),
                lone: None,
            })
        }
    }
}

/// A story's end: before a body's final section properties.
fn end_of(docx: &Docx, story: &str) -> Result<Place> {
    let found = docx
        .story(story)
        .ok_or_else(|| EditError::Refused(format!("no story {story:?}")))?;
    let root = found.root();
    if ["footnotes", "endnotes", "comments"]
        .iter()
        .any(|l| root.name().is(Some(W), l))
    {
        return Err(EditError::Refused(format!(
            "{story} holds notes or comments: insert after one of their paragraphs"
        )));
    }
    let body = root
        .children()
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), "body")));
    let (container, element) = match body.and_then(|k| Some((k, root.children().get(k)?))) {
        Some((k, Node::Element(e))) => (vec![u32::try_from(k).unwrap_or_default()], e),
        _ => (Vec::new(), root),
    };
    let section = element
        .children()
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name().is(Some(W), "sectPr")))
        .filter(|_| element.name().is(Some(W), "body"));
    Ok(Place {
        part: found.part.clone(),
        story: story.to_owned(),
        container,
        before: section.map(|k| u32::try_from(k).unwrap_or_default()),
        lone: None,
    })
}

/// The container's element and the stretch `(from, to)` of its children the edit holds:
/// from the block before the place to the block (or section properties) after it.
fn stretch_of(container: &Element, before: Option<u32>) -> Option<(u32, u32)> {
    let children = container.children();
    let len = u32::try_from(children.len()).ok()?;
    let at = before.unwrap_or(len).min(len);
    let element = |k: u32| match children.get(k as usize) {
        Some(Node::Element(e)) => Some(e),
        _ => None,
    };
    let previous = (0..at).rev().find(|k| element(*k).is_some_and(|e| is_block(e)));
    let next = (at..len).find(|k| element(*k).is_some_and(|e| is_block(e) || e.name().is(Some(W), "sectPr")));
    let from = match previous {
        Some(p) => p,
        None if next.is_none() && at == len => at.checked_sub(1)?,
        None => at,
    };
    let to = next.unwrap_or_else(|| at.saturating_sub(1).max(from));
    (from <= to && to < len).then_some((from, to))
}

/// Write the blocks at their place (`_insert`).
pub(crate) fn insert(
    sink: &mut EditSink<'_, '_>,
    command: &InsertMarkdown,
    blocks: &[Block],
) -> Result<MarkdownInserted> {
    let docx = sink.docx()?;
    let place = locate(sink.doc(), &docx, &command.at, blocks)?;
    let renamed = stamp_document(sink, &docx)?;
    let part = place.part.clone();
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let path_error = || EditError::Path(part.clone());
    let container = if place.container.is_empty() {
        Arc::clone(tree.root())
    } else {
        Arc::clone(ooxml_word_model::docx::at_path(tree.root(), &place.container).ok_or_else(path_error)?)
    };
    let (from, to) = stretch_of(&container, place.before)
        .ok_or_else(|| EditError::Unsupported("insert_markdown into an empty container".to_owned()))?;
    let path = |k: u32| -> Box<[u32]> { place.container.iter().copied().chain([k]).collect() };
    let (mut work, held) = crate::ranges::stretch(tree.root(), &[path(from), path(to)]).ok_or_else(path_error)?;
    let child = |k: u32| held.get(k.checked_sub(from)? as usize).copied();
    let anchor = place.before.and_then(child);
    let tracked = matches!(sink.policy(), RevisionPolicy::Tracked(_));
    let mut removed = Vec::new();
    let lone = place.lone.as_ref().and_then(|(id, k)| Some((id.clone(), child(*k)?)));
    let continued = lone
        .as_ref()
        .and_then(|(_, node)| {
            let get = |local: &str| work.get(*node, Some(W14), local).map(str::to_owned);
            Some((get("paraId")?, get("textId")?))
        })
        .filter(|(p, t)| valid_long_hex(p) && valid_long_hex(t))
        .filter(|_| !matches!(blocks.first(), Some(Block::Table(_))));
    let mut old_mark = None;
    let mut insert_before = anchor;
    if let (Some((id, node)), true) = (&lone, tracked) {
        let properties = work.find(*node, W, "pPr");
        old_mark = Some(clean_properties(&mut work, properties, *node));
        insert_before = next_block(&work, *node);
        work.remove(*node);
        removed.push(id.clone());
    }
    let mut writer = Writer::new(&part, &command.style_map, command.options.clone());
    let mut elements = writer.blocks(sink, &mut work, blocks)?;
    let root = work.root();
    for element in &elements {
        match insert_before {
            Some(before) => work.add_previous(before, *element),
            None => work.append(root, *element),
        }
    }
    if let (Some((id, node)), false) = (&lone, tracked) {
        work.remove(*node);
        removed.push(id.clone());
        sink.forget_aliases(std::slice::from_ref(id))?;
    }
    close_tables(sink, &mut work, &mut writer, &mut elements)?;
    let starts_paragraph = elements.first().is_some_and(|e| work.is(*e, W, "p"));
    let stamped = stamp(
        sink,
        &mut work,
        &place,
        &elements,
        continued.filter(|_| starts_paragraph),
    )?;
    let wrote = sink.track(&mut work, |work, stamp, orders| {
        track(work, &elements, stamp, orders, old_mark);
        Vec::new()
    })?;
    commit(sink, &work, &place, (from, to), &stamped.0, &wrote)?;
    let (created, top) = stamped;
    Ok(MarkdownInserted {
        first: created.first().cloned(),
        changed: true,
        created,
        renamed,
        removed,
        warnings: writer.warnings,
        blocks: top,
    })
}

/// The stretch's new content spliced into the part, with the keys it read and wrote and
/// the annotation ids drawn into it; the namespaces and people a tracked write needs.
fn commit(
    sink: &mut EditSink<'_, '_>,
    work: &Work,
    place: &Place,
    (from, to): (u32, u32),
    created: &[String],
    wrote: &crate::sink::TextWrite,
) -> Result<()> {
    let part = place.part.clone();
    let path_error = || EditError::Path(part.clone());
    sink.keep_written(&part, work);
    let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
    let mut draws = Vec::new();
    let stamped_docx = sink.docx()?;
    let mut keys = crate::delete::stretch_keys(&stamped_docx, &part, &place.container, (from, to));
    for (k, node) in nodes.iter().enumerate() {
        if let Node::Element(e) = node {
            if paragraph_has_annotations(e) && !keys.contains(&Key::Annotations) {
                keys.push(Key::Annotations);
            }
            if let (Some(numbered), Ok(k)) = (wrote.annotations, u32::try_from(k)) {
                draws.extend(annotation_draws(e, numbered, k));
            }
        }
    }
    keys.extend(created.iter().map(|id| block_key(&part, id)));
    sink.splice_children(
        &part,
        (&place.container, from as usize, (to - from + 1) as usize),
        nodes,
        keys,
        draws,
    )?;
    if wrote.dated {
        sink.ensure_w14(&part, false, &[("w16du", W16DU)])?;
    }
    sink.ensure_w14(&part, false, &[])?;
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
        sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    }
    Ok(())
}

/// The block (or section properties) after `node`.
fn next_block(work: &Work, node: Id) -> Option<Id> {
    let mut at = work.next(node);
    while let Some(n) = at {
        if work.is_any(n, W, &["p", "tbl", "sdt", "customXml", "sectPr"]) {
            return Some(n);
        }
        at = work.next(n);
    }
    None
}

/// The block before `node`.
fn previous_block(work: &Work, node: Id) -> Option<Id> {
    let mut at = work.previous(node);
    while let Some(n) = at {
        if work.is_any(n, W, &BLOCKS) {
            return Some(n);
        }
        at = work.previous(n);
    }
    None
}

/// No table beside one already there (a separator between), and a container ending in a
/// paragraph after a table.
fn close_tables(
    sink: &mut EditSink<'_, '_>,
    work: &mut Work,
    writer: &mut Writer<'_>,
    elements: &mut Vec<Id>,
) -> Result<()> {
    let table = |work: &Work, n: Option<Id>| n.is_some_and(|n| work.is(n, W, "tbl"));
    if let Some(&first) = elements.first() {
        if table(work, Some(first)) && table(work, previous_block(work, first)) {
            let separator = writer.separator(sink, work)?;
            work.add_previous(first, separator);
            elements.insert(0, separator);
        }
    }
    if let Some(&last) = elements.last() {
        let following = next_block(work, last).filter(|n| !work.is(*n, W, "sectPr"));
        if table(work, Some(last)) && table(work, following) {
            let separator = writer.separator(sink, work)?;
            work.add_next(last, separator);
            elements.push(separator);
        }
    }
    if let Some(&last) = elements.last() {
        let following = next_block(work, last).filter(|n| !work.is(*n, W, "sectPr"));
        if table(work, Some(last)) && following.is_none() {
            let trailing = writer.paragraph(sink, work, &[], None, None)?;
            work.add_next(last, trailing);
            elements.push(trailing);
        }
    }
    Ok(())
}

/// Every new paragraph and row given its paraId and textId (`Writer.stamp`): the ids made
/// in document order, and the top-level blocks'.
fn stamp(
    sink: &mut EditSink<'_, '_>,
    work: &mut Work,
    place: &Place,
    elements: &[Id],
    continued: Option<(String, String)>,
) -> Result<(Vec<String>, Vec<String>)> {
    let part = &place.part;
    let rows: usize = elements.iter().map(|e| work.iter(*e, W, &["tr"]).len()).sum();
    let paragraphs: usize = elements
        .iter()
        .map(|e| {
            if work.is(*e, W, "p") {
                1
            } else {
                work.iter(*e, W, &["p"]).len()
            }
        })
        .sum();
    let row_ids = sink.draw(&format!("{part}\0row"), 2 * rows)?;
    let mut paragraph_ids: Vec<String> = continued.iter().flat_map(|(p, t)| [p.clone(), t.clone()]).collect();
    let skip = if continued.is_some() { 2 } else { 0 };
    paragraph_ids.extend(sink.draw(part, (2 * paragraphs).saturating_sub(skip))?);
    let (mut rows, mut paragraphs) = (row_ids.into_iter(), paragraph_ids.into_iter());
    let tree = sink.doc().tree(part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let w14 = w14_prefix(tree.root(), &place.container);
    let prefix = if place.story == "body" {
        String::new()
    } else {
        format!("{}/", place.story)
    };
    let set = |work: &mut Work, node: Id, ids: &mut dyn Iterator<Item = String>| -> Option<String> {
        let (para, text) = (ids.next()?, ids.next()?);
        work.set(node, Arc::new(QName::new(Some(&w14), "paraId", Some(W14))), &para);
        work.set(node, Arc::new(QName::new(Some(&w14), "textId", Some(W14))), &text);
        Some(para)
    };
    let (mut out, mut top) = (Vec::new(), Vec::new());
    let missing = || EditError::Path(part.clone());
    for &element in elements {
        if work.is(element, W, "p") {
            let id = format!("{prefix}p:{}", set(work, element, &mut paragraphs).ok_or_else(missing)?);
            out.push(id.clone());
            top.push(id);
            continue;
        }
        let mut first_row = None;
        let mut nested = Vec::new();
        for node in work.iter(element, W, &["p", "tr"]) {
            if work.is(node, W, "tr") {
                let id = set(work, node, &mut rows).ok_or_else(missing)?;
                first_row.get_or_insert(id);
            } else {
                nested.push(format!(
                    "{prefix}p:{}",
                    set(work, node, &mut paragraphs).ok_or_else(missing)?
                ));
            }
        }
        let id = format!("{prefix}t:{}", first_row.unwrap_or_default());
        out.push(id.clone());
        top.push(id);
        out.extend(nested);
    }
    Ok((out, top))
}

/// A new paragraph's runs inside `w:ins`: at paragraph level one per stretch, inside a
/// hyperlink within it (`_insert_content`).
fn insert_content(work: &mut Work, paragraph: Id, stamp: &mut Stamp) {
    let mut current: Option<Id> = None;
    let children: Vec<Id> = work.children(paragraph).to_vec();
    for child in children {
        if !work.is_element(child) || work.is(child, W, "pPr") {
            continue;
        }
        if work.is(child, W, "hyperlink") {
            current = None;
            let inner = stamp.make(work, "ins");
            for run in work.children(child).to_vec() {
                work.append(inner, run);
            }
            work.append(child, inner);
            continue;
        }
        let holder = match current {
            Some(h) => h,
            None => {
                let h = stamp.make(work, "ins");
                work.add_previous(child, h);
                current = Some(h);
                h
            }
        };
        work.append(holder, child);
    }
}

/// `_track_table`: every row inserted (`w:trPr/w:ins`), then its paragraphs.
fn track_table(work: &mut Work, table: Id, stamp: &mut Stamp, orders: &Orders) {
    for row in work.iter(table, W, &["tr"]) {
        let properties = match work.find(row, W, "trPr") {
            Some(p) => p,
            None => {
                let p = crate::review::xml::make(work, "trPr", &[]);
                insert_in_order(work, row, p, orders);
                p
            }
        };
        let ins = stamp.make(work, "ins");
        insert_in_order(work, properties, ins, orders);
    }
    for paragraph in work.iter(table, W, &["p"]) {
        insert_content(work, paragraph, stamp);
        mark(work, paragraph, "ins", stamp, orders);
    }
}

/// The insertion as one revision group (`Writer.track`); `old_mark`: the properties of
/// the empty paragraph typed into, whose mark the last new paragraph takes.
fn track(work: &mut Work, elements: &[Id], stamp: &mut Stamp, orders: &Orders, old_mark: Option<Id>) {
    for &element in elements {
        if work.is(element, W, "p") {
            insert_content(work, element, stamp);
            mark(work, element, "ins", stamp, orders);
        } else {
            track_table(work, element, stamp, orders);
        }
    }
    let (Some(&first), Some(&last)) = (elements.first(), elements.last()) else {
        return;
    };
    let last_in_container = work.is(last, W, "p") && {
        let mut at = work.next(last);
        let mut found = false;
        while let Some(n) = at {
            found |= work.is_any(n, W, &BLOCKS);
            at = work.next(n);
        }
        !found
    };
    if !last_in_container {
        return;
    }
    let previous = previous_block(work, first).filter(|p| {
        work.is(*p, W, "p")
            && mark_record(work, *p).is_none()
            && work
                .find(*p, W, "pPr")
                .and_then(|q| work.find(q, W, "sectPr"))
                .is_none()
    });
    let old = match (previous, old_mark) {
        (Some(previous), _) => {
            drop_mark(work, last);
            pass_mark(work, previous, stamp, orders)
        }
        (None, Some(old)) => {
            drop_mark(work, last);
            old
        }
        (None, None) => return,
    };
    crate::marks::record_paragraph_change(work, last, old, stamp, orders);
}

/// A paragraph's mark record removed, and its mark's `w:rPr` when that leaves it empty.
fn drop_mark(work: &mut Work, paragraph: Id) {
    let Some(record) = mark_record(work, paragraph) else {
        return;
    };
    let run_properties = work.parent(record);
    work.remove(record);
    if let Some(rp) = run_properties.filter(|r| work.children(*r).iter().all(|c| !work.is_element(*c))) {
        work.remove(rp);
    }
}
