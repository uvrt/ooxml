//! Owns: a table row's borders and shading on a page as filled rectangles, as Word's PDF
//! draws them (docx2svg's `_row_borders`, measured there): the vertical edges centred on
//! their grid lines rounded against the text column's edge, the edge above the row as a
//! band of the winning borders, the edge below the last row on the page drawn up from the
//! band's bottom, each cell's shading between its grid lines less the inner half of its
//! borders; and the rows of tables nested in its cells, drawn first.
//!
//! Must not: place rows (the table layout has), or draw a cell's text.

use super::spans::{colour, shading};
use super::Rule;
use crate::px::Px;
use crate::table::{PlacedRow, TableRequest};
use crate::tables::{CellFlow, Content, MeasuredTable};
use ooxml_base::Q;
use ooxml_word_model::resolve::{edge_winner, ResolvedCell};
use ooxml_word_model::value::Border;
use std::collections::HashSet;

/// What a table's rows add to a page: its borders (in order) and its shading (in order;
/// each goes in front of the page's rules, as docx2svg inserts it).
#[derive(Debug, Default)]
pub(super) struct TableRules {
    pub(super) borders: Vec<Rule>,
    pub(super) shading: Vec<Rule>,
}

fn twip_px(t: Q) -> Q {
    t * Q::new(5, 24)
}

fn round_half_up(x: Q) -> i64 {
    Px::from(x).round_half_up()
}

fn round_half_away(x: Q) -> i64 {
    if x < Q::from_integer(0) {
        -round_half_up(-x)
    } else {
        round_half_up(x)
    }
}

fn drawn(b: Option<&Border>) -> bool {
    ooxml_word_model::resolve::drawn(b)
}

/// One line of a border: its width in whole twips, truncated to whole px.
fn line_px(b: Option<&Border>) -> i64 {
    match b.filter(|b| drawn(Some(b))) {
        Some(b) => Px::twips(b.size * 20 / 8).floor(),
        None => 0,
    }
}

/// A border's whole width: a `double` one three lines.
fn total_px(b: Option<&Border>) -> i64 {
    line_px(b)
        * if b.and_then(|b| b.style.as_deref()) == Some("double") {
            3
        } else {
            1
        }
}

fn side(cell: &ResolvedCell, k: usize) -> Option<&Border> {
    cell.borders.get(k).and_then(Option::as_ref)
}

/// A table's geometry across: its column's left edge and grid lines, twips.
struct Across<'t> {
    table: &'t MeasuredTable,
}

impl Across<'_> {
    fn columns(&self) -> i64 {
        i64::try_from(self.table.grid.len()).unwrap_or(1) - 1
    }

    /// The px a grid line is drawn centred on (`grid_line_px`).
    fn grid_px(&self, index: i64) -> i64 {
        let line = crate::tables::grid_line(&self.table.grid, index);
        round_half_up(twip_px(self.table.page_left)) + round_half_away(twip_px(line))
    }

    fn vertical_span(&self, b: Option<&Border>, index: i64) -> (i64, i64) {
        let width = total_px(b);
        let x0 = self.grid_px(index) - (width + 1) / 2;
        (x0, x0 + width)
    }

    fn shading_edge(&self, index: i64, b: Option<&Border>, sign: i64) -> i64 {
        let inner = if drawn(b) { crate::tables::inner_part(b) } else { 0 };
        let line = crate::tables::grid_line(&self.table.grid, index);
        round_half_up(twip_px(
            self.table.page_left + line + Q::from_integer(i128::from(sign * inner)),
        ))
    }

    fn right_index(&self, cell: &ResolvedCell) -> i64 {
        (cell.column + cell.span).min(self.columns())
    }

    /// A horizontal line's extent across `cell`: between its vertical lines.
    fn line_extent(&self, cell: &ResolvedCell) -> (i64, i64) {
        let left = cell.left_edge.0.as_ref();
        let right = cell.right_edge.0.as_ref();
        let x0 = if drawn(left) {
            self.vertical_span(left, cell.column).1
        } else {
            self.grid_px(cell.column)
        };
        let r = self.right_index(cell);
        let x1 = if drawn(right) {
            self.vertical_span(right, r).0
        } else {
            self.grid_px(r)
        };
        (x0, x1)
    }
}

