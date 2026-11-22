//! Owns: what a `sync` keeps of where its values came from and what it says (docx-agent's
//! `table_sync._keep_sources`, `_said`, `_unchanged`): when every data row of the table is a
//! source row afterwards, the table's record of the source (its rows in the table's order);
//! else each cell written keeps its value's source on its paragraph (in the command's
//! transaction, [`Keeping`]) and a record the sync made untrue is dropped, with a note; the
//! reply -- counts of what was added, changed and missing or removed, each listing up to
//! [`LISTED`] keys -- and the reply of a sync with nothing to change (rule 3).
//!
//! Must not: change the document but through the sync's one command.

use super::set_from::{record as placement_record, Timing};
use super::table_sync::{Compared, Layout, Plan, Taken, LISTED};
use super::table_sync_keys::{Matched, Slot};
use super::table_view::TableView;
use crate::originals::Watch;
use crate::review::Editing;
use crate::structure::common::{outcome, Outcome};
use ooxml_store::{Command, Txn};
use ooxml_tools::{Done as Reply, ToolResult, TryCall};
use ooxml_word_edit::{EditError, KeepText, PlaceRows, PlacedRecord, RowsPlaced, TextKeep, WordDoc};
use ooxml_word_model::docx::provenance::{kind_of, provenance, Kind};
use ooxml_word_model::docx::tables::TableGrid;
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::docx::Docx;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// `{count, keys, more}` of a list (`_capped`).
fn capped(keys: &[String]) -> Value {
    let mut out = Map::new();
    out.insert("count".into(), json!(keys.len()));
    out.insert("keys".into(), json!(keys.iter().take(LISTED).collect::<Vec<_>>()));
    if keys.len() > LISTED {
        out.insert("more".into(), json!(keys.len() - LISTED));
    }
    Value::Object(out)
}

/// `value` with `key` set to `found` (where it is an object).
fn with(mut value: Value, key: &str, found: Value) -> Value {
    if let Value::Object(map) = &mut value {
        map.insert(key.into(), found);
    }
    value
}

/// What the reply says of a sync (`_said`), in docx-agent's order.
#[allow(clippy::too_many_arguments)]
pub(super) fn said(
    plan: &Plan,
    taken: &Taken<'_>,
    compared: &Compared,
    source_keys: &[Option<String>],
    gone: &[String],
    remove: bool,
    placement: &str,
    loose: Option<super::edit_table_args::Loose>,
    matched: &Matched,
) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert(
        "key".into(),
        json!({"col": plan.key_col, "column": taken.label(plan.key_pick)}),
    );
    let mapped: Map<String, Value> = plan
        .pairs
        .iter()
        .map(|(col, pick)| (col.to_string(), json!(taken.label(*pick))))
        .collect();
    out.insert("mapped".into(), Value::Object(mapped));
    let added: Vec<String> = compared
        .added
        .iter()
        .filter_map(|i| source_keys.get(*i).cloned().flatten())
        .collect();
    let mut added_said = capped(&added);
    if !added.is_empty() {
        added_said = with(added_said, "placed", json!(placement));
    }
    out.insert("added".into(), added_said);
    let keys: Map<String, Value> = compared
        .changed
        .iter()
        .take(LISTED)
        .map(|(k, labels)| (k.clone(), json!(labels)))
        .collect();
    let mut changed = json!({"count": compared.changed.len(), "cells": compared.writes.len(), "keys": keys});
    if compared.changed.len() > LISTED {
        changed = with(changed, "more", json!(compared.changed.len() - LISTED));
    }
    out.insert("changed".into(), changed);
    out.insert((if remove { "removed" } else { "missing" }).into(), capped(gone));
    out.insert(
        "unchanged".into(),
        json!(plan.order.len().saturating_sub(compared.changed.len() + gone.len())),
    );
    if loose.is_some() {
        let pairs: Map<String, Value> = matched
            .loosely
            .iter()
            .take(LISTED)
            .map(|(t, d)| (t.clone(), json!(d)))
            .collect();
        let mut said = json!({"count": matched.loosely.len(), "pairs": pairs});
        if matched.loosely.len() > LISTED {
            said = with(said, "more", json!(matched.loosely.len() - LISTED));
        }
        out.insert("loose_matches".into(), said);
    }
    if !matched.paired.is_empty() {
        out.insert("paired".into(), json!(matched.paired.len()));
    }
    if !plan.keyless.is_empty() {
        out.insert("no_key_rows".into(), json!(plan.keyless.len()));
    }
    if !plan.totals.is_empty() {
        out.insert("total_rows".into(), json!(plan.totals.len()));
    }
    out
}

