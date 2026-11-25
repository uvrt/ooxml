//! Owns: [`Grid`], one table's grid as Word places it, built from plain values
//! ([`RowSpec`]): each cell's first grid column and span, the vertical merges' origins, the
//! grid's width and each row's coverage of `w:tblGrid`; the value rule ([`grid_value`]) and
//! the cell walk ([`row_cells`]) every reader of a table's grid shares, and the adapter from
//! a row element ([`RowSpec::of`]).
//!
//! The rules (Word 16.106, `observations/grid-values-word.json`):
//! - a `w:gridSpan` or `w:gridBefore` value is read as Word reads it ([`grid_value`]): an
//!   optional sign, decimal digits and an optional fraction, which Word drops (`2.0` and `2.9`
//!   are 2), the whole part wrapped to 32 bits (`99999999999999999999` is 1661992959); anything
//!   else is absent. Word does not open a document with a span or `w:gridBefore` that is so
//!   read below 0 (`-1`, `-2.0`, `4294967295`) or above [`GRID_VALUE_MAX`], or that is no such
//!   number (` 2`, `1_0`, `1e1`). A span below 1 is 1 (Word places a span of `0` as one
//!   column); a count below 0 is 0. The raw span is kept ([`CellSpec::span_raw`]), so
//!   validation still reports one that is not valid;
//! - Word does not read `w:gridAfter` at all: it opens any value (`x`, `-5`, `1e0`) and writes
//!   back the columns the row leaves. The grid does the same: a row's trailing columns are
//!   those it leaves of `w:tblGrid` ([`GridRow::trailing`]), and its coverage does not read
//!   `w:gridAfter`. The written count is kept ([`GridRow::after`], by the same rule), so a
//!   value other than the columns the row leaves can be said ([`GridRow::after_differs`]);
//! - a row whose cells reach past `w:tblGrid` (a span past the grid, a `w:gridBefore` of 3 in
//!   three columns) Word re-grids: it writes a `w:tblGrid` of its own. The grid places such
//!   cells as written and reports the row ([`Coverage::Over`]);
//! - a row's cells are its `w:tc`, also inside `w:sdt`/`w:sdtContent` and `w:customXml`, at
//!   any depth, never inside a cell or a nested table;
//! - a cell continuing a vertical merge (`w:vMerge` without `restart`) belongs to the cell
//!   whose grid column it starts at in the rows above (its origin); with none there (the
//!   table's first row) it is an origin itself, drawn in its own place as Word draws it.
//!
//! Must not: read anything but the values it is given (the elements are read only by the
//! adapters), or decide what a view of the table keeps.

use super::merge_heads::VMerge;
use crate::ids::W;
use ooxml_xml::Element;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

/// A grid value (`w:gridSpan`, `w:gridBefore`, `w:gridAfter`) as Word 16.106 reads it
/// (`observations/grid-values-word.json`): an optional `+` or `-`, decimal digits, and an
/// optional `.` with more decimal digits, and nothing else (no white space, `_` or exponent).
/// Word drops the fraction (`2.9` is 2, `.5` and `-0.5` are 0), reads no digits as 0 (``, `+`,
/// `.`), and keeps the whole part modulo 2^32 as a signed 32-bit number (`4294967298` is 2,
/// `99999999999999999999` is 1661992959, `4294967295` is -1); anything else is `None`.
#[must_use]
pub fn grid_value(raw: &str) -> Option<i64> {
    let (negative, body) = match raw.as_bytes().first() {
        Some(b'-') => (true, raw.get(1..)?),
        Some(b'+') => (false, raw.get(1..)?),
        _ => (false, raw),
    };
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !digits(fraction) {
        return None;
    }
    let low = whole
        .bytes()
        .fold(0u32, |v, b| v.wrapping_mul(10).wrapping_add(u32::from(b - b'0')));
    let low = if negative { low.wrapping_neg() } else { low };
    Some(i64::from(i32::from_ne_bytes(low.to_ne_bytes())))
}

/// The largest `w:gridSpan` or `w:gridBefore` Word 16.106 opens a document with (as
/// [`grid_value`] reads it): it does not open one of `GRID_VALUE_MAX + 1`
/// (`observations/grid-values-word.json`, `span-` and `before-` cases at the bound).
pub const GRID_VALUE_MAX: i64 = 2_147_483_645;

