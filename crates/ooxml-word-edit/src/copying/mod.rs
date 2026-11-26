//! Owns: [`CopyBlocks`], blocks copied from another open document (or from this one, as
//! it was before the call) as docx-agent's `copy_blocks` copies them (`edit/importing.py`):
//! the source blocks named ([`source`]), each made the source's current view ([`view`]),
//! its styles brought by name ([`styles`]), placed as insert_markdown places blocks
//! ([`crate::placing`]) with a paragraph after a table that ends its container, and its
//! ids kept where the document never had them ([`stamp`]); the reply maps each source id
//! to its copy's.
//!
//! Styles come by name, mapped or flattened as the copy's [`StylePolicy`], `style_map`
//! and [`UnmappedPolicy`] say ([`styles`], [`flatten`], with the cascade they read in
//! [`cascade`] and the themes in [`theme`]).
//!
//! The lists the copies number with are brought as new lists here, or continue the list
//! before the place ([`ListPolicy`], [`lists`]). The relationships they name are made
//! again, the parts they reach copied ([`parts`]); their drawings keep their numbers where
//! this document never had them ([`crate::imported`]).
//!
//! The notes and comments the copies hold are copied with them ([`notes`], [`comments`]),
//! their content brought as the blocks' is ([`content`]); their bookmarks renamed where
//! this document has the name, or dropped ([`BookmarkPolicy`], [`bookmarks`]); their
//! content controls given ids no control has ([`crate::imported::Controls`]), a
//! block-level control a top-level block of its own in the reply.
//!
//! Tracked, the copy is one insertion, as Word's tracked paste is: every paragraph's content
//! and mark, every row, the notes' content ([`crate::markdown::track`]).
//!
//! At `replace:`, the blocks replaced are deleted by the copy's policy ([`replace`]).
//!
//! Must not: write but through the sink, or know a tool's JSON.

mod bookmarks;
mod canonical;
mod cascade;
mod comments;
mod content;
mod flatten;
mod lists;
mod notes;
mod parts;
mod source;
mod stamp;
mod styles;
mod theme;
mod view;

use crate::names::W;
use crate::placing::{locate, Place, Stretch};
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::{Place as Where, PlaceKind};
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::Docx;
use source::Selected;
use std::sync::Arc;

/// The markers of a note, a comment or a bookmark: a copy holding one reads the whole
/// document (a note's number is one above the largest, a comment's or a bookmark's id the
/// next annotation id, a bookmark's name one no story has).
const NOTED: [&str; 7] = [
    "footnoteReference",
    "endnoteReference",
    "commentRangeStart",
    "commentRangeEnd",
    "commentReference",
    "bookmarkStart",
    "bookmarkEnd",
];

/// A note's reference in a story.
const NOTE_REFERENCES: [&str; 2] = ["footnoteReference", "endnoteReference"];

/// Blocks holding a note's reference refused at a place in a text box, as Word 16.106 has no
/// note reference there (its Insert Footnote is off in a text box, and its paste there drops
/// the reference): the way out the places before and after the paragraph anchoring the text
/// box (`word_notes`' refusal, [`crate::textbox::text_box_anchor`]).
fn no_notes_in_text_box(work: &Work, elements: &[Id], docx: &Docx, place: &Place) -> Result<()> {
    if !view::holds(work, elements, &NOTE_REFERENCES) {
        return Ok(());
    }
    let mut inside = place.container.clone();
    inside.push(place.before.unwrap_or(0));
    let Some(anchor) = crate::textbox::text_box_anchor(docx, &place.part, &inside) else {
        return Ok(());
    };
    let message = "the blocks hold a note's reference and the place is in a text box: Word puts no note \
                   reference in a text box";
    let Some(paragraph) = docx.part(&place.part).and_then(|story| story.ids.paragraph_at(&anchor)) else {
        return Err(EditError::Refused(message.to_owned()));
    };
    let paragraph = paragraph.id;
    Err(EditError::Argument {
        field: "at".to_owned(),
        message: format!("{message}; put them before or after the paragraph anchoring the text box, {paragraph}"),
        options: vec![format!("before:{paragraph}"), format!("after:{paragraph}")],
    })
}

/// The document the blocks are copied from.
#[derive(Debug, Clone)]
pub enum CopySource {
    /// This document, as it is when the copy begins.
    Own,
    /// Another open document, at the version the call holds (its addresses resolved by its
    /// renames).
    Other(Box<WordDoc>),
}

/// The source blocks.
#[derive(Debug, Clone)]
pub enum CopyRange {
    /// A block id, or `first..last` (blocks of one container).
    Blocks(String),
    /// Ids, each a block or `first..last` (a heading's section).
    Ids(Vec<String>),
}

