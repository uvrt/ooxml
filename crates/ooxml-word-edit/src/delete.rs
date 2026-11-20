//! Owns: [`DeleteBlock`], a paragraph or table deleted as docx-agent's `delete_block`
//! deletes it (a block-level content control: [`crate::control_delete`]). Refused before anything changes: a paragraph that ends a section, one
//! holding a note or comment reference (untracked, a block comment's references move on
//! to the paragraph after it), one cutting a field, and a block whose container would be
//! left without a paragraph to end on. Untracked -- or tracked, one's own insertion -- the
//! block goes, range markers whose partner stays moved to the nearest paragraph; tracked,
//! a paragraph's content and mark are recorded as deleted (last in its container, the mark
//! before it is, and it takes that paragraph's properties), a table's rows as Word deletes
//! them ([`crate::table_delete`]).
//!
//! Must not: write but through the sink, or read outside what it records reading: the
//! blocks of the stretch it rewrites, the ledger seeds the stamping draws from, the
//! annotation ids when tracked.

use crate::block_checks::{
    block_references, check_container_survives, ends_section, has_lonely_markers, held_reference, neighbour_path,
};
use crate::doc::{Key, WordDoc};
use crate::marks::mark_record;
use crate::names::{W, W16DU};
use crate::paragraphs::{anchor, followed, Anchor};
use crate::policy::RevisionPolicy;
use crate::ranges::stretch_opening;
use crate::set_text::block_key;
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::stamp_document;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Delete a paragraph or a table with its content.
#[derive(Debug, Clone)]
pub struct DeleteBlock {
    /// The paragraph or table (an id, an alias).
    pub target: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a delete did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDeleted {
    /// The block's id when it stays (a tracked deletion), `None` when it went.
    pub id: Option<String>,
    /// Ids the edit's stamping renamed, old to new, in the order made (but the removed).
    pub renamed: Vec<(String, String)>,
    /// The ids of the blocks that went: the block's, a table's paragraphs'.
    pub removed: Vec<String>,
    /// The block's part.
    pub part: String,
    /// What the caller should know: a content control deleted tracked is removed
    /// untracked (Word has no revision for a control).
    pub warnings: Vec<String>,
}

/// The ids of the paragraphs inside a table, in document order.
fn nested(docx: &Docx, block: &Anchor) -> Vec<String> {
    if block.paragraph {
        return Vec::new();
    }
    docx.part(&block.part)
        .map(|story| {
            story
                .ids
                .paragraphs()
                .iter()
                .filter(|p| p.path.starts_with(&block.path))
                .map(|p| p.id)
                .collect()
        })
        .unwrap_or_default()
}

/// Refuse what docx-agent refuses before anything changes; for a paragraph deleted
/// untracked, whether block comment references move on.
fn check(root: &Arc<Element>, block: &Anchor, element: &Element, tracked: bool) -> Result<bool> {
    let mut moving = false;
    if block.paragraph {
        if ends_section(element) {
            return Err(EditError::Refused(format!(
                "{} ends a section; deleting it would join two sections",
                block.id
            )));
        }
        let references = if tracked { None } else { block_references(root, element) };
        if let Some(local) = references.is_none().then(|| held_reference(element)).flatten() {
            return Err(EditError::Refused(format!(
                "{} holds a {local}; deleting it would orphan the note or comment",
                block.id
            )));
        }
        moving = references.unwrap_or(false);
        let work = Work::of(&Arc::new(element.clone()), Vec::new());
        if !crate::across::fields_balanced(&work, work.root()) {
            return Err(EditError::Refused(format!(
                "{} holds part of a field that spans paragraphs",
                block.id
            )));
        }
    }
    check_container_survives(root, &block.path)?;
    Ok(moving)
}

/// Whether a paragraph is wholly the author's own insertion: its mark and every run
/// (docx-agent's `_own_insertion`).
pub(crate) fn own_insertion(work: &Work, paragraph: Id, author: &str) -> bool {
    let mine = |record: Id| work.is(record, W, "ins") && work.get(record, Some(W), "author") == Some(author);
    if !mark_record(work, paragraph).is_some_and(mine) {
        return false;
    }
    work.iter(paragraph, W, &["r"]).into_iter().all(|run| {
        let mut at = work.parent(run);
        while let Some(node) = at.filter(|n| *n != paragraph && !work.is(*n, W, "p")) {
            if work.is_any(node, W, &["ins", "del", "moveFrom", "moveTo"]) {
                return mine(node);
            }
            at = work.parent(node);
        }
        false
    })
}

/// The element of `work` at `path` below `top` (a node of the stretch whose tree element
/// is `element`): tree places count text, the work's children do not.
fn descend(work: &Work, mut at: Id, element: &Arc<Element>, path: &[u32]) -> Option<Id> {
    let mut element = Arc::clone(element);
    for &k in path {
        let nodes = element.children().to_vec();
        let index = nodes
            .iter()
            .take(k as usize)
            .filter(|n| !matches!(n, Node::Text(_)))
            .count();
        at = *work.children(at).get(index)?;
        element = match nodes.get(k as usize)? {
            Node::Element(e) => Arc::clone(e),
            _ => return None,
        };
    }
    Some(at)
}

/// The stretch of one parent's children that holds every place of `paths`: the parent's
/// place and the first and last child's.
fn span(paths: &[Vec<u32>]) -> Option<(Vec<u32>, u32, u32)> {
    let first = paths.first()?;
    let mut common = first.len();
    for p in paths {
        common = common.min(p.iter().zip(first).take_while(|(a, b)| a == b).count());
    }
    if paths.iter().any(|p| p.len() == common) {
        common = common.checked_sub(1)?;
    }
    let parent = first.get(..common)?.to_vec();
    let at: Vec<u32> = paths.iter().filter_map(|p| p.get(common).copied()).collect();
    Some((parent, *at.iter().min()?, *at.iter().max()?))
}

/// The paragraph before `index` among `parent`'s children that a tracked deletion joins
/// into: past nodes that are not blocks and paragraphs whose mark is deleted already.
fn joined_into(parent: &Element, index: usize) -> Option<usize> {
    let children = parent.children().to_vec();
    (0..index).rev().find(|&k| match children.get(k) {
        Some(Node::Element(e)) if e.name().is(Some(W), "tbl") => true,
        Some(Node::Element(e)) if e.name().is(Some(W), "p") => {
            let work = Work::of(e, Vec::new());
            !mark_record(&work, work.root()).is_some_and(|r| work.is_any(r, W, &["del", "moveFrom"]))
        }
        _ => false,
    })
}

impl Command<WordDoc> for DeleteBlock {
    type Out = BlockDeleted;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        if crate::control_delete::is_control(&self.target) {
            return crate::control_delete::check(doc, &self.target);
        }
        let docx = doc.docx()?;
        let block = resolve(doc, &docx, &self.target)?;
        let tree = doc
            .tree(&block.part)?
            .ok_or_else(|| EditError::NoPart(block.part.clone()))?;
        let element = at_path(tree.root(), &block.path).ok_or_else(|| EditError::Path(block.part.clone()))?;
        check(
            tree.root(),
            &block,
            element,
            matches!(self.policy, RevisionPolicy::Tracked(_)),
        )
        .map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<BlockDeleted> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = crate::delete_write::delete(&mut sink, &self.target)?;
        crate::textbox::sync(&mut sink)?;
        crate::revision_ids::make_unique(&mut sink)?;
        Ok(out)
    }
}

