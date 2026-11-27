//! Owns: writing a table's tracked deletion (docx-agent's `delete_block` for a table, by an
//! author who did not insert all of it): the stretch around the table made editable, its
//! rows deleted as Word deletes them ([`crate::table_delete`]), the table and its separator
//! paragraph gone once no row of its own is left, the stretch spliced back, and the table
//! renamed after its new first row when that row went.
//!
//! Must not: write but through the sink, or touch children outside the stretch.

use crate::block_checks::neighbour_path;
use crate::delete::{declare, draws_of, editable, stretch_keys, BlockDeleted, Editable, Planned};
use crate::doc::Key;
use crate::names::W;
use crate::policy::Tracking;
use crate::sink::EditSink;
use crate::table_delete::{drop_separator, own_rows, track_delete_rows, track_delete_table};
use crate::work::Id;
use crate::{EditError, Result};
use ooxml_word_model::docx::Docx;
use std::collections::HashMap;

/// The tracked deletion of a table that is not wholly the author's own insertion.
///
/// # Errors
/// When the stretch is not where the plan found it.
pub(crate) fn tracked_table(
    sink: &mut EditSink<'_, '_>,
    docx: &Docx,
    planned: Planned,
    separator: Option<(Vec<u32>, String)>,
    tracking: &Tracking,
) -> Result<BlockDeleted> {
    let block = &planned.block;
    let part = block.part.clone();
    let path_error = || EditError::no_element(part.clone());
    let tree = sink
        .doc()
        .tree(&part)?
        .ok_or_else(|| EditError::no_part(part.clone()))?;
    let path = block.path.to_vec();
    let ((mut work, parent, range, found), names) = stretch_of(
        docx,
        tree.root(),
        (&part, &path),
        separator.as_ref().map(|(p, _)| p.as_slice()),
    )
    .ok_or_else(path_error)?;
    let &element = found.first().ok_or_else(path_error)?;
    let mut gone: Vec<Id> = Vec::new();
    let wrote = sink.track(&mut work, |work, stamp, orders| {
        let (rows, released) = track_delete_table(work, element, stamp, orders);
        gone = rows;
        released
    })?;
    let mut removed: Vec<String> = Vec::new();
    for row in &gone {
        let mut nodes = vec![*row];
        nodes.extend(work.iter(*row, W, &["p"]));
        removed.extend(nodes.iter().filter_map(|n| names.get(n).cloned()));
    }
    let table_gone = work.parent(element).is_none();
    let mut removed_ids = planned.removed.clone();
    if table_gone {
        if let (Some((_, id)), Some(&paragraph)) = (&separator, found.get(1)) {
            if drop_separator(&mut work, paragraph) {
                removed_ids.push(id.clone());
            }
        }
    }
    splice_back(sink, docx, (&part, &parent, range), &work, &wrote, tracking)?;
    if table_gone {
        return Ok(BlockDeleted {
            id: None,
            renamed: Vec::new(),
            removed: removed_ids,
            part: part.into(),
            warnings: Vec::new(),
        });
    }
    let (current, renamed) = if gone.is_empty() {
        (block.id.clone(), Vec::new())
    } else {
        named_after_first_row(sink, (&part, &path), &block.id)?
    };
    Ok(BlockDeleted {
        id: Some(current),
        renamed,
        removed,
        part: part.into(),
        warnings: Vec::new(),
    })
}

/// The tracked deletion of row `index` of the table at `path` (not its first) across the
/// stretch around the table, its neighbours in it: what another author inserted in a row
/// that was the author's own moves to the paragraph after the table, as Word moves it
/// (round 51, R103, `rows-word.json`'s MineOtherText). Whether the row went.
///
/// # Errors
/// When the table or the row is not where the caller found it.
pub(crate) fn tracked_row_across(
    sink: &mut EditSink<'_, '_>,
    (part, path): (&str, &[u32]),
    index: usize,
    tracking: &Tracking,
) -> Result<bool> {
    let path_error = || EditError::no_element(part.to_owned());
    let docx = sink.docx()?;
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::no_part(part.to_owned()))?;
    let ((mut work, parent, range, found), _) =
        stretch_of(&docx, tree.root(), (part, path), None).ok_or_else(path_error)?;
    let &table = found.first().ok_or_else(path_error)?;
    let row = *own_rows(&work, table).get(index).ok_or_else(path_error)?;
    let mut gone = false;
    let wrote = sink.track(&mut work, |work, stamp, orders| {
        let (rows, released) = track_delete_rows(work, &[row], false, stamp, orders);
        gone = !rows.is_empty();
        released
    })?;
    splice_back(sink, &docx, (part, &parent, range), &work, &wrote, tracking)?;
    Ok(gone)
}

