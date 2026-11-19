//! Word's object tools (docx-agent's `tools/objects.py`): `word_edit_table`,
//! `word_format_table`, `word_drawings` and `word_controls`, over the edit core
//! (`ooxml-word-edit`).
//!
//! Owns: the tools' definitions (descriptions beside them) and their handlers: a function
//! of the typed arguments and an [`crate::Editing`], whose edits run as one command per
//! call through the editor (one history step).
//!
//! Must not: hold sessions or documents by id, keep state between calls, or write the
//! document but through a command.

mod cell_fills;
mod controls;
mod drawings;
mod drawings_args;
mod edit_table;
mod edit_table_args;
mod format_table;
mod format_table_args;
mod set_cells;
mod table_view;

pub use controls::word_controls;
pub use drawings::word_drawings;
pub use drawings_args::{Against, ControlAction, ControlType, DrawingAction, WordControls, WordDrawings, Wrap, ZOrder};
pub use edit_table::word_edit_table;
pub use edit_table_args::{CellArg, KeyPair, Loose, Position, Side, TableAction, WordEditTable};
pub use format_table::word_format_table;
pub use format_table_args::{Across, BorderStyle, HeightRule, Look, Scope, Vertical, WidthOf, WordFormatTable};
pub(crate) use table_view::{read_look, LOOK_NAMES};
