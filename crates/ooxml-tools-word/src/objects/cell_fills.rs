//! Owns: `set_cells`' cells read as places (docx-agent's `tools/objects.py`,
//! `_column_rows`): an entry with `rows` is its column's cells over those rows, each given
//! the entry's one value (round 48, R89) -- a merged cell once, a row without a cell there
//! left out and said, `*` and an open start leaving the header rows out (`_header_rows`);
//! the range refused with the table's rows (rule 2); and what the reply says of each fill
//! (`_count_fills`, `_fill_brief`: the range and counts, rule 4).
//!
//! Rows and columns count the table as accepted ([`AcceptedView`]): a row whose deletion is
//! pending not counted, a cell Accept All removes left out and said.
//!
//! Must not: change the document.

use super::edit_table_args::CellArg;
use super::table_view::{read_look, AcceptedView, TableView};
use crate::refusal::address_refusal;
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use ooxml_word_model::docx::tables::{Axis, TableGrid};
use ooxml_word_model::docx::{at_path, find_path};
use serde_json::{json, Map, Value};

/// Rows a fill's note lists by number (the rest counted).
const LISTED: usize = 20;

/// A column filled with one value.
pub(super) struct Fill {
    /// The cells entry it comes from.
    pub(super) entry: usize,
    col: usize,
    first: usize,
    last: usize,
    header: usize,
    /// The text of each cell written, before.
    before: Vec<String>,
    missing: Vec<usize>,
    /// Said as warnings.
    pub(super) notes: Vec<String>,
    already: usize,
}

/// Each cell to write as a cells entry, the entry it comes from, and the fills.
pub(super) type Places = (Vec<CellArg>, Vec<usize>, Vec<Fill>);

/// `set_cells`' cells as places: each cell to write as a cells entry (a fill's as row and
/// column), the entry each comes from, and the fills.
pub(super) fn column_rows(
    view: &TableView,
    grid: &TableGrid<'_>,
    accepted: &mut AcceptedView,
    cells: &[CellArg],
    retry: &TryCall,
) -> Result<Places, ToolError> {
    let given = |cell: &CellArg| cell.rows.as_deref().is_some_and(|r| !r.is_empty());
    if !cells.iter().any(given) {
        return Ok((cells.to_vec(), (0..cells.len()).collect(), Vec::new()));
    }
    // rows as accepted: a row whose tracked deletion is pending not counted
    let rows: Vec<usize> = if accepted.table.rows_active() {
        accepted.table.rows.clone()
    } else {
        (0..view.grid.len()).collect()
    };
    let height = rows.len();
    let header = header_rows(view);
    let mut places = Vec::new();
    let mut owners = Vec::new();
    let mut fills = Vec::new();
    for (k, cell) in cells.iter().enumerate() {
        let Some(said) = cell.rows.as_deref().filter(|r| !r.is_empty()) else {
            places.push(cell.clone());
            owners.push(k);
            continue;
        };
        let col = fill_column(view, grid, accepted, cell, k, retry)?;
        let grid_col = if accepted.table.active() {
            let field = format!("cells[{k}].col");
            accepted.grid_column(i64::try_from(col).unwrap_or(i64::MAX), &field, retry)?
        } else {
            col
        };
        let (first, last, mut notes) = fill_range(&view.table.id, said, cell, k, (height, header))?;
        let (placed, written, missing) = column_cells(view, accepted, &rows, (first, last), grid_col, retry)?;
        for row in placed {
            places.push(at(row, col));
            owners.push(k);
        }
        if !missing.is_empty() {
            let shown: Vec<String> = missing.iter().take(LISTED).map(ToString::to_string).collect();
            let mut text = shown.join(", ");
            if missing.len() > LISTED {
                text.push_str(&format!(" and {} more", missing.len() - LISTED));
            }
            notes.push(format!(
                "cells[{k}]: row(s) {text} have no cell at column {col} (left as they are)"
            ));
        }
        if written.is_empty() {
            return Err(ToolError::refusal(
                Refusal::NotFound,
                format!("cells[{k}]: rows {first}..{last} have no cell at column {col}"),
                one_option(format!("{header}..{}", last_row(height)), retry),
            )
            .at(format!("cells[{k}].rows")));
        }
        fills.push(Fill {
            entry: k,
            col,
            first,
            last,
            header,
            before: written
                .iter()
                .map(|(r, c)| grid.cell_text(*r, *c).unwrap_or_default())
                .collect(),
            missing,
            notes,
            already: 0,
        });
    }
    Ok((places, owners, fills))
}

