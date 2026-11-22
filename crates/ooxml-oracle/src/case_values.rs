//! Owns: the oracle standing in for the host's references in a `word_edit_table` case, as
//! docx-agent's dispatcher and the tool's adapter take them before the call, against the
//! case's datasets (`datasets`: `{id, columns, types, rows}` each): the datasets the
//! sources name checked first (the call's `from` naming one value, then each cell's `from`:
//! an unknown one refused there, with the datasets there are), then each
//! `{{d:<id>/#<n>/<column>}}` in a cell's text written as its value (`call.facts`; an
//! unknown dataset refused at the text), then -- for `set_cells` -- each cell's one value
//! (its `from`, or the call's for a cell without its own) resolved as `call.fact` resolves
//! it: a [`Written`] at `cells[k].from`, or the refusal of a row or column the dataset lacks,
//! in docx-agent's words, handed to the call (`CallDocuments(sources=...)`).
//!
//! Only the forms the cases use: a dataset's row by number and a column by name, a text or
//! number value without a format; any other is an error of the case, not a guess.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use ooxml_tools_word::Written;
use serde_json::{json, Map, Value};

/// What the host did with a case's references before the call.
pub(crate) struct CaseValues {
    /// The arguments as the call gets them (the texts' references written as their values).
    pub(crate) arguments: Value,
    /// The values written by reference.
    pub(crate) written: Vec<Written>,
    /// The host's refusals of cells' values, as `CallDocuments(sources=...)` takes them.
    pub(crate) refused: Vec<Value>,
}

/// A reference resolved.
enum Resolved {
    /// Its text and record.
    Value(String, Map<String, Value>),
    /// No such dataset: the datasets there are.
    NoDataset(String, Vec<String>),
    /// No such row or column: the message and the whole addresses that would do.
    Refused(String, Vec<String>),
}

/// The case's references resolved (`Ok(Err(refusal))`: the dispatcher refused the call).
///
/// # Errors
/// A message for a reference form the oracle does not resolve.
pub(crate) fn case_values(case: &Value, arguments: Value) -> Result<Result<CaseValues, ToolError>, String> {
    let mut out = CaseValues {
        arguments,
        written: Vec::new(),
        refused: Vec::new(),
    };
    if case.get("tool").and_then(Value::as_str) != Some("word_edit_table") {
        return Ok(Ok(out));
    }
    let datasets: Vec<Value> = case
        .get("datasets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let set_cells = out.arguments.get("action").and_then(Value::as_str) == Some("set_cells");
    let top = out
        .arguments
        .get("from")
        .and_then(Value::as_str)
        .filter(|f| one_value(f).is_some())
        .map(str::to_owned);
    // the datasets the sources name, the call's first (a run of rows the session's to word)
    let mut cells = cells_of(&out.arguments);
    let froms = cells
        .iter()
        .enumerate()
        .filter_map(|(k, cell)| Some((format!("cells[{k}].from"), cell.get("from")?.as_str()?.to_owned())));
    for (field, given) in top.iter().map(|t| ("from".to_owned(), t.clone())).chain(froms) {
        if let Some(refusal) = unknown_dataset(&given, &field, &datasets) {
            return Ok(Err(refusal));
        }
    }
    for (k, cell) in cells.iter_mut().enumerate() {
        let field = format!("cells[{k}].text");
        if let Some(text) = cell.get("text").and_then(Value::as_str).filter(|t| t.contains("{{")) {
            match written_text(text, &field, &datasets)? {
                Ok((text, found)) => {
                    cell.insert("text".into(), Value::from(text));
                    out.written.extend(found);
                }
                Err(refusal) => return Ok(Err(refusal)),
            }
        }
    }
    for (k, cell) in cells.iter().enumerate().filter(|_| set_cells) {
        let given = |key: &str| cell.get(key).and_then(Value::as_str).filter(|v| !v.is_empty());
        let source = match (given("from"), given("text"), &top) {
            (Some(from), None, _) => from.to_owned(),
            (None, None, Some(top)) => top.clone(),
            // none, or text and from (the tool refuses it)
            _ => continue,
        };
        let field = format!("cells[{k}].from");
        match resolve(&source, &datasets)? {
            Resolved::Value(text, record) => out.written.push(written(&field, 0, &source, &text, record)),
            Resolved::NoDataset(..) => return Err(format!("oracle: {source} was checked")),
            Resolved::Refused(message, options) => out.refused.push(json!({
                "given": source, "field": field,
                "refused": {"code": "not_found", "message": message, "options": options},
            })),
        }
    }
    if let Some(Value::Array(listed)) = out.arguments.get_mut("cells") {
        for (place, cell) in listed.iter_mut().zip(cells).filter(|(place, _)| place.is_object()) {
            *place = Value::Object(cell);
        }
    }
    Ok(Ok(out))
}

/// The cells entries the arguments give, as objects (anything else as an empty one).
fn cells_of(arguments: &Value) -> Vec<Map<String, Value>> {
    let listed = arguments.get("cells").and_then(Value::as_array).into_iter().flatten();
    listed.map(|c| c.as_object().cloned().unwrap_or_default()).collect()
}

/// The dispatcher's refusal of a source at `field` naming a dataset there is not.
fn unknown_dataset(given: &str, field: &str, datasets: &[Value]) -> Option<ToolError> {
    let id = given.strip_prefix("d:")?.split(['/', '[']).next()?;
    if dataset(datasets, id).is_some() {
        return None;
    }
    Some(no_dataset(format!("no dataset d:{id}"), ids(datasets), field))
}

fn no_dataset(message: String, options: Vec<String>, field: &str) -> ToolError {
    let call = TryCall {
        tool: "word_edit_table".into(),
        arguments: Map::new(),
        fill: Vec::new(),
    };
    let way = WayOut::names(options).unwrap_or_else(|| WayOut::call(call));
    ToolError::refusal(Refusal::NotFound, message, way).at(field)
}

/// `text` with each `{{reference}}` written as its value, and the values; the dispatcher's
/// refusal of an unknown dataset at `field`.
fn written_text(
    text: &str,
    field: &str,
    datasets: &[Value],
) -> Result<Result<(String, Vec<Written>), ToolError>, String> {
    let mut out = String::new();
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("{{") {
        let Some(end) = rest[at..].find("}}").map(|e| at + e) else {
            break;
        };
        out.push_str(&rest[..at]);
        let address = rest[at + 2..end].trim();
        match resolve(address, datasets)? {
            Resolved::Value(value, record) => {
                found.push(written(field, out.chars().count(), address, &value, record));
                out.push_str(&value);
            }
            Resolved::NoDataset(message, options) => return Ok(Err(no_dataset(message, options, field))),
            Resolved::Refused(..) => return Err(format!("oracle: {address} in a text names no value")),
        }
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    Ok(Ok((out, found)))
}

/// One value written for `address` at `start` of the argument `field`: its record and brief
/// as `textfacts.Fact` gives them.
fn written(field: &str, start: usize, address: &str, text: &str, record: Map<String, Value>) -> Written {
    let mut brief = Map::new();
    brief.insert("from".into(), Value::from(address));
    brief.insert("text".into(), Value::from(text));
    Written {
        field: field.to_owned(),
        start,
        end: start + text.chars().count(),
        text: text.to_owned(),
        record,
        brief,
        markdown: None,
    }
}

/// A dataset's id, row number and column name of `address` (`d:<id>/#<n>/<column>`).
fn one_value(address: &str) -> Option<(&str, usize, &str)> {
    let rest = address.strip_prefix("d:")?;
    let mut parts = rest.split('/');
    let (id, row, column) = (parts.next()?, parts.next()?, parts.next()?);
    let row = row.strip_prefix('#')?.parse().ok().filter(|n| *n > 0)?;
    (parts.next().is_none()).then_some((id, row, column))
}

fn dataset<'d>(datasets: &'d [Value], id: &str) -> Option<&'d Value> {
    datasets
        .iter()
        .find(|d| d.get("id").and_then(Value::as_str) == Some(id))
}

