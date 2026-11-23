//! Owns: the body's lines a laid page draws, in docx2svg's drawing order -- the endnotes'
//! continuation separator a page opens with, then the flow's lines item by item: a drop
//! cap's lines with the first line of the paragraph it drops into, a paragraph's lines, a
//! table's rows' cell lines -- each with its baseline and text rows on whole device pixels
//! and the left edge of the text column or cell it starts from.
//!
//! Must not: place a line anywhere but where the paginator (or the table's rows) placed it.

use super::linebox::{baseline_in_box, line_extent, text_band, LineBox};
use super::PageFrame;
use crate::engine::DocumentLayout;
use crate::flow::FlowItem;
use crate::heights::LineHeight;
use crate::paginate::{LaidPage, PlacedLine};
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use std::sync::Arc;

/// Where a drawn line comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineSource {
    /// A line of the paragraph that is flow item `item`.
    Flow {
        /// The flow item.
        item: usize,
    },
    /// A line of a paragraph in a cell of the table that is flow item `item`: its
    /// paragraph `block` counted from the table's first (nested tables' included).
    Cell {
        /// The flow item.
        item: usize,
        /// The paragraph, from the table's first.
        block: usize,
    },
    /// A line of the drop cap that drops into flow item `item`.
    DropCap {
        /// The flow item it drops into.
        item: usize,
    },
    /// The endnotes' continuation separator's line, at the top of a page they go on to
    /// (flow item `item` the endnote paragraph the page starts with).
    Opening {
        /// The flow item.
        item: usize,
    },
}

impl LineSource {
    /// The flow item it is drawn for.
    pub fn item(self) -> usize {
        match self {
            LineSource::Flow { item }
            | LineSource::Cell { item, .. }
            | LineSource::DropCap { item }
            | LineSource::Opening { item } => item,
        }
    }
}

/// One line a page draws, on whole device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawnLine {
    /// Where it comes from.
    pub source: LineSource,
    /// Its paragraph among the body's paragraphs (cells included); `None` for one of no
    /// body paragraph (the continuation separator's, a drop cap's frame paragraph).
    pub block: Option<usize>,
    /// The top-level body block of a drop cap's paragraph.
    pub body: Option<usize>,
    /// Its number in its paragraph.
    pub number: usize,
    /// The exact top of its pitch.
    pub top: Px,
    /// Its pitch.
    pub pitch: Px,
    /// The device-pixel row of its baseline.
    pub baseline: i64,
    /// The rows its text fills.
    pub band: (i64, i64),
    /// The text column it is in, in a section of several columns.
    pub column: Option<usize>,
    /// The left edge it starts from: its text column's, or its cell's text start.
    pub left: Px,
    /// In a cell whose fill turns automatic text white.
    pub white_text: bool,
}

/// A paragraph item's lines as page `laid` placed them: broken again there, or the flow's.
pub(crate) fn measured_on(layout: &DocumentLayout, laid: &LaidPage, item: usize) -> Option<Arc<MeasuredParagraph>> {
    match laid.broken.get(&item) {
        Some(again) => Some(Arc::clone(&again.measured)),
        None => layout
            .flow
            .items
            .get(item)
            .and_then(FlowItem::para)
            .map(|p| Arc::clone(&p.measured)),
    }
}

/// A text column's left edge (`column_edges`): the margin, gutter and the columns before
/// it on a whole pixel -- in mode 15 from the length held in layout units -- held in the
/// nearest layout unit.
pub(crate) fn column_left(layout: &DocumentLayout, frame: &PageFrame, offset: i64) -> Px {
    let Some(section) = layout.sections.get(frame.section) else {
        return frame.left;
    };
    let m = &section.props.margins;
    let total = m.left + m.gutter + offset;
    let px = if layout.mode15 {
        Px::twips(total).round_half_up()
    } else {
        Px::ratio(i128::from(total) * 300, 1440).map_or(0, Px::round_half_up)
    };
    Px::int(px).quantise()
}

/// The body's lines page `laid` draws, in drawing order.
pub(crate) fn body_lines(layout: &DocumentLayout, laid: &LaidPage, frame: &PageFrame) -> Vec<DrawnLine> {
    let mut by_item: Vec<(usize, DrawnLine)> = Vec::new();
    for placed in &laid.lines {
        by_item.extend(
            drop_cap_lines(layout, placed, frame)
                .into_iter()
                .map(|l| (placed.item, l)),
        );
        if let Some(line) = flow_line(layout, laid, placed, frame) {
            by_item.push((placed.item, line));
        }
    }
    by_item.extend(table_lines(layout, laid, frame));
    by_item.sort_by_key(|(item, _)| *item);
    opening_line(layout, laid, frame)
        .into_iter()
        .chain(by_item.into_iter().map(|(_, line)| line))
        .collect()
}

