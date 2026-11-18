//! Owns: a table's cells and rows as measured -- each cell's blocks (a paragraph's lines
//! at the cell's width, a nested table as one line as tall as it: [`CellBlock`]), its
//! lines stacked as the body stacks paragraphs ([`cell_stack`], docx2svg's
//! `table.cell_stack`), how tall a slice of them is ([`CellFlow::slice_height`]), and a
//! row's height ([`row_height`]).
//!
//! Must not: measure paragraphs or size columns (`super::build` does), or place rows on
//! pages (`super::place`).

use super::grid::CellBox;
use super::measured::MeasuredTable;
use crate::floating::AnchorRef;
use crate::paragraph::MeasuredParagraph;
use crate::px::{CompactPx, Px};
use crate::vertical::paragraph_gap;
use ooxml_word_model::resolve::ResolvedCell;
use ooxml_word_model::value::Text;
use std::sync::Arc;

/// What a cell's block is.
#[derive(Debug, Clone)]
pub enum Content {
    /// A paragraph, measured at the cell's width.
    Paragraph(Arc<MeasuredParagraph>),
    /// A table nested in the cell.
    Table(Arc<MeasuredTable>),
}

/// Where a drawing text wraps around in a cell is (`CellParagraph.positioned`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Positioned {
    /// The drawing: its index in the paragraph's text, its run and its index in the run.
    pub at: (i64, usize, usize),
    /// Its left edge, px from the page's left.
    pub x: Px,
    /// Its top, px from below the row's top border.
    pub y: Px,
}

/// A cell paragraph's floating drawings, as the cell is laid out around them (docx2svg's
/// `CellParagraph.anchors`, `approximate`, `positioned`, `drops`, and its lines broken
/// beside them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellDrawings {
    /// Its floating drawings and the lines they are on (`anchors_of`).
    pub anchors: Vec<AnchorRef>,
    /// `(run, index in the run)` -> why, for each positioned as no probe measured in a
    /// cell: laid out as if text did not wrap around it, placed by its anchor where it
    /// can be, and said (`cell_anchors_approximate`).
    pub approximate: Vec<((usize, usize), String)>,
    /// `(text index, run, index in the run)` -> where a drawing text wraps around in the
    /// cell is, px: from the page's left edge, and from below the row's top border
    /// (`wrap_cell`).
    pub positioned: Vec<Positioned>,
    /// Line -> how far it went down below a drawing it could not go beside, or out of
    /// the band of one text wraps above and below.
    pub drops: Vec<(usize, Px)>,
    /// Per line, what it carries beyond its pieces: the segments beside drawings it was
    /// broken across (empty where none is).
    pub extra: Vec<crate::rebreak::LineExtra>,
}

impl CellDrawings {
    /// How far line `line` went down.
    pub fn drop(&self, line: usize) -> Px {
        self.drops
            .iter()
            .find(|(l, _)| *l == line)
            .map_or(Px::ZERO, |(_, d)| *d)
    }

    /// Why drawing `(run, index)` is approximated, where it is.
    pub fn approximated(&self, at: (usize, usize)) -> Option<&str> {
        self.approximate
            .iter()
            .find(|(k, _)| *k == at)
            .map(|(_, why)| why.as_str())
    }
}

/// One block of a cell, as its stack takes it.
#[derive(Debug, Clone)]
pub struct CellBlock {
    /// Its index among the table's paragraphs in document order, from the table's first
    /// (a nested table's: its first paragraph's).
    pub block: usize,
    /// What it is.
    pub content: Content,
    /// Each line's pitch (a nested table's one line: its height), kept compact.
    pub pitches: Vec<CompactPx>,
    /// Space before and after, twips.
    pub before: i64,
    /// Space after, twips.
    pub after: i64,
    /// What its borders take above its first line.
    pub top_border: Px,
    /// What its borders take below its last line.
    pub border: Px,
    /// `w:contextualSpacing`.
    pub contextual: bool,
    /// Its `w:pStyle` as written.
    pub style: Option<Text>,
    /// The paragraph style that applies (the default where none is named).
    pub effective_style: Option<Text>,
    /// A cell's last paragraph that takes no room (`w:hideMark` over an empty one, or an
    /// empty one after a nested table).
    pub hidden: bool,
    /// `w:keepNext`.
    pub keep_next: bool,
    /// `w:widowControl`, where stated.
    pub widow: Option<bool>,
    /// Its floating drawings, as the cell is laid out around them.
    pub drawings: CellDrawings,
}

