//! The grid readers of this crate that `Grid` (`ooxml_word_model::docx::tables`) replaced,
//! kept as a reference while the grid's PRs are open (lane ph2-types3, with ooxml-word-model's
//! `grid_reference_tests`): each old reader, on the editable copy (`Work`) and on elements,
//! compared with the new one on every table of the oracle's fixtures and corpus and on
//! generated tables with odd values. Every difference is a value whose old reading differs
//! from Word's (`observations/grid-values-word.json`): a table holding one is compared only
//! where that reader reads it as before ([`differs`]), and [`the_listed_value_fixes`] lists
//! each such value per reader.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::type_complexity,
    clippy::too_many_lines
)]

use crate::names::W;
use crate::review::xml::{ancestor, find_path};
use crate::work::{Id, Work};
use ooxml_word_model::docx::tables::{grid_count, grid_span, grid_value, CellSpec, VMerge};
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

// ---- the old value readers ----

/// How a reader read a value: `review::xml::int` (`trim`, `parse::<i64>`).
fn old_int(raw: Option<&str>, default: i64) -> i64 {
    raw.and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// `review::xml::digits` (all ASCII digits).
fn old_digits(raw: Option<&str>) -> Option<i64> {
    let value = raw?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// `set_cells`'s `number` (`trim`, `parse::<usize>`).
fn old_usize(raw: Option<&str>, default: usize) -> usize {
    raw.and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(default)
}

/// `row_look`'s `int_val` (`trim`, `parse::<i64>`, below 0 is 0).
fn old_row_look(raw: Option<&str>, default: usize) -> usize {
    raw.and_then(|v| v.trim().parse::<i64>().ok())
        .map_or(default, |v| usize::try_from(v).unwrap_or(0))
}

/// The value families: each old reader's span and count of a raw value, against the new
/// rule's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// `review::xml::int`: `tables::shape`, `columns`, `split`, `new_column`,
    /// `review::edges`, `review::grid`'s `fit_grid`.
    Int,
    /// `review::xml::digits`: `review::grid`'s `grid_map`, `fits`, `absorb`, `skip_after`,
    /// `fix_grid`.
    Digits,
    /// `set_cells`.
    Usize,
    /// `tables::row_look`.
    RowLook,
    /// Python's `int()`: `validate::grid`, `facts::grid`.
    Python,
}

const FAMILIES: [Family; 5] = [
    Family::Int,
    Family::Digits,
    Family::Usize,
    Family::RowLook,
    Family::Python,
];

fn signed(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The old span and count of `raw` in `family`.
fn old_values(family: Family, raw: &str) -> (i64, i64) {
    let raw = Some(raw);
    match family {
        Family::Int => (old_int(raw, 1).max(1), old_int(raw, 0)),
        Family::Digits => (old_digits(raw).unwrap_or(1), old_digits(raw).unwrap_or(0)),
        Family::Usize => (signed(old_usize(raw, 1).max(1)), signed(old_usize(raw, 0))),
        Family::RowLook => (signed(old_row_look(raw, 1).max(1)), signed(old_row_look(raw, 0))),
        Family::Python => (
            raw.and_then(py_int).unwrap_or(1).max(1),
            raw.and_then(py_int).unwrap_or(0),
        ),
    }
}

/// Whether `family` read `raw` otherwise than Word's rule: its span, its count, or (Python's,
/// for `validate`) whether the span is reported as not valid.
fn value_differs(family: Family, raw: &str, span: bool) -> bool {
    let (old_span, old_count) = old_values(family, raw);
    if span {
        let new = signed(grid_span(Some(raw)));
        // validate reports a span not valid: before, one Python's int() reads below 1
        let reported = family == Family::Python
            && py_int(raw).is_some_and(|v| v < 1) == CellSpec::new(Some(raw), VMerge::None).span_valid();
        old_span != new || reported
    } else {
        old_count != signed(grid_count(Some(raw)))
    }
}

/// Whether `rows` hold a grid value `family` read otherwise than Word's rule: the table's
/// old and new readings may differ there, each difference a listed fix.
fn differs(rows: &[&Element], family: Family) -> bool {
    fn walk(e: &Element, family: Family) -> bool {
        let value = e.attribute(Some(W), "val");
        let odd = |name: &str, span: bool| {
            e.name().is(Some(W), name) && value.is_some_and(|v| value_differs(family, v, span))
        };
        odd("gridSpan", true)
            || odd("gridBefore", false)
            || odd("gridAfter", false)
            || e.child_elements().any(|c| walk(c, family))
    }
    rows.iter().any(|r| walk(r, family))
}

// ---- the old readers on the editable copy ----

fn val(work: &Work, id: Id, path: &[&str]) -> Option<String> {
    find_path(work, id, path).and_then(|n| work.get(n, Some(W), "val").map(str::to_owned))
}

fn old_own_rows(work: &Work, table: Id) -> Vec<Id> {
    work.iter(table, W, &["tr"])
        .into_iter()
        .filter(|row| ancestor(work, *row, "tbl") == Some(table))
        .collect()
}

fn old_cells(work: &Work, row: Id) -> Vec<Id> {
    work.iter(row, W, &["tc"])
        .into_iter()
        .filter(|cell| ancestor(work, *cell, "tr") == Some(row))
        .collect()
}

/// `tables::shape::row_cells` (`int`), and as `review::edges::cells_of` read it.
fn old_row_cells(work: &Work, row: Id) -> Vec<(Id, i64, i64)> {
    let mut column = old_int(val(work, row, &["trPr", "gridBefore"]).as_deref(), 0);
    let mut out = Vec::new();
    for cell in old_cells(work, row) {
        let span = old_int(val(work, cell, &["tcPr", "gridSpan"]).as_deref(), 1).max(1);
        out.push((cell, column, span));
        column += span;
    }
    out
}

/// `review::grid::grid_map` (`digits`).
fn old_grid_map(work: &Work, table: Id) -> Vec<(Id, Vec<(Id, i64, i64)>)> {
    old_own_rows(work, table)
        .into_iter()
        .map(|row| {
            let mut column = old_digits(val(work, row, &["trPr", "gridBefore"]).as_deref()).unwrap_or(0);
            let mut cells = Vec::new();
            for cell in old_cells(work, row) {
                let span = old_digits(val(work, cell, &["tcPr", "gridSpan"]).as_deref()).unwrap_or(1);
                cells.push((cell, column, span));
                column += span;
            }
            (row, cells)
        })
        .collect()
}

/// `review::grid::fits` (`digits`).
fn old_fits(work: &Work, table: Id) -> bool {
    let count = work.find(table, W, "tblGrid").map_or(0, |g| {
        work.children(g).iter().filter(|c| work.is(**c, W, "gridCol")).count()
    });
    old_grid_map(work, table).into_iter().all(|(row, cells)| {
        let extra = old_digits(val(work, row, &["trPr", "gridAfter"]).as_deref()).unwrap_or(0);
        let reach = cells.last().map_or(0, |(_, start, span)| start + span) + extra;
        usize::try_from(reach).ok() == Some(count)
    })
}

/// `review::grid::fit_grid`'s reaches (`int`).
fn old_reaches(work: &Work, table: Id) -> Vec<(Id, i64, i64)> {
    old_own_rows(work, table)
        .into_iter()
        .map(|row| {
            let before = old_int(val(work, row, &["trPr", "gridBefore"]).as_deref(), 0);
            let after = old_int(val(work, row, &["trPr", "gridAfter"]).as_deref(), 0);
            let cells = old_row_cells(work, row);
            let end = cells.last().map_or(before, |(_, start, span)| start + span);
            (row, end, after)
        })
        .collect()
}

/// `split`'s and `new_column`'s `skips`, `columns`'s `before` (`int`).
fn old_skips(work: &Work, row: Id) -> (usize, usize) {
    let of = |local: &str| usize::try_from(old_int(val(work, row, &["trPr", local]).as_deref(), 0)).unwrap_or(0);
    (of("gridBefore"), of("gridAfter"))
}

/// `split`'s `no_cell_message` grid (`int`).
fn old_spots(work: &Work, rows: &[Id]) -> Vec<Vec<Option<(usize, usize)>>> {
    let mut grid = Vec::new();
    let mut origins: HashMap<usize, (usize, usize)> = HashMap::new();
    for (r, element) in rows.iter().enumerate() {
        let before = old_skips(work, *element).0;
        let mut line = vec![None; before];
        for (cell, start, span) in old_row_cells(work, *element) {
            let (start, span) = (usize::try_from(start).unwrap_or(0), usize::try_from(span).unwrap_or(1));
            let continues = find_path(work, cell, &["tcPr", "vMerge"])
                .is_some_and(|m| work.get(m, Some(W), "val").unwrap_or("continue") == "continue");
            let origin = continues
                .then(|| origins.get(&start).copied())
                .flatten()
                .unwrap_or_else(|| {
                    for c in start..start + span {
                        origins.insert(c, (r, start));
                    }
                    (r, start)
                });
            line.extend(std::iter::repeat_n(Some(origin), span));
        }
        grid.push(line);
    }
    grid
}

// ---- the old readers on elements ----

fn el_val<'e>(element: &'e Element, path: &[&str]) -> Option<&'e str> {
    ooxml_word_model::docx::find_path(element, path).and_then(|e| e.attribute(Some(W), "val"))
}