/// The datasets' addresses, in the order defined.
fn ids(datasets: &[Value]) -> Vec<String> {
    datasets
        .iter()
        .filter_map(|d| d.get("id").and_then(Value::as_str))
        .map(|d| format!("d:{d}"))
        .collect()
}

/// The value `address` names, as `call.fact` gives it (a refusal with `with_rows`' whole
/// addresses).
fn resolve(address: &str, datasets: &[Value]) -> Result<Resolved, String> {
    let unsupported = || format!("oracle: the reference {address:?} is no form the oracle resolves");
    let (id, row, column) = one_value(address).ok_or_else(unsupported)?;
    let Some(found) = dataset(datasets, id) else {
        return Ok(Resolved::NoDataset(format!("no dataset d:{id}"), ids(datasets)));
    };
    let columns: Vec<&str> = found
        .get("columns")
        .and_then(Value::as_array)
        .ok_or_else(unsupported)?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let rows = found.get("rows").and_then(Value::as_array).ok_or_else(unsupported)?;
    let Some(at) = columns.iter().position(|c| *c == column) else {
        let options = columns.iter().map(|c| format!("d:{id}/#{row}/{c}")).collect();
        return Ok(Resolved::Refused(
            format!("d:{id} has no column {}", repr_str(column)),
            options,
        ));
    };
    if row > rows.len() {
        let mut options = vec![
            format!("d:{id}/#1/{column}"),
            format!("d:{id}/#{}/{column}", rows.len()),
        ];
        options.dedup();
        return Ok(Resolved::Refused(
            format!("d:{id} has {} rows; there is no row #{row}", rows.len()),
            options,
        ));
    }
    let text = match rows.get(row - 1).and_then(|r| r.get(at)) {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.fract() != 0.0 && f.abs() < 1e15 => f.to_string(),
            _ => return Err(unsupported()),
        },
        _ => return Err(unsupported()),
    };
    let mut record = Map::new();
    record.insert("source".into(), Value::from(address));
    record.insert("text".into(), Value::from(text.as_str()));
    Ok(Resolved::Value(text, record))
}
