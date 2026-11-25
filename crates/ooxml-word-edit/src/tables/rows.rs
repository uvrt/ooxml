//! Owns: a table's rows inserted and deleted as docx-agent's `edit/tables.py` does it
//! ([`EditTable`]: `insert_row`, `delete_row`), untracked and tracked, as one step: the
//! document stamped first, a new row shaped as Word's Insert Rows shapes it
//! ([`super::new_row`]) with fresh ids and, tracked, its insertion recorded; a row deleted
//! with a merge down it starts carried on from the row below (tracked: as Word records a
//! divided merge), its markers rehomed and relationships released, or, tracked, deleted as
//! Word deletes rows ([`crate::table_delete`]). The other changes of [`TableChange`] --
//! columns inserted and deleted, a row copied, cells merged and split -- are dispatched to
//! their modules from here; a merge already made and a split into one cell write nothing,
//! not even the document's ids.
//!
//! Must not: decide what a tool's arguments mean, or read past the table but for its
//! ids and its part's relationships. Rows holding note or comment references are not
//! ported: they are refused as [`EditError::Unsupported`] for the host to answer its own
//! way. Over pending tracked structure ([`super::pending`]), tracked: a new row's cells
//! follow the pending column changes beside them, a row deleted gives back the text pending
//! merges took from it, and a row holding a pending merge's first cell is refused.

use super::new_row::{merges_note, row_like, vmerge, Beside};
use super::pending::{follow_pending_columns, Reshape};
use super::shape::{own_rows, row_cells};
use super::tracker::TableTracker;
use super::{find_table, write, Found, Written};
use crate::across::rehome_markers;
use crate::inline::relationship_ids;
use crate::marks::{insert_in_order, mark};
use crate::names::{W, W14};
use crate::policy::{RevisionPolicy, Stamp};
use crate::review::xml::{find_path, make, set_attr};
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::stamping::{stamp_document, w14_prefix};
use crate::table_delete::{relocates_past_table, row_fate, track_delete_rows, Fate};
use crate::validate::Orders;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// What [`EditTable`] does to its table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableChange {
    /// A new row beside row `index` (the last by default), below it unless `below` is
    /// false; `plain`: one cell per grid column beside a row with a cell over several.
    InsertRow {
        /// The row beside which, from 0.
        index: Option<usize>,
        /// Below it (else above).
        below: bool,
        /// A row for values, one per column.
        plain: bool,
    },
    /// Row `index` deleted.
    DeleteRow {
        /// The row, from 0.
        index: usize,
    },
    /// A new column beside grid column `index`, to its right unless `right` is false.
    InsertColumn {
        /// The grid column beside which, from 0.
        index: usize,
        /// To its right (else its left).
        right: bool,
    },
    /// A copy of row `index` (the last by default) with its content, below it unless
    /// `below` is false.
    CopyRow {
        /// The row copied, from 0.
        index: Option<usize>,
        /// Below it (else above).
        below: bool,
    },
    /// Grid column `index` deleted.
    DeleteColumn {
        /// The grid column, from 0.
        index: usize,
    },
    /// The cells from `first` to `last` (row, grid column; inclusive) merged into one, the
    /// rectangle grown to take in every cell it touches whole.
    Merge {
        /// The top left position.
        first: (usize, usize),
        /// The bottom right position.
        last: (usize, usize),
    },
    /// The cell over `(row, column)` split into `rows` x `columns` cells (`columns` by
    /// default the cell's grid columns where it spans several, else 2).
    Split {
        /// The row, from 0.
        row: usize,
        /// The grid column, from 0.
        column: usize,
        /// Into how many rows.
        rows: usize,
        /// Into how many columns.
        columns: Option<usize>,
    },
}

/// Change a table's rows: one edit, one step.
#[derive(Debug, Clone)]
pub struct EditTable {
    /// The table (an id, an alias).
    pub table: String,
    /// What to do.
    pub change: TableChange,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What an [`EditTable`] did, as docx-agent's `EditResult` says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableChanged {
    /// The edit's id: the row made, or none.
    pub id: Option<String>,
    /// The table's id after the edit (a row put or taken at the top renames it).
    pub table: String,
    /// Ids made.
    pub created: Vec<String>,
    /// Ids removed.
    pub removed: Vec<String>,
    /// Ids renamed (stamping), old to new.
    pub renamed: ooxml_word_model::ids::Renames,
    /// What it did that the caller did not ask for.
    pub warnings: Vec<String>,
    /// Nothing changed (the cells merged already, a split into one cell).
    pub unchanged: bool,
}

