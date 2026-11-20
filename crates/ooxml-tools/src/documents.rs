//! A tool's documents as a declared rule (docx-agent's `Tool.documents`): the arguments that
//! name documents, in order, the first one present being the document the call works on. A
//! name is an argument (`doc`, `source_doc`); a path into an object (`reference.doc`) is a
//! `doc` that defaults to the call's own, so it names another document only when it is given,
//! not empty (`""`, a placeholder) and not one named before. A path's step ending in `[]`
//! (`copies[].reference.doc`) is an array: every item's value is read, in order.
//!
//! Owns: [`named`] and [`named_at`] (the documents a call's arguments name) and [`path_problem`] (whether a
//! declared path names a string inside an object).
//!
//! Must not: know which documents are open, or read anything but the call's arguments and
//! the tool's schema.

use crate::schema::{Kind, ObjectNode};
use serde_json::{Map, Value};

/// The documents `arguments` name by the declared `documents`: an argument's value when it
/// is a string; a path's when it is a string neither empty nor named before.
#[must_use]
pub fn named(documents: &[String], arguments: &Map<String, Value>) -> Vec<String> {
    named_at(documents, arguments).into_iter().map(|(_, id)| id).collect()
}

/// [`named`], each with the declared name (argument or path) that names it: the field a
/// refusal of that document points at.
#[must_use]
pub fn named_at<'d>(documents: &'d [String], arguments: &Map<String, Value>) -> Vec<(&'d str, String)> {
    let mut out: Vec<(&str, String)> = Vec::new();
    for name in documents {
        if !name.contains('.') {
            if let Some(value) = arguments.get(name).and_then(Value::as_str) {
                out.push((name, value.to_owned()));
            }
            continue;
        }
        let steps: Vec<&str> = name.split('.').collect();
        let mut found = Vec::new();
        values_at(arguments, &steps, &mut found);
        for value in found {
            if !value.is_empty() && !out.iter().any(|(_, seen)| seen == value) {
                out.push((name, value.to_owned()));
            }
        }
    }
    out
}

/// The strings at the path `steps` below `object`, in order: a step `name[]` reads every
/// item of the array `name`.
fn values_at<'v>(object: &'v Map<String, Value>, steps: &[&str], out: &mut Vec<&'v str>) {
    let Some((step, rest)) = steps.split_first() else {
        return;
    };
    let (name, array) = match step.strip_suffix("[]") {
        Some(name) => (name, true),
        None => (*step, false),
    };
    let Some(value) = object.get(name) else {
        return;
    };
    let items: Vec<&Value> = match (array, value) {
        (true, Value::Array(items)) => items.iter().collect(),
        (true, _) => Vec::new(),
        (false, value) => vec![value],
    };
    for item in items {
        match (rest.is_empty(), item) {
            (true, Value::String(text)) => out.push(text),
            (false, Value::Object(inner)) => values_at(inner, rest, out),
            _ => {}
        }
    }
}

/// What is wrong with the declared document `name` in a tool whose arguments are `root`: a
/// path that names no string inside an object (an argument name is checked by its type).
#[must_use]
pub fn path_problem(root: &ObjectNode, name: &str) -> Option<String> {
    let (object, last) = name.rsplit_once('.')?;
    let mut node = root;
    for part in object.split('.') {
        let kind = match part.strip_suffix("[]") {
            Some(array) => match node.property(array).map(|p| &p.schema.kind) {
                Some(Kind::Array(array)) => Some(&array.items.kind),
                _ => None,
            },
            None => node.property(part).map(|p| &p.schema.kind),
        };
        match kind {
            Some(Kind::Object(inner)) => node = inner,
            _ => return Some(format!("document path {name:?} names no string in an object")),
        }
    }
    match node.property(last).map(|p| &p.schema.kind) {
        Some(Kind::String(_)) => None,
        _ => Some(format!("document path {name:?} names no string in an object")),
    }
}

#[cfg(test)]
mod tests {
    use super::{named, named_at};
    use serde_json::{json, Map, Value};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn map(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    #[test]
    fn a_path_names_another_document_only_when_it_differs() {
        let declared = names(&["doc", "reference.doc"]);
        let call = |reference: Value| named(&declared, &map(json!({"doc": "d1", "reference": reference})));
        assert_eq!(call(json!({"block": "p:1"})), ["d1"]);
        assert_eq!(call(json!({"doc": "", "block": "p:1"})), ["d1"]);
        assert_eq!(call(json!({"doc": "d1", "block": "p:1"})), ["d1"]);
        assert_eq!(call(json!({"doc": "d2", "block": "p:1"})), ["d1", "d2"]);
        assert_eq!(call(json!("p:1")), ["d1"]);
        let at = named_at(
            &declared,
            &map(json!({"doc": "d1", "reference": {"doc": "d2", "block": "p:1"}})),
        );
        assert_eq!(at, [("doc", "d1".to_owned()), ("reference.doc", "d2".to_owned())]);
    }

    #[test]
    fn an_array_step_reads_every_item() {
        let declared = names(&["doc", "copies[].reference.doc"]);
        let copies = json!({"doc": "d1", "copies": [
            {"reference": {"doc": "d2"}}, {"reference": {"block": "p:1"}},
            {"reference": {"doc": "d1"}}, {"reference": {"doc": "d3"}}, {"reference": {"doc": "d2"}}]});
        assert_eq!(named(&declared, &map(copies)), ["d1", "d2", "d3"]);
        assert_eq!(named(&declared, &map(json!({"doc": "d1", "copies": "x"}))), ["d1"]);
    }

    #[test]
    fn an_argument_name_is_read_as_given() {
        let declared = names(&["doc", "source_doc"]);
        assert_eq!(
            named(&declared, &map(json!({"doc": "d1", "source_doc": "d1"}))),
            ["d1", "d1"]
        );
        assert_eq!(named(&declared, &map(json!({"source_doc": "d2"}))), ["d2"]);
    }
}