/// The block `target` names: a paragraph or a table (a content control is
/// [`crate::control_delete`]'s).
fn resolve(doc: &WordDoc, docx: &Docx, target: &str) -> Result<Anchor> {
    anchor(doc, docx, target)
}

/// What [`crate::delete_write`] needs of this module.
pub(crate) struct Planned {
    pub(crate) block: Anchor,
    pub(crate) moving: bool,
    pub(crate) removed: Vec<String>,
}

/// The block, checked, with the ids that go with it.
pub(crate) fn plan(sink: &mut EditSink<'_, '_>, docx: &Docx, target: &str) -> Result<Planned> {
    let block = resolve(sink.doc(), docx, target)?;
    let tree = sink
        .doc()
        .tree(&block.part)?
        .ok_or_else(|| EditError::NoPart(block.part.clone()))?;
    let element = at_path(tree.root(), &block.path).ok_or_else(|| EditError::Path(block.part.clone()))?;
    let tracked = matches!(sink.policy(), RevisionPolicy::Tracked(_));
    let moving = check(tree.root(), &block, element, tracked)?;
    let mut removed = vec![block.id.clone()];
    removed.extend(nested(docx, &block));
    Ok(Planned { block, moving, removed })
}

/// The paths a deletion's stretch must hold: the block; the paragraph after it when
/// references move or markers are rehomed (before it when there is none after); the
/// paragraph a tracked deletion joins into.
pub(crate) fn stretch_paths(root: &Element, element: &Element, path: &[u32], moving: bool) -> Vec<Vec<u32>> {
    let mut paths = vec![path.to_vec()];
    if moving || has_lonely_markers(element) {
        match neighbour_path(root, path, true) {
            Some(after) => paths.push(after),
            None => paths.extend(neighbour_path(root, path, false)),
        }
    }
    paths
}

/// The place of the paragraph a tracked deletion of the paragraph at `path` joins into,
/// when it is last in its container.
pub(crate) fn join_path(root: &Arc<Element>, path: &[u32]) -> Option<Vec<u32>> {
    let (&index, above) = path.split_last()?;
    let parent = if above.is_empty() { root } else { at_path(root, above)? };
    if followed(parent, index as usize) {
        return None;
    }
    let k = joined_into(parent, index as usize)?;
    let mut out = above.to_vec();
    out.push(u32::try_from(k).ok()?);
    Some(out)
}

/// A stretch made editable: its work, its parent's place, its first and last child's
/// index, and the work's nodes asked for.
pub(crate) type Editable = (Work, Vec<u32>, (u32, u32), Vec<Id>);