/// The rows of a fill (positions `first..=last`, as accepted) written in grid column
/// `grid_col`, each merged cell once: the positions written, their (document row, grid
/// column), and the positions with no cell there (or one Accept All removes).
type ColumnCells = (Vec<usize>, Vec<(usize, usize)>, Vec<usize>);

fn column_cells(
    view: &TableView,
    accepted: &mut AcceptedView,
    rows: &[usize],
    (first, last): (usize, usize),
    grid_col: usize,
    retry: &TryCall,
) -> Result<ColumnCells, ToolError> {
    let mut placed = Vec::new();
    let mut written = Vec::new();
    let mut missing = Vec::new();
    let mut seen = Vec::new();
    for row in first..=last {
        let doc_row = rows.get(row).copied().unwrap_or(row);
        if accepted.table.lost.contains(&(doc_row, grid_col)) {
            missing.push(row);
            continue;
        }
        let column = if accepted.table.active() {
            accepted.at(doc_row, grid_col, retry)?
        } else {
            grid_col
        };
        let Some(origin) = view.origin(doc_row, column) else {
            missing.push(row);
            continue;
        };
        if seen.contains(&origin) {
            continue; // merged with a row above it in the range
        }
        seen.push(origin);
        placed.push(row);
        written.push((doc_row, column));
    }
    Ok((placed, written, missing))
}

/// The cell at `(row, col)` as a cells entry.
fn at(row: usize, col: usize) -> CellArg {
    CellArg {
        row: Some(i64::try_from(row).unwrap_or(i64::MAX)),
        col: Some(i64::try_from(col).unwrap_or(i64::MAX)),
        row_label: None,
        col_label: None,
        text: None,
        from: None,
        rows: None,
    }
}

fn last_row(height: usize) -> i64 {
    i64::try_from(height).unwrap_or(i64::MAX) - 1
}

fn one_option(option: String, retry: &TryCall) -> WayOut {
    WayOut::names([option]).unwrap_or_else(|| WayOut::call(retry.clone()))
}

fn options(names: &[String], retry: &TryCall) -> WayOut {
    WayOut::names(names.iter().cloned()).unwrap_or_else(|| WayOut::call(retry.clone()))
}

/// The table's header rows, as a column's body leaves them out (`_header_rows`): the rows
/// marked to repeat as a header (`w:tblHeader`, leading), else the first row where the
/// table's look has its header row on.
pub(super) fn header_rows(view: &TableView) -> usize {
    let table = &view.table;
    let count = table
        .rows
        .iter()
        .take_while(|(path, _)| {
            at_path(&table.element, path.get(table.path.len()..).unwrap_or_default())
                .is_some_and(|row| find_path(row, &["trPr", "tblHeader"]).is_some())
        })
        .count();
    if count == 0 && read_look(find_path(&table.element, &["tblPr", "tblLook"]))[0] {
        return 1;
    }
    count
}