/// Whether `element` holds a note or comment reference.
fn holds_reference(element: &Element) -> bool {
    element.children().iter().any(|n| {
        let Node::Element(child) = n else { return false };
        let name = child.name();
        (name.namespace() == Some(W)
            && matches!(
                name.local(),
                "footnoteReference" | "endnoteReference" | "commentReference"
            ))
            || holds_reference(child)
    })
}

/// docx-agent's record of where a placed table's values came from (`edit/provenance.py`).
const PROVENANCE: &str = "urn:uvrt:docx-agent:provenance:v1";

/// Whether the document keeps a placement record for the table `key` (a custom XML item
/// of [`PROVENANCE`], found by its root's namespace as docx-agent finds it).
pub(crate) fn placed(doc: &WordDoc, key: &str) -> bool {
    let package = doc.package();
    package
        .part_names()
        .iter()
        .filter(|name| name.starts_with("customXml/item") && !name.starts_with("customXml/itemProps"))
        .filter_map(|name| package.tree(name).ok().flatten())
        .filter(|tree| tree.root().name().namespace() == Some(PROVENANCE))
        .any(|tree| {
            tree.root()
                .child_elements()
                .any(|s| s.attribute(None, "kind") == Some("table") && s.attribute(None, "key") == Some(key))
        })
}

impl EditTable {
    /// The change's arguments against the table as it is; the row it names.
    fn check(&self, doc: &WordDoc, found: &Found, placing: Placing) -> Result<usize> {
        if !placing.recorded && placed(doc, &found.id) {
            return Err(EditError::Unsupported(
                "a change of a table placed from a source: its record is kept through KeepingRecord".to_owned(),
            ));
        }
        let tracked = matches!(self.policy, RevisionPolicy::Tracked(_));
        let work = Work::of(&found.element, Vec::new());
        let rows = own_rows(&work, work.root());
        let index = match self.change {
            TableChange::CopyRow { index, .. } => {
                let index = index.unwrap_or(rows.len().saturating_sub(1));
                return super::copy_row::check_copy(&work, &found.id, &rows, index).map(|()| index);
            }
            TableChange::InsertRow { index, .. } => index.unwrap_or(rows.len().saturating_sub(1)),
            TableChange::DeleteRow { index } => index,
            TableChange::DeleteColumn { index } => {
                return super::columns::check_column(&work, &found.id, index, tracked).map(|()| index)
            }
            TableChange::Merge { first, last } => {
                return super::merge::rectangle(&work, first, last, tracked).map(|_| 0)
            }
            TableChange::Split {
                row,
                column,
                rows,
                columns,
            } => return super::split::plan_split(&work, &found.id, (row, column), rows, columns).map(|_| 0),
            TableChange::InsertColumn { index, .. } => {
                return if index < super::shape::grid_widths(&work, work.root()).len() {
                    Ok(index)
                } else {
                    Err(EditError::Refused(format!("{} has no column {index}", found.id)))
                };
            }
        };
        let Some(row) = rows.get(index).copied() else {
            return Err(EditError::Refused(format!("{} has no row {index}", found.id)));
        };
        if let TableChange::DeleteRow { .. } = self.change {
            if rows.len() == 1 {
                return Err(EditError::Refused(format!(
                    "{} has one row; delete the table instead",
                    found.id
                )));
            }
            if work.build_element(row).is_some_and(|r| holds_reference(&r)) {
                return Err(EditError::Unsupported(
                    "word_edit_table delete_row of a row holding a note or comment reference".to_owned(),
                ));
            }
            if tracked {
                super::pending::no_reshape_over_merge(&work, Reshape::DeleteRow(index), &format!("row {index}"))?;
            }
        }
        Ok(index)
    }
}

impl Command<WordDoc> for EditTable {
    type Out = TableChanged;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.validate_with(doc, Placing::default())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<TableChanged> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink, Placing::default())?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

/// How a change is made as part of placing a source's rows ([`super::place_rows`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Placing {
    /// A table with a record of its source is changed too: the placing command keeps the
    /// record.
    pub(super) recorded: bool,
    /// A new row is a body row: a header row's likeness made one (docx-agent's
    /// `set_row(header=False)` after `insert_row`).
    pub(super) body: bool,
}