/// A border line, or a double's two lines, filling `[x0, x1) x [y0, y1)`.
fn band(out: &mut Vec<Rule>, path: &str, b: &Border, (x0, x1): (i64, i64), (y0, y1): (i64, i64), vertical: bool) {
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let colour = colour(b.color.as_deref()).unwrap_or_else(|| "000000".to_owned());
    let mut push = |x: (i64, i64), y: (i64, i64)| {
        out.push(Rule {
            kind: "table-border",
            path: path.to_owned(),
            x: Px::int(x.0),
            y: Px::int(y.0),
            width: Px::int(x.1 - x.0),
            height: Px::int(y.1 - y.0),
            color: Some(colour.clone()),
        });
    };
    if b.style.as_deref() == Some("double") {
        let w = line_px(Some(b));
        if vertical {
            push((x0, x0 + w), (y0, y1));
            push((x1 - w, x1), (y0, y1));
        } else {
            push((x0, x1), (y0, y0 + w));
            push((x0, x1), (y1 - w, y1));
        }
        return;
    }
    push((x0, x1), (y0, y1));
}

/// The vertical edges drawn with a cell: its left one, and the row's last right one.
fn vertical_edges<'c>(across: &Across<'_>, cell: &'c ResolvedCell, last: bool) -> Vec<(Option<&'c Border>, i64)> {
    let mut out = vec![(cell.left_edge.0.as_ref(), cell.column)];
    if last {
        out.push((cell.right_edge.0.as_ref(), across.right_index(cell)));
    }
    out
}

/// A row's top edge across each of `cells` (`horizontal_edge`), and the vertical edges
/// where they cross it.
fn horizontal_edge(
    out: &mut Vec<Rule>,
    across: &Across<'_>,
    cells: &[&ResolvedCell],
    winners: &[Option<Border>],
    (y0, y1, from_bottom): (i64, i64, bool),
) {
    for (cell, winner) in cells.iter().zip(winners) {
        let Some(w) = winner.as_ref().filter(|w| drawn(Some(w))) else {
            continue;
        };
        let x = across.line_extent(cell);
        let y = if from_bottom {
            (y1 - total_px(Some(w)), y1)
        } else {
            (y0, y0 + total_px(Some(w)))
        };
        band(out, &cell.path, w, x, y, false);
    }
    for (k, cell) in cells.iter().enumerate() {
        for (b, index) in vertical_edges(across, cell, k + 1 == cells.len()) {
            if let (Some(b), true) = (b.filter(|b| drawn(Some(b))), y1 > y0) {
                band(out, &cell.path, b, across.vertical_span(Some(b), index), (y0, y1), true);
            }
        }
    }
}

/// The winner on the horizontal edge above `cell`, `above` the row above's cells.
fn edge_above(cell: &CellFlow, above: &[&CellFlow], carried: &HashSet<i64>) -> Option<Border> {
    let c = &cell.resolved;
    if cell.merged() {
        return if carried.contains(&c.column) {
            side(c, 0).cloned()
        } else {
            None
        };
    }
    let own = c.own.first().copied().unwrap_or(false);
    let mut winner = side(c, 0).cloned();
    for other in above {
        let o = &other.resolved;
        if o.column < c.column + c.span && c.column < o.column + o.span {
            let bottom = (side(o, 2), o.own.get(2).copied().unwrap_or(false));
            winner = edge_winner(bottom, (winner.as_ref(), own)).0;
        }
    }
    winner
}