/// `set_cells`'s cell walk: `w:tc`, an `w:sdt`'s first `w:sdtContent`, `w:customXml`.
fn old_set_cells_walk(element: &Element, at: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, Arc<Element>)>) {
    for (k, node) in element.children().iter().enumerate() {
        let (Node::Element(child), Ok(k)) = (node, u32::try_from(k)) else {
            continue;
        };
        at.push(k);
        if child.name().is(Some(W), "tc") {
            out.push((at.clone(), Arc::clone(child)));
        } else if child.name().is(Some(W), "sdt") {
            if let Some((c, content)) = child.children().iter().enumerate().find_map(|(c, n)| {
                n.as_element()
                    .filter(|e| e.name().is(Some(W), "sdtContent"))
                    .map(|e| (c, e))
            }) {
                at.push(u32::try_from(c).unwrap_or(u32::MAX));
                old_set_cells_walk(content, at, out);
                at.pop();
            }
        } else if child.name().is(Some(W), "customXml") {
            old_set_cells_walk(child, at, out);
        }
        at.pop();
    }
}

/// `set_cells`'s `grid` and `cell_path` over the rows of `table` (paths from the table).
fn old_set_cells(rows: &[&Element]) -> (Vec<Vec<Option<(usize, usize)>>>, Vec<Vec<Option<Vec<u32>>>>) {
    let mut origins: HashMap<usize, (usize, usize)> = HashMap::new();
    let (mut grid, mut paths) = (Vec::new(), Vec::new());
    for (r, row) in rows.iter().enumerate() {
        let before = old_usize(el_val(row, &["trPr", "gridBefore"]), 0);
        let mut line = vec![None; before];
        let mut column = before;
        let mut found = Vec::new();
        old_set_cells_walk(row, &mut Vec::new(), &mut found);
        let mut by_column = HashMap::new();
        for (below, tc) in found {
            let span = old_usize(el_val(&tc, &["tcPr", "gridSpan"]), 1).max(1);
            let continues = ooxml_word_model::docx::find_path(&tc, &["tcPr", "vMerge"])
                .is_some_and(|m| matches!(m.attribute(Some(W), "val"), None | Some("continue")));
            let origin = match continues.then(|| origins.get(&column).copied()).flatten() {
                Some(origin) => origin,
                None => {
                    for c in column..column + span {
                        origins.insert(c, (r, column));
                    }
                    (r, column)
                }
            };
            by_column.insert(column, below);
            line.extend(std::iter::repeat_n(Some(origin), span));
            column += span;
        }
        let width = column + 3;
        paths.push((0..width).map(|c| by_column.get(&c).cloned()).collect());
        grid.push(line);
    }
    (grid, paths)
}

