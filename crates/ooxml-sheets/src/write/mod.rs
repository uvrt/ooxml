//! Owns: [`WorkbookWriter`] -- a workbook written cell by cell, as ooxml-edit's
//! `sheets/workbook.py` writes the cells a chart edit changes (`Worksheet.set_value`, then
//! `sync_tables`, then `Workbook.finish`): a number, a text (a shared string, or inline in
//! a workbook without a table) or nothing put in a cell found by its sheet's name; the
//! sheet's tables named after their headers again; and at the end the shared strings'
//! counts and each written sheet's `dimension` brought up to date, the package saved with
//! only the parts that changed written again.
//!
//! Must not: evaluate or write formulas (a cell holding one is refused, `Unsupported`),
//! write styles or rich text, or change a part that nothing written touches.

mod nodes;
mod sheet;
mod strings;
mod table_shift;
mod tables;

pub use table_shift::{Axis, Shift};

use crate::items::descendants;
use crate::SheetsError;
use crate::{Cell, Value};
use ooxml_opc::{Limits, Package};
use ooxml_xml::{Element, Tree};
use sheet::SheetEdit;
use std::collections::BTreeMap;
use std::sync::Arc;
use strings::Strings;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_OFFICE_DOCUMENT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_SHARED_STRINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";
const REL_WORKSHEET: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const REL_TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";

/// What a cell is given.
#[derive(Debug, Clone, PartialEq)]
pub enum Write {
    /// Nothing: the value goes, the cell's format stays.
    Blank,
    /// Text, stored as a shared string (inline without a table).
    Text(String),
    /// A finite number.
    Number(f64),
}

/// A workbook being written.
#[derive(Debug)]
pub struct WorkbookWriter {
    package: Package,
    main: String,
    main_root: Arc<Element>,
    strings: Option<Strings>,
    /// The sheets opened, by name (`None`: no such sheet).
    sheets: BTreeMap<String, Option<SheetEdit>>,
    /// Tables whose columns were named again, by part.
    tables: BTreeMap<String, Arc<Element>>,
}

impl WorkbookWriter {
    /// The workbook in `bytes`, opened for writing within `limits`.
    ///
    /// # Errors
    /// The package's errors, [`SheetsError::NotAWorkbook`], or a part's XML error.
    pub fn open(bytes: Arc<[u8]>, limits: &Limits) -> Result<WorkbookWriter, SheetsError> {
        let package = Package::open(bytes, limits)?;
        let rels = package.relationships("")?;
        let main = rels
            .targets_of_type(REL_OFFICE_DOCUMENT)
            .next()
            .map(str::to_owned)
            .ok_or(SheetsError::NotAWorkbook)?;
        let main_root = Arc::clone(package.tree(&main)?.ok_or(SheetsError::NotAWorkbook)?.root());
        let strings = match package.relationships(&main)?.targets_of_type(REL_SHARED_STRINGS).next() {
            Some(part) if package.has_part(part) => package
                .tree(part)?
                .map(|tree| Strings::new(part.to_owned(), Arc::clone(tree.root()))),
            _ => None,
        };
        Ok(WorkbookWriter {
            package,
            main,
            main_root,
            strings,
            sheets: BTreeMap::new(),
            tables: BTreeMap::new(),
        })
    }

    /// Whether the workbook has a sheet named `name` (its relationship and part there).
    ///
    /// # Errors
    /// The sheet's XML error, or a sheet without `sheetData`.
    pub fn has_sheet(&mut self, name: &str) -> Result<bool, SheetsError> {
        Ok(self.sheet(name)?.is_some())
    }

    fn sheet(&mut self, name: &str) -> Result<Option<&mut SheetEdit>, SheetsError> {
        if !self.sheets.contains_key(name) {
            let opened = self.open_sheet(name)?;
            self.sheets.insert(name.to_owned(), opened);
        }
        Ok(self.sheets.get_mut(name).and_then(Option::as_mut))
    }