impl Placing {
    /// A change of a table whose record of its source the command running it keeps
    /// ([`super::kept`]).
    pub(super) fn recorded() -> Placing {
        Placing {
            recorded: true,
            body: false,
        }
    }
}

impl EditTable {
    /// The change's arguments checked against the table as `doc` has it, made `placing`.
    pub(super) fn validate_with(&self, doc: &WordDoc, placing: Placing) -> Result<()> {
        let docx = doc.docx()?;
        self.check(doc, &find_table(doc, &docx, &self.table)?, placing)
            .map(|_| ())
    }

    /// The change made through `sink`, made `placing`.
    pub(super) fn edit_with(&self, sink: &mut EditSink<'_, '_>, placing: Placing) -> Result<TableChanged> {
        self.edit(sink, placing)
    }

    /// The change made through `sink`, as one part of a command placing a source's rows.
    pub(super) fn edit_placing(&self, sink: &mut EditSink<'_, '_>) -> Result<TableChanged> {
        self.edit(
            sink,
            Placing {
                recorded: true,
                body: true,
            },
        )
    }
}

/// The table being changed: its copy, where it is, and what its rows were.
pub(super) struct Held {
    pub(super) found: Found,
    pub(super) work: Work,
    /// The table's id once stamped.
    pub(super) id: String,
    /// The table as stamped, before the edit.
    pub(super) table: Arc<Element>,
    /// The table's place in its part's tree.
    pub(super) root: Arc<Element>,
    /// The story's name.
    pub(super) story: String,
}

impl Held {
    /// Where `node` of the unchanged copy stands in its part (a copy's children are the
    /// element's but its text).
    pub(super) fn path_of(&self, node: Id) -> Option<Vec<u32>> {
        let below = super::pending::path_below(&self.work, &self.table, node)?;
        Some(self.found.path.iter().copied().chain(below).collect())
    }
}

impl EditTable {
    /// What a merge already made or a split into one cell answers: nothing written, not
    /// even the document's ids stamped (docx-agent returns before it prepares).
    fn unchanged(&self, found: &Found) -> Result<Option<TableChanged>> {
        let work = Work::of(&found.element, Vec::new());
        let warnings = match self.change {
            TableChange::Merge { first, last } => {
                // tracked, a pending tracked merge counts as made (round 55, R115)
                let tracked = matches!(self.policy, RevisionPolicy::Tracked(_));
                let rectangle = super::merge::rectangle(&work, first, last, tracked)?;
                let plan = if tracked {
                    super::merge::tracked_plan(&work, rectangle.plan.clone())
                } else {
                    rectangle.plan.clone()
                };
                let made = super::merge::merged_already(&work, &plan)
                    || (tracked && super::merge::merged_down_pending(&work, &plan));
                if !made {
                    return Ok(None);
                }
                super::merge::grown_note(&rectangle, first, last).into_iter().collect()
            }
            TableChange::Split {
                row,
                column,
                rows,
                columns,
            } => {
                if super::split::plan_split(&work, &found.id, (row, column), rows, columns)?.is_some() {
                    return Ok(None);
                }
                Vec::new()
            }
            _ => return Ok(None),
        };
        Ok(Some(TableChanged {
            id: Some(found.id.clone()),
            table: found.id.clone(),
            warnings,
            unchanged: true,
            ..TableChanged::default()
        }))
    }

