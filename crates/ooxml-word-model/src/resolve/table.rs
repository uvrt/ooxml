//! Owns: a table's resolved formats -- docx2svg's `table.resolve_table`: the table style
//! that applies (the named one, else the default), the table's, each row's and each cell's
//! properties through the style chain and its regions (a region's cell borders taken per
//! side from where the cell is in the region: measured round 58), each cell's place on the
//! grid, its margins, its borders before and after neighbours are settled (the wider wins,
//! a `double` line counting three widths), and the grid widened to a row that reaches past
//! `w:tblGrid` (measured, `make_grid_mismatch_probe.py`).
//!
//! Must not: size columns or rows (layout, L3).

use super::cascade::{banded, Sources};
use super::origin::{Holder, Level, Origin, Trace};
use crate::docx::tables::past_widest;
use crate::model::Table;
use crate::styles::{Style, StyleKind};
use crate::value::{joined, key, Border, Key, Props, Text, Value};
use std::sync::Arc;

/// The cell margin Word uses on a left or right side nothing declares, twips (measured).
pub const DEFAULT_SIDE_MARGIN: i64 = 10;
/// The width of a grid column Word adds for a cell past `w:tblGrid` stating no width.
pub const UNSIZED_COLUMN: i64 = 360;
/// The sides of a cell, in docx2svg's order.
pub const SIDES: [&str; 4] = ["top", "left", "bottom", "right"];

/// Which of a table style's parts a merge reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attribute {
    Table,
    Row,
    Cell,
}

/// A border on an edge and which side declared it (`true`: the right or lower cell).
pub type Edge = (Option<Border>, bool);

/// One cell, resolved and placed on the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCell {
    /// Its row.
    pub row: usize,
    /// Its index among its row's cells.
    pub index: usize,
    /// The first grid column it covers.
    pub column: i64,
    /// How many it covers.
    pub span: i64,
    /// Its properties: style, regions, then its own.
    pub properties: Props,
    /// `w:vMerge`.
    pub merge: Option<Text>,
    /// Margins, twips, in [`SIDES`] order.
    pub margins: [i64; 4],
    /// Each side's border before neighbours are settled, in [`SIDES`] order.
    pub borders: [Option<Border>; 4],
    /// Whether the cell itself states each side.
    pub own: [bool; 4],
    /// The border drawn on its left edge.
    pub left_edge: Edge,
    /// The border drawn on its right edge.
    pub right_edge: Edge,
}

/// One row, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRow {
    /// Its index.
    pub index: usize,
    /// Its properties.
    pub properties: Props,
    /// Its cells, each shared with what keeps it (a table's layout keeps its cells').
    pub cells: Vec<Arc<ResolvedCell>>,
    /// `w:gridBefore`.
    pub grid_before: i64,
    /// `w:gridAfter`.
    pub grid_after: i64,
}

/// A table's resolved formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTable {
    /// The table's properties.
    pub properties: Props,
    /// Its rows.
    pub rows: Vec<ResolvedRow>,
    /// Whether any level states `w:tblInd`.
    pub indent_stated: bool,
    /// The grid laid out on, twips.
    pub grid: Vec<i64>,
    /// `(row, columns it reaches)` of each row not reaching exactly across `w:tblGrid`.
    pub mismatched: Vec<(usize, i64)>,
    /// `(row, columns it reaches)` of the first row reaching past the widest a table is
    /// fitted to ([`past_widest`]): the grid is then `w:tblGrid` as written, not widened, and
    /// the table is not laid out.
    pub too_wide: Option<(usize, i64)>,
}

fn part(style: &Style, attribute: Attribute) -> &Props {
    match attribute {
        Attribute::Table => &style.table,
        Attribute::Row => &style.table_row,
        Attribute::Cell => &style.table_cell,
    }
}

fn region_part(region: &crate::styles::Region, attribute: Attribute) -> &Props {
    match attribute {
        Attribute::Table => &region.table,
        Attribute::Row => &region.row,
        Attribute::Cell => &region.cell,
    }
}

const ROW_REGIONS: &[&str] = &["band1Horz", "band2Horz", "firstRow", "lastRow"];
const COLUMN_REGIONS: &[&str] = &["band1Vert", "band2Vert", "firstCol", "lastCol"];