/// `row_look::row_cells`: every `w:tc` below the row outside a nested table.
fn old_row_look_cells(row: &Element) -> Vec<(Vec<u32>, usize, usize)> {
    fn cells_in(element: &Element, path: &[u32], out: &mut Vec<(Arc<Element>, Vec<u32>)>) {
        for (k, node) in element.children().iter().enumerate() {
            let Some(child) = node.as_element() else { continue };
            let mut place = path.to_vec();
            place.push(u32::try_from(k).unwrap_or(u32::MAX));
            if child.name().is(Some(W), "tc") {
                out.push((Arc::clone(child), place));
            } else if !child.name().is(Some(W), "tbl") && !child.name().is(Some(W), "tr") {
                cells_in(child, &place, out);
            }
        }
    }
    let mut column = old_row_look(el_val(row, &["trPr", "gridBefore"]), 0);
    let mut found = Vec::new();
    cells_in(row, &[], &mut found);
    found
        .into_iter()
        .map(|(cell, place)| {
            let span = old_row_look(el_val(&cell, &["tcPr", "gridSpan"]), 1).max(1);
            let start = column;
            column += span;
            (place, start, span)
        })
        .collect()
}

/// `validate::grid::grid_problems` as it was (Python's `int()`, every `w:tc` below the row).
fn old_grid_problems(table: &Element) -> Vec<(&'static str, String)> {
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
            .and_then(py_int)
            .unwrap_or(default)
    }
    use ooxml_word_model::docx::find_path;
    let mut all = Vec::new();
    below(table, "tbl", &mut all, true);
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
    let count = signed(
        table
            .find(Some(W), "tblGrid")
            .map_or(0, |g| g.child_elements().filter(|c| is_w(c, "gridCol")).count()),
    );
    let label = find_path(table, &["tr", "tc"])
        .map(|c| c.text_content().chars().take(30).collect::<String>())
        .unwrap_or_default();
    let label = ooxml_base::pyfmt::repr_str(&label);
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