    fn edit(&self, sink: &mut EditSink<'_, '_>, placing: Placing) -> Result<TableChanged> {
        let docx = sink.docx()?;
        let found = find_table(sink.doc(), &docx, &self.table)?;
        let index = self.check(sink.doc(), &found, placing)?;
        if let Some(unchanged) = self.unchanged(&found)? {
            return Ok(unchanged);
        }
        // the row's id as it was named before stamping (docx-agent's `entry.rows`)
        let old_row = docx
            .part(&found.part)
            .and_then(|p| p.ids.table_at(&found.path))
            .and_then(|t| t.rows.get(index).map(|r| r.1.clone()))
            .unwrap_or_default();
        sink.read(block_key(&found.part, &found.id));
        let renamed = stamp_document(&mut *sink, &docx)?;
        let docx = sink.docx()?;
        let story = docx.part(&found.part).map(|p| p.story.clone()).unwrap_or_default();
        let id = docx
            .part(&found.part)
            .and_then(|p| p.ids.table_at(&found.path))
            .map_or_else(|| found.id.clone(), |t| t.id);
        let tree = sink
            .doc()
            .tree(&found.part)?
            .ok_or_else(|| EditError::no_part(found.part.clone()))?;
        let element = at_path(tree.root(), &found.path).ok_or_else(|| EditError::no_element(found.part.clone()))?;
        let work = Work::of(element, scope_above(tree.root(), &found.path));
        let mut held = Held {
            found,
            work,
            id,
            table: Arc::clone(element),
            root: Arc::clone(tree.root()),
            story,
        };
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let mut out = match self.change {
            TableChange::InsertRow { below, plain, .. } => {
                let beside = (below, plain, placing.body);
                insert_row(sink, &mut held, index, beside, orders)?
            }
            TableChange::DeleteRow { .. } => delete_row(sink, &mut held, &docx, index, old_row, orders)?,
            TableChange::DeleteColumn { .. } => super::columns::delete_column(sink, &mut held, &docx, index, orders)?,
            TableChange::CopyRow { below, .. } => super::copy_row::copy_row(sink, &mut held, index, below, orders)?,
            TableChange::InsertColumn { right, .. } => {
                super::new_column::insert_column(sink, &mut held, index, right, orders)?
            }
            TableChange::Merge { first, last } => super::merge::merge_cells(sink, &mut held, first, last, orders)?,
            TableChange::Split {
                row,
                column,
                rows,
                columns,
            } => {
                let columns = super::split::plan_split(&held.work, &held.id, (row, column), rows, columns)?
                    .ok_or_else(|| EditError::no_element(held.found.part.clone()))?;
                let split = super::split::Split {
                    row,
                    column,
                    rows,
                    columns,
                };
                super::split::split_cell(sink, &mut held, split, orders)?
            }
        };
        out.renamed = renamed;
        out.table = sink
            .docx()?
            .part(&held.found.part)
            .and_then(|p| p.ids.table_at(&held.found.path))
            .map_or_else(|| held.id.clone(), |t| t.id);
        Ok(out)
    }
}

/// Run `write` under the policy: through a [`Stamp`] when tracked.
pub(super) fn under_policy(
    sink: &mut EditSink<'_, '_>,
    write: impl FnOnce(Option<&mut Stamp>) -> Result<()>,
) -> Result<Option<(crate::sink::Numbered, bool)>> {
    match sink.policy().clone() {
        RevisionPolicy::Tracked(tracking) => {
            let ((), numbered, dated) = sink.stamped(tracking, |stamp| write(Some(stamp)))?;
            Ok(Some((numbered, dated)))
        }
        RevisionPolicy::Untracked => write(None).map(|()| None),
    }
}

/// The table's copy put in place.
pub(super) fn put(
    sink: &mut EditSink<'_, '_>,
    held: &Held,
    tracked: Option<(crate::sink::Numbered, bool)>,
) -> Result<()> {
    let new = held
        .work
        .build()
        .ok_or_else(|| EditError::unbuilt(held.found.part.clone()))?;
    write(sink, &held.found, &held.id, new, &Written::of(tracked))
}

