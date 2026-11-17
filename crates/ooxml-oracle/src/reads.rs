//! `word-reads <manifest.json>`: the Rust Word reading tools' replies (`describe`,
//! `word_read`) on documents, in the form `python/word_reads_diff.py` writes docx-agent's:
//! the summary, data, next cursor and total, or the refusal.
//!
//! The manifest is a JSON list of `{"file": path, "case": name, "tool": name, "arguments": {...}}`.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_base::{Cancel, Limits};
use ooxml_opc::Package;
use ooxml_tools::{ArgType, Done, ToolError};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

/// Never cancelled.
struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Run over a manifest: one JSON line per case on stdout.
///
/// # Errors
/// A message when the manifest cannot be read.
pub fn run(args: &[String]) -> Result<(), String> {
    let manifest = args.first().ok_or("usage: word-reads <manifest.json>")?;
    let text = std::fs::read_to_string(manifest).map_err(|e| e.to_string())?;
    let items: Vec<Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut opened: Option<(String, Package)> = None;
    for item in items {
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .ok_or("an item without a file")?;
        let case = item.get("case").cloned().unwrap_or(Value::Null);
        let tool = item.get("tool").and_then(Value::as_str).unwrap_or("describe");
        let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if opened.as_ref().is_none_or(|(f, _)| f != file) {
            let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| format!("{file}: {e}"))?;
            opened = Some((file.to_owned(), package));
        }
        let Some((_, package)) = opened.as_ref() else { continue };
        let name = Path::new(file).file_name().and_then(|n| n.to_str()).unwrap_or(file);
        let reply = call(tool, &arguments, package, name);
        println!("{}", json!({"file": file, "case": case, "reply": reply}));
    }
    Ok(())
}

fn call(tool: &str, arguments: &Value, package: &Package, name: &str) -> Value {
    let result = match tool {
        "describe" => ooxml_tools_word::Describe::from_arg(arguments, "")
            .map_err(|e| ToolError::internal(e.to_string(), "arguments"))
            .and_then(|a| ooxml_tools_word::describe(&a, package, name, &Never)),
        "word_read" => ooxml_tools_word::WordRead::from_arg(arguments, "")
            .map_err(|e| ToolError::internal(e.to_string(), "arguments"))
            .and_then(|a| ooxml_tools_word::word_read(&a, package, name, &Never)),
        other => Err(ToolError::internal(format!("no tool {other}"), "oracle")),
    };
    match result {
        Ok(done) => reply(&done),
        Err(error) => json!({"error": error.to_json()}),
    }
}

fn reply(done: &Done<Value>) -> Value {
    json!({"summary": done.summary, "data": done.data, "next_cursor": done.next_cursor, "total": done.total})
}
