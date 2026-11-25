//! Owns: a table's grid consistency as docx-agent checks it (`validate.py`,
//! `grid_problems`): every row covers the grid exactly (`w:gridBefore`, its cells' spans,
//! `w:gridAfter`), and a vertical merge goes on only under a cell of the same columns; a
//! table holding cell or grid revisions is held to it once they are accepted or rejected.
//! The rows are placed as [`Grid`] places them (Word's value rule and cell walk); a span
//! Word does not read as one column or more is reported (`grid-span-invalid`).
//!
//! Must not: look outside the table.

use crate::names::W;
use ooxml_word_model::docx::tables::{row_cells, Coverage, Grid, RowSpec};
use ooxml_xml::{Element, Node};

const STRUCTURAL: &[&str] = &[
    "cellIns",
    "cellDel",
    "cellMerge",
    "tcPrChange",
    "tblGridChange",
    "trPrChange",
];

fn is_w(element: &Element, local: &str) -> bool {
    element.name().is(Some(W), local)
}

/// Elements below `element` (not itself) in document order, with the nearest enclosing
/// element named `owner` of each (`None`: `element` itself is nearest).
fn below<'e>(element: &'e Element, owner: &str, out: &mut Vec<(&'e Element, bool)>, mine: bool) {
    for node in element.children().iter() {
        let Node::Element(child) = node else { continue };
        out.push((child, mine));
        below(child, owner, out, mine && !is_w(child, owner));
    }
}

fn find_path<'e>(element: &'e Element, path: &[&str]) -> Option<&'e Element> {
    let mut at = element;
    for local in path {
        at = at.find(Some(W), local)?;
    }
    Some(at)
}

/// The text of every text node below, as lxml's `itertext` gives it.
fn itertext(element: &Element) -> String {
    element.text_content()
}

/// Python's `repr` of a string, as the detail quotes the table's first cell.
fn repr(text: &str) -> String {
    ooxml_base::pyfmt::repr_str(text)
}

/// The table's grid problems: `(code, detail)`.
pub(crate) fn grid_problems(table: &Element) -> Vec<(&'static str, String)> {
    let mut all = Vec::new();
    below(table, "tbl", &mut all, true);
    // the table's own rows: those whose nearest enclosing table is this one
    let rows: Vec<&Element> = all
        .iter()
        .filter(|(e, mine)| *mine && is_w(e, "tr"))
        .map(|(e, _)| *e)
        .collect();
    let structural = rows.iter().any(|row| {
        let mut inner = Vec::new();
        below(row, "", &mut inner, true);
        STRUCTURAL.iter().any(|s| is_w(row, s)) || inner.iter().any(|(e, _)| STRUCTURAL.iter().any(|s| is_w(e, s)))
    });
    if structural || find_path(table, &["tblGrid", "tblGridChange"]).is_some() {
        return Vec::new();
    }
    let count = table
        .find(Some(W), "tblGrid")
        .map_or(0, |g| g.child_elements().filter(|c| is_w(c, "gridCol")).count()) as i64;
    let label = find_path(table, &["tr", "tc"])
        .map(|c| itertext(c).chars().take(30).collect::<String>())
        .unwrap_or_default();
    let label = repr(&label);
    let mut out = Vec::new();
    let specs: Vec<RowSpec> = rows.iter().map(|row| RowSpec::of(row)).collect();
    let grid = Grid::build(&specs, usize::try_from(count).unwrap_or(0));
    let mut previous: Vec<(usize, usize)> = Vec::new();
    for (k, ((row, spec), line)) in rows.iter().zip(&specs).zip(&grid.rows).enumerate() {
        let mut current = Vec::new();
        for (((cell, _), value), placed) in row_cells(row).iter().zip(&spec.cells).zip(&line.cells) {
            if !value.span_valid() {
                out.push(("grid-span-invalid", format!("{label} row {k}")));
            }
            let (column, span) = (placed.first, placed.span);
            if let Some(merge) = find_path(cell, &["tcPr", "vMerge"]) {
                let value = merge
                    .attribute(Some(W), "val")
                    .filter(|v| !v.is_empty())
                    .unwrap_or("continue");
                let above = previous.iter().find(|(c, _)| *c == column).map(|(_, s)| *s);
                if value == "continue" && above != Some(span) {
                    out.push(("vmerge-without-start", format!("{label} row {k} column {column}")));
                }
            }
            current.push((column, span));
        }
        let reach = line.end + line.after;
        if line.coverage != Coverage::Exact {
            out.push((
                "grid-row-mismatch",
                format!("{label} row {k}: {reach} of {count} columns"),
            ));
        }
        previous = current;
    }
    out
}