    fn open_sheet(&self, name: &str) -> Result<Option<SheetEdit>, SheetsError> {
        let found = descendants(&self.main_root, "sheet")
            .into_iter()
            .find(|s| s.attribute(None, "name") == Some(name));
        let Some(id) = found.and_then(|s| s.attribute(Some(REL), "id")) else {
            return Ok(None);
        };
        let rels = self.package.relationships(&self.main)?;
        let Some(part) = rels.get(id).and_then(|r| r.target_part.clone()) else {
            return Ok(None);
        };
        match self.package.tree(&part)? {
            None => Ok(None),
            Some(tree) => SheetEdit::new(part, name, Arc::clone(tree.root())).map(Some),
        }
    }

    /// What the cell `cell` of the sheet `name` holds now.
    ///
    /// # Errors
    /// As [`WorkbookWriter::has_sheet`], or a cell that cannot be read.
    pub fn value(&mut self, name: &str, cell: Cell) -> Result<Option<Value>, SheetsError> {
        if self.sheet(name)?.is_none() {
            return Ok(None);
        }
        match self.sheets.get(name).and_then(Option::as_ref) {
            Some(sheet) => sheet.value(cell, self.strings.as_ref()),
            None => Ok(None),
        }
    }

    /// Write `value` into `cell` of the sheet `name` (a new cell styled as `style_from`, where
    /// given and styled), then name its tables' columns after their headers again.
    ///
    /// # Errors
    /// [`SheetsError::NoSheet`] for a sheet that is not there; [`SheetsError::Unsupported`]
    /// for a cell holding a formula or a table this does not read; a reference that cannot
    /// be read.
    pub fn set_value(
        &mut self,
        name: &str,
        cell: Cell,
        value: &Write,
        style_from: Option<Cell>,
    ) -> Result<(), SheetsError> {
        self.write_cell(name, cell, value, style_from)?;
        self.sync_tables(name)
    }

    /// [`WorkbookWriter::set_value`] without naming the tables' columns again (`set_value`
    /// alone; [`WorkbookWriter::sync_tables`] after).
    ///
    /// # Errors
    /// As [`WorkbookWriter::set_value`].
    pub fn write_cell(
        &mut self,
        name: &str,
        cell: Cell,
        value: &Write,
        style_from: Option<Cell>,
    ) -> Result<(), SheetsError> {
        let mut strings = self.strings.take();
        let written = self
            .opened(name)
            .and_then(|sheet| sheet.set_value(cell, value, style_from, &mut strings));
        self.strings = strings;
        written
    }

    /// The sheet `name`, opened.
    fn opened(&mut self, name: &str) -> Result<&mut SheetEdit, SheetsError> {
        self.sheet(name)?.ok_or_else(|| SheetsError::NoSheet(name.to_owned()))
    }

    /// Whether `cell` of the sheet `name` holds no value (`is_empty`: a cell with only a
    /// style counts as empty).
    ///
    /// # Errors
    /// [`SheetsError::NoSheet`]; a cell that cannot be read.
    pub fn is_empty(&mut self, name: &str, cell: Cell) -> Result<bool, SheetsError> {
        self.opened(name)?;
        match self.sheets.get(name).and_then(Option::as_ref) {
            Some(sheet) => sheet.is_empty(cell, self.strings.as_ref()),
            None => Err(SheetsError::NoSheet(name.to_owned())),
        }
    }

    /// Move the cell at `from` of the sheet `name` -- value, formula and style -- onto `to`
    /// (`move`).
    ///
    /// # Errors
    /// [`SheetsError::NoSheet`]; a reference that cannot be read.
    pub fn move_cell(&mut self, name: &str, from: Cell, to: Cell) -> Result<(), SheetsError> {
        self.opened(name)?.move_cell(from, to)
    }

    /// Remove `cell` of the sheet `name`, style and all (`clear`).
    ///
    /// # Errors
    /// [`SheetsError::NoSheet`]; [`SheetsError::Unsupported`] for a cell holding a formula.
    pub fn clear(&mut self, name: &str, cell: Cell) -> Result<(), SheetsError> {
        self.opened(name)?.clear(cell)
    }