/// What a body line a page draws is of, before it is placed on device pixels: its
/// source, its body paragraph and its top-level block, as its [`DrawnLine`] holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineOf {
    /// Where it comes from.
    pub(crate) source: LineSource,
    /// Its paragraph among the body's paragraphs ([`DrawnLine::block`]).
    pub(crate) block: Option<usize>,
    /// The top-level body block of a drop cap's paragraph ([`DrawnLine::body`]).
    pub(crate) body: Option<usize>,
}

impl From<&DrawnLine> for LineOf {
    fn from(line: &DrawnLine) -> LineOf {
        LineOf {
            source: line.source,
            block: line.block,
            body: line.body,
        }
    }
}

/// What each body line page `laid` (`height` px high) draws is of, in [`body_lines`]'
/// order, without placing any: the lines it draws are those, one for one (what a reader of
/// which paragraphs a page draws asks, not where).
pub(crate) fn body_lines_of(layout: &DocumentLayout, laid: &LaidPage, height: i64) -> Vec<LineOf> {
    let mut by_item: Vec<(usize, LineOf)> = Vec::new();
    for placed in &laid.lines {
        if let Some(drop) = drop_cap_of(layout, placed) {
            let line = LineOf {
                source: LineSource::DropCap { item: placed.item },
                block: None,
                body: Some(drop.body),
            };
            by_item.extend(drop.measured.heights.iter().map(|_| (placed.item, line)));
        }
        if let Some((item, ..)) = flow_drawn(layout, laid, placed) {
            let line = LineOf {
                source: LineSource::Flow { item: placed.item },
                block: Some(item.block),
                body: Some(item.body),
            };
            by_item.push((placed.item, line));
        }
    }
    let height = Px::int(height);
    for placed in &laid.rows {
        let Some(FlowItem::Table(t)) = layout.flow.items.get(placed.item) else {
            continue;
        };
        let flow = placed.flow(t);
        let flow: &dyn crate::table::TableFlow = flow.as_ref();
        for line in flow.lines(&placed.row) {
            if row_line_height(flow, &line, height).is_some() {
                let of = LineOf {
                    source: LineSource::Cell {
                        item: placed.item,
                        block: line.block,
                    },
                    block: Some(t.block + line.block),
                    body: Some(t.body),
                };
                by_item.push((placed.item, of));
            }
        }
    }
    by_item.sort_by_key(|(item, _)| *item);
    let opening = opening_of(layout, laid).map(|(item, _)| LineOf {
        source: LineSource::Opening { item },
        block: None,
        body: None,
    });
    opening
        .into_iter()
        .chain(by_item.into_iter().map(|(_, line)| line))
        .collect()
}

/// The left edge a placed line starts from: its column's.
fn line_left(layout: &DocumentLayout, placed: &PlacedLine, frame: &PageFrame) -> Px {
    match placed.column {
        Some(_) => column_left(layout, frame, placed.offset),
        None => frame.left,
    }
}

/// A paragraph's placed line where it draws anything: its paragraph item, its height and
/// whether it is the paragraph's last.
fn flow_drawn<'l>(
    layout: &'l DocumentLayout,
    laid: &LaidPage,
    placed: &PlacedLine,
) -> Option<(&'l crate::flow::ParaItem, LineHeight, bool)> {
    let item = layout.flow.items.get(placed.item)?.para()?;
    let measured = measured_on(layout, laid, placed.item)?;
    if item.placeholder.is_some() || measured.undrawn.contains(&placed.line) {
        // A placeholder's lines hold no text (`Layout._box` draws its extent instead).
        return None;
    }
    let height = measured.heights.get(placed.line)?.clone();
    Some((item, height, placed.line + 1 == measured.heights.len()))
}

/// A paragraph's placed line, where it draws anything.
fn flow_line(layout: &DocumentLayout, laid: &LaidPage, placed: &PlacedLine, frame: &PageFrame) -> Option<DrawnLine> {
    let (item, height, last) = flow_drawn(layout, laid, placed)?;
    let space_after = if last {
        let same_next = layout
            .flow
            .items
            .get(placed.item + 1)
            .and_then(FlowItem::para)
            .is_some_and(|next| next.style == item.style);
        let own_after = if item.contextual && same_next { 0 } else { item.after };
        item.border + Px::twips(own_after)
    } else {
        Px::ZERO
    };
    let b = LineBox::new(height, placed.space_before, space_after);
    Some(DrawnLine {
        source: LineSource::Flow { item: placed.item },
        block: Some(item.block),
        body: Some(item.body),
        number: placed.line,
        top: placed.top,
        pitch: placed.pitch,
        baseline: baseline_in_box(placed.top - b.space_before, &b),
        band: text_band(placed.top - b.space_before, &b),
        column: placed.column,
        left: line_left(layout, placed, frame),
        white_text: false,
    })
}

