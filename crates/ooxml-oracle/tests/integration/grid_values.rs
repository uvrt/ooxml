//! The one table grid (`ooxml_word_model::docx::tables::Grid`) held to Word: where Word 16.106
//! drew each cell of the grid-values probe (`word/observations/grid-values-word.json`,
//! `word/probes/grid_values_probe.py`), the grid places it; where Word does not open a case,
//! the value that made it so is not a valid span or count.
//!
//! Must not: score anything but the grid's placement against Word.

// Test code: a recording that is not as read fails the test.
#![allow(clippy::panic, clippy::indexing_slicing, clippy::unwrap_used)]

use ooxml_word_model::docx::tables::{
    grid_value, own_rows, row_cells, CellSpec, Coverage, Grid, RowSpec, GRID_VALUE_MAX,
};
use serde_json::Value;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn observations() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/word/observations/grid-values-word.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The text of every `w:t` below `element`.
fn texts(element: &ooxml_xml::Element) -> String {
    element
        .child_elements()
        .map(|e| {
            if e.name().is(Some(W), "t") {
                ooxml_xml::view::text(e)
            } else {
                texts(e)
            }
        })
        .collect()
}

/// A case's table read by the grid: per row its specs, and the grid.
struct Read {
    /// Each cell: its text, first grid column and values.
    cells: Vec<(String, usize, CellSpec)>,
    rows: Vec<RowSpec>,
    grid: Grid,
}

