//! Owns: `word_edit_table`'s `sync` (docx-agent's `tools/table_sync.py`, round 49, R90): a
//! table kept in step with a source's rows by a key column, in one call -- its arguments
//! read as meant (`_sync`), the columns taken (`from_columns`, else the header labels that
//! name source columns), the key column, the body rows (below the header rows, above the
//! total rows; rows with no key counted, not matched), the keys matched
//! ([`super::table_sync_keys`]), the cells that differ rewritten (equal ones and the key
//! cell never), the rows of new keys added beside the data rows (in key order where the
//! table is in it, else in the source's), the rows of keys gone listed or (`remove`)
//! deleted -- all as one command ([`PlaceRows`]) -- and the reply
//! ([`super::table_sync_said`]).
//!
//! Positions count the table's columns as accepted (a column whose deletion is pending left
//! out); its rows as the document has them, a row whose deletion is pending no data row.
//!
//! Must not: change the document but through the one command; change anything before every
//! refusal has been weighed.

use super::edit_table_args::{Loose, WordEditTable};
use super::set_from::Timing;
use super::table_sync_keys::{matches, merged_refusal, note_loose, slots, sorted, Matched, Slot};
use super::table_sync_read::refusal;
use super::table_sync_read::{body, key_column, mapping, source_keys, sync_grid, table_keys};
use super::table_sync_said::{record, reply, said, unchanged, Done, Kept};
use super::table_view::{AcceptedView, TableView};
use crate::review::Editing;
use crate::sources::{placed, SourceRows};
use crate::structure::common::{policy, refused};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{CellAxis, CellWrite, EditError, PlaceRows, RowsBeside, RowsPlaced, WordDoc};
use ooxml_word_model::docx::Docx;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

/// Keys each list of the reply names at most (rule 4); the rest are counted.
pub(super) const LISTED: usize = 20;

/// A cell of the table as sync reads it: its origin (document row, column as accepted) and
/// its text (paragraphs one line each).
#[derive(Debug, Clone)]
pub(super) struct Cell {
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) text: String,
}

/// Per document row, per column as accepted: the cell there.
pub(super) type Grid = Vec<Vec<Option<Cell>>>;

/// White space runs as one space, none at the ends (`_norm`).
pub(super) fn norm(text: &str) -> String {
    text.split(crate::written::py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a sync reads before it changes anything (`_Plan`).
#[derive(Debug, Default)]
pub(super) struct Plan {
    /// (table column, index into the block's columns).
    pub(super) pairs: Vec<(usize, usize)>,
    pub(super) key_col: usize,
    /// The key's index into the block's columns.
    pub(super) key_pick: usize,
    /// The first body row, and the row after the last.
    pub(super) body: usize,
    pub(super) end: usize,
    /// Table key -> row.
    pub(super) by_key: HashMap<String, usize>,
    /// The table's keys, in table order.
    pub(super) order: Vec<String>,
    /// Body rows with no key.
    pub(super) keyless: Vec<usize>,
    /// Body rows computed by a formula field.
    pub(super) totals: Vec<usize>,
    /// Columns taken by `from_columns` rather than by the header's labels.
    pub(super) by_columns: bool,
}

/// What the source gives a sync: its rows, the columns taken (source columns, in pick
/// order) and each row's texts per pick.
pub(super) struct Taken<'r> {
    pub(super) rows: &'r SourceRows,
    pub(super) fields: Vec<usize>,
    pub(super) texts: Vec<Vec<String>>,
}

impl Taken<'_> {
    /// Source row `i`'s text at `pick`.
    pub(super) fn text(&self, i: usize, pick: usize) -> &str {
        self.texts
            .get(i)
            .and_then(|line| line.get(pick))
            .map_or("", String::as_str)
    }

    /// How a reply names a column taken (`_label`): the source column's name.
    pub(super) fn label(&self, pick: usize) -> String {
        self.fields
            .get(pick)
            .and_then(|f| self.rows.columns.get(*f))
            .cloned()
            .unwrap_or_default()
    }
}

/// What a sync read and weighed before it changes anything.
struct Weighed<'r> {
    plan: Plan,
    grid: Grid,
    taken: Taken<'r>,
    source_keys: Vec<Option<String>>,
    matched: Matched,
    compared: Compared,
    gone: Vec<String>,
    slots: Vec<Slot>,
    notes: Vec<String>,
}

