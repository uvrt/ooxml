//! Owns: `word_edit_table`'s definition (docx-agent's `tools/objects.py`,
//! `word_edit_table`): its arguments, byte-equal to today's.
//!
//! Must not: change the document but through a command.

use ooxml_tools::{ArgEnum, ToolArgs};

/// What `word_edit_table` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAction {
    /// Write cells by position or labels.
    SetCells,
    /// Insert a row.
    InsertRow,
    /// Delete a row.
    DeleteRow,
    /// Insert a column.
    InsertColumn,
    /// Delete a column.
    DeleteColumn,
    /// Merge a rectangle of cells.
    Merge,
    /// Split a cell.
    Split,
    /// Keep rows in step with a source, by key.
    Sync,
    /// Copy a row with its content.
    CopyRow,
}

/// Which side of `index` a row or column goes.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// After it.
    After,
    /// Before it.
    Before,
}

/// Whose spelling of a loosely matched key stays.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loose {
    /// The table's.
    Table,
    /// The data's.
    Data,
}

/// One cell of `set_cells`.
// Today's schema describes `row` and `col` not at all: a doc comment would become a description.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct CellArg {
    #[arg(minimum = 0)]
    pub row: Option<i64>,
    #[arg(minimum = 0)]
    pub col: Option<i64>,
    /// Or the row's first cell text.
    pub row_label: Option<String>,
    /// Or the column's header text.
    pub col_label: Option<String>,
    /// Lines are paragraphs.
    #[arg(facts)]
    pub text: Option<String>,
    /// Or one value: d:<id>/#12/<column>, :0.0 to round.
    #[arg(rename = "from", sources)]
    pub from: Option<String>,
    /// Or a column's rows: 1..40, or * (all but header rows).
    pub rows: Option<String>,
}

/// A cell's place, from 0.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    #[arg(minimum = 0)]
    pub row: i64,
    #[arg(minimum = 0)]
    pub col: i64,
}

/// A key pair `sync` matches first: the table's key and the data's.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct KeyPair {
    pub table: String,
    pub data: String,
}

/// `word_edit_table`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "word_edit_table",
    description = include_str!("word_edit_table.md"),
    group = "word_objects",
    mutates,
    retry_key = false,
    paths_first("from")
)]
pub struct WordEditTable {
    /// Document id.
    pub doc: String,
    /// Table (t:...).
    #[arg(refs)]
    pub table: String,
    /// What to do.
    pub action: TableAction,
    /// set_cells: the cells.
    pub cells: Option<Vec<CellArg>>,
    /// set_cells, sync: rows of d:<id> or t:<table>, from row index.
    #[arg(rename = "from", sources)]
    pub from: Option<String>,
    /// from's columns, in order (sync: per table column, default by labels). Default all.
    pub from_columns: Option<Vec<String>>,
    /// sync: key column label or index. Default first.
    pub key: Option<String>,
    /// sync: delete rows of gone keys.
    pub remove: Option<bool>,
    /// sync: also match keys loosely (case, spacing, separators, 05=5); whose spelling stays.
    pub loose: Option<Loose>,
    /// sync: key pairs matched first.
    #[arg(map_as_pairs, rename = "match")]
    pub pairs: Option<Vec<KeyPair>>,
    /// from's header texts; [] none. Default column names.
    pub header: Option<Vec<String>>,
    /// Dates of from as e.g. d mmmm yyyy or dd/mm/yyyy.
    pub date_format: Option<String>,
    /// insert_*, delete_*, copy_row: row or column from 0. Default the last. set_cells from: first row.
    #[arg(minimum = 0)]
    pub index: Option<i64>,
    /// set_cells from: add rows the data needs.
    pub grow: Option<bool>,
    /// insert_*, copy_row: relative to index. Default after.
    pub side: Option<Side>,
    /// merge, split: first cell, from 0.
    pub first: Option<Position>,
    /// merge, split: first cell, from 0.
    pub last: Option<Position>,
    /// split: into rows. Default 1.
    #[arg(minimum = 1, maximum = 63)]
    pub rows: Option<i64>,
    /// split: columns. Default span or 2
    #[arg(minimum = 1, maximum = 63, names_as = "from_columns")]
    pub columns: Option<i64>,
}
