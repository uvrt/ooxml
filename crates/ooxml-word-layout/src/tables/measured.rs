//! Owns: a table as measured for the page stack ([`MeasuredTable`], docx2svg's
//! `table.TableFlow`) and what the paginator and the pages ask of it
//! ([`crate::table::TableFlow`]): its rows' keeps (`keeps_with_next`: the first paragraph
//! of a row's first cell, through nested tables), its header rows, its edges, one page's
//! part of it (`super::place`), and the lines a placed row draws (`layout._table_drawn`'s
//! line placement).
//!
//! Must not: measure (`super::build` does) or draw borders and shading (lane L3d).

use super::cells::{CellFlow, Content, RowFlow};
use super::place::{place_table, MergeDown};
use crate::naming::ItemName;
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use crate::table::{
    CellLine, FloatingTable, PlacedRow, StackedBlock, StackedEnd, TableFlow, TablePage, TableRequest, Unsupported,
};
use ooxml_base::Q;
use ooxml_word_model::ids::{Anchor, BlockKey, Sub};
use ooxml_word_model::value::Text;
use std::sync::Arc;

/// What the approximate rules laid out of a table, as docx2svg reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approximated {
    /// Why: `table` (its widths), `frame` (a frame in a cell).
    pub reason: &'static str,
    /// What was done.
    pub message: String,
    /// What it is about: the table, or a paragraph of a cell.
    pub at: ItemName,
}

/// How far below a nested table's top its placement may reach: a page that never ends.
const FAR: i64 = 10_000_000;

/// Automatic text on a cell's fill darker than this Rec. 601 luma (0-255, times 1,000) is
/// drawn white (`AUTOMATIC_WHITE_BELOW_CELL`, measured).
const WHITE_BELOW: i64 = 60_000;

/// Whether a cell's shading is a fill dark enough that Word draws automatic text white.
fn white_text(props: &ooxml_word_model::value::Props) -> bool {
    let Some(ooxml_word_model::value::Value::Shading(s)) = props.get("shd") else {
        return false;
    };
    if s.kind.as_deref() == Some("nil") {
        return false;
    }
    let Some(fill) = s
        .fill
        .as_deref()
        .filter(|f| f.len() == 6 && f.chars().all(|c| c.is_ascii_hexdigit()))
    else {
        return false;
    };
    let channel = |k: usize| {
        fill.get(k..k + 2)
            .and_then(|h| i64::from_str_radix(h, 16).ok())
            .unwrap_or(0)
    };
    299 * channel(0) + 587 * channel(2) + 114 * channel(4) < WHITE_BELOW
}

/// A table measured for the page stack.
#[derive(Debug, Clone)]
pub struct MeasuredTable {
    /// Its block's key (`None`: a table no story's index holds).
    pub key: Option<BlockKey>,
    /// Where no story's index holds it, the path its cells are named below (a table of a
    /// part no index holds).
    pub unkeyed_cells: Option<Text>,
    /// Each cell's place in the table as written (the model's [`Table::cell_places`]).
    ///
    /// [`Table::cell_places`]: ooxml_word_model::model::Table::cell_places
    pub cell_places: ooxml_word_model::model::CellPlaces,
    /// Its rows.
    pub rows: Vec<RowFlow>,
    /// The border below its last row.
    pub bottom_border: Px,
    /// Every grid line, twips from its column's left edge.
    pub grid: Vec<Q>,
    /// Its column's left edge, twips from the page's.
    pub page_left: Q,
    /// The document is in compatibility mode 15 or later.
    pub mode15: bool,
    /// How many rows at its top repeat as headers.
    pub header_rows: usize,
    /// Per row, whether it keeps with the next.
    pub keeps: Vec<bool>,
    /// Its merges down over more than one row.
    pub merges: Vec<MergeDown>,
    /// Laid out by the approximate rules: why the measured ones could not.
    pub approximation: Option<String>,
    /// What the approximate rules laid out inside it.
    pub approximated: Vec<Approximated>,
    /// Its paragraphs' measures by block (nested tables' included); `None` for a block
    /// that is not a paragraph of it.
    pub measures: Vec<Option<Arc<MeasuredParagraph>>>,
    /// Where it is positioned, where it floats.
    pub floating: Option<FloatingTable>,
    /// The lines of its paragraphs broken beside drawings in their cells, by block
    /// ([`MeasuredTable::line_extra`]).
    pub extras: Vec<(usize, Vec<crate::rebreak::LineExtra>)>,
    /// How far right of its column's left edge it reaches as a room measures it, twips
    /// (`table_fits`: its last grid line with its right border's outer part in mode 15,
    /// less its last cell's right offset below).
    pub fit_right: Q,
}

