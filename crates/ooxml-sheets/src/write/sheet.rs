//! Owns: one sheet being written (ooxml-edit's `Worksheet.set_value` and what it keeps in
//! step): a cell found by walking its row, made where there is none (its row made in order),
//! its value replaced by a number, a shared or inline string or nothing, a row's `spans`
//! kept true, an emptied cell (and an emptied row) taken out, and at the end the sheet's
//! `dimension` measured again over every cell.
//!
//! Must not: write a formula, or overwrite one (a cell holding a formula is refused as
//! [`SheetsError::Unsupported`]: ooxml-edit then drops the workbook's calculation chain,
//! which this writer does not port yet).

use super::nodes::{
    append_child, element, element_at, insert_child, places, remove_child, replace_child, text_element, with_attr,
    without_attr,
};
use super::strings::Strings;
use super::Write;
use crate::items::{child, is};
use crate::sheet::read_cell;
use crate::{Area, Cell, SheetsError, Value};
use ooxml_xml::view::py_int;
use ooxml_xml::{Atom, Element, Node};
use std::sync::Arc;

/// A sheet being written: its root, its `sheetData`'s place, and its rows by number.
#[derive(Debug)]
pub(crate) struct SheetEdit {
    /// The part.
    pub(crate) part: String,
    /// The worksheet's root as written so far.
    pub(crate) root: Arc<Element>,
    /// Where `sheetData` is among the root's children.
    data: usize,
    /// Each row's number and place in `sheetData`, in document order.
    rows: Vec<(i64, usize)>,
    /// Whether a cell was written.
    pub(crate) changed: bool,
}

impl SheetEdit {
    /// The sheet `part` whose root is `root`.
    ///
    /// # Errors
    /// [`SheetsError::NoSheetData`] (named `name`), or a row number that is not one.
    pub(crate) fn new(part: String, name: &str, root: Arc<Element>) -> Result<SheetEdit, SheetsError> {
        let data = places(&root, "sheetData")
            .next()
            .map(|(i, _)| i)
            .ok_or_else(|| SheetsError::NoSheetData(name.to_owned()))?;
        let mut sheet = SheetEdit {
            part,
            root,
            data,
            rows: Vec::new(),
            changed: false,
        };
        sheet.index()?;
        Ok(sheet)
    }

    fn sheet_data(&self) -> &Arc<Element> {
        // the place was found when the sheet was opened, and only rows change below it
        element_at(&self.root, self.data).unwrap_or(&self.root)
    }

    fn index(&mut self) -> Result<(), SheetsError> {
        let mut rows = Vec::new();
        let mut number = 0i64;
        for (place, row) in places(self.sheet_data(), "row") {
            number = match row.attribute(None, "r").filter(|r| !r.is_empty()) {
                None => number + 1,
                Some(given) => py_int(given).ok_or_else(|| SheetsError::RowNumber {
                    part: self.part.clone(),
                    text: given.to_owned(),
                })?,
            };
            rows.push((number, place));
        }
        self.rows = rows;
        Ok(())
    }

    /// The place of row `number` (the first of that number).
    fn row_place(&self, number: i64) -> Option<usize> {
        self.rows.iter().find(|(n, _)| *n == number).map(|(_, p)| *p)
    }

    fn row(&self, place: usize) -> Option<&Arc<Element>> {
        element_at(self.sheet_data(), place)
    }

    /// The place of `cell` in its row, and the cell; `None` where there is none.
    fn find(&self, cell: Cell) -> Result<Option<(usize, usize, Arc<Element>)>, SheetsError> {
        let Some(place) = self.row_place(i64::from(cell.row)) else {
            return Ok(None);
        };
        let Some(row) = self.row(place) else { return Ok(None) };
        Ok(cells(row)?
            .into_iter()
            .find(|(column, _, _)| *column == cell.column)
            .map(|(_, at, node)| (place, at, node)))
    }

    /// The cell `cell`'s value, its shared strings looked up in `strings`.
    ///
    /// # Errors
    /// As [`read_cell`].
    pub(crate) fn value(&self, cell: Cell, strings: Option<&Strings>) -> Result<Option<Value>, SheetsError> {
        match self.find(cell)? {
            None => Ok(None),
            Some((_, _, node)) => read_cell(&node, &self.part, &|index| shared(strings, index)),
        }
    }

