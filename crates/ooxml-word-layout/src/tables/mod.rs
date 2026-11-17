//! Owns: Word's tables laid out (docs/architecture/README.md, section 8; phase PL, lane
//! L3b) -- docx2svg's `table` module: [`WordTables`], the [`TableLayout`] the engine lays
//! top-level tables out with; the grid and the cells' boxes (`grid`), the columns' widths
//! (`widths`, `autofit`) from the cells' content where Word sizes by it (`content`),
//! whether a table floats (`floating`), the cells and rows measured (`cells`, `build`),
//! the table as the paginator sees it (`measured`) and one page's part of it (`place`).
//!
//! The rules are docx2svg's, each measured in Word (ROADMAP.md, "Tables --
//! measured"); a table the measured rules refuse is laid out by docx2svg's approximate
//! ones where those are rules of widths (its grid) or of a frame in a cell, and reports
//! [`MeasuredTable::approximation`]; anything else stays refused.
//!
//! Must not: lay out floating tables, floating drawings text wraps around in a cell, or
//! notes in cells (lane L3c), or draw (lane L3d); hold state between tables (the
//! paginator's flow keeps each table's measure by its `Arc`).

mod autofit;
mod build;
mod cells;
mod content;
mod floating;
mod grid;
mod measured;
mod place;
mod widths;

pub use build::{effective_style, table_paragraphs, Frame};
pub use cells::{Align, CellBlock, CellFlow, Content, Merge, RowFlow, RowHeight, StackedLine};
pub use grid::CellBox;
pub use measured::{Approximated, MeasuredTable};
pub use place::MergeDown;

use crate::table::{TableContext, TableFlow, TableJob, TableLayout, Unsupported};
use build::{lay_table, TableInput};
use ooxml_base::Q;
use std::sync::Arc;

/// Word's tables, laid out as docx2svg lays them out.
#[derive(Debug, Clone, Copy, Default)]
pub struct WordTables;

impl WordTables {
    /// A top-level table measured: by the measured rules, else by the approximate ones
    /// (`paginate.flow`'s carry-on), the first refusal kept as its approximation.
    pub fn measure(cx: &mut TableContext<'_, '_>, job: &TableJob<'_>) -> Result<MeasuredTable, Unsupported> {
        let margins = &job.section.props.margins;
        let input = TableInput {
            table: job.table,
            frame: Frame {
                page_left: Q::from_integer(i128::from(margins.left + margins.gutter)),
                column_width: Q::from_integer(i128::from(job.section.column_width(0))),
            },
            previous_style: job.previous.and_then(|p| effective_style(cx.document, p)),
            labels: job.labels,
            nested: false,
        };
        match lay_table(cx, &input, false) {
            Ok(table) => Ok(table),
            Err(why) => {
                let mut table = lay_table(cx, &input, true)?;
                table.approximation = Some(why.0);
                Ok(table)
            }
        }
    }
}

impl TableLayout for WordTables {
    fn flow_table(&self, cx: &mut TableContext<'_, '_>, job: &TableJob<'_>) -> Result<Arc<dyn TableFlow>, Unsupported> {
        Ok(Arc::new(WordTables::measure(cx, job)?))
    }
}
