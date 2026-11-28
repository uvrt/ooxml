//! Owns: a table's grid in a view, as Word rebuilds it from the cells the view leaves, and
//! where each cell of each row is in it (Word 16.106, Accept All and Reject All alike; lanes
//! fix-final-grid, `word-grid.json`, and fix-moved-left, `word-moved.json`):
//! - Word works on cells with widths: each cell is as wide as the `w:tblGrid` columns it
//!   spans (Word's grid widths, not `w:tcW`: `WidthsStaggerDel`), a row's cells side by side
//!   from its `w:gridBefore`'s end;
//! - a cell the view removes that keeps something of its own (text, or a paragraph mark the
//!   view keeps: [`super::content_kept_in`]) is merged into the cell before it in its row,
//!   or into the cell after it where none is before (`Stagger`, `SpanRemoved`), the widths
//!   summed and its text not kept; one that keeps nothing goes, and the cells after it move
//!   left with their widths (`Stagger-gone`, r55); so does a cell the final view takes into
//!   the cell before it, that cell as wide as the last it takes in (`EndTaken`), and one
//!   gone with its end ([`super::cells`]);
//! - the view's grid columns run between the places any row has a cell edge at (its start,
//!   each cell's start and end, its `w:gridAfter`'s end), up to the widest row; so columns no
//!   row divides are one (`19830c99`, `RedundantPlain`), a column past every row goes
//!   (`UncoveredColumn`), and a row past `w:tblGrid` adds columns (`Merge`); but a row
//!   reaching past the widest a table is fitted to ([`past_widest`]: 63 columns) widens
//!   nothing, Word re-griding it a way of its own: the grid is then `w:tblGrid` and each cell
//!   is at its own grid columns;
//! - a vertical merge whose cell no longer has the edges of the cell above it goes (Word
//!   drops the merge: `VMergeBeside`, `VMergeMoved`, `VMergeRemoved`; [`ViewGrid::merge_broken`]);
//! - each cell spans as its own side of a pending properties change (`w:tcPrChange`) has it,
//!   the original view's the old properties clean ([`View::cell_properties`], user decision
//!   original-clean-grid-2026-11-28), and a vertical merge either side has stays
//!   ([`View::merge_element`]).
//!
//! Where `w:tblGrid` gives a column no width (none, or 0), each column counts as one wide:
//! the grid then follows the cells' columns. A column past `w:tblGrid` is as wide as the
//! grid's columns on average (measured only on r55's shape, where Word took the cell's own
//! `w:tcW`).
//!
//! Positions: a [`ViewRow`] says where each of a row's cells is in the view's columns, and
//! which source grid columns it was; [`ViewGrid::source_column`] says which source grid
//! column a column of the view is in every row, where it is one.
//!
//! Must not: build a model, word a change, or change a table.

use super::{content_kept_in, row_ends, View};
use crate::docx::tables::{grid_span, past_widest, row_cells, RowSpec};
use crate::xmlread::{attr_in, child_in, Ns};
use ooxml_xml::Element;
use std::collections::{BTreeSet, HashSet};
use std::ops::Range;

/// What a view does with one of a row's cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// A cell of the view (of a final view's cells taken in, the first).
    Kept,
    /// Taken into the kept cell before it by its end (the final view): that cell is as wide
    /// as the last it takes in.
    Taken,
    /// Removed, keeping something of its own: merged into the kept cell before it, or after
    /// it where none is before.
    Joins,
    /// Removed with nothing of its own, or gone with its end: the cells after it move left.
    Dropped,
}

/// One row of a table as a view leaves it, in source grid columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowShape {
    /// Where its first cell begins (`w:gridBefore`).
    pub before: usize,
    /// Its cells in order (row-level wrappers seen through).
    pub cells: Vec<ShapeCell>,
    /// Its `w:gridAfter`'s columns.
    pub after: usize,
}

/// A cell of a [`RowShape`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeCell {
    /// The source grid columns it covers.
    pub columns: Range<usize>,
    /// What the view does with it.
    pub fate: Fate,
}

