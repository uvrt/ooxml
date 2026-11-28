//! Owns: where insert_markdown's blocks go and how they are put there (docx-agent's
//! `_lone_empty_paragraph`, `_insert`, `Writer.stamp`, `Writer.track`): a body that is one
//! empty paragraph typed into (the place itself is [`crate::placing`]'s), blocks replaced
//! (`replace:`: deleted by [`crate::replacing`], the new paragraphs keeping their settings
//! by [`super::keep`], the list instances no paragraph names any more removed), separators and a
//! closing paragraph, paraIds and textIds drawn in document order (rows from their own
//! sequence), and the insertion tracked as one revision group in Word's typing form.
//!
//! Must not: write but through the sink.

use super::writer::Writer;
use super::{InsertMarkdown, KeptSettings, MarkdownInserted};
use crate::format::list_indent::ListIndent;
use crate::marks::{clean_properties, insert_in_order, mark, mark_record, pass_mark};
use crate::names::{W, W14};
use crate::placing::{Place, Stretch, BLOCKS};
use crate::policy::{RevisionPolicy, Stamp};
use crate::replacing::{Order, Replaced};
use crate::sink::EditSink;
use crate::stamping::w14_prefix;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::address::{Place as Where, PlaceKind};
use ooxml_word_model::docx::formatting::Formatting;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::valid_long_hex;
use ooxml_word_model::markdown::model::Block;
use ooxml_xml::{Element, QName};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Where the blocks go, and the body's one empty paragraph typed into (its id and node
/// index).
pub(crate) struct Located {
    place: Place,
    lone: Option<(String, u32)>,
    /// The blocks `replace:` names, in order.
    replaced: Vec<String>,
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
/// A place that does not exist, a block replaced that is neither a paragraph, a table nor a
/// content control, a story of notes or comments.
pub(crate) fn locate(doc: &WordDoc, docx: &Docx, at: &Where, blocks: &[Block]) -> Result<Located> {
    if let PlaceKind::Replace(named) = at.kind() {
        let (place, replaced) = crate::placing::replacing(doc, docx, named)?;
        return Ok(Located {
            place,
            lone: None,
            replaced,
        });
    }
    let lone = if at.is_body_end() && !matches!(blocks.first(), Some(Block::Table(_))) {
        lone_empty_paragraph(docx)
    } else {
        None
    };
    if let Some((id, path)) = lone {
        let index = path.last().copied().unwrap_or_default();
        return Ok(Located {
            place: Place {
                part: docx.main.clone(),
                story: "body".to_owned(),
                container: path.get(..1).unwrap_or_default().to_vec(),
                before: Some(index),
            },
            lone: (!matches!(blocks.last(), Some(Block::Table(_)))).then_some((id, index)),
            replaced: Vec::new(),
        });
    }
    Ok(Located {
        place: crate::placing::locate(doc, docx, at, "insert_markdown")?,
        lone: None,
        replaced: Vec::new(),
    })
}

/// Write the blocks at their place (`_insert`).
pub(crate) fn insert(
    sink: &mut EditSink<'_, '_>,
    command: &InsertMarkdown,
    blocks: &[Block],
) -> Result<MarkdownInserted> {
    let docx = sink.docx()?;
    let Located {
        mut place,
        lone,
        replaced,
    } = locate(sink.doc(), &docx, &command.at, blocks)?;
    // the replaced paragraphs read before anything changes
    let donors = super::keep::donors(&docx, &place.part, &replaced);
    let part = place.part.clone();
    let tracked = matches!(sink.policy(), RevisionPolicy::Tracked(_));
    let mut replacing = Replaced::begin(sink, &mut place, replaced, tracked, Order::After)?;
    let mut stretch = Stretch::open(sink, &place, "insert_markdown")?;
    let anchor = stretch.anchor(&place);
    let lone = lone.and_then(|(id, k)| Some((id, stretch.child(k)?)));
    let work = &mut stretch.work;
    let mut removed = Vec::new();
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
        old_mark = Some(clean_properties(work, properties, *node));
        insert_before = next_block(work, *node);
        work.remove(*node);
        removed.push(id.clone());
    }
    let mut writer = Writer::new(&part, &command.style_map, command.options.clone());
    if writer.like.any() {
        writer.like.followed_at(position_of(work, insert_before, &docx));
    }
    let mut elements = writer.blocks(sink, work, blocks)?;
    let (kept, unnamed) = keep_settings(sink, work, &writer, &donors)?;
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
    close_tables(sink, work, &mut writer, &mut elements)?;
    let starts_paragraph = elements.first().is_some_and(|e| work.is(*e, W, "p"));
    let stamped = stamp(sink, work, &place, &elements, continued.filter(|_| starts_paragraph))?;
    let wrote = sink.track(work, |work, stamp, orders| {
        track(work, &elements, stamp, orders, old_mark);
        Vec::new()
    })?;
    // the tables' first rows' ids, now stamped: the tables found again where the edit lands
    let written_tables = table_ids(work, &writer);
    crate::placing::commit(sink, &stretch, &place, &stamped.0, &wrote, false)?;
    let cell_warnings = cell_indents(sink, &part, &written_tables)?;
    let (created, top) = stamped;
    let kept = said(&stretch.work, kept);
    let per_block: Vec<Vec<String>> = top.iter().map(|id| vec![id.clone()]).collect();
    replacing.finish(sink, tracked, &per_block)?;
    removed.extend(replacing.removed);
    crate::lists::numbering::reap(sink, &docx.main, &unnamed)?;
    writer.warnings.extend(cell_warnings);
    Ok(MarkdownInserted {
        first: created.first().cloned(),
        changed: true,
        created,
        removed,
        warnings: writer.warnings,
        blocks: top,
        kept,
    })
}