/// `facts::grid::off_grid_rows` as it was (Python's `int()`).
fn old_off_grid_rows(table: &Element, rows: &[&Element]) -> (i64, Vec<(usize, i64)>) {
    let count = signed(table.find(Some(W), "tblGrid").map_or(0, |g| {
        g.child_elements().filter(|c| c.name().is(Some(W), "gridCol")).count()
    }));
    let int_val = |raw: Option<&str>, default: i64| raw.and_then(py_int).unwrap_or(default);
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut reach =
            int_val(el_val(row, &["trPr", "gridBefore"]), 0) + int_val(el_val(row, &["trPr", "gridAfter"]), 0);
        for cell in ooxml_word_model::docx::tables::cells_at(row, &[]) {
            reach += int_val(el_val(&cell.element, &["tcPr", "gridSpan"]), 1).max(1);
        }
        if reach != count {
            out.push((r, reach));
        }
    }
    (count, out)
}

// ---- the comparison ----

/// Where old and new differ on `table` (none where a listed value is the cause).
fn compare(table: &Arc<Element>) -> Vec<String> {
    let mut failures = Vec::new();
    let owned = ooxml_word_model::docx::tables::own_rows(table);
    let rows: Vec<&Element> = owned.iter().map(|(r, _)| &**r).collect();
    let same = |family: Family| !differs(&rows, family);
    let mut check = |what: &str, equal: bool, detail: &dyn Fn() -> String| {
        if !equal {
            failures.push(format!("{what}: {}", detail()));
        }
    };

    // on the editable copy
    let work = Work::of(table, Vec::new());
    let root = work.root();
    let work_rows = crate::tables::shape::own_rows(&work, root);
    assert_eq!(work_rows, old_own_rows(&work, root));
    if same(Family::Int) {
        for &row in &work_rows {
            let new: Vec<(Id, i64, i64)> = crate::tables::shape::row_cells(&work, row)
                .into_iter()
                .map(|p| (p.cell, signed(p.start), signed(p.span)))
                .collect();
            let old = old_row_cells(&work, row);
            check("shape::row_cells", new == old, &|| format!("{new:?} != {old:?}"));
            let edges = crate::review::edges::cells_of(&work, row);
            check("edges::cells_of", edges == old, &|| format!("{edges:?} != {old:?}"));
            let skips = crate::tables::shape::skips(&work, row);
            check("skips", skips == old_skips(&work, row), &|| format!("{skips:?}"));
        }
        let (new, old) = (crate::review::grid::reaches(&work, root), old_reaches(&work, root));
        check("fit_grid reaches", new == old, &|| format!("{new:?} != {old:?}"));
        let (new, old) = (
            crate::tables::split::spots(&work, &work_rows),
            old_spots(&work, &work_rows),
        );
        check("split spots", new == old, &|| format!("{new:?} != {old:?}"));
    }
    if same(Family::Digits) {
        let (new, old) = (crate::review::grid::grid_map(&work, root), old_grid_map(&work, root));
        check("grid_map", new == old, &|| format!("{new:?} != {old:?}"));
        let (new, old) = (crate::review::grid::fits(&work, root), old_fits(&work, root));
        check("fits", new == old, &|| format!("{new:?} != {old:?}"));
    }

    // on elements
    if same(Family::Usize) {
        let paths: Vec<Box<[u32]>> = owned.iter().map(|(_, p)| p.clone().into_boxed_slice()).collect();
        let new_grid = crate::set_cells::grid(table, &paths);
        let (old_grid, old_paths) = old_set_cells(&rows);
        check("set_cells grid", new_grid == old_grid, &|| {
            format!("{new_grid:?} != {old_grid:?}")
        });
        for ((path, _), old) in paths.iter().zip(&owned).zip(&old_paths) {
            for (column, old) in old.iter().enumerate() {
                let new = crate::set_cells::cell_path(table, path, column);
                let old = old.as_ref().map(|below| [&path[..], below].concat());
                check("set_cells cell_path", new == old, &|| format!("{new:?} != {old:?}"));
            }
        }
    }
    if same(Family::RowLook) {
        for (row, _) in &owned {
            let new: Vec<(Vec<u32>, usize, usize)> = crate::tables::row_cells(row, &[])
                .into_iter()
                .map(|(_, place, start, span)| (place, start, span))
                .collect();
            let old = old_row_look_cells(row);
            check("row_look::row_cells", new == old, &|| format!("{new:?} != {old:?}"));
        }
    }
    if same(Family::Python) {
        let (new, old) = (crate::validate::grid::grid_problems(table), old_grid_problems(table));
        check("grid_problems", new == old, &|| format!("{new:?} != {old:?}"));
        let row_arcs: Vec<Arc<Element>> = owned.iter().map(|(r, _)| Arc::clone(r)).collect();
        let (count, off) = crate::facts::grid::off_grid_rows(table, &row_arcs);
        let new = (count, off.iter().map(|o| (o.row, o.reaches)).collect::<Vec<_>>());
        let old = old_off_grid_rows(table, &rows);
        check("off_grid_rows", new == old, &|| format!("{new:?} != {old:?}"));
    }
    failures
}

