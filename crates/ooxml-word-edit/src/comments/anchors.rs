//! Owns: where a new comment is anchored in its story, as docx-agent's `add_comment`
//! anchors one -- over a text range (an empty one comments on a place), or over a whole
//! table or row as Word anchors one when the table (or row) is selected
//! (`_add_block_comment`), or over a whole content control ([`super::control`]):
//! `w:commentRangeStart`, `w:commentRangeEnd` and the reference run after the end, in the
//! Comment Reference style at the size of the text it follows.
//!
//! Must not: write but into the edit's copy of the story ([`Space`]).

use super::control::{control, mark_control, Control};
use super::size::Sizes;
use crate::names::W;
use crate::review::space::Space;
use crate::review::xml::make;
use crate::text::atoms;
use crate::text_range::{range, range_id, Span};
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::address::{in_table, resolve};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;

/// What a comment is anchored to.
pub(super) enum Anchor {
    /// A text range: its part, its first and last paragraphs' places, its offsets, and
    /// whether it is empty.
    Range {
        part: String,
        first: Vec<u32>,
        last: Vec<u32>,
        start: usize,
        end: usize,
        collapsed: bool,
    },
    /// A whole table or row: its part, its place, its id, and whether it is a row.
    Block {
        part: String,
        path: Vec<u32>,
        id: String,
        row: bool,
    },
    /// A whole content control.
    Control(Control),
}

/// What `place` anchors a comment to: a whole table or row (`t:...`, `tr:...`, `t:X/r1`;
/// `_commented_block`), else the text range it names.
///
/// # Errors
/// The range's refusal; a range outside its paragraphs' text.
pub(super) fn anchor(doc: &WordDoc, docx: &Docx, place: &str) -> Result<Anchor> {
    if place.starts_with("cc:") || place.starts_with("cc@") {
        return control(docx, place).map(Anchor::Control);
    }
    if let Some(block) = block(doc, docx, place) {
        return Ok(block);
    }
    let found = range(doc, docx, place)?;
    let span = Span::of(doc, docx, &found)?;
    let (Some(first), Some(last)) = (span.first(), span.last()) else {
        return Err(EditError::Path(span.part.clone()));
    };
    let length = |element: &std::sync::Arc<ooxml_xml::Element>| {
        let work = Work::of(element, Vec::new());
        atoms(&work, work.root()).len()
    };
    if span.start > length(&first.element) || span.end > length(&last.element) {
        return Err(EditError::Refused(format!(
            "{} is outside its paragraphs' text",
            range_id(&found)
        )));
    }
    Ok(Anchor::Range {
        part: span.part.clone(),
        first: first.path.to_vec(),
        last: last.path.to_vec(),
        start: span.start,
        end: span.end,
        collapsed: found.start_id == found.end_id && found.start == found.end,
    })
}

/// The table or row `place` names, if it names one.
fn block(doc: &WordDoc, docx: &Docx, place: &str) -> Option<Anchor> {
    if place.contains('@') || place.contains("..") {
        return None;
    }
    let followed = doc.ids().follow(place);
    if let Ok(Some(found)) = in_table(docx, &followed) {
        if found.column.is_some() {
            return None;
        }
        let table = found.part.ids.tables().get(found.table)?;
        let (path, id) = table.rows.get(found.row)?;
        return Some(Anchor::Block {
            part: found.part.part.clone(),
            path: path.to_vec(),
            id: id.clone(),
            row: true,
        });
    }
    match resolve(docx, &followed).ok()? {
        (part, Entry::Table(t)) => {
            let table = part.ids.tables().get(t)?;
            Some(Anchor::Block {
                part: part.part.clone(),
                path: table.path.to_vec(),
                id: table.id.clone(),
                row: false,
            })
        }
        (_, Entry::Paragraph(_)) => None,
    }
}