/// A count of grid columns (`w:gridBefore`, `w:gridAfter`) as written: absent or not valid
/// is 0, below 0 is 0.
#[must_use]
pub fn grid_count(raw: Option<&str>) -> usize {
    raw.and_then(grid_value)
        .map_or(0, |v| usize::try_from(v.max(0)).unwrap_or(usize::MAX))
}

/// A cell's span (`w:gridSpan`) as written: absent or not valid is 1, below 1 is 1.
#[must_use]
pub fn grid_span(raw: Option<&str>) -> usize {
    raw.and_then(grid_value)
        .map_or(1, |v| usize::try_from(v.max(1)).unwrap_or(usize::MAX))
}

/// One cell's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSpec {
    /// `w:gridSpan`'s value as written, where there is one.
    pub span_raw: Option<Box<str>>,
    /// The span ([`grid_value`], at least 1).
    pub span: usize,
    /// Its part in a vertical merge.
    pub vmerge: VMerge,
}

impl CellSpec {
    /// A cell of `w:gridSpan` `span_raw` (as written) and vertical merge `vmerge`.
    #[must_use]
    pub fn new(span_raw: Option<&str>, vmerge: VMerge) -> CellSpec {
        CellSpec {
            span_raw: span_raw.map(Into::into),
            span: grid_span(span_raw),
            vmerge,
        }
    }

    /// Whether its span is valid: absent, or a [`grid_value`] of at least 1 and at most
    /// [`GRID_VALUE_MAX`] (Word opens a span of 0 and re-grids its row, so it is reported too).
    #[must_use]
    pub fn span_valid(&self) -> bool {
        self.span_raw
            .as_deref()
            .is_none_or(|raw| grid_value(raw).is_some_and(|v| (1..=GRID_VALUE_MAX).contains(&v)))
    }

    /// The values of the cell `tc`.
    #[must_use]
    pub fn of(tc: &Element) -> CellSpec {
        CellSpec::read(&Elements::default(), tc)
    }

    /// The values of the cell `tc` of `source`.
    #[must_use]
    pub fn read<S: GridSource>(source: &S, tc: S::Node) -> CellSpec {
        let properties = child(source, tc, "tcPr");
        let raw = properties
            .and_then(|p| child(source, p, "gridSpan"))
            .and_then(|g| source.w_val(g));
        let vmerge = match properties
            .and_then(|p| child(source, p, "vMerge"))
            .map(|m| source.w_val(m))
        {
            Some(None | Some("continue")) => VMerge::Continue,
            Some(Some("restart")) => VMerge::Restart,
            _ => VMerge::None,
        };
        CellSpec::new(raw, vmerge)
    }
}

/// One row's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSpec {
    /// Grid columns before its first cell (`w:gridBefore`).
    pub before: usize,
    /// Grid columns after its last cell (`w:gridAfter`).
    pub after: usize,
    /// Its cells, in order.
    pub cells: Vec<CellSpec>,
}

impl RowSpec {
    /// A row of `w:gridBefore` / `w:gridAfter` values as written (absent: `None`).
    #[must_use]
    pub fn new(before_raw: Option<&str>, after_raw: Option<&str>, cells: Vec<CellSpec>) -> RowSpec {
        RowSpec {
            before: grid_count(before_raw),
            after: grid_count(after_raw),
            cells,
        }
    }

    /// The values of the row `tr` (its cells by [`row_cells`]).
    #[must_use]
    pub fn of(tr: &Element) -> RowSpec {
        RowSpec::read(&Elements::default(), tr)
    }

    /// The values of the row `tr` of `source` (its cells by [`source_cells`]).
    #[must_use]
    pub fn read<S: GridSource>(source: &S, tr: S::Node) -> RowSpec {
        let properties = child(source, tr, "trPr");
        let value = |name: &str| {
            properties
                .and_then(|p| child(source, p, name))
                .and_then(|g| source.w_val(g))
        };
        let cells = source_cells(source, tr)
            .into_iter()
            .map(|(tc, _)| CellSpec::read(source, tc))
            .collect();
        RowSpec::new(value("gridBefore"), value("gridAfter"), cells)
    }
}

