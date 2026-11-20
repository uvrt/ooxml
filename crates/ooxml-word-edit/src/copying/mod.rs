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
//! Not ported yet ([`EditError::Unsupported`]: the host copies such blocks its own way):
//! the copy tracked, `replace:`, and copies holding notes, comments, bookmarks or content
//! controls.
//!
//! Must not: write but through the sink, or know a tool's JSON.

mod canonical;
mod cascade;
mod flatten;
mod lists;
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
use crate::sink::{EditSink, TextWrite};
use crate::stamping::stamp_document;
use crate::work::{Id, Work};
use crate::{EditError, Ids, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::Docx;
use source::Selected;
use std::sync::Arc;

/// The document the blocks are copied from.
#[derive(Debug, Clone)]
pub enum CopySource {
    /// This document, as it is when the copy begins.
    Own,
    /// Another open document, at the version the call holds.
    Other(Arc<Docx>),
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

/// Blocks copied here from a source document.
#[derive(Debug, Clone)]
pub struct CopyBlocks {
    /// Where they come from.
    pub source: CopySource,
    /// Which blocks.
    pub range: CopyRange,
    /// `end`, `end:<story>`, `after:<id>`, `before:<id>` or a bare id (after it).
    pub at: String,
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
}

/// What a copy did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlocksCopied {
    /// The first top-level block made.
    pub id: Option<String>,
    /// Every paragraph and table made, in document order.
    pub created: Vec<String>,
    /// Ids the edit's stamping renamed, old to new.
    pub renamed: Vec<(String, String)>,
    /// The styles imported.
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
            CopySource::Other(docx) => Arc::clone(docx),
        };
        self.select(doc, &source)?;
        let docx = doc.docx()?;
        destination(doc, &docx, &self.at)?;
        if matches!(self.policy, RevisionPolicy::Tracked(_)) {
            return Err(EditError::Unsupported("copy_from tracked".to_owned()));
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<BlocksCopied> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let source = match &self.source {
            CopySource::Own => sink.docx()?,
            CopySource::Other(docx) => Arc::clone(docx),
        };
        let out = self.copy(&mut sink, &source)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

/// The place `at` names, in the body, a header or footer, or a cell.
fn destination(doc: &WordDoc, docx: &Docx, at: &str) -> Result<Place> {
    let place = locate(doc, docx, at, "copy_from")?;
    let tree = doc
        .tree(&place.part)?
        .ok_or_else(|| EditError::NoPart(place.part.clone()))?;
    if ["footnotes", "endnotes", "comments"]
        .iter()
        .any(|l| tree.root().name().is(Some(W), l))
    {
        return Err(EditError::Refused(
            "blocks are copied into the body, a header or a footer, or a cell".to_owned(),
        ));
    }
    Ok(place)
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

    /// The source blocks: in this document by its renames; in another by its ids as they
    /// are (one it does not know may be one its session renamed: the host's to answer).
    fn select(&self, doc: &WordDoc, source: &Docx) -> Result<Selected> {
        match &self.source {
            CopySource::Own => source::select(doc.ids(), source, &self.range),
            CopySource::Other(_) => source::select(&Ids::default(), source, &self.range).map_err(|e| match e {
                EditError::Address(_) => {
                    EditError::Unsupported("a source block the source's session may have renamed".to_owned())
                }
                other => other,
            }),
        }
    }

    fn copy(&self, sink: &mut EditSink<'_, '_>, source: &Arc<Docx>) -> Result<BlocksCopied> {
        let selected = self.select(sink.doc(), source)?;
        if matches!(self.source, CopySource::Own) {
            read_source(sink, source, &selected);
        }
        let source_ids = source::block_ids(source, &selected);
        let docx = sink.docx()?;
        let place = destination(sink.doc(), &docx, &self.at)?;
        if matches!(self.policy, RevisionPolicy::Tracked(_)) {
            return Err(EditError::Unsupported("copy_from tracked".to_owned()));
        }
        let renamed = stamp_document(sink, &docx)?;
        let mut stretch = Stretch::open(sink, &place, "copy_from")?;
        let anchor = stretch.anchor(&place);
        let mut space = Space::default();
        let mut styles = styles::Styles::new(sink, source, self.policies(), &place)?;
        let work = &mut stretch.work;
        let mut elements = Vec::with_capacity(selected.blocks.len());
        for (_, element) in &selected.blocks {
            let copy = work
                .adopt_copy(element)
                .ok_or_else(|| EditError::Path(selected.part.clone()))?;
            view::current_view(work, copy);
            elements.push(copy);
        }
        if let Some(what) = view::left_to_host(work, &elements) {
            return Err(EditError::Unsupported(what.to_owned()));
        }
        for &element in &elements {
            styles.bring(sink, &mut space, work, element)?;
        }
        for &element in &elements {
            styles.bring_lists(sink, &mut space, work, element)?;
        }
        let mut parts = parts::Parts::new(&source.package, sink.doc().package());
        parts.bring(sink, work, &elements, (&selected.part, &place.part))?;
        crate::imported::keep_or_renumber_drawings(sink, work, &elements)?;
        let root = work.root();
        for &element in &elements {
            match anchor {
                Some(before) => work.add_previous(before, element),
                None => work.append(root, element),
            }
        }
        close_table(work, &mut elements);
        let stamped = stamp::stamp(sink, work, &place, &elements)?;
        crate::placing::commit(
            sink,
            &stretch,
            &place,
            &stamped.created,
            &TextWrite::default(),
            stamped.drawings,
        )?;
        space.finish(sink)?;
        parts.finish(sink)?;
        let mut copied: Vec<(String, String)> = Vec::new();
        for (old_ids, new_ids) in source_ids.iter().zip(&stamped.per_element) {
            for (old, new) in old_ids.iter().zip(new_ids) {
                if old.is_empty() || new.is_empty() {
                    continue;
                }
                match copied.iter_mut().find(|(o, _)| o == old) {
                    Some(found) => found.1.clone_from(new),
                    None => copied.push((old.clone(), new.clone())),
                }
            }
        }
        let blocks: Vec<String> = stamped
            .per_element
            .iter()
            .filter_map(|ids| ids.first().filter(|id| !id.is_empty()).cloned())
            .collect();
        Ok(BlocksCopied {
            id: blocks.first().cloned(),
            created: stamped.created,
            renamed,
            warnings: styles.warnings,
            blocks,
            copied,
        })
    }
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
