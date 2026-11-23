//! Word's object tools (docx-agent's `tools/objects.py`): `word_edit_table`,
//! `word_format_table`, `word_drawings` and `word_controls`, over the edit core
//! (`ooxml-word-edit`).
//!
//! Owns: the tools' definitions (descriptions beside them) and their handlers: a function
//! of the typed arguments and an [`crate::Editing`], whose edits run as one command per
//! call through the editor (one history step).
//!
//! Must not: hold sessions or documents by id, keep state between calls (the rows written
//! are the host's record, handed in), or write the document but through a command.

mod cell_fills;
mod cell_values;
mod chart_add;
mod chart_add_plan;
mod chart_block;
mod chart_data;
mod chart_fold;
mod chart_keeper;
mod chart_names;
mod chart_read;
mod chart_record;
mod chart_style;
mod chart_workbook;
mod controls;
mod drawings;
mod drawings_args;
mod drawings_picture;
mod edit_chart;
mod edit_chart_args;
mod edit_table;
mod edit_table_args;
mod edit_table_cells;
mod edit_table_pending;
mod format_table;
mod format_table_args;
mod made_by;
mod rows_written;
mod set_cells;
mod set_from;
mod table_sync;
mod table_sync_keys;
mod table_sync_read;
mod table_sync_said;
mod table_view;

pub use controls::word_controls;
pub use drawings::word_drawings;
pub use drawings_args::{Against, ControlAction, ControlType, DrawingAction, WordControls, WordDrawings, Wrap, ZOrder};
pub use edit_chart::{edit_chart, edit_chart_read};
pub use edit_chart_args::{
    Blanks, ChartAction, ChartAxis, ChartBox, ChartPosition, ChartType, Dash, EditChart, Gridlines, Marker, ScaleArg,
    ScaleKey, SeriesArg, SeriesType, StyleArg, TextPart,
};
pub use edit_table::word_edit_table;
pub use edit_table_args::{CellArg, KeyPair, Loose, Position, Side, TableAction, WordEditTable};
pub use format_table::word_format_table;
pub use format_table_args::{Across, BorderStyle, HeightRule, Look, Scope, Vertical, WidthOf, WordFormatTable};
pub use made_by::{MadeBy, Shape as TableShape};
pub use rows_written::RowsWritten;
pub(crate) use table_view::table_view;
pub(crate) use table_view::{read_look, LOOK_NAMES};

/// Registers the group's tools, in the order a model is shown them (the crate's
/// [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    format_table::register(r);
    edit_table::register(r);
    drawings::register(r);
    controls::register(r);
    edit_chart::register(r);
}
