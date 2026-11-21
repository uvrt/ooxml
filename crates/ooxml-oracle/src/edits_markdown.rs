//! `word-edits`' `insert_markdown` cases ([`ooxml_word_edit::InsertMarkdown`]): `markdown`,
//! `target` (the `at`), `html` (`text` keeps raw HTML as text), `emphasis` (`direct`),
//! `style_map` (pairs) and `images` (each source's bytes in hex); the result in the form `python/word_edits_diff.py` writes
//! docx-agent's `EditResult`.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_opc::Package;
use ooxml_store::Caller;
use ooxml_word_edit::{EditError, Editor, InsertMarkdown, MarkdownOptions, Pictures, WriteMap};
use serde_json::{json, Value};

/// Run the case on the document `package` opened; the case's JSON of the edit and its result.
///
/// # Errors
/// The edit's refusal.
pub fn insert_markdown(editor: &Editor, caller: &Caller, package: &Package, case: &Value) -> Result<Value, EditError> {
    let text = |key: &str| case.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let pairs: Vec<(String, Option<String>)> = case
        .get("style_map")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().map(str::to_owned)))
                .collect()
        })
        .unwrap_or_default();
    let style_map = WriteMap::from_pairs(&pairs).map_err(EditError::Refused)?;
    let command = InsertMarkdown {
        markdown: text("markdown"),
        at: text("target"),
        style_map,
        options: MarkdownOptions {
            html_as_text: text("html") == "text",
            emphasis_direct: text("emphasis") == "direct",
            pictures: images(case),
        },
        policy: crate::edits::policy(case),
    };
    editor.run(caller, &command).map(|done| {
        let out = &done.out;
        let result = json!({"id": out.first, "changed": out.changed, "created": out.created,
                            "renamed": crate::edits::renamed(&out.renamed), "removed": out.removed,
                            "warnings": out.warnings, "blocks": out.blocks});
        crate::edits::report(package, &done, result)
    })
}

/// A case's `images`: each source's bytes, given in hex.
fn images(case: &Value) -> Pictures {
    let hex = |text: &str| -> Vec<u8> {
        text.as_bytes()
            .chunks(2)
            .filter_map(|pair| {
                std::str::from_utf8(pair)
                    .ok()
                    .and_then(|p| u8::from_str_radix(p, 16).ok())
            })
            .collect()
    };
    case.get("images")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(src, bytes)| (src.clone(), std::sync::Arc::from(hex(bytes.as_str().unwrap_or("")))))
        .collect()
}