/// The table's id and size as accepted (`data`'s first keys).
fn size(editing: &Editing<'_>, table: &str, retry: &TryCall) -> Map<String, Value> {
    let head = editing.editor.store().snapshot();
    let found = head
        .doc()
        .docx()
        .ok()
        .and_then(|docx| super::edit_table::table_json(&docx, table, head.doc(), false, retry));
    let mut out = Map::new();
    for key in ["id", "rows", "columns"] {
        out.insert(
            key.into(),
            found.as_ref().and_then(|f| f.get(key)).cloned().unwrap_or(Value::Null),
        );
    }
    out
}

/// Rule 3: the table holds the source's values already -- nothing written (`_unchanged`).
#[allow(clippy::too_many_arguments)]
pub(super) fn unchanged(
    editing: &Editing<'_>,
    view: &TableView,
    address: &str,
    plan: &Plan,
    gone: &[String],
    (said, notes): (Map<String, Value>, Vec<String>),
    timing: Timing,
    retry: &TryCall,
) -> ToolResult<Value> {
    let mut data = size(editing, &view.table.id, retry);
    for key in ["key", "mapped"] {
        if let Some(value) = said.get(key) {
            data.insert(key.into(), value.clone());
        }
    }
    data.insert("added".into(), json!({"count": 0, "keys": []}));
    data.insert("changed".into(), json!({"count": 0, "cells": 0, "keys": {}}));
    data.insert("missing".into(), capped(gone));
    data.insert("unchanged".into(), json!(plan.order.len().saturating_sub(gone.len())));
    data.insert("timing".into(), timing.json());
    for key in ["loose_matches", "paired"] {
        if let Some(value) = said.get(key) {
            data.insert(key.into(), value.clone());
        }
    }
    for key in ["no_key_rows", "total_rows"] {
        if let Some(value) = said.get(key) {
            data.insert(key.into(), value.clone());
        }
    }
    let loosely = said
        .get("loose_matches")
        .and_then(|l| l.get("count"))
        .and_then(Value::as_u64)
        .filter(|n| *n > 0);
    let mut summary = format!("sync of {} with {address}: nothing to change", view.table.id);
    if !gone.is_empty() {
        summary.push_str(&format!("; {} missing (left; remove: true deletes them)", gone.len()));
    }
    if let Some(n) = loosely {
        summary.push_str(&format!("; {n} key(s) matched loosely"));
    }
    let mut done = Reply::new(summary);
    done.data = Some(Value::Object(data));
    let mut result = ToolResult::done(done);
    result
        .warnings
        .push("the table holds the source's values already: nothing changed".to_owned());
    result.warnings.extend(notes);
    result
}

/// One value a cell written keeps: the cell (row as the command leaves the table, column as
/// accepted) and the fact, its text and place filled in by the paragraph it went into.
#[derive(Debug, Clone)]
pub(super) struct CellKeep {
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) fact: Map<String, Value>,
}

/// What a sync keeps of where its values came from, decided before the command.
#[derive(Debug)]
pub(super) struct Kept {
    pub(super) record: PlacedRecord,
    pub(super) note: Option<String>,
    pub(super) cells: Vec<CellKeep>,
    /// The grid column of each column as accepted, where a column's deletion is pending.
    pub(super) accepted_columns: Option<Vec<usize>>,
}

/// The table's record of its source, where it has one.
fn record_of(docx: &Docx, table: &str) -> Option<Map<String, Value>> {
    (kind_of(table) == Some(Kind::Table))
        .then(|| provenance(docx).ok()?.get(table).cloned())
        .flatten()
}

/// A source's address without a row range, and its first row's number from 1 (`_base`).
fn base(address: &str) -> (String, usize) {
    if let Some((head, tail)) = address.rsplit_once("/#") {
        let (first, rest) = tail.split_once(':').unwrap_or((tail, ""));
        let rest = rest.strip_prefix('#').unwrap_or(rest);
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        if digits(first) && (rest.is_empty() || digits(rest) || tail.ends_with(':')) && !head.contains("/#") {
            if let Ok(n) = first.parse() {
                return (head.to_owned(), n);
            }
        }
    }
    (address.to_owned(), 1)
}

