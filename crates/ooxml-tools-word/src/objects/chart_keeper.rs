//! Owns: the embedded workbook a chart edit keeps in step with the chart's caches
//! (ooxml-edit's `_Editor`, `charts/chart.py`): opened when first written (or the reason the
//! chart has none), a sheet found by a formula's sheet name (or the reason it is not there),
//! a point's cell written (`write`), cells moved, cleared and written and the sheet's tables
//! shifted and named again for an edit that changes the data's shape, and at the end the
//! workbook's bytes where it changed -- or the reason the edit changed the cache only.
//!
//! Must not: change the chart part, or decide which cells an edit moves (`chart_block`).

use super::chart_workbook::workbook_part;
use ooxml_base::pyfmt::repr_str;
use ooxml_chart::edit::{PointValue, SourceRef};
use ooxml_opc::{Limits, Package};
use ooxml_sheets::{parse_formula, Cell, SheetRange, SheetsError, Shift, WorkbookWriter, Write};
use ooxml_word_edit::{EditError, PartWrite};
use std::sync::Arc;

/// What docx-agent calls the package in a reason (`edit/charts.py`'s `DOCUMENT`).
const DOCUMENT: &str = "document";

/// The embedded workbook kept in step with the cache: opened when first written, or the
/// reason the edit changed the cache only.
pub(crate) struct Keeper<'p> {
    package: &'p Package,
    chart_part: &'p str,
    opened: bool,
    workbook: Option<(String, WorkbookWriter)>,
    cache_only: Option<String>,
}