/// A tree a table's grid is read from: [`Element`]s, or an editor's own copy of them. Every
/// tree's rows are read by the one cell walk ([`source_cells`]) and value rule
/// ([`RowSpec::read`], [`grid_value`]).
pub trait GridSource {
    /// A node of the tree.
    type Node: Copy;
    /// `node`'s child elements, each with its place among `node`'s children.
    fn element_children(&self, node: Self::Node) -> Vec<(u32, Self::Node)>;
    /// Whether `node` is the element `w:<local>`.
    fn is_w(&self, node: Self::Node, local: &str) -> bool;
    /// `node`'s `w:val`.
    fn w_val(&self, node: Self::Node) -> Option<&str>;
}

/// [`Element`]s (of lifetime `'a`) as a [`GridSource`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Elements<'a>(PhantomData<&'a Element>);

impl<'a> GridSource for Elements<'a> {
    type Node = &'a Element;
    fn element_children(&self, node: &'a Element) -> Vec<(u32, &'a Element)> {
        node.children()
            .iter()
            .enumerate()
            .filter_map(|(k, n)| n.as_element().map(|e| (u32::try_from(k).unwrap_or(u32::MAX), &**e)))
            .collect()
    }
    fn is_w(&self, node: &'a Element, local: &str) -> bool {
        node.name().is(Some(W), local)
    }
    fn w_val(&self, node: &'a Element) -> Option<&str> {
        node.attribute(Some(W), "val")
    }
}

/// `node`'s first child element `w:<local>`.
fn child<S: GridSource>(source: &S, node: S::Node, local: &str) -> Option<S::Node> {
    source
        .element_children(node)
        .into_iter()
        .map(|(_, c)| c)
        .find(|c| source.is_w(*c, local))
}

/// A row's cells in `source`: its `w:tc`, also inside `w:sdt`/`w:sdtContent` and
/// `w:customXml` at any depth (never inside a cell or a nested table), each with its place
/// below the row (child indexes).
#[must_use]
pub fn source_cells<S: GridSource>(source: &S, row: S::Node) -> Vec<(S::Node, Box<[u32]>)> {
    fn walk<S: GridSource>(source: &S, node: S::Node, at: &mut Vec<u32>, out: &mut Vec<(S::Node, Box<[u32]>)>) {
        for (k, child) in source.element_children(node) {
            at.push(k);
            if source.is_w(child, "tc") {
                out.push((child, at.clone().into_boxed_slice()));
            } else if ["sdt", "sdtContent", "customXml"].iter().any(|n| source.is_w(child, n)) {
                walk(source, child, at, out);
            }
            at.pop();
        }
    }
    let mut out = Vec::new();
    walk(source, row, &mut Vec::new(), &mut out);
    out
}

/// A row's cells ([`source_cells`] over [`Element`]s), each with its place below the row.
#[must_use]
pub fn row_cells(row: &Element) -> Vec<(Arc<Element>, Box<[u32]>)> {
    source_cells(&Elements::default(), row)
        .into_iter()
        .filter_map(|(_, place)| {
            let (last, path) = place.split_last()?;
            let mut at = row;
            for k in path {
                at = at.children().get(usize::try_from(*k).ok()?)?.as_element()?;
            }
            let tc = at.children().get(usize::try_from(*last).ok()?)?.as_element()?;
            Some((Arc::clone(tc), place))
        })
        .collect()
}

/// A row's cells ([`row_cells`]) placed on the grid by the row alone (no merges): each with
/// its place below the row, first grid column and span.
#[must_use]
pub fn placed_row_cells(row: &Element) -> Vec<(Arc<Element>, Box<[u32]>, GridCell)> {
    let grid = Grid::build(std::slice::from_ref(&RowSpec::of(row)), 0);
    let placed = grid.rows.into_iter().next().map(|r| r.cells).unwrap_or_default();
    row_cells(row)
        .into_iter()
        .zip(placed)
        .map(|((tc, place), cell)| (tc, place, cell))
        .collect()
}

/// A cell placed on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridCell {
    /// The grid column it starts at.
    pub first: usize,
    /// How many grid columns it covers.
    pub span: usize,
    /// The cell its vertical merge starts at, `(row, cell)`: itself where it starts one or
    /// is in none.
    pub origin: (usize, usize),
}

impl GridCell {
    /// The grid columns it covers.
    #[must_use]
    pub fn columns(&self) -> std::ops::Range<usize> {
        self.first..self.first + self.span
    }
}

/// How a row's grid columns (before and cells; the columns it leaves are its trailing ones,
/// as Word fills them) meet `w:tblGrid`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// Within them.
    Exact,
    /// This many more: Word re-grids the row.
    Over(usize),
}

