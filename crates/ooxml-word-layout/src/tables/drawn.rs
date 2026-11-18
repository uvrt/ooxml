//! Owns: what a table's placed rows give the drawing of their cells' floating drawings
//! (docx2svg's `_table_drawn` walking a row's cells, `_cell_anchors`): each cell line
//! holding floating drawings where the row puts it -- its top, its paragraph's top, the
//! cell's top and how far its `w:vAlign` moves it, the cell's edges -- and each cell line's
//! segments beside the drawings it was broken across ([`MeasuredTable::line_extra`]).
//!
//! Must not: place a drawing (the paint does, from [`AnchoredLine`]), or lay a cell out.

use super::anchors::CellEdges;
use super::cells::{CellBlock, CellFlow, Content};
use super::measured::MeasuredTable;
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use crate::rebreak::LineExtra;
use crate::table::PlacedRow;
use std::sync::Arc;

/// A cell line holding floating drawings, as a placed row draws it.
#[derive(Debug, Clone)]
pub struct AnchoredLine<'t> {
    /// Its paragraph, as [`crate::table::CellLine::block`].
    pub block: usize,
    /// Its line in the paragraph.
    pub line: usize,
    /// The top of its pitch.
    pub top: Px,
    /// Its pitch.
    pub pitch: Px,
    /// Its paragraph's top for its drawings: the first's below the cell's top margin,
    /// another's where the one before ends after its space after.
    pub paragraph_top: Px,
    /// Where the row's content starts: below its top border.
    pub cell_top: Px,
    /// How far its cell's `w:vAlign` moves its lines and drawings down.
    pub offset: Px,
    /// Where the row starts: above its top border.
    pub row_top: Px,
    /// The cell's edges, twips.
    pub edges: CellEdges,
    /// The paragraph's block in its cell: its drawings.
    pub entry: &'t CellBlock,
    /// Its measure.
    pub measured: &'t Arc<MeasuredParagraph>,
}

impl MeasuredTable {
    /// The cell lines of `piece` (placed on a page) holding floating drawings, its
    /// paragraphs' blocks counted from `base`, in drawing order (`_table_drawn`).
    pub fn anchored_lines(&self, piece: &PlacedRow, base: usize) -> Vec<AnchoredLine<'_>> {
        let mut out = Vec::new();
        let Some(row) = self.rows.get(piece.row) else {
            return out;
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
            .filter_map(|m| Some((self.cell(m.row, m.cell)?, m.start, m.end, true)));
        for (cell, start, end, is_carried) in own.chain(carried) {
            if cell.merged() {
                continue;
            }
            let mut offset = Px::ZERO;
            if whole && !is_carried {
                let space = cell.span_height.unwrap_or(row.height) - cell.margin_top - cell.margin_bottom;
                offset = cell.align_offset(space);
            }
            let range = usize::try_from(start).unwrap_or(0)..usize::try_from(end).unwrap_or(0);
            let walk = CellWalk {
                table: self,
                cell,
                content_top,
                offset,
                row_top: piece.top,
                base,
            };
            walk.lines(range, &mut out);
        }
        out
    }

    /// What line `line` of paragraph `block` (as [`crate::table::CellLine::block`],
    /// nested tables' included) carries beyond its pieces: the segments beside drawings
    /// it was broken across.
    pub fn line_extra(&self, block: usize, line: usize) -> Option<&LineExtra> {
        let k = self.extras.binary_search_by_key(&block, |(b, _)| *b).ok()?;
        self.extras.get(k).and_then(|(_, extra)| extra.get(line))
    }
}

/// The lines of a table's paragraphs (nested tables' included) broken beside drawings,
/// by block, in block order: what [`MeasuredTable::line_extra`] reads.
pub(super) fn extras_of(table: &MeasuredTable) -> Vec<(usize, Vec<LineExtra>)> {
    let mut out = Vec::new();
    for entry in table.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.entries) {
        match &entry.content {
            Content::Paragraph(_) if entry.drawings.extra.iter().any(LineExtra::wrapped) => {
                out.push((entry.block, entry.drawings.extra.clone()));
            }
            Content::Table(nested) => {
                out.extend(nested.extras.iter().map(|(b, e)| (entry.block + b, e.clone())));
            }
            Content::Paragraph(_) => {}
        }
    }
    out.sort_by_key(|(b, _)| *b);
    out
}

/// One cell of a placed row walked for its drawings.
struct CellWalk<'t> {
    table: &'t MeasuredTable,
    cell: &'t CellFlow,
    content_top: Px,
    offset: Px,
    row_top: Px,
    base: usize,
}

impl<'t> CellWalk<'t> {
    /// The cell's stacked lines `range` holding drawings, each with where it is.
    fn lines(&self, range: std::ops::Range<usize>, out: &mut Vec<AnchoredLine<'t>>) {
        let cell = self.cell;
        let mut line_y = self.content_top + cell.margin_top + self.offset;
        let mut paragraph_top = line_y;
        let edges = CellEdges::of(&cell.resolved, self.table.page_left, &self.table.grid);
        for k in range {
            let Some(placed) = cell.stack.get(k) else { continue };
            let Some(entry) = cell.entries.get(placed.entry) else {
                continue;
            };
            let top = line_y + placed.above();
            if placed.number == 0 && k > 0 {
                let before = cell.stack.get(k - 1).and_then(|p| cell.entries.get(p.entry));
                if let Some(before) = before {
                    let same = entry.style == before.style;
                    paragraph_top = line_y + Px::twips(if before.contextual && same { 0 } else { before.after });
                }
            }
            line_y = top + placed.pitch() + placed.below();
            let Content::Paragraph(measured) = &entry.content else {
                continue;
            };
            if !entry.drawings.anchors.iter().any(|a| a.line == placed.number) {
                continue;
            }
            out.push(AnchoredLine {
                block: self.base + entry.block,
                line: placed.number,
                top,
                pitch: placed.pitch(),
                paragraph_top,
                cell_top: self.content_top,
                offset: self.offset,
                row_top: self.row_top,
                edges,
                entry,
                measured,
            });
        }
    }
}
