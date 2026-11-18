//! Owns: a flow of its own stacked from a given top with nothing to break it into pages --
//! a header's, a footer's, a page's footnotes (docx2svg's `_walk` from position 0 to the
//! end, its bottom far below): each paragraph's lines with their boxes, on whole device
//! pixels, and each table's rows as placed, in drawing order.
//!
//! Each paragraph's top as a floating drawing anchored in it sees it (docx2svg's
//! `_paragraph_top`) is recorded beside its lines, for the story's drawings to be placed.
//!
//! Must not: break pages, or place floating drawings (a story's are placed with its paint).

use super::linebox::{baseline_in_box, text_band, LineBox};
use super::lines::{DrawnLine, LineSource};
use crate::flow::FlowItem;
use crate::paginate::table_gap;
use crate::px::Px;
use crate::table::{PlacedRow, TableRequest};
use crate::vertical::{keeps_space_before_at_page_top, page_top_gap, paragraph_gap};

/// What walking a flow drew: its lines (cells' included) and its tables' rows, by item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Walked {
    /// The lines, in drawing order.
    pub lines: Vec<DrawnLine>,
    /// Each table's rows as placed: the flow item and the row.
    pub rows: Vec<(usize, PlacedRow)>,
    /// Where the stack ends.
    pub end: Px,
    /// Each paragraph's top as its floating drawings' `paragraph` frame sees it: the flow
    /// item and the top (the stack's top for the first, else where the paragraph before
    /// ends after its own space after).
    pub paragraph_tops: Vec<(usize, Px)>,
}

/// How a flow is walked: from `top`, its text starting at `left`.
#[derive(Debug, Clone, Copy)]
pub struct WalkFrom {
    /// The top.
    pub top: Px,
    /// The text column's left edge.
    pub left: Px,
    /// Compatibility mode 15 or later.
    pub mode15: bool,
    /// The last paragraph keeps everything down to here below its text (a text box's
    /// room); `None` for none.
    pub bottom: Option<Px>,
}

/// The gap above a paragraph's first line and what of it is in its box (`_walk`).
fn gap_above(item: &crate::flow::ParaItem, previous: Option<&FlowItem>, first: bool, mode15: bool) -> (Px, Px) {
    if first {
        let keeps = keeps_space_before_at_page_top(item.section_start, item.page_break_before, mode15);
        return page_top_gap(item.before, keeps, 0);
    }
    match previous {
        Some(FlowItem::Para(before)) => {
            let same = item.style == before.style;
            let own = if item.contextual && same { 0 } else { item.before };
            let after = if before.contextual && same { 0 } else { before.after };
            paragraph_gap(after, own)
        }
        _ => (Px::twips(item.before), Px::twips(item.before)),
    }
}

/// Where a paragraph whose first line is walked at `y` is for its floating drawings
/// (`_paragraph_top`): `y` for the stack's first or after a table, else after the space
/// after of the paragraph before.
fn paragraph_top(item: &crate::flow::ParaItem, previous: Option<&FlowItem>, first: bool, y: Px) -> Px {
    match previous {
        Some(FlowItem::Para(before)) if !first => {
            let same = item.style == before.style;
            y + Px::twips(if before.contextual && same { 0 } else { before.after })
        }
        _ => y,
    }
}

/// Walk `items` whole from `from.top`.
pub fn walk(items: &[FlowItem], from: &WalkFrom) -> Walked {
    walk_between(items, from, (0, 0), (items.len(), 0))
}

/// Walk `items` from line `start.1` of item `start.0` up to (not including) line `end.1`
/// of item `end.0`, from `from.top` (`_walk` between two positions).
pub fn walk_between(items: &[FlowItem], from: &WalkFrom, start: (usize, usize), end: (usize, usize)) -> Walked {
    let mut out = Walked::default();
    let mut y = from.top;
    let mut previous: Option<&FlowItem> = None;
    let mut first = true;
    for (index, item) in items.iter().enumerate().skip(start.0) {
        let from_line = if index == start.0 { start.1 } else { 0 };
        if (index, from_line) >= end {
            break;
        }
        let p = match item {
            FlowItem::Para(p) => p,
            FlowItem::Table(t) => {
                if !first {
                    y += table_gap(previous, t);
                }
                let request = TableRequest {
                    y,
                    bottom: y + Px::int(10_000_000),
                    row: 0,
                    part: Vec::new(),
                    first_on_page: first,
                    mode15: from.mode15,
                    split_kept: false,
                    stop: None,
                };
                let Ok(laid) = t.flow.place(&request) else { break };
                y = laid.end;
                for row in &laid.rows {
                    out.lines.extend(super::lines::row_lines(
                        t,
                        t.flow.as_ref(),
                        (index, row),
                        None,
                        Px::int(i64::MAX / 4),
                    ));
                }
                out.rows.extend(laid.rows.into_iter().map(|r| (index, r)));
                first = first && out.rows.is_empty();
                previous = Some(item);
                continue;
            }
            FlowItem::Stop(_) => break,
        };
        y = walk_paragraph(
            &mut out,
            (items, index, p),
            from,
            (from_line, end),
            (y, previous, &mut first),
        );
        previous = Some(item);
    }
    out.end = y;
    out
}

/// One paragraph's lines from `from_line` up to `end`; where the stack is after them.
fn walk_paragraph(
    out: &mut Walked,
    (items, index, p): (&[FlowItem], usize, &crate::flow::ParaItem),
    from: &WalkFrom,
    (from_line, end): (usize, (usize, usize)),
    (mut y, previous, first): (Px, Option<&FlowItem>, &mut bool),
) -> Px {
    let count = p.measured.heights.len();
    let mut space_before = Px::ZERO;
    for (number, height) in p.measured.heights.iter().enumerate().skip(from_line) {
        if (index, number) >= end {
            return y;
        }
        if number == 0 {
            out.paragraph_tops.push((index, paragraph_top(p, previous, *first, y)));
            let (gap, boxed) = gap_above(p, previous, *first, from.mode15);
            y += gap + p.top_border;
            space_before = boxed + p.top_border;
        }
        let last = number + 1 == count;
        let mut space_after = Px::ZERO;
        if last {
            let same_next = items
                .get(index + 1)
                .and_then(FlowItem::para)
                .is_some_and(|next| next.style == p.style);
            let own_after = if p.contextual && same_next { 0 } else { p.after };
            space_after = p.border + Px::twips(own_after);
            if let (Some(bottom), true) = (from.bottom, index + 1 == items.len()) {
                space_after = space_after.max(bottom - (y + height.pitch()));
            }
        }
        let b = LineBox::new(
            height.clone(),
            if number == 0 { space_before } else { Px::ZERO },
            space_after,
        );
        if !p.measured.undrawn.contains(&number) {
            out.lines.push(DrawnLine {
                source: LineSource::Flow { item: index },
                block: Some(p.block),
                body: Some(p.body),
                number,
                top: y,
                pitch: height.pitch(),
                baseline: baseline_in_box(y - b.space_before, &b),
                band: text_band(y - b.space_before, &b),
                column: None,
                left: from.left,
                white_text: false,
            });
        }
        y += height.pitch();
        *first = false;
    }
    if (index, count) <= end {
        y += p.border;
    }
    y
}