/// The column a fill writes (`_fill_column`): `col`, or `col_label` (the first row's
/// text), refused with the table's columns (rule 2); a position as the table is accepted.
fn fill_column(
    view: &TableView,
    grid: &TableGrid<'_>,
    accepted: &mut AcceptedView,
    cell: &CellArg,
    k: usize,
    retry: &TryCall,
) -> Result<usize, ToolError> {
    let label = cell.col_label.as_deref().filter(|l| !l.is_empty());
    if accepted.table.active() {
        match (cell.col, label) {
            (Some(col), None) => {
                accepted.grid_column(col, &format!("cells[{k}].col"), retry)?;
                return Ok(usize::try_from(col).unwrap_or(usize::MAX));
            }
            (_, Some(label)) => {
                let found = accepted
                    .table
                    .locate(grid, (Axis::At(0), Axis::Label(label)), &mut accepted.notes)
                    .map_err(|m| address_refusal(m.error, &m.field, retry.clone()))?;
                return Ok(accepted.table.from_grid(found.1));
            }
            (None, None) => {}
        }
    }
    let width = view.column_count();
    match (cell.col, label) {
        (Some(col), None) => usize::try_from(col).ok().filter(|c| *c < width).ok_or_else(|| {
            let columns: Vec<String> = (0..width.min(50)).map(|c| c.to_string()).collect();
            ToolError::refusal(
                Refusal::NotFound,
                format!(
                    "cells[{k}].col {col}: {} has grid columns 0 to {}",
                    view.table.id,
                    last_row(width)
                ),
                options(&columns, retry),
            )
            .at(format!("cells[{k}].col"))
        }),
        (None, None) => Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "cells[{k}] gives rows but no column: give col (from 0) or col_label (its header text), e.g. \
                 {{col: 2, rows: '*', text: 'Yes'}}"
            ),
            options(&["col".to_owned(), "col_label".to_owned()], retry),
        )
        .at(format!("cells[{k}].col"))),
        (_, Some(label)) => grid
            .column_labelled(0, label)
            .map_err(|e| crate::refusal::address_refusal(e, "", retry.clone())),
    }
}

/// A `rows` value read (`_ROWS`): `*` or `all`, else `a`, `a..b`, `a..`, `..b` (`-` and
/// `:` too) -- `(all, a, to, b)`; `None` where it is not rows.
fn parse_rows(said: &str) -> Option<(bool, Option<usize>, bool, Option<usize>)> {
    let text = said.trim();
    if text == "*" || text.eq_ignore_ascii_case("all") {
        return Some((true, None, false, None));
    }
    let digits = |s: &str| -> (Option<usize>, usize) {
        let n = s.bytes().take_while(u8::is_ascii_digit).count();
        let value = s
            .get(..n)
            .filter(|d| !d.is_empty())
            .map(|d| d.parse().unwrap_or(usize::MAX));
        (value, n)
    };
    let (a, used) = digits(text);
    let rest = text.get(used..)?.trim_start();
    let (to, rest) = ["..", "-", ":"]
        .iter()
        .find_map(|t| rest.strip_prefix(t).map(|r| (true, r)))
        .unwrap_or((false, rest));
    let rest = rest.trim_start();
    let (b, used) = digits(rest);
    if !rest.get(used..)?.trim().is_empty() || (a.is_none() && b.is_none()) {
        return None;
    }
    Some((false, a, to, b))
}

/// The table's body rows (`a..b`) and its rows as a refusal says them.
fn row_shape(table: &str, height: usize, header: usize) -> (String, String) {
    let end = last_row(height);
    let body = if header < height {
        format!("{header}..{end}")
    } else {
        format!("0..{end}")
    };
    let headers = if header > 0 {
        format!(" ({header} header row(s))")
    } else {
        String::new()
    };
    let shape = format!(
        "{table} has rows 0 to {end}{headers}: rows '*' (every row below the header), '{body}', or a range of them"
    );
    (body, shape)
}