impl CellBlock {
    /// Whether line `line` draws nothing (a nested table's one line draws its own).
    pub fn undrawn(&self, line: usize) -> bool {
        match &self.content {
            Content::Paragraph(p) => p.undrawn.contains(&line),
            Content::Table(_) => true,
        }
    }
}

/// One line of a cell as its stack places it (`table.StackedLine`), its lengths kept
/// compact ([`CompactPx`]) and read as [`Px`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackedLine {
    /// Its block, an index into the cell's blocks.
    pub entry: usize,
    /// Its line in the block.
    pub number: usize,
    above: CompactPx,
    pitch: CompactPx,
    below: CompactPx,
    box_before: CompactPx,
    box_after: CompactPx,
}

/// The lengths of a [`StackedLine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackedRoom {
    /// The room above it that is its own: its paragraph's gap and top border, on its
    /// first line.
    pub above: Px,
    /// Its pitch.
    pub pitch: Px,
    /// The room below: its paragraph's bottom border, on its last line.
    pub below: Px,
    /// What of the space before it is in its line box: the kept part of its paragraph's
    /// space before and its top border, on the first line.
    pub box_before: Px,
    /// What of the space after it is in its line box: its paragraph's bottom border and
    /// space after, on the last line.
    pub box_after: Px,
}

impl StackedLine {
    /// Line `number` of block `entry`, taking `room`.
    pub fn new(entry: usize, number: usize, room: StackedRoom) -> StackedLine {
        StackedLine {
            entry,
            number,
            above: room.above.into(),
            pitch: room.pitch.into(),
            below: room.below.into(),
            box_before: room.box_before.into(),
            box_after: room.box_after.into(),
        }
    }

    /// The room above it that is its own: its paragraph's gap and top border, on its
    /// first line.
    pub fn above(&self) -> Px {
        self.above.get()
    }

    /// Its pitch.
    pub fn pitch(&self) -> Px {
        self.pitch.get()
    }

    /// The room below: its paragraph's bottom border, on its last line.
    pub fn below(&self) -> Px {
        self.below.get()
    }

    /// What of the space before it is in its line box.
    pub fn box_before(&self) -> Px {
        self.box_before.get()
    }

    /// What of the space after it is in its line box.
    pub fn box_after(&self) -> Px {
        self.box_after.get()
    }

    /// Its whole height.
    pub fn height(&self) -> Px {
        self.above() + self.pitch() + self.below()
    }
}

