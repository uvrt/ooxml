//! Owns: [`Worksheet`] -- one sheet's cells read by position, as ooxml-edit's
//! `Worksheet.value` reads them -- and [`Value`], what a cell holds.
//!
//! Rows and cells may leave out `r` (each is then the one after the one before). Rows are
//! indexed once, when a cell is first asked for; a row number that repeats keeps its first
//! row. A cell is found by walking its row up to it, so a malformed reference after it is
//! never read.
//!
//! Must not: write cells, or evaluate a formula (a formula's cell holds its cached value).

use crate::items::{child, children, item_text, leading_text};
use crate::{Cell, SheetsError, Workbook};
use ooxml_xml::view::{py_float, py_int, py_space};
use ooxml_xml::{Element, Tree};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// What a cell holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Text: a shared or inline string, a formula's text result, an error's text (`#N/A`),
    /// or a value that is not a number where one was expected, as written.
    Text(String),
    /// A number written with a point or an exponent, or not whole (Python's `float`).
    Number(f64),
    /// A whole number written without a point or exponent (Python's `int`), kept as the
    /// `f64` it was read as: every digit of it is that float's.
    Integer(f64),
    /// A boolean.
    Bool(bool),
}

/// One sheet of a [`Workbook`].
#[derive(Debug)]
pub struct Worksheet<'w> {
    workbook: &'w Workbook,
    part: String,
    name: String,
    data: Arc<Element>,
    rows: OnceLock<Result<HashMap<i64, Arc<Element>>, SheetsError>>,
}

impl<'w> Worksheet<'w> {
    pub(crate) fn new(workbook: &'w Workbook, part: String, name: &str, tree: &Tree) -> Result<Self, SheetsError> {
        let data = tree
            .root()
            .child_elements()
            .find(|e| crate::items::is(e, "sheetData"))
            .cloned()
            .ok_or_else(|| SheetsError::NoSheetData(name.to_owned()))?;
        Ok(Worksheet {
            workbook,
            part,
            name: name.to_owned(),
            data,
            rows: OnceLock::new(),
        })
    }

    /// The sheet's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The sheet's part, `xl/worksheets/sheet1.xml`.
    pub fn part(&self) -> &str {
        &self.part
    }

    /// What `cell` holds; `None` for a cell that is not there or holds no value.
    ///
    /// # Errors
    /// A row or cell reference that cannot be read on the way to it, or a shared-string
    /// cell whose index is not one or not in the table.
    pub fn value(&self, cell: Cell) -> Result<Option<Value>, SheetsError> {
        match self.find(cell)? {
            None => Ok(None),
            Some(node) => self.read(node),
        }
    }

    fn rows(&self) -> Result<&HashMap<i64, Arc<Element>>, SheetsError> {
        let rows = self.rows.get_or_init(|| {
            let mut by_number = HashMap::new();
            let mut number = 0i64;
            for row in self.data.child_elements().filter(|e| crate::items::is(e, "row")) {
                number = match row.attribute(None, "r").filter(|r| !r.is_empty()) {
                    None => number + 1,
                    Some(given) => py_int(given).ok_or_else(|| SheetsError::RowNumber {
                        part: self.part.clone(),
                        text: given.to_owned(),
                    })?,
                };
                by_number.entry(number).or_insert_with(|| Arc::clone(row));
            }
            Ok(by_number)
        });
        rows.as_ref().map_err(Clone::clone)
    }

    fn find(&self, cell: Cell) -> Result<Option<&Element>, SheetsError> {
        let Some(row) = self.rows()?.get(&i64::from(cell.row)) else {
            return Ok(None);
        };
        let mut column = 0u32;
        for node in children(row, "c") {
            column = match node.attribute(None, "r").filter(|r| !r.is_empty()) {
                None => column.saturating_add(1),
                Some(given) => Cell::parse(given)?.column,
            };
            if column == cell.column {
                return Ok(Some(node));
            }
        }
        Ok(None)
    }

    fn read(&self, node: &Element) -> Result<Option<Value>, SheetsError> {
        read_cell(node, &self.part, &|index| self.workbook.shared_string(index).map(str::to_owned))
    }
}

/// What the cell `node` of the sheet `part` holds, its shared strings looked up with
/// `string` (ooxml-edit's `Worksheet._read`).
///
/// # Errors
/// A shared-string cell whose value is not an index, or `string`'s error for one.
pub(crate) fn read_cell(
    node: &Element,
    part: &str,
    string: &dyn Fn(i64) -> Result<String, SheetsError>,
) -> Result<Option<Value>, SheetsError> {
    let kind = node.attribute(None, "t").unwrap_or("n");
    if kind == "inlineStr" {
        return Ok(child(node, "is").map(|item| Value::Text(item_text(item))));
    }
    let Some(text) = child(node, "v").and_then(leading_text) else {
        return Ok(None);
    };
    Ok(Some(match kind {
        "s" => {
            let index = py_int(&text).ok_or_else(|| SheetsError::SharedStringIndex {
                part: part.to_owned(),
                text: text.clone(),
            })?;
            Value::Text(string(index)?)
        }
        "str" | "e" => Value::Text(text),
        "b" => Value::Bool(matches!(text.trim_matches(py_space), "1" | "true")),
        _ => number(text),
    }))
}

/// A numeric cell's value as Python's `float` reads it, whole when written without a point
/// or an exponent; the text itself when it is not a number.
fn number(text: String) -> Value {
    match py_float(&text) {
        None => Value::Text(text),
        Some(value) if value.is_finite() && value.fract() == 0.0 && !text.contains(['e', 'E', '.']) => {
            Value::Integer(value)
        }
        Some(value) => Value::Number(value),
    }
}
