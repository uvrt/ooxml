//! The grid readers [`super::Grid`] replaced, kept as a reference while the grid's PRs are
//! open (lane ph2-types2): the old `TableGrid::new` placement and `cells_at`, compared with
//! the new ones on every table of the oracle's fixtures and corpus, and on generated tables
//! with odd values. Every difference is a listed fix backed by Word's probe
//! (`observations/grid-values-word.json`): [`fixed`].
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{cell_merge, cells_at, own_rows, Grid, RowSpec, VMerge};
use crate::docx::find_path;
use crate::ids::W;
use crate::xmlread::py_int;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn int_val(node: Option<&Element>, default: i64) -> i64 {
    node.and_then(|n| n.attribute(Some(W), "val"))
        .and_then(py_int)
        .unwrap_or(default)
}

/// The old cell walk: `w:tc`, `w:sdt` (its first `w:sdtContent`) and `w:customXml`.
fn old_cells(row: &Element) -> Vec<Arc<Element>> {
    let mut out = Vec::new();
    for child in row.child_elements() {
        if child.name().is(Some(W), "tc") {
            out.push(Arc::clone(child));
        } else if child.name().is(Some(W), "sdt") {
            if let Some(content) = child.child_elements().find(|e| e.name().is(Some(W), "sdtContent")) {
                out.extend(old_cells(content));
            }
        } else if child.name().is(Some(W), "customXml") {
            out.extend(old_cells(child));
        }
    }
    out
}

/// The old `cells_at`: each cell's first column and span.
fn old_cells_at(row: &Element) -> Vec<(usize, usize)> {
    let mut column = usize::try_from(int_val(find_path(row, &["trPr", "gridBefore"]), 0)).unwrap_or(0);
    let mut out = Vec::new();
    for tc in old_cells(row) {
        let span = usize::try_from(int_val(find_path(&tc, &["tcPr", "gridSpan"]), 1).max(1)).unwrap_or(1);
        out.push((column, span));
        column += span;
    }
    out
}

/// The old `TableGrid::new` positions: per row, per grid column, the origin `(row, column)`.
fn old_positions(rows: &[&Element]) -> Vec<Vec<Option<(usize, usize)>>> {
    let mut origins: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let before = usize::try_from(int_val(find_path(row, &["trPr", "gridBefore"]), 0)).unwrap_or(0);
        let mut line = vec![None; before];
        let mut column = before;
        for tc in old_cells(row) {
            let span = usize::try_from(int_val(find_path(&tc, &["tcPr", "gridSpan"]), 1).max(1)).unwrap_or(1);
            let continues = cell_merge(&tc, false) == VMerge::Continue;
            let origin = match (continues, origins.get(&column)) {
                (true, Some(origin)) => *origin,
                _ => {
                    for c in column..column + span {
                        origins.insert(c, (r, column));
                    }
                    (r, column)
                }
            };
            line.extend(std::iter::repeat_n(Some(origin), span));
            column += span;
        }
        out.push(line);
    }
    out
}

/// The new positions, in the old form.
fn new_positions(table: &Element, rows: &[&Element]) -> Vec<Vec<Option<(usize, usize)>>> {
    let grid = Grid::of(table, rows);
    grid.rows
        .iter()
        .map(|line| {
            let mut out = vec![None; line.before];
            for cell in &line.cells {
                let (or, ok) = cell.origin;
                let first = grid.rows[or].cells[ok].first;
                out.extend(std::iter::repeat_n(Some((or, first)), cell.span));
            }
            out
        })
        .collect()
}

/// Whether a table holds a value the new rule reads differently, each a listed fix: a grid
/// value Python's `int()` reads otherwise than Word -- white space around it (` 2`) or an
/// underscore (`1_0`), which Word does not open at all (`span-space-2`, `span-1_0` in Word's
/// probe), and the new rule reads as absent; a fraction (`2.0`, Word's `span-2.0`), which Word
/// drops; a number past 32 bits, which Word wraps (`span-overflow`, `span-4294967298`).
fn fixed(rows: &[&Element]) -> bool {
    fn odd(e: &Element) -> bool {
        let value = |name: &str| {
            e.name().is(Some(W), name)
                && e.attribute(Some(W), "val")
                    .is_some_and(|v| py_int(v) != super::grid_value(v))
        };
        value("gridSpan") || value("gridBefore") || value("gridAfter") || e.child_elements().any(|c| odd(c))
    }
    rows.iter().any(|r| odd(r))
}

fn tables(element: &Arc<Element>, out: &mut Vec<Arc<Element>>) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "tbl") {
            out.push(Arc::clone(child));
        }
        tables(child, out);
    }
}