/// A key or column as a reference writes it (`textfacts.quote`).
fn quote(part: &str) -> String {
    if part.contains('/') || part.contains('\'') || part == "*" || part.starts_with('#') || part.starts_with('{') {
        format!("'{}'", part.replace('\'', "''"))
    } else {
        part.to_owned()
    }
}

/// The fact a document keeps of one value written (`textfacts.record`, without `v`).
fn value_fact(taken: &Taken<'_>, row: usize, column: usize, address: &str, text: &str) -> Map<String, Value> {
    let rows = taken.rows;
    let mut placed = Map::new();
    placed.insert("source".into(), json!(address));
    placed.insert("doc".into(), rows.doc.clone().map_or(Value::Null, Value::String));
    placed.insert("version".into(), rows.version.map_or(Value::Null, Value::from));
    placed.insert("text".into(), json!(text));
    if let Some(sources) = &rows.sources {
        let mut seen: Vec<String> = Vec::new();
        for source in sources.get(row).and_then(|r| r.get(column)).into_iter().flatten() {
            if !seen.contains(source) {
                seen.push(source.clone());
            }
        }
        placed.insert("sources".into(), json!(seen));
    }
    if rows.stale {
        placed.insert("stale".into(), json!(true));
    }
    let mut out = placement_record(&placed, rows.name.as_deref());
    out.shift_remove("v");
    out
}

/// Where the rows stand once the command has run: the rows deleted outright (untracked;
/// tracked, a deleted row stays as a tracked deletion) taken out.
struct After<'l> {
    layout: &'l Layout,
    taken_out: Vec<usize>,
}

impl After<'_> {
    fn shift(&self, row: usize) -> usize {
        row - self.taken_out.iter().filter(|r| **r < row).count()
    }

    fn old(&self, row: usize) -> usize {
        self.shift(self.layout.old(row))
    }

    fn added(&self, i: usize) -> usize {
        self.shift(self.layout.at_added(i))
    }
}