/// `sync` from `rows` into the table `view` (found on `docx`, the version before).
pub(super) fn sync(
    args: &WordEditTable,
    editing: &Editing<'_>,
    docx: &Docx,
    view: &TableView,
    accepted: &AcceptedView,
    rows: &SourceRows,
    retry: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let notes = arguments(args, retry)?;
    let mut timing = Timing::default();
    let started = Instant::now();
    let (plan, grid, taken) = read(args, editing, docx, view, accepted, rows, retry)?;
    timing.add("resolve", started);
    let started = Instant::now();
    let weighed = weigh(args, view, (plan, grid, taken), notes, retry)?;
    timing.add("compare", started);
    let remove = args.remove == Some(true);
    let in_order = sorted(&weighed.plan.order);
    let placement = if in_order { "key order" } else { "source order" };
    let summary_of = said(
        &weighed.plan,
        &weighed.taken,
        &weighed.compared,
        &weighed.source_keys,
        &weighed.gone,
        remove,
        placement,
        args.loose,
        &weighed.matched,
    );
    let Weighed {
        plan,
        matched,
        compared,
        gone,
        notes,
        ..
    } = &weighed;
    if compared.writes.is_empty() && compared.added.is_empty() && !(remove && !gone.is_empty()) {
        let said = (summary_of, notes.clone());
        return Ok(unchanged(editing, view, &rows.address, plan, gone, said, timing, retry));
    }
    let (command, kept, removed) = changes(args, editing, view, accepted, &weighed);
    let done = Done {
        said: summary_of,
        notes: notes.clone(),
        address: &rows.address,
        added: compared.added.len(),
        changed: compared.changed.len(),
        gone: gone.len(),
        remove,
        loosely: matched.loosely.len(),
    };
    run(editing, view, (&command, &kept, &removed), timing, done, retry)
}

/// The one command a sync's changes are (rows added beside the data rows from the bottom,
/// the cells written, the rows of keys gone deleted), and what it keeps of where the values
/// came from.
fn changes(
    args: &WordEditTable,
    editing: &Editing<'_>,
    view: &TableView,
    accepted: &AcceptedView,
    weighed: &Weighed<'_>,
) -> (PlaceRows, Kept, Vec<(usize, usize)>) {
    let Weighed {
        plan,
        taken,
        matched,
        compared,
        gone,
        slots,
        ..
    } = weighed;
    let height = weighed.grid.len();
    let final_rows = Layout::of(height, slots);
    // the rows of keys gone: (row before, row once the rows are added), from the bottom
    let mut removed: Vec<(usize, usize)> = if args.remove == Some(true) {
        gone.iter()
            .filter_map(|k| plan.by_key.get(k))
            .map(|row| (*row, final_rows.old(*row)))
            .collect()
    } else {
        Vec::new()
    };
    removed.sort_unstable_by_key(|a| std::cmp::Reverse(a.1));
    let delete: Vec<usize> = removed.iter().map(|(_, at)| *at).collect();
    let accepted_columns = accepted.table.active().then(|| accepted.table.kept.clone());
    let kept = record(
        editing,
        view,
        (plan, taken, matched, compared),
        (&final_rows, slots),
        (height, &delete),
        accepted_columns,
    );
    let command = PlaceRows {
        table: args.table.clone(),
        beside: slots
            .iter()
            .rev()
            .map(|slot| RowsBeside {
                row: Some(slot.row),
                below: slot.below,
                count: slot.rows.len(),
            })
            .collect(),
        writes: writes(plan, taken, compared, slots, &final_rows, accepted),
        delete,
        record: kept.record.clone(),
        policy: policy(editing),
    };
    (command, kept, removed)
}

