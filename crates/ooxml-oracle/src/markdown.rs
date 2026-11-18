//! `markdown-tokens <cases.json>`: texts read by `ooxml-markdown`, as JSON in the form
//! `python/markdown_diff.py` writes markdown-it-py's and docx-agent's readings: for a case
//! `{"mode": "styled", "text"}` the styled pieces (`[text, strong, em, style]`); for
//! `{"mode": "tokens", "text"}` markdown-it's token stream, every field.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_markdown::{styled, Token, Value as Attr};
use serde_json::{json, Map, Value};

/// Read every case of the file named first: one JSON line each on stdout.
///
/// # Errors
/// A message when the file cannot be read.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("usage: markdown-tokens <cases.json>")?;
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let cases: Vec<Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for case in cases {
        let text = case.get("text").and_then(Value::as_str).unwrap_or("");
        let out = match case.get("mode").and_then(Value::as_str) {
            Some("styled") => styled_json(text),
            Some("tokens") => Value::Array(ooxml_markdown::parse(text).0.iter().map(token_json).collect()),
            other => json!({"error": format!("no mode {other:?}")}),
        };
        println!("{out}");
    }
    Ok(())
}

fn styled_json(text: &str) -> Value {
    let read = styled::parse(text);
    let pieces: Vec<Value> = read
        .pieces
        .iter()
        .map(|p| json!([p.start, p.end, p.text, p.strong, p.em, p.style]))
        .collect();
    json!({"plain": read.plain, "pieces": pieces})
}

/// A token as `python/markdown_diff.py` writes markdown-it-py's.
fn token_json(token: &Token) -> Value {
    let attrs: Vec<Value> = token
        .attrs
        .iter()
        .map(|(k, v)| match v {
            Attr::Text(t) => json!([k, t]),
            Attr::Number(n) => json!([k, n]),
        })
        .collect();
    let mut meta = Map::new();
    if let Some(note) = &token.meta {
        if let Some(id) = note.id {
            meta.insert("id".into(), json!(id));
        }
        if let Some(sub) = note.sub_id {
            meta.insert("subId".into(), json!(sub));
        }
        if let Some(label) = &note.label {
            meta.insert("label".into(), json!(label));
        }
    }
    json!({
        "type": token.kind.name(), "tag": token.tag, "nesting": token.nesting, "level": token.level,
        "content": token.content, "markup": token.markup, "info": token.info, "attrs": attrs,
        "map": token.map.map(|(a, b)| json!([a, b])),
        "children": token.children.as_ref().map(|c| c.iter().map(token_json).collect::<Vec<_>>()),
        "meta": meta, "block": token.block, "hidden": token.hidden,
    })
}