/// Where the values came from (`_keep_sources`), decided from the plan: the record kept
/// when every data row is a source row afterwards, else each cell written keeping its
/// value's source and the record dropped.
pub(super) fn record(
    editing: &Editing<'_>,
    view: &TableView,
    (plan, taken, matched, compared): (&Plan, &Taken<'_>, &Matched, &Compared),
    (layout, slots): (&Layout, &[Slot]),
    (height, delete): (usize, &[usize]),
    accepted_columns: Option<Vec<usize>>,
) -> Kept {
    let head = editing.editor.store().snapshot();
    let had = head.doc().docx().ok().and_then(|docx| record_of(&docx, &view.table.id));
    let after = After {
        layout,
        taken_out: if editing.tracking.is_none() {
            delete.to_vec()
        } else {
            Vec::new()
        },
    };
    if let Some(saved) = whole(view, (plan, taken, matched, compared), &after, (height, delete)) {
        return Kept {
            record: PlacedRecord::Store(placement_record(&saved, taken.rows.name.as_deref())),
            note: None,
            cells: Vec::new(),
            accepted_columns,
        };
    }
    let note = had.as_ref().map(|found| {
        format!(
            "the table's record of where its values came from ({}) no longer holds for every row: dropped; each \
             cell written keeps its value's source",
            found
                .get("source")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("a source")
        )
    });
    Kept {
        record: if had.is_some() {
            PlacedRecord::Drop
        } else {
            PlacedRecord::Leave
        },
        note,
        cells: cell_keeps(plan, taken, (compared, slots), &after),
        accepted_columns,
    }
}

/// The table's record of its source when every data row is a source row once the command
/// has run, in the table's order; `None` otherwise.
fn whole(
    view: &TableView,
    (plan, taken, matched, compared): (&Plan, &Taken<'_>, &Matched, &Compared),
    after: &After<'_>,
    (height, delete): (usize, &[usize]),
) -> Option<Map<String, Value>> {
    let deleted_before = super::edit_table::deleted_rows(&view.table);
    let total = height + compared.added.len() - after.taken_out.len();
    // the rows accepted after: the table's not deleted before nor now, and the rows added
    let mut accepted: Vec<usize> = (0..height)
        .filter(|r| !deleted_before.contains(r))
        .map(|r| after.layout.old(r))
        .filter(|r| !delete.contains(r))
        .chain(compared.added.iter().filter_map(|i| after.layout.added(*i)))
        .map(|r| after.shift(r))
        .collect();
    accepted.sort_unstable();
    let end = total - (height - plan.end);
    let mut index_of: HashMap<usize, usize> = HashMap::new();
    for i in 0..taken.texts.len() {
        let found = super::table_sync::norm(&taken.text(i, plan.key_pick).replace('\u{b}', " "));
        if let Some(row) = matched.row_of.get(&found) {
            index_of.insert(after.old(*row), i);
        } else if after.layout.added(i).is_some() {
            index_of.insert(after.added(i), i);
        }
    }
    let body: Vec<usize> = accepted
        .iter()
        .copied()
        .filter(|r| (plan.body..end).contains(r))
        .collect();
    let order: Vec<usize> = body.iter().map(|r| index_of.get(r).copied()).collect::<Option<_>>()?;
    let top = *body.first()?;
    if order.len() != taken.texts.len() {
        return None;
    }
    let rows = taken.rows;
    let mut saved = rows.describe();
    saved.insert("header_row".into(), Value::Null);
    saved.insert(
        "first_row".into(),
        json!(accepted.iter().position(|r| *r == top).unwrap_or(0)),
    );
    let columns: Vec<Value> = plan
        .pairs
        .iter()
        .map(|(col, pick)| {
            let mut entry = Map::new();
            entry.insert("col".into(), json!(col));
            entry.extend(rows.provenance(taken.fields.get(*pick).copied().unwrap_or(0), &order));
            Value::Object(entry)
        })
        .collect();
    saved.insert("columns".into(), Value::Array(columns));
    Some(saved)
}

/// Each cell written, keeping its value's source: the matched rows' cells rewritten, then
/// the rows added's values slot by slot (a blank one left out).
fn cell_keeps(
    plan: &Plan,
    taken: &Taken<'_>,
    (compared, slots): (&Compared, &[Slot]),
    after: &After<'_>,
) -> Vec<CellKeep> {
    let rows = taken.rows;
    let (address, first_row) = base(&rows.address);
    let mut cells: Vec<(usize, usize, usize, usize)> = compared
        .writes
        .iter()
        .map(|w| (after.old(w.row), w.col, w.source, w.pick))
        .collect();
    for i in slots.iter().flat_map(|slot| &slot.rows) {
        for (col, pick) in &plan.pairs {
            if !taken.text(*i, *pick).is_empty() {
                cells.push((after.added(*i), *col, *i, *pick));
            }
        }
    }
    cells
        .into_iter()
        .filter_map(|(row, col, i, pick)| {
            let column = *taken.fields.get(pick)?;
            let named = format!("{address}/#{}/{}", first_row + i, quote(rows.columns.get(column)?));
            Some(CellKeep {
                row,
                column: col,
                fact: value_fact(taken, i, column, &named, taken.text(i, pick)),
            })
        })
        .collect()
}

/// The sync's command, then each cell written keeping its value's source, as one
/// transaction.
pub(super) struct Keeping<'a> {
    pub(super) command: &'a PlaceRows,
    pub(super) cells: &'a [CellKeep],
    pub(super) accepted: Option<&'a [usize]>,
    pub(super) retry: &'a TryCall,
}

impl Command<WordDoc> for Keeping<'_> {
    type Out = (RowsPlaced, bool);
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.command.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Self::Out, EditError> {
        let out = self.command.prepare(tx)?;
        if self.cells.is_empty() {
            return Ok((out, false));
        }
        let docx = tx.doc().docx()?;
        let keeps = keeps(&docx, tx.doc(), &out.table, self.cells, (self.accepted, self.retry));
        if keeps.is_empty() {
            return Ok((out, false));
        }
        let keep = KeepText { keeps };
        keep.validate(tx.doc())?;
        keep.prepare(tx)?;
        Ok((out, true))
    }
}

