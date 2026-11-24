//! Owns: [`crate::DeleteBlock`] for a block-level content control (`cc:`, `cc@`), as
//! docx-agent's `_delete_control_block` and `remove_control(keep_content=False)` delete it:
//! refused before anything changes for a lock, a section break, a note or comment reference
//! inside, or a container left without a paragraph to end on. Untracked, the control goes
//! with its content (range markers whose partner stays rehomed, its relationships
//! released); tracked, each paragraph of its content is recorded as deleted and each table
//! deleted row by row as Word deletes rows ([`crate::table_delete::track_delete_rows`]),
//! and the control itself is removed untracked -- Word has no revision for it -- with a
//! warning saying so. Either way a comment or bookmark mark Word displaced beside the
//! control (`w:displacedByCustomXml`) no longer says so (`_undisplace`).
//!
//! Must not: write but through the sink, touch children outside the stretch around the
//! control, or take a control that is not a block (the block resolver refuses it).

use crate::blocks::resolve_block;
use crate::controls::undisplace;
use crate::delete::{declare, draws_of, editable, stretch_keys, stretch_paths, BlockDeleted};
use crate::delete_write::track_delete_paragraph;
use crate::doc::Key;
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::table_delete::{own_rows, track_delete_rows};
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::Element;
use std::sync::Arc;

/// docx-agent's warning for a tracked deletion of a control.
const UNTRACKED: &str = "Word has no revision for a control itself: it is removed untracked";

/// The reference elements a control deleted with its content may not hold (`_REFERENCES`).
const REFERENCES: [&str; 3] = ["footnoteReference", "endnoteReference", "commentReference"];

/// Whether `target` names a content control.
pub(crate) fn is_control(target: &str) -> bool {
    target.starts_with("cc:") || target.starts_with("cc@")
}

/// A control to delete: its part, place, element and the ids of the paragraphs and
/// tables inside it (the paragraphs first, then the tables, as docx-agent lists them).
struct Planned {
    part: String,
    path: Vec<u32>,
    nested: Vec<String>,
}

/// The first reference element at or below `element`, by its local name.
fn first_reference(element: &Element) -> Option<&'static str> {
    if element.name().namespace() == Some(W) {
        if let Some(local) = REFERENCES.iter().find(|l| **l == element.name().local()) {
            return Some(local);
        }
    }
    element.child_elements().find_map(|c| first_reference(c))
}

/// The control `target` names, checked as `_delete_control_block` checks it.
fn plan(doc: &WordDoc, docx: &Docx, target: &str) -> Result<Planned> {
    let (part, block) = resolve_block(doc, docx, target)?;
    let tree = doc.tree(&part)?.ok_or_else(|| EditError::no_part(part.clone()))?;
    let sdt = at_path(tree.root(), &block.path).ok_or_else(|| EditError::no_element(part.clone()))?;
    crate::moving::control_movable(sdt, target, "deleting")?;
    if let Some(local) = first_reference(sdt) {
        return Err(EditError::Refused(format!(
            "{target} holds a {local}; deleting it would orphan the note or comment"
        )));
    }
    crate::block_checks::check_container_survives(tree.root(), &block.path)?;
    let story = docx.part(&part).ok_or_else(|| EditError::no_part(part.clone()))?;
    let inside = |p: &[u32]| p.len() > block.path.len() && p.starts_with(&block.path);
    let mut nested: Vec<String> = story
        .ids
        .paragraphs()
        .iter()
        .filter(|p| inside(&p.path))
        .map(|p| p.id)
        .collect();
    nested.extend(
        story
            .ids
            .tables()
            .iter()
            .filter(|t| inside(&t.path))
            .map(|t| t.id.clone()),
    );
    Ok(Planned {
        part,
        path: block.path,
        nested,
    })
}

/// Refuse what docx-agent refuses before anything changes.
///
/// # Errors
/// The refusals of [`plan`].
pub(crate) fn check(doc: &WordDoc, target: &str) -> Result<()> {
    plan(doc, &*doc.docx()?, target).map(|_| ())
}