/// Write comment `id`'s markers and reference run where `anchor` says, in `space`'s copy
/// of its part (the reference sized by `sizes`, styled `reference`).
///
/// # Errors
/// A table or row with no paragraph in it, or none after it; a place that is gone.
pub(super) fn mark(
    space: &mut Space,
    doc: &WordDoc,
    anchor: &Anchor,
    sizes: &Sizes<'_>,
    (id, reference): (i64, &str),
) -> Result<()> {
    let part = match anchor {
        Anchor::Range { part, .. } | Anchor::Block { part, .. } => part.clone(),
        Anchor::Control(found) => return mark_control(space, doc, found, sizes, (id, reference)),
    };
    copy_anchored(space, doc, &part, anchor)?;
    let gone = || EditError::Path(part.clone());
    match anchor {
        Anchor::Range {
            first,
            last,
            start,
            end,
            collapsed,
            ..
        } => {
            let (first, last) = (
                space.locate(&part, first).ok_or_else(gone)?,
                space.locate(&part, last).ok_or_else(gone)?,
            );
            let size = size_at(space, &part, last, *end, sizes);
            let work = space.get_mut(&part).ok_or_else(gone)?;
            let start_marker = make(work, "commentRangeStart", &[("id", &id.to_string())]);
            let end_marker = make(work, "commentRangeEnd", &[("id", &id.to_string())]);
            // the start first: placed before the character at the start, it moves no
            // character, so the end (after the last one) lands after it, even on a place
            let (parent, index) = crate::inline::position(work, first, *start)?;
            work.insert(parent, index, start_marker);
            if *collapsed {
                work.add_next(start_marker, end_marker);
            } else {
                let (parent, index) = crate::inline::position(work, last, *end)?;
                work.insert(parent, index, end_marker);
            }
            let run = reference_run(work, id, reference, size);
            work.add_next(end_marker, run);
        }
        Anchor::Block {
            path, id: name, row, ..
        } => {
            let element = space.locate(&part, path).ok_or_else(gone)?;
            let work = space.get(&part).ok_or_else(gone)?;
            let end_after = end_after(work, element, *row).ok_or_else(gone)?;
            let first = work.iter(element, W, &["p"]).first().copied();
            let following = following_paragraph(work, end_after);
            let Some(first) = first else {
                return Err(EditError::Refused(format!(
                    "{name} has no paragraph to anchor a comment at"
                )));
            };
            let Some(following) = following else {
                return Err(EditError::Refused(format!(
                    "{name} has no paragraph after it for the comment's mark"
                )));
            };
            let size = size_at(space, &part, following, 0, sizes);
            let work = space.get_mut(&part).ok_or_else(gone)?;
            let start_marker = make(work, "commentRangeStart", &[("id", &id.to_string())]);
            let end_marker = make(work, "commentRangeEnd", &[("id", &id.to_string())]);
            let (parent, index) = crate::inline::position(work, first, 0)?;
            work.insert(parent, index, start_marker);
            work.add_next(end_after, end_marker);
            let (parent, index) = crate::inline::position(work, following, 0)?;
            let run = reference_run(work, id, reference, size);
            work.insert(parent, index, run);
        }
        Anchor::Control(_) => {}
    }
    Ok(())
}

/// Copy into `space` what marking `anchor` (a range, a table or a row) changes of `part`:
/// the stretch of its story's blocks from the anchor's first to its last -- for a table or
/// row, on to the first block after it with a paragraph, which takes the reference -- else
/// (no stretch to be had) the whole part.
///
/// # Errors
/// When the part is not XML.
fn copy_anchored(space: &mut Space, doc: &WordDoc, part: &str, anchor: &Anchor) -> Result<()> {
    let docx = doc.docx()?;
    let Some(story) = docx.part(part) else {
        space.work(doc, part)?;
        return Ok(());
    };
    let container = story.ids.container().to_vec();
    let (paths, through): (Vec<&[u32]>, Option<u32>) = match anchor {
        Anchor::Range { first, last, .. } => (vec![first.as_slice(), last.as_slice()], None),
        Anchor::Block { path, .. } => {
            let top = path
                .strip_prefix(container.as_slice())
                .and_then(|rest| rest.first().copied());
            (
                vec![path.as_slice()],
                top.and_then(|top| paragraph_after(story.root(), &container, top)),
            )
        }
        Anchor::Control(_) => (Vec::new(), None),
    };
    if paths.is_empty() || !space.stretch(doc, part, &container, &paths, through)? {
        space.work(doc, part)?;
    }
    Ok(())
}