impl<'p> Keeper<'p> {
    pub(crate) fn new(package: &'p Package, chart_part: &'p str) -> Keeper<'p> {
        Keeper {
            package,
            chart_part,
            opened: false,
            workbook: None,
            cache_only: None,
        }
    }

    fn note(&mut self, reason: String) {
        if self.cache_only.is_none() {
            self.cache_only = Some(reason);
        }
    }

    /// Open the workbook the first time it is written (`_Editor.workbook`): a reason the
    /// chart has none replaces any noted before, as ooxml-edit sets it.
    fn open(&mut self) -> Result<(), EditError> {
        if !self.opened {
            self.opened = true;
            let root = self
                .package
                .tree(self.chart_part)?
                .ok_or_else(|| EditError::no_part(self.chart_part.to_owned()))?;
            let found = workbook_part(self.package, self.chart_part, root.root(), DOCUMENT)
                .map_err(|why| not_supported(why.0))?;
            match found {
                Err(reason) => self.cache_only = Some(reason),
                Ok(part) => {
                    let bytes = self
                        .package
                        .read(&part)?
                        .ok_or_else(|| EditError::no_part(part.clone()))?;
                    // ooxml-edit words a workbook it cannot open with the exception's text
                    let writer = WorkbookWriter::open(bytes, &Limits::default())
                        .map_err(|_| not_supported("an embedded workbook that cannot be read"))?;
                    self.workbook = Some((part, writer));
                }
            }
        }
        Ok(())
    }

    /// Why the edit changes the cache only, where it does (`_Editor.cache_only`).
    pub(crate) fn cache_only(&self) -> Option<&str> {
        self.cache_only.as_deref()
    }

    /// Whether the sheet `range` points into is there to write (`_Editor.sheet`, for a
    /// source with a formula): the workbook opened, the reason noted where it is not.
    pub(crate) fn sheet(&mut self, range: &SheetRange) -> Result<bool, EditError> {
        self.open()?;
        let has_sheet = match self.workbook.as_mut() {
            Some((_, workbook)) => workbook.has_sheet(&range.sheet).map_err(sheets_error)?,
            None => return Ok(false),
        };
        if !has_sheet {
            self.note(format!("its workbook has no sheet {}", repr_str(&range.sheet)));
        }
        Ok(has_sheet)
    }

    fn writer(&mut self) -> Result<&mut WorkbookWriter, EditError> {
        self.workbook
            .as_mut()
            .map(|(_, w)| w)
            .ok_or_else(|| not_supported("a chart workbook written before it was opened"))
    }

    /// Whether `cell` of `sheet` holds no value (`is_empty`).
    pub(crate) fn is_empty(&mut self, sheet: &str, cell: Cell) -> Result<bool, EditError> {
        self.writer()?.is_empty(sheet, cell).map_err(sheets_error)
    }

    /// Move a cell (`move`).
    pub(crate) fn move_cell(&mut self, sheet: &str, from: Cell, to: Cell) -> Result<(), EditError> {
        self.writer()?.move_cell(sheet, from, to).map_err(sheets_error)
    }

    /// Remove a cell (`clear`).
    pub(crate) fn clear(&mut self, sheet: &str, cell: Cell) -> Result<(), EditError> {
        self.writer()?.clear(sheet, cell).map_err(sheets_error)
    }

    /// Write a cell (`set_value`, the tables named again after, by [`Keeper::sync`]).
    pub(crate) fn set(
        &mut self,
        sheet: &str,
        cell: Cell,
        value: &Write,
        style_from: Option<Cell>,
    ) -> Result<(), EditError> {
        self.writer()?
            .write_cell(sheet, cell, value, style_from)
            .map_err(sheets_error)
    }

    /// Keep the sheet's tables in step with a line put in or taken out (`shift_tables`).
    pub(crate) fn shift(&mut self, sheet: &str, shift: Shift) -> Result<(), EditError> {
        self.writer()?.shift_tables(sheet, shift).map_err(sheets_error)
    }

    /// Name the sheet's tables' columns after their headers again (`sync_tables`).
    pub(crate) fn sync(&mut self, sheet: &str) -> Result<(), EditError> {
        self.writer()?.sync_tables(sheet).map_err(sheets_error)
    }

    /// Write point `index` of `source` into its cell (`_Editor.write`).
    pub(crate) fn write(
        &mut self,
        source: Option<&SourceRef>,
        index: usize,
        value: &PointValue,
    ) -> Result<(), EditError> {
        let Some(formula) = source.and_then(|s| s.formula.as_deref()) else {
            return Ok(()); // a literal: nothing in the workbook to keep in step
        };
        let range = match parse_formula(formula) {
            Ok(Some(range)) => range,
            Ok(None) => {
                self.note(format!("the formula {} is not a single range", repr_str(formula)));
                return Ok(());
            }
            Err(_) => return Err(not_supported("a formula ooxml-sheets cannot read")),
        };
        if !self.sheet(&range)? {
            return Ok(());
        }
        let cell = u32::try_from(index)
            .ok()
            .filter(|i| range.is_line() && *i < range.length())
            .and_then(|i| range.cell(i));
        let Some(cell) = cell else {
            let spelled = range
                .formula()
                .map_err(|_| not_supported("a formula ooxml-sheets cannot spell"))?;
            self.note(format!("the formula {spelled} does not have a cell for point {index}"));
            return Ok(());
        };
        let given = match value {
            PointValue::Blank => Write::Blank,
            PointValue::Text(text) => Write::Text(text.clone()),
            PointValue::Number(number) => Write::Number(*number),
        };
        self.writer()?
            .set_value(&range.sheet, cell, &given, None)
            .map_err(sheets_error)
    }

    /// The workbook's bytes among `writes` where it changed (its part among `read` where it
    /// was opened); the reason the edit changed the cache only, if it did.
    pub(crate) fn finish(
        self,
        writes: &mut Vec<PartWrite>,
        read: &mut Vec<String>,
    ) -> Result<Option<String>, EditError> {
        if let Some((part, writer)) = self.workbook {
            read.push(part.clone());
            if let Some(bytes) = writer.finish().map_err(sheets_error)? {
                writes.push(PartWrite::Bytes(part, Arc::from(bytes)));
            }
        }
        Ok(self.cache_only)
    }
}

/// A chart edit ooxml-edit refuses (`ChartDataError`, `IndexError`), as docx-agent words it:
/// `invalid_arguments` with the message stripped of quotes at its ends.
pub(crate) fn refused(message: &str) -> EditError {
    EditError::Argument {
        field: String::new(),
        message: unquoted(message).to_owned(),
        options: Vec::new(),
    }
}

/// `message` without the quotes a Python `KeyError`'s text has at both ends (one quoted at
/// its start only, `'0.25' is not a number`, keeps them).
fn unquoted(message: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = message.strip_prefix(quote).and_then(|m| m.strip_suffix(quote)) {
            return inner;
        }
    }
    message
}

pub(crate) fn not_supported(what: &str) -> EditError {
    EditError::Unsupported(format!("edit_chart: {what}"))
}

pub(crate) fn sheets_error(error: SheetsError) -> EditError {
    match error {
        SheetsError::Unsupported(what) => not_supported(&what),
        // ooxml-edit words a workbook it cannot write with its exceptions' texts
        _ => not_supported("an embedded workbook ooxml-sheets cannot write"),
    }
}
