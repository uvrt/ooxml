//! `word-edits`' `set_text_styled` cases: one `word_set_text` item with inline emphasis
//! ([`ooxml_word_edit::SetStyledText`]), its result in the form `python/word_edits_diff.py`
//! writes docx-agent's tool's (`_set_one`): the edits, the styled runs (their text cut as
//! the tool's `short` cuts it), the notes and the paragraphs written.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_opc::Package;
use ooxml_store::Caller;
use ooxml_word_edit::{EditError, Editor, SetStyledText};
use serde_json::{json, Map, Value};

/// Run the case on the document `package` opened; the case's JSON of the edit and its result.
///
/// # Errors
/// The edit's refusal.
pub fn set_text_styled(editor: &Editor, caller: &Caller, package: &Package, case: &Value) -> Result<Value, EditError> {
    let text = |key: &str| case.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let command = SetStyledText {
        target: ooxml_word_model::address::Address::from(text("target")),
        text: text("text"),
        paragraph_style: case.get("style").and_then(Value::as_str).map(str::to_owned),
        policy: crate::edits::policy(case),
    };
    editor.run(caller, &command).map(|done| {
        let out = &done.out;
        let edits: Vec<Value> = out
            .edits
            .iter()
            .map(|e| {
                json!({"id": e.id, "changed": e.changed, "renamed": {},
                       "created": e.created, "removed": e.removed})
            })
            .collect();
        let styled: Vec<Value> = out
            .styled
            .iter()
            .map(|r| {
                let mut entry = Map::new();
                entry.insert("range".into(), json!(r.range));
                entry.insert("text".into(), json!(short(&r.text, 40)));
                entry.insert("style".into(), json!(r.style));
                if r.bold {
                    entry.insert("bold".into(), json!(true));
                }
                if r.italic {
                    entry.insert("italic".into(), json!(true));
                }
                Value::Object(entry)
            })
            .collect();
        let result = json!({"edits": edits, "styled": styled, "notes": out.notes, "written": out.written});
        crate::edits::report(package, &done, result)
    })
}

/// docx-agent's `short`: white space collapsed, cut to `limit` characters with an ellipsis.
fn short(text: &str, limit: usize) -> String {
    let space = |c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c);
    let joined = text
        .split(space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let mut out: String = joined.chars().take(limit - 1).collect();
    out.push('…');
    out
}