impl MeasuredTable {
    /// What the cell `cell` of row `row` (the final view's) is drawn for: the table's cell
    /// at its place as written; its path for a table no story's index holds.
    pub fn cell_name(&self, row: usize, cell: usize) -> ItemName {
        let anchor = (|| {
            let &(r, c) = self.cell_places.get(row)?.get(cell)?;
            Some(Anchor {
                block: self.key?,
                sub: Sub::Cell(u16::try_from(r).ok()?, u16::try_from(c).ok()?),
            })
        })();
        match anchor {
            Some(anchor) => ItemName::Anchored(anchor),
            None => ItemName::Path(
                (|| {
                    let table = self.unkeyed_cells.as_deref()?;
                    let &(r, c) = self.cell_places.get(row)?.get(cell)?;
                    Some(Text::from(ooxml_word_model::model::cell_path(table, r, c).as_str()))
                })()
                .unwrap_or_default(),
            ),
        }
    }

    /// Row `row`'s cell `cell`.
    pub fn cell(&self, row: usize, cell: usize) -> Option<&CellFlow> {
        self.rows.get(row).and_then(|r| r.cells.get(cell))
    }

    /// The edge below row `r` (the table's bottom border below the last).
    pub fn edge_below(&self, r: usize) -> Px {
        self.rows.get(r + 1).map_or(self.bottom_border, |row| row.top_border)
    }

    /// Whether row `row` keeps with the next (`keeps_with_next`).
    pub fn row_keeps(&self, row: usize) -> bool {
        self.keeps.get(row).copied().unwrap_or(false)
    }

    /// The paragraph a row's `keepNext` is read from keeps with the next: its first cell's
    /// first block, or a nested table's first row's.
    pub fn leads_kept(row: &RowFlow) -> bool {
        let Some(lead) = row.cells.first().and_then(|c| c.entries.first()) else {
            return false;
        };
        match &lead.content {
            Content::Paragraph(_) => lead.keep_next,
            Content::Table(nested) => nested.rows.first().is_some_and(MeasuredTable::leads_kept),
        }
    }

    /// The lines placed row `piece` draws, its paragraphs' blocks counted from `base`.
    fn draw(&self, piece: &PlacedRow, base: usize, out: &mut Vec<CellLine>) {
        let Some(row) = self.rows.get(piece.row) else { return };
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
            let mut line_y = content_top + cell.margin_top + offset;
            let range = usize::try_from(start).unwrap_or(0)..usize::try_from(end).unwrap_or(0);
            for placed in cell.stack.get(range).unwrap_or_default() {
                let Some(entry) = cell.entries.get(placed.entry) else {
                    continue;
                };
                let top = line_y + placed.above();
                line_y = top + placed.pitch() + placed.below();
                match &entry.content {
                    Content::Table(nested) => nested.draw_whole(top, base + entry.block, out),
                    Content::Paragraph(_) => out.push(CellLine {
                        block: base + entry.block,
                        line: placed.number,
                        top,
                        pitch: placed.pitch(),
                        drawn: !entry.undrawn(placed.number),
                        space_before: placed.box_before(),
                        space_after: placed.box_after(),
                        left: cell.cell_box.text_left,
                        white_text: white_text(&cell.resolved.properties),
                    }),
                }
            }
        }
    }

    /// A nested table's lines, placed whole from `top`.
    fn draw_whole(&self, top: Px, base: usize, out: &mut Vec<CellLine>) {
        let laid = place_table(
            self,
            &TableRequest {
                y: top,
                bottom: top + Px::int(FAR),
                row: 0,
                part: Vec::new(),
                first_on_page: false,
                mode15: self.mode15,
                split_kept: false,
                stop: None,
            },
        );
        for piece in &laid.rows {
            self.draw(piece, base, out);
        }
    }

    /// Its height placed whole on a page that never ends, from 0 (a nested table's line).
    pub fn whole_height(&self) -> Px {
        place_table(
            self,
            &TableRequest {
                y: Px::ZERO,
                bottom: Px::int(FAR),
                row: 0,
                part: Vec::new(),
                first_on_page: false,
                mode15: self.mode15,
                split_kept: false,
                stop: None,
            },
        )
        .end
    }
}