/// A table holding a listed fix, placed by the value rule alone: each row's cells one after
/// the other from its `w:gridBefore`, each over its `w:gridSpan` as [`super::grid_value`] reads it.
fn placed_by_the_rule(table: &Element, rows: &[&Element]) -> Result<(), String> {
    let grid = Grid::of(table, rows);
    for (row, line) in rows.iter().zip(&grid.rows) {
        let read = |e: Option<&Element>, least: i64| {
            e.and_then(|n| n.attribute(Some(W), "val"))
                .and_then(super::grid_value)
                .map_or(least, |v| v.max(least))
        };
        let mut column = usize::try_from(read(find_path(row, &["trPr", "gridBefore"]), 0)).unwrap();
        for ((tc, _), cell) in super::row_cells(row).iter().zip(&line.cells) {
            let span = usize::try_from(read(find_path(tc, &["tcPr", "gridSpan"]), 1)).unwrap();
            if (cell.first, cell.span) != (column, span) {
                return Err(format!(
                    "cell at {} of {}, the rule reads {column} of {span}",
                    cell.first, cell.span
                ));
            }
            column += span;
        }
    }
    Ok(())
}

fn compare(table: &Element) -> Result<(), String> {
    let owned = own_rows(table);
    let rows: Vec<&Element> = owned.iter().map(|(r, _)| &**r).collect();
    if fixed(&rows) {
        return placed_by_the_rule(table, &rows);
    }
    for row in &rows {
        let new: Vec<(usize, usize)> = cells_at(row, &[]).iter().map(|c| (c.column, c.span)).collect();
        if new != old_cells_at(row) {
            return Err(format!("cells_at {:?} != {:?}", new, old_cells_at(row)));
        }
        assert_eq!(RowSpec::of(row).cells.len(), new.len());
    }
    let (old, new) = (old_positions(&rows), new_positions(table, &rows));
    if old != new {
        return Err(format!("positions {new:?} != {old:?}"));
    }
    Ok(())
}

fn docx_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            docx_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "docx") {
            out.push(path);
        }
    }
}

#[test]
fn the_grid_places_every_fixture_and_corpus_table_as_the_old_readers_did() {
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ooxml-oracle");
    let mut files = Vec::new();
    docx_files(&oracle.join("fixtures"), &mut files);
    docx_files(&oracle.join("corpus"), &mut files);
    assert!(files.len() > 100, "the oracle's documents are where they were");
    let (mut seen, mut failures) = (0, Vec::new());
    for file in files {
        let bytes: Arc<[u8]> = std::fs::read(&file).unwrap().into();
        let Ok(archive) = ooxml_zip::Archive::read(bytes, &ooxml_zip::Limits::default()) else {
            continue;
        };
        for entry in archive.entries() {
            let name = &entry.meta().name;
            if !(name.starts_with("word/") && name.ends_with(".xml")) {
                continue;
            }
            let Ok(data) = archive.read_entry(entry) else { continue };
            let Ok(tree) = ooxml_xml::parse(&data, &ooxml_xml::Limits::default()) else {
                continue;
            };
            let mut found = Vec::new();
            tables(tree.root(), &mut found);
            for table in found {
                seen += 1;
                if let Err(e) = compare(&table) {
                    failures.push(format!("{} {name}: {e}", file.display()));
                }
            }
        }
    }
    assert!(seen > 100, "{seen} tables");
    assert!(failures.is_empty(), "{failures:#?}");
}

const VALUES: [&str; 18] = [
    "",
    "0",
    "1",
    "2",
    "3",
    "-1",
    "+2",
    " 2",
    "2 ",
    "x",
    "-0",
    "99999999999999999999",
    "2.0",
    "2.5",
    ".",
    "-0.5",
    "4294967298",
    "1e1",
];

fn generated(seed: u64) -> String {
    let mut state = seed;
    let mut next = |n: usize| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % n
    };
    let attr = |name: &str, k: usize| match VALUES[k] {
        "" => String::new(),
        v => format!(r#"<w:{name} w:val="{v}"/>"#),
    };
    let mut xml = String::from(
        r#"<w:tbl xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:tblGrid><w:gridCol/><w:gridCol/><w:gridCol/></w:tblGrid>"#,
    );
    for _ in 0..1 + next(4) {
        let (before, after) = (
            attr("gridBefore", next(VALUES.len())),
            attr("gridAfter", next(VALUES.len())),
        );
        xml.push_str(&format!("<w:tr><w:trPr>{before}{after}</w:trPr>"));
        for _ in 0..next(4) {
            let merge = [
                "",
                "<w:vMerge/>",
                r#"<w:vMerge w:val="restart"/>"#,
                r#"<w:vMerge w:val="continue"/>"#,
            ][next(4)];
            let cell = format!(
                "<w:tc><w:tcPr>{}{merge}</w:tcPr><w:p/></w:tc>",
                attr("gridSpan", next(VALUES.len()))
            );
            xml.push_str(&match next(4) {
                0 => format!("<w:sdt><w:sdtContent>{cell}</w:sdtContent></w:sdt>"),
                1 => format!("<w:customXml>{cell}</w:customXml>"),
                _ => cell,
            });
        }
        xml.push_str("</w:tr>");
    }
    xml.push_str("</w:tbl>");
    xml
}

#[test]
fn the_grid_places_generated_tables_with_odd_values_as_the_old_readers_did() {
    for seed in 0..3000 {
        let xml = generated(seed);
        let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap();
        compare(tree.root()).unwrap_or_else(|e| panic!("{xml}: {e}"));
    }
}