/// The replaced paragraphs' settings kept by the writer's new ones ([`super::keep`]), and the
/// list instances these stopped naming; nothing where nothing is replaced.
fn keep_settings(
    sink: &EditSink<'_, '_>,
    work: &mut Work,
    writer: &Writer<'_>,
    donors: &[super::keep::Donor],
) -> Result<(Vec<super::keep::Kept>, BTreeSet<i64>)> {
    if donors.is_empty() {
        return Ok((Vec::new(), BTreeSet::new()));
    }
    let shared = sink.doc().shared_handle();
    let docx = sink.docx()?;
    Ok(super::keep::keep_replaced(
        work,
        &writer.tagged,
        donors,
        (writer.map, &docx, shared.facts.orders()),
    ))
}

/// What each new paragraph kept, by the id it was stamped with, and the replaced one's id.
fn said(work: &Work, kept: Vec<super::keep::Kept>) -> Vec<KeptSettings> {
    kept.into_iter()
        .filter_map(|k| {
            let block = format!("p:{}", work.get(k.node, Some(W14), "paraId")?);
            Some(KeptSettings {
                block,
                from: k.from,
                settings: k.settings,
            })
        })
        .collect()
}

/// The tables the writer made and kept, each by its id (`t:` and its first row's
/// `w14:paraId`, stamped) with its grid shares.
fn table_ids(work: &Work, writer: &Writer<'_>) -> Vec<(String, Vec<i64>)> {
    writer
        .tables
        .iter()
        .filter(|(node, _)| work.parent(*node).is_some())
        .filter_map(|(node, shares)| {
            let row = work.find(*node, W, "tr")?;
            Some((format!("t:{}", work.get(row, Some(W14), "paraId")?), shares.clone()))
        })
        .collect()
}

/// The tables the writer made whose cell styles' indents do not fit their cells
/// ([`super::cell_indents`]), read on the version the edit lands on (its transaction's
/// document as the edit leaves it: under a rebase, the document as committed).
fn cell_indents(sink: &EditSink<'_, '_>, part: &str, tables: &[(String, Vec<i64>)]) -> Result<Vec<String>> {
    if tables.is_empty() {
        return Ok(Vec::new());
    }
    let docx = sink.docx()?;
    let Some(story) = docx.part(part) else {
        return Ok(Vec::new());
    };
    let placed: Vec<(Vec<u32>, Vec<i64>)> = tables
        .iter()
        .filter_map(|(id, shares)| {
            let table = story.ids.tables().iter().find(|t| &t.id == id)?;
            Some((table.path.to_vec(), shares.clone()))
        })
        .collect();
    Ok(super::cell_indents::cell_indent_warnings(&docx, part, &placed))
}

/// The position of the paragraph `node`, where the new blocks go before one (its first line
/// and text are stops for an item moved in a one-level list, [`crate::format::list_indent`]).
fn position_of(work: &Work, node: Option<Id>, docx: &Docx) -> Option<ListIndent> {
    let node = node.filter(|n| work.is(*n, W, "p"))?;
    let paragraph = work.build_element(node)?;
    Some(ListIndent::of(&Formatting::read(docx).paragraph(&paragraph, None)))
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
    let tree = sink.doc().tree(part)?.ok_or_else(|| EditError::no_part(part.clone()))?;
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
    let missing = || EditError::no_element(part.clone());
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
pub(crate) fn insert_content(work: &mut Work, paragraph: Id, stamp: &mut Stamp) {
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
pub(crate) fn track_table(work: &mut Work, table: Id, stamp: &mut Stamp, orders: &Orders) {
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

/// The insertion as one revision group (`Writer.track`, and the importer's `_track`):
/// a paragraph's content and mark, a table's rows and paragraphs, each paragraph of any
/// other block (a content control); `old_mark`: the properties of the empty paragraph
/// typed into, whose mark the last new paragraph takes.
pub(crate) fn track(work: &mut Work, elements: &[Id], stamp: &mut Stamp, orders: &Orders, old_mark: Option<Id>) {
    for &element in elements {
        if work.is(element, W, "p") {
            insert_content(work, element, stamp);
            mark(work, element, "ins", stamp, orders);
        } else if work.is(element, W, "tbl") {
            track_table(work, element, stamp, orders);
        } else {
            for paragraph in work.iter(element, W, &["p"]) {
                insert_content(work, paragraph, stamp);
                mark(work, paragraph, "ins", stamp, orders);
            }
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
