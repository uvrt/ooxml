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
use crate::ranges::renamed_id;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::table_delete::{drop_separator, track_delete_table};
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
    let path_error = || EditError::Path(part.clone());
    let mut renamed = stamp_document(sink, docx)?;
    let stamped = sink.docx()?;
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let path = block.path.to_vec();
    let ((mut work, parent, range, found), names) = stretch_of(
        &stamped,
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
    let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
    let draws = draws_of(&nodes, wrote.annotations);
    let mut keys = stretch_keys(&stamped, &part, &parent, range);
    keys.push(Key::Annotations);
    let count = (range.1 - range.0) as usize + 1;
    sink.splice_children(&part, (&parent, range.0 as usize, count), nodes, keys, draws)?;
    declare(sink, &part, wrote.dated)?;
    let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
    sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    sink.release(&part, &wrote.released)?;
    if table_gone {
        renamed.retain(|(old, _)| !removed_ids.contains(old));
        return Ok(BlockDeleted {
            id: None,
            renamed,
            removed: removed_ids,
            part,
            warnings: Vec::new(),
        });
    }
    let current = if gone.is_empty() {
        renamed_id(&renamed, &block.id)
    } else {
        named_after_first_row(sink, (&part, &path), &block.id, &mut renamed)?
    };
    Ok(BlockDeleted {
        id: Some(current),
        renamed,
        removed,
        part,
        warnings: Vec::new(),
    })
}

/// A table whose first row went is named after its new first row (docx-agent's
/// `_rename({old: now, current: now})`): the table's id now.
fn named_after_first_row(
    sink: &mut EditSink<'_, '_>,
    (part, path): (&str, &[u32]),
    old: &str,
    renamed: &mut Vec<(String, String)>,
) -> Result<String> {
    let current = renamed_id(renamed, old);
    let now = sink
        .docx()?
        .part(part)
        .and_then(|s| s.ids.table_at(path))
        .map(|t| t.id)
        .ok_or_else(|| EditError::Path(part.to_owned()))?;
    if now == current {
        return Ok(current);
    }
    let mut renames = vec![(old.to_owned(), now.clone())];
    if current != old {
        renames.push((current, now.clone()));
    }
    sink.rename(renames)?;
    match renamed.iter_mut().find(|(o, _)| o == old) {
        Some(slot) => slot.1 = now.clone(),
        None => renamed.push((old.to_owned(), now.clone())),
    }
    Ok(now)
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
