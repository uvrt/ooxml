//! The one table grid (`ooxml_word_model::docx::tables::Grid`) held to Word: where Word 16.106
//! drew each cell of the grid-values probe (`word/observations/grid-values-word.json`,
//! `word/probes/grid_values_probe.py`), the grid places it; where Word does not open a case,
//! the value that made it so is not a valid span or count.
//!
//! Must not: score anything but the grid's placement against Word.

// Test code: a recording that is not as read fails the test.
#![allow(clippy::panic, clippy::indexing_slicing, clippy::unwrap_used)]

use ooxml_word_model::docx::tables::{own_rows, CellSpec, Grid};
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

/// Each marker's grid column in the grid built from the case's table.
fn placed(table_xml: &str) -> Vec<(String, usize, Option<CellSpec>)> {
    let xml = table_xml.replacen("<w:tbl>", &format!(r#"<w:tbl xmlns:w="{W}">"#), 1);
    let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap();
    let rows = own_rows(tree.root());
    let elements: Vec<&ooxml_xml::Element> = rows.iter().map(|(r, _)| &**r).collect();
    let grid = Grid::of(tree.root(), &elements);
    let mut out = Vec::new();
    for (row, line) in elements.iter().zip(&grid.rows) {
        let cells = ooxml_word_model::docx::tables::row_cells(row);
        for ((tc, _), cell) in cells.iter().zip(&line.cells) {
            let text = texts(tc);
            out.push((text, cell.first, Some(CellSpec::of(tc))));
        }
    }
    out
}

#[test]
fn the_grid_places_each_cell_where_word_draws_it() {
    let recorded = observations();
    let mut drawn_cases = 0;
    for (case, entry) in recorded.as_object().unwrap() {
        if case.starts_with('_') {
            continue;
        }
        let table = entry["table"].as_str().unwrap();
        let cells = placed(table);
        let Some(drawn) = entry["drawn"].as_object() else {
            // Word opened nothing: some span or count in the case is not valid.
            let invalid = cells
                .iter()
                .any(|(_, _, spec)| spec.as_ref().is_some_and(|s| !s.span_valid()))
                || table.contains(r#"w:gridBefore w:val="-"#);
            assert!(invalid, "{case}: Word refused it, the grid reads every value as valid");
            continue;
        };
        drawn_cases += 1;
        // Where Word drew each grid column's left edge: the reference row's cells, one a column.
        let left = |first: usize| {
            cells
                .iter()
                .find(|(text, at, _)| text.starts_with('R') && *at == first)
                .and_then(|(text, _, _)| drawn.get(text))
                .and_then(|at| at["column"].as_f64())
        };
        for (marker, at) in drawn {
            let Some(column) = at["column"].as_f64() else { continue };
            let (_, first, _) = cells
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
    assert!(drawn_cases >= 10, "{drawn_cases} cases drawn");
}