/// A row on the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRow {
    /// Grid columns before its first cell.
    pub before: usize,
    /// Grid columns after its last cell as written (`w:gridAfter`, which Word does not read).
    pub after: usize,
    /// Grid columns after its last cell as Word fills them: those of `w:tblGrid` it leaves.
    pub trailing: usize,
    /// Its cells, in order.
    pub cells: Vec<GridCell>,
    /// The grid column after its last cell.
    pub end: usize,
    /// Its columns (before and cells) against `w:tblGrid`'s.
    pub coverage: Coverage,
}

impl GridRow {
    /// Whether its written `w:gridAfter` is not the columns it leaves of the grid, in a row
    /// within the grid (Word writes back the columns it leaves).
    #[must_use]
    pub fn after_differs(&self) -> bool {
        self.coverage == Coverage::Exact && self.after != self.trailing
    }
}

/// A table's grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// Its rows.
    pub rows: Vec<GridRow>,
    /// The widest row's grid columns, its trailing ones counted.
    pub width: usize,
    /// `w:tblGrid`'s columns.
    pub columns: usize,
}

impl Grid {
    /// The grid of `rows` in a table whose `w:tblGrid` has `columns` columns.
    #[must_use]
    pub fn build(rows: &[RowSpec], columns: usize) -> Grid {
        let mut origins = Origins::default();
        let mut out = Vec::with_capacity(rows.len());
        for (r, row) in rows.iter().enumerate() {
            let mut column = row.before;
            let mut cells = Vec::with_capacity(row.cells.len());
            for (k, cell) in row.cells.iter().enumerate() {
                let continued = match cell.vmerge {
                    VMerge::Continue => origins.at(column),
                    VMerge::None | VMerge::Restart => None,
                };
                let origin = continued.unwrap_or_else(|| {
                    origins.set(column, column.saturating_add(cell.span), (r, k));
                    (r, k)
                });
                cells.push(GridCell {
                    first: column,
                    span: cell.span,
                    origin,
                });
                column = column.saturating_add(cell.span);
            }
            let coverage = if column > columns {
                Coverage::Over(column - columns)
            } else {
                Coverage::Exact
            };
            out.push(GridRow {
                before: row.before,
                after: row.after,
                trailing: columns.saturating_sub(column),
                cells,
                end: column,
                coverage,
            });
        }
        let width = out.iter().map(|r| r.end.saturating_add(r.trailing)).max().unwrap_or(0);
        Grid {
            rows: out,
            width,
            columns,
        }
    }

    /// The grid of the rows `rows` of `table` (`w:tblGrid` read from `table`).
    #[must_use]
    pub fn of(table: &Element, rows: &[&Element]) -> Grid {
        let specs: Vec<RowSpec> = rows.iter().map(|r| RowSpec::of(r)).collect();
        Grid::build(&specs, grid_columns(table))
    }

    /// The cell `(row, cell)` covering grid position `(row, column)`, where there is one.
    #[must_use]
    pub fn cell_at(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        let line = self.rows.get(row)?;
        let k = line.cells.iter().position(|c| c.columns().contains(&column))?;
        Some((row, k))
    }

    /// The origin `(row, cell)` of the cell covering grid position `(row, column)`.
    #[must_use]
    pub fn origin_at(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        let (r, k) = self.cell_at(row, column)?;
        self.rows.get(r)?.cells.get(k).map(|c| c.origin)
    }
}

/// Which cell `(row, cell)` a vertical merge starting at each grid column goes on from: the
/// last cell placed over that column. Kept as column ranges, so a span of millions of columns
/// (a value Word wraps to 32 bits, [`grid_value`]) costs one entry, not one per column.
#[derive(Debug, Default)]
struct Origins(BTreeMap<usize, (usize, (usize, usize))>);

impl Origins {
    /// The origin over grid column `column`.
    fn at(&self, column: usize) -> Option<(usize, usize)> {
        self.0
            .range(..=column)
            .next_back()
            .filter(|(_, (end, _))| column < *end)
            .map(|(_, (_, origin))| *origin)
    }