/// The keeps of the cells written, each on the cell's paragraphs with a durable id and text.
pub(super) fn keeps(
    docx: &Docx,
    doc: &WordDoc,
    table: &str,
    cells: &[CellKeep],
    (accepted, retry): (Option<&[usize]>, &TryCall),
) -> Vec<TextKeep> {
    let Ok(view) = super::table_view::table_view(doc, docx, table, retry) else {
        return Vec::new();
    };
    let Some(part) = docx.part(&view.part) else {
        return Vec::new();
    };
    let grid = TableGrid::new(part, &view.table);
    let mut out = Vec::new();
    for cell in cells {
        let column = accepted.map_or(cell.column, |kept| {
            kept.get(cell.column).copied().unwrap_or(cell.column)
        });
        for k in grid.cell_paragraphs(cell.row, column) {
            let Some(paragraph) = part.ids.paragraphs().get(k) else {
                continue;
            };
            let text = paragraph_text(&paragraph.element, View::Current);
            if !paragraph.id.rsplit('/').next().unwrap_or_default().starts_with("p:") || text.is_empty() {
                continue;
            }
            let mut fact = cell.fact.clone();
            fact.insert("text".into(), json!(text));
            fact.insert("p".into(), json!(0));
            fact.insert("at".into(), json!(0));
            out.push(TextKeep {
                paragraph: paragraph.id.clone(),
                facts: vec![Value::Object(fact)],
                labels: Vec::new(),
            });
        }
    }
    out
}

/// The watch on the file's text the rows removed take out of the final view: each row by
/// its place on `now` (the version before), named as the table `named` has it when the row
/// is deleted (after the rows added).
pub(super) fn watch<'a>(
    editing: &Editing<'_>,
    now: &'a WordDoc,
    view: &TableView,
    removed: &[(usize, usize)],
    named: &str,
) -> Option<Watch<'a>> {
    if removed.is_empty() {
        return None;
    }
    let opened = editing.opened.docx().ok()?;
    let docx = now.docx().ok()?;
    let mut watch = Watch::new(opened, now);
    for (row, at) in removed {
        watch.row_as(&docx, &view.part, &view.table, *row, (named, *at));
    }
    Some(watch)
}

/// What a sync that changed the table says, beside its edits.
pub(super) struct Done<'d> {
    pub(super) said: Map<String, Value>,
    pub(super) notes: Vec<String>,
    pub(super) address: &'d str,
    pub(super) added: usize,
    pub(super) changed: usize,
    pub(super) gone: usize,
    pub(super) remove: bool,
    pub(super) loosely: usize,
}

/// The reply of a sync that changed the table (`sync`'s end).
pub(super) fn reply(
    editing: &Editing<'_>,
    view: &TableView,
    placed: RowsPlaced,
    (note, done): (Option<String>, Done<'_>),
    retry: &TryCall,
) -> ToolResult<Value> {
    let table = view.table.id.as_str();
    let now = placed.table.clone();
    let mut data = size(editing, &now, retry);
    data.extend(done.said);
    let mut summary = format!(
        "sync of {now} with {}: {} added, {} changed",
        done.address, done.added, done.changed
    );
    if done.gone > 0 {
        summary.push_str(&if done.remove {
            format!(", {} removed", done.gone)
        } else {
            format!(", {} missing (left; remove: true deletes them)", done.gone)
        });
    }
    if done.loosely > 0 {
        summary.push_str(&format!("; {} key(s) matched loosely", done.loosely));
    }
    let created: Vec<String> = placed.added.iter().filter_map(|r| r.id.clone()).collect();
    let removed: Vec<String> = placed
        .deleted
        .iter()
        .flat_map(|d| d.removed.iter())
        .filter(|id| id.rsplit('/').next().unwrap_or_default().starts_with("tr:"))
        .cloned()
        .collect();
    let mut edits: Vec<Outcome> = placed.added.into_iter().map(Outcome::from).collect();
    edits.extend(placed.cells.edits.into_iter().map(Outcome::from));
    edits.extend(placed.deleted.into_iter().map(Outcome::from));
    let mut result = outcome(&edits, summary, Some(Value::Object(data)));
    if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
        found.changed = vec![now.clone()];
        found.created = created;
        found.removed = removed;
        if now != table {
            match found.renamed.iter_mut().find(|(old, _)| old == table) {
                Some(entry) => entry.1.clone_from(&now),
                None => found.renamed.push((table.to_owned(), now.clone())),
            }
        }
    }
    result.warnings.extend(done.notes);
    result.warnings.extend(note);
    result
}
