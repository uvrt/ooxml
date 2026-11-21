//! Owns: [`SheetsError`].
//! Must not: hide which part or reference an error is about.

use crate::AddressError;
use ooxml_opc::OpcError;
use thiserror::Error;

/// Why a workbook, or what a reader asked of it, could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SheetsError {
    /// The package, or one of its parts' XML.
    #[error(transparent)]
    Package(#[from] OpcError),
    /// The package has no main part (no `officeDocument` relationship to a part it holds).
    #[error("not a workbook: no main part")]
    NotAWorkbook,
    /// A sheet without `sheetData`.
    #[error("sheet {0:?} has no sheetData")]
    NoSheetData(String),
    /// A row whose `r` is not a number.
    #[error("{part}: row number {text:?}")]
    RowNumber {
        /// The sheet's part.
        part: String,
        /// The `r` as written.
        text: String,
    },
    /// A cell's `r`, or a reference asked for.
    #[error(transparent)]
    Address(#[from] AddressError),
    /// A shared-string cell whose value is not an index.
    #[error("{part}: shared string index {text:?}")]
    SharedStringIndex {
        /// The sheet's part.
        part: String,
        /// The value as written.
        text: String,
    },
    /// A shared-string index the table does not have (or no table at all).
    #[error("shared string {0} does not exist")]
    NoSharedString(i64),
    /// A sheet asked for by name that the workbook does not have.
    #[error("its workbook has no sheet {0:?}")]
    NoSheet(String),
    /// What the writer does not do yet (a formula overwritten, a table whose reference
    /// cannot be read): the caller leaves such a write to a host that does.
    #[error("not supported yet: {0}")]
    Unsupported(String),
}