/// The first and last row (both written) of a cells entry's `rows`, and the notes
/// (`_fill_range`).
fn fill_range(
    table: &str,
    said: &str,
    cell: &CellArg,
    k: usize,
    (height, header): (usize, usize),
) -> Result<(usize, usize, Vec<String>), ToolError> {
    let end = last_row(height);
    let (body, shape) = row_shape(table, height, header);
    let field = format!("cells[{k}].rows");
    let refuse = |code: Refusal, message: String, names: Vec<String>| {
        ToolError::refusal(
            code,
            message,
            WayOut::names(names).unwrap_or_else(|| WayOut::one(json!("*"))),
        )
        .at(&field)
    };
    let quoted = repr_str(said);
    if cell.row.is_some() || cell.row_label.as_deref().is_some_and(|l| !l.is_empty()) {
        let which = if cell.row.is_some() { "row" } else { "row_label" };
        return Err(refuse(
            Refusal::InvalidArguments,
            format!("cells[{k}] gives rows and {which}: rows names the rows; give one (for one cell, row and no rows)"),
            vec!["rows".into(), "row".into(), "row_label".into()],
        ));
    }
    let Some((all, a, to, b)) = parse_rows(said) else {
        return Err(refuse(
            Refusal::InvalidArguments,
            format!("cells[{k}].rows {quoted} is not rows: {shape}"),
            vec!["*".into(), body],
        ));
    };
    let mut notes = Vec::new();
    let (first, last): (i64, i64) = if all {
        (i64::try_from(header).unwrap_or(i64::MAX), end)
    } else {
        let wide = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        let (first, last) = if to {
            (a.map_or(wide(header), wide), b.map_or(end, wide))
        } else {
            let one = a.map_or(0, wide);
            (one, one)
        };
        if first > last {
            return Err(refuse(
                Refusal::InvalidArguments,
                format!("cells[{k}].rows {quoted} runs backwards: {shape}"),
                vec![format!("{last}..{first}"), "*".into()],
            ));
        }
        if last > end {
            let from = if first <= end { format!("{first}..{end}") } else { body };
            return Err(refuse(
                Refusal::NotFound,
                format!(
                    "cells[{k}].rows {quoted}: no row {last}; {shape}. Rows are not added: insert_row first, or \
                     set_cells from with grow"
                ),
                vec!["*".into(), from],
            ));
        }
        if a.is_some() && first < wide(header) {
            notes.push(format!(
                "cells[{k}]: rows {first}..{last} include header row(s) {first}..{}, written as asked ('*' leaves \
                 them out)",
                wide(header).min(last + 1) - 1
            ));
        }
        (first, last)
    };
    if first > last {
        return Err(refuse(
            Refusal::NotFound,
            format!(
                "cells[{k}].rows {quoted}: {table} has no rows below its {header} header row(s); name them to \
                 write them: rows '0..{end}'"
            ),
            vec![format!("0..{end}")],
        ));
    }
    let place = |n: i64| usize::try_from(n).unwrap_or(0);
    Ok((place(first), place(last), notes))
}

/// Each fill's cells written, and how many held its value already (`_count_fills`, rule 3).
pub(super) fn count_fills(fills: &mut [Fill], texts: &[String]) {
    for fill in fills {
        let text = texts.get(fill.entry).map(String::as_str).unwrap_or_default();
        fill.already = fill.before.iter().filter(|held| *held == text).count();
        if fill.already == fill.before.len() {
            fill.notes.push(format!(
                "cells[{}]: the {} cell(s) held that text already (nothing to change)",
                fill.entry,
                fill.before.len()
            ));
        }
    }
}

/// What the reply says of a fill: the range written and counts (`_fill_brief`, rule 4).
pub(super) fn fill_brief(fill: &Fill) -> Value {
    let mut out = Map::new();
    out.insert("entry".into(), json!(fill.entry));
    out.insert("col".into(), json!(fill.col));
    out.insert("rows".into(), json!(format!("{}..{}", fill.first, fill.last)));
    out.insert("cells".into(), json!(fill.before.len()));
    out.insert("changed".into(), json!(fill.before.len() - fill.already));
    if fill.header > 0 && fill.first >= fill.header {
        out.insert("header_rows".into(), json!(fill.header));
    }
    if !fill.missing.is_empty() {
        out.insert("no_cell".into(), json!(fill.missing.len()));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::parse_rows;

    #[test]
    fn rows_read_as_docx_agent_reads_them() {
        assert_eq!(parse_rows(" * "), Some((true, None, false, None)));
        assert_eq!(parse_rows("ALL"), Some((true, None, false, None)));
        assert_eq!(parse_rows("1..4"), Some((false, Some(1), true, Some(4))));
        assert_eq!(parse_rows("2 - 3"), Some((false, Some(2), true, Some(3))));
        assert_eq!(parse_rows("..3"), Some((false, None, true, Some(3))));
        assert_eq!(parse_rows("5:"), Some((false, Some(5), true, None)));
        assert_eq!(parse_rows("7"), Some((false, Some(7), false, None)));
        assert_eq!(parse_rows("1 2"), Some((false, Some(1), false, Some(2))));
        assert_eq!(parse_rows(".."), None);
        assert_eq!(parse_rows("x"), None);
        assert_eq!(parse_rows(""), None);
    }
}