impl RowShape {
    /// `row` in `view`: `None` where the view removes the row.
    #[must_use]
    pub fn of(view: View, row: &Element) -> Option<RowShape> {
        if view.removes_row(row) {
            return None;
        }
        let spec = RowSpec::of(row);
        let all: Vec<_> = row_cells(row).into_iter().map(|(tc, _)| tc).collect();
        // the final view's cell ends, by place among the cells it shows
        let shown: Vec<usize> = (0..all.len())
            .filter(|&k| all.get(k).is_some_and(|c| !view.removes_cell(c)))
            .collect();
        let (taken, lost): (HashSet<usize>, HashSet<usize>) = if view == View::Final {
            let ends = row_ends(row);
            let at = |n: &usize| shown.get(*n).copied();
            let taken: HashSet<usize> = ends
                .groups
                .iter()
                .flat_map(|g| g.iter().skip(1))
                .filter_map(at)
                .collect();
            let lost = ends.gone.iter().filter_map(at).filter(|k| !taken.contains(k)).collect();
            (taken, lost)
        } else {
            (HashSet::new(), HashSet::new())
        };
        let mut out = RowShape {
            before: spec.before,
            after: spec.after,
            ..RowShape::default()
        };
        let mut column = spec.before;
        for (k, cell) in all.iter().enumerate() {
            let columns = column..column.saturating_add(view.span(cell));
            column = columns.end;
            let fate = if view.removes_cell(cell) {
                if content_kept_in(view, cell) {
                    Fate::Joins
                } else {
                    Fate::Dropped
                }
            } else if taken.contains(&k) {
                Fate::Taken
            } else if lost.contains(&k) {
                Fate::Dropped
            } else {
                Fate::Kept
            };
            out.cells.push(ShapeCell { columns, fate });
        }
        Some(out)
    }

    /// The source grid column after its last cell, its `w:gridAfter` counted.
    fn end(&self) -> usize {
        self.cells
            .last()
            .map_or(self.before, |c| c.columns.end)
            .saturating_add(self.after)
    }

    /// Whether the view changes it: a cell not kept.
    fn changed(&self) -> bool {
        self.cells.iter().any(|c| c.fate != Fate::Kept)
    }
}

/// A row of the view: where its cells are in the view's columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewRow {
    /// The view's columns before its first cell (its `w:gridBefore`).
    pub before: usize,
    /// Its cells in the view, in order.
    pub cells: Vec<ViewCell>,
    /// The view's columns of its `w:gridAfter`.
    pub after: usize,
    /// Whether the view changes it (a cell of it removed, taken in or gone).
    pub changed: bool,
    /// Its `w:gridBefore`'s source grid columns.
    pub source_before: Range<usize>,
    /// Its `w:gridAfter`'s source grid columns.
    pub source_after: Range<usize>,
}

/// A cell of a [`ViewRow`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewCell {
    /// The row's cell it is, by place among the row's cells (wrappers seen through).
    pub cell: usize,
    /// The row's cells it is made of, by place, in order: itself, those merged into it and
    /// those it takes in.
    pub parts: Vec<usize>,
    /// Its source grid columns (its own).
    pub source: Range<usize>,
    /// The view's columns it covers.
    pub columns: Range<usize>,
    /// Whether it is where its source grid columns are: no cell before it moved it, and it
    /// is as wide as the columns it and the cells merged into it cover.
    pub in_place: bool,
}

impl ViewRow {
    /// The cell over the view's column `column`.
    #[must_use]
    pub fn at(&self, column: usize) -> Option<&ViewCell> {
        self.cells.iter().find(|c| c.columns.contains(&column))
    }

    /// The cell that is the row's cell `cell` (by place), or that it is part of.
    #[must_use]
    pub fn holding(&self, cell: usize) -> Option<&ViewCell> {
        self.cells.iter().find(|c| c.parts.contains(&cell))
    }
}

/// Places across a table, in the source grid's widths.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Widths {
    /// Each source grid line's place: `count + 1` of them.
    lines: Vec<i128>,
    /// A column's width past the source grid.
    past: i128,
    /// Whether the places are widths (every column has one), not column counts.
    real: bool,
}

impl Widths {
    /// Columns as wide as `widths` says, the source grid's `count` and those past it; a
    /// column further past as wide as the source grid's on average.
    fn of(widths: &[i64], count: usize) -> Widths {
        let real = !widths.is_empty() && widths.iter().all(|w| *w > 0);
        let width = |w: &i64| if real { i128::from(*w) } else { 1 };
        let mut lines = vec![0i128];
        let mut at = 0i128;
        for w in widths {
            at += width(w);
            lines.push(at);
        }
        let inside = widths.get(..count.min(widths.len())).unwrap_or_default();
        let sum: i128 = inside.iter().map(width).sum();
        let past = if real {
            (sum / i128::try_from(inside.len()).unwrap_or(i128::MAX).max(1)).max(1)
        } else {
            1
        };
        Widths { lines, past, real }
    }

