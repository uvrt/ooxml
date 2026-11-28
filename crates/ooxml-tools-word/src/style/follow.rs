//! Owns: `word_follow_sample` (docx-agent's `edit/follow.py` and its tool in
//! `tools/style.py`): the edit core's [`FollowSample`] run as one command (on a private
//! store from the head version for a `preview`, which changes nothing), the sample's
//! document handed over by the session at the version pinned for the call; then what is
//! still different per copy as `word_compare_blocks` finds it on the result
//! ([`super::follow_report`]), and the reply.
//!
//! Must not: change the document but through the command, or judge whether a difference is
//! intended (each is reported with why it was not followed).

use super::compare::{compare, Side};
use super::compare_args::{FollowReport, WordFollowSample};
use super::follow_report::{copy_entry, own_counted, CAP};
use crate::refusal::address_refused;
use crate::review::{Editing, Still};
use crate::structure::common::{call, outcome, policy, refused, Outcome};
use ooxml_store::{Caller, Store, StoreConfig};
use ooxml_tools::{Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{FollowSample, Followed, WordDoc};
use ooxml_word_model::docx::Docx;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// `word_follow_sample` (docx-agent's): each copy made to follow the sample, one step; per
/// copy what changed and what was not followed.
#[must_use]
pub fn word_follow_sample(args: &WordFollowSample, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let again = call("word_follow_sample", &[("doc", json!(args.doc))]);
    run(args, editing, &again).unwrap_or_else(ToolResult::failed)
}

/// A sample document the call was not handed (the session refuses one not open first).
fn not_open(id: &str, doc: &str) -> ToolError {
    let message = format!("no open document {}", ooxml_tools::pyfmt::repr_str(id));
    match WayOut::names([doc.to_owned()]) {
        Some(way_out) => ToolError::refusal(Refusal::NotFound, message, way_out).at("sample.doc"),
        None => ToolError::internal(message, "word_follow_sample:sample"),
    }
}

fn model(error: impl std::fmt::Display) -> ToolError {
    ToolError::internal(error.to_string(), "word:model")
}

fn run(args: &WordFollowSample, editing: &Editing<'_>, again: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let other = args.sample.doc.as_deref().filter(|d| !d.is_empty() && *d != args.doc);
    let source: Option<Arc<Docx>> = match other {
        Some(id) => Some(editing.others.get(id).ok_or_else(|| not_open(id, &args.doc))?.docx()?),
        None => None,
    };
    let preview = args.preview.unwrap_or(false);
    let command = FollowSample {
        sample: args.sample.block.clone(),
        source: source.clone(),
        copies: args.copies.clone(),
        policy: policy(editing),
    };
    let ran = if preview {
        let head = editing.editor.current();
        let store = Store::new(head.doc().clone(), StoreConfig::default(), Arc::new(Still));
        store
            .run(&Caller::anonymous(), &command)
            .map(|done| (done.out, done.version.doc().clone()))
    } else {
        editing
            .editor
            .run(editing.caller, &command)
            .map(|done| (done.out, done.version.doc().clone()))
    };
    let (followed, after): (Followed, WordDoc) = match ran {
        Ok(found) => found,
        Err(error) => return Ok(refused(error, again.clone())),
    };
    let docx = after.docx().map_err(model)?;
    let theirs = source.unwrap_or_else(|| Arc::clone(&docx));
    let (mine, sample) = (Side::new(&docx), Side::new(&theirs));
    let mut copies = Vec::new();
    for copy in &followed.copies {
        let found = compare(&mine, std::slice::from_ref(&copy.id), &followed.sample, &sample)
            .map_err(|e| address_refused(e, again.clone()))?;
        let left: Vec<Value> = match found.get("differences") {
            Some(Value::Array(all)) => all.iter().filter_map(|d| textless(&docx, d)).collect(),
            _ => Vec::new(),
        };
        copies.push(copy_entry(copy, &left));
    }
    Ok(reply(args, &followed, copies, preview))
}

/// The run properties a comparison reports.
const RUN: [&str; 8] = [
    "bold",
    "italic",
    "underline",
    "strike",
    "size",
    "font",
    "color",
    "highlight",
];

/// A run property's difference without the paragraphs that have no text (their mark took
/// the sample's run formatting, which a comparison, reading text, does not see); `None`
/// where none is left.
fn textless(docx: &Docx, difference: &Value) -> Option<Value> {
    let property = difference.get("property").and_then(Value::as_str).unwrap_or("");
    let paragraphs = difference.get("paragraphs").and_then(Value::as_array);
    let Some(paragraphs) = paragraphs.filter(|p| RUN.contains(&property) && !p.is_empty()) else {
        return Some(difference.clone());
    };
    let kept: Vec<Value> = paragraphs
        .iter()
        .filter(|p| p.as_str().is_some_and(|id| has_text(docx, id)))
        .cloned()
        .collect();
    if kept.is_empty() {
        return None;
    }
    let mut out = difference.as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(fix)) = out.get_mut("fix") {
        if let Some(Value::Array(calls)) = fix.get_mut("calls") {
            for one in calls.iter_mut() {
                if let Value::Object(one) = one {
                    one.insert("targets".into(), Value::Array(kept.clone()));
                }
            }
        }
    }
    out.insert("paragraphs".into(), Value::Array(kept));
    Some(Value::Object(out))
}

