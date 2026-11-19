//! Owns: [`MoveBlocks`], blocks moved together beside another block of their story as
//! docx-agent's `move_blocks` and `move_block` move them: each keeps its id; refused, before
//! anything changes, for a target inside the blocks, custom XML, a paragraph ending a
//! section, a locked control, and a move that would leave a cell or story without a final
//! paragraph; a block already where it would go stays (no change); and a heading's section
//! as `section_blocks` reads it ([`section_blocks`]).
//!
//! Tracked, docx-agent writes Word's move form (`w:moveFrom`/`w:moveTo` and their ranges)
//! or a deletion and an insertion: not ported yet ([`EditError::Unsupported`]), but for a
//! content control or a block holding a note or comment reference, which move untracked
//! (said in a warning) as docx-agent moves them.
//!
//! Must not: write but through the sink.

use crate::blocks::{blocks_of, resolve_block, BlockAt, BlockKind};
use crate::delete::{editable, stretch_keys};
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::resolve::body_headings;
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::outline::body_blocks;
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use std::sync::Arc;

/// What moves.
#[derive(Debug, Clone)]
pub enum Moving {
    /// A block, or a span of blocks of one container (`p:A..t:B`).
    Blocks(String),
    /// Blocks by id, in order (a heading's section).
    Ids(Vec<String>),
}

