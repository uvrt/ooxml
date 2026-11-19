//! A tool's documents as a declared rule (docx-agent's `Tool.documents`): the arguments that
//! name documents, in order, the first one present being the document the call works on. A
//! name is an argument (`doc`, `source_doc`); a path into an object (`reference.doc`) is a
//! `doc` that defaults to the call's own, so it names another document only when it is given,
//! not empty (`""`, a placeholder) and not one named before.
//!
//! Owns: [`named`] (the documents a call's arguments name) and [`path_problem`] (whether a
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
    let mut out: Vec<String> = Vec::new();
    for name in documents {
        if !name.contains('.') {
            if let Some(value) = arguments.get(name).and_then(Value::as_str) {
                out.push(value.to_owned());
            }
            continue;
        }
        let mut parts = name.split('.');
        let first = parts.next().and_then(|part| arguments.get(part));
        let value = parts.try_fold(first, |node, part| Some(node.and_then(|n| n.get(part))));
        if let Some(value) = value.flatten().and_then(Value::as_str) {
            if !value.is_empty() && !out.iter().any(|seen| seen == value) {
                out.push(value.to_owned());
            }
        }
    }
    out
}

/// What is wrong with the declared document `name` in a tool whose arguments are `root`: a
/// path that names no string inside an object (an argument name is checked by its type).
#[must_use]
pub fn path_problem(root: &ObjectNode, name: &str) -> Option<String> {
    let (object, last) = name.rsplit_once('.')?;
    let mut node = root;
    for part in object.split('.') {
        match node.property(part).map(|p| &p.schema.kind) {
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
    use super::named;
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