    /// The place of source grid line `column`.
    fn x(&self, column: usize) -> i128 {
        let count = self.lines.len() - 1;
        match self.lines.get(column) {
            Some(x) => *x,
            None => {
                let beyond = i128::try_from(column - count).unwrap_or(i128::MAX);
                self.lines
                    .last()
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(beyond.saturating_mul(self.past))
            }
        }
    }

    /// The source grid line at place `x`, if one is there.
    fn column(&self, x: i128) -> Option<usize> {
        if let Ok(k) = self.lines.binary_search(&x) {
            return Some(k);
        }
        let end = self.lines.last().copied().unwrap_or(0);
        let beyond = x.checked_sub(end)?;
        (beyond > 0 && beyond % self.past == 0)
            .then(|| usize::try_from(beyond / self.past).ok())
            .flatten()
            .map(|b| self.lines.len() - 1 + b)
    }

    fn width(&self, columns: &Range<usize>) -> i128 {
        self.x(columns.end) - self.x(columns.start)
    }
}

/// A table's grid in a view: its columns, and where each row's cells are in them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewGrid {
    /// Each grid line of the view, as a place across the table: `width + 1` of them.
    lines: Vec<i128>,
    /// Each source row's place in the view (`None`: a row the view removes).
    rows: Vec<Option<ViewRow>>,
    /// The source grid's widths.
    widths: Vec<i64>,
    places: Widths,
    /// The source grid kept as it is (a row past the widest a table is fitted to).
    source: bool,
}

/// The width Word gives each source grid column (and each past it a row reaches, up to the
/// widest a table is fitted to): the widest `w:tcW` in twips of the cells over that column
/// alone, else `w:tblGrid`'s (Word 16.106, lane fix-moved-left: a re-save writes these as
/// the grid, `WidthsOnly`, `WidthsStagger`; Accept All moves cells by them, `TcwNotGrid-gone`,
/// `R55-gone`).
fn word_widths(view: View, rows: &[&Element], shapes: &[Option<RowShape>], grid: &[i64]) -> Vec<i64> {
    let widest = shapes.iter().flatten().map(RowShape::end).max().unwrap_or(0);
    let count = if past_widest(grid.len(), widest) {
        grid.len()
    } else {
        widest.max(grid.len())
    };
    let mut out: Vec<i64> = (0..count).map(|k| grid.get(k).copied().unwrap_or(0)).collect();
    let mut stated = vec![false; count];
    for (row, shape) in rows.iter().zip(shapes) {
        let Some(shape) = shape else { continue };
        for ((tc, _), cell) in row_cells(row).iter().zip(&shape.cells) {
            if cell.columns.len() != 1 {
                continue;
            }
            let width = child_in(tc, Ns::W, "tcPr")
                .map(|p| view.cell_properties(p))
                .and_then(|p| child_in(p, Ns::W, "tcW"))
                .filter(|w| matches!(attr_in(w, Some(Ns::W), "type"), Some("dxa") | None))
                .and_then(|w| attr_in(w, Some(Ns::W), "w"))
                .and_then(crate::docx::tables::grid_value)
                .filter(|w| *w > 0);
            let at = cell.columns.start;
            if let (Some(width), Some(slot), Some(seen)) = (width, out.get_mut(at), stated.get_mut(at)) {
                *slot = if *seen { (*slot).max(width) } else { width };
                *seen = true;
            }
        }
    }
    // a column past the grid no cell states a width for: as wide as the grid's on average
    let stated_grid: Vec<i64> = grid.iter().copied().filter(|w| *w > 0).collect();
    let average = i64::try_from(stated_grid.len())
        .ok()
        .filter(|n| *n > 0)
        .map_or(0, |n| stated_grid.iter().sum::<i64>() / n);
    for (slot, seen) in out.iter_mut().zip(&stated).skip(grid.len()) {
        if !seen {
            *slot = average;
        }
    }
    out
}

/// A row's cells laid out: each view cell's place across (start, end), its cell and its
/// parts, and where the row starts and ends.
struct Laid {
    start: i128,
    cells: Vec<(i128, i128, usize, Vec<usize>)>,
    end: i128,
}