/// Blocks moved beside a block of their story.
#[derive(Debug, Clone)]
pub struct MoveBlocks {
    /// The blocks.
    pub blocks: Moving,
    /// The block they go after; or
    pub after: Option<String>,
    /// the block they go before.
    pub before: Option<String>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a move did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlocksMoved {
    /// The first block's id after the move.
    pub id: Option<String>,
    /// Whether anything moved.
    pub changed: bool,
    /// Each block's id after the move, in order.
    pub moved: Vec<String>,
    /// Ids renamed, old to new.
    pub renamed: Vec<(String, String)>,
    /// What moved otherwise than asked.
    pub warnings: Vec<String>,
}

/// The ids of a heading's section: the heading and every block after it up to the next
/// heading of its level or a higher one (docx-agent's `section_blocks`).
///
/// # Errors
/// A heading that is no body paragraph, or no heading.
pub fn section_blocks(doc: &WordDoc, heading: &str) -> Result<Vec<String>> {
    let docx = doc.docx()?;
    let found = crate::set_text::find(doc, &docx, heading)?;
    if found.part != docx.main || found.path.len() != 2 {
        return Err(EditError::Refused(format!(
            "{heading} is not a paragraph of the body itself (not in a table or a control)"
        )));
    }
    let Some(owned) = body_headings(&docx).into_iter().find(|h| h.id == found.id) else {
        return Err(EditError::Refused(format!(
            "{} is not a heading: its style has no heading level",
            found.id
        )));
    };
    let all = controls(&docx);
    Ok(body_blocks(&docx, &all)
        .into_iter()
        .skip(owned.start)
        .take(owned.end - owned.start)
        .filter_map(|b| b.id)
        .collect())
}

/// A block's id at `path` of the story `part` (docx-agent's `_block_id`).
fn block_id(docx: &Docx, part: &str, block: &BlockAt) -> Option<String> {
    let story = docx.part(part)?;
    match block.kind {
        BlockKind::Paragraph => story.ids.paragraph_at(&block.path).map(|p| p.id),
        BlockKind::Table => story
            .ids
            .tables()
            .iter()
            .find(|t| *t.path == *block.path)
            .map(|t| t.id.clone()),
        BlockKind::Control => controls(docx)
            .into_iter()
            .find(|c| c.part == part && *c.path == *block.path)
            .map(|c| c.id),
        BlockKind::Custom => None,
    }
}

/// The element at a block's place.
fn element_at(docx: &Docx, part: &str, block: &BlockAt) -> Option<Arc<Element>> {
    let story = docx.part(part)?;
    ooxml_word_model::docx::at_path(story.root(), &block.path).cloned()
}

impl MoveBlocks {
    /// The blocks, their part and ids, checked as `move_blocks` checks them.
    fn plan(&self, doc: &WordDoc, docx: &Docx) -> Result<(String, Vec<BlockAt>, Vec<String>, BlockAt)> {
        let target = match (&self.after, &self.before) {
            (Some(t), None) | (None, Some(t)) => t,
            _ => return Err(EditError::Refused("give exactly one of after= or before=".to_owned())),
        };
        let (part, blocks) = match &self.blocks {
            Moving::Blocks(address) => blocks_of(doc, docx, address)?,
            Moving::Ids(ids) => {
                if ids.is_empty() {
                    return Err(EditError::Refused("no blocks to move".to_owned()));
                }
                let mut found = Vec::new();
                let mut part = None;
                for id in ids {
                    let (p, block) = resolve_block(doc, docx, id)?;
                    if part.as_ref().is_some_and(|q| *q != p) {
                        return Err(EditError::Refused("the blocks moved are of one story".to_owned()));
                    }
                    part = Some(p);
                    if !found.iter().any(|b: &BlockAt| b.path == block.path) {
                        found.push(block);
                    }
                }
                found.sort_by(|a, b| a.path.cmp(&b.path));
                (part.unwrap_or_default(), found)
            }
        };
        let (target_part, anchor) = resolve_block(doc, docx, target)?;
        if target_part != part {
            return Err(EditError::Refused(
                "blocks move within their story; between documents is copy_blocks".to_owned(),
            ));
        }
        if blocks.iter().any(|b| anchor.path.starts_with(&b.path)) {
            return Err(EditError::Refused(format!(
                "{target} is one of the blocks moved, or inside one"
            )));
        }
        let mut ids = Vec::new();
        for block in &blocks {
            if block.kind == BlockKind::Custom {
                return Err(EditError::Refused(
                    "custom XML is not moved: move_blocks moves paragraphs, tables and content-control blocks"
                        .to_owned(),
                ));
            }
            let id = block_id(docx, &part, block).unwrap_or_default();
            let element = element_at(docx, &part, block).ok_or_else(|| EditError::Path(part.clone()))?;
            if block.kind == BlockKind::Paragraph
                && ooxml_word_model::docx::find_path(&element, &["pPr", "sectPr"]).is_some()
            {
                return Err(EditError::Refused(format!(
                    "{id} ends a section; moving it would move the section break"
                )));
            }
            if block.kind == BlockKind::Control {
                control_movable(&element, &id, "moving")?;
            }
            ids.push(id);
        }
        Ok((part, blocks, ids, anchor))
    }
}

/// Refuse to delete or move (`doing`) a locked control or one holding a section break
/// (`_control_movable`).
///
/// # Errors
/// [`EditError::Refused`] naming the lock or the section break.
pub(crate) fn control_movable(sdt: &Element, id: &str, doing: &str) -> Result<()> {
    let lock = ooxml_word_model::docx::find_path(sdt, &["sdtPr", "lock"]).and_then(|l| l.attribute(Some(W), "val"));
    match lock {
        Some(value @ ("sdtLocked" | "sdtContentLocked")) => Err(EditError::Refused(format!(
            "{id} is locked against deletion (w:lock {value}): {doing} it is refused"
        ))),
        Some("contentLocked") => Err(EditError::Refused(format!(
            "{id}'s content is locked (w:lock contentLocked): {doing} it is refused"
        ))),
        _ if holds_section_break(sdt) => Err(EditError::Refused(format!(
            "{id} holds a section break; {doing} it would move or join sections"
        ))),
        _ => Ok(()),
    }
}

fn holds_section_break(element: &Element) -> bool {
    element
        .child_elements()
        .any(|c| (c.name().is(Some(W), "pPr") && c.find(Some(W), "sectPr").is_some()) || holds_section_break(c))
}

/// Whether `element` (in `work`) can go: its container keeps a paragraph to end on
/// (`_check_container_survives`).
fn survives(work: &Work, element: Id, outside: &Outside) -> Result<()> {
    let mut container = work.parent(element);
    while let Some(c) = container.filter(|c| work.is_any(*c, W, &["sdtContent", "sdt", "customXml"])) {
        container = work.parent(c);
    }
    let Some(container) = container else {
        return Err(EditError::Refused("the block has no container".to_owned()));
    };
    let mut blocks = Vec::new();
    direct_blocks(work, container, &mut blocks);
    blocks.retain(|b| *b != element);
    if container == work.root() {
        // the stretch stands for some of its parent's children: the blocks around it count
        match outside.after {
            Some(true) => {
                return Err(EditError::Refused(
                    "deleting it would leave a table last in its story or cell; Word needs a paragraph after it"
                        .to_owned(),
                ))
            }
            Some(false) => return Ok(()),
            None if blocks.is_empty() && outside.before => return Ok(()),
            None => {}
        }
    }
    match blocks.last() {
        None => Err(EditError::Refused(
            "deleting it would leave its story or cell with no paragraph".to_owned(),
        )),
        Some(last) if work.is(*last, W, "tbl") => Err(EditError::Refused(
            "deleting it would leave a table last in its story or cell; Word needs a paragraph after it".to_owned(),
        )),
        Some(_) => Ok(()),
    }
}

/// The blocks of a stretch's parent outside the stretch: whether the last after it is a
/// table (`None`: none after it), and whether any stands before it.
struct Outside {
    after: Option<bool>,
    before: bool,
}

fn outside_blocks(root: &Arc<Element>, parent: &[u32], (from, to): (u32, u32)) -> Outside {
    let holder = if parent.is_empty() {
        Some(root)
    } else {
        ooxml_word_model::docx::at_path(root, parent)
    };
    let Some(holder) = holder else {
        return Outside {
            after: None,
            before: false,
        };
    };
    let mut after = None;
    let mut before = false;
    for (k, node) in holder.children().iter().enumerate() {
        let ooxml_xml::Node::Element(e) = node else { continue };
        let mut found = Vec::new();
        raw_blocks(e, &mut found);
        let Some(last) = found.last() else { continue };
        let k = u32::try_from(k).unwrap_or(u32::MAX);
        if k < from {
            before = true;
        } else if k > to {
            after = Some(*last);
        }
    }
    Outside { after, before }
}

/// Whether each direct block of `element` (itself, or what a control or custom XML
/// holds) is a table, in order.
fn raw_blocks(element: &Element, out: &mut Vec<bool>) {
    let name = element.name();
    if name.namespace() != Some(W) {
        return;
    }
    match name.local() {
        "p" => out.push(false),
        "tbl" => out.push(true),
        "sdt" => {
            if let Some(content) = element.find(Some(W), "sdtContent") {
                for child in content.child_elements() {
                    raw_blocks(child, out);
                }
            }
        }
        "customXml" => {
            for child in element.child_elements() {
                raw_blocks(child, out);
            }
        }
        _ => {}
    }
}

fn direct_blocks(work: &Work, container: Id, out: &mut Vec<Id>) {
    for child in work.children(container).to_vec() {
        if work.is_any(child, W, &["p", "tbl"]) {
            out.push(child);
        } else if work.is(child, W, "sdt") {
            if let Some(content) = work.find(child, W, "sdtContent") {
                direct_blocks(work, content, out);
            }
        } else if work.is(child, W, "customXml") {
            direct_blocks(work, child, out);
        }
    }
}

/// Whether a block holds a note or comment reference.
fn holds_reference(element: &Element) -> bool {
    element.child_elements().any(|c| {
        (c.name().namespace() == Some(W)
            && matches!(
                c.name().local(),
                "footnoteReference" | "endnoteReference" | "commentReference"
            ))
            || holds_reference(c)
    })
}

/// A stretch arranged: its work, its parent's place and its first and last child's index.
type Arranged = (Work, Vec<u32>, (u32, u32));

impl MoveBlocks {
    /// The blocks put beside the target in a work of the stretch holding them all, as
    /// `move_block` puts each; `None` when every block is where it would go already.
    fn arrange(
        &self,
        root: &Arc<Element>,
        part: &str,
        (blocks, ids, anchor): (&[BlockAt], &[String], &BlockAt),
    ) -> Result<Option<Arranged>> {
        let mut paths: Vec<Vec<u32>> = blocks.iter().map(|b| b.path.clone()).collect();
        paths.push(anchor.path.clone());
        let wanted: Vec<&[u32]> = paths.iter().map(Vec::as_slice).collect();
        let path_error = || EditError::Path(part.to_owned());
        let (mut work, parent, stretch, nodes) = editable(root, &paths, &wanted).ok_or_else(path_error)?;
        let outside = outside_blocks(root, &parent, stretch);
        let (&target, moving) = nodes.split_last().ok_or_else(path_error)?;
        let after = self.after.is_some();
        let mut changed = false;
        let mut previous: Option<Id> = None;
        for (&node, id) in moving.iter().zip(ids) {
            let (beside, side_after) = match previous {
                Some(p) => (p, true),
                None => (target, after),
            };
            if node == beside {
                return Err(EditError::Refused("a block cannot move beside itself".to_owned()));
            }
            if crate::links::inside(&work, beside, node) {
                return Err(EditError::Refused(format!("{id} holds the block it would move beside")));
            }
            survives(&work, node, &outside)?;
            let already = if side_after {
                work.next(beside) == Some(node)
            } else {
                work.previous(beside) == Some(node)
            };
            if !already {
                if side_after {
                    work.add_next(beside, node);
                } else {
                    work.add_previous(beside, node);
                }
                changed = true;
            }
            previous = Some(node);
        }
        Ok(changed.then_some((work, parent, stretch)))
    }
}

impl Command<WordDoc> for MoveBlocks {
    type Out = BlocksMoved;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.plan(doc, &*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<BlocksMoved> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl MoveBlocks {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<BlocksMoved> {
        sink.read_everything();
        let docx = sink.docx()?;
        let (part, blocks, ids, anchor) = self.plan(sink.doc(), &docx)?;
        let before = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
        if self.arrange(before.root(), &part, (&blocks, &ids, &anchor))?.is_none() {
            return Ok(BlocksMoved {
                id: ids.first().cloned(),
                changed: false,
                moved: ids,
                renamed: Vec::new(),
                warnings: Vec::new(),
            });
        }
        let mut warnings: Vec<(usize, String)> = Vec::new();
        if matches!(self.policy, RevisionPolicy::Tracked(_)) {
            for (k, block) in blocks.iter().enumerate() {
                let element = element_at(&docx, &part, block).ok_or_else(|| EditError::Path(part.clone()))?;
                if block.kind == BlockKind::Control {
                    warnings.push((
                        k,
                        "is a content control: Word has no revision for a control, so it moved untracked".to_owned(),
                    ));
                } else if holds_reference(&element) {
                    warnings.push((k, "holds a note or comment reference: moved untracked".to_owned()));
                } else {
                    return Err(EditError::Unsupported(
                        "a block moved while tracking changes".to_owned(),
                    ));
                }
            }
        }
        let renamed = stamp_document(sink, &docx)?;
        // docx-agent moves the blocks one by one, the first stamping the document: a later
        // block's warning names it by its stamped id
        let warnings: Vec<String> = warnings
            .into_iter()
            .map(|(k, said)| {
                let id = ids.get(k).cloned().unwrap_or_default();
                let id = if k == 0 {
                    id
                } else {
                    crate::ranges::renamed_id(&renamed, &id)
                };
                format!("{id} {said}")
            })
            .collect();
        let docx = sink.docx()?;
        let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
        let (work, parent, stretch) = self
            .arrange(tree.root(), &part, (&blocks, &ids, &anchor))?
            .ok_or_else(|| EditError::Path(part.clone()))?;
        let built = work
            .build_content(work.root())
            .ok_or_else(|| EditError::Path(part.clone()))?;
        let keys = stretch_keys(&docx, &part, &parent, stretch);
        let count = (stretch.1 - stretch.0) as usize + 1;
        sink.splice_children(&part, (&parent, stretch.0 as usize, count), built, keys, Vec::new())?;
        let moved: Vec<String> = ids.iter().map(|id| crate::ranges::renamed_id(&renamed, id)).collect();
        Ok(BlocksMoved {
            id: moved.first().cloned(),
            changed: true,
            moved,
            renamed,
            warnings,
        })
    }
}
