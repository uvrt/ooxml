//! Owns: the footnotes referenced in a table's cells at the page's foot (docx2svg's
//! `paginate.cell_notes`, `_table_notes`; measured by `make_cell_footnote_probe.py`): a
//! cell line holding a reference fits as a body line with one does -- its end, with the
//! cell's bottom margin and the edge under its row, above the separator, the notes on the
//! page, its new notes but the last whole and the least of the last; when it goes in, the
//! last note takes what then fits. A later line fits above the notes as placed. Where a
//! line does not, the table is placed again ending above it: its row splits there or
//! moves ([`PageContext::table_notes`]).
//!
//! Must not: lay a table out or decide where the page ends (the scan does).

use super::beside::{Beside, Placed};
use super::notes::Foot;
use super::PageContext;
use crate::flow::TableItem;
use crate::notes::flowed::{line_at, line_of_sources};
use crate::notes::NoteLines;
use crate::px::Px;
use crate::table::{TableFlow, TableRequest, Unsupported};
use ooxml_base::Q;
use std::sync::Arc;

/// A stacked line of the table's part of the page: where it ends, its row, cell and place
/// in the cell, and the notes referenced on it.
type Flowed<'t> = (Px, usize, usize, usize, Vec<&'t NoteLines>);

/// At most this many placements of a table on one page while its cells' notes are fitted.
const ATTEMPTS: usize = 64;

impl PageContext<'_> {
    /// The notes referenced on line `paragraph` (block, line) of `flow`, a layout of `t`.
    fn cell_line_notes<'t>(
        t: &'t TableItem,
        flow: &dyn TableFlow,
        paragraph: Option<(usize, usize)>,
    ) -> Vec<&'t NoteLines> {
        let Some((block, line)) = paragraph else {
            return Vec::new();
        };
        let (Some(references), Some(measured)) = (t.notes.references.get(&block), flow.paragraph(block)) else {
            return Vec::new();
        };
        let m = &measured.measured;
        let line_of = line_of_sources(&m.pieces, &m.lines);
        references
            .iter()
            .filter(|(source, _)| line_at(&line_of, *source, m.lines.len()) == line)
            .filter_map(|(_, note)| t.notes.lines.get(note))
            .collect()
    }

    /// The stacked lines of a table's part of the page, in the order they end.
    fn flowed<'t>(t: &'t TableItem, placed: &Placed) -> Vec<Flowed<'t>> {
        let mut out = Vec::new();
        for (k, row) in placed.page.rows.iter().enumerate() {
            if row.header {
                continue;
            }
            let flow: &Arc<dyn TableFlow> = match &placed.room {
                Some(room) if k >= placed.from => &room.flow,
                _ => &t.flow,
            };
            for end in flow.line_ends(row) {
                let notes = Self::cell_line_notes(t, flow.as_ref(), end.paragraph);
                out.push((end.end, row.row, end.cell, end.line, notes));
            }
        }
        out.sort_by_key(|a| (a.0, a.1, a.2, a.3));
        out
    }

    /// The table's part of the page (`placed`, from `request`) with its cells' footnotes
    /// at the foot (`_table_notes`): placed again ending above the first line that does
    /// not fit with its notes, until every line does; and the foot with them. `bottom` is
    /// where the page's text ends.
    pub(super) fn table_notes(
        &self,
        t: &TableItem,
        mut placed: Placed,
        request: &TableRequest,
        beside: Option<&Beside>,
        foot: &Foot,
        bottom: Px,
    ) -> Result<(Placed, Foot), Unsupported> {
        if t.notes.is_empty() {
            return Ok((placed, foot.clone()));
        }
        let mut limit = bottom;
        for _attempt in 0..ATTEMPTS {
            let flowed = Self::flowed(t, &placed);
            if flowed.iter().all(|f| f.4.is_empty()) {
                return Ok((placed, foot.clone()));
            }
            let mut trial = foot.clone();
            let mut violation = None;
            for (number, (end, _, _, _, new)) in flowed.iter().enumerate() {
                let need = trial.need_below_cell(new, *end, bottom);
                let first = request.first_on_page && number == 0;
                if !first && *end > bottom - need {
                    violation = Some(*end);
                    break;
                }
                trial.place_below_cell(new, *end, bottom);
            }
            let Some(violation) = violation else {
                return Ok((placed, trial));
            };
            // Placed again, ending just above the line that does not fit.
            limit = limit.min(violation - Px::from(Q::new(1, 1_000_000)));
            let again = TableRequest {
                bottom: limit,
                ..request.clone()
            };
            placed = self.place_beside(t, beside, &again)?;
        }
        Ok((placed, foot.clone()))
    }
}
