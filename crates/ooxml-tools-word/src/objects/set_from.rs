//! Owns: `word_edit_table`'s `set_cells` from a source (docx-agent's `tools/objects.py`,
//! `_set_from`): the block ([`crate::sources::placed`]) written from row `index` (default 0),
//! column 0, as the table is accepted; a block longer than the rows below refused with the
//! call that grows the table (`_no_room`), or with `grow` the rows it needs added after the
//! last, each like it (a header row's likeness a body row); a block over a merged cell or a
//! place with no cell refused before anything changes, with the calls that leave the row as
//! it is (`_one_cell_each`); the table's record of the source kept; and the reply: the table
//! as `table_json` gives it with `data.from` (where the values came from, per column) and
//! where the time went, the cells the block left alone said.
//!
//! Must not: change the document but through one command ([`PlaceRows`]), or keep a record
//! of a table it did not write.

use super::edit_table::table_json;
use super::table_view::{AcceptedView, TableView};
use crate::review::Editing;
use crate::sources::{first_rows, fits, placed, thousands, Placed, SourceRows};
use crate::structure::common::{outcome, policy, refused, Outcome};
use ooxml_tools::{ArgEnum, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{CellAxis, CellWrite, EditError, PlaceRows, RowsPlaced, WordDoc};
use ooxml_word_model::docx::tables::Accepted;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::Instant;

/// Rows a table from data takes at most, the header included (Word's).
const MAX_ROWS: usize = 4096;
/// Columns a table takes at most (Word's).
const MAX_COLUMNS: usize = 63;

/// A grid as accepted: per row, per column, the origin of the cell there (row, column, both
/// as accepted).
type Grid = Vec<Vec<Option<(usize, usize)>>>;

/// Where a call's time went, part by part (docx-agent's `Timing`): seconds each, to the
/// millisecond.
#[derive(Default)]
pub(super) struct Timing {
    parts: Map<String, Value>,
}

impl Timing {
    /// The time since `since` added to the part `name`.
    pub(super) fn add(&mut self, name: &str, since: Instant) {
        self.seconds(name, since.elapsed().as_secs_f64());
    }

    /// `seconds` added to the part `name`.
    pub(super) fn seconds(&mut self, name: &str, seconds: f64) {
        let before = self.parts.get(name).and_then(Value::as_f64).unwrap_or(0.0);
        self.parts
            .insert(name.to_owned(), json!(((before + seconds) * 1000.0).round() / 1000.0));
    }

    /// The parts, as a reply's `data.timing`.
    pub(super) fn json(&self) -> Value {
        Value::Object(self.parts.clone())
    }
}

/// What the call asked, as a refusal's calls repeat it.
fn asked(args: &super::edit_table_args::WordEditTable) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("doc".into(), json!(args.doc));
    out.insert("table".into(), json!(args.table));
    out.insert("action".into(), json!(args.action.as_str()));
    out.insert("from".into(), json!(args.from));
    if let Some(columns) = &args.from_columns {
        out.insert("from_columns".into(), json!(columns));
    }
    if let Some(header) = &args.header {
        out.insert("header".into(), json!(header));
    }
    if let Some(format) = &args.date_format {
        out.insert("date_format".into(), json!(format));
    }
    if let Some(index) = args.index {
        out.insert("index".into(), json!(index));
    }
    out
}

