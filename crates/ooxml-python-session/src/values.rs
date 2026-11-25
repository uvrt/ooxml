//! Owns: the JSON a host's hooks send and get back for a check of numbers and a scale on a
//! version (`Version.number_texts`, `Version.scale_values`; lane H7e): the request read
//! (`range`, `pages`, a scale's name, steps, targets and `labels`), the reply written (the
//! texts or values, the warnings), and a refusal as its envelope's `error`.
//!
//! Must not: read the document (`ooxml_session::Opened` does).

use ooxml_session::Opened;
use ooxml_session_word::ValuesOf;
use ooxml_tools::ToolError;
use ooxml_tools_word::{NumberTexts, ReadScale, ReadStep, ScaleValues, ValueHandle, WordLayout};
use serde_json::{json, Map, Value};

/// A request that is not one: said as the hooks' internal error.
fn bad(why: &str) -> ToolError {
    ToolError::internal(format!("a values request {why}"), "values:request")
}

fn strings(value: Option<&Value>) -> Result<Option<Vec<String>>, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| i.as_str().map(str::to_owned).ok_or_else(|| bad("range is not text")))
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err(bad("range is not a list")),
    }
}

fn numbers(value: Option<&Value>) -> Result<Option<Vec<usize>>, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| {
                i.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| bad("pages are not numbers"))
            })
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err(bad("pages is not a list")),
    }
}

fn text(value: &Value, key: &str) -> Result<String, ToolError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| bad(&format!("has no {key}")))
}

fn scale(value: &Value) -> Result<ReadScale, ToolError> {
    let steps = value
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("has no steps"))?
        .iter()
        .map(|s| {
            Ok(ReadStep {
                fill: text(s, "fill")?,
                text: text(s, "text")?,
                label: s.get("label").and_then(Value::as_str).map(str::to_owned),
            })
        })
        .collect::<Result<_, ToolError>>()?;
    Ok(ReadScale {
        name: text(value, "name")?,
        steps,
        targets: strings(value.get("targets"))?.unwrap_or_default(),
        labels: value.get("labels").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// The reply: `{"error": envelope}` for a refusal, else `reply(found)`; `null` for a
/// document that is not a Word document.
fn replied<T>(found: Result<Option<T>, ToolError>, reply: impl FnOnce(T) -> Value) -> String {
    match found {
        Ok(Some(found)) => reply(found),
        Ok(None) => Value::Null,
        Err(error) => json!({"error": error.to_json()}),
    }
    .to_string()
}

/// `Version.number_texts`' reply: `{"texts": [{"at", "text", "written", "page"}],
/// "warnings"}`.
pub(crate) fn number_texts(version: &Opened, doc: &str, request: &str, layout: Option<&dyn WordLayout>) -> String {
    let found = serde_json::from_str::<Value>(request)
        .map_err(|e| bad(&e.to_string()))
        .and_then(|request| {
            let range = strings(request.get("range"))?;
            let pages = numbers(request.get("pages"))?;
            version.number_texts(doc, range.as_deref(), pages.as_deref(), layout)
        });
    replied(found, |found: NumberTexts| {
        let texts: Vec<Value> = found
            .texts
            .into_iter()
            .map(|t| json!({"at": t.at, "text": t.text, "written": t.written, "page": t.page}))
            .collect();
        json!({"texts": texts, "warnings": found.warnings})
    })
}

/// `Version.scale_values`' reply: `{"values": [{"target", "text", "given", "now", "exact",
/// "differs", "skip", "page", "handle": [kind, where, ids], "kept", "source"}]}`.
pub(crate) fn scale_values(version: &Opened, doc: &str, request: &str, layout: Option<&dyn WordLayout>) -> String {
    let found = serde_json::from_str::<Value>(request)
        .map_err(|e| bad(&e.to_string()))
        .and_then(|request| {
            let read = scale(request.get("scale").ok_or_else(|| bad("has no scale"))?)?;
            let range = strings(request.get("range"))?;
            let pages = numbers(request.get("pages"))?;
            version.scale_values(doc, &read, range.as_deref(), pages.as_deref(), layout)
        });
    replied(found, |found: ScaleValues| {
        let values: Vec<Value> = found
            .values
            .into_iter()
            .map(|v| {
                let mut out = Map::new();
                out.insert("target".into(), v.target.into());
                out.insert("text".into(), v.text.into());
                out.insert("given".into(), v.given.into());
                out.insert("now".into(), v.now.into());
                out.insert("exact".into(), v.exact.into());
                out.insert("differs".into(), v.differs.into());
                out.insert("skip".into(), v.skip.into());
                out.insert("page".into(), v.page.into());
                let handle = v.handle.map(|(at, ids)| match at {
                    ValueHandle::Range { paragraph, start, end } => json!(["range", [paragraph, start, end], ids]),
                    ValueHandle::Cell(cell) => json!(["cell", cell, ids]),
                });
                out.insert("handle".into(), handle.into());
                out.insert("kept".into(), v.kept.into());
                out.insert("source".into(), v.source.into());
                Value::Object(out)
            })
            .collect();
        json!({ "values": values })
    })
}
