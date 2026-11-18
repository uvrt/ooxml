//! Owns: writing a planned [`crate::DeleteBlock`]: the stretch around the block made
//! editable, the block removed outright (untracked, or tracked and one's own insertion:
//! block comment references moved to the paragraph after it, range markers rehomed, what
//! it held released) or its paragraph recorded as deleted (docx-agent's
//! `_track_delete_block` for a paragraph), and the stretch spliced back.
//!
//! Must not: write but through the sink, or touch children outside the stretch.

use crate::delete::{
    declare, draws_of, editable, join_path, kept_renames, own_insertion, plan, stretch_keys, stretch_paths,
    BlockDeleted, Planned,
};
use crate::doc::Key;
use crate::marks::{delete_mark, mark, mark_record, take_properties};
use crate::names::{W, W14};
use crate::policy::{RevisionPolicy, Stamp, Tracking};
use crate::ranges::renamed_id;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::table_delete::{drop_separator, own_table, separator_of};
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::Element;
use std::sync::Arc;

/// Delete the block `target` names, by the sink's policy.
///
/// # Errors
/// The refusals of [`crate::delete::plan`]; [`EditError::Unsupported`] for a tracked
/// table deletion (it lands with the table edits).
pub(crate) fn delete(sink: &mut EditSink<'_, '_>, target: &str) -> Result<BlockDeleted> {
    let docx = sink.docx()?;
    let planned = plan(sink, &docx, target)?;
    let part = planned.block.part.clone();
    sink.read(block_key(&part, &planned.block.id));
    let RevisionPolicy::Tracked(tracking) = sink.policy().clone() else {
        return outright(sink, &docx, planned, None);
    };
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let element = at_path(tree.root(), &planned.block.path).ok_or_else(|| EditError::Path(part.clone()))?;
    let work = Work::of(element, Vec::new());
    if planned.block.paragraph {
        if own_insertion(&work, work.root(), &tracking.author) {
            return outright(sink, &docx, planned, None);
        }
        return tracked(sink, &docx, &planned, &tracking);
    }
    let separator = separator(tree.root(), &docx, &planned, &tracking.author);
    if own_table(&work, work.root(), &tracking.author) {
        return outright(sink, &docx, planned, separator);
    }
    crate::table_delete_write::tracked_table(sink, &docx, planned, separator, &tracking)
}

/// The separator paragraph a table docx-agent inserted came with: its place and id.
fn separator(root: &Arc<Element>, docx: &Docx, planned: &Planned, author: &str) -> Option<(Vec<u32>, String)> {
    let (&index, above) = planned.block.path.split_last()?;
    let parent = if above.is_empty() { root } else { at_path(root, above)? };
    let k = separator_of(parent, index as usize, author)?;
    let mut path = above.to_vec();
    path.push(u32::try_from(k).ok()?);
    let id = docx.part(&planned.block.part)?.ids.paragraph_at(&path)?.id;
    Some((path, id))
}

/// The block removed outright.
fn outright(
    sink: &mut EditSink<'_, '_>,
    docx: &Docx,
    planned: Planned,
    separator: Option<(Vec<u32>, String)>,
) -> Result<BlockDeleted> {
    let part = planned.block.part.clone();
    let path_error = || EditError::Path(part.clone());
    let mut renamed = kept_renames(sink, docx, &planned.removed)?;
    let mut removed = planned.removed;
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let path = planned.block.path.to_vec();
    let element = at_path(tree.root(), &path).ok_or_else(path_error)?;
    let mut paths = stretch_paths(tree.root(), element, &path, planned.moving);
    let mut wanted: Vec<&[u32]> = vec![&path];
    if let Some((at, _)) = &separator {
        paths.push(at.clone());
        wanted.push(at);
    }
    let (mut work, parent, range, found) = editable(tree.root(), &paths, &wanted).ok_or_else(path_error)?;
    let &block = found.first().ok_or_else(path_error)?;
    let released = if matches!(sink.policy(), RevisionPolicy::Tracked(_)) {
        crate::inline::relationship_ids(&work, block)
    } else {
        Vec::new()
    };
    if planned.moving {
        let following = crate::across::neighbour(&work, block, true).ok_or_else(|| {
            EditError::Refused("a comment's reference has no paragraph after it to move to".to_owned())
        })?;
        let runs: Vec<Id> = work
            .iter(block, W, &["commentReference"])
            .into_iter()
            .filter_map(|r| work.parent(r))
            .collect();
        let (holder, mut at) = crate::inline::position(&mut work, following, 0)?;
        for run in runs {
            work.insert(holder, at, run);
            at += 1;
        }
    }
    crate::across::rehome_markers(&mut work, block);
    work.remove(block);
    if let (Some((_, id)), Some(&paragraph)) = (&separator, found.get(1)) {
        if drop_separator(&mut work, paragraph) {
            removed.push(id.clone());
            renamed.retain(|(old, _)| old != id);
        }
    }
    let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
    let keys = stretch_keys(docx, &part, &parent, range);
    let count = (range.1 - range.0) as usize + 1;
    sink.splice_children(&part, (&parent, range.0 as usize, count), nodes, keys, Vec::new())?;
    sink.release(&part, &released)?;
    Ok(BlockDeleted {
        id: None,
        renamed,
        removed,
        part,
    })
}