    /// Write `value` into `cell` (ooxml-edit's `Worksheet.set_value`): a new cell takes the
    /// style of `style_from` where that cell has one; a cell already holding the value is
    /// left as it is.
    ///
    /// # Errors
    /// [`SheetsError::Unsupported`] for a cell holding a formula; a cell or row reference
    /// that cannot be read.
    pub(crate) fn set_value(
        &mut self,
        cell: Cell,
        value: &Write,
        style_from: Option<Cell>,
        strings: &mut Option<Strings>,
    ) -> Result<(), SheetsError> {
        let (place, at, node) = match self.find(cell)? {
            Some((place, at, node)) => {
                if child(&node, "f").is_some() {
                    return Err(SheetsError::Unsupported(format!(
                        "{}: a cell holding a formula",
                        cell.reference()?
                    )));
                }
                if self.holds(&node, value, strings.as_ref())? {
                    return Ok(());
                }
                (place, at, node)
            }
            None => {
                if matches!(value, Write::Blank) {
                    return Ok(());
                }
                let style = match style_from {
                    Some(from) => self.find(from)?.and_then(|(_, _, n)| n.attribute(None, "s").map(str::to_owned)),
                    None => None,
                };
                let reference = cell.reference()?;
                let mut attributes = vec![("r", reference.as_str())];
                if let Some(style) = style.as_deref() {
                    attributes.push(("s", style));
                }
                self.create(cell, &attributes)?
            }
        };
        let mut written = without_values(&node);
        written = match value {
            Write::Blank => without_attr(&written, "t"),
            Write::Text(text) => match strings.as_mut() {
                Some(table) => {
                    let index = table.add(text).to_string();
                    let raw = element(&written, "v", &[], vec![Node::Text(Atom::from(index.as_str()))]);
                    append_child(&with_attr(&written, "t", "s"), raw)
                }
                None => {
                    let inline = element(&written, "is", &[], vec![Node::Element(Arc::new(text_element(&written, text)))]);
                    append_child(&with_attr(&written, "t", "inlineStr"), inline)
                }
            },
            Write::Number(number) => {
                let raw = element(&written, "v", &[], vec![Node::Text(Atom::from(number_text(*number)))]);
                append_child(&without_attr(&written, "t"), raw)
            }
        };
        let emptied = matches!(value, Write::Blank) && written.attribute(None, "s").is_none() && written.children().is_empty();
        let row = self.row(place).cloned().ok_or_else(|| SheetsError::NoSheetData(self.part.clone()))?;
        if emptied {
            let row = remove_child(&row, at);
            let bare = row.child_elements().next().is_none()
                && row.attributes().iter().all(|a| matches!(a.name.local(), "r" | "spans") && a.name.namespace().is_none());
            if bare {
                self.set_data(remove_child(self.sheet_data(), place));
                self.index()?;
            } else {
                self.put_row(place, spans(&row)?);
            }
        } else {
            self.put_row(place, replace_child(&row, at, Node::Element(Arc::new(written))));
        }
        self.changed = true;
        Ok(())
    }

    /// Whether `node` holds `value` already, stored as the same kind (`_holds`).
    fn holds(&self, node: &Element, value: &Write, strings: Option<&Strings>) -> Result<bool, SheetsError> {
        let current = read_cell(node, &self.part, &|index| shared(strings, index))?;
        Ok(match value {
            Write::Blank => current.is_none(),
            Write::Text(text) => {
                let kind = node.attribute(None, "t");
                let rich = match kind {
                    Some("s") => child(node, "v")
                        .and_then(crate::items::leading_text)
                        .and_then(|t| py_int(&t))
                        .is_some_and(|i| strings.is_some_and(|s| s.is_rich(i))),
                    Some("inlineStr") => child(node, "is").is_some_and(|item| child(item, "r").is_some()),
                    _ => false,
                };
                !rich
                    && matches!(kind, Some("s" | "inlineStr" | "str"))
                    && current == Some(Value::Text(text.clone()))
            }
            Write::Number(number) => {
                matches!(node.attribute(None, "t").unwrap_or("n"), "n")
                    && matches!(current, Some(Value::Number(v) | Value::Integer(v)) if v == *number)
            }
        })
    }

    /// Make the cell `cell` with `attributes`, its row made where there is none: the row's
    /// place, the cell's place in it, and the cell.
    fn create(&mut self, cell: Cell, attributes: &[(&str, &str)]) -> Result<(usize, usize, Arc<Element>), SheetsError> {
        let place = match self.row_place(i64::from(cell.row)) {
            Some(place) => place,
            None => self.new_row(i64::from(cell.row))?,
        };
        let row = self.row(place).cloned().ok_or_else(|| SheetsError::NoSheetData(self.part.clone()))?;
        let node = Arc::new(element(&row, "c", attributes, Vec::new()));
        let at = cells(&row)?
            .into_iter()
            .find(|(column, _, _)| *column > cell.column)
            .map(|(_, at, _)| at)
            .or_else(|| places(&row, "extLst").next().map(|(i, _)| i))
            .unwrap_or(row.children().len());
        let row = insert_child(&row, at, Node::Element(Arc::clone(&node)));
        self.put_row(place, spans(&row)?);
        Ok((place, at, node))
    }

