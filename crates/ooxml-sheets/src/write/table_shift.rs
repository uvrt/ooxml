//! Owns: a table kept in step with a line of cells put in or taken out (ooxml-edit's
//! `Table.shift`, which `Worksheet.shift_tables` runs on every table of the sheet): a table
//! the change touches grows, shrinks or moves as Excel's own insert and delete would move it
//! -- an insertion just past its end grows it, which is how a chart's data range is
//! extended -- with a column made (`Column<n>`) or taken out where columns change, and its
//! `ref` and its filter's kept the same.
//!
//! Must not: name columns after their headers (`tables::synced` does, after).

use super::nodes::{insert_child, places, remove_child, replace_child, with_attr};
use crate::{Area, Cell, SheetsError};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// The sheet axis a line of cells is put in or taken out along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// A row put in or taken out (points running down a column move along rows).
    Row,
    /// A column.
    Column,
}

/// A line of cells put in (`delta` 1) before `position` or taken out (-1) at it, along
/// `axis`, in the lines `span` of the other axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shift {
    /// The axis.
    pub axis: Axis,
    /// The row or column number.
    pub position: u32,
    /// 1 or -1.
    pub delta: i64,
    /// The first and last line of the other axis the change is in.
    pub span: (u32, u32),
}

/// The table whose root is `root` after `shift`; `None` where nothing changes.
///
/// # Errors
/// [`SheetsError::Unsupported`] for a reference that is not a plain area (ooxml-edit reads
/// some of those leniently).
pub(crate) fn shifted(root: &Element, shift: Shift) -> Result<Option<Element>, SheetsError> {
    let given = root.attribute(None, "ref").unwrap_or("");
    let area = Area::parse(given).map_err(|_| SheetsError::Unsupported(format!("a table's reference {given:?}")))?;
    if area.reference()? != given {
        return Err(SheetsError::Unsupported(format!("a table's reference {given:?}")));
    }
    let (first, last) = (area.first, area.last);
    let (low, high, across) = match shift.axis {
        Axis::Row => (first.row, last.row, (first.column, last.column)),
        Axis::Column => (first.column, last.column, (first.row, last.row)),
    };
    if across.1 < shift.span.0 || across.0 > shift.span.1 {
        return Ok(None); // beside the change, not in its lines
    }
    let (mut low, mut high) = (i64::from(low), i64::from(high));
    let position = i64::from(shift.position);
    let reach = if shift.delta > 0 { high + 1 } else { high };
    if position < low {
        low += shift.delta;
        high += shift.delta;
    } else if position <= reach {
        high += shift.delta;
    } else {
        return Ok(None);
    }
    if high < low {
        return Ok(None);
    }
    let number = |n: i64| {
        u32::try_from(n).map_err(|_| SheetsError::Unsupported(format!("a table moved off the sheet ({given:?})")))
    };
    let (low, high) = (number(low)?, number(high)?);
    let mut table = root.clone();
    let new = match shift.axis {
        Axis::Row => Area {
            first: Cell { row: low, ..first },
            last: Cell { row: high, ..last },
        },
        Axis::Column => {
            let offset = position - i64::from(first.column);
            table = shifted_columns(&table, offset, shift.delta);
            Area {
                first: Cell { column: low, ..first },
                last: Cell { column: high, ..last },
            }
        }
    };
    let reference = new.reference()?;
    if table.attribute(None, "ref") != Some(reference.as_str()) {
        table = with_attr(&table, "ref", &reference);
    }
    let filter = places(&table, "autoFilter")
        .next()
        .filter(|(_, f)| f.attribute(None, "ref") != Some(reference.as_str()))
        .map(|(at, f)| (at, with_attr(f, "ref", &reference)));
    if let Some((at, filter)) = filter {
        table = replace_child(&table, at, Node::Element(Arc::new(filter)));
    }
    Ok(Some(table))
}

/// `_shift_columns`: a column made at `offset` (`delta` 1: the next id, named `Column<id>`)
/// or the one there taken out (-1), and the columns' `count` kept true.
fn shifted_columns(table: &Element, offset: i64, delta: i64) -> Element {
    let Some((container_at, container)) = places(table, "tableColumns").next() else {
        return table.clone();
    };
    let columns: Vec<usize> = places(container, "tableColumn").map(|(i, _)| i).collect();
    let mut changed = (**container).clone();
    if delta > 0 {
        let used = places(container, "tableColumn")
            .filter_map(|(_, c)| c.attribute(None, "id"))
            .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
            .filter_map(|id| id.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        let next = (used + 1).to_string();
        let node = super::nodes::element(
            container,
            "tableColumn",
            &[("id", &next), ("name", &format!("Column{next}"))],
            Vec::new(),
        );
        let node = Node::Element(Arc::new(node));
        let at = match usize::try_from(offset.max(0)).ok().and_then(|o| columns.get(o)) {
            Some(&at) if offset < columns.len() as i64 => at,
            // after the last column and the text after it (lxml's `addnext`)
            _ => match columns.last() {
                Some(&last) => (last + 1..changed.children().len())
                    .find(|&i| !matches!(changed.children().get(i), Some(Node::Text(_))))
                    .unwrap_or(changed.children().len()),
                None => changed.children().len(),
            },
        };
        changed = insert_child(&changed, at, node);
    } else if let Some(&at) = usize::try_from(offset).ok().and_then(|o| columns.get(o)) {
        changed = remove_child(&changed, at);
    }
    let count = places(&changed, "tableColumn").count().to_string();
    changed = with_attr(&changed, "count", &count);
    replace_child(table, container_at, Node::Element(Arc::new(changed)))
}