/// A region's cell properties for one cell: its border sides taken from the region's edge
/// or inside lines by where the cell is (`outer`, in [`SIDES`] order).
fn region_borders(props: &Props, outer: [bool; 4]) -> Props {
    if !props.keys().any(|k| k.starts_with("tcBorders.")) {
        return props.clone();
    }
    let border_keys = ["top", "left", "bottom", "right", "insideH", "insideV"].map(|s| format!("tcBorders.{s}"));
    let mut out: Props = props
        .iter()
        .filter(|(k, _)| !border_keys.iter().any(|b| b == &***k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (side, on_edge) in SIDES.iter().zip(outer) {
        let inside = if matches!(*side, "top" | "bottom") {
            "insideH"
        } else {
            "insideV"
        };
        let from = format!("tcBorders.{}", if on_edge { side } else { inside });
        if let Some(v) = props.get(from.as_str()) {
            out.insert(joined("tcBorders", side), v.clone());
        }
    }
    out
}

/// The cells' named regions, for the edges of each region.
struct Regions<'a> {
    named: &'a [Vec<Vec<Text>>],
}

impl Regions<'_> {
    fn has(&self, r: i64, c: Option<i64>, condition: &str) -> bool {
        let Some(row) = usize::try_from(r).ok().and_then(|r| self.named.get(r)) else {
            return false;
        };
        match c {
            None => row.iter().any(|cell| cell.iter().any(|x| &**x == condition)),
            Some(c) => usize::try_from(c)
                .ok()
                .and_then(|c| row.get(c))
                .is_some_and(|cell| cell.iter().any(|x| &**x == condition)),
        }
    }

    fn outer(&self, r: usize, c: usize, condition: &str) -> [bool; 4] {
        let (ri, ci) = (i64::try_from(r).unwrap_or(0), i64::try_from(c).unwrap_or(0));
        let last_cell = self.named.get(r).map_or(0, |row| row.len().saturating_sub(1));
        if ROW_REGIONS.contains(&condition) {
            return [
                !self.has(ri - 1, None, condition),
                c == 0,
                !self.has(ri + 1, None, condition),
                c == last_cell,
            ];
        }
        if COLUMN_REGIONS.contains(&condition) {
            let last_row = self.named.len().saturating_sub(1);
            return [
                r == 0,
                !self.has(ri, Some(ci - 1), condition),
                r == last_row,
                !self.has(ri, Some(ci + 1), condition),
            ];
        }
        [true; 4]
    }
}

fn merged(
    chain: &[Arc<Style>],
    attribute: Attribute,
    direct: &Props,
    conditions: &[Text],
    at: Option<(&Regions<'_>, usize, usize)>,
    regions: bool,
) -> Props {
    merged_traced::<()>(chain, attribute, direct, conditions, at, regions).0
}

impl Attribute {
    fn holder(self) -> Holder {
        match self {
            Attribute::Table => Holder::Table,
            Attribute::Row => Holder::Row,
            Attribute::Cell => Holder::Cell,
        }
    }
}

/// A region's cell properties for one cell ([`region_borders`]), with each key's origin:
/// a side read from an inside line says so.
fn region_piece<T: Trace>(props: &Props, outer: Option<[bool; 4]>, origin: impl Fn() -> Origin) -> (Props, T) {
    let mut trace = T::default();
    let Some(outer) = outer else {
        trace.declare(props, &origin);
        return (props.clone(), trace);
    };
    let piece = region_borders(props, outer);
    if !T::RECORDS {
        return (piece, trace);
    }
    let origin = origin();
    trace.declare(&piece, || origin);
    for (side, on_edge) in SIDES.iter().zip(outer) {
        if !on_edge && piece.contains_key(joined("tcBorders", side).as_ref()) {
            trace.set(&joined("tcBorders", side), Origin { inside: true, ..origin });
        }
    }
    (piece, trace)
}

fn merged_traced<T: Trace>(
    chain: &[Arc<Style>],
    attribute: Attribute,
    direct: &Props,
    conditions: &[Text],
    at: Option<(&Regions<'_>, usize, usize)>,
    regions: bool,
) -> (Props, T) {
    let conditions = banded(chain, conditions);
    let holder = attribute.holder();
    let mut out = Props::new();
    let mut trace = T::default();
    for style in chain {
        let own = part(style, attribute);
        out.extend(own.iter().map(|(k, v)| (k.clone(), v.clone())));
        trace.declare(own, || Origin::style(Level::TableStyle, style, holder));
        if !regions {
            continue;
        }
        for condition in &conditions {
            let Some(region) = style.conditional.get(condition) else {
                continue;
            };
            let props = region_part(region, attribute);
            let outer = at.map(|(r, row, cell)| r.outer(row, cell, condition));
            let origin = || Origin::region(Level::TableStyle, style, condition, holder);
            let (piece, from) = region_piece::<T>(props, outer, origin);
            trace.absorb(&from, &piece);
            out.extend(piece);
        }
    }
    out.extend(direct.iter().map(|(k, v)| (k.clone(), v.clone())));
    trace.declare(direct, || Origin::at(Level::Direct, holder));
    (out, trace)
}

fn margin(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Width(w, kind)) if &**kind == "dxa" => *w,
        _ => 0,
    }
}