/// The blocks of a cell stacked as the body stacks paragraphs, and the space after the
/// last (`cell_stack`): the first paragraph loses its space before where it is contextual
/// and of the style of the paragraph before the table, the last its space after where it
/// is contextual and of the default style.
pub fn cell_stack(
    entries: &[CellBlock],
    previous_style: Option<&Text>,
    default_style: Option<&Text>,
) -> (Vec<StackedLine>, Px) {
    let shown: Vec<(usize, &CellBlock)> = entries.iter().enumerate().filter(|(_, e)| !e.hidden).collect();
    // A cell's stack is kept with its measure: no room beyond its lines.
    let mut out = Vec::with_capacity(shown.iter().map(|(_, e)| e.pitches.len()).sum());
    let mut previous: Option<&CellBlock> = None;
    for (index, (entry, paragraph)) in shown.iter().enumerate() {
        let (gap, boxed) = match previous {
            None => {
                let kept = paragraph.contextual
                    && previous_style.is_some()
                    && paragraph.effective_style.as_ref() == previous_style;
                let gap = Px::twips(if kept { 0 } else { paragraph.before });
                (gap, gap)
            }
            Some(before) => {
                let same = paragraph.style == before.style;
                let own_before = if paragraph.contextual && same {
                    0
                } else {
                    paragraph.before
                };
                let previous_after = if before.contextual && same { 0 } else { before.after };
                paragraph_gap(previous_after, own_before)
            }
        };
        let own_after = match shown.get(index + 1) {
            None if paragraph.contextual && paragraph.effective_style.as_ref() == default_style => 0,
            Some((_, following)) if paragraph.contextual && following.style == paragraph.style => 0,
            _ => paragraph.after,
        };
        let last = paragraph.pitches.len().saturating_sub(1);
        for (n, pitch) in paragraph.pitches.iter().enumerate() {
            let first = n == 0;
            let ends = n == last;
            let room = StackedRoom {
                above: (if first { gap + paragraph.top_border } else { Px::ZERO }) + paragraph.drawings.drop(n),
                pitch: pitch.get(),
                below: if ends { paragraph.border } else { Px::ZERO },
                box_before: if first { boxed + paragraph.top_border } else { Px::ZERO },
                box_after: if ends {
                    paragraph.border + Px::twips(own_after)
                } else {
                    Px::ZERO
                },
            };
            out.push(StackedLine::new(*entry, n, room));
        }
        previous = Some(paragraph);
    }
    let after = match previous {
        Some(p) if !(p.contextual && p.effective_style.as_ref() == default_style) => Px::twips(p.after),
        _ => Px::ZERO,
    };
    (out, after)
}

/// Where a cell is merged: not, the first of a merge down, or one below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    /// Not merged down.
    None,
    /// `w:vMerge="restart"`.
    Restart,
    /// `w:vMerge="continue"`.
    Continue,
}

/// A cell's vertical alignment (`w:vAlign`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Top (and anything not centre or bottom).
    Top,
    /// Centre.
    Center,
    /// Bottom.
    Bottom,
}

/// One cell, measured.
#[derive(Debug, Clone)]
pub struct CellFlow {
    /// The cell as resolved: its path, properties and borders settled with its
    /// neighbours' (what its borders and shading are drawn from).
    pub resolved: Arc<ResolvedCell>,
    /// Its grid column.
    pub column: i64,
    /// Its own top border's width, twips (before its neighbours' are settled).
    pub border_top: i64,
    /// Its own bottom border's width, twips.
    pub border_bottom: i64,
    /// Its top margin, px.
    pub margin_top: Px,
    /// Its bottom margin, px.
    pub margin_bottom: Px,
    /// Where it is merged (`w:vMerge`, a stated value of any kind).
    pub merge: Merge,
    /// It states `w:vMerge` at all.
    pub merge_stated: bool,
    /// Its `w:vAlign`.
    pub align: Align,
    /// Where its text goes across.
    pub cell_box: CellBox,
    /// Its blocks.
    pub entries: Vec<CellBlock>,
    /// Its blocks' height, the first one's space before to the last one's after.
    pub content: Px,
    /// A cell merged down: how many rows it covers.
    pub rows: usize,
    /// A cell merged down: its rows' height below its top border to the edge below the
    /// last.
    pub span_height: Option<Px>,
    /// Its lines as stacked.
    pub stack: Vec<StackedLine>,
    /// The space after its last block.
    pub after: Px,
    /// `(top, foot)` of each floating drawing text does not wrap around positioned in a
    /// vertically aligned cell, px from below the row's top border, laid out from the
    /// cell's top: its foot counts in the height aligned (`CellFlow.loose`).
    pub loose: Vec<(Px, Px)>,
}

impl CellFlow {
    /// A cell below the first of a merge down: it draws nothing.
    pub fn merged(&self) -> bool {
        self.merge == Merge::Continue
    }

    /// A cell of a merge down: its first (holding the merge's lines) or one below it.
    pub fn merging(&self) -> bool {
        self.merged() || self.rows > 1
    }

    /// Its top and bottom margins.
    pub fn margins(&self) -> Px {
        self.margin_top + self.margin_bottom
    }

