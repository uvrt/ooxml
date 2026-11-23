//! Owns: `column_fit` (docx-agent's `column_fit` and `_fit_columns`, round 31, R6): per
//! body table, a column whose widest line uses at most [`ROOMY_SHARE`] of its text width
//! (every paragraph of it on one line) beside a column with a paragraph drawn on at least
//! [`WRAP_LINES`] lines, and the grid widths that let both fit -- the roomy one its widest
//! line or word plus its cell margins and [`FIT_SLACK`], the room it frees (at least
//! [`MIN_FREED`]) to the wrapping one. Cells over more than one column are not counted.
//!
//! Must not: word an entry (the tool layer does) or cap a list.

use super::tables::sources;
use super::tree::{self, cell_line, is_w, W};
use super::{points, round, Facts};
use ooxml_word_model::resolve::resolve_table;
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// A column is roomy when its widest line uses at most this share of its text width...
pub const ROOMY_SHARE: f64 = 0.6;
/// ... and it would give its neighbour at least this, pt;
pub const MIN_FREED: f64 = 18.0;
/// a column wraps heavily when one of its cells has a paragraph drawn on this many lines.
pub const WRAP_LINES: usize = 3;
/// Room left beside the roomy column's widest line, pt.
pub const FIT_SLACK: f64 = 3.0;

/// One column of a `column_fit` entry, pt.
#[derive(Debug, Clone, PartialEq)]
pub struct FitColumn {
    /// Its grid column, from 0.
    pub column: usize,
    /// The grid column's width.
    pub width: f64,
    /// Its widest line as drawn.
    pub widest_line: f64,
    /// Its widest run without a space.
    pub longest_word: f64,
    /// The most lines one of its paragraphs is drawn on.
    pub lines: usize,
}

/// A roomy column beside one that wraps.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnFit {
    /// The table's id.
    pub table: String,
    /// The roomy column.
    pub roomy: FitColumn,
    /// The one that wraps.
    pub wraps: FitColumn,
    /// Every grid column's suggested width, pt.
    pub widths: Vec<f64>,
    /// The table's 1-based pages.
    pub pages: Vec<usize>,
}

/// What one cell draws: each paragraph's lines' ink widths and each line's widest word, pt.
#[derive(Debug, Default)]
struct DrawnCell {
    lines: HashMap<String, Vec<f64>>,
    words: Vec<f64>,
    pages: BTreeSet<usize>,
}