/// Which styles copied blocks keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StylePolicy {
    /// This document's styles of the same names, the source's imported where it has none
    /// (Word's default paste).
    #[default]
    UseDestination,
    /// The default paragraph style, the source's look as direct formatting (Keep Source
    /// Formatting).
    KeepSource,
    /// The default paragraph style, bold, italic and underline direct; a heading or list
    /// item keeps a style of its kind (Merge Formatting, kept structure).
    Merge,
}

/// What a source style neither mapped nor in this document by name becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnmappedPolicy {
    /// Imported (Word's way).
    #[default]
    Import,
    /// Word's built-in style of that name, else this document's body style.
    Body,
}

/// Which list a copied list paragraph goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListPolicy {
    /// A new list of its own (Word's paste).
    #[default]
    Separate,
    /// The list before the place, where its first level counts as the copied one's.
    Continue,
}

/// What a copied bookmark whose name this document has becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BookmarkPolicy {
    /// Renamed (`Name_1`, ...), the copies' links and fields to it following (a taken
    /// `_GoBack` is dropped).
    #[default]
    Rename,
    /// Dropped (Word's paste).
    Drop,
}

/// Blocks copied here from a source document.
#[derive(Debug, Clone)]
pub struct CopyBlocks {
    /// Where they come from.
    pub source: CopySource,
    /// Which blocks.
    pub range: CopyRange,
    /// `end`, `end:<story>`, `after:<id>`, `before:<id>`, `replace:<blocks>` or a bare id
    /// (after it).
    pub at: Where,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
    /// Which styles the copies keep.
    pub styles: StylePolicy,
    /// Source style name -> this document's style name (a later pair for the same name,
    /// in any case, wins).
    pub style_map: Vec<(String, String)>,
    /// What a style neither mapped nor here becomes.
    pub unmapped: UnmappedPolicy,
    /// Which list copied list paragraphs go into.
    pub lists: ListPolicy,
    /// What a copied bookmark whose name this document has becomes.
    pub bookmarks: BookmarkPolicy,
}

/// What a copy did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlocksCopied {
    /// The first top-level block made.
    pub id: Option<String>,
    /// Every paragraph and table made, in document order.
    pub created: Vec<String>,
    /// Ids the edit's stamping renamed, old to new.
    pub renamed: ooxml_word_model::ids::Renames,
    /// The ids of the blocks replaced that went (`replace:`).
    pub removed: Vec<String>,
    /// The styles imported, the bookmarks renamed.
    pub warnings: Vec<String>,
    /// The top-level blocks made.
    pub blocks: Vec<String>,
    /// Each source id (blocks, the paragraphs in them) and its copy's, in order.
    pub copied: Vec<(String, String)>,
}

impl Command<WordDoc> for CopyBlocks {
    type Out = BlocksCopied;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let source = match &self.source {
            CopySource::Own => doc.docx()?,
            CopySource::Other(other) => other.docx()?,
        };
        self.select(doc, &source)?;
        let docx = doc.docx()?;
        destination(doc, &docx, &self.at)?;
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<BlocksCopied> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let source = match &self.source {
            CopySource::Own => sink.docx()?,
            CopySource::Other(other) => other.docx()?,
        };
        let out = self.copy(&mut sink, &source)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

/// The place `at` names, in the body, a header or footer, or a cell, and the blocks it
/// replaces (`replace:`).
fn destination(doc: &WordDoc, docx: &Docx, at: &Where) -> Result<(Place, Vec<String>)> {
    let (place, replaced) = match at.kind() {
        PlaceKind::Replace(blocks) => crate::placing::replacing(doc, docx, blocks)?,
        _ => (locate(doc, docx, at, "copy_from")?, Vec::new()),
    };
    let tree = doc
        .tree(&place.part)?
        .ok_or_else(|| EditError::no_part(place.part.clone()))?;
    if ["footnotes", "endnotes", "comments"]
        .iter()
        .any(|l| tree.root().name().is(Some(W), l))
    {
        return Err(EditError::Refused(
            "blocks are copied into the body, a header or a footer, or a cell".to_owned(),
        ));
    }
    Ok((place, replaced))
}

impl CopyBlocks {
    /// The style policies, the map's names casefolded.
    fn policies(&self) -> styles::Policies {
        styles::Policies {
            styles: self.styles,
            names: self
                .style_map
                .iter()
                .map(|(from, to)| (ooxml_word_model::docx::casefold(from), to.clone()))
                .collect(),
            unmapped: self.unmapped,
            lists: self.lists,
        }
    }

    /// The source blocks, by the renames of the document they are in.
    fn select(&self, doc: &WordDoc, source: &Docx) -> Result<Selected> {
        let ids = match &self.source {
            CopySource::Own => doc.ids(),
            CopySource::Other(other) => other.ids(),
        };
        source::select(ids, source, &self.range)
    }

