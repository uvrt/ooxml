//! Owns: `word_changes`' `list` (docx-agent's `_list_changes`, `change_json`,
//! `revision_json`): the changes grouped as a reviewer reads them, or Word's records, by
//! author, kind and place, a page at a time.
//!
//! Must not: change the document.

use super::changes::{try_call, ChangeDetail, Reading, WordChanges};
use crate::describe::short;
use ooxml_tools::paging::page_list;
use ooxml_tools::{Done, ToolError};
use ooxml_word_model::address::Address;
use ooxml_word_model::docx::changes::{changes, LogicalChange};
use ooxml_word_model::docx::revisions::{describe_record, records};
use ooxml_word_model::docx::Docx;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// A change as `word_changes` lists it (`change_json`).
pub(crate) fn change_json(change: &LogicalChange) -> Value {
    let mut data = Map::new();
    data.insert("id".into(), json!(change.id()));
    data.insert("kind".into(), json!(change.kind));
    data.insert("author".into(), json!(change.author));
    data.insert("date".into(), json!(change.date));
    if let Some(structure) = change.structure {
        data.insert("structure".into(), json!(structure));
    }
    if !change.old_text.is_empty() {
        data.insert("old_text".into(), json!(short(&change.old_text, 200)));
    }
    if !change.text.is_empty() {
        data.insert("text".into(), json!(short(&change.text, 200)));
    }
    let paragraphs: Vec<&String> = change.paragraph_ids.iter().take(5).collect();
    data.insert("paragraphs".into(), json!(paragraphs));
    if change.records.len() > 1 {
        data.insert("records".into(), json!(change.records));
    }
    Value::Object(data)
}

/// The paragraph ids a `within` covers; its refusal at no field, as today's tool names
/// none.
fn paragraphs_within(docx: &Docx, within: &str, args: &WordChanges) -> Result<BTreeSet<String>, ToolError> {
    let mut call = args.clone();
    call.within = None;
    ooxml_word_model::address::scope_paragraphs(docx, &Address::parse(within))
        .map(|(part, indexes)| {
            indexes
                .iter()
                .filter_map(|k| part.ids.paragraphs().get(*k).map(|p| p.id))
                .collect()
        })
        .map_err(|e| crate::refusal::address_refused(e, try_call(&call)))
}

/// `list`, of the version `reading` reads.
pub(crate) fn list(args: &WordChanges, reading: &Reading<'_>) -> Result<Done<Value>, ToolError> {
    let docx = reading
        .doc
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let records_wanted = args.detail == Some(ChangeDetail::Records);
    // the paragraphs first, for either detail, as docx-agent's _list_changes reads them: a
    // refusal is theirs
    let inside = args
        .within
        .as_deref()
        .map(|w| paragraphs_within(&docx, w, args))
        .transpose()?;
    let items: Vec<Value> = if records_wanted {
        let within = match &args.within {
            Some(w) => Some(ooxml_word_edit::review::records_within(&docx, w).map_err(|e| {
                let mut call = args.clone();
                call.within = None;
                super::edit_refusal(e, "", try_call(&call))
            })?),
            None => None,
        };
        records(&docx)
            .into_iter()
            .filter(|r| within.as_ref().is_none_or(|w| w.contains(&r.id)))
            .filter(|r| args.author.as_deref().is_none_or(|a| r.author() == Some(a)))
            .map(|r| (describe_record(&docx, &r), r))
            .filter(|(info, _)| {
                args.kind
                    .is_none_or(|k| info.kind.starts_with(k.name()) || info.kind.contains(k.name()))
            })
            .map(|(info, r)| {
                let mut data = Map::new();
                data.insert("id".into(), json!(r.id));
                data.insert("kind".into(), json!(info.kind));
                data.insert("author".into(), json!(r.author()));
                data.insert("date".into(), json!(info.date));
                data.insert("paragraph".into(), json!(info.paragraph));
                if !info.text.is_empty() {
                    data.insert("text".into(), json!(short(&info.text, 200)));
                }
                Value::Object(data)
            })
            .collect()
    } else {
        changes(&docx)
            .into_iter()
            .filter(|c| args.author.as_deref().is_none_or(|a| c.author.as_deref() == Some(a)))
            .filter(|c| args.kind.is_none_or(|k| c.kind == k.name()))
            .filter(|c| {
                inside
                    .as_ref()
                    .is_none_or(|i| c.paragraph_ids.iter().any(|p| i.contains(p)))
            })
            .map(|c| change_json(&c))
            .collect()
    };
    super::stopped(reading.cancel)?;
    let mut first = args.clone();
    first.cursor = None;
    let (shown, total, next) = page_list(
        &items,
        args.cursor.as_deref(),
        reading.limits.list_items,
        try_call(&first),
    )?;
    let noun = if records_wanted { "record" } else { "change" };
    let mut done = Done::new(format!("{total} {noun}(s)"));
    done.data = Some(Value::Array(shown));
    done.next_cursor = next;
    done.total = Some(total as u64);
    Ok(done)
}