/// Whether the paragraph `id` has text other than white space.
fn has_text(docx: &Docx, id: &str) -> bool {
    use ooxml_word_model::docx::text::paragraph_text;
    use ooxml_word_model::ids::Entry;
    use ooxml_word_model::view::View;
    match ooxml_word_model::address::resolve(docx, id) {
        Ok((story, Entry::Paragraph(k))) => story.ids.paragraphs().get(k).is_some_and(|p| {
            !paragraph_text(&p.element, View::Final)
                .trim_matches(crate::describe::py_space)
                .is_empty()
        }),
        _ => false,
    }
}

/// The reply: the copies' entries within the reply's budget, the summary and the edit's
/// outcome (none for a preview).
fn reply(
    args: &WordFollowSample,
    followed: &Followed,
    mut copies: Vec<Map<String, Value>>,
    preview: bool,
) -> ToolResult<Value> {
    let listed = |entry: &Map<String, Value>| entry.get("not_followed").and_then(Value::as_array).map_or(0, Vec::len);
    let count = |entry: &Map<String, Value>, key: &str| entry.get(key).and_then(Value::as_u64).unwrap_or(0);
    let actionable = args.report == Some(FollowReport::Actionable);
    if actionable || copies.iter().map(listed).sum::<usize>() > CAP {
        // over the reply's budget: the copies' own become counts before anything is cut
        copies.iter_mut().for_each(own_counted);
    }
    let mut room = CAP;
    for entry in &mut copies {
        if let Some(Value::Array(shown)) = entry.get_mut("not_followed") {
            if shown.len() > room {
                let cut = (shown.len() - room) as u64;
                shown.truncate(room);
                let more = entry.get("more").and_then(Value::as_u64).unwrap_or(0) + cut;
                entry.insert("more".into(), json!(more));
            }
        }
        room = room.saturating_sub(listed(entry));
    }
    let left_out: u64 = copies.iter().map(|e| count(e, "more")).sum();
    let changed: Vec<String> = followed
        .copies
        .iter()
        .filter(|c| c.changed())
        .map(|c| c.id.clone())
        .collect();
    let left: u64 = copies
        .iter()
        .map(|e| listed(e) as u64 + count(e, "more") + count(e, "own"))
        .sum();
    let total = copies.len();
    let mut data = Map::new();
    data.insert("sample".into(), json!(followed.sample));
    data.insert("preview".into(), json!(preview));
    data.insert(
        "copies".into(),
        Value::Array(copies.into_iter().map(Value::Object).collect()),
    );
    if left_out > 0 {
        data.insert(
            "left_out".into(),
            json!(format!(
                "{left_out} more difference(s) not followed, counted in each copy's 'more': \
                 word_compare_blocks on that copy lists them all, paged"
            )),
        );
    }
    let summary = format!(
        "{} of {total} cop{} {} {}{}",
        changed.len(),
        if total == 1 { "y" } else { "ies" },
        if preview { "would follow" } else { "followed" },
        followed.sample,
        if left > 0 {
            format!("; {left} difference(s) not followed (see not_followed)")
        } else {
            "; nothing left to follow".to_owned()
        }
    );
    if preview {
        return outcome(&[], summary, Some(Value::Object(data)));
    }
    let edit = Outcome {
        id: followed.copies.first().map(|c| c.id.clone()),
        changed: !changed.is_empty(),
        created: Vec::new(),
        removed: Vec::new(),
        renamed: Vec::new(),
        warnings: followed.warnings.clone(),
    };
    let mut result = outcome(&[edit], summary, Some(Value::Object(data)));
    if let ooxml_tools::Outcome::Done(done) = &mut result.outcome {
        for id in changed {
            if !done.changed.contains(&id) {
                done.changed.push(id);
            }
        }
    }
    result
}

/// Registers `word_follow_sample` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.edit::<WordFollowSample>(word_follow_sample);
}
