//! Owns: what a chart's embedded workbook holds for its ranges (ooxml-edit's
//! `Chart.workbook_values`, `charts/chart.py`): the workbook behind the chart's
//! `c:externalData` (`workbook_part`: none, a missing relationship, a workbook linked from
//! outside or an OLE object each say why), opened with `ooxml-sheets`, and each series'
//! name, categories and values read cell by cell through its formulas.
//!
//! Must not: change the chart or the workbook. A workbook that cannot be read, a formula
//! `ooxml-sheets` refuses or a cell it cannot read is [`Unreadable`]: ooxml-edit words those
//! as its own exceptions' texts, so the caller leaves such a read to the host.

use ooxml_chart::data::{series_references, Reference};
use ooxml_opc::{Limits, Package};
use ooxml_sheets::{parse_formula, Cell, SheetRange, Value as CellValue, Workbook};
use ooxml_xml::view::{attr_ns, child};
use ooxml_xml::Element;
use serde_json::{json, Map, Value};

/// A package relationship: an embedded package (a workbook).
const REL_PACKAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/package";
/// The relationships namespace of a part's `r:id`.
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// At most this many cells read from one range: past it the read is left to the host.
const MOST_CELLS: u64 = ooxml_chart::model::MAX_CACHE_POINTS as u64;

/// A workbook (or a range of it) this port does not read as ooxml-edit words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unreadable(pub &'static str);

/// The workbook part behind the chart part `chart_part` (whose tree is `root`) -- `Ok` --
/// or why there is none, in ooxml-edit's words (`document`: what the package is called).
///
/// # Errors
/// [`Unreadable`] where the chart part's relationships cannot be read.
pub(crate) fn workbook_part(
    package: &Package,
    chart_part: &str,
    root: &Element,
    document: &str,
) -> Result<Result<String, String>, Unreadable> {
    let Some(node) = child(root, "externalData") else {
        return Ok(Err("the chart has no workbook".to_owned()));
    };
    let rels = package
        .relationships(chart_part)
        .map_err(|_| Unreadable("edit_chart read: a chart part's relationships that cannot be read"))?;
    let Some(rel) = rels.get(attr_ns(node, NS_R, "id").unwrap_or("")) else {
        return Ok(Err("its workbook relationship is missing".to_owned()));
    };
    if rel.external {
        return Ok(Err(format!(
            "its workbook is linked from outside the {document} ({})",
            rel.target
        )));
    }
    Ok(match rel.target_part.as_deref() {
        Some(part) if rel.rel_type == REL_PACKAGE && package.has_part(part) => Ok(part.to_owned()),
        _ => Err("its data is embedded as an OLE object, not as a workbook".to_owned()),
    })
}

/// `workbook_values`: `{"workbook", "sheet", "categories", "series"}`, or with `workbook`
/// `null` and the reason under `missing` where there is no embedded workbook.
///
/// # Errors
/// [`Unreadable`] where the workbook, a formula or a cell cannot be read.
pub(crate) fn workbook_values(
    package: &Package,
    chart_part: &str,
    root: &Element,
    document: &str,
) -> Result<Value, Unreadable> {
    let mut out = Map::new();
    let part = match workbook_part(package, chart_part, root, document)? {
        Ok(part) => part,
        Err(reason) => {
            out.insert("workbook".into(), Value::Null);
            out.insert("sheet".into(), Value::Null);
            out.insert("categories".into(), Value::Null);
            out.insert("series".into(), json!([]));
            out.insert("missing".into(), Value::from(reason));
            return Ok(Value::Object(out));
        }
    };
    let bytes = package
        .read(&part)
        .ok()
        .flatten()
        .ok_or(Unreadable("edit_chart read: an embedded workbook that cannot be read"))?;
    let workbook = Workbook::open(bytes, &Limits::default())
        .map_err(|_| Unreadable("edit_chart read: an embedded workbook that cannot be read"))?;
    let reader = Reader { workbook: &workbook };
    let mut categories: Option<Value> = None;
    let mut fallback: Option<Value> = None;
    let mut series = Vec::new();
    let mut sheets: Vec<String> = Vec::new();
    for each in series_references(root) {
        if categories.is_none() && matches!(each.categories, Reference::Formula(_)) {
            categories = reader.read(&each.categories, false)?;
        } else if fallback.is_none() && each.categories != Reference::Absent {
            fallback = reader.read(&each.categories, false)?;
        }
        let mut one = Map::new();
        one.insert("name".into(), reader.read(&each.name, true)?.unwrap_or(Value::Null));
        one.insert(
            "values".into(),
            reader.read(&each.values, false)?.unwrap_or(Value::Null),
        );
        series.push(Value::Object(one));
        if let Some(range) = range_of(&each.values)? {
            if !sheets.contains(&range.sheet) {
                sheets.push(range.sheet);
            }
        }
    }
    let sheet = match sheets.len() {
        0 => Value::Null,
        1 => Value::from(sheets.remove(0)),
        _ => Value::from(sheets),
    };
    out.insert("workbook".into(), Value::from(part));
    out.insert("sheet".into(), sheet);
    out.insert("categories".into(), categories.or(fallback).unwrap_or(Value::Null));
    out.insert("series".into(), Value::Array(series));
    Ok(Value::Object(out))
}