/// The table and the source as sync reads them: the columns taken, the key column, the body
/// rows and the table's keys; the source's values as cell text.
fn read<'r>(
    args: &WordEditTable,
    editing: &Editing<'_>,
    docx: &Docx,
    view: &TableView,
    accepted: &AcceptedView,
    rows: &'r SourceRows,
    retry: &TryCall,
) -> Result<(Plan, Grid, Taken<'r>), ToolError> {
    let mut plan = Plan::default();
    let header = super::cell_fills::header_rows(view);
    let grid = sync_grid(docx, view, accepted);
    let (height, width) = (grid.len(), accepted.table.width());
    let from_columns = args.from_columns.as_deref();
    let fields = mapping(
        rows,
        &grid,
        (header, width),
        from_columns,
        &view.table.id,
        &mut plan,
        retry,
    )?;
    let names: Vec<String> = fields.iter().filter_map(|f| rows.columns.get(*f).cloned()).collect();
    let locale = editing.locale.cloned().unwrap_or_default();
    let date_format = args.date_format.as_deref();
    let block = placed(rows, Some(&names), Some(&[]), date_format, &locale, retry)?;
    let texts: Vec<Vec<String>> = block
        .lines(retry)?
        .into_iter()
        .map(|line| line.into_iter().map(|lines| lines.join("\u{b}")).collect())
        .collect();
    let taken = Taken { rows, fields, texts };
    key_column(&mut plan, args.key.as_deref(), &grid, header, &taken, retry)?;
    body(view, &mut plan, &grid, (header, height), &taken);
    let deleted = super::edit_table::deleted_rows(&view.table);
    table_keys(&mut plan, &grid, &deleted, &view.table.id, retry)?;
    Ok((plan, grid, taken))
}

/// The keys matched and what differs (`sync`'s comparison), every refusal before any change.
fn weigh<'r>(
    args: &WordEditTable,
    view: &TableView,
    (plan, grid, taken): (Plan, Grid, Taken<'r>),
    mut notes: Vec<String>,
    retry: &TryCall,
) -> Result<Weighed<'r>, ToolError> {
    let table = view.table.id.as_str();
    let address = &taken.rows.address;
    let source_keys = source_keys(&taken.texts, plan.key_pick, address, retry)?;
    blank_keys(&source_keys, address, &mut notes);
    let pairs: Vec<(String, String)> = args
        .pairs
        .iter()
        .flatten()
        .map(|p| (p.table.clone(), p.data.clone()))
        .collect();
    let loose = args.loose;
    let matched = matches(
        &plan,
        &source_keys,
        &pairs,
        loose.is_some(),
        (address, table),
        &mut notes,
        retry,
    )?;
    let compared = compare(&plan, &grid, &taken, &source_keys, &matched, loose, table, retry)?;
    let taken_rows: Vec<usize> = matched.row_of.values().copied().collect();
    let gone: Vec<String> = plan
        .order
        .iter()
        .filter(|k| plan.by_key.get(*k).is_some_and(|row| !taken_rows.contains(row)))
        .cloned()
        .collect();
    let slots = slots(
        &plan,
        &source_keys,
        &compared.added,
        sorted(&plan.order),
        &matched.row_of,
    );
    for slot in &slots {
        let what = format!("new rows are made like row {} of {table}", slot.row);
        merged_refusal(&what, slot.row, &grid, &plan, &taken, retry)?;
    }
    if loose.is_none() {
        let added: Vec<String> = compared
            .added
            .iter()
            .filter_map(|i| source_keys.get(*i).cloned().flatten())
            .collect();
        note_loose(&gone, &added, args.remove == Some(true), &mut notes);
    }
    Ok(Weighed {
        plan,
        grid,
        taken,
        source_keys,
        matched,
        compared,
        gone,
        slots,
        notes,
    })
}