/// `insert_row`: a new row beside row `index`, stamped with fresh ids, its insertion
/// tracked under the policy.
fn insert_row(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    index: usize,
    (below, plain, body): (bool, bool, bool),
    orders: &Orders,
) -> Result<TableChanged> {
    let work = &mut held.work;
    let table = work.root();
    let rows = own_rows(work, table);
    let reference = *rows
        .get(index)
        .ok_or_else(|| EditError::no_element(held.found.part.clone()))?;
    let neighbour = if below {
        rows.get(index + 1).copied()
    } else {
        index.checked_sub(1).and_then(|k| rows.get(k).copied())
    };
    let beside = Beside {
        reference,
        neighbour,
        below,
        plain,
        body,
    };
    let (row, note, header_dropped, pending) = row_like(work, table, &beside, orders);
    if below {
        work.add_next(reference, row);
    } else {
        work.add_previous(reference, row);
    }
    let paragraphs = work.iter(row, W, &["p"]);
    let seed = format!("{}\0table", held.found.part);
    let drawn = sink.draw(&seed, 2 * (1 + paragraphs.len()))?;
    let prefix = w14_prefix(&held.root, &held.found.path);
    for (node, pair) in std::iter::once(row)
        .chain(paragraphs.iter().copied())
        .zip(drawn.chunks(2))
    {
        for (local, value) in ["paraId", "textId"].into_iter().zip(pair) {
            held.work
                .set(node, Arc::new(QName::new(Some(&prefix), local, Some(W14))), value);
        }
    }
    let new_id = format!(
        "{}tr:{}",
        if held.story == "body" {
            String::new()
        } else {
            format!("{}/", held.story)
        },
        drawn.first().map_or("", String::as_str)
    );
    // the new cells of columns whose deletion or insertion is pending go with that change
    // (round 53, R107), their records numbered before the row's own
    let is_tracked = matches!(sink.policy(), RevisionPolicy::Tracked(_));
    let tracked = match sink.policy() {
        RevisionPolicy::Untracked if pending.is_empty() => None,
        RevisionPolicy::Untracked => {
            let work = &mut held.work;
            let (_, numbered) = sink.numbered(|next| follow_pending_columns(work, &pending, next, orders))?;
            Some((numbered, false))
        }
        RevisionPolicy::Tracked(_) => under_policy(sink, |stamp| {
            if let Some(stamp) = stamp {
                let work = &mut held.work;
                let marked = follow_pending_columns(work, &pending, &mut || stamp.next_id(), orders);
                let unmarked: Vec<Id> = paragraphs.iter().copied().filter(|p| !marked.contains(p)).collect();
                track_insert(work, row, &unmarked, stamp, orders);
            }
            Ok(())
        })?,
    };
    let at = index + usize::from(below);
    let beside = merges_note(&held.work, &own_rows(&held.work, table), at);
    // a header made a body row is not tracked, as Word does not track it (docx-agent's
    // `set_row(header=False)`)
    let untracked = (header_dropped && is_tracked)
        .then(|| "Word does not track a row's height, breaking or header: applied untracked".to_owned());
    put(sink, held, tracked)?;
    sink.ensure_w14(&held.found.part, false, &[])?;
    Ok(TableChanged {
        id: Some(new_id.clone()),
        created: vec![new_id],
        warnings: note.into_iter().chain(beside).chain(untracked).collect(),
        ..TableChanged::default()
    })
}

/// A new row's insertion recorded: the row's and each paragraph mark's (`paragraphs`).
fn track_insert(work: &mut Work, row: Id, paragraphs: &[Id], stamp: &mut Stamp, orders: &Orders) {
    let properties = match work.find(row, W, "trPr") {
        Some(p) => p,
        None => {
            let p = make(work, "trPr", &[]);
            insert_in_order(work, row, p, orders);
            p
        }
    };
    let record = stamp.make(work, "ins");
    insert_in_order(work, properties, record, orders);
    for paragraph in paragraphs {
        mark(work, *paragraph, "ins", stamp, orders);
    }
}

