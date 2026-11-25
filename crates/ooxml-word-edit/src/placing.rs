//! Owns: where new blocks go and how the stretch of their container is edited, as
//! docx-agent's `markdown/write.py` finds and fills a place (`_locate`, `_place`) for the
//! commands that write whole blocks (insert_markdown, copy_from): a story's end (before a
//! body's final section properties), after or before a block; the stretch of the container
//! an edit holds (the blocks beside the place) as an editable copy; and that stretch spliced
//! back with the keys it read and wrote.
//!
//! Must not: decide what the blocks are, or write but through the sink.

use crate::blocks::BlockKind;
use crate::doc::Key;
use crate::names::{W, W16DU};
use crate::policy::RevisionPolicy;
use crate::set_text::{block_key, paragraph_has_annotations};
use crate::sink::{annotation_draws, EditSink, TextWrite};
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::address::{Address, Place as Where, PlaceKind};
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// The block elements a container holds.
pub(crate) const BLOCKS: [&str; 4] = ["p", "tbl", "sdt", "customXml"];

/// Where the blocks go.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    /// The story part.
    pub(crate) part: String,
    /// Its story's name.
    pub(crate) story: String,
    /// The container's path from the part's root.
    pub(crate) container: Vec<u32>,
    /// The child (node index) they go before; none: the end.
    pub(crate) before: Option<u32>,
}

/// Whether `e` is a block a container holds.
pub(crate) fn is_block(e: &Element) -> bool {
    e.name().namespace() == Some(W) && BLOCKS.contains(&e.name().local())
}

/// The place `at` names (`_locate`): `end` or `end:<story>`, `after:<id>`, `before:<id>`,
/// a bare id (after it). `doing` names the edit in the refusal of `replace:` (an edit that
/// replaces blocks reads its place with [`replacing`]).
///
/// # Errors
/// A place that does not exist, `replace:` (not ported yet), a story of notes or comments.
pub(crate) fn locate(doc: &WordDoc, docx: &Docx, at: &Where, doing: &str) -> Result<Place> {
    let (block, after) = match at.kind() {
        PlaceKind::End(story) => return end_of(docx, story.as_deref().unwrap_or("body")),
        PlaceKind::Replace(_) => return Err(EditError::Unsupported(format!("{doing} at replace:"))),
        PlaceKind::After(block) | PlaceKind::At(block) => (block, true),
        PlaceKind::Before(block) => (block, false),
    };
    {
        {
            let (part, found) = crate::blocks::resolve_block(doc, docx, block.as_str())?;
            let story = docx.part(&part).map(|p| p.story.clone()).unwrap_or_default();
            let (&index, container) = found
                .path
                .split_last()
                .ok_or_else(|| EditError::no_element(part.clone()))?;
            Ok(Place {
                part,
                story,
                container: container.to_vec(),
                before: Some(if after { index + 1 } else { index }),
            })
        }
    }
}

/// `replace:<id>` or `replace:<id>..<id>` (`_locate`'s): the place before the first block
/// named, and each block's id in order (a content control's `cc:` id).
///
/// # Errors
/// The address's refusal; a block neither a paragraph, a table nor a content control.
pub(crate) fn replacing(doc: &WordDoc, docx: &Docx, blocks: &Address) -> Result<(Place, Vec<String>)> {
    let (part, found) = crate::blocks::span_or_block(doc, docx, blocks)?;
    let story = docx.part(&part).ok_or_else(|| EditError::no_part(part.clone()))?;
    let mut ids = Vec::with_capacity(found.len());
    let mut listed = None;
    for block in &found {
        let id = match block.kind {
            BlockKind::Paragraph => story.ids.paragraph_at(&block.path).map(|p| p.id),
            BlockKind::Table => story
                .ids
                .tables()
                .iter()
                .find(|t| *t.path == *block.path)
                .map(|t| t.id.clone()),
            BlockKind::Control => {
                let all = listed.get_or_insert_with(|| controls(docx));
                all.iter()
                    .find(|c| c.part == part && *c.path == *block.path)
                    .map(|c| c.id.clone())
            }
            BlockKind::Custom => None,
        };
        ids.push(id.ok_or_else(|| {
            EditError::Refused("a block replaced must be a paragraph, a table or a content control".to_owned())
        })?);
    }
    let first = found.first().ok_or_else(|| EditError::no_element(part.clone()))?;
    let (&index, container) = first
        .path
        .split_last()
        .ok_or_else(|| EditError::no_element(part.clone()))?;
    let place = Place {
        story: story.story.clone(),
        container: container.to_vec(),
        before: Some(index),
        part,
    };
    Ok((place, ids))
}

