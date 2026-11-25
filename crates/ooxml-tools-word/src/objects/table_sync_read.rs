//! Owns: what a `sync` reads of the table and the source before it changes anything
//! (docx-agent's `table_sync.py`: `_mapping`, `_key_column`, `_body`, `_table_keys`,
//! `_source_keys`): the table's cells with their columns as accepted, the columns taken
//! (`from_columns`, else the header labels that name source columns), the key column, the
//! body rows (below the header rows, above the total rows: the look's last row and rows
//! holding a formula field), the table's keys and the source's -- duplicates refused naming
//! them.
//!
//! Must not: change the document.

use super::table_sync::{norm, Cell, Grid, Plan, Taken};
use super::table_view::{read_look, AcceptedView, TableView};
use crate::sources::SourceRows;
use ooxml_base::pyfmt::repr_str;
use ooxml_base::pystr::str_is_digit;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use ooxml_word_model::docx::tables::TableGrid;
use ooxml_word_model::docx::{at_path, casefold, find_path, Docx};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

/// The table's cells as sync reads them (`_accepted_grid` of `Table.grid`): rows as the
/// document has them, columns as accepted.
pub(super) fn sync_grid(docx: &Docx, view: &TableView, accepted: &AcceptedView) -> Grid {
    let shape = &accepted.table;
    let Some(part) = docx.part(&view.part) else {
        return Vec::new();
    };
    let cells = TableGrid::new(part, &view.table);
    let columns: Vec<usize> = if shape.active() {
        shape.kept.clone()
    } else {
        (0..shape.width()).collect()
    };
    (0..view.grid.row_count())
        .map(|r| {
            columns
                .iter()
                .map(|c| {
                    let at = if shape.active() {
                        shape.given.get(r, *c).unwrap_or(*c)
                    } else {
                        *c
                    };
                    let (row, column) = view.origin(r, at)?;
                    Some(Cell {
                        row,
                        column: if shape.active() {
                            shape.from_grid(column)
                        } else {
                            column
                        },
                        text: cells.cell_text(r, at).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .collect()
}

/// Row `row`'s text per column, a merged cell's at its first column only (`_labels`).
pub(super) fn labels(grid: &Grid, row: usize, width: usize) -> Vec<String> {
    (0..width)
        .map(
            |col| match grid.get(row).and_then(|line| line.get(col)).and_then(Option::as_ref) {
                Some(cell) if cell.column == col => norm(&cell.text),
                _ => String::new(),
            },
        )
        .collect()
}

/// A refusal at `field` with `options` its way out (else the call again).
pub(super) fn refusal(code: Refusal, message: String, field: &str, options: Vec<String>, retry: &TryCall) -> ToolError {
    ToolError::refusal(
        code,
        message,
        WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone())),
    )
    .at(field)
}

/// The source columns taken, in pick order; `plan.pairs` each table column with its pick
/// (`_mapping`): `from_columns` where it names one, else the header labels that name source
/// columns.
pub(super) fn mapping(
    rows: &SourceRows,
    grid: &Grid,
    (header, width): (usize, usize),
    from_columns: Option<&[String]>,
    table: &str,
    plan: &mut Plan,
    retry: &TryCall,
) -> Result<Vec<usize>, ToolError> {
    if let Some(given) = from_columns.filter(|g| g.iter().any(|n| !n.is_empty())) {
        plan.by_columns = true;
        return named(rows, given, (width, table), plan, retry);
    }
    let columns = &rows.columns;
    let first: Vec<String> = columns.iter().take(50).cloned().collect();
    let mut fields: Vec<usize> = Vec::new();
    let labels = labels(grid, header.saturating_sub(1), width);
    for (col, label) in labels.iter().enumerate() {
        if label.is_empty() {
            continue;
        }
        let mut hits: Vec<usize> = (0..columns.len())
            .filter(|k| columns.get(*k).is_some_and(|c| norm(c) == *label))
            .collect();
        if hits.is_empty() {
            hits = (0..columns.len())
                .filter(|k| columns.get(*k).is_some_and(|c| casefold(&norm(c)) == casefold(label)))
                .collect();
        }
        let &[hit] = hits.as_slice() else { continue };
        if fields.contains(&hit) {
            return Err(refusal(
                Refusal::Ambiguous,
                format!(
                    "two columns of {table} have the label {}: say which takes {} with from_columns (one source \
                     column per table column, '' for one left as it is)",
                    repr_str(label),
                    repr_str(columns.get(hit).map_or("", String::as_str))
                ),
                "from_columns",
                first,
                retry,
            ));
        }
        plan.pairs.push((col, fields.len()));
        fields.push(hit);
    }
    if fields.is_empty() {
        return Err(no_labels(rows, &labels, (width, table), retry));
    }
    Ok(fields)
}

/// The columns `from_columns` names, one source column per table column from the first,
/// `""` for one left as it is.
fn named(
    rows: &SourceRows,
    given: &[String],
    (width, table): (usize, &str),
    plan: &mut Plan,
    retry: &TryCall,
) -> Result<Vec<usize>, ToolError> {
    if given.len() > width {
        return Err(refusal(
            Refusal::InvalidArguments,
            format!(
                "from_columns names {} columns; {table} has {width}: one source column per table column, from the \
                 first, '' for one left as it is",
                given.len()
            ),
            "from_columns",
            rows.columns.iter().take(50).cloned().collect(),
            retry,
        ));
    }
    let mut fields: Vec<usize> = Vec::new();
    for (col, name) in given.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        let found = rows.column(name, &format!("from_columns[{col}]"), retry)?;
        if fields.contains(&found) {
            return Err(refusal(
                Refusal::InvalidArguments,
                format!(
                    "from_columns names {} twice: one table column per source column",
                    repr_str(name)
                ),
                &format!("from_columns[{col}]"),
                Vec::new(),
                retry,
            ));
        }
        plan.pairs.push((col, fields.len()));
        fields.push(found);
    }
    Ok(fields)
}

/// No header label names a source column: the refusal, with the `from_columns` that would
/// take the source's columns in order.
fn no_labels(rows: &SourceRows, labels: &[String], (width, table): (usize, &str), retry: &TryCall) -> ToolError {
    let columns = &rows.columns;
    let example: Vec<String> = columns.iter().take(width).cloned().collect();
    let shown: Vec<String> = labels.iter().filter(|t| !t.is_empty()).map(|t| repr_str(t)).collect();
    let named: Vec<String> = columns.iter().take(20).map(|c| repr_str(c)).collect();
    let example_said = format!(
        "[{}]",
        example.iter().map(|e| repr_str(e)).collect::<Vec<_>>().join(", ")
    );
    refusal(
        Refusal::NotFound,
        format!(
            "no header label of {table} ({}) is a column of {} ({}): give from_columns, one source column per table \
             column, e.g. {example_said}",
            if shown.is_empty() {
                "none".to_owned()
            } else {
                shown.join(", ")
            },
            rows.address,
            named.join(", ")
        ),
        "from_columns",
        columns.iter().take(50).cloned().collect(),
        retry,
    )
    .with_detail("from_columns", json!(example))
}

/// The key column (`_key_column`): `key` as a label of a column taken, its index, or a
/// source column's name; default the first column, else the first taken.
pub(super) fn key_column(
    plan: &mut Plan,
    key: Option<&str>,
    grid: &Grid,
    header: usize,
    taken: &Taken<'_>,
    retry: &TryCall,
) -> Result<(), ToolError> {
    let mapped: BTreeMap<usize, usize> = plan.pairs.iter().copied().collect();
    let width = grid.first().map_or(0, Vec::len);
    let labels = labels(grid, header.saturating_sub(1), width);
    let label = |col: usize| labels.get(col).cloned().unwrap_or_default();
    let col = match key.filter(|k| !k.is_empty()) {
        None => {
            if mapped.contains_key(&0) {
                Some(0)
            } else {
                plan.pairs.first().map(|p| p.0)
            }
        }
        Some(key) => {
            let said = norm(key);
            let hits: Vec<usize> = mapped
                .keys()
                .copied()
                .filter(|c| casefold(&label(*c)) == casefold(&said))
                .collect();
            if hits.len() == 1 {
                hits.first().copied()
            } else if str_is_digit(&said) {
                said.parse::<usize>().ok()
            } else {
                let hits: Vec<usize> = mapped
                    .iter()
                    .filter(|(_, pick)| casefold(&taken.label(**pick)) == casefold(&said))
                    .map(|(c, _)| *c)
                    .collect();
                hits.first().copied().filter(|_| hits.len() == 1)
            }
            .filter(|c| mapped.contains_key(c))
            .map_or_else(
                || {
                    let named: Vec<String> = plan
                        .pairs
                        .iter()
                        .map(|(c, pick)| {
                            let shown = label(*c);
                            format!("{c}: {}", if shown.is_empty() { taken.label(*pick) } else { shown })
                        })
                        .collect();
                    Err(refusal(
                        Refusal::NotFound,
                        format!(
                            "key {} is not a column sync takes from the source; the columns taken: {}",
                            repr_str(key),
                            named.join(", ")
                        ),
                        "key",
                        key_options(plan, &labels),
                        retry,
                    ))
                },
                |c| Ok(Some(c)),
            )?
        }
    };
    let col = col.unwrap_or(0);
    plan.key_col = col;
    plan.key_pick = mapped.get(&col).copied().unwrap_or(0);
    Ok(())
}

/// What `key` takes for each column sync takes (`_key_options`): the column's label when no
/// other column taken has it (case aside), else the column's index.
fn key_options(plan: &Plan, labels: &[String]) -> Vec<String> {
    let mapped: Vec<usize> = plan.pairs.iter().map(|p| p.0).collect();
    mapped
        .iter()
        .map(|col| {
            let label = labels.get(*col).cloned().unwrap_or_default();
            let alone = !label.is_empty()
                && !str_is_digit(&label)
                && mapped
                    .iter()
                    .filter(|c| labels.get(**c).is_some_and(|l| casefold(l) == casefold(&label)))
                    .count()
                    == 1;
            if alone {
                label
            } else {
                col.to_string()
            }
        })
        .collect()
}

/// The body's rows (`_body`): below the header rows (or a first row of labels), above a
/// total row.
pub(super) fn body(
    view: &TableView,
    plan: &mut Plan,
    grid: &Grid,
    (header, height): (usize, usize),
    taken: &Taken<'_>,
) {
    let mut body = header;
    if header == 0 && height > 0 {
        let first = grid
            .first()
            .and_then(|line| line.get(plan.key_col))
            .and_then(Option::as_ref);
        let name = casefold(&norm(&taken.label(plan.key_pick)));
        if !plan.by_columns || first.is_some_and(|c| casefold(&norm(&c.text)) == name) {
            body = 1;
        }
    }
    let mut end = height;
    let look = read_look(find_path(&view.table.element, &["tblPr", "tblLook"]));
    if look[3] && height.saturating_sub(1) > body {
        end = height - 1;
    }
    // a row holding a formula field (=SUM(ABOVE), any = field) computes over the table: a
    // total row, never matched, added to or removed; those at the bottom end the body
    let table = &view.table;
    plan.totals = (body..end)
        .filter(|r| {
            table
                .rows
                .get(*r)
                .and_then(|(path, _)| at_path(&table.element, path.get(table.path.len()..).unwrap_or_default()))
                .is_some_and(|row| formula_row(row))
        })
        .collect();
    while end > body + 1 && plan.totals.contains(&(end - 1)) {
        end -= 1;
    }
    plan.body = body;
    plan.end = end;
}

/// Whether a row holds a formula field (`_formula_row`): a `w:fldSimple` or a field's
/// instruction text starting with `=`.
fn formula_row(element: &Element) -> bool {
    element.child_elements().any(|child| {
        let said = if child.name().is(Some(W), "fldSimple") {
            child.attribute(Some(W), "instr").map(str::to_owned)
        } else if child.name().is(Some(W), "instrText") {
            Some(child.text_content())
        } else {
            None
        };
        said.is_some_and(|s| s.trim_start_matches(crate::written::py_space).starts_with('=')) || formula_row(child)
    })
}

/// The table's keys (`_table_keys`): duplicates refused naming them.
pub(super) fn table_keys(
    plan: &mut Plan,
    grid: &Grid,
    deleted: &[usize],
    table: &str,
    retry: &TryCall,
) -> Result<(), ToolError> {
    let mut duplicate: Vec<(String, Vec<usize>)> = Vec::new();
    for row in plan.body..plan.end {
        if deleted.contains(&row) || plan.totals.contains(&row) {
            continue;
        }
        let text = match grid.get(row).and_then(|l| l.get(plan.key_col)).and_then(Option::as_ref) {
            Some(cell) if cell.row == row => norm(&cell.text),
            _ => String::new(),
        };
        if text.is_empty() {
            plan.keyless.push(row);
            continue;
        }
        if let Some(first) = plan.by_key.get(&text) {
            match duplicate.iter_mut().find(|(k, _)| *k == text) {
                Some((_, rows)) => rows.push(row),
                None => duplicate.push((text, vec![*first, row])),
            }
            continue;
        }
        plan.by_key.insert(text.clone(), row);
        plan.order.push(text);
    }
    if duplicate.is_empty() {
        return Ok(());
    }
    // rows as positions count them (a pending deleted row not counted)
    let accepted: Vec<usize> = (0..grid.len()).filter(|r| !deleted.contains(r)).collect();
    let place = |r: usize| accepted.iter().position(|a| *a == r).unwrap_or(r);
    let shown: Vec<String> = duplicate
        .iter()
        .take(10)
        .map(|(k, rows)| {
            format!(
                "{} in rows {}",
                repr_str(k),
                rows.iter()
                    .map(|r| place(*r).to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    let more = if duplicate.len() > 10 {
        format!(" and {} more", duplicate.len() - 10)
    } else {
        String::new()
    };
    Err(refusal(
        Refusal::Refused,
        format!(
            "{table} has duplicate keys in column {}: {}{more}: a key names one row; make them unique (set_cells) or \
             delete the extra rows (delete_row), or sync by another key column",
            plan.key_col,
            shown.join("; ")
        ),
        "key",
        duplicate.into_iter().take(50).map(|(k, _)| k).collect(),
        retry,
    ))
}

/// Each source row's key (`None` for a blank one); duplicates refused naming them
/// (`_source_keys`).
pub(super) fn source_keys(
    texts: &[Vec<String>],
    pick: usize,
    address: &str,
    retry: &TryCall,
) -> Result<Vec<Option<String>>, ToolError> {
    let mut out = Vec::with_capacity(texts.len());
    let mut seen: Vec<(String, Vec<usize>)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (i, line) in texts.iter().enumerate() {
        let found = norm(&line.get(pick).map_or_else(String::new, |t| t.replace('\u{b}', " ")));
        if !found.is_empty() {
            match at.get(&found) {
                Some(k) => seen.get_mut(*k).into_iter().for_each(|(_, rows)| rows.push(i)),
                None => {
                    at.insert(found.clone(), seen.len());
                    seen.push((found.clone(), vec![i]));
                }
            }
        }
        out.push((!found.is_empty()).then_some(found));
    }
    let duplicate: Vec<&(String, Vec<usize>)> = seen.iter().filter(|(_, v)| v.len() > 1).collect();
    if duplicate.is_empty() {
        return Ok(out);
    }
    let shown: Vec<String> = duplicate
        .iter()
        .take(10)
        .map(|(k, rows)| {
            format!(
                "{} in rows {}",
                repr_str(k),
                rows.iter()
                    .map(|r| format!("#{}", r + 1))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    let more = if duplicate.len() > 10 {
        format!(" and {} more", duplicate.len() - 10)
    } else {
        String::new()
    };
    Err(refusal(
        Refusal::Refused,
        format!(
            "{address} has duplicate keys: {}{more}: a key names one row; derive a dataset with one row per key, or \
             sync by another key column",
            shown.join("; ")
        ),
        "from",
        duplicate.iter().take(50).map(|(k, _)| k.clone()).collect(),
        retry,
    ))
}