fn lay(row: &RowShape, places: &Widths) -> Laid {
    let start = places.x(row.before);
    let mut x = start;
    let mut cells: Vec<(i128, i128, usize, Vec<usize>)> = Vec::new();
    // removed cells keeping something, with no kept cell before them: into the next one
    let mut waiting: Vec<(usize, i128)> = Vec::new();
    for (k, cell) in row.cells.iter().enumerate() {
        let width = places.width(&cell.columns);
        match cell.fate {
            Fate::Kept => {
                let mut parts: Vec<usize> = waiting.iter().map(|(w, _)| *w).collect();
                let wide = width + waiting.drain(..).map(|(_, w)| w).sum::<i128>();
                parts.push(k);
                cells.push((x, x + wide, k, parts));
                x += wide;
            }
            Fate::Joins => match cells.last_mut() {
                Some(last) => {
                    last.1 += width;
                    last.3.push(k);
                    x = last.1;
                }
                None => waiting.push((k, width)),
            },
            Fate::Taken => {
                if let Some(last) = cells.last_mut() {
                    last.1 = last.0 + width;
                    last.3.push(k);
                    x = last.1;
                }
            }
            Fate::Dropped => {}
        }
    }
    let last = row.cells.last().map_or(row.before, |c| c.columns.end);
    let end = x + places.width(&(last..last.saturating_add(row.after)));
    Laid { start, cells, end }
}

impl ViewGrid {
    /// The grid of a table whose rows are `rows` and whose `w:tblGrid` widths are `grid`, in
    /// `view`.
    #[must_use]
    pub fn of(view: View, rows: &[&Element], grid: &[i64]) -> ViewGrid {
        let shapes: Vec<Option<RowShape>> = rows.iter().map(|r| RowShape::of(view, r)).collect();
        let word = word_widths(view, rows, &shapes, grid);
        ViewGrid::build_on(&shapes, grid, &word)
    }

    /// The grid of a table whose `w:tblGrid` widths are `grid` and whose rows in the view are
    /// `rows` (`None`: a row the view removes), each column as wide as `w:tblGrid` says.
    #[must_use]
    pub fn build(rows: &[Option<RowShape>], grid: &[i64]) -> ViewGrid {
        ViewGrid::build_on(rows, grid, grid)
    }

    /// [`ViewGrid::build`] with the columns as wide as `word` says (Word's widths, one per
    /// source grid column and past it).
    fn build_on(rows: &[Option<RowShape>], grid: &[i64], word: &[i64]) -> ViewGrid {
        let count = grid.len();
        let places = Widths::of(word, count);
        let widest = rows.iter().flatten().map(RowShape::end).max().unwrap_or(count);
        if past_widest(count, widest) {
            return ViewGrid::as_source(rows, grid, places);
        }
        let laid: Vec<Option<Laid>> = rows.iter().map(|r| r.as_ref().map(|r| lay(r, &places))).collect();
        let reach = laid
            .iter()
            .flatten()
            .map(|l| l.end)
            .max()
            .unwrap_or_else(|| places.x(count));
        let mut edges: BTreeSet<i128> = BTreeSet::from([0, reach]);
        for row in laid.iter().flatten() {
            edges.insert(row.start);
            edges.insert(row.end);
            for cell in &row.cells {
                edges.insert(cell.0);
                edges.insert(cell.1);
            }
        }
        let lines: Vec<i128> = edges.into_iter().filter(|e| *e <= reach).collect();
        let at = |x: i128| lines.partition_point(|l| *l < x);
        let view_rows = rows
            .iter()
            .zip(laid)
            .map(|(shape, laid)| {
                let (shape, laid) = (shape.as_ref()?, laid?);
                let last = laid.cells.last().map_or(laid.start, |c| c.1);
                Some(ViewRow {
                    before: at(laid.start),
                    cells: laid
                        .cells
                        .into_iter()
                        .map(|(start, end, cell, parts)| {
                            let column = |k: Option<&usize>| k.and_then(|k| shape.cells.get(*k));
                            let first = column(parts.first()).map_or(0, |c| c.columns.start);
                            let last = column(parts.last()).map_or(0, |c| c.columns.end);
                            ViewCell {
                                cell,
                                source: shape.cells.get(cell).map_or(0..0, |c| c.columns.clone()),
                                in_place: start == places.x(first) && end == places.x(last),
                                parts,
                                columns: at(start)..at(end),
                            }
                        })
                        .collect(),
                    after: at(laid.end) - at(last),
                    changed: shape.changed(),
                    source_before: 0..shape.before,
                    source_after: shape.end() - shape.after..shape.end(),
                })
            })
            .collect();
        ViewGrid {
            lines,
            rows: view_rows,
            widths: grid.to_vec(),
            places,
            source: false,
        }
    }