/// Row `piece`'s borders and shading, its bottom at `bottom`; `last` where it is the last
/// of its table on the page (or the rows below go on elsewhere).
pub(super) fn row_rules(table: &MeasuredTable, piece: &PlacedRow, bottom: Px, last: bool, out: &mut TableRules) {
    let r = piece.row;
    let Some(row) = table.rows.get(r) else { return };
    let across = Across { table };
    let (row_top, content_top) = (piece.top, piece.top + piece.top_border);
    let carried: HashSet<i64> = piece
        .carry
        .iter()
        .filter_map(|m| table.cell(m.row, m.cell))
        .map(|c| c.resolved.column)
        .collect();
    let above: Vec<&CellFlow> = match (r, piece.header || piece.continued) {
        (0, _) | (_, true) => Vec::new(),
        _ => table
            .rows
            .get(r - 1)
            .map(|a| a.cells.iter().collect())
            .unwrap_or_default(),
    };
    let cells: Vec<&ResolvedCell> = row.cells.iter().map(|c| &*c.resolved).collect();
    let tops: Vec<Option<Border>> = row.cells.iter().map(|c| edge_above(c, &above, &carried)).collect();
    let top = (
        row_top.round_half_up(),
        (row_top + row.top_border).round_half_up(),
        false,
    );
    horizontal_edge(&mut out.borders, &across, &cells, &tops, top);
    past_ends(&mut out.borders, &across, &cells, &above, row_top.round_half_up());
    let y0 = content_top.round_half_up();
    let mut y1 = bottom.round_half_up();
    let mut bottom_band = None;
    let winners: Vec<Option<Border>> = cells.iter().map(|c| side(c, 2).cloned()).collect();
    if last {
        let below = table.rows.get(r + 1).map_or(table.bottom_border, |n| n.top_border);
        let band_bottom = (bottom + below).round_half_up();
        let widest = winners
            .iter()
            .filter(|w| drawn(w.as_ref()))
            .map(|w| total_px(w.as_ref()))
            .max();
        let band_top = widest.map_or(bottom.round_half_up(), |w| band_bottom - w);
        y1 = band_top;
        bottom_band = Some((band_top, band_bottom));
    }
    for (k, (cell, winner)) in row.cells.iter().zip(&tops).enumerate() {
        cell_shading(
            out,
            &across,
            piece,
            cell,
            winner.as_ref(),
            (row_top, content_top, y0, y1),
        );
        for (b, index) in vertical_edges(&across, &cell.resolved, k + 1 == cells.len()) {
            if let Some(b) = b.filter(|b| drawn(Some(b))) {
                band(
                    &mut out.borders,
                    &cell.resolved.path,
                    b,
                    across.vertical_span(Some(b), index),
                    (y0, y1),
                    true,
                );
            }
        }
    }
    if let Some((band_top, band_bottom)) = bottom_band {
        horizontal_edge(
            &mut out.borders,
            &across,
            &cells,
            &winners,
            (band_top, band_bottom, true),
        );
    }
}

/// The row above reaching past this one's ends: its bottom border there.
fn past_ends(out: &mut Vec<Rule>, across: &Across<'_>, cells: &[&ResolvedCell], above: &[&CellFlow], y0: i64) {
    let covered: HashSet<i64> = cells.iter().flat_map(|c| c.column..c.column + c.span).collect();
    for other in above {
        let o = &other.resolved;
        let Some(b) = side(o, 2).filter(|b| drawn(Some(b))) else {
            continue;
        };
        let past: Vec<i64> = (o.column..across.right_index(o))
            .filter(|k| !covered.contains(k))
            .collect();
        let (Some(first), Some(last)) = (past.first(), past.last()) else {
            continue;
        };
        let end = last + 1;
        let right = o.right_edge.0.as_ref();
        let x1 = if end == across.right_index(o) && drawn(right) {
            across.vertical_span(right, end).1
        } else {
            across.grid_px(end)
        };
        band(
            out,
            &o.path,
            b,
            (across.grid_px(*first), x1),
            (y0, y0 + total_px(Some(b))),
            false,
        );
    }
}

