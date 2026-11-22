//! Owns: a sheet's cells moved and cleared, as ooxml-edit's `Worksheet` moves them when a
//! chart's data grows or shrinks (`move`, `clear`, `is_empty`, `_remove_cell`, `_place`): a
//! cell -- value, formula and style -- moved onto a position (what was there removed), a
//! cell removed style and all, an emptied row taken out, and each row's `spans` kept true.
//!
//! Must not: clear a cell holding a formula (ooxml-edit then drops the workbook's calculation
//! chain, which this writer does not port yet: [`SheetsError::Unsupported`]).

use super::super::nodes::{remove_child, with_attr};
use super::super::strings::Strings;
use super::{shared, spans, SheetEdit};
use crate::items::child;
use crate::sheet::read_cell;
use crate::{Cell, SheetsError};
use ooxml_xml::Element;

impl SheetEdit {
    /// Whether `cell` holds no value (`is_empty`: a cell with only a style counts as empty).
    ///
    /// # Errors
    /// A cell that cannot be read.
    pub(crate) fn is_empty(&self, cell: Cell, strings: Option<&Strings>) -> Result<bool, SheetsError> {
        Ok(match self.find(cell)? {
            None => true,
            Some((_, _, node)) => {
                read_cell(&node, &self.part, &|index| shared(strings, index))?.is_none() && child(&node, "f").is_none()
            }
        })
    }

    /// Remove `cell` entirely, style and all (`clear`).
    ///
    /// # Errors
    /// [`SheetsError::Unsupported`] for a cell holding a formula; a reference that cannot be
    /// read.
    pub(crate) fn clear(&mut self, cell: Cell) -> Result<(), SheetsError> {
        if let Some((place, at, node)) = self.find(cell)? {
            if child(&node, "f").is_some() {
                return Err(SheetsError::Unsupported(format!(
                    "{}: a cell holding a formula",
                    cell.reference()?
                )));
            }
            self.take_out(place, at)?;
            self.changed = true;
        }
        Ok(())
    }

    /// Move the cell at `from` -- value, formula and style -- onto `to`, whatever was there
    /// removed (`move`); the sheet counts as written even where `from` holds nothing, as
    /// ooxml-edit marks it.
    ///
    /// # Errors
    /// A reference that cannot be read.
    pub(crate) fn move_cell(&mut self, from: Cell, to: Cell) -> Result<(), SheetsError> {
        if let Some((place, at, _)) = self.find(to)? {
            self.take_out(place, at)?;
        }
        self.changed = true;
        let Some((place, at, node)) = self.find(from)? else {
            return Ok(());
        };
        self.take_out(place, at)?;
        let moved = with_attr(&node, "r", &to.reference()?);
        self.place(to, moved)?;
        Ok(())
    }

    /// Take out the cell at `at` of the row at `place` (`_remove_cell`): a row left with
    /// nothing but its number and spans goes too, otherwise its spans are kept true.
    pub(super) fn take_out(&mut self, place: usize, at: usize) -> Result<(), SheetsError> {
        let row = self
            .row(place)
            .cloned()
            .ok_or_else(|| SheetsError::NoSheetData(self.part.clone()))?;
        let row = remove_child(&row, at);
        if bare(&row) {
            self.set_data(remove_child(self.sheet_data(), place));
            self.index()?;
        } else {
            self.put_row(place, spans(&row)?);
        }
        Ok(())
    }
}

/// A row holding no cells and no attribute but its number and spans.
pub(super) fn bare(row: &Element) -> bool {
    row.child_elements().next().is_none()
        && row
            .attributes()
            .iter()
            .all(|a| matches!(a.name.local(), "r" | "spans") && a.name.namespace().is_none())
}