    /// The source grid as it is, each cell at its own columns (removed cells left out, none
    /// merged or moved).
    fn as_source(rows: &[Option<RowShape>], grid: &[i64], places: Widths) -> ViewGrid {
        let count = grid.len();
        let view_rows = rows
            .iter()
            .map(|shape| {
                let shape = shape.as_ref()?;
                // each kept cell at its own columns, with the cells merged or taken into it
                let mut cells: Vec<ViewCell> = Vec::new();
                let mut waiting: Vec<usize> = Vec::new();
                for (k, c) in shape.cells.iter().enumerate() {
                    match (c.fate, cells.last_mut()) {
                        (Fate::Kept, _) => {
                            let mut parts = std::mem::take(&mut waiting);
                            parts.push(k);
                            cells.push(ViewCell {
                                cell: k,
                                parts,
                                source: c.columns.clone(),
                                columns: c.columns.clone(),
                                in_place: true,
                            });
                        }
                        (Fate::Joins | Fate::Taken, Some(last)) => last.parts.push(k),
                        (Fate::Joins, None) => waiting.push(k),
                        _ => {}
                    }
                }
                let last = shape.cells.last().map_or(shape.before, |c| c.columns.end);
                Some(ViewRow {
                    before: shape.before,
                    cells,
                    after: shape.after.min(count.saturating_sub(last)),
                    changed: shape.changed(),
                    source_before: 0..shape.before,
                    source_after: last..shape.end(),
                })
            })
            .collect();
        ViewGrid {
            lines: (0..=count).map(|k| places.x(k)).collect(),
            rows: view_rows,
            widths: grid.to_vec(),
            places,
            source: true,
        }
    }

    /// The view's column count.
    #[must_use]
    pub fn width(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }

    /// Each source row in the view (`None`: removed), in order.
    #[must_use]
    pub fn rows(&self) -> &[Option<ViewRow>] {
        &self.rows
    }

    /// Source row `row` in the view.
    #[must_use]
    pub fn row(&self, row: usize) -> Option<&ViewRow> {
        self.rows.get(row).and_then(Option::as_ref)
    }

    /// Whether the view's grid is the source grid as it is, every cell where it was.
    #[must_use]
    pub fn is_source(&self) -> bool {
        let count = self.widths.len();
        self.source
            || (self.width() == count
                && (0..=count).all(|k| self.lines.get(k) == Some(&self.places.x(k)))
                && self.rows.iter().flatten().all(|r| r.cells.iter().all(|c| c.in_place)))
    }

    /// The source grid columns column `column` of the view is in every row, where it is so:
    /// every row's cell over it is at that grid column -- where its source columns are, or
    /// moved left as a whole with as many columns as it had ([`ViewGrid::row_source`]).
    #[must_use]
    pub fn source_column(&self, column: usize) -> Option<Range<usize>> {
        let mut found: Option<usize> = None;
        for row in self.rows.iter().flatten() {
            match self.row_source(row, column) {
                None => {}
                Some(None) => return None,
                Some(Some(at)) if found.is_some_and(|f| f != at) => return None,
                Some(Some(at)) => found = Some(at),
            }
        }
        let start = self.source_line(column);
        let end = self.source_line(column + 1);
        match (found, start, end) {
            (Some(at), Some(s), Some(e)) if at == s => Some(s..e),
            (Some(at), _, _) => Some(at..at + 1),
            (None, Some(s), Some(e)) => Some(s..e),
            _ => None,
        }
    }

    /// The source grid column `row`'s cell over column `column` of the view is at there:
    /// `None` where no cell of the row is over it, `Some(None)` where the cell's columns do
    /// not match the view's one for one (merged with a cell beside it, or as wide as a cell
    /// it takes in, and moved).
    #[must_use]
    pub fn row_source(&self, row: &ViewRow, column: usize) -> Option<Option<usize>> {
        let cell = row.at(column)?;
        let offset = column - cell.columns.start;
        if cell.in_place {
            return Some(Some(self.source_line(column).unwrap_or(cell.source.start)));
        }
        Some((cell.columns.len() == cell.source.len()).then_some(cell.source.start + offset))
    }

    /// The source grid line column `column` of the view begins at, where one is there.
    #[must_use]
    pub fn source_line(&self, column: usize) -> Option<usize> {
        self.places.column(*self.lines.get(column)?)
    }

    /// The source grid columns before the widest row's end (in source columns).
    #[must_use]
    pub fn reach(&self) -> usize {
        self.rows
            .iter()
            .flatten()
            .map(|r| r.source_after.end)
            .max()
            .unwrap_or(self.widths.len())
    }

    /// The source grid's column count.
    #[must_use]
    pub fn count(&self) -> usize {
        self.widths.len()
    }