/// The case's table as the grid reads it.
fn read(table_xml: &str) -> Read {
    let xml = table_xml.replacen("<w:tbl>", &format!(r#"<w:tbl xmlns:w="{W}">"#), 1);
    let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap();
    let rows = own_rows(tree.root());
    let elements: Vec<&ooxml_xml::Element> = rows.iter().map(|(r, _)| &**r).collect();
    let grid = Grid::of(tree.root(), &elements);
    let mut cells = Vec::new();
    for (row, line) in elements.iter().zip(&grid.rows) {
        for ((tc, _), cell) in row_cells(row).iter().zip(&line.cells) {
            cells.push((texts(tc), cell.first, CellSpec::of(tc)));
        }
    }
    let rows = elements.iter().map(|r| RowSpec::of(r)).collect();
    Read { cells, rows, grid }
}

/// Whether a span or `w:gridBefore` as written is one Word does not open a document with: not
/// a number by its rule, or one below 0 or above `GRID_VALUE_MAX` (a span of `0` it opens).
fn refused(raw: &str) -> bool {
    grid_value(raw).is_none_or(|v| !(0..=GRID_VALUE_MAX).contains(&v))
}

/// Whether Word did not open the case's table: some span or `w:gridBefore` in it is
/// [`refused`].
fn refused_values(table_xml: &str, read: &Read) -> bool {
    let negative_before = table_xml
        .split(r#"<w:gridBefore w:val=""#)
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .any(refused);
    let refused_span = read
        .cells
        .iter()
        .any(|(_, _, spec)| spec.span_raw.as_deref().is_some_and(refused));
    negative_before || refused_span
}

/// The written grid: three 2000-twip columns (`cases/grid_values_cases.py`).
const WRITTEN_GRID: [u64; 3] = [2000, 2000, 2000];

/// Whether Word wrote the case's `w:tblGrid` otherwise than written: it re-gridded the table,
/// and its drawing is on a grid of its own.
fn regridded(entry: &Value) -> bool {
    entry["saved"]["tblGrid"]
        .as_array()
        .is_some_and(|g| !g.iter().map(Value::as_u64).eq(WRITTEN_GRID.iter().map(|w| Some(*w))))
}

#[test]
fn the_grid_places_each_cell_where_word_draws_it() {
    let recorded = observations();
    let (mut drawn_cases, mut regridded_cases) = (0, 0);
    for (case, entry) in recorded.as_object().unwrap() {
        if case.starts_with('_') {
            continue;
        }
        let table = entry["table"].as_str().unwrap();
        let read = read(table);
        let drawn = entry["drawn"].as_object().filter(|d| d.contains_key("R1"));
        if entry["saved"].is_null() && drawn.is_none() {
            // Word opened nothing: some span or count in the case is one it refuses.
            assert!(
                refused_values(table, &read),
                "{case}: Word refused it, the grid reads every value as one Word opens"
            );
            continue;
        }
        assert!(
            !refused_values(table, &read),
            "{case}: Word opened it, the grid reads a value as one Word refuses"
        );
        if regridded(entry) || read.grid.rows.iter().any(|r| r.end > read.grid.columns) {
            // Word repairs a table whose rows do not cover its grid -- a `w:tblGrid` of its own,
            // or (`before-2147483645`, `span-alone-99999999999999999999`) the row's values --
            // each time its own way: the grid reports the row -- off its grid, or short of it
            // with no `w:gridAfter` for the columns it leaves (Word does not read `w:gridAfter`:
            // it fills them, `span-1.5`, and sizes the grid's columns anew;
            // `grid-after-differs`).
            regridded_cases += 1;
            assert!(
                read.grid
                    .rows
                    .iter()
                    .any(|r| r.coverage != Coverage::Exact || r.after_differs()),
                "{case}: Word re-gridded the table, the grid reads every row as covering it"
            );
            continue;
        }
        let drawn = drawn.unwrap_or_else(|| panic!("{case}: Word kept the grid and drew nothing"));
        drawn_cases += 1;
        // Where Word drew each grid column's left edge: the reference row's cells, one a column.
        let left = |first: usize| {
            read.cells
                .iter()
                .find(|(text, at, _)| text.starts_with('R') && *at == first)
                .and_then(|(text, _, _)| drawn.get(text))
                .and_then(|at| at["column"].as_f64())
        };
        for (marker, at) in drawn {
            let Some(column) = at["column"].as_f64() else { continue };
            let (_, first, _) = read
                .cells
                .iter()
                .find(|(text, _, _)| text == marker)
                .unwrap_or_else(|| panic!("{case}: no cell {marker}"));
            let edge = left(*first).unwrap_or_else(|| panic!("{case}: no reference cell at column {first}"));
            assert!(
                (column - edge).abs() < 0.05,
                "{case}: {marker} drawn at {column}, the grid puts it at column {first} ({edge})"
            );
        }
    }
    assert!(drawn_cases >= 40, "{drawn_cases} cases drawn on the written grid");
    assert!(regridded_cases >= 20, "{regridded_cases} cases re-gridded");
}

/// A value Word wrote back: `None` where it wrote none.
fn written(value: &Value) -> Option<&str> {
    value.as_str()
}

/// Where no row's cells of a case reach past its grid, Word writes back each span and `w:gridBefore`
/// as the rule reads it (`2.9` as `2`, `99999999999999999999` past the grid is not such a case),
/// and a `w:gridAfter` of the columns the row leaves, whatever was written there: Word does
/// not read `w:gridAfter`.
#[test]
fn word_writes_back_the_values_the_grid_reads() {
    let recorded = observations();
    let mut compared = 0;
    for (case, entry) in recorded.as_object().unwrap() {
        if case.starts_with('_') || entry["saved"].is_null() {
            continue;
        }
        let read = read(entry["table"].as_str().unwrap());
        if read.grid.rows.iter().any(|r| r.end > read.grid.columns) {
            continue;
        }
        compared += 1;
        let saved = entry["saved"]["rows"].as_array().unwrap();
        for ((row, line), spec) in saved.iter().zip(&read.grid.rows).zip(&read.rows) {
            let count = |v: &Value| written(v).map_or(0, |n| n.parse::<usize>().unwrap());
            assert_eq!(count(&row["gridBefore"]), spec.before, "{case}: gridBefore");
            assert_eq!(
                count(&row["gridAfter"]),
                read.grid.columns - line.end,
                "{case}: gridAfter"
            );
            for (cell, placed) in row["cells"].as_array().unwrap().iter().zip(&line.cells) {
                let span = written(&cell["gridSpan"]).map_or(1, |n| n.parse::<usize>().unwrap());
                assert_eq!(span, placed.span, "{case}: {} gridSpan", cell["text"]);
            }
        }
    }
    assert!(compared >= 40, "{compared} cases compared");
}