    /// Keep the tables of the sheet `name` in step with a line of cells put in or taken out
    /// (`shift_tables`).
    ///
    /// # Errors
    /// [`SheetsError::NoSheet`]; [`SheetsError::Unsupported`] for a table reference this
    /// does not read.
    pub fn shift_tables(&mut self, name: &str, shift: Shift) -> Result<(), SheetsError> {
        let part = self.opened(name)?.part.clone();
        for table in self.package.relationships(&part)?.targets_of_type(REL_TABLE) {
            let Some(root) = self.table_root(table)? else { continue };
            if let Some(changed) = table_shift::shifted(&root, shift)? {
                self.tables.insert(table.to_owned(), Arc::new(changed));
            }
        }
        Ok(())
    }

    fn table_root(&self, part: &str) -> Result<Option<Arc<Element>>, SheetsError> {
        Ok(match self.tables.get(part) {
            Some(root) => Some(Arc::clone(root)),
            None => self.package.tree(part)?.map(|tree| Arc::clone(tree.root())),
        })
    }

    /// Name the columns of the tables of the sheet `name` after their header cells again
    /// (`sync_tables`).
    ///
    /// # Errors
    /// [`SheetsError::Unsupported`] for a table this does not read; a header that cannot be
    /// read.
    pub fn sync_tables(&mut self, name: &str) -> Result<(), SheetsError> {
        let Some(sheet) = self.sheets.get(name).and_then(Option::as_ref) else {
            return Ok(());
        };
        for part in self.package.relationships(&sheet.part)?.targets_of_type(REL_TABLE) {
            let Some(root) = self.table_root(part)? else { continue };
            if let Some(changed) = tables::synced(&root, sheet, self.strings.as_ref())? {
                self.tables.insert(part.to_owned(), Arc::new(changed));
            }
        }
        Ok(())
    }

    /// The workbook's bytes with what was written (`None` where nothing changed): the shared
    /// strings' `count` and `uniqueCount` and each written sheet's `dimension` first brought
    /// up to date (`Workbook.finish`).
    ///
    /// # Errors
    /// A part's XML error, or the package's when it is saved.
    pub fn finish(mut self) -> Result<Option<Vec<u8>>, SheetsError> {
        let written: Vec<&mut SheetEdit> = self.sheets.values_mut().flatten().filter(|s| s.changed).collect();
        let any_sheet = !written.is_empty();
        for sheet in written {
            sheet.measure()?;
        }
        let mut package = self.package.clone();
        let mut changed = false;
        if let Some(mut strings) = self.strings.take() {
            if strings.changed || any_sheet {
                let uses = self.string_uses()?;
                strings.recount(uses);
            }
            if strings.changed {
                package = with_root(&package, &strings.part, &strings.root)?;
                changed = true;
            }
        }
        for sheet in self.sheets.values().flatten().filter(|s| s.changed) {
            package = with_root(&package, &sheet.part, &sheet.root)?;
            changed = true;
        }
        for (part, root) in &self.tables {
            package = with_root(&package, part, root)?;
            changed = true;
        }
        if !changed {
            return Ok(None);
        }
        Ok(Some(package.save()?))
    }

    /// The cells of every worksheet that hold a shared string (`t="s"`).
    fn string_uses(&self) -> Result<usize, SheetsError> {
        let mut total = 0;
        for part in self.package.relationships(&self.main)?.targets_of_type(REL_WORKSHEET) {
            if !self.package.has_part(part) {
                continue;
            }
            let opened = self.sheets.values().flatten().find(|s| s.part == part);
            let root = match opened {
                Some(sheet) => Arc::clone(&sheet.root),
                None => match self.package.tree(part)? {
                    Some(tree) => Arc::clone(tree.root()),
                    None => continue,
                },
            };
            total += descendants(&root, "c")
                .into_iter()
                .filter(|c| c.attribute(None, "t") == Some("s"))
                .count();
        }
        Ok(total)
    }
}

/// `package` with `part`'s root replaced by `root` (its prolog kept).
fn with_root(package: &Package, part: &str, root: &Arc<Element>) -> Result<Package, SheetsError> {
    let tree = package.tree(part)?.unwrap_or_else(|| Tree::from_root(Arc::clone(root)));
    Ok(package.with_tree(part, tree.with_root(Arc::clone(root)))?)
}