fn int(props: &Props, name: &str) -> Option<i64> {
    props.get(name).and_then(Value::int)
}

/// A border's width in whole twips, truncated (a `double` line counts three).
pub fn border_twips(border: Option<&Border>) -> i64 {
    match border {
        Some(b) if drawn(Some(b)) => b.size * 20 / 8 * if b.style.as_deref() == Some("double") { 3 } else { 1 },
        _ => 0,
    }
}

/// Whether a border is drawn.
pub fn drawn(border: Option<&Border>) -> bool {
    border.is_some_and(|b| !matches!(b.style.as_deref(), None | Some("nil" | "none")))
}

/// The border drawn where two cells meet: the wider, else the left (upper) one's.
pub fn edge_winner(first: (Option<&Border>, bool), second: (Option<&Border>, bool)) -> Edge {
    let ((a, _), (b, b_own)) = (first, second);
    if second_wins(a, b) {
        (b.cloned(), b_own)
    } else {
        (a.cloned(), false)
    }
}

/// Whether, where two cells meet, the right (lower) one's border `b` is drawn rather than
/// the left (upper) one's `a` ([`edge_winner`]).
#[must_use]
pub fn second_wins(a: Option<&Border>, b: Option<&Border>) -> bool {
    drawn(b) && (!drawn(a) || border_twips(b) > border_twips(a))
}

fn border_of(props: &Props, name: &str) -> Option<Border> {
    match props.get(name) {
        Some(Value::Border(b) | Value::RunBorder(b)) => Some(b.clone()),
        _ => None,
    }
}

/// Which sides of a cell are on the table's edge, in [`SIDES`] order.
fn on_edges(cell: &ResolvedCell, rows: usize, columns: i64) -> [bool; 4] {
    [
        cell.row == 0,
        cell.column == 0,
        cell.row + 1 == rows,
        cell.column + cell.span >= columns,
    ]
}

/// The key a cell's border side `k` ([`SIDES`]) was read from, in a table of `rows` rows
/// on `columns` grid columns: the cell's own `tcBorders.<side>` where it has one (`true`),
/// else the table's `tblBorders` line it takes -- its edge's, or the inside line.
#[must_use]
pub fn border_key(cell: &ResolvedCell, k: usize, rows: usize, columns: i64) -> (bool, Key) {
    let side = SIDES.get(k).copied().unwrap_or("top");
    let own = joined("tcBorders", side);
    if cell.properties.contains_key(own.as_ref()) {
        return (true, own);
    }
    let on_edge = on_edges(cell, rows, columns).get(k).copied().unwrap_or(false);
    let inside = if matches!(side, "top" | "bottom") {
        "insideH"
    } else {
        "insideV"
    };
    (false, joined("tblBorders", if on_edge { side } else { inside }))
}

fn cell_borders(table: &Props, cell: &mut ResolvedCell, rows: usize, columns: i64) {
    for k in 0..SIDES.len() {
        let (own, from) = border_key(cell, k, rows, columns);
        let value = border_of(if own { &cell.properties } else { table }, &from);
        if let (Some(slot), Some(o)) = (cell.borders.get_mut(k), cell.own.get_mut(k)) {
            *slot = value;
            *o = own;
        }
    }
}

fn reach(row: &ResolvedRow) -> i64 {
    row.grid_before + row.cells.iter().map(|c| c.span).sum::<i64>() + row.grid_after
}

/// A table's grid as fitted to its rows ([`fitted_grid`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FittedGrid {
    /// The grid laid out on, twips.
    pub grid: Vec<i64>,
    /// `(row, columns it reaches)` of each row not reaching exactly across `w:tblGrid`.
    pub mismatched: Vec<(usize, i64)>,
    /// `(row, columns it reaches)` of the first row reaching past the widest a table is
    /// fitted to ([`past_widest`]); `grid` is then `w:tblGrid` as written.
    pub too_wide: Option<(usize, i64)>,
}