    /// A new row `number`, before the first row numbered after it (where the rows ascend; in
    /// document order otherwise), else after the last row: its place.
    fn new_row(&mut self, number: i64) -> Result<usize, SheetsError> {
        let data = Arc::clone(self.sheet_data());
        let ascending = self.rows.iter().zip(self.rows.iter().skip(1)).all(|(a, b)| b.0 > a.0) && self.rows.first().is_none_or(|r| r.0 > 0);
        let before = if ascending {
            self.rows.iter().find(|(n, _)| *n >= number)
        } else {
            self.rows.iter().find(|(n, _)| *n > number)
        };
        let at = match (before, self.rows.last()) {
            (Some((_, place)), _) => *place,
            // after the last row and the text that follows it (lxml's `addnext` keeps a tail)
            (None, Some((_, last))) => {
                let nodes = data.children();
                (last + 1..nodes.len())
                    .find(|&i| !matches!(nodes.get(i), Some(Node::Text(_))))
                    .unwrap_or(nodes.len())
            }
            (None, None) => data.children().len(),
        };
        let row = element(&data, "row", &[("r", &number.to_string())], Vec::new());
        self.set_data(insert_child(&data, at, Node::Element(Arc::new(row))));
        self.index()?;
        Ok(at)
    }

    fn put_row(&mut self, place: usize, row: Element) {
        let data = replace_child(self.sheet_data(), place, Node::Element(Arc::new(row)));
        self.set_data(data);
    }

    fn set_data(&mut self, data: Element) {
        self.root = Arc::new(replace_child(&self.root, self.data, Node::Element(Arc::new(data))));
    }

    /// The `dimension` measured again over every cell (`A1` for a sheet without any), where
    /// the sheet states one (`_update_dimension` after a write).
    ///
    /// # Errors
    /// A cell reference that cannot be read.
    pub(crate) fn measure(&mut self) -> Result<(), SheetsError> {
        let Some((at, dimension)) = places(&self.root, "dimension").next() else {
            return Ok(());
        };
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for (number, place) in &self.rows {
            let Some(row) = self.row(*place) else { continue };
            let found = cells(row)?;
            let (Some(first), Some(last)) = (found.iter().map(|c| c.0).min(), found.iter().map(|c| c.0).max()) else {
                continue;
            };
            let number = u32::try_from(*number).unwrap_or(0);
            bounds = Some(match bounds {
                None => (number, first, number, last),
                Some((r0, c0, r1, c1)) => (r0.min(number), c0.min(first), r1.max(number), c1.max(last)),
            });
        }
        let wanted = match bounds {
            None => "A1".to_owned(),
            Some((r0, c0, r1, c1)) => Area {
                first: Cell { row: r0, column: c0 },
                last: Cell { row: r1, column: c1 },
            }
            .reference()?,
        };
        if dimension.attribute(None, "ref") != Some(wanted.as_str()) {
            let changed = with_attr(dimension, "ref", &wanted);
            self.root = Arc::new(replace_child(&self.root, at, Node::Element(Arc::new(changed))));
        }
        Ok(())
    }
}

/// A shared string's text from `strings`.
fn shared(strings: Option<&Strings>, index: i64) -> Result<String, SheetsError> {
    strings
        .and_then(|s| s.text(index))
        .map(str::to_owned)
        .ok_or(SheetsError::NoSharedString(index))
}

/// A row's cells: each one's column, place in the row, and the cell (a cell without `r` is
/// the one after the cell before it).
fn cells(row: &Element) -> Result<Vec<(u32, usize, Arc<Element>)>, SheetsError> {
    let mut out = Vec::new();
    let mut column = 0u32;
    for (at, node) in places(row, "c") {
        column = match node.attribute(None, "r").filter(|r| !r.is_empty()) {
            None => column.saturating_add(1),
            Some(given) => Cell::parse(given)?.column,
        };
        out.push((column, at, Arc::clone(node)));
    }
    Ok(out)
}

/// `node` without its value (`v`) and inline string (`is`).
fn without_values(node: &Element) -> Element {
    let kept: Vec<Node> = node
        .children()
        .iter()
        .filter(|n| !n.as_element().is_some_and(|e| is(e, "v") || is(e, "is")))
        .cloned()
        .collect();
    node.with_children(ooxml_xml::Children::from_vec(kept))
}

/// `row` with its optional `spans` (first and last used column) true (`_update_spans`).
fn spans(row: &Element) -> Result<Element, SheetsError> {
    if row.attribute(None, "spans").is_none() {
        return Ok(row.clone());
    }
    let columns: Vec<u32> = cells(row)?.into_iter().map(|c| c.0).collect();
    let (Some(low), Some(high)) = (columns.iter().min(), columns.iter().max()) else {
        return Ok(row.clone());
    };
    Ok(with_attr(row, "spans", &format!("{low}:{high}")))
}

/// A number as Excel writes it (ooxml-edit's `number_text`): whole numbers below 10^15 as
/// integers, others as Python's `repr` of the float.
pub(crate) fn number_text(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        // whole and below 10^15: exact as an integer
        #[allow(clippy::cast_possible_truncation)]
        let whole = number as i64;
        return whole.to_string();
    }
    ooxml_base::pyfmt::repr_float(number)
}