fn tables(element: &Arc<Element>, out: &mut Vec<Arc<Element>>) {
    for child in element.child_elements() {
        if child.name().is(Some(W), "tbl") {
            out.push(Arc::clone(child));
        }
        tables(child, out);
    }
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
fn the_grid_places_every_fixture_and_corpus_table_as_the_old_edit_readers_did() {
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
                for e in compare(&table) {
                    failures.push(format!("{} {name}: {e}", file.display()));
                }
            }
        }
    }
    eprintln!("grid reference (word-edit): {seen} fixture and corpus tables");
    assert!(seen > 100, "{seen} tables");
    assert!(failures.is_empty(), "{failures:#?}");
}

const VALUES: [&str; 22] = [
    "",
    "0",
    "1",
    "2",
    "3",
    "-1",
    "+2",
    " 2",
    "2 ",
    "1_0",
    "x",
    "-0",
    "+0",
    "01",
    "2.0",
    "-2",
    "99999999999999999999",
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
    let attr = |name: &str, k: usize| match VALUES.get(k).copied() {
        None | Some("") => String::new(),
        Some(v) => format!(r#"<w:{name} w:val="{v}"/>"#),
    };
    let mut xml = String::from(
        r#"<w:tbl xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:tblGrid><w:gridCol/><w:gridCol/><w:gridCol/></w:tblGrid>"#,
    );
    // half the values absent, so most tables read alike and are compared whole
    let n = VALUES.len() * 2;
    for _ in 0..1 + next(4) {
        let (before, after) = (attr("gridBefore", next(n)), attr("gridAfter", next(n)));
        xml.push_str(&format!("<w:tr><w:trPr>{before}{after}</w:trPr>"));
        for _ in 0..next(5) {
            let merge = [
                "",
                "<w:vMerge/>",
                r#"<w:vMerge w:val="restart"/>"#,
                r#"<w:vMerge w:val="continue"/>"#,
            ][next(4)];
            let cell = format!(
                "<w:tc><w:tcPr>{}{merge}</w:tcPr><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc>",
                attr("gridSpan", next(n))
            );
            xml.push_str(&match next(5) {
                0 => format!("<w:sdt><w:sdtContent>{cell}</w:sdtContent></w:sdt>"),
                1 => format!("<w:customXml>{cell}</w:customXml>"),
                2 => format!("<w:customXml><w:sdt><w:sdtContent>{cell}</w:sdtContent></w:sdt></w:customXml>"),
                _ => cell,
            });
        }
        xml.push_str("</w:tr>");
    }
    xml.push_str("</w:tbl>");
    xml
}

#[test]
fn the_grid_places_generated_tables_with_odd_values_as_the_old_edit_readers_did() {
    let mut whole = 0;
    for seed in 0..3000 {
        let xml = generated(seed);
        let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_xml::Limits::default()).unwrap();
        let table = Arc::clone(tree.root());
        let failures = compare(&table);
        assert!(failures.is_empty(), "{xml}: {failures:#?}");
        let rows: Vec<Arc<Element>> = ooxml_word_model::docx::tables::own_rows(&table)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        let refs: Vec<&Element> = rows.iter().map(|r| &**r).collect();
        if FAMILIES.iter().all(|f| !differs(&refs, *f)) {
            whole += 1;
        }
    }
    eprintln!("grid reference (word-edit): {whole} of 3000 generated tables compared by every reader");
    assert!(whole > 300, "{whole}");
}

