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
//! ids and its part's relationships. Tables with pending tracked structure, and rows
//! holding note or comment references, are not ported: they are refused as
//! [`EditError::Unsupported`] for the host to answer its own way.

use super::new_row::{merges_note, row_like, vmerge, Beside};
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
use crate::table_delete::{row_fate, track_delete_rows, Fate};
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
    pub renamed: Vec<(String, String)>,
    /// What it did that the caller did not ask for.
    pub warnings: Vec<String>,
    /// Nothing changed (the cells merged already, a split into one cell).
    pub unchanged: bool,
}

/// Whether `table` holds tracked structure -- a row or cell deleted or inserted, cells
/// merged, a deleted paragraph mark -- that its accepted view counts otherwise
/// (docx-agent's `_Accepted` positions, not ported yet).
#[must_use]
pub fn pending_structure(table: &Element) -> bool {
    fn walk(element: &Element, parent: Option<&str>) -> bool {
        element.children().iter().any(|n| {
            let Node::Element(child) = n else { return false };
            let local = child.name().local();
            let held = child.name().namespace() == Some(W);
            if held && matches!(local, "cellDel" | "cellMerge" | "cellIns") {
                return true;
            }
            if held && local == "del" && matches!(parent, Some("trPr" | "rPr")) {
                return true;
            }
            walk(child, Some(local))
        })
    }
    walk(table, None)
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
fn placed(doc: &WordDoc, key: &str) -> bool {
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
    fn check(&self, doc: &WordDoc, found: &Found) -> Result<usize> {
        if pending_structure(&found.element) {
            return Err(EditError::Unsupported(
                "word_edit_table rows and columns of a table with pending tracked structure".to_owned(),
            ));
        }
        if placed(doc, &found.id) {
            return Err(EditError::Unsupported(
                "word_edit_table rows of a table placed from a source (its record kept)".to_owned(),
            ));
        }
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
                return super::columns::check_column(&work, &found.id, index).map(|()| index)
            }
            TableChange::Merge { first, last } => return super::merge::rectangle(&work, first, last).map(|_| 0),
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
        }
        Ok(index)
    }
}

impl Command<WordDoc> for EditTable {
    type Out = TableChanged;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        self.check(doc, &find_table(doc, &docx, &self.table)?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<TableChanged> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
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
        let mut chain = Vec::new();
        let mut at = node;
        while let Some(parent) = self.work.parent(at) {
            chain.push(self.work.children(parent).iter().position(|c| *c == at)?);
            at = parent;
        }
        let mut path = self.found.path.clone();
        let mut element = &self.table;
        for k in chain.into_iter().rev() {
            let (place, child) = element
                .children()
                .iter()
                .enumerate()
                .filter(|(_, n)| !matches!(n, Node::Text(_)))
                .nth(k)?;
            path.push(u32::try_from(place).ok()?);
            let Node::Element(child) = child else { return None };
            element = child;
        }
        Some(path)
    }
}

impl EditTable {
    /// What a merge already made or a split into one cell answers: nothing written, not
    /// even the document's ids stamped (docx-agent returns before it prepares).
    fn unchanged(&self, found: &Found) -> Result<Option<TableChanged>> {
        let work = Work::of(&found.element, Vec::new());
        let warnings = match self.change {
            TableChange::Merge { first, last } => {
                let rectangle = super::merge::rectangle(&work, first, last)?;
                if !super::merge::merged_already(&work, &rectangle.plan) {
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

    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<TableChanged> {
        let docx = sink.docx()?;
        let found = find_table(sink.doc(), &docx, &self.table)?;
        let index = self.check(sink.doc(), &found)?;
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
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let element = at_path(tree.root(), &found.path).ok_or_else(|| EditError::Path(found.part.clone()))?;
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
            TableChange::InsertRow { below, plain, .. } => insert_row(sink, &mut held, index, below, plain, orders)?,
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
                    .ok_or_else(|| EditError::Path(held.found.part.clone()))?;
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
        .ok_or_else(|| EditError::Path(held.found.part.clone()))?;
    write(sink, &held.found, &held.id, new, &Written::of(tracked))
}

/// `insert_row`: a new row beside row `index`, stamped with fresh ids, its insertion
/// tracked under the policy.
fn insert_row(
    sink: &mut EditSink<'_, '_>,
    held: &mut Held,
    index: usize,
    below: bool,
    plain: bool,
    orders: &Orders,
) -> Result<TableChanged> {
    let work = &mut held.work;
    let table = work.root();
    let rows = own_rows(work, table);
    let reference = *rows
        .get(index)
        .ok_or_else(|| EditError::Path(held.found.part.clone()))?;
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
    };
    let (row, note) = row_like(work, table, &beside, orders);
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
    let tracked = under_policy(sink, |stamp| {
        if let Some(stamp) = stamp {
            track_insert(&mut held.work, row, &paragraphs, stamp, orders);
        }
        Ok(())
    })?;
    let at = index + usize::from(below);
    let beside = merges_note(&held.work, &own_rows(&held.work, table), at);
    put(sink, held, tracked)?;
    sink.ensure_w14(&held.found.part, false, &[])?;
    Ok(TableChanged {
        id: Some(new_id.clone()),
        created: vec![new_id],
        warnings: note.into_iter().chain(beside).collect(),
        ..TableChanged::default()
    })
}

/// A new row's insertion recorded: the row's and each paragraph mark's.
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
        .ok_or_else(|| EditError::Path(held.found.part.clone()))?;
    let below = rows.get(index + 1).map(|r| row_cells(work, *r)).unwrap_or_default();
    let restarts: Vec<Id> = row_cells(work, row)
        .into_iter()
        .filter(|p| vmerge(work, p.cell).as_deref() == Some("restart"))
        .filter_map(|p| below.iter().find(|b| b.start == p.start).map(|b| b.cell))
        .filter(|cell| vmerge(work, *cell).as_deref() == Some("continue"))
        .collect();
    let inside = paragraphs_of_row(docx, &held.found, index);
    let mut removed = vec![old_row];
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