/// `column_fit` of the body's tables, the most wrapped first; on the pages asked for, every
/// page of each table measured.
#[must_use]
pub fn column_fit(facts: &Facts<'_>) -> Vec<ColumnFit> {
    let (order, drawn) = drawn_cells(facts);
    let Some(body) = facts.docx().body() else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for path in order {
        let Some(cells) = drawn.get(&path) else { continue };
        let pages: Vec<usize> = cells
            .values()
            .flat_map(|c| c.pages.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !facts.shown(pages.iter().copied()) {
            continue;
        }
        let Some((element, _)) = tree::resolve(body, &path) else {
            continue;
        };
        let id = facts.version().id_of(&path).unwrap_or(&path).to_owned();
        for (roomy, wraps, widths) in fit_columns(facts, element, cells, &path) {
            entries.push(ColumnFit {
                table: id.clone(),
                roomy,
                wraps,
                widths,
                pages: pages.clone(),
            });
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.wraps.lines));
    entries
}

/// A table's cells as drawn, by `(row, cell)` from 0.
type TableCells = BTreeMap<(usize, usize), DrawnCell>;

/// Each body table's cells as drawn, with the tables in the order first drawn: with pages
/// asked for, only the tables drawn on them, on every page each is.
fn drawn_cells(facts: &Facts<'_>) -> (Vec<String>, HashMap<String, TableCells>) {
    let pages = &facts.version().facts().pages;
    let body_cells = |index: usize| {
        pages
            .get(index)
            .into_iter()
            .flat_map(|p| p.lines.iter())
            .filter(|l| super::story_kind(&l.path) == "body")
            .filter_map(|l| cell_line(&l.path).map(|(t, ..)| t.to_owned()))
    };
    let shown: Option<BTreeSet<String>> = (!facts.whole()).then(|| {
        (0..pages.len())
            .filter(|i| facts.shown([i + 1]))
            .flat_map(body_cells)
            .collect()
    });
    let wanted = |table: &str| shown.as_ref().is_none_or(|s| s.contains(table));
    let mut order: Vec<String> = Vec::new();
    let mut drawn: HashMap<String, TableCells> = HashMap::new();
    for index in 0..pages.len() {
        if !body_cells(index).any(|t| wanted(&t)) {
            continue; // only the pages drawing a cell's line are painted
        }
        for line in facts
            .texts(index)
            .unwrap_or_default()
            .iter()
            .filter(|l| l.story.is_none())
        {
            let Some((table, row, cell, paragraph)) = cell_line(&line.path) else {
                continue;
            };
            if !wanted(table) {
                continue;
            }
            if !drawn.contains_key(table) {
                order.push(table.to_owned());
            }
            let seen = drawn
                .entry(table.to_owned())
                .or_default()
                .entry((row, cell))
                .or_default();
            seen.pages.insert(index + 1);
            let (Some((low, high)), Some(widest)) = (line.ink, line.widest) else {
                continue;
            };
            seen.lines
                .entry(paragraph.to_owned())
                .or_default()
                .push(points(high - low));
            seen.words.push(points(widest));
        }
    }
    (order, drawn)
}

/// A column's measures so far.
struct Measured {
    text_width: f64,
    margins: f64,
    widest_line: f64,
    longest_word: f64,
    lines: usize,
}

/// `column_fit`'s columns for one table (`_fit_columns`); none for a table of fewer than
/// two grid columns.
fn fit_columns(
    facts: &Facts<'_>,
    table: &Arc<Element>,
    cells: &TableCells,
    path: &str,
) -> Vec<(FitColumn, FitColumn, Vec<f64>)> {
    #[allow(clippy::cast_precision_loss)]
    let grid: Vec<f64> = grid_widths(table).iter().map(|w| *w as f64 / 20.0).collect();
    if grid.len() < 2 {
        return Vec::new();
    }
    let margins = cell_margins(facts, path);
    let mut columns: BTreeMap<usize, Measured> = BTreeMap::new();
    for (r, row) in table.child_elements().filter(|e| is_w(e, "tr")).enumerate() {
        for (k, (start, span)) in row_cells(row).into_iter().enumerate() {
            let Some(seen) = cells.get(&(r, k)) else { continue };
            let Some(&width) = grid.get(start).filter(|_| span == 1) else {
                continue;
            };
            let (left, right) = margins.get(&(r, k)).copied().unwrap_or((5.4, 5.4));
            let column = columns.entry(start).or_insert(Measured {
                text_width: width - left - right,
                margins: left + right,
                widest_line: 0.0,
                longest_word: 0.0,
                lines: 0,
            });
            column.text_width = column.text_width.min(width - left - right);
            column.margins = column.margins.max(left + right);
            for widths in seen.lines.values() {
                column.widest_line = widths.iter().copied().fold(column.widest_line, f64::max);
                column.lines = column.lines.max(widths.len());
            }
            column.longest_word = seen.words.iter().copied().fold(column.longest_word, f64::max);
        }
    }
    let mut out = Vec::new();
    for (&c, roomy) in &columns {
        if let Some(found) = fit_beside(&grid, &columns, c, roomy) {
            out.push(found);
        }
    }
    out
}

/// The roomy column `c` (`roomy`) fitted beside its most wrapped neighbour, where it is
/// roomy and frees enough: the two columns and every grid column's suggested width.
fn fit_beside(
    grid: &[f64],
    columns: &BTreeMap<usize, Measured>,
    c: usize,
    roomy: &Measured,
) -> Option<(FitColumn, FitColumn, Vec<f64>)> {
    if roomy.lines != 1 || roomy.text_width <= 0.0 || roomy.widest_line > ROOMY_SHARE * roomy.text_width {
        return None;
    }
    let need = round(roomy.widest_line.max(roomy.longest_word) + FIT_SLACK + roomy.margins, 1);
    let freed = round(grid.get(c)? - need, 1);
    if freed < MIN_FREED {
        return None;
    }
    let beside = [c.checked_sub(1), Some(c + 1)]
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n, columns.get(&n)?)))
        .filter(|(_, m)| m.lines >= WRAP_LINES);
    // the first of the most wrapped (Python's max)
    let (n, wraps) = beside.fold(None, |best: Option<(usize, &Measured)>, (n, m)| match best {
        Some(b) if b.1.lines >= m.lines => Some(b),
        _ => Some((n, m)),
    })?;
    let mut widths: Vec<f64> = grid.iter().map(|w| round(*w, 1)).collect();
    *widths.get_mut(c)? = need;
    *widths.get_mut(n)? = round(grid.get(n)? + freed, 1);
    let said = |column: usize, m: &Measured| FitColumn {
        column,
        width: grid.get(column).map_or(0.0, |w| round(*w, 1)),
        widest_line: m.widest_line,
        longest_word: m.longest_word,
        lines: m.lines,
    };
    Some((said(c, roomy), said(n, wraps), widths))
}

