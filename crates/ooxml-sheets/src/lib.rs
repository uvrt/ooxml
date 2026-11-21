//! Workbooks, read (docs/architecture/README.md, section 3 `ooxml-sheets`; layer 3): what
//! ooxml-edit's `sheets/workbook.py` reads of a SpreadsheetML package, ported for the chart
//! tools first (a chart's embedded workbook, read cell by cell through its series' formulas)
//! and kept general for the sheets phase to build on.
//!
//! Owns: A1 references and chart formulas ([`address`]); a workbook opened from its package,
//! its sheets found by name through the main part's relationships, its shared strings and
//! its date system ([`Workbook`]); a sheet's cells read by position ([`Worksheet`],
//! [`Value`]).
//!
//! Beside the reading, [`write`]: a workbook written cell by cell ([`WorkbookWriter`]), as
//! ooxml-edit writes the cells a chart edit changes.
//!
//! Must not: evaluate a formula, or hold state beyond what one opened
//! workbook memoises: every value is `Send + Sync`, and nothing here depends on an
//! interpreter lock.
#![forbid(unsafe_code)]

pub mod address;
mod error;
mod items;
mod sheet;
mod workbook;
pub mod write;

pub use address::{
    column_index, column_letters, parse_formula, AddressError, Area, Cell, SheetRange, MAX_COLUMN, MAX_ROW,
};
pub use error::SheetsError;
pub use sheet::{Value, Worksheet};
pub use workbook::Workbook;
pub use write::{WorkbookWriter, Write};

/// SpreadsheetML's main namespace (transitional), the one ooxml-edit reads.
pub const SML_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