    fn copy(&self, sink: &mut EditSink<'_, '_>, source: &Arc<Docx>) -> Result<BlocksCopied> {
        let selected = self.select(sink.doc(), source)?;
        if matches!(self.source, CopySource::Own) {
            read_source(sink, source, &selected);
        }
        let source_ids = source::block_ids(source, &selected);
        let docx = sink.docx()?;
        let (mut place, replaced) = destination(sink.doc(), &docx, &self.at)?;
        let renamed = stamp_document(sink, &docx)?;
        let tracked = matches!(self.policy, RevisionPolicy::Tracked(_));
        let mut replaced = crate::replacing::Replaced::begin(
            sink,
            &mut place,
            replaced,
            tracked,
            crate::replacing::Order::FirstWhereFollowed,
        )?;
        let mut stretch = Stretch::open(sink, &place, "copy_from")?;
        let anchor = stretch.anchor(&place);
        let space = Space::default();
        let styles = styles::Styles::new(sink, source, self.policies(), &place)?;
        let from = stretch.from;
        let work = &mut stretch.work;
        let mut elements = copies(work, &selected)?;
        no_notes_in_text_box(work, &elements, &docx, &place)?;
        if view::holds(work, &elements, &NOTED) {
            sink.read_everything();
        }
        let mut bringer = content::Bringer {
            styles,
            space,
            parts: parts::Parts::new(&source.package, sink),
            bookmarks: bookmarks::Bookmarks::new(self.bookmarks),
            controls: crate::imported::Controls::default(),
        };
        bringer.content(sink, work, &elements, (&selected.part, &place.part))?;
        let mut stories = Space::default();
        let notes = notes::bring(sink, &mut bringer, &mut stories, (source, work), &elements)?;
        let comments = comments::bring(sink, &mut bringer, &mut stories, (source, work), &elements)?;
        bringer.bookmarks(sink, work, &elements)?;
        crate::imported::keep_or_renumber_drawings(sink, work, &elements)?;
        let controls = bringer.controls(sink, work, &elements)?;
        let root = work.root();
        for &element in &elements {
            match anchor {
                Some(before) => work.add_previous(before, element),
                None => work.append(root, element),
            }
        }
        close_table(work, &mut elements);
        let placed = placed_controls(work, &place, from, &elements);
        let mut stamped = stamp::stamp(sink, work, &place, &elements)?;
        let mut note_ids = Vec::new();
        let notes_parts = notes.stamp(sink, &mut stories, &mut note_ids)?;
        // tracked, one revision group: the blocks (Word's typing form at a container's end),
        // then the notes
        let wrote = sink.track(work, |work, stamp, orders| {
            crate::markdown::track(work, &elements, stamp, orders, None);
            Vec::new()
        })?;
        let notes_dated = notes.track(sink, &mut stories)?;
        crate::placing::commit(sink, &stretch, &place, &stamped.created, &wrote, stamped.drawings)?;
        stamped.created.extend(note_ids);
        let warnings = bringer.finish(sink)?;
        stories.finish(sink)?;
        let comments_part = comments.part.iter().map(|p| (p.clone(), false));
        for (part, drawings) in notes_parts.into_iter().chain(comments_part) {
            let dated: &[(&str, &str)] = if notes_dated.contains(&part) {
                &[("w16du", crate::names::W16DU)]
            } else {
                &[]
            };
            sink.ensure_w14(&part, drawings, dated)?;
        }
        replaced.finish(sink, tracked, &stamped.per_element)?;
        let (blocks, top_controls) = top_blocks(sink, (source, &selected), &place, &placed, &stamped.per_element)?;
        // docx-agent's `copied`: the notes', comments' and controls' first, then the blocks',
        // then the block-level controls'
        let annotations = [&notes.copied, &comments.copied, &controls];
        let copied = copied_pairs(&annotations, (&source_ids, &stamped.per_element), &top_controls);
        Ok(BlocksCopied {
            id: blocks.first().cloned(),
            created: stamped.created,
            renamed,
            removed: replaced.removed,
            warnings,
            blocks,
            copied,
        })
    }
}

/// Old and new ids, in order.
type Pairs = Vec<(String, String)>;

/// docx-agent's `copied` of `annotations` (each `(old, new)` in order), then each source
/// block's ids with its copy's, then `after`.
fn copied_pairs(
    annotations: &[&Pairs],
    (source_ids, per_element): (&[Vec<String>], &[Vec<String>]),
    after: &Pairs,
) -> Pairs {
    let block_ids = source_ids.iter().zip(per_element).flat_map(|(o, n)| o.iter().zip(n));
    first_to_last(
        annotations
            .iter()
            .flat_map(|pairs| pairs.iter())
            .map(|(o, n)| (o, n))
            .chain(block_ids)
            .chain(after.iter().map(|(o, n)| (o, n))),
    )
}

/// Where each of `elements` (placed in the stretch `work` of the container at `place`,
/// whose first child is the container's child `from`) that is a block-level content
/// control is in the part.
fn placed_controls(work: &Work, place: &Place, from: u32, elements: &[Id]) -> Vec<Option<Box<[u32]>>> {
    elements
        .iter()
        .map(|&e| {
            let index = work.index(e).filter(|_| work.is(e, W, "sdt"))?;
            let index = from + u32::try_from(index).ok()?;
            Some(place.container.iter().copied().chain([index]).collect())
        })
        .collect()
}

/// The top-level blocks made, as docx-agent's `copy_blocks` lists them: a block-level
/// content control's id in this document (`cc:...`), each other block's own id where it
/// has one; and each copied control's source id with its copy's.
///
/// # Errors
/// A placed control this document does not list.
fn top_blocks(
    sink: &mut EditSink<'_, '_>,
    (source, selected): (&Docx, &Selected),
    place: &Place,
    placed: &[Option<Box<[u32]>>],
    per_element: &[Vec<String>],
) -> Result<(Vec<String>, Pairs)> {
    let mut found = None;
    let mut blocks = Vec::new();
    let mut copied = Vec::new();
    for (k, ids) in per_element.iter().enumerate() {
        let Some(path) = placed.get(k).and_then(Option::as_ref) else {
            blocks.extend(ids.first().filter(|id| !id.is_empty()).cloned());
            continue;
        };
        if found.is_none() {
            sink.read_everything();
            let here = sink.docx()?;
            found = Some((controls(&here), controls(source)));
        }
        let Some((here, there)) = &found else { continue };
        let id = here
            .iter()
            .find(|c| c.part == place.part && c.path == *path)
            .map(|c| c.id.clone())
            .ok_or_else(|| EditError::no_element(place.part.clone()))?;
        let original = selected
            .blocks
            .get(k)
            .and_then(|(at, _)| there.iter().find(|c| c.part == selected.part && *c.path == at[..]));
        if let Some(original) = original {
            copied.push((original.id.clone(), id.clone()));
        }
        blocks.push(id);
    }
    Ok((blocks, copied))
}

/// The source blocks copied into `work` (detached), each as the source shows it now.
///
/// # Errors
/// A block that cannot be copied.
fn copies(work: &mut Work, selected: &Selected) -> Result<Vec<Id>> {
    let mut elements = Vec::with_capacity(selected.blocks.len());
    for (_, element) in &selected.blocks {
        let copy = work
            .adopt_copy(element)
            .ok_or_else(|| EditError::no_element(selected.part.clone()))?;
        view::current_view(work, copy);
        elements.push(copy);
    }
    Ok(elements)
}

/// Each old id with the new id it was last given, in the order first given (a dict's
/// update), pairs with an empty side left out.
fn first_to_last<'a>(pairs: impl Iterator<Item = (&'a String, &'a String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (old, new) in pairs {
        if old.is_empty() || new.is_empty() {
            continue;
        }
        match out.iter_mut().find(|(o, _)| o == old) {
            Some(found) => found.1.clone_from(new),
            None => out.push((old.clone(), new.clone())),
        }
    }
    out
}