/// The note on the source rows with a blank key, which are not placed.
fn blank_keys(source_keys: &[Option<String>], address: &str, notes: &mut Vec<String>) {
    let blank: Vec<usize> = source_keys
        .iter()
        .enumerate()
        .filter(|(_, k)| k.is_none())
        .map(|(i, _)| i)
        .collect();
    if blank.is_empty() {
        return;
    }
    let mut shown = blank
        .iter()
        .take(LISTED)
        .map(|i| format!("#{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    if blank.len() > LISTED {
        shown.push_str(&format!(" and {} more", blank.len() - LISTED));
    }
    notes.push(format!(
        "{} row(s) of {address} with a blank key not placed: {shown}",
        blank.len()
    ));
}

/// The cells written, by row as the rows added leave the table and grid column: the
/// matched rows' cells that differ, then the rows added's values (a blank one left out).
fn writes(
    plan: &Plan,
    taken: &Taken<'_>,
    compared: &Compared,
    slots: &[Slot],
    final_rows: &Layout,
    accepted: &AcceptedView,
) -> Vec<CellWrite> {
    let mut cells: Vec<(usize, usize, String)> = compared
        .writes
        .iter()
        .map(|w| (final_rows.old(w.row), w.col, w.text.clone()))
        .collect();
    for i in slots.iter().flat_map(|slot| &slot.rows) {
        for (col, pick) in &plan.pairs {
            let text = taken.text(*i, *pick);
            if !text.is_empty() {
                cells.push((final_rows.at_added(*i), *col, text.to_owned()));
            }
        }
    }
    cells.sort_by_key(|(row, col, _)| (*row, *col));
    let grid_column = |col: usize| {
        if accepted.table.active() {
            accepted.table.kept.get(col).copied().unwrap_or(col)
        } else {
            col
        }
    };
    cells
        .into_iter()
        .map(|(row, col, text)| CellWrite {
            row: CellAxis::At(row),
            column: CellAxis::At(grid_column(col)),
            text,
        })
        .collect()
}

/// The command run, the values' sources kept and the rows written looked at in its
/// transaction; the reply.
fn run(
    editing: &Editing<'_>,
    view: &TableView,
    (command, kept, removed): (&PlaceRows, &Kept, &[(usize, usize)]),
    mut timing: Timing,
    done: Done<'_>,
    retry: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let table = view.table.id.as_str();
    let before = editing.editor.store().snapshot();
    let keeping = super::table_sync_said::Keeping {
        command,
        cells: &kept.cells,
        accepted: kept.accepted_columns.as_deref(),
        retry,
    };
    let look = |out: &(RowsPlaced, bool), _: &WordDoc| {
        Some((
            out.0.table.clone(),
            out.0.added.iter().filter_map(|r| r.id.clone()).collect::<Vec<_>>(),
        ))
    };
    let ((placed, _), rows_said) = match super::rows_written::run_writing_rows(editing, &keeping, table, look, retry) {
        Ok(found) => found,
        Err(error @ (EditError::Address(_) | EditError::Argument { .. } | EditError::Position(_))) => {
            return Ok(ToolResult::failed(crate::review::edit_refusal(
                error,
                "",
                retry.clone(),
            )));
        }
        Err(error) => return Ok(refused(error, retry.clone())),
    };
    for (name, seconds) in &placed.parts {
        timing.seconds(name, *seconds);
    }
    let started = Instant::now();
    let named = placed.table.clone();
    let watched = super::table_sync_said::watch(editing, before.doc(), view, removed, &named);
    let mut result = reply(editing, view, placed, (kept.note.clone(), done), retry);
    if let Some(mut watched) = watched {
        let head = editing.editor.store().snapshot();
        crate::originals::add(&mut result, watched.report(head.doc(), editing.doc));
    }
    result.warnings.extend(rows_said);
    timing.add("result", started);
    if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
        if let Some(Value::Object(map)) = &mut found.data {
            map.insert("timing".into(), timing.json());
        }
    }
    Ok(result)
}

/// The arguments for another action refused, the ones sync does not use said (`_sync`).
fn arguments(args: &WordEditTable, retry: &TryCall) -> Result<Vec<String>, ToolError> {
    let invalid = |message: &str, field: &str, options: &[&str]| {
        ToolError::refusal(
            Refusal::InvalidArguments,
            message.to_owned(),
            WayOut::names(options.iter().copied()).unwrap_or_else(|| WayOut::call(retry.clone())),
        )
        .at(field)
    };
    if args.cells.as_ref().is_some_and(|c| !c.is_empty()) {
        return Err(invalid(
            "sync writes the cells from: cells are for set_cells; leave them out (from_columns picks the columns)",
            "cells",
            &["from_columns"],
        ));
    }
    if args.header.as_ref().is_some_and(|h| !h.is_empty()) {
        return Err(invalid(
            "sync leaves the header rows as they are: header is for set_cells from; from_columns names the source \
             column of each table column",
            "header",
            &["from_columns"],
        ));
    }
    let mut notes = Vec::new();
    if let Some(index) = args.index.filter(|i| *i != 0) {
        notes.push(format!("index {index} not used: sync places each row by its key"));
    }
    if args.grow == Some(true) {
        notes.push("grow not needed: sync adds the rows of new keys".to_owned());
    }
    Ok(notes)
}

/// One cell a sync rewrites: its row and column (as read), the text, the source row and
/// the pick.
#[derive(Debug, Clone)]
pub(super) struct Write {
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) text: String,
    pub(super) source: usize,
    pub(super) pick: usize,
}

