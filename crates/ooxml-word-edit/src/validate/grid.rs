//! Owns: a table's grid consistency as docx-agent checks it (`validate.py`,
//! `grid_problems`): every row covers the grid exactly (`w:gridBefore`, its cells' spans,
//! `w:gridAfter`), and a vertical merge goes on only under a cell of the same columns; a
//! table holding cell or grid revisions is held to it once they are accepted or rejected.
//! The rows are placed as [`Grid`] places them (Word's value rule and cell walk); a span
//! Word does not read as one column or more is reported (`grid-span-invalid`): one Word does
//! not open the document with, or one it reads as 0 (`0`, `0.5`, empty), whose row it
//! re-grids. A span Word reads as a number past 32 bits or with a fraction (`2.0`,
//! `99999999999999999999`) is not reported; a row it takes past the grid is
//! (`grid-row-mismatch`).
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

#[cfg(test)]
mod tests {
    use super::grid_problems;

    /// A table of three grid columns whose one row is a cell of `span` and a plain cell.
    fn problems(span: &str) -> Vec<&'static str> {
        let xml = format!(
            r#"<w:tbl xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:tblGrid><w:gridCol/><w:gridCol/><w:gridCol/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="{span}"/></w:tcPr><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>"#
        );
        let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap_or_else(|e| panic!("{e}"));
        grid_problems(tree.root()).into_iter().map(|(code, _)| code).collect()
    }

    #[test]
    fn spans_word_opens_are_not_reported_and_those_it_refuses_are() {
        // Word 16.106, observations/grid-values-word.json
        for opened in ["2", "2.0", "2.5", "+2.0", "4294967298"] {
            assert_eq!(problems(opened), Vec::<&str>::new(), "{opened:?}");
        }
        // opened, past the grid (Word re-grids the table)
        assert_eq!(problems("99999999999999999999"), ["grid-row-mismatch"]);
        assert_eq!(problems("2147483645"), ["grid-row-mismatch"]);
        // refused by Word, or read as 0 (Word re-grids the short row)
        for refused in ["1e1", " 2", "-2.0", "4294967295", "2147483646"] {
            assert!(problems(refused).contains(&"grid-span-invalid"), "{refused:?}");
        }
        for zero in ["0", ".", "-0.5", "0.9"] {
            assert_eq!(problems(zero), ["grid-span-invalid", "grid-row-mismatch"], "{zero:?}");
        }
    }
}