/// `set_cells` from `rows` into the table `view` (found on the version before).
pub(super) fn set_from(
    args: &super::edit_table_args::WordEditTable,
    editing: &Editing<'_>,
    view: &TableView,
    accepted: &AcceptedView,
    rows: &SourceRows,
    retry: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let mut timing = Timing::default();
    let started = Instant::now();
    let locale = editing.locale.cloned().unwrap_or_default();
    let block = placed(
        rows,
        args.from_columns.as_deref(),
        args.header.as_deref(),
        args.date_format.as_deref(),
        &locale,
        retry,
    )?;
    timing.add("resolve", started);
    let started = Instant::now();
    let plan = plan(args, view, &accepted.table, &block, retry)?;
    let lines = block.lines(retry)?;
    timing.add("check", started);
    let placed_from = block.provenance(plan.top);
    let command = PlaceRows {
        table: args.table.clone(),
        grow: plan.short_by,
        writes: writes(&lines, &accepted.table, &plan),
        record: record(&placed_from, rows.name.as_deref()),
        policy: policy(editing),
    };
    let look = |out: &RowsPlaced, _: &WordDoc| {
        Some((
            out.table.clone(),
            out.added.iter().filter_map(|r| r.id.clone()).collect::<Vec<_>>(),
        ))
    };
    let table = &view.table.id;
    let (done, rows_said) = match super::rows_written::run_writing_rows(editing, &command, table, look, retry) {
        Ok(done) => done,
        Err(error @ (EditError::Address(_) | EditError::Argument { .. } | EditError::Position(_))) => {
            return Ok(ToolResult::failed(crate::review::edit_refusal(
                error,
                "",
                retry.clone(),
            )));
        }
        Err(error) => return Ok(refused(error, retry.clone())),
    };
    for (name, seconds) in &done.parts {
        timing.seconds(name, *seconds);
    }
    let started = Instant::now();
    let mut result = reply(editing, (table, rows), &plan, done, placed_from, retry);
    notes(&mut result, &accepted.table, &plan);
    result.warnings.extend(rows_said);
    timing.add("result", started);
    if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
        if let Some(Value::Object(map)) = &mut found.data {
            map.insert("timing".into(), timing.json());
        }
    }
    Ok(result)
}

/// Where the block goes and how far the table falls short.
struct Plan {
    top: usize,
    height: usize,
    width: usize,
    need_rows: usize,
    need_columns: usize,
    short_by: usize,
}

/// The block checked against the table as accepted (`_set_from`'s checks, before anything
/// changes): its first row, the rows it lacks; refused where it does not fit.
fn plan(
    args: &super::edit_table_args::WordEditTable,
    view: &TableView,
    shape: &Accepted,
    block: &Placed<'_>,
    retry: &TryCall,
) -> Result<Plan, ToolError> {
    let table = &view.table.id;
    let (height, width) = (shape.height(), shape.width());
    let top = usize::try_from(args.index.unwrap_or(0)).unwrap_or(usize::MAX);
    let grow = args.grow == Some(true);
    if top > height || (top == height && !grow) {
        let at_end = top == height;
        let way_out = if at_end { WayOut::names(["grow"]) } else { None };
        return Err(ToolError::refusal(
            Refusal::NotFound,
            format!(
                "no row {top}; {table} has {height}{}",
                if at_end {
                    format!(" (with grow, index {height} adds the rows after the last)")
                } else {
                    String::new()
                }
            ),
            way_out.unwrap_or_else(|| WayOut::call(retry.clone())),
        )
        .at("index"));
    }
    let (need_rows, need_columns) = block.size();
    let room = height - top;
    let short_by = need_rows.saturating_sub(room);
    let grid = accepted_grid(view, shape);
    let asked = asked(args);
    let checks = Checks {
        table,
        block,
        asked: &asked,
        retry,
    };
    if short_by > 0 && !grow {
        if need_columns <= width {
            checks.one_cell_each(&grid, (top, room, need_columns), None, Some(need_rows))?;
        }
        return Err(checks.no_room(room, top, short_by));
    }
    if grow {
        let columns_said = format!("{table} has {width} columns");
        fits(block, need_rows, width, Some(&columns_said), retry)?;
        fits(block, MAX_ROWS.saturating_sub(top), MAX_COLUMNS, None, retry)?;
    } else {
        let room_said = format!("{table} has {room} rows from row {top}");
        fits(block, room, width, Some(&room_said), retry)?;
    }
    checks.one_cell_each(&grid, (top, need_rows.min(room), need_columns), None, Some(need_rows))?;
    if short_by > 0 {
        let like = grown_like(view, shape, &grid);
        checks.one_cell_each(&vec![like; short_by], (0, short_by, need_columns), Some(height), None)?;
    }
    Ok(Plan {
        top,
        height,
        width,
        need_rows,
        need_columns,
        short_by,
    })
}