/// A paragraph after the copies when the last is a table that ends its container
/// (`_is_last`).
fn close_table(work: &mut Work, elements: &mut Vec<Id>) {
    let Some(&last) = elements.last() else { return };
    if !work.is(last, W, "tbl") {
        return;
    }
    let mut at = work.next(last);
    while let Some(node) = at {
        if work.is_any(node, W, &crate::placing::BLOCKS) {
            return;
        }
        at = work.next(node);
    }
    let trailing = crate::review::xml::make(work, "p", &[]);
    work.add_next(last, trailing);
    elements.push(trailing);
}

/// The source blocks read (this document's own: an edit of them conflicts with the copy).
fn read_source(sink: &mut EditSink<'_, '_>, docx: &Docx, selected: &Selected) {
    let Some(story) = docx.part(&selected.part) else { return };
    let within = |path: &[u32]| selected.blocks.iter().any(|(p, _)| path.starts_with(p));
    let paragraphs = story
        .ids
        .paragraphs()
        .iter()
        .filter(|p| within(&p.path))
        .map(|p| p.id.clone());
    let tables = story
        .ids
        .tables()
        .iter()
        .filter(|t| within(&t.path))
        .map(|t| t.id.clone());
    let keys: Vec<_> = paragraphs
        .chain(tables)
        .map(|id| block_key(&selected.part, &id))
        .collect();
    for key in keys {
        sink.read(key);
    }
}