/// Each reader's values read otherwise than Word's rule now -- the PR's listed fixes, each
/// backed by Word's probe (`grid-values-word.json`): `span-0`, `span-plus-2`,
/// `span-space-2`, `span-1_0`, `span-minus-1`, `before-minus-1`, `after-minus-1`, `span-x`,
/// `span-empty`; a fraction Word drops (`span-2.0`, `span-2.5`, `before-1.5`), a number Word
/// wraps to 32 bits (`span-overflow`, `span-4294967298`, `before-4294967298`), no digits read
/// as 0 (`span-.`), an exponent Word refuses (`span-1e1`).
#[test]
fn the_listed_value_fixes() {
    let listed = |family: Family, span: bool| -> Vec<&str> {
        VALUES
            .iter()
            .copied()
            .filter(|v| value_differs(family, v, span))
            .collect()
    };
    const WORD: [&str; 3] = ["2.0", "99999999999999999999", "2.5"];
    let with_word = |head: &[&'static str], tail: &[&'static str]| -> Vec<&'static str> {
        head.iter().chain(&WORD).chain(tail).copied().collect()
    };
    assert_eq!(listed(Family::Int, true), with_word(&[" 2", "2 "], &["4294967298"]));
    assert_eq!(
        listed(Family::Int, false),
        [
            "-1",
            " 2",
            "2 ",
            "2.0",
            "-2",
            "99999999999999999999",
            "2.5",
            "4294967298"
        ]
    );
    assert_eq!(listed(Family::Digits, true), with_word(&["0", "+2"], &["4294967298"]));
    assert_eq!(listed(Family::Digits, false), with_word(&["+2"], &["4294967298"]));
    for family in [Family::Usize, Family::RowLook] {
        assert_eq!(listed(family, true), with_word(&[" 2", "2 "], &["4294967298"]));
        assert_eq!(listed(family, false), with_word(&[" 2", "2 "], &["4294967298"]));
    }
    assert_eq!(
        listed(Family::Python, true),
        [
            "",
            " 2",
            "2 ",
            "1_0",
            "x",
            "2.0",
            "99999999999999999999",
            "2.5",
            ".",
            "-0.5",
            "4294967298",
            "1e1"
        ]
    );
    assert_eq!(
        listed(Family::Python, false),
        [
            "-1",
            " 2",
            "2 ",
            "1_0",
            "2.0",
            "-2",
            "99999999999999999999",
            "2.5",
            "4294967298"
        ]
    );
    // the review's reject (`review::apply`): a skip of 0 goes, by value (Word reads ``, `.`
    // and `-0.5` as 0, `span-empty`, `span-.`, `before--0.5`); spans compare by value
    for v in VALUES {
        let old = v == "0";
        let new = grid_value(v) == Some(0);
        assert!(old == new || ["", "-0", "+0", ".", "-0.5"].contains(&v), "{v:?}");
    }
}