/// The block's cells by document row and grid column: rows as accepted, the rows grown the
/// last ones after the document's; columns as accepted.
fn writes(lines: &[Vec<Vec<String>>], shape: &Accepted, plan: &Plan) -> Vec<CellWrite> {
    let document_row = |row: usize| -> usize {
        if !shape.rows_active() {
            row
        } else if row < plan.height {
            shape.rows.get(row).copied().unwrap_or(row)
        } else {
            shape.total_rows + (row - plan.height)
        }
    };
    let grid_column = |col: usize| {
        if shape.active() {
            shape.kept.get(col).copied().unwrap_or(col)
        } else {
            col
        }
    };
    lines
        .iter()
        .enumerate()
        .flat_map(|(k, line)| {
            line.iter().enumerate().map(move |(col, cell)| CellWrite {
                row: CellAxis::At(document_row(plan.top + k)),
                column: CellAxis::At(grid_column(col)),
                text: cell.join("\u{b}"),
            })
        })
        .collect()
}

/// The reply (`_set_from`'s): the table as `table_json` gives it with `data.from`, the
/// edits' ids, the table and its cells changed.
fn reply(
    editing: &Editing<'_>,
    (table, rows): (&str, &SourceRows),
    plan: &Plan,
    done: RowsPlaced,
    placed_from: Map<String, Value>,
    retry: &TryCall,
) -> ToolResult<Value> {
    let head = editing.editor.store().snapshot();
    let mut data = head
        .doc()
        .docx()
        .ok()
        .and_then(|docx| table_json(&docx, &done.table, head.doc(), false, retry))
        .unwrap_or_else(|| json!({}));
    if let Value::Object(map) = &mut data {
        map.insert("from".into(), Value::Object(placed_from));
        map.insert("timing".into(), Value::Null);
    }
    let added = done.added.len();
    // the table as named after the edit (its durable id where the edit stamped the document)
    let now = done.table.clone();
    let summary = format!(
        "set_cells on {now} from {}{}",
        rows.address,
        if added > 0 {
            format!(
                "; {added} row(s) added after row {}, each like it",
                i64::try_from(plan.height).unwrap_or(0) - 1
            )
        } else {
            String::new()
        }
    );
    let mut edits: Vec<Outcome> = done.added.into_iter().map(Outcome::from).collect();
    edits.extend(done.cells.edits.into_iter().map(Outcome::from));
    let mut result = outcome(&edits, summary, Some(data));
    if let ooxml_tools::Outcome::Done(found) = &mut result.outcome {
        let cells = done.cells.ids.iter().map(|id| {
            id.rsplit_once("/c")
                .map_or_else(|| id.clone(), |(_, place)| format!("{now}/c{place}"))
        });
        // the table as the rows grown left it (docx-agent's `changed`, the table's id read
        // after them: the call's own where none were), then its cells as the edit left them
        let grown = if added > 0 { now.clone() } else { table.to_owned() };
        for id in std::iter::once(grown).chain(cells) {
            if !found.changed.contains(&id) {
                found.changed.push(id);
            }
        }
    }
    result
}

/// What the block left alone, and where its columns went in a table whose columns'
/// deletion is pending.
fn notes(result: &mut ToolResult<Value>, shape: &Accepted, plan: &Plan) {
    let (top, height, width) = (plan.top, plan.height, plan.width);
    let (need_rows, need_columns) = (plan.need_rows, plan.need_columns);
    let below = height.saturating_sub(top + need_rows);
    if below > 0 || need_columns < width {
        let mut line = format!(
            "from filled rows {top}-{}, columns 0-{} of a {height}x{width} table; the other cells keep their text",
            i64::try_from(top + need_rows).unwrap_or(0) - 1,
            i64::try_from(need_columns).unwrap_or(0) - 1
        );
        if below > 0 {
            line.push_str(&format!(
                " (rows {}-{} below the data: delete_row index {}, {below} time(s), removes them)",
                top + need_rows,
                height - 1,
                top + need_rows
            ));
        }
        result.warnings.push(line);
    }
    if shape.active() && need_columns > 0 && shape.kept.get(need_columns - 1) != Some(&(need_columns - 1)) {
        result.warnings.push(format!(
            "{}: the block's columns 0 to {} are written in those",
            shape.shape(),
            need_columns - 1
        ));
    }
}

