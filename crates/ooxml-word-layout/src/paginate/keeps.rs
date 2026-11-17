//! Owns: where a page (or column) ends when a line does not fit on it
//! (docx2svg's `paginate._keeps` and `_break_before`): widow and orphan control,
//! `keepLines`, `keepNext` (the whole paragraph below mode 15, its last line in mode 15),
//! a table's rows that keep with the next, and a chain of keeps taller than a page.
//!
//! Must not: measure or place anything; it reads the flow only.

use super::{PageContext, Position};
use crate::flow::FlowItem;

/// Widow and orphan control: the lines it keeps together at each end of a paragraph.
const WIDOW_LINES: usize = 2;

impl PageContext<'_> {
    /// The first line that must go to the next page when the line at `overflow` does not
    /// fit (`_keeps`).
    pub(crate) fn keeps(&self, start: &Position, overflow: Position) -> Position {
        let index = overflow.item;
        let mut number = overflow.line;
        let first = if index == start.item { start.line } else { 0 };
        let mut target = overflow.clone();
        match self.items.get(index) {
            Some(FlowItem::Table(t)) => {
                if !overflow.part.is_empty() {
                    return overflow;
                }
                while number > 0 && t.flow.keeps_with_next(number - 1) {
                    number -= 1;
                }
                if number < overflow.line && 0 < number && number <= t.flow.header_rows() && self.mode15 {
                    number = 0;
                }
                target = Position::at(index, number);
            }
            Some(FlowItem::Para(p)) if number > 0 => {
                if p.keep_lines && first == 0 {
                    target = Position::at(index, 0);
                } else if p.widow.unwrap_or(true) {
                    let mut end = p.count();
                    if self.mode15 {
                        end = p
                            .measured
                            .page_breaks
                            .iter()
                            .filter(|&&k| k >= number)
                            .map(|k| k + 1)
                            .min()
                            .unwrap_or(end);
                    }
                    if end.saturating_sub(number) < WIDOW_LINES && end >= number {
                        number = first.max(end.saturating_sub(WIDOW_LINES));
                    }
                    if first == 0 && 0 < number && number < WIDOW_LINES {
                        number = 0;
                    }
                    target = Position::at(index, number);
                }
            }
            _ => {}
        }
        if target.line == 0 && target.part.is_empty() && target.item > start.item {
            let previous = target.item - 1;
            match self.items.get(previous) {
                Some(FlowItem::Para(p)) if p.keep_next => {
                    let line = if self.mode15 { p.count().saturating_sub(1) } else { 0 };
                    return self.keeps(start, Position::at(previous, line));
                }
                Some(FlowItem::Table(t)) if t.flow.rows() > 0 && t.flow.keeps_with_next(t.flow.rows() - 1) => {
                    return self.keeps(start, Position::at(previous, t.flow.rows() - 1));
                }
                _ => {}
            }
        }
        target
    }

    /// Where the page ends, given that the line at `overflow` does not fit on it;
    /// `empty_ok`: the column did not start empty, so the keeps may leave it holding
    /// nothing (`_break_before`).
    pub(crate) fn break_before(&self, start: &Position, overflow: Position, empty_ok: bool) -> Position {
        let target = self.keeps(start, overflow.clone());
        if empty_ok {
            return target.max(start.clone());
        }
        if target <= *start {
            if self.mode15
                && matches!(self.items.get(overflow.item), Some(FlowItem::Table(_)))
                && Position::at(overflow.item, 0) > *start
            {
                return Position::at(overflow.item, 0);
            }
            return overflow;
        }
        target
    }
}
