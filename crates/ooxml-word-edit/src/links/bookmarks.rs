//! Owns: [`AddBookmark`], a bookmark added as docx-agent's `Document.add_bookmark` adds
//! one: over a text range (an empty one marks a place), or over blocks as Word's Insert >
//! Bookmark marks a selection of them (`_bookmark_place`, `_table_end`, `_control_end`);
//! a bookmark of that name marking this text already is no change (rule 3); its markers'
//! `w:id` one above the largest annotation id in use, drawn again where it is committed.
//!
//! Must not: write but through the sink; it reads every story (names are unique in the
//! document).

use super::{marker_draws, new_w, LinkEdited};
use crate::blocks::{blocks_of, resolve_block, BlockAt, BlockKind};
use crate::delete::{editable, stretch_keys};
use crate::doc::Key;
use crate::names::W;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::text::atoms;
use crate::text_range::{range, range_id, Span};
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::marks::{bookmark_markers, bookmark_range};
use ooxml_word_model::address::{expand_heading, is_text_range, span_ends, RangeText};
use ooxml_word_model::docx::{at_path, casefold, Docx};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// A bookmark over text or blocks.
#[derive(Debug, Clone)]
pub struct AddBookmark {
    /// What it marks: a range, a position, a paragraph, a cell, a table, a content
    /// control, a span of blocks or a heading's range.
    pub place: String,
    /// Its name.
    pub name: String,
}

/// Where a bookmark goes: the text it marks, and the blocks its markers stand beside.
struct Place {
    range: RangeText,
    /// The content control its start goes before.
    first_block: Option<BlockAt>,
    /// The table or content control its end goes after.
    last_block: Option<BlockAt>,
}

/// The text a bookmark over `address` marks (docx-agent's `_bookmark_place`).
fn place(doc: &WordDoc, docx: &Docx, address: &str) -> Result<Place> {
    let address = expand_heading(docx, address).map_err(EditError::Address)?;
    let elements = if span_ends(&address).map_err(EditError::Address)?.is_some() {
        Some(blocks_of(doc, docx, &address)?)
    } else if !is_text_range(&address) {
        match resolve_block(doc, docx, &address) {
            Ok((part, block)) if block.kind != BlockKind::Paragraph => Some((part, vec![block])),
            _ => None,
        }
    } else {
        None
    };
    let Some((part, blocks)) = elements else {
        return Ok(Place {
            range: range(doc, docx, &address)?,
            first_block: None,
            last_block: None,
        });
    };
    let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
    let (Some(first), Some(last)) = (blocks.first(), blocks.last()) else {
        return Err(EditError::Refused(format!("{address} holds no paragraph to mark")));
    };
    let paragraphs_of = |block: &BlockAt| {
        if block.kind == BlockKind::Paragraph {
            story.ids.paragraph_at(&block.path).into_iter().collect::<Vec<_>>()
        } else {
            story.ids.paragraphs_within(&block.path)
        }
    };
    let mut found = paragraphs_of(first);
    found.extend(paragraphs_of(last));
    let (Some(a), Some(b)) = (found.first(), found.last()) else {
        return Err(EditError::Refused(format!("{address} holds no paragraph to mark")));
    };
    let length = {
        let work = Work::of(&b.element, Vec::new());
        atoms(&work, work.root()).len()
    };
    Ok(Place {
        range: RangeText {
            start_id: a.id.clone(),
            start: 0,
            end_id: b.id.clone(),
            end: length,
        },
        first_block: (first.kind == BlockKind::Control).then(|| first.clone()),
        last_block: matches!(last.kind, BlockKind::Table | BlockKind::Control).then(|| last.clone()),
    })
}