/// The record a document keeps of a placement (ooxml-edit's `provenance.record`): `data.from`
/// without the session's document handle, its source without one, and the name of the
/// document the values came from where there is one; keys whose value is `null` left out.
pub(super) fn record(placed: &Map<String, Value>, document: Option<&str>) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("v".into(), json!(1));
    let source = placed.get("source").and_then(Value::as_str).unwrap_or_default();
    let unqualified = source
        .split_once('/')
        .filter(|(d, _)| d.len() > 1 && d.starts_with('d') && d[1..].bytes().all(|c| c.is_ascii_digit()))
        .map_or(source, |(_, rest)| rest);
    out.insert("source".into(), json!(unqualified));
    if let Some(name) = document.filter(|n| !n.is_empty()) {
        out.insert("document".into(), json!(name));
    }
    for (key, value) in placed {
        if matches!(key.as_str(), "source" | "document" | "v" | "doc") || value.is_null() {
            continue;
        }
        out.insert(key.clone(), value.clone());
    }
    out
}

/// The table's grid as accepted (`_accepted_grid`, `_positioned`): a column a pending
/// tracked deletion removes left out, a position a pending tracked merge takes in answering
/// as the merged cell, a row whose deletion is pending not counted.
fn accepted_grid(view: &TableView, shape: &Accepted) -> Grid {
    let rows: Vec<usize> = if shape.rows_active() {
        shape.rows.clone()
    } else {
        (0..view.grid.len()).collect()
    };
    let columns: Vec<usize> = if shape.active() {
        shape.kept.clone()
    } else {
        (0..shape.columns.max(view.grid.iter().map(Vec::len).max().unwrap_or(0))).collect()
    };
    rows.iter()
        .map(|r| {
            columns
                .iter()
                .map(|c| {
                    let at = if shape.active() {
                        shape.given.get(&(*r, *c)).copied().unwrap_or(*c)
                    } else {
                        *c
                    };
                    view.origin(*r, at).map(|(row, column)| {
                        (
                            if shape.rows_active() { shape.from_row(row) } else { row },
                            if shape.active() {
                                shape.from_grid(column)
                            } else {
                                column
                            },
                        )
                    })
                })
                .collect()
        })
        .collect()
}

/// The grid row the rows `grow` adds are like (`_grown_like`): the last row, or where a
/// cell of it spans columns the nearest row without one (a row whose deletion is pending
/// passed over), else a cell per column.
fn grown_like(view: &TableView, shape: &Accepted, grid: &Grid) -> Vec<Option<(usize, usize)>> {
    let last = grid.last().cloned().unwrap_or_default();
    let count = view.table.rows.len();
    let spanned = |row: usize| view.row_cells(row).iter().any(|(_, span)| *span > 1);
    if count == 0 || !spanned(count - 1) {
        return last;
    }
    let deleted = super::edit_table::deleted_rows(&view.table);
    let at = count - 1;
    let found = (1..count).find_map(|distance| {
        [at.checked_sub(distance), at.checked_add(distance)]
            .into_iter()
            .flatten()
            .find(|k| *k < count && !deleted.contains(k) && !spanned(*k))
    });
    match found {
        None => (0..last.len()).map(|c| Some((0, c))).collect(),
        Some(row) if shape.rows_active() => shape
            .rows
            .iter()
            .position(|r| *r == row)
            .and_then(|k| grid.get(k).cloned())
            .unwrap_or(last),
        Some(row) => grid.get(row).cloned().unwrap_or(last),
    }
}

/// The refusals of a block that does not fit its table.
struct Checks<'c> {
    table: &'c str,
    block: &'c Placed<'c>,
    asked: &'c Map<String, Value>,
    retry: &'c TryCall,
}

