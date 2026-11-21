//! Owns: a table's columns named after their header cells (ooxml-edit's `Table.sync_names`,
//! which a write runs on every table of its sheet): each column takes its header's text (a
//! number as Excel writes it), keeps its own name where the header is empty or not text, and
//! is made unique, ignoring case, with a number after it.
//!
//! Must not: move or resize a table, or name a column of a table without a header row.

use super::nodes::{replace_child, with_attr};
use super::sheet::{number_text, SheetEdit};
use super::strings::Strings;
use crate::{Area, Cell, SheetsError, Value};
use ooxml_xml::{Element, Node};
use std::collections::HashSet;
use std::sync::Arc;

/// The table whose root is `root` with its columns named after `sheet`'s header cells;
/// `None` where nothing changes.
///
/// # Errors
/// [`SheetsError::Unsupported`] for a reference that is not a plain area (ooxml-edit reads
/// some of those leniently), or a header cell that cannot be read.
pub(crate) fn synced(
    root: &Element,
    sheet: &SheetEdit,
    strings: Option<&Strings>,
) -> Result<Option<Element>, SheetsError> {
    let given = root.attribute(None, "ref").unwrap_or("");
    let area = Area::parse(given).map_err(|_| SheetsError::Unsupported(format!("a table's reference {given:?}")))?;
    if area.reference()? != given {
        return Err(SheetsError::Unsupported(format!("a table's reference {given:?}")));
    }
    let headers = match root.attribute(None, "headerRowCount") {
        None => 1,
        Some(raw) => ooxml_xml::view::py_int(raw)
            .ok_or_else(|| SheetsError::Unsupported(format!("a table's header row count {raw:?}")))?,
    };
    let Some((at, container)) = root.children().iter().enumerate().find_map(|(i, n)| {
        n.as_element()
            .filter(|e| crate::items::is(e, "tableColumns"))
            .map(|e| (i, e))
    }) else {
        return Ok(None);
    };
    if headers == 0 {
        return Ok(None);
    }
    let mut taken: HashSet<String> = HashSet::new();
    let mut columns = (**container).clone();
    let mut changed = false;
    let places: Vec<usize> = columns
        .children()
        .iter()
        .enumerate()
        .filter(|(_, n)| n.as_element().is_some_and(|e| crate::items::is(e, "tableColumn")))
        .map(|(i, _)| i)
        .collect();
    for (offset, place) in places.into_iter().enumerate() {
        let Some(column) = columns.children().get(place).and_then(Node::as_element).cloned() else {
            continue;
        };
        let header_cell = Cell {
            row: area.first.row,
            column: area.first.column + u32::try_from(offset).unwrap_or(u32::MAX - area.first.column),
        };
        let header = match sheet.value(header_cell, strings)? {
            Some(Value::Text(text)) => Some(text),
            Some(Value::Number(n) | Value::Integer(n)) => Some(number_text(n)),
            _ => None,
        };
        let own = column.attribute(None, "name").map(str::to_owned);
        let mut name = match header {
            Some(text) if !text.trim_matches(ooxml_xml::view::py_space).is_empty() => Some(text),
            _ => own.clone().filter(|n| !n.is_empty()),
        };
        if name.as_ref().is_none_or(|n| taken.contains(&n.to_lowercase())) {
            let base = name.unwrap_or_else(|| "Column".to_owned());
            let mut number = 1;
            while taken.contains(&format!("{base}{number}").to_lowercase()) {
                number += 1;
            }
            name = Some(format!("{base}{number}"));
        }
        let name = name.unwrap_or_default();
        taken.insert(name.to_lowercase());
        if own.as_deref() != Some(name.as_str()) {
            columns = replace_child(
                &columns,
                place,
                Node::Element(Arc::new(with_attr(&column, "name", &name))),
            );
            changed = true;
        }
    }
    Ok(changed.then(|| replace_child(root, at, Node::Element(Arc::new(columns)))))
}