/// The grid a table is laid out on (`w:tblGrid` widened to a row reaching past it), and
/// the rows not reaching exactly across it. A row reaching past the widest a table is
/// fitted to ([`past_widest`]) leaves the grid as written, said in `too_wide`.
pub fn fitted_grid(grid: &[i64], rows: &[ResolvedRow]) -> FittedGrid {
    let mut widths = grid.to_vec();
    let count = i64::try_from(widths.len()).unwrap_or(i64::MAX);
    let mismatched: Vec<(usize, i64)> = rows
        .iter()
        .filter(|r| !r.cells.is_empty() && reach(r) != count)
        .map(|r| (r.index, reach(r)))
        .collect();
    let too_wide = mismatched
        .iter()
        .find(|(_, r)| past_widest(widths.len(), usize::try_from(*r).unwrap_or(usize::MAX)))
        .copied();
    if too_wide.is_some() || !mismatched.iter().any(|(_, r)| *r > count) {
        return FittedGrid {
            grid: widths,
            mismatched,
            too_wide,
        };
    }
    let len = |w: &Vec<i64>| i64::try_from(w.len()).unwrap_or(i64::MAX);
    for row in rows {
        for cell in &row.cells {
            let end = cell.column + cell.span;
            if end <= len(&widths) {
                continue;
            }
            let added = end - len(&widths).max(cell.column);
            let covered: i64 = widths.iter().skip(usize::try_from(cell.column).unwrap_or(0)).sum();
            let rest = match cell.properties.get("tcW") {
                Some(Value::Width(w, kind)) if &**kind == "dxa" && *w > covered => w - covered,
                _ => UNSIZED_COLUMN * added,
            };
            let pad = (cell.column - len(&widths)).max(0);
            widths.extend(std::iter::repeat_n(UNSIZED_COLUMN, usize::try_from(pad).unwrap_or(0)));
            if added > 0 {
                widths.extend((0..added).map(|k| rest.div_euclid(added) + i64::from(k < rest.rem_euclid(added))));
            }
        }
        let missing = reach(row) - len(&widths);
        if missing > 0 {
            widths.extend(std::iter::repeat_n(
                UNSIZED_COLUMN,
                usize::try_from(missing).unwrap_or(0),
            ));
        }
    }
    FittedGrid {
        grid: widths,
        mismatched,
        too_wide: None,
    }
}

fn cell_margins(props: &Props, table: &Props) -> [i64; 4] {
    SIDES.map(|side| {
        let own = props.get(format!("tcMar.{side}").as_str());
        let from_table = table.get(format!("tblCellMar.{side}").as_str());
        match (own, from_table) {
            (None, None) => {
                if matches!(side, "left" | "right") {
                    DEFAULT_SIDE_MARGIN
                } else {
                    0
                }
            }
            (Some(v), _) | (None, Some(v)) => margin(Some(v)),
        }
    })
}

fn settle_edges(row: &mut ResolvedRow) {
    for k in 0..row.cells.len() {
        let (before, rest) = row.cells.split_at_mut(k);
        let Some(cell) = rest.first_mut().map(Arc::make_mut) else {
            continue;
        };
        let left_border = cell.borders.get(1).and_then(Option::as_ref);
        let left_own = cell.own.get(1).copied().unwrap_or(false);
        match before.last_mut().map(Arc::make_mut) {
            None => cell.left_edge = (left_border.cloned(), left_own),
            Some(left) => {
                let edge = edge_winner(
                    (
                        left.borders.get(3).and_then(Option::as_ref),
                        left.own.get(3).copied().unwrap_or(false),
                    ),
                    (left_border, left_own),
                );
                left.right_edge = edge.clone();
                cell.left_edge = edge;
            }
        }
    }
    if let Some(last) = row.cells.last_mut().map(Arc::make_mut) {
        last.right_edge = (last.borders.get(3).cloned().flatten(), false);
    }
}

impl Sources<'_> {
    /// The table style that applies to a table: the named one, else the default.
    pub fn table_style_of(&self, table: &Table) -> Option<Text> {
        let sheet = self.styles?;
        if let Some(id) = table.style_id.as_deref().filter(|s| !s.is_empty()) {
            if sheet.get(id).is_some_and(|s| s.kind == StyleKind::Table) {
                return Some(key(id));
            }
        }
        sheet.default_of(StyleKind::Table).map(|s| s.style_id.clone())
    }
}