impl Checks<'_> {
    /// Refuse a block over a merged cell, or where a row has no cell (`_one_cell_each`):
    /// `count` rows of `grid` from `top`, `columns` wide; `first` names them from there when
    /// they are rows to add. Over a merged cell of a row the table has, the way out that
    /// leaves the row as it is comes first (`need_rows`: the block's rows).
    fn one_cell_each(
        &self,
        grid: &Grid,
        (top, count, columns): (usize, usize, usize),
        first: Option<usize>,
        need_rows: Option<usize>,
    ) -> Result<(), ToolError> {
        let mut seen: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
        for k in 0..count {
            let row = top + k;
            let line = grid.get(row).map_or(&[][..], Vec::as_slice);
            for col in 0..columns {
                let cell = line.get(col).copied().flatten();
                let origin = cell.map(|(r, c)| (if first.is_some() { row } else { r }, c));
                let merged = origin.and_then(|o| seen.get(&o).copied());
                if origin.is_some() && merged.is_none() {
                    if let Some(origin) = origin {
                        seen.insert(origin, (row, col));
                    }
                    continue;
                }
                let named = first.map_or(row, |f| f + k);
                let what = merged.map_or_else(|| "not a cell".to_owned(), |(r, c)| format!("merged with ({r}, {c})"));
                let mut message = format!(
                    "({named}, {col}) of {} is {what}{}: the block from {} needs one cell per value",
                    self.table,
                    if first.is_some() {
                        " in the rows grow adds (each like the last row)"
                    } else {
                        ""
                    },
                    self.block.rows.address
                );
                let Some(need_rows) = need_rows.filter(|_| first.is_none()) else {
                    message.push_str("; split it first");
                    return Err(
                        ToolError::refusal(Refusal::Refused, message, WayOut::call(self.retry.clone())).at("from"),
                    );
                };
                let times = need_rows - k;
                let mut insert = Map::new();
                if let Some(doc) = self.asked.get("doc") {
                    insert.insert("doc".into(), doc.clone());
                }
                insert.insert("table".into(), json!(self.table));
                insert.insert("action".into(), json!("insert_row"));
                insert.insert("index".into(), json!(row));
                insert.insert("side".into(), json!("before"));
                let mut again = self.asked.clone();
                again.remove("grow");
                let mut calls = vec![
                    json!({"tool": "word_edit_table", "arguments": insert, "times": times}),
                    json!({"tool": "word_edit_table", "arguments": again}),
                ];
                message.push_str(&format!(
                    ". To keep row {row} as it is, insert the rows above it: insert_row index {row} side before, {times} \
                     time(s) (each new row a cell per column), then this call again (index {top}, without grow)"
                ));
                if let Some(lead) = merged {
                    // the positions of the row the one cell takes
                    let span = line.iter().filter(|c| **c == cell).count();
                    let mut split = Map::new();
                    if let Some(doc) = self.asked.get("doc") {
                        split.insert("doc".into(), doc.clone());
                    }
                    split.insert("table".into(), json!(self.table));
                    split.insert("action".into(), json!("split"));
                    split.insert("first".into(), json!({"row": lead.0, "col": lead.1}));
                    split.insert("columns".into(), json!(span));
                    calls.push(json!({"tool": "word_edit_table", "arguments": split}));
                    message.push_str(&format!(
                        "; or split ({}, {}) into {span} columns, which changes row {row}",
                        lead.0, lead.1
                    ));
                }
                return Err(ToolError::refusal(
                    Refusal::Refused,
                    message,
                    WayOut::names(["insert_row", "split"]).unwrap_or_else(|| WayOut::call(self.retry.clone())),
                )
                .at("from")
                .with_detail("calls", Value::Array(calls)));
            }
        }
        Ok(())
    }

    /// The refusal of a block longer than the table's rows from `top` (`_no_room`): the call
    /// with `grow` that writes it, or a row range of the source that fits.
    fn no_room(&self, room: usize, top: usize, short_by: usize) -> ToolError {
        let header = !self.block.header.is_empty();
        let address = &self.block.rows.address;
        let fewer = (room > 0)
            .then(|| first_rows(address, room as i64 - i64::from(header)))
            .flatten();
        let mut meant = self.asked.clone();
        meant.insert("grow".into(), json!(true));
        let mut options = vec!["grow".to_owned()];
        options.extend(fewer.clone());
        ToolError::refusal(
            Refusal::Refused,
            format!(
                "{address} gives {} rows{}; {} has {room} rows from row {top}: give grow: true to add the {short_by} \
                 row(s) it needs, each like the last row{} (nothing is cut)",
                thousands(self.block.size().0),
                if header { " with the header" } else { "" },
                self.table,
                fewer
                    .as_ref()
                    .map_or_else(String::new, |f| format!(", or place fewer with {f}"))
            ),
            WayOut::names(options).unwrap_or_else(|| WayOut::call(self.retry.clone())),
        )
        .at("from")
        .with_detail("call", json!({"tool": "word_edit_table", "arguments": meant}))
    }
}