/// The paths the stretch around the control at `path` must hold: the control, its
/// neighbours (a mark displaced beside it, the paragraph a lonely marker moves to).
fn paths_around(root: &Arc<Element>, sdt: &Element, path: &[u32]) -> Vec<Vec<u32>> {
    let mut paths = stretch_paths(root, sdt, path, false);
    if let Some((&k, above)) = path.split_last() {
        let count = if above.is_empty() {
            Some(root.children().len())
        } else {
            at_path(root, above).map(|p| p.children().len())
        };
        for side in [k.checked_sub(1), k.checked_add(1)].into_iter().flatten() {
            if count.is_some_and(|n| (side as usize) < n) {
                let mut at = above.to_vec();
                at.push(side);
                paths.push(at);
            }
        }
    }
    paths
}

/// Delete the control `target` names, by the sink's policy.
///
/// # Errors
/// The refusals of [`plan`]; when the stretch is not where the plan found it.
pub(crate) fn delete(sink: &mut EditSink<'_, '_>, target: &str) -> Result<BlockDeleted> {
    let docx = sink.docx()?;
    let planned = plan(sink.doc(), &docx, target)?;
    let part = planned.part.clone();
    let path_error = || EditError::no_element(part.clone());
    let renamed = stamp_document(sink, &docx)?;
    let stamped = sink.docx()?;
    let tree = sink
        .doc()
        .tree(&part)?
        .ok_or_else(|| EditError::no_part(part.clone()))?;
    let element = at_path(tree.root(), &planned.path).ok_or_else(path_error)?;
    let paths = paths_around(tree.root(), element, &planned.path);
    let (mut work, parent, range, found) = editable(tree.root(), &paths, &[&planned.path]).ok_or_else(path_error)?;
    let &sdt = found.first().ok_or_else(path_error)?;
    let mut keys = stretch_keys(&stamped, &part, &parent, range);
    let count = (range.1 - range.0) as usize + 1;
    let RevisionPolicy::Tracked(tracking) = sink.policy().clone() else {
        let beside: Vec<Id> = [work.previous(sdt), work.next(sdt)].into_iter().flatten().collect();
        let released = crate::inline::relationship_ids(&work, sdt);
        crate::across::rehome_markers(&mut work, sdt);
        work.remove(sdt);
        undisplace(&mut work, &beside);
        let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
        sink.splice_children(&part, (&parent, range.0 as usize, count), nodes, keys, Vec::new())?;
        sink.release(&part, &released)?;
        let mut removed = vec![target.to_owned()];
        removed.extend(planned.nested.into_iter().filter(|id| id != target));
        return Ok(BlockDeleted {
            id: None,
            renamed,
            removed,
            part,
            warnings: Vec::new(),
        });
    };
    let content = work.find(sdt, W, "sdtContent");
    let children: Vec<Id> = content.map(|c| work.children(c).to_vec()).unwrap_or_default();
    let mut beside: Vec<Id> = [work.previous(sdt), work.next(sdt)].into_iter().flatten().collect();
    beside.extend(&children);
    let of =
        |work: &Work, local: &str| -> Vec<Id> { children.iter().copied().filter(|c| work.is(*c, W, local)).collect() };
    let (paragraphs, tables) = (of(&work, "p"), of(&work, "tbl"));
    let wrote = sink.track(&mut work, |work, stamp, orders| {
        let mut released = Vec::new();
        for paragraph in paragraphs {
            released.extend(track_delete_paragraph(work, paragraph, None, stamp, orders));
        }
        for table in tables {
            let rows = own_rows(work, table);
            released.extend(track_delete_rows(work, &rows, true, stamp, orders).1);
            if own_rows(work, table).is_empty() {
                work.remove(table);
            }
        }
        released
    })?;
    let kept: Vec<Id> = children
        .iter()
        .copied()
        .filter(|c| content.is_some_and(|at| work.parent(*c) == Some(at)))
        .collect();
    for child in kept {
        work.add_previous(sdt, child);
    }
    work.remove(sdt);
    undisplace(&mut work, &beside);
    let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
    let draws = draws_of(&nodes, wrote.annotations);
    keys.push(Key::Annotations);
    sink.splice_children(&part, (&parent, range.0 as usize, count), nodes, keys, draws)?;
    declare(sink, &part, wrote.dated)?;
    let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
    sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    sink.release(&part, &wrote.released)?;
    Ok(BlockDeleted {
        id: None,
        renamed,
        removed: vec![target.to_owned()],
        part,
        warnings: vec![UNTRACKED.to_owned()],
    })
}