/// A story's end: before a body's final section properties.
fn end_of(docx: &Docx, story: &str) -> Result<Place> {
    let found = docx
        .story(story)
        .ok_or_else(|| EditError::Refused(format!("no story {}", repr_str(story))))?;
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

/// The stretch of a place's container an edit holds, as an editable copy.
pub(crate) struct Stretch {
    /// The copy: its root stands for the container, its children for the stretch.
    pub(crate) work: Work,
    /// The node each child of the stretch was read as.
    held: Vec<Id>,
    /// The stretch's first and last child (node indexes in the container).
    pub(crate) from: u32,
    pub(crate) to: u32,
}

impl Stretch {
    /// The stretch around `place`; `empty` names the edit in the refusal of an empty
    /// container.
    ///
    /// # Errors
    /// No such part or path; an empty container (not ported).
    pub(crate) fn open(sink: &EditSink<'_, '_>, place: &Place, empty: &str) -> Result<Stretch> {
        let part = &place.part;
        let tree = sink.doc().tree(part)?.ok_or_else(|| EditError::no_part(part.clone()))?;
        let path_error = || EditError::no_element(part.clone());
        let container = if place.container.is_empty() {
            Arc::clone(tree.root())
        } else {
            Arc::clone(ooxml_word_model::docx::at_path(tree.root(), &place.container).ok_or_else(path_error)?)
        };
        let (from, to) = stretch_of(&container, place.before)
            .ok_or_else(|| EditError::Unsupported(format!("{empty} into an empty container")))?;
        let path = |k: u32| -> Box<[u32]> { place.container.iter().copied().chain([k]).collect() };
        let (work, held) = crate::ranges::stretch(tree.root(), &[path(from), path(to)]).ok_or_else(path_error)?;
        Ok(Stretch { work, held, from, to })
    }

    /// The node the container's child `k` was read as (in the stretch).
    pub(crate) fn child(&self, k: u32) -> Option<Id> {
        self.held.get(k.checked_sub(self.from)? as usize).copied()
    }

    /// The node the blocks go before (`None`: the container's end).
    pub(crate) fn anchor(&self, place: &Place) -> Option<Id> {
        place.before.and_then(|k| self.child(k))
    }
}

/// The stretch's new content spliced into the part, with the keys it read and wrote and
/// the annotation ids drawn into it; the namespaces (`w14`, with `wp14` for `drawings`) and
/// people a tracked write needs.
///
/// # Errors
/// When the part cannot be written.
pub(crate) fn commit(
    sink: &mut EditSink<'_, '_>,
    stretch: &Stretch,
    place: &Place,
    created: &[String],
    wrote: &TextWrite,
    drawings: bool,
) -> Result<()> {
    let (work, from, to) = (&stretch.work, stretch.from, stretch.to);
    let part = place.part.clone();
    let path_error = || EditError::no_element(part.clone());
    sink.keep_written(&part, None, work);
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
        sink.ensure_w14(&part, drawings, &[("w16du", W16DU)])?;
    }
    sink.ensure_w14(&part, drawings, &[])?;
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
        sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    }
    Ok(())
}