    /// The height of its stacked lines `start` to `end`, and the space after the last
    /// block where the slice ends the cell (`slice_height`).
    pub fn slice_height(&self, start: usize, end: usize) -> Px {
        let lines: Px = self
            .stack
            .get(start..end.min(self.stack.len()))
            .unwrap_or_default()
            .iter()
            .map(StackedLine::height)
            .sum();
        if end == self.stack.len() {
            lines + self.after
        } else {
            lines
        }
    }

    /// How many of its stacked lines from `start` end at or above `limit` when the first
    /// starts at `top` (`_fitting`).
    pub fn fitting(&self, start: usize, top: Px, limit: Px) -> usize {
        let mut end = start;
        while end < self.stack.len() && top + self.slice_height(start, end + 1) <= limit {
            end += 1;
        }
        end
    }

    /// How far down its lines -- and the drawings anchored in it -- go under its
    /// `w:vAlign`, `space` the room between its margins (`align_offset`): the height
    /// aligned is its content's, down to the foot of a drawing text does not wrap around
    /// that starts above its bottom margin (`make_cell_valign_probe.py`, F.25).
    pub fn align_offset(&self, space: Px) -> Px {
        let mut height = self.content;
        for (top, foot) in &self.loose {
            if *top - self.margin_top < space {
                height = height.max(*foot - self.margin_top);
            }
        }
        match self.align {
            Align::Top => Px::ZERO,
            Align::Center => {
                let half = (space - height).scale(1, 2).unwrap_or(Px::ZERO);
                half.max(Px::ZERO)
            }
            Align::Bottom => (space - height).max(Px::ZERO),
        }
    }
}

/// `w:trHeight` and its rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowHeight {
    /// The height stated, twips (0 unstated).
    pub value: i64,
    /// `exact` (else at least).
    pub exact: bool,
}

/// One row, measured.
#[derive(Debug, Clone)]
pub struct RowFlow {
    /// Its cells.
    pub cells: Vec<CellFlow>,
    /// The border on the edge above it (the table's top border for the first).
    pub top_border: Px,
    /// From below that border to the edge below.
    pub height: Px,
    /// `w:trHeight`.
    pub stated: RowHeight,
    /// `w:cantSplit`.
    pub cant_split: bool,
    /// `w:tblHeader`.
    pub header: bool,
}

impl RowFlow {
    /// Each cell's own top and bottom borders' widths, twips.
    pub fn cells_borders(&self) -> Vec<(i64, i64)> {
        self.cells.iter().map(|c| (c.border_top, c.border_bottom)).collect()
    }
}

/// The row's height between its top border and the edge below (`row_height`): the tallest
/// cell with its margins, at least `w:trHeight` and the margins, or exactly `w:trHeight`
/// less the border above; a cell merged down counts only its margins here.
pub fn row_height(row: &RowFlow) -> Px {
    let RowHeight { value, exact } = row.stated;
    if exact && value != 0 {
        return Px::twips(value) - row.top_border;
    }
    let floor = if value != 0 { Px::twips(value) } else { Px::ZERO };
    row.cells
        .iter()
        .map(|c| {
            let content = if c.merge_stated { Px::ZERO } else { c.content };
            content.max(floor) + c.margins()
        })
        .max()
        .unwrap_or(Px::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stacked_line_keeps_its_room_exact_in_half_the_room() {
        let room = StackedRoom {
            above: Px::twips(120),
            pitch: Px::ratio(2_875, 48).unwrap_or(Px::ZERO),
            below: Px::ZERO,
            box_before: Px::twips(3),
            box_after: Px::int(-2),
        };
        let line = StackedLine::new(1, 2, room);
        assert_eq!(
            StackedRoom {
                above: line.above(),
                pitch: line.pitch(),
                below: line.below(),
                box_before: line.box_before(),
                box_after: line.box_after(),
            },
            room
        );
        assert_eq!(line.height(), room.above + room.pitch + room.below);
        assert_eq!(std::mem::size_of::<StackedLine>(), 96);
    }
}
