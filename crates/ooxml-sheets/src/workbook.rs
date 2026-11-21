//! Owns: [`Workbook`] -- a SpreadsheetML package opened for reading: its main part, its
//! sheets found by name through the main part's relationships, its shared strings (read once,
//! when a cell first needs one) and its date system.
//!
//! Must not: change the package, or read a sheet's cells (that is [`Worksheet`]'s).

use crate::items::{child, children, descendants, item_text};
use crate::{SheetsError, Worksheet};
use ooxml_opc::{Limits, Package};
use ooxml_xml::view::py_space;
use ooxml_xml::Tree;
use std::sync::{Arc, OnceLock};

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_OFFICE_DOCUMENT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_SHARED_STRINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";

/// A workbook, opened for reading. Cheap to share: one package version, its main part's
/// tree, and the shared strings once read.
#[derive(Debug)]
pub struct Workbook {
    package: Package,
    part: String,
    tree: Tree,
    /// The shared strings' texts, in table order; `None` without a table.
    strings: OnceLock<Result<Option<Arc<[String]>>, SheetsError>>,
}

impl Workbook {
    /// Open an `.xlsx` package from its bytes within `limits`.
    ///
    /// # Errors
    /// The package's own errors, or [`SheetsError::NotAWorkbook`].
    pub fn open(bytes: Arc<[u8]>, limits: &Limits) -> Result<Workbook, SheetsError> {
        Workbook::from_package(Package::open(bytes, limits)?)
    }

    /// The workbook a package holds: its main part (the package's `officeDocument`
    /// relationship), parsed.
    ///
    /// # Errors
    /// [`SheetsError::NotAWorkbook`] without a main part, or its XML's error.
    pub fn from_package(package: Package) -> Result<Workbook, SheetsError> {
        let rels = package.relationships("")?;
        let part = rels
            .targets_of_type(REL_OFFICE_DOCUMENT)
            .next()
            .map(str::to_owned)
            .ok_or(SheetsError::NotAWorkbook)?;
        let tree = package.tree(&part)?.ok_or(SheetsError::NotAWorkbook)?;
        Ok(Workbook {
            package,
            part,
            tree,
            strings: OnceLock::new(),
        })
    }

    /// The package.
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// The main part, `xl/workbook.xml`.
    pub fn part(&self) -> &str {
        &self.part
    }

    /// The sheets' names, in the workbook's order.
    pub fn sheet_names(&self) -> Vec<String> {
        descendants(self.tree.root(), "sheet")
            .into_iter()
            .map(|s| s.attribute(None, "name").unwrap_or_default().to_owned())
            .collect()
    }

    /// The sheet named `name` (the first of that name, names compared exactly); `None` when
    /// there is none, or its relationship or part is missing.
    ///
    /// # Errors
    /// The sheet part's XML error, or [`SheetsError::NoSheetData`].
    pub fn sheet(&self, name: &str) -> Result<Option<Worksheet<'_>>, SheetsError> {
        let found = descendants(self.tree.root(), "sheet")
            .into_iter()
            .find(|s| s.attribute(None, "name") == Some(name));
        let Some(id) = found.and_then(|s| s.attribute(Some(REL), "id")) else {
            return Ok(None);
        };
        let rels = self.package.relationships(&self.part)?;
        let Some(part) = rels.get(id).and_then(|r| r.target_part.clone()) else {
            return Ok(None);
        };
        match self.package.tree(&part)? {
            None => Ok(None),
            Some(tree) => Worksheet::new(self, part, name, &tree).map(Some),
        }
    }

    /// Whether dates count from 1904 (`workbookPr date1904`) rather than 1900.
    pub fn date1904(&self) -> bool {
        child(self.tree.root(), "workbookPr")
            .and_then(|p| p.attribute(None, "date1904"))
            .is_some_and(|v| matches!(v.trim_matches(py_space).to_lowercase().as_str(), "1" | "true"))
    }

    /// Shared string `index`'s text.
    ///
    /// # Errors
    /// [`SheetsError::NoSharedString`] for an index the table lacks (or without a table), or
    /// the table's XML error.
    pub fn shared_string(&self, index: i64) -> Result<&str, SheetsError> {
        let missing = || SheetsError::NoSharedString(index);
        let table = match self.strings() {
            Ok(table) => table.as_ref().ok_or_else(missing)?,
            Err(error) => return Err(error.clone()),
        };
        let at = usize::try_from(index).map_err(|_| missing())?;
        table.get(at).map(String::as_str).ok_or_else(missing)
    }

    fn strings(&self) -> &Result<Option<Arc<[String]>>, SheetsError> {
        self.strings.get_or_init(|| {
            let rels = self.package.relationships(&self.part)?;
            let Some(part) = rels.targets_of_type(REL_SHARED_STRINGS).next() else {
                return Ok(None);
            };
            let Some(tree) = self.package.tree(part)? else {
                return Ok(None);
            };
            Ok(Some(children(tree.root(), "si").map(item_text).collect()))
        })
    }
}