/// A cell's shading, and the row of it under a line drawn short of its exact width.
fn cell_shading(
    out: &mut TableRules,
    across: &Across<'_>,
    piece: &PlacedRow,
    cell: &CellFlow,
    winner: Option<&Border>,
    (row_top, content_top, y0, y1): (Px, Px, i64, i64),
) {
    let c = &cell.resolved;
    let Some(fill) = shading(c.properties.get("shd")).filter(|_| !cell.merged()) else {
        return;
    };
    let x0 = across.shading_edge(c.column, c.left_edge.0.as_ref(), 1);
    let x1 = across.shading_edge(across.right_index(c), c.right_edge.0.as_ref(), -1);
    let whole = !piece.continued && !piece.split;
    let span_bottom = match cell.span_height {
        Some(h) if whole && !h.is_zero() => (content_top + h).round_half_up(),
        _ => y1,
    };
    let rule = |x: (i64, i64), y: (i64, i64)| Rule {
        kind: "cell-shading",
        path: c.path.to_string(),
        x: Px::int(x.0),
        y: Px::int(y.0),
        width: Px::int(x.1 - x.0),
        height: Px::int(y.1 - y.0),
        color: Some(fill.clone()),
    };
    out.shading.push(rule((x0, x1), (y0, span_bottom)));
    if let Some(w) = winner.filter(|w| drawn(Some(w))) {
        let exact = Px::twips(ooxml_word_model::resolve::border_twips(Some(w))).round_half_up();
        let rest = exact - total_px(Some(w));
        if rest > 0 {
            let (a, b) = across.line_extent(c);
            let under = row_top.round_half_up() + total_px(Some(w));
            out.shading.push(rule((a, b), (under, under + rest)));
        }
    }
}

/// A placed row's tables nested in its cells, placed whole where its lines put them: each
/// with the top it is placed from.
pub(super) fn nested_tables<'t>(table: &'t MeasuredTable, piece: &PlacedRow) -> Vec<(&'t MeasuredTable, Px)> {
    let Some(row) = table.rows.get(piece.row) else {
        return Vec::new();
    };
    let content_top = piece.top + piece.top_border;
    let whole = !piece.continued && !piece.split;
    let own = row.cells.iter().enumerate().map(|(c, cell)| {
        let start = piece.start.get(c).copied().unwrap_or(0);
        let end = piece.end.get(c).copied().unwrap_or(0);
        (cell, start, end, false)
    });
    let carried = piece
        .carry
        .iter()
        .filter_map(|m| Some((table.cell(m.row, m.cell)?, m.start, m.end, true)));
    let mut out = Vec::new();
    for (cell, start, end, is_carried) in own.chain(carried) {
        if cell.merged() {
            continue;
        }
        let mut offset = Px::ZERO;
        if whole && !is_carried {
            let space = cell.span_height.unwrap_or(row.height) - cell.margin_top - cell.margin_bottom;
            offset = cell.align_offset(space);
        }
        let mut line_y = content_top + cell.margin_top + offset;
        let range = usize::try_from(start).unwrap_or(0)..usize::try_from(end).unwrap_or(0);
        for placed in cell.stack.get(range).unwrap_or_default() {
            let top = line_y + placed.above;
            line_y = top + placed.pitch + placed.below;
            if let Some(Content::Table(nested)) = cell.entries.get(placed.entry).map(|e| &e.content) {
                out.push((&**nested, top));
            }
        }
    }
    out
}

/// A nested table placed whole from `top`.
pub(super) fn whole_rows(table: &MeasuredTable, top: Px) -> Vec<PlacedRow> {
    crate::tables::place_table(
        table,
        &TableRequest {
            y: top,
            bottom: top + Px::int(10_000_000),
            row: 0,
            part: Vec::new(),
            first_on_page: false,
            mode15: table.mode15,
            split_kept: false,
            stop: None,
        },
    )
    .rows
}
