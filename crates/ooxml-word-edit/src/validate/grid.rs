//! Owns: a table's grid consistency as docx-agent checks it (`validate.py`,
//! `grid_problems`): every row covers the grid exactly (`w:gridBefore`, its cells' spans,
//! `w:gridAfter`), and a vertical merge goes on only under a cell of the same columns; a
//! table holding cell or grid revisions is held to it once they are accepted or rejected.
//!
//! Must not: look outside the table.

use crate::names::W;
use crate::validate::facts::python_int;
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

fn number(element: Option<&Element>, default: i64) -> i64 {
    element
        .and_then(|e| e.attribute(Some(W), "val"))
        .and_then(python_int)
        .unwrap_or(default)
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
    let mut previous: Vec<(i64, i64)> = Vec::new();
    for (k, row) in rows.iter().enumerate() {
        let properties = row.find(Some(W), "trPr");
        let mut column = properties.map_or(0, |p| number(p.find(Some(W), "gridBefore").map(|e| &**e), 0));
        let after = properties.map_or(0, |p| number(p.find(Some(W), "gridAfter").map(|e| &**e), 0));
        let mut current = Vec::new();
        let mut cells = Vec::new();
        below(row, "tr", &mut cells, true);
        for (cell, mine) in cells {
            if !mine || !is_w(cell, "tc") {
                continue;
            }
            let mut span = number(find_path(cell, &["tcPr", "gridSpan"]), 1);
            if span < 1 {
                out.push(("grid-span-invalid", format!("{label} row {k}")));
                span = 1;
            }
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
            column += span;
        }
        if column + after != count {
            out.push((
                "grid-row-mismatch",
                format!("{label} row {k}: {} of {count} columns", column + after),
            ));
        }
        previous = current;
    }
    out
}
