//! Owns: a call's record ([`CallRecord`], ooxml-edit's `logs.py`): its tool, the documents it
//! named, its arguments' digest and shape -- the arguments themselves only where the host
//! logs them --, how it went, how long it took, the version it left, and the roles and
//! context it ran with; and the retry keys a session keeps ([`RetryKeys`]): a changing
//! call's `key`, by document and tool, with the reply its first call gave, so a repeat makes
//! nothing twice.
//!
//! Must not: keep a document's content, or a reply's data in a record.

use ooxml_base::sha256::sha256;
use ooxml_tools::{Done, ToolResult};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

/// One call, as a transcript keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct CallRecord {
    /// The tool, as the host read its name.
    pub tool: String,
    /// The documents it named.
    pub docs: Vec<String>,
    /// Its arguments' digest ([`digest`]).
    pub digest: String,
    /// Its arguments' shape ([`shape`]).
    pub shape: Value,
    /// Whether it succeeded.
    pub ok: bool,
    /// Its error's code, where it failed.
    pub code: Option<String>,
    /// Its summary (its error's message, where it failed).
    pub summary: String,
    /// How long it took.
    pub duration: Duration,
    /// The version it left.
    pub version: Option<u64>,
    /// Its arguments, where the host logs them.
    pub arguments: Option<Map<String, Value>>,
    /// The roles it ran with, where narrower than its session's.
    pub roles: Option<Vec<String>>,
    /// The application's context of the call, where it has one.
    pub context: Option<Map<String, Value>>,
    /// The panic it answered as `internal`, for the log; `None` for every call that did not.
    pub panic: Option<String>,
}

impl CallRecord {
    /// The record as ooxml-edit's `CallRecord.to_json` writes it: its fields in that order,
    /// the duration in seconds to four places, the optional ones only where given.
    #[must_use]
    pub fn to_json(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("tool".into(), Value::from(self.tool.clone()));
        out.insert("docs".into(), Value::from(self.docs.clone()));
        out.insert("digest".into(), Value::from(self.digest.clone()));
        out.insert("shape".into(), self.shape.clone());
        out.insert("ok".into(), Value::from(self.ok));
        out.insert("code".into(), Value::from(self.code.clone()));
        out.insert("summary".into(), Value::from(self.summary.clone()));
        let seconds = (self.duration.as_secs_f64() * 10_000.0).round() / 10_000.0;
        out.insert("duration".into(), Value::from(seconds));
        out.insert("version".into(), Value::from(self.version));
        if let Some(arguments) = &self.arguments {
            out.insert("arguments".into(), Value::Object(arguments.clone()));
        }
        if let Some(roles) = &self.roles {
            out.insert("roles".into(), Value::from(roles.clone()));
        }
        if let Some(context) = &self.context {
            out.insert("context".into(), Value::Object(context.clone()));
        }
        out
    }
}

/// A stable 16-hex-digit hash of `arguments`' canonical JSON (keys sorted, non-ASCII kept).
#[must_use]
pub fn digest(arguments: &Value) -> String {
    let mut text = String::new();
    canonical(arguments, &mut text);
    let mut out = String::with_capacity(16);
    for byte in &sha256(text.as_bytes())[..8] {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::from(key.as_str()).to_string());
                out.push(':');
                if let Some(item) = map.get(key) {
                    canonical(item, out);
                }
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        Value::Number(n) => out.push_str(&ooxml_tools::pyfmt::number(n)),
        other => out.push_str(&other.to_string()),
    }
}

/// `value`'s shape with the content left out: `{"text": "str[42]", "n": "int"}`.
#[must_use]
pub fn shape(value: &Value) -> Value {
    shape_at(value, 0)
}

fn shape_at(value: &Value, depth: usize) -> Value {
    match value {
        Value::Object(map) if depth >= 3 => Value::from(format!("object[{}]", map.len())),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), shape_at(v, depth + 1))).collect()),
        Value::Array(items) => match items.first() {
            Some(first) if depth < 3 => Value::Array(vec![
                shape_at(first, depth + 1),
                Value::from(format!("x{}", items.len())),
            ]),
            _ => Value::from(format!("array[{}]", items.len())),
        },
        Value::String(text) => Value::from(format!("str[{}]", text.chars().count())),
        Value::Bool(_) => Value::from("bool"),
        Value::Null => Value::from("null"),
        Value::Number(n) if n.is_f64() => Value::from("float"),
        Value::Number(_) => Value::from("int"),
    }
}

/// A changing call's retry keys, by document, tool and key: the reply its first call gave.
#[derive(Debug, Default)]
pub struct RetryKeys(BTreeMap<(String, String, String), Box<Done<Value>>>);

impl RetryKeys {
    /// The reply a call of `tool` with `key` on `doc` made before, as a repeat answers:
    /// "already done", nothing new made, what it made then said again.
    #[must_use]
    pub fn repeat(&self, doc: &str, tool: &str, key: &str) -> Option<ToolResult<Value>> {
        let kept = self.0.get(&(doc.to_owned(), tool.to_owned(), key.to_owned()))?;
        let mut done = Done::new(format!(
            "already done with key {}: nothing new was made",
            ooxml_tools::pyfmt::repr_str(key)
        ));
        done.created.clone_from(&kept.created);
        done.changed.clone_from(&kept.changed);
        done.refs.clone_from(&kept.refs);
        done.data.clone_from(&kept.data);
        Some(ToolResult::done(done))
    }

    /// Keep what a call of `tool` with `key` on `doc` made (a success only).
    pub fn keep(&mut self, doc: &str, tool: &str, key: &str, result: &ToolResult<Value>) {
        if let ooxml_tools::Outcome::Done(done) = &result.outcome {
            self.0
                .insert((doc.to_owned(), tool.to_owned(), key.to_owned()), done.clone());
        }
    }

    /// Forget the keys of `doc` (closed).
    pub fn forget(&mut self, doc: &str) {
        self.0.retain(|(d, _, _), _| d != doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_record_keeps_the_shape_and_a_digest_never_the_content() {
        let arguments = json!({"text": "héllo", "n": 3, "x": 1.5, "items": [{"a": [1, 2]}], "flag": true});
        assert_eq!(
            shape(&arguments),
            json!({"text": "str[5]", "n": "int", "x": "float", "items": [{"a": "array[2]"}, "x1"], "flag": "bool"})
        );
        // Python: hashlib.sha256(json.dumps({"b": 1, "a": "é"}, sort_keys=True,
        // ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()[:16]
        assert_eq!(digest(&json!({"b": 1, "a": "é"})).len(), 16);
        assert_eq!(digest(&json!({"b": 1, "a": "é"})), digest(&json!({"a": "é", "b": 1})));
    }
}