/// The stretch's work written back in place of children `range` of `parent`, with the
/// tracked write's dates, people and released relationships.
fn splice_back(
    sink: &mut EditSink<'_, '_>,
    stamped: &Docx,
    (part, parent, range): (&str, &[u32], (u32, u32)),
    work: &crate::work::Work,
    wrote: &crate::sink::TextWrite,
    tracking: &Tracking,
) -> Result<()> {
    let nodes = work
        .build_content(work.root())
        .ok_or_else(|| EditError::no_element(part.to_owned()))?;
    let draws = draws_of(&nodes, wrote.annotations);
    let mut keys = stretch_keys(stamped, part, parent, range);
    keys.push(Key::Annotations);
    let count = (range.1 - range.0) as usize + 1;
    sink.splice_children(part, (parent, range.0 as usize, count), nodes, keys, draws)?;
    declare(sink, part, wrote.dated)?;
    let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
    sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    sink.release(part, &wrote.released)
}

/// A table whose first row went is named after its new first row (docx-agent's
/// `_rename({old: now})`): the table's id now, and the rename where it changed.
fn named_after_first_row(
    sink: &mut EditSink<'_, '_>,
    (part, path): (&str, &[u32]),
    old: &str,
) -> Result<(String, ooxml_word_model::ids::Renames)> {
    let now = sink
        .docx()?
        .part(part)
        .and_then(|s| s.ids.table_at(path))
        .map(|t| t.id)
        .ok_or_else(|| EditError::no_element(part.to_owned()))?;
    if now == old {
        return Ok((now, Vec::new()));
    }
    let renames: ooxml_word_model::ids::Renames = vec![(old.into(), now.as_str().into())];
    sink.rename(renames.clone())?;
    Ok((now, renames))
}

/// The stretch around the table at `path` made editable: the table's neighbours and its
/// separator in it; the work's nodes for the table (and separator), and the ids of the
/// table's own rows and of its paragraphs by their nodes.
fn stretch_of(
    stamped: &Docx,
    root: &std::sync::Arc<ooxml_xml::Element>,
    (part, path): (&str, &[u32]),
    separator: Option<&[u32]>,
) -> Option<(Editable, HashMap<Id, String>)> {
    let story = stamped.part(part)?;
    let table = story.ids.table_at(path)?;
    let mut named: Vec<(Vec<u32>, String)> = table.rows.iter().map(|(p, id)| (p.to_vec(), id.clone())).collect();
    named.extend(
        story
            .ids
            .paragraphs_within(path)
            .into_iter()
            .map(|p| (p.path.to_vec(), p.id)),
    );
    let mut paths = vec![path.to_vec()];
    paths.extend(neighbour_path(root, path, true));
    paths.extend(neighbour_path(root, path, false));
    let mut wanted: Vec<&[u32]> = vec![path];
    if let Some(at) = separator {
        paths.push(at.to_vec());
        wanted.push(at);
    }
    let first_named = wanted.len();
    wanted.extend(named.iter().map(|(p, _)| p.as_slice()));
    let (work, parent, range, found) = editable(root, &paths, &wanted)?;
    let names: HashMap<Id, String> = found
        .iter()
        .skip(first_named)
        .copied()
        .zip(named.into_iter().map(|(_, id)| id))
        .collect();
    let found = found.into_iter().take(first_named).collect();
    Some(((work, parent, range, found), names))
}