/// `w:tblGrid`'s widths, twips (`grid_widths`: an unreadable width is 0).
fn grid_widths(table: &Element) -> Vec<i64> {
    table
        .find(Some(W), "tblGrid")
        .map(|grid| {
            grid.child_elements()
                .filter(|e| is_w(e, "gridCol"))
                .map(|col| int(col.attribute(Some(W), "w")).unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}

/// A row's own cells (row-level content controls seen through): each one's first grid
/// column (`w:gridBefore` counted) and span (`row_cells`).
fn row_cells(row: &Element) -> Vec<(usize, usize)> {
    let before = row
        .find(Some(W), "trPr")
        .and_then(|p| p.find(Some(W), "gridBefore"))
        .and_then(|g| int(g.attribute(Some(W), "val")))
        .unwrap_or(0);
    let mut column = usize::try_from(before).unwrap_or(0);
    let mut out = Vec::new();
    let mut stack: Vec<&Arc<Element>> = row.child_elements().collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        if is_w(node, "tc") {
            let span = node
                .find(Some(W), "tcPr")
                .and_then(|p| p.find(Some(W), "gridSpan"))
                .and_then(|g| int(g.attribute(Some(W), "val")))
                .unwrap_or(1)
                .max(1);
            let span = usize::try_from(span).unwrap_or(1);
            out.push((column, span));
            column += span;
        } else if !is_w(node, "tr") {
            let below: Vec<&Arc<Element>> = node.child_elements().collect();
            stack.extend(below.into_iter().rev());
        }
    }
    out
}

/// An attribute as Python's `int()` reads it (surrounding white space allowed).
fn int(value: Option<&str>) -> Option<i64> {
    value?.trim().parse().ok()
}

/// Each cell's left and right margins, pt, as the layout model resolves the table at
/// `path` (`_cell_margins`), by `(row, cell)`.
fn cell_margins(facts: &Facts<'_>, path: &str) -> HashMap<(usize, usize), (f64, f64)> {
    let Some(table) = facts.model_table(path) else {
        return HashMap::new();
    };
    let resolved = resolve_table(&sources(facts), table);
    #[allow(clippy::cast_precision_loss)]
    resolved
        .rows
        .iter()
        .enumerate()
        .flat_map(|(r, row)| {
            row.cells
                .iter()
                .enumerate()
                .map(move |(k, cell)| ((r, k), (cell.margins[1] as f64 / 20.0, cell.margins[3] as f64 / 20.0)))
        })
        .collect()
}