/// A drop cap's lines, drawn with the first line of the paragraph it drops into: from
/// that paragraph's top past the drop cap's space before, one under the other, each
/// baseline from the foot of its line (`_drop_cap`).
fn drop_cap_lines(layout: &DocumentLayout, placed: &PlacedLine, frame: &PageFrame) -> Vec<DrawnLine> {
    let Some(drop) = drop_cap_of(layout, placed) else {
        return Vec::new();
    };
    let column = line_left(layout, placed, frame);
    let left = if drop.beside() {
        column
    } else {
        let width = Px::from(drop.width * Px::layout_unit());
        column - Px::int(width.round_half_up())
    };
    let mut y = placed.paragraph_top + Px::twips(drop.before);
    let mut out = Vec::new();
    for (number, height) in drop.measured.heights.iter().enumerate() {
        let b = LineBox::new(height.clone(), Px::ZERO, Px::ZERO);
        let below = line_extent(height).1;
        out.push(DrawnLine {
            source: LineSource::DropCap { item: placed.item },
            block: None,
            body: Some(drop.body),
            number,
            top: y,
            pitch: height.pitch(),
            baseline: (y + height.pitch()).round_half_up() - below.round_half_up(),
            band: text_band(y, &b),
            column: placed.column,
            left,
            white_text: false,
        });
        y += height.pitch();
    }
    out
}

/// The drop cap a placed line is drawn with: the one dropping into its paragraph, with
/// that paragraph's first line.
fn drop_cap_of<'l>(layout: &'l DocumentLayout, placed: &PlacedLine) -> Option<&'l crate::flow::DropCap> {
    layout
        .flow
        .items
        .get(placed.item)
        .and_then(FlowItem::para)
        .and_then(|p| p.drop_cap.as_deref())
        .filter(|_| placed.line == 0)
}

/// The flow item a page the endnotes go on to opens with, and its continuation
/// separator's line height.
fn opening_of(layout: &DocumentLayout, laid: &LaidPage) -> Option<(usize, LineHeight)> {
    laid.opens?;
    let item = laid.start.item;
    let height = layout.flow.items.get(item)?.para()?.continuation.clone()?;
    Some((item, height))
}

/// The line a page the endnotes go on to opens with: the continuation separator's.
fn opening_line(layout: &DocumentLayout, laid: &LaidPage, frame: &PageFrame) -> Option<DrawnLine> {
    let opening = laid.opens?;
    let (item, height) = opening_of(layout, laid)?;
    let b = LineBox::new(height, Px::ZERO, Px::ZERO);
    Some(DrawnLine {
        source: LineSource::Opening { item },
        block: None,
        body: None,
        number: 0,
        top: opening.top,
        pitch: opening.pitch,
        baseline: baseline_in_box(opening.top, &b),
        band: text_band(opening.top, &b),
        column: None,
        left: frame.left,
        white_text: false,
    })
}

/// The lines a page's table rows draw, by flow item and in drawing order; a line that
/// starts below the page's edge is not drawn (`_table_drawn`).
fn table_lines(layout: &DocumentLayout, laid: &LaidPage, frame: &PageFrame) -> Vec<(usize, DrawnLine)> {
    let height = Px::int(frame.height_px);
    let mut out = Vec::new();
    for placed in &laid.rows {
        let Some(FlowItem::Table(t)) = layout.flow.items.get(placed.item) else {
            continue;
        };
        let lines = row_lines(
            t,
            placed.flow(t).as_ref(),
            (placed.item, &placed.row),
            placed.column,
            height,
        );
        out.extend(lines.into_iter().map(|l| (placed.item, l)));
    }
    out
}

/// The height of a table's cell line, where it is drawn: a line not drawn, or starting at
/// or below `height` (the page's edge), is not (`_table_drawn`).
fn row_line_height(
    flow: &dyn crate::table::TableFlow,
    line: &crate::table::CellLine,
    height: Px,
) -> Option<LineHeight> {
    if !line.drawn || line.top >= height {
        return None;
    }
    flow.paragraph(line.block)
        .and_then(|m| m.heights.get(line.line).cloned())
}

/// The lines a placed row of table item `item` draws, on whole device px, as `flow` (the
/// item's table, or the table laid out in the row's room) placed it; a line that starts
/// below `height` (the page's edge) is not drawn (`_table_drawn`).
pub(crate) fn row_lines(
    t: &crate::flow::TableItem,
    flow: &dyn crate::table::TableFlow,
    (item, row): (usize, &crate::table::PlacedRow),
    column: Option<usize>,
    height: Px,
) -> Vec<DrawnLine> {
    let mut out = Vec::new();
    for line in flow.lines(row) {
        let Some(h) = row_line_height(flow, &line, height) else {
            continue;
        };
        let b = LineBox::new(h, line.space_before, line.space_after);
        out.push(DrawnLine {
            source: LineSource::Cell {
                item,
                block: line.block,
            },
            block: Some(t.block + line.block),
            body: Some(t.body),
            number: line.line,
            top: line.top,
            pitch: line.pitch,
            baseline: baseline_in_box(line.top - b.space_before, &b),
            band: text_band(line.top - b.space_before, &b),
            column,
            left: line.left,
            white_text: line.white_text,
        });
    }
    out
}