/// `delete_row`: row `index` deleted; a merge down it starts carried on from the row below.
fn delete_row(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    docx: &Docx,
    index: usize,
    old_row: String,
    orders: &Orders,
) -> Result<TableChanged> {
    let work = &mut held.work;
    let table = work.root();
    let rows = own_rows(work, table);
    let row = *rows
        .get(index)
        .ok_or_else(|| EditError::no_element(held.found.part.clone()))?;
    let below = rows.get(index + 1).map(|r| row_cells(work, *r)).unwrap_or_default();
    let restarts: Vec<Id> = row_cells(work, row)
        .into_iter()
        .filter(|p| vmerge(work, p.cell).as_deref() == Some("restart"))
        .filter_map(|p| below.iter().find(|b| b.start == p.start).map(|b| b.cell))
        .filter(|cell| vmerge(work, *cell).as_deref() == Some("continue"))
        .collect();
    let inside = paragraphs_of_row(docx, &held.found, index);
    // the cells of pending tracked merges in the row: their text leaves the merged cell
    // again, as had the row gone first (round 55, R115)
    let taken = super::pending::taken_in(work, table, Reshape::DeleteRow(index));
    let mut removed = vec![old_row];
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        // a last row whose other author's text goes past the table: written across the
        // stretch around it (no merge down it starts, being the last; none pending in it)
        if restarts.is_empty() && taken.is_empty() && relocates_past_table(work, row, &tracking.author) {
            let gone = crate::table_delete_write::tracked_row_across(
                sink,
                (&held.found.part, &held.found.path),
                index,
                &tracking,
            )?;
            removed.extend(inside);
            return Ok(TableChanged {
                removed: if gone { removed } else { Vec::new() },
                ..TableChanged::default()
            });
        }
    }
    let mut released = Vec::new();
    let mut gone = true;
    let tracked = under_policy(sink, |stamp| {
        let work = &mut held.work;
        let tracker = stamp
            .is_some()
            .then(|| (!restarts.is_empty()).then(|| TableTracker::new(work, table, false)))
            .flatten();
        for cell in &restarts {
            if let Some(merge) = find_path(work, *cell, &["tcPr", "vMerge"]) {
                set_attr(work, merge, "val", "restart");
            }
        }
        let Some(stamp) = stamp else {
            removed.extend(inside);
            rehome_markers(work, row);
            released = relationship_ids(work, row);
            work.remove(row);
            return Ok(());
        };
        if let Some(tracker) = &tracker {
            tracker.record(work, stamp, false, true, orders);
            merges_as_cell_merges(work, tracker, stamp, orders);
        }
        if matches!(row_fate(work, row, stamp.author()), Fate::Remove | Fate::Relocate) {
            removed.extend(inside);
        }
        for (origin, cell) in &taken {
            super::pending::drop_copies(work, Some(*origin), *cell, stamp, orders);
        }
        let (taken, freed) = track_delete_rows(work, &[row], false, stamp, orders);
        gone = !taken.is_empty();
        released = freed;
        Ok(())
    })?;
    put(sink, held, tracked)?;
    sink.release(&held.found.part, &released)?;
    Ok(TableChanged {
        removed: if gone { removed } else { Vec::new() },
        ..TableChanged::default()
    })
}

/// The ids of the paragraphs in row `index` of the table (stamped).
fn paragraphs_of_row(docx: &Docx, found: &Found, index: usize) -> Vec<String> {
    let Some(part) = docx.part(&found.part) else {
        return Vec::new();
    };
    let Some(path) = part
        .ids
        .table_at(&found.path)
        .and_then(|t| t.rows.get(index).map(|r| r.0.clone()))
    else {
        return Vec::new();
    };
    let Some(row) = at_path(part.root(), &path) else {
        return Vec::new();
    };
    let mut places = Vec::new();
    paragraph_places(row, &mut path.to_vec(), &mut places);
    places
        .iter()
        .filter_map(|place| part.ids.paragraph_at(place).map(|p| p.id))
        .collect()
}

/// The places of the `w:p` under `element` (at `at`), in document order.
fn paragraph_places(element: &Element, at: &mut Vec<u32>, out: &mut Vec<Vec<u32>>) {
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        at.push(u32::try_from(k).unwrap_or(u32::MAX));
        if child.name().is(Some(W), "p") {
            out.push(at.clone());
        }
        paragraph_places(child, at, out);
        at.pop();
    }
}

/// Each cell whose vertical merge a tracked edit changed: the new merge as a
/// `w:cellMerge` (`_merges_as_cell_merges`), the form Word records a divided merge in.
pub(super) fn merges_as_cell_merges(work: &mut Work, tracker: &TableTracker, stamp: &mut Stamp, orders: &Orders) {
    let short = |v: &str| if v == "restart" { "rest" } else { "cont" };
    for &(cell, old) in tracker.cells() {
        if work.parent(cell).is_none() {
            continue;
        }
        let was = work
            .find(old, W, "vMerge")
            .map(|m| work.get(m, Some(W), "val").unwrap_or("continue").to_owned());
        let now = vmerge(work, cell);
        if was == now {
            continue;
        }
        let Some(properties) = work.find(cell, W, "tcPr") else {
            continue;
        };
        if let Some(node) = work.find(properties, W, "vMerge") {
            work.remove(node);
        }
        let record = stamp.make(work, "cellMerge");
        if let Some(now) = &now {
            set_attr(work, record, "vMerge", short(now));
        }
        if let Some(was) = &was {
            set_attr(work, record, "vMergeOrig", short(was));
        }
        insert_in_order(work, properties, record, orders);
    }
}