    /// Where column `column` of the view is no one source column in every row
    /// ([`ViewGrid::source_column`]), the source rows whose cell over it the view moved or
    /// merged (not where its source columns are); none where it is one.
    #[must_use]
    pub fn moved_rows(&self, column: usize) -> Vec<usize> {
        if self.source_column(column).is_some() {
            return Vec::new();
        }
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.as_ref().and_then(|r| r.at(column)).is_some_and(|c| !c.in_place))
            .map(|(k, _)| k)
            .collect()
    }

    /// The widths of the view's columns that begin inside the source grid (a column past it
    /// has none of its own, the table's resolution sizes it as one written past the grid).
    /// Where the view moves or merges no cell, each column's `w:tblGrid` columns summed;
    /// otherwise its place across, in Word's widths ([`word_widths`]; where those leave a
    /// column with no width, the `w:tblGrid` columns between its edges).
    #[must_use]
    pub fn widths(&self) -> Vec<i64> {
        let count = self.widths.len();
        let end = self.places.x(count);
        let real = self.places.real;
        let changed = self.rows.iter().flatten().any(|r| r.changed);
        let summed = |a: usize, b: usize| -> i64 {
            self.widths
                .get(a.min(count)..b.min(count))
                .unwrap_or_default()
                .iter()
                .sum()
        };
        self.lines
            .windows(2)
            .filter_map(|w| match w {
                [a, b] if *a < end => Some((*a, *b)),
                _ => None,
            })
            .map(|(a, b)| match (self.places.column(a), self.places.column(b)) {
                (Some(s), Some(e)) if !changed || !real => summed(s, e),
                _ if real => i64::try_from(b - a).unwrap_or(i64::MAX),
                _ => summed(usize::try_from(a).unwrap_or(0), usize::try_from(b).unwrap_or(0)),
            })
            .collect()
    }

    /// Whether a vertical merge continued in row `row`'s view cell `cell` (by place among
    /// the row's cells in the view) is broken by the view: the row or the row of the view
    /// above it changed, and no cell above has its edges (Word drops such a merge).
    #[must_use]
    pub fn merge_broken(&self, row: usize, cell: usize) -> bool {
        let Some(here) = self.row(row) else { return false };
        let Some(me) = here.cells.get(cell) else { return false };
        let Some(above) = self.rows.get(..row).and_then(|r| r.iter().rev().flatten().next()) else {
            return false;
        };
        (here.changed || above.changed) && !above.cells.iter().any(|c| c.columns == me.columns)
    }
}

impl View {
    /// The cell properties (`w:tcPr`) this view shows: in the original view the `w:tcPr` a
    /// `w:tcPrChange` holds -- the old properties, clean (user decision
    /// original-clean-grid-2026-11-28: Word's Reject All and its Review > Original also take
    /// the widths of the row's other cells away, drawing them 18 pt wide, which no view here
    /// follows); otherwise `properties` itself.
    #[must_use]
    pub fn cell_properties(self, properties: &Element) -> &Element {
        if self == View::Original {
            if let Some(old) = child_in(properties, Ns::W, "tcPrChange").and_then(|c| child_in(c, Ns::W, "tcPr")) {
                return old;
            }
        }
        properties
    }

    /// A cell's `w:gridSpan` in this view (at least 1), from [`View::cell_properties`].
    #[must_use]
    pub fn span(self, cell: &Element) -> usize {
        let raw = child_in(cell, Ns::W, "tcPr")
            .map(|p| self.cell_properties(p))
            .and_then(|p| child_in(p, Ns::W, "gridSpan"))
            .and_then(|g| attr_in(g, Some(Ns::W), "val"));
        grid_span(raw)
    }

