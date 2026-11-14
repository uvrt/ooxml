//! A table's cells and rows from their paragraphs' measured lines: `table.cell_stack`,
//! `content_height` and `row_height` (no merges, no nested tables, no drawings in cells).

use crate::measure::LineH;
use crate::model::{CellIn, RowIn};
use crate::paginate::{FCell, Stk};
use crate::units::Px;

/// `table.cell_stack`'s lines, from each paragraph's heights (`hs`, the cell's paragraphs in order).
pub fn cell_stack(px: &Px, c: &CellIn, hs: &[Vec<LineH>], previous_style: i64, _default_style: i64) -> Vec<Stk> {
    let mut out = Vec::new();
    let mut previous: Option<usize> = None;
    for (k, p) in c.paragraphs.iter().enumerate() {
        if p.hidden {
            continue;
        }
        let gap = match previous {
            None => {
                let mut before = p.before;
                if p.contextual && previous_style != -1 && p.effective_style == previous_style {
                    before = 0;
                }
                px.twips(before)
            }
            Some(j) => {
                let prev = &c.paragraphs[j];
                let same = p.style == prev.style;
                let own_before = if p.contextual && same { 0 } else { p.before };
                let prev_after = if prev.contextual && same { 0 } else { prev.after };
                let box_before = if own_before > prev_after { px.twips(own_before - prev_after) } else { 0 };
                px.twips(prev_after) + box_before
            }
        };
        let heights = &hs[k];
        let last = heights.len().saturating_sub(1);
        for (n, h) in heights.iter().enumerate() {
            let above = if n == 0 { gap + p.top_border } else { 0 };
            let below = if n == last { p.border } else { 0 };
            out.push(Stk { h: above + h.pitch + below, above, pitch: h.pitch, para: k as u32, number: n as u32, para_lines: heights.len() as u32, widow: p.widow.unwrap_or(true) });
        }
        previous = Some(k);
    }
    out
}

/// The space after the cell's last paragraph (`cell_stack`'s second value).
pub fn cell_after(px: &Px, c: &CellIn, default_style: i64) -> i64 {
    match c.paragraphs.iter().rev().find(|p| !p.hidden) {
        Some(p) => px.twips(if p.contextual && p.effective_style == default_style { 0 } else { p.after }),
        None => 0,
    }
}

/// `table.row_height` from the cells as stacked.
pub fn row_height(px: &Px, r: &RowIn, cells: &[FCell]) -> i64 {
    let (value, rule) = match &r.tr_height {
        Some((v, rule)) => (*v, rule.clone()),
        None => (0, None),
    };
    if rule.as_deref() == Some("exact") && value != 0 {
        return value - r.top_border;
    }
    let _ = px;
    cells
        .iter()
        .map(|c| {
            let content: i64 = c.stack.iter().map(|s| s.h).sum::<i64>() + c.after;
            content.max(value) + c.margins
        })
        .max()
        .unwrap_or(0)
}