impl TableFlow for MeasuredTable {
    fn floating(&self) -> Option<&FloatingTable> {
        self.floating.as_ref()
    }

    fn fits(&self, width: Q) -> bool {
        self.fit_right <= width
    }

    fn rows(&self) -> usize {
        self.rows.len()
    }

    fn keeps_with_next(&self, row: usize) -> bool {
        self.row_keeps(row)
    }

    fn header_rows(&self) -> usize {
        self.header_rows
    }

    fn opens_with_style(&self, style: Option<&str>) -> bool {
        self.rows
            .first()
            .and_then(|r| r.cells.first())
            .and_then(|c| c.entries.first())
            .is_some_and(|e| e.style.as_deref() == style)
    }

    fn top_border(&self) -> Px {
        self.rows.first().map_or(Px::ZERO, |r| r.top_border)
    }

    fn bottom_border(&self) -> Px {
        self.bottom_border
    }

    fn place(&self, request: &TableRequest) -> Result<TablePage, Unsupported> {
        Ok(place_table(self, request))
    }

    fn lines(&self, row: &PlacedRow) -> Vec<CellLine> {
        let mut out = Vec::new();
        self.draw(row, 0, &mut out);
        out
    }

    fn stacks(&self, row: usize) -> Vec<Vec<StackedBlock>> {
        let Some(row) = self.rows.get(row) else {
            return Vec::new();
        };
        row.cells
            .iter()
            .map(|cell| {
                cell.stack
                    .iter()
                    .map(|line| {
                        let entry = cell.entries.get(line.entry)?;
                        (!entry.undrawn(line.number)).then_some(entry.block)
                    })
                    .collect()
            })
            .collect()
    }

    fn paragraph(&self, block: usize) -> Option<Arc<MeasuredParagraph>> {
        self.measures.get(block).cloned().flatten()
    }

    fn line_ends(&self, placed: &PlacedRow) -> Vec<StackedEnd> {
        let Some(row) = self.rows.get(placed.row) else {
            return Vec::new();
        };
        let edge = self.edge_below(placed.row);
        let mut out = Vec::new();
        for (c, cell) in row.cells.iter().enumerate() {
            let top = placed.top + placed.top_border + cell.margin_top;
            let at = |p: &[u32]| p.get(c).map_or(0, |v| usize::try_from(*v).unwrap_or(0));
            let start = at(&placed.start);
            for k in start..at(&placed.end) {
                let paragraph = cell.stack.get(k).and_then(|line| {
                    let entry = cell.entries.get(line.entry)?;
                    matches!(entry.content, Content::Paragraph(_)).then_some((entry.block, line.number))
                });
                out.push(StackedEnd {
                    end: top + cell.slice_height(start, k + 1) + cell.margin_bottom + edge,
                    cell: c,
                    line: k,
                    paragraph,
                });
            }
        }
        out
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
