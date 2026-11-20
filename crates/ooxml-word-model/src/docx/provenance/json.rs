//! Owns: a record written as `ooxml_edit.provenance.dumps` writes it --
//! `json.dumps(record, ensure_ascii=False, separators=(",", ":"))`: no spaces, text as it
//! is (only `"`, `\` and control characters escaped), keys in their order, a float as
//! Python's `repr` writes it.
//!
//! Must not: write JSON for anything but records kept in the document.

use ooxml_base::pyfmt::repr_float;
use serde_json::Value;

/// `value` as docx-agent writes a record.
#[must_use]
pub fn dumps(value: &Value) -> String {
    let mut out = String::new();
    dump(value, &mut out);
    out
}

fn dump(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (k, item) in items.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                dump(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (k, (key, item)) in map.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                out.push_str(&Value::from(key.as_str()).to_string());
                out.push(':');
                dump(item, out);
            }
            out.push('}');
        }
        Value::Number(n) if n.is_f64() => out.push_str(&repr_float(n.as_f64().unwrap_or(0.0))),
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::dumps;
    use serde_json::json;

    #[test]
    fn written_as_python_writes_a_record() {
        let record = json!({"v": 1, "paragraph": "Größe \"x\"\n\u{1}", "facts": [{"value": 1e-5, "n": 2.0, "at": 3}]});
        assert_eq!(
            dumps(&record),
            r#"{"v":1,"paragraph":"Größe \"x\"\n\u0001","facts":[{"value":1e-05,"n":2.0,"at":3}]}"#
        );
    }
}