/// The index of the first child of the container at `container` after its `top`th that
/// is or holds a paragraph; with none, its last child.
fn paragraph_after(root: &std::sync::Arc<ooxml_xml::Element>, container: &[u32], top: u32) -> Option<u32> {
    let parent = if container.is_empty() {
        root
    } else {
        ooxml_word_model::docx::at_path(root, container)?
    };
    let children = parent.children();
    let found = (top as usize + 1..children.len()).find(|&k| {
        children
            .get(k)
            .and_then(ooxml_xml::Node::as_element)
            .is_some_and(|e| holds_paragraph(e))
    });
    u32::try_from(found.unwrap_or(children.len().saturating_sub(1))).ok()
}

/// Whether `element` is a paragraph or has one below it.
fn holds_paragraph(element: &ooxml_xml::Element) -> bool {
    element.name().is(Some(W), "p") || element.child_elements().any(|e| holds_paragraph(e))
}

/// What a comment on the table or row `element` ends after (`_add_block_comment`): a
/// row's `w:tr` with rows after it, else the table's `w:tbl`.
fn end_after(work: &Work, element: Id, row: bool) -> Option<Id> {
    if !row {
        return Some(element);
    }
    let later = work
        .parent(element)
        .map(|p| work.children(p).to_vec())
        .unwrap_or_default()
        .into_iter()
        .skip_while(|c| *c != element)
        .skip(1)
        .any(|c| work.is(c, W, "tr"));
    if later {
        Some(element)
    } else {
        work.parent(element)
    }
}

/// The size of the text before `offset` of the paragraph `paragraph` of `space`'s copy of
/// `part`.
fn size_at(space: &Space, part: &str, paragraph: Id, offset: usize, sizes: &Sizes<'_>) -> Option<i64> {
    let path = space.path_of(part, paragraph)?;
    let element = space.get(part)?.build_element(paragraph)?;
    sizes.at(part, &path, &element, offset)
}

/// A comment's reference run (`_reference_run`): in the Comment Reference style, at `size`
/// half-points where known.
pub(super) fn reference_run(work: &mut Work, id: i64, style: &str, size: Option<i64>) -> Id {
    let run = make(work, "r", &[]);
    let properties = make(work, "rPr", &[]);
    let styled = make(work, "rStyle", &[("val", style)]);
    work.append(properties, styled);
    if let Some(size) = size {
        let size = size.to_string();
        for local in ["sz", "szCs"] {
            let node = make(work, local, &[("val", &size)]);
            work.append(properties, node);
        }
    }
    work.append(run, properties);
    let reference = make(work, "commentReference", &[("id", &id.to_string())]);
    work.append(run, reference);
    run
}

/// The first paragraph after `node` in document order, not inside it: in a later sibling,
/// else after its parent (`_following_paragraph`).
fn following_paragraph(work: &Work, node: Id) -> Option<Id> {
    let mut at = Some(node);
    while let Some(here) = at {
        if work.is(here, W, "body") {
            break;
        }
        let parent = work.parent(here)?;
        let siblings = work.children(parent);
        let k = siblings.iter().position(|c| *c == here)?;
        for &sibling in siblings.iter().skip(k + 1) {
            if !work.is_element(sibling) {
                continue;
            }
            if work.is(sibling, W, "p") {
                return Some(sibling);
            }
            if let Some(found) = work.iter(sibling, W, &["p"]).first() {
                return Some(*found);
            }
        }
        at = Some(parent);
    }
    None
}