/// A paragraph's tracked deletion: its content and mark recorded as deleted; last in its
/// container after a paragraph, that paragraph's mark is the deleted one, and this one
/// takes its properties (docx-agent's `_track_delete_block`).
pub(crate) fn track_delete_paragraph(
    work: &mut Work,
    paragraph: Id,
    previous: Option<Id>,
    stamp: &mut Stamp,
    orders: &Orders,
) -> Vec<String> {
    let runs = crate::text::runs(work, paragraph);
    let released = crate::track::delete_runs(work, &runs, stamp);
    match previous {
        Some(previous) => {
            let properties = match work.find(previous, W, "pPr") {
                Some(p) => work.deep_copy(p),
                None => work.new_element(work.element_name(previous, W, "pPr", "w")),
            };
            delete_mark(work, previous, stamp, orders);
            take_properties(work, paragraph, properties, stamp, orders);
        }
        None if mark_record(work, paragraph).is_none() => {
            mark(work, paragraph, "del", stamp, orders);
        }
        None => {}
    }
    released
}

/// Whether `node` is below `ancestor` in the work.
fn below(work: &Work, node: Id, ancestor: Id) -> bool {
    let mut at = work.parent(node);
    while let Some(n) = at {
        if n == ancestor {
            return true;
        }
        at = work.parent(n);
    }
    false
}

/// The tracked deletion of a paragraph that is not wholly one's own insertion.
fn tracked(sink: &mut EditSink<'_, '_>, docx: &Docx, planned: &Planned, tracking: &Tracking) -> Result<BlockDeleted> {
    let block = &planned.block;
    let part = block.part.clone();
    let path_error = || EditError::Path(part.clone());
    let renamed = stamp_document(sink, docx)?;
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let path = block.path.to_vec();
    let join = join_path(tree.root(), &path).filter(|p| {
        at_path(tree.root(), p).is_some_and(|e| e.name().is(Some(W), "p") && !crate::block_checks::ends_section(e))
    });
    let mut paths = vec![path.clone()];
    paths.extend(join.clone());
    let wanted: Vec<&[u32]> = paths.iter().map(Vec::as_slice).collect();
    let (mut work, parent, range, found) = editable(tree.root(), &paths, &wanted).ok_or_else(path_error)?;
    let &paragraph = found.first().ok_or_else(path_error)?;
    let previous = found.get(1).copied();
    let own_runs = work.iter(paragraph, W, &["r"]);
    let joined = previous.and_then(|p| work.get(p, Some(W14), "paraId").map(str::to_owned));
    let wrote = sink.delete_paragraph(&mut work, paragraph, previous)?;
    let mine = work.get(paragraph, Some(W14), "paraId").map(str::to_owned);
    if let (Some(previous), Some(joined), Some(mine)) = (previous, joined, mine) {
        if work.parent(previous).is_none() {
            // the mark was the author's own insertion: that paragraph joined into this one
            let prefix = if block.story == "body" {
                String::new()
            } else {
                format!("{}/", block.story)
            };
            if own_runs.iter().any(|r| below(&work, *r, paragraph)) {
                sink.rename(vec![(format!("{prefix}p:{joined}"), format!("{prefix}p:{mine}"))])?;
            } else {
                let name = work.qname(W14, "paraId", "w14");
                work.set(paragraph, name, &joined);
                sink.rename(vec![(format!("{prefix}p:{mine}"), format!("{prefix}p:{joined}"))])?;
            }
        }
    }
    let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
    let draws = draws_of(&nodes, wrote.annotations);
    let docx_after = sink.docx()?;
    let mut keys = stretch_keys(&docx_after, &part, &parent, range);
    keys.push(Key::Annotations);
    let count = (range.1 - range.0) as usize + 1;
    sink.splice_children(&part, (&parent, range.0 as usize, count), nodes, keys, draws)?;
    declare(sink, &part, wrote.dated)?;
    let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
    sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    sink.release(&part, &wrote.released)?;
    Ok(BlockDeleted {
        id: Some(renamed_id(&renamed, &block.id)),
        renamed,
        removed: Vec::new(),
        part,
    })
}