/// A stretch of `root`'s part made editable: its work, the parent's place, the first and
/// last child's index, and the work's node for each of `wanted`.
pub(crate) fn editable(root: &Arc<Element>, paths: &[Vec<u32>], wanted: &[&[u32]]) -> Option<Editable> {
    editable_opening(root, paths, wanted, Opening::All)
}

/// Which children of a stretch an edit reads whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opening {
    /// Every one (an edit of the whole stretch).
    All,
    /// Those holding one of the paths, and the element before and after each: an edit of
    /// the stretch's ends (a block moved far, a section break early in a long body), which
    /// reads and moves those and passes the others whole ([`Work::of_stretch`]).
    Ends,
}

/// [`editable`], reading the children `opening` says whole.
pub(crate) fn editable_opening(
    root: &Arc<Element>,
    paths: &[Vec<u32>],
    wanted: &[&[u32]],
    opening: Opening,
) -> Option<Editable> {
    let (parent, from, to) = span(paths)?;
    let ends: Vec<Box<[u32]>> = [from, to]
        .iter()
        .map(|k| {
            let mut p = parent.clone();
            p.push(*k);
            p.into_boxed_slice()
        })
        .collect();
    let holder = if parent.is_empty() {
        Arc::clone(root)
    } else {
        Arc::clone(at_path(root, &parent)?)
    };
    let opened = match opening {
        Opening::All => None,
        Opening::Ends => Some(ends_and_neighbours(&holder, paths, parent.len())),
    };
    let open = |k: u32| opened.as_ref().is_none_or(|o| o.contains(&k));
    let (work, _) = stretch_opening(root, &ends, &open)?;
    let mut found = Vec::new();
    for path in wanted {
        let rest = path.get(parent.len()..)?;
        let (&top, below) = rest.split_first()?;
        let nodes = holder.children().to_vec();
        let index = nodes
            .iter()
            .skip(from as usize)
            .take(top.checked_sub(from)? as usize)
            .filter(|n| !matches!(n, Node::Text(_)))
            .count();
        let at = *work.children(work.root()).get(index)?;
        let Node::Element(element) = nodes.get(top as usize)? else {
            return None;
        };
        found.push(descend(&work, at, element, below)?);
    }
    Some((work, parent, (from, to), found))
}

/// The places among `holder`'s children of the child holding each of `paths` (below
/// `depth`) and of the element before and after it.
fn ends_and_neighbours(holder: &Element, paths: &[Vec<u32>], depth: usize) -> BTreeSet<u32> {
    let children = holder.children();
    let element_at = |k: usize| children.get(k).is_some_and(|n| matches!(n, Node::Element(_)));
    let mut out = BTreeSet::new();
    for top in paths.iter().filter_map(|p| p.get(depth)) {
        out.insert(*top);
        let at = *top as usize;
        let before = (0..at).rev().find(|k| element_at(*k));
        let after = (at + 1..children.len()).find(|k| element_at(*k));
        out.extend(before.into_iter().chain(after).filter_map(|k| u32::try_from(k).ok()));
    }
    out
}

/// The keys of the blocks in the part's stretch `from..=to` of the parent at `parent`
/// (its paragraphs, then its tables), read from the index's segments there alone.
pub(crate) fn stretch_keys(docx: &Docx, part: &str, parent: &[u32], stretch: (u32, u32)) -> Vec<Key> {
    let Some(story) = docx.part(part) else {
        return Vec::new();
    };
    let (paragraphs, tables) = story.ids.within_stretch(parent, stretch);
    let paragraphs = paragraphs.iter().map(|p| block_key(part, &p.id));
    paragraphs
        .chain(tables.iter().map(|t| block_key(part, &t.id)))
        .collect()
}

/// The stamping's renames but those of the blocks that went.
pub(crate) fn kept_renames(
    sink: &mut EditSink<'_, '_>,
    docx: &Docx,
    removed: &[String],
) -> Result<Vec<(String, String)>> {
    let renamed = stamp_document(sink, docx)?;
    // docx-agent: the aliases of the blocks that go go with them
    sink.forget_aliases(removed)?;
    Ok(renamed.into_iter().filter(|(old, _)| !removed.contains(old)).collect())
}

/// The draws a rewritten stretch's revision records need.
pub(crate) fn draws_of(nodes: &[Node], numbered: Option<crate::sink::Numbered>) -> Vec<crate::doc::Draw> {
    let Some(numbered) = numbered else { return Vec::new() };
    nodes
        .iter()
        .enumerate()
        .filter_map(|(k, n)| match (n, u32::try_from(k)) {
            (Node::Element(e), Ok(k)) => Some(annotation_draws(e, numbered, k)),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Declare `w16du` on the part's root when a revision record carries it.
pub(crate) fn declare(sink: &mut EditSink<'_, '_>, part: &str, dated: bool) -> Result<()> {
    if dated {
        sink.ensure_w14(part, false, &[("w16du", W16DU)])?;
    }
    Ok(())
}