    /// `origin` over the grid columns `start..end`, in place of what was there.
    fn set(&mut self, start: usize, end: usize, origin: (usize, usize)) {
        if start >= end {
            return;
        }
        let over: Vec<(usize, usize, (usize, usize))> = self
            .0
            .range(..end)
            .rev()
            .take_while(|(_, (e, _))| *e > start)
            .map(|(s, (e, o))| (*s, *e, *o))
            .collect();
        for (s, e, o) in over {
            self.0.remove(&s);
            if s < start {
                self.0.insert(s, (start, o));
            }
            if e > end {
                self.0.insert(end, (e, o));
            }
        }
        self.0.insert(start, (end, origin));
    }
}

/// The number of `w:gridCol` in `table`'s `w:tblGrid`.
#[must_use]
pub fn grid_columns(table: &Element) -> usize {
    table.find(Some(W), "tblGrid").map_or(0, |g| {
        g.child_elements().filter(|c| c.name().is(Some(W), "gridCol")).count()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_values_are_read_as_word_opens_them() {
        // Word 16.106, observations/grid-values-word.json: each value and what Word read
        for (raw, read) in [
            ("2", 2),
            ("+2", 2),
            ("-1", -1),
            ("2.0", 2),
            ("2.", 2),
            ("2.5", 2),
            ("2.9", 2),
            ("+2.0", 2),
            ("2.00", 2),
            ("1.5", 1),
            ("0.5", 0),
            (".5", 0),
            ("+.5", 0),
            ("-0", 0),
            ("-0.5", 0),
            ("-.5", 0),
            ("-2.0", -2),
            ("", 0),
            ("+", 0),
            ("-", 0),
            (".", 0),
            ("4294967298", 2),
            ("8589934594", 2),
            ("4294967296", 0),
            ("4294967295", -1),
            ("2147483648", -2_147_483_648),
            ("9223372036854775807", -1),
            ("9223372036854775808", 0),
            ("99999999999999999999", 1_661_992_959),
            ("7766279631452241919", 1_661_992_959),
            ("6442450943", 2_147_483_647),
        ] {
            assert_eq!(grid_value(raw), Some(read), "{raw:?}");
        }
        for refused in [" 2", "2 ", "1_0", "x", "1e1", "2e0", "2.0.0", "--2", "+-2", "2,0"] {
            assert_eq!(grid_value(refused), None, "{refused:?}");
        }
        let valid = |raw: &str| CellSpec::new(Some(raw), VMerge::None).span_valid();
        assert_eq!(CellSpec::new(Some("0"), VMerge::None).span, 1);
        assert_eq!(CellSpec::new(Some("2.0"), VMerge::None).span, 2);
        assert_eq!(
            CellSpec::new(Some("99999999999999999999"), VMerge::None).span,
            1_661_992_959
        );
        for opened in ["2", "+2", "2.0", "2.9", "99999999999999999999", "2147483000", "1000"] {
            assert!(valid(opened), "{opened:?}");
        }
        // below 1 (Word opens 0 and re-grids the row), or refused by Word
        for not_valid in ["0", "0.5", "", "-1", "-2.0", " 2", "1e1", "2147483647", "4294967295"] {
            assert!(!valid(not_valid), "{not_valid:?}");
        }
        assert!(CellSpec::new(None, VMerge::None).span_valid());
        assert_eq!(
            RowSpec::new(Some("-1"), Some("-1"), Vec::new()),
            RowSpec::new(None, None, Vec::new())
        );
        assert_eq!(RowSpec::new(Some("1.9"), Some("2.0"), Vec::new()).before, 1);
        assert_eq!(RowSpec::new(Some("1.9"), Some("2.0"), Vec::new()).after, 2);
    }

    /// Word's reading spelled out otherwise: the grammar by hand, the whole part as a number
    /// of any size reduced modulo 2^32 (u128 digit by digit), then the sign.
    fn reference(raw: &str) -> Option<i64> {
        let mut chars = raw.chars().peekable();
        let negative = match chars.peek() {
            Some('-') => {
                chars.next();
                true
            }
            Some('+') => {
                chars.next();
                false
            }
            _ => false,
        };
        let mut whole: u128 = 0;
        let mut dot = false;
        for c in chars {
            match c {
                '.' if !dot => dot = true,
                '0'..='9' if dot => {}
                '0'..='9' => whole = (whole * 10 + u128::from(c as u8 - b'0')) % (1 << 32),
                _ => return None,
            }
        }
        let signed = if negative { (1u128 << 32) - whole } else { whole } % (1 << 32);
        let signed = i64::try_from(signed).ok()?;
        Some(if signed >= 1 << 31 { signed - (1 << 32) } else { signed })
    }

    #[test]
    fn generated_values_are_read_as_the_reference_reads_them() {
        let mut state = 7u64;
        let mut next = |n: usize| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        const ALPHABET: &[u8] = b"0123456789999+-.. e_x";
        for _ in 0..50_000 {
            let len = next(26);
            let raw: String = (0..len).map(|_| char::from(ALPHABET[next(ALPHABET.len())])).collect();
            assert_eq!(grid_value(&raw), reference(&raw), "{raw:?}");
            let span = CellSpec::new(Some(&raw), VMerge::None);
            assert_eq!(
                span.span_valid(),
                reference(&raw).is_some_and(|v| (1..=GRID_VALUE_MAX).contains(&v)),
                "{raw:?}"
            );
            assert!(span.span >= 1);
        }
    }

    #[test]
    fn a_span_of_millions_of_columns_is_placed_without_a_column_each() {
        let cell = |span: &str, vmerge| CellSpec::new(Some(span), vmerge);
        let rows = [
            RowSpec::new(None, None, vec![cell("99999999999999999999", VMerge::Restart)]),
            RowSpec::new(Some("1000000000"), None, vec![cell("1", VMerge::Continue)]),
            RowSpec::new(Some("1"), None, vec![cell("2", VMerge::Restart)]),
            RowSpec::new(
                None,
                None,
                vec![cell("2", VMerge::Continue), cell("3", VMerge::Continue)],
            ),
            RowSpec::new(Some("3"), None, vec![cell("1", VMerge::Continue)]),
        ];
        let grid = Grid::build(&rows, 3);
        assert_eq!(grid.rows[1].cells[0].origin, (0, 0), "inside the wide cell");
        assert_eq!(grid.rows[3].cells[0].origin, (0, 0), "column 0 is the wide cell's");
        assert_eq!(grid.rows[3].cells[1].origin, (2, 0), "column 2 is row 2's cell's");
        assert_eq!(
            grid.rows[4].cells[0].origin,
            (0, 0),
            "column 3 is the wide cell's again"
        );
        assert_eq!(grid.rows[0].coverage, Coverage::Over(1_661_992_956));
    }

    #[test]
    fn a_grid_places_cells_follows_merges_and_measures_rows_against_the_grid() {
        let cell = |span: &str, vmerge| CellSpec::new(Some(span), vmerge);
        let rows = [
            RowSpec::new(Some("1"), None, vec![cell("2", VMerge::Restart)]),
            RowSpec::new(Some("1"), Some("1"), vec![cell("2", VMerge::Continue)]),
            RowSpec::new(None, None, vec![cell("1", VMerge::Continue), cell("3", VMerge::None)]),
        ];
        let grid = Grid::build(&rows, 3);
        assert_eq!(
            grid.rows[0].cells[0],
            GridCell {
                first: 1,
                span: 2,
                origin: (0, 0)
            }
        );
        assert_eq!(grid.rows[1].cells[0].origin, (0, 0));
        assert_eq!(
            grid.rows[2].cells[0].origin,
            (2, 0),
            "nothing merged starts at column 0"
        );
        assert_eq!(grid.rows[0].coverage, Coverage::Exact);
        // `w:gridAfter` is not read for coverage (Word does not read it): the row's cells end
        // at the grid's last column, and the written 1 is said to differ from the 0 it leaves
        assert_eq!(grid.rows[1].coverage, Coverage::Exact);
        assert_eq!((grid.rows[1].after, grid.rows[1].trailing), (1, 0));
        assert!(grid.rows[1].after_differs() && !grid.rows[0].after_differs());
        assert_eq!(grid.rows[2].coverage, Coverage::Over(1));
        assert!(!grid.rows[2].after_differs());
        assert_eq!(grid.width, 4);
        // a short row: its trailing columns are those it leaves, written or not
        let short = Grid::build(&[RowSpec::new(None, None, vec![cell("1", VMerge::None)])], 3);
        assert_eq!(short.rows[0].coverage, Coverage::Exact);
        assert_eq!(short.rows[0].trailing, 2);
        assert!(short.rows[0].after_differs());
        let filled = Grid::build(&[RowSpec::new(None, Some("2"), vec![cell("1", VMerge::None)])], 3);
        assert!(!filled.rows[0].after_differs());
        assert_eq!(grid.origin_at(1, 2), Some((0, 0)));
        assert_eq!(grid.cell_at(1, 0), None);
    }
}