/// What differs: the cells of matched rows, the source rows to add, the keys changed with
/// the columns that changed (in order).
#[derive(Debug, Default)]
pub(super) struct Compared {
    pub(super) writes: Vec<Write>,
    pub(super) added: Vec<usize>,
    pub(super) changed: Vec<(String, Vec<String>)>,
}

/// The matched rows' cells against the source's values (`sync`'s comparison): a cell that
/// is merged or missing where a value goes refused.
#[allow(clippy::too_many_arguments)]
fn compare(
    plan: &Plan,
    grid: &Grid,
    taken: &Taken<'_>,
    source_keys: &[Option<String>],
    matched: &Matched,
    loose: Option<Loose>,
    table: &str,
    retry: &TryCall,
) -> Result<Compared, ToolError> {
    let respell: HashMap<&str, &str> = if loose == Some(Loose::Data) {
        matched
            .paired
            .iter()
            .chain(&matched.loosely)
            .map(|(t, d)| (t.as_str(), d.as_str()))
            .collect()
    } else {
        HashMap::new()
    };
    let table_key: HashMap<usize, &str> = plan.by_key.iter().map(|(k, r)| (*r, k.as_str())).collect();
    let mut out = Compared::default();
    for (i, found) in source_keys.iter().enumerate() {
        let Some(found) = found else { continue };
        let Some(row) = matched.row_of.get(found).copied() else {
            out.added.push(i);
            continue;
        };
        let own = table_key.get(&row).copied().unwrap_or_default();
        let named = if respell.contains_key(own) { found.as_str() } else { own };
        for (col, pick) in &plan.pairs {
            if *col == plan.key_col && !respell.contains_key(own) {
                continue;
            }
            let new = taken.text(i, *pick);
            let cell = grid.get(row).and_then(|l| l.get(*col)).and_then(Option::as_ref);
            let Some(cell) = cell.filter(|c| (c.row, c.column) == (row, *col)) else {
                if cell.is_some_and(|c| c.text == new) {
                    continue;
                }
                if cell.is_some_and(|c| c.row == row) {
                    let what = format!("row {row} of {table} (key {})", repr_str(named));
                    merged_refusal(&what, row, grid, plan, taken, retry)?;
                }
                let what = cell.map_or_else(
                    || "not a cell".to_owned(),
                    |c| format!("merged with ({}, {})", c.row, c.column),
                );
                return Err(refusal(
                    Refusal::Refused,
                    format!("({row}, {col}) of {table} is {what}: sync writes one value per cell; split it first"),
                    "from",
                    Vec::new(),
                    retry,
                ));
            };
            if cell.text != new && (*col != plan.key_col || norm(&cell.text) != *found) {
                out.writes.push(Write {
                    row,
                    col: *col,
                    text: new.to_owned(),
                    source: i,
                    pick: *pick,
                });
                let label = taken.label(*pick);
                match out.changed.iter_mut().find(|(k, _)| k == named) {
                    Some((_, labels)) => labels.push(label),
                    None => out.changed.push((named.to_owned(), vec![label])),
                }
            }
        }
    }
    Ok(out)
}

/// Each row's place after the rows are added (`_layout`).
pub(super) struct Layout {
    places: HashMap<(bool, usize), usize>,
}

impl Layout {
    /// The places in a table of `height` rows once `slots` are added.
    fn of(height: usize, slots: &[Slot]) -> Layout {
        let mut places = HashMap::new();
        let mut k = 0;
        let mut put = |key: (bool, usize)| {
            places.insert(key, k);
            k += 1;
        };
        for row in 0..height {
            for slot in slots.iter().filter(|s| s.row == row && !s.below) {
                slot.rows.iter().for_each(|i| put((true, *i)));
            }
            put((false, row));
            for slot in slots.iter().filter(|s| s.row == row && s.below) {
                slot.rows.iter().for_each(|i| put((true, *i)));
            }
        }
        Layout { places }
    }

    /// Where the table's row `row` is now.
    pub(super) fn old(&self, row: usize) -> usize {
        self.places.get(&(false, row)).copied().unwrap_or(row)
    }

    /// Where the row added for source row `i` is, if one was.
    pub(super) fn added(&self, i: usize) -> Option<usize> {
        self.places.get(&(true, i)).copied()
    }

    /// Where the row added for source row `i` is.
    pub(super) fn at_added(&self, i: usize) -> usize {
        self.added(i).unwrap_or_default()
    }
}