fn resolve_row<T: Trace>(
    chain: &[Arc<Style>],
    table: &Table,
    r: usize,
    named: &[Vec<Vec<Text>>],
    properties: &Props,
) -> (ResolvedRow, Vec<T>) {
    let regions = Regions { named };
    let empty = Props::new();
    let row_direct = table.row_properties.get(r).unwrap_or(&empty);
    let row_conditions: Vec<Text> = table
        .conditions
        .get(r)
        .and_then(|row| row.first())
        .map(|c| c.to_vec())
        .unwrap_or_default();
    let row_properties = merged(chain, Attribute::Row, row_direct, &row_conditions, None, true);
    let grid_before = int(&row_properties, "gridBefore").unwrap_or(0).max(0);
    let grid_after = int(&row_properties, "gridAfter").unwrap_or(0).max(0);
    let mut column = grid_before;
    let mut cells = Vec::new();
    let mut traces = Vec::new();
    let count = table.rows.get(r).map_or(0, |row| row.len());
    for c in 0..count {
        let direct = table
            .cell_properties
            .get(r)
            .and_then(|row| row.get(c))
            .unwrap_or(&empty);
        let conditions = named.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default();
        let at = (!conditions.is_empty()).then_some((&regions, r, c));
        let (props, trace) = merged_traced::<T>(chain, Attribute::Cell, direct, &conditions, at, true);
        traces.push(trace);
        let span = int(&props, "gridSpan").filter(|s| *s != 0).unwrap_or(1).max(1);
        cells.push(Arc::new(ResolvedCell {
            row: r,
            index: c,
            column,
            span,
            merge: props.get("vMerge").and_then(Value::str).map(key),
            margins: cell_margins(&props, properties),
            properties: props,
            borders: Default::default(),
            own: [false; 4],
            left_edge: (None, false),
            right_edge: (None, false),
        }));
        column += span;
    }
    let row = ResolvedRow {
        index: r,
        properties: row_properties,
        cells,
        grid_before,
        grid_after,
    };
    (row, traces)
}

/// Where a table's resolved values came from: its properties' and each cell's
/// ([`ResolvedCell::properties`]), by row and cell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableOrigins<T> {
    /// The table's properties'.
    pub table: T,
    /// Each cell's, by row and index.
    pub cells: Vec<Vec<T>>,
}

/// A table's resolved formats.
pub fn resolve_table(src: &Sources<'_>, table: &Table) -> ResolvedTable {
    resolve_table_traced::<()>(src, table).0
}

/// [`resolve_table`], with where each value came from.
pub fn resolve_table_traced<T: Trace>(src: &Sources<'_>, table: &Table) -> (ResolvedTable, TableOrigins<T>) {
    let style_id = src.table_style_of(table);
    let chain = src
        .styles
        .map(|s| s.chain(style_id.as_deref(), StyleKind::Table))
        .unwrap_or_default();
    let (properties, table_trace) = merged_traced::<T>(&chain, Attribute::Table, &table.properties, &[], None, false);
    let indent_stated = table.properties.contains_key("tblInd") || chain.iter().any(|s| s.table.contains_key("tblInd"));
    let named: Vec<Vec<Vec<Text>>> = table
        .conditions
        .iter()
        .map(|row| row.iter().map(|cell| banded(&chain, cell)).collect())
        .collect();
    let (mut rows, cell_traces): (Vec<ResolvedRow>, Vec<Vec<T>>) = (0..table.rows.len())
        .map(|r| resolve_row::<T>(&chain, table, r, &named, &properties))
        .unzip();
    let FittedGrid {
        grid,
        mismatched,
        too_wide,
    } = fitted_grid(&table.grid, &rows);
    let columns = if grid.is_empty() {
        rows.iter()
            .flat_map(|r| r.cells.iter().map(|c| c.column + c.span))
            .max()
            .unwrap_or(0)
    } else {
        i64::try_from(grid.len()).unwrap_or(i64::MAX)
    };
    let count = rows.len();
    for row in &mut rows {
        for cell in &mut row.cells {
            cell_borders(&properties, Arc::make_mut(cell), count, columns);
        }
        settle_edges(row);
    }
    let resolved = ResolvedTable {
        properties,
        rows,
        indent_stated,
        grid,
        mismatched,
        too_wide,
    };
    let origins = TableOrigins {
        table: table_trace,
        cells: cell_traces,
    };
    (resolved, origins)
}