/// Whether two ranges span the same text: the same paragraphs and offsets.
fn same_range(doc: &WordDoc, docx: &Docx, one: &RangeText, other: &RangeText) -> bool {
    let (Ok(a), Ok(b)) = (Span::of(doc, docx, one), Span::of(doc, docx, other)) else {
        return false;
    };
    let ends = |s: &Span| (s.first().map(|f| f.path.clone()), s.last().map(|f| f.path.clone()));
    ends(&a) == ends(&b) && (a.start, a.end) == (b.start, b.end)
}

impl Command<WordDoc> for AddBookmark {
    type Out = LinkEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        super::check_name(&self.name)?;
        place(doc, &*doc.docx()?, &self.place).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<LinkEdited> {
        let mut sink = EditSink::new(tx, crate::RevisionPolicy::Untracked);
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl AddBookmark {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<LinkEdited> {
        sink.read_everything();
        super::check_name(&self.name)?;
        let docx = sink.docx()?;
        let place = place(sink.doc(), &docx, &self.place)?;
        let wanted = casefold(&self.name);
        let markers = bookmark_markers(&docx);
        if let Some(existing) = markers.iter().find(|b| casefold(&b.name) == wanted) {
            let marked = bookmark_range(&docx, existing).map_err(EditError::Address)?;
            if same_range(sink.doc(), &docx, &marked, &place.range) {
                let id = format!("bm:{}", existing.name);
                return Ok(LinkEdited {
                    warnings: vec![format!("{id} marks this text already")],
                    id: Some(id),
                    ..LinkEdited::default()
                });
            }
            return Err(EditError::Refused(format!(
                "the document already has a bookmark {}",
                repr_str(&self.name)
            )));
        }
        let span = Span::of(sink.doc(), &docx, &place.range)?;
        let (Some(first), Some(last)) = (span.first(), span.last()) else {
            return Err(EditError::Path(span.part.clone()));
        };
        let length = |element: &Arc<Element>| {
            let work = Work::of(element, Vec::new());
            atoms(&work, work.root()).len()
        };
        if span.start > length(&first.element) || span.end > length(&last.element) {
            return Err(EditError::Refused(format!(
                "{} is outside its paragraphs' text",
                range_id(&place.range)
            )));
        }
        let (part, first_path, last_path) = (span.part.clone(), first.path.to_vec(), last.path.to_vec());
        let renamed = stamp_document(&mut *sink, &docx)?;
        let (value, offset) = sink.marker_id()?;
        let docx = sink.docx()?;
        let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
        let root = tree.root();
        let start_anchor = place.first_block.as_ref().map_or(first_path, |b| b.path.clone());
        let end_anchor = place.last_block.as_ref().map_or(last_path, |b| b.path.clone());
        let mut paths = vec![start_anchor.clone(), end_anchor.clone()];
        let next = place.last_block.as_ref().and_then(|b| next_element(root, &b.path));
        paths.extend(next.clone());
        let mut wanted: Vec<&[u32]> = vec![&start_anchor, &end_anchor];
        if let Some(next) = &next {
            wanted.push(next);
        }
        let path_error = || EditError::Path(part.clone());
        let (mut work, parent, stretch, found) = editable(root, &paths, &wanted).ok_or_else(path_error)?;
        let (&start_node, &end_node) = (
            found.first().ok_or_else(path_error)?,
            found.get(1).ok_or_else(path_error)?,
        );
        let marks = Marks {
            id: value.to_string(),
            name: self.name.clone(),
        };
        let end_kind = place.last_block.as_ref().map(|b| b.kind);
        let end = marks.end(&mut work, end_node, end_kind, found.get(2).copied(), span.end)?;
        marks.start(&mut work, start_node, place.first_block.is_some(), span.start, end)?;
        let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
        let draws = marker_draws(&nodes, &["bookmarkStart", "bookmarkEnd"], value, offset);
        let mut keys = stretch_keys(&docx, &part, &parent, stretch);
        keys.push(Key::Annotations);
        let count = (stretch.1 - stretch.0) as usize + 1;
        sink.splice_children(&part, (&parent, stretch.0 as usize, count), nodes, keys, draws)?;
        let id = format!("bm:{}", self.name);
        Ok(LinkEdited {
            id: Some(id.clone()),
            changed: true,
            created: vec![id],
            renamed,
            ..LinkEdited::default()
        })
    }
}

/// The new bookmark's markers.
struct Marks {
    id: String,
    name: String,
}

impl Marks {
    /// The end, put at `offset` of the paragraph `node` -- or after the table or control
    /// `node` is, as Word puts it (`_table_end`, `_control_end`; `next`: the element after
    /// it).
    fn end(&self, work: &mut Work, node: Id, block: Option<BlockKind>, next: Option<Id>, offset: usize) -> Result<Id> {
        let (parent, index) = match block {
            None => crate::inline::position(work, node, offset)?,
            Some(BlockKind::Table) => {
                let text_follows = next.is_some_and(|n| work.is(n, W, "p") && !atoms(work, n).is_empty());
                match work.parent(node).filter(|_| text_follows) {
                    Some(parent) => (parent, work.index(node).map_or(0, |k| k + 1)),
                    None => (node, work.children(node).len()),
                }
            }
            Some(_) => match next.filter(|n| work.is(*n, W, "tbl")) {
                Some(table) => match work.iter(table, W, &["tc"]).first().copied() {
                    Some(cell) => {
                        let at = work
                            .find(cell, W, "tcPr")
                            .and_then(|p| work.index(p))
                            .map_or(0, |k| k + 1);
                        (cell, at)
                    }
                    None => after(work, node),
                },
                None => after(work, node),
            },
        };
        let end = new_w(work, parent, "bookmarkEnd", &[("id", &self.id)]);
        work.insert(parent, index, end);
        Ok(end)
    }

    /// The start, put at `offset` of the paragraph `node` -- or before the content control
    /// `node` is (`before_block`), displaced by it -- and never after the end.
    fn start(&self, work: &mut Work, node: Id, before_block: bool, offset: usize, end: Id) -> Result<()> {
        let (parent, mut index) = if before_block {
            let parent = work
                .parent(node)
                .ok_or_else(|| EditError::Path("the control's parent".to_owned()))?;
            (parent, work.index(node).unwrap_or(0))
        } else {
            crate::inline::position(work, node, offset)?
        };
        let mut attributes = vec![("id", self.id.as_str()), ("name", self.name.as_str())];
        if before_block {
            attributes.push(("displacedByCustomXml", "next"));
        }
        let start = new_w(work, parent, "bookmarkStart", &attributes);
        if work.parent(end) == Some(parent) {
            if let Some(at) = work.index(end).filter(|at| index > *at) {
                index = at;
            }
        }
        work.insert(parent, index, start);
        Ok(())
    }
}

/// Right after `node` in its parent.
fn after(work: &Work, node: Id) -> (Id, usize) {
    let parent = work.parent(node).unwrap_or(node);
    (parent, work.index(node).map_or(0, |k| k + 1))
}

/// The place of the element after the one at `path`, past range markers and nodes that
/// are not elements.
fn next_element(root: &Arc<Element>, path: &[u32]) -> Option<Vec<u32>> {
    let (&index, above) = path.split_last()?;
    let parent = if above.is_empty() { root } else { at_path(root, above)? };
    let markers = [
        "bookmarkStart",
        "bookmarkEnd",
        "commentRangeStart",
        "commentRangeEnd",
        "moveFromRangeStart",
        "moveFromRangeEnd",
        "moveToRangeStart",
        "moveToRangeEnd",
        "permStart",
        "permEnd",
        "proofErr",
    ];
    parent
        .children()
        .iter()
        .enumerate()
        .skip(index as usize + 1)
        .find_map(|(k, node)| match node {
            Node::Element(e) if !(e.name().namespace() == Some(W) && markers.contains(&e.name().local())) => {
                let mut out = above.to_vec();
                out.push(u32::try_from(k).ok()?);
                Some(out)
            }
            _ => None,
        })
}