/// A reference's formula as one range on one sheet (`None`: no formula, or not one range).
fn range_of(reference: &Reference) -> Result<Option<SheetRange>, Unreadable> {
    let Some(formula) = reference.formula() else {
        return Ok(None);
    };
    parse_formula(formula).map_err(|_| Unreadable("edit_chart read: a chart formula past the sheet's last column"))
}

/// Reads ranges of one workbook.
struct Reader<'w> {
    workbook: &'w Workbook,
}

impl Reader<'_> {
    /// `{"ref", "values"}` (`single`: `{"ref", "value"}`, its first cell) for a source with a
    /// reference; `None` for one without.
    fn read(&self, reference: &Reference, single: bool) -> Result<Option<Value>, Unreadable> {
        let key = if single { "value" } else { "values" };
        if *reference == Reference::Absent {
            return Ok(None);
        }
        let mut out = Map::new();
        out.insert("ref".into(), reference.formula().map_or(Value::Null, Value::from));
        let range = range_of(reference)?;
        let sheet = match &range {
            Some(range) => self
                .workbook
                .sheet(&range.sheet)
                .map_err(|_| Unreadable("edit_chart read: a workbook sheet that cannot be read"))?,
            None => None,
        };
        let (Some(range), Some(sheet)) = (range, sheet) else {
            out.insert(key.into(), Value::Null);
            return Ok(Some(Value::Object(out)));
        };
        let cell = |at: Cell| -> Result<Value, Unreadable> {
            let value = sheet
                .value(at)
                .map_err(|_| Unreadable("edit_chart read: a workbook cell that cannot be read"))?;
            cell_json(value)
        };
        let area = range.area;
        let value = if single {
            cell(area.first)?
        } else {
            if u64::from(area.rows()) * u64::from(area.columns()) > MOST_CELLS {
                return Err(Unreadable(
                    "edit_chart read: a workbook range past the points a chart reads",
                ));
            }
            if range.is_line() {
                let mut values = Vec::new();
                for k in 0..range.length() {
                    values.push(range.cell(k).map_or(Ok(Value::Null), cell)?);
                }
                Value::Array(values)
            } else {
                let mut rows = Vec::new();
                for row in area.first.row..=area.last.row {
                    let mut cells = Vec::new();
                    for column in area.first.column..=area.last.column {
                        cells.push(cell(Cell { row, column })?);
                    }
                    rows.push(Value::Array(cells));
                }
                Value::Array(rows)
            }
        };
        out.insert(key.into(), value);
        Ok(Some(Value::Object(out)))
    }
}

/// A cell's value as Python's JSON writes it: text, a float, an integer or a boolean,
/// `null` for none.
///
/// # Errors
/// [`Unreadable`] for a number JSON cannot hold as Python writes it (not finite, or an
/// integer past `i64`).
fn cell_json(value: Option<CellValue>) -> Result<Value, Unreadable> {
    let unwritable = Unreadable("edit_chart read: a workbook number JSON does not hold as Python writes it");
    Ok(match value {
        None => Value::Null,
        Some(CellValue::Text(text)) => Value::from(text),
        Some(CellValue::Bool(on)) => Value::Bool(on),
        Some(CellValue::Number(number)) if number.is_finite() => Value::from(number),
        Some(CellValue::Integer(number)) if number.abs() < 9.0e18 => {
            #[allow(clippy::cast_possible_truncation)]
            let whole = number as i64;
            Value::from(whole)
        }
        Some(CellValue::Number(_) | CellValue::Integer(_)) => return Err(unwritable),
    })
}

#[cfg(test)]
mod tests {
    use super::{cell_json, Unreadable};
    use ooxml_sheets::Value as CellValue;
    use serde_json::json;

    #[test]
    fn cells_are_written_as_python_writes_them() {
        assert_eq!(cell_json(None), Ok(json!(null)));
        assert_eq!(cell_json(Some(CellValue::Integer(3.0))), Ok(json!(3)));
        assert_eq!(cell_json(Some(CellValue::Number(2.5))), Ok(json!(2.5)));
        assert_eq!(cell_json(Some(CellValue::Bool(true))), Ok(json!(true)));
        assert_eq!(cell_json(Some(CellValue::Text("a".into()))), Ok(json!("a")));
        assert!(matches!(
            cell_json(Some(CellValue::Number(f64::NAN))),
            Err(Unreadable(_))
        ));
        assert!(matches!(cell_json(Some(CellValue::Integer(1e20))), Err(Unreadable(_))));
    }
}