    /// A cell's `w:vMerge` in this view: its own side's of a pending properties change
    /// ([`View::cell_properties`]), else the other side's -- Word keeps a vertical merge
    /// either side has (Accept All of a change that ends a merge keeps it, Reject All of one
    /// that makes it keeps it; Word 16.106, lane fix-final-grid, `VMerge`, `VMergeBack`).
    #[must_use]
    pub fn merge_element(self, cell: &Element) -> Option<&Element> {
        let properties = child_in(cell, Ns::W, "tcPr")?;
        let old = child_in(properties, Ns::W, "tcPrChange").and_then(|c| child_in(c, Ns::W, "tcPr"));
        let (own, other) = match self {
            View::Final => (Some(properties), old),
            View::Original => (old.or(Some(properties)), old.map(|_| properties)),
            View::Markup => (Some(properties), None),
        };
        [own, other]
            .into_iter()
            .flatten()
            .find_map(|p| child_in(p, Ns::W, "vMerge"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Fate, RowShape, ShapeCell, ViewGrid};

    const G: [i64; 3] = [1000, 2000, 3000];

    fn row(before: usize, cells: &[(usize, usize, Fate)], after: usize) -> Option<RowShape> {
        Some(RowShape {
            before,
            cells: cells
                .iter()
                .map(|&(s, e, fate)| ShapeCell { columns: s..e, fate })
                .collect(),
            after,
        })
    }

    fn plain(spans: &[(usize, usize)]) -> Option<RowShape> {
        let cells: Vec<_> = spans.iter().map(|&(s, e)| (s, e, Fate::Kept)).collect();
        row(0, &cells, 0)
    }

    /// Each row's cells as (view columns, source start).
    fn shape(grid: &ViewGrid) -> Vec<Vec<(std::ops::Range<usize>, usize)>> {
        grid.rows()
            .iter()
            .flatten()
            .map(|r| r.cells.iter().map(|c| (c.columns.clone(), c.source.start)).collect())
            .collect()
    }

    use Fate::{Dropped, Joins, Kept, Taken};

    #[test]
    fn a_removed_cell_keeping_its_mark_joins_the_cell_before_or_after_it() {
        // Stagger (Word 16.106): [del][B][C], [A][del][C], [A][B][del], [A][B][C]
        let rows = [
            row(0, &[(0, 1, Joins), (1, 2, Kept), (2, 3, Kept)], 0),
            row(0, &[(0, 1, Kept), (1, 2, Joins), (2, 3, Kept)], 0),
            row(0, &[(0, 1, Kept), (1, 2, Kept), (2, 3, Joins)], 0),
            plain(&[(0, 1), (1, 2), (2, 3)]),
        ];
        let grid = ViewGrid::build(&rows, &G);
        assert_eq!(grid.width(), 3);
        assert_eq!(grid.widths(), [1000, 2000, 3000]);
        assert_eq!(
            shape(&grid),
            [
                vec![(0..2, 1), (2..3, 2)],
                vec![(0..2, 0), (2..3, 2)],
                vec![(0..1, 0), (1..3, 1)],
                vec![(0..1, 0), (1..2, 1), (2..3, 2)],
            ]
        );
        assert!(grid.rows().iter().flatten().all(|r| r.cells.iter().all(|c| c.in_place)));
        assert_eq!(grid.source_column(0), Some(0..1));
        // the grid is the source grid, every cell where its columns are (merged cells wider)
        assert!(grid.is_source());
    }

    #[test]
    fn a_removed_cell_keeping_nothing_goes_and_the_cells_after_it_move_left() {
        // Stagger-gone: grid 1000 x 6
        let rows = [
            row(0, &[(0, 1, Dropped), (1, 2, Kept), (2, 3, Kept)], 0),
            row(0, &[(0, 1, Kept), (1, 2, Dropped), (2, 3, Kept)], 0),
            row(0, &[(0, 1, Kept), (1, 2, Kept), (2, 3, Dropped)], 0),
            plain(&[(0, 1), (1, 2), (2, 3)]),
        ];
        let grid = ViewGrid::build(&rows, &G);
        assert_eq!(grid.widths(), [1000; 6]);
        assert_eq!(
            shape(&grid),
            [
                vec![(0..2, 1), (2..5, 2)],
                vec![(0..1, 0), (1..4, 2)],
                vec![(0..1, 0), (1..3, 1)],
                vec![(0..1, 0), (1..3, 1), (3..6, 2)],
            ]
        );
        assert_eq!(grid.source_column(0), None);
        assert_eq!(grid.moved_rows(1), [0, 1]);
        // r55: equal widths, two columns
        let rows = [
            row(0, &[(0, 1, Dropped), (1, 2, Kept), (2, 3, Dropped), (3, 4, Kept)], 0),
            row(0, &[(0, 1, Kept), (1, 2, Dropped), (2, 3, Kept)], 0),
        ];
        let grid = ViewGrid::build(&rows, &[2268; 3]);
        assert_eq!(grid.width(), 2);
        assert_eq!(grid.widths(), [2268, 2268]);
    }

    #[test]
    fn a_cell_taken_in_is_as_wide_as_the_last_it_takes() {
        // EndTaken: [A1 + B1][C1] over [A2][B2][C2]: grid 1000/1000/1000/2000/1000
        let rows = [
            row(0, &[(0, 1, Kept), (1, 2, Taken), (2, 3, Kept)], 0),
            plain(&[(0, 1), (1, 2), (2, 3)]),
        ];
        let grid = ViewGrid::build(&rows, &G);
        assert_eq!(grid.widths(), [1000, 1000, 1000, 2000, 1000]);
        assert_eq!(shape(&grid)[0], [(0..2, 0), (2..4, 2)]);
        assert_eq!(grid.rows()[0].as_ref().map(|r| r.after), Some(0));
    }

    #[test]
    fn grid_before_and_after_keep_their_widths() {
        // BeforeAfter: [before 1][del keeps][C], [A][del keeps][after 1]
        let rows = [
            row(1, &[(1, 2, Joins), (2, 3, Kept)], 0),
            row(0, &[(0, 1, Kept), (1, 2, Joins)], 1),
            plain(&[(0, 1), (1, 2), (2, 3)]),
        ];
        let grid = ViewGrid::build(&rows, &G);
        assert_eq!(grid.width(), 3);
        let first = grid.row(0).cloned().unwrap_or_default();
        assert_eq!(
            (first.before, first.cells.first().map(|c| c.columns.clone())),
            (1, Some(1..3))
        );
        let second = grid.row(1).cloned().unwrap_or_default();
        assert_eq!(
            (second.after, second.cells.first().map(|c| c.columns.clone())),
            (1, Some(0..2))
        );
    }

    #[test]
    fn columns_no_row_divides_are_one() {
        // 19830c99 accepted: every row's first cell over grid columns 0 and 1
        let rows = [plain(&[(0, 2), (2, 3), (3, 4)]), plain(&[(0, 2), (2, 3), (3, 4)])];
        let grid = ViewGrid::build(&rows, &[1512, 1512, 3024, 3024]);
        assert_eq!(grid.width(), 3);
        assert_eq!(grid.widths(), [3024, 3024, 3024]);
        assert_eq!(grid.source_column(0), Some(0..2));
        assert!(!grid.is_source());
    }

    #[test]
    fn one_row_dividing_keeps_the_columns() {
        let rows = [plain(&[(0, 2), (2, 3)]), plain(&[(0, 1), (1, 2), (2, 3)])];
        let grid = ViewGrid::build(&rows, &G);
        assert!(grid.is_source());
        assert_eq!(shape(&grid)[0], [(0..2, 0), (2..3, 2)]);
    }

    #[test]
    fn the_grid_spans_to_the_widest_row() {
        // Merge accepted: a row past the grid adds a column
        let rows = [plain(&[(0, 3), (3, 4)]), plain(&[(0, 1), (1, 2), (2, 3)])];
        let grid = ViewGrid::build(&rows, &[2000, 2000, 2000]);
        assert_eq!(grid.width(), 4);
        assert_eq!(grid.widths(), [2000, 2000, 2000]);
        assert_eq!(grid.source_column(3), Some(3..4));
        // UncoveredColumn: a column past every row goes
        let rows = [plain(&[(0, 1), (1, 2), (2, 3)])];
        let grid = ViewGrid::build(&rows, &[2000; 4]);
        assert_eq!(grid.width(), 3);
        assert!(!grid.is_source());
    }

    #[test]
    fn a_grid_without_widths_counts_columns() {
        let rows = [
            row(0, &[(0, 1, Dropped), (1, 2, Kept), (2, 3, Kept)], 0),
            plain(&[(0, 1), (1, 2), (2, 3)]),
        ];
        let grid = ViewGrid::build(&rows, &[0, 0, 0]);
        assert_eq!(grid.width(), 3);
        assert_eq!(shape(&grid)[0], [(0..1, 1), (1..2, 2)]);
        assert_eq!(grid.widths(), [0, 0, 0]);
    }

    #[test]
    fn a_row_past_the_widest_table_widens_nothing() {
        let rows = [
            plain(&[(0, 2_147_483_645), (2_147_483_645, 2_147_483_646)]),
            plain(&[(0, 1), (1, 2)]),
        ];
        let grid = ViewGrid::build(&rows, &[1, 1]);
        assert!(grid.is_source());
        assert_eq!(
            shape(&grid)[0],
            [(0..2_147_483_645, 0), (2_147_483_645..2_147_483_646, 2_147_483_645)]
        );
    }

    #[test]
    fn a_merge_whose_edges_the_view_moves_is_broken() {
        // VMergeMoved: [A1][B1 restart][C1], [del][B2 continue][C2]: B2 now over 0..2
        let rows = [
            plain(&[(0, 1), (1, 2), (2, 3)]),
            row(0, &[(0, 1, Joins), (1, 2, Kept), (2, 3, Kept)], 0),
            plain(&[(0, 1), (1, 2), (2, 3)]),
        ];
        let grid = ViewGrid::build(&rows, &G);
        assert!(grid.merge_broken(1, 0));
        assert!(!grid.merge_broken(1, 1));
        assert!(grid.merge_broken(2, 0));
        assert!(!grid.merge_broken(2, 2));
    }
}
