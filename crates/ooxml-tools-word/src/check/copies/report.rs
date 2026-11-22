//! Owns: `copies`' reply (docx-agent's `compare_copies`' report): each copy's entry within
//! the reply's budget (rule 4: what is left out is counted, and how to read it said), the
//! counts, `fix` -- one `word_follow_sample` call per reference block -- and `list_fix`, the
//! `word_lists` calls of every copy's list differences, alike ones joined (rule 5).
//!
//! Must not: compare (`super::compare` does).

use super::settings::LIST_SETTINGS;
use super::{Resolved, Said};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Differences listed per copy (the copy's own are counted, not listed).
const DIFFERENCES_PER_COPY: usize = 20;
/// Differences listed in one reply over every copy; past it, counted.
const DIFFERENCES_PER_REPLY: usize = 100;
/// Places with example text listed per copy, and per reply.
const EXAMPLES_PER_COPY: usize = 10;
const EXAMPLES_PER_REPLY: usize = 50;
/// `word_lists` calls in a reply's `list_fix` at most (the rest counted: `more`).
pub(super) const LIST_CALLS: usize = 40;

/// The copies of one reference block `word_follow_sample` makes follow it.
struct FollowCall {
    /// The reference's document where it is another.
    other: Option<String>,
    reference: String,
    copies: Vec<String>,
}

/// The reply as it is made, copy by copy.
pub(super) struct Report {
    items: Vec<Value>,
    kept: usize,
    room: usize,
    examples_room: usize,
    follow: Vec<FollowCall>,
    pub(super) list_calls: Vec<Value>,
}

impl Default for Report {
    fn default() -> Self {
        Report {
            items: Vec::new(),
            kept: 0,
            room: DIFFERENCES_PER_REPLY,
            examples_room: EXAMPLES_PER_REPLY,
            follow: Vec::new(),
            list_calls: Vec::new(),
        }
    }
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn listed<'f>(found: &'f Map<String, Value>, key: &str) -> &'f [Value] {
    found.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

impl Report {
    /// One copy's entry, from its comparison `found`, and its fixes kept for the reply's.
    pub(super) fn add(&mut self, r: &Resolved<'_>, said: &Said<'_>, found: &Map<String, Value>) {
        self.kept += usize::from(said.kept);
        let mut entry = Map::new();
        entry.insert("copy".into(), Value::from(r.item.blocks.trim()));
        entry.insert("reference".into(), Value::from(r.item.reference.block.trim()));
        if let Some(other) = said.other {
            entry.insert("doc".into(), Value::from(other));
        }
        if let Some(runs_from) = said.runs_from {
            entry.insert("runs_on_from".into(), Value::from(runs_from));
        }
        entry.insert(
            "follows".into(),
            found.get("follows").cloned().unwrap_or(Value::Bool(false)),
        );
        self.differences(&mut entry, listed(found, "differences"));
        self.examples(&mut entry, listed(found, "example_text"));
        entry.insert("cached".into(), Value::from(said.kept));
        self.fixes(said.other, found);
        self.items.push(Value::Object(entry));
    }

    /// The differences that are not the copy's own, as many as the budget lets; the others
    /// counted.
    fn differences(&mut self, entry: &mut Map<String, Value>, differences: &[Value]) {
        let listed: Vec<&Value> = differences.iter().filter(|d| text(d, "kind") != "own").collect();
        let own = differences.len() - listed.len();
        let shown: Vec<Value> = listed
            .iter()
            .take(DIFFERENCES_PER_COPY.min(self.room))
            .map(|d| (*d).clone())
            .collect();
        self.room -= shown.len();
        let more = listed.len() - shown.len();
        entry.insert("differences".into(), Value::Array(shown));
        if own > 0 {
            entry.insert("own".into(), Value::from(own));
        }
        if more > 0 {
            entry.insert("more".into(), Value::from(more));
        }
    }

    /// The places with example text, as many as the budget lets; the others counted.
    fn examples(&mut self, entry: &mut Map<String, Value>, texts: &[Value]) {
        if texts.is_empty() {
            return;
        }
        let shown: Vec<Value> = texts
            .iter()
            .take(EXAMPLES_PER_COPY.min(self.examples_room))
            .cloned()
            .collect();
        self.examples_room -= shown.len();
        let left = texts.len() - shown.len();
        entry.insert("example_text".into(), Value::Array(shown));
        if left > 0 {
            entry.insert("example_text_more".into(), Value::from(left));
        }
    }

    /// The copy's blocks `word_follow_sample` makes follow (formatting that is not its
    /// lists': `word_lists` makes those, `list_fix`), and its list differences' calls.
    fn fixes(&mut self, other: Option<&str>, found: &Map<String, Value>) {
        let differences = listed(found, "differences");
        for pair in listed(found, "pairs") {
            let (Some(copy), Some(reference)) =
                (pair.get(0).and_then(Value::as_str), pair.get(1).and_then(Value::as_str))
            else {
                continue;
            };
            let follows_sample = differences.iter().any(|d| {
                text(d, "kind") == "format"
                    && d.get("block").and_then(Value::as_str) == Some(copy)
                    && !LIST_SETTINGS.contains(&text(d, "property").as_str())
            });
            if !follows_sample {
                continue;
            }
            let other = other.map(str::to_owned);
            match self
                .follow
                .iter_mut()
                .find(|f| f.other == other && f.reference == reference)
            {
                Some(found) => found.copies.push(copy.to_owned()),
                None => self.follow.push(FollowCall {
                    other,
                    reference: reference.to_owned(),
                    copies: vec![copy.to_owned()],
                }),
            }
        }
        for difference in differences {
            let fix = difference.get("fix").filter(|f| !f.is_null());
            let lists = fix.is_some_and(|f| f.get("tool").and_then(Value::as_str) == Some("word_lists"));
            if LIST_SETTINGS.contains(&text(difference, "property").as_str()) && lists {
                if let Some(Value::Array(calls)) = fix.and_then(|f| f.get("calls")) {
                    self.list_calls.extend(calls.iter().cloned());
                }
            }
        }
    }

    /// The reply: the counts, each copy's entry, the fixes, and what is left out.
    pub(super) fn finish(self, doc: &str, alone: &[usize]) -> Value {
        let count = |key: &str| {
            self.items
                .iter()
                .filter(|e| match e.get(key) {
                    Some(Value::Bool(b)) => *b,
                    Some(Value::Array(a)) => !a.is_empty(),
                    _ => false,
                })
                .count()
        };
        let sum = |key: &str| -> u64 {
            self.items
                .iter()
                .filter_map(|e| e.get(key).and_then(Value::as_u64))
                .sum()
        };
        let (more, more_examples) = (sum("more"), sum("example_text_more"));
        let mut report = Map::new();
        report.insert("compared".into(), Value::from(self.items.len()));
        report.insert("follow".into(), Value::from(count("follows")));
        report.insert("with_example_text".into(), Value::from(count("example_text")));
        report.insert("kept".into(), Value::from(self.kept));
        report.insert("items".into(), Value::Array(self.items));
        if !self.follow.is_empty() {
            // Rule 5: one call per reference block makes every copy of it follow (preview first).
            let calls: Vec<Value> = self.follow.into_iter().map(|f| follow_call(f, doc)).collect();
            report.insert("fix".into(), json!({"tool": "word_follow_sample", "calls": calls}));
        }
        if !self.list_calls.is_empty() {
            report.insert("list_fix".into(), list_fix(&self.list_calls));
        }
        if !alone.is_empty() {
            let named: Vec<String> = alone.iter().map(ToString::to_string).collect();
            report.insert(
                "note".into(),
                Value::from(format!(
                    "lists run_on on copies[{}] alone of their reference: the first copy of a run numbers as its \
                     sample shows, each later one counts on from the copy before it; name the whole run in one check",
                    named.join(", ")
                )),
            );
        }
        if more > 0 || more_examples > 0 {
            report.insert(
                "left_out".into(),
                Value::from(format!(
                    "{more} difference(s) and {more_examples} place(s) with example text past the reply's budget, \
                     counted per copy ('more', 'example_text_more'): check fewer copies at once, or \
                     word_compare_blocks on one copy's block for all its differences, paged"
                )),
            );
        }
        Value::Object(report)
    }
}

/// One `word_follow_sample` call: the reference block (with its document where it is not
/// `doc`) and its copies, each once.
fn follow_call(f: FollowCall, doc: &str) -> Value {
    let mut sample = Map::new();
    sample.insert("block".into(), Value::from(f.reference));
    if let Some(other) = f.other.filter(|o| o != doc) {
        sample.insert("doc".into(), Value::from(other));
    }
    let mut copies: Vec<String> = Vec::new();
    for copy in f.copies {
        if !copies.contains(&copy) {
            copies.push(copy);
        }
    }
    json!({"sample": sample, "copies": copies})
}

/// Rule 5: every copy's list differences in one batch of `word_lists` calls, alike ones
/// joined, at most [`LIST_CALLS`] (`more` the rest).
fn list_fix(calls: &[Value]) -> Value {
    let merged = merged(calls);
    let mut out = Map::new();
    out.insert("tool".into(), Value::from("word_lists"));
    out.insert(
        "calls".into(),
        Value::from(merged.iter().take(LIST_CALLS).cloned().collect::<Vec<_>>()),
    );
    if merged.len() > LIST_CALLS {
        out.insert("more".into(), Value::from(merged.len() - LIST_CALLS));
    }
    Value::Object(out)
}

/// `word_lists` calls with the same arguments but their targets made one, in order -- those
/// of an action that changes each target (`level`, `add`, `remove`); the others (`format`,
/// `restart`, `continue`: the first target's list) kept apart.
pub(super) fn merged(calls: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut joined: Vec<(String, usize)> = Vec::new();
    for call in calls {
        let action = call.get("action").and_then(Value::as_str).unwrap_or("");
        if !matches!(action, "level" | "add" | "remove") {
            out.push(call.clone());
            continue;
        }
        let rest: BTreeMap<&String, &Value> = call
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, _)| k.as_str() != "targets")
            .collect();
        let key = serde_json::to_string(&rest).unwrap_or_default();
        let Some(at) = joined.iter().find(|(k, _)| *k == key).map(|(_, at)| *at) else {
            joined.push((key, out.len()));
            out.push(call.clone());
            continue;
        };
        let targets: Vec<Value> = call
            .get("targets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(Value::Array(found)) = out.get_mut(at).and_then(|c| c.get_mut("targets")) {
            for target in targets {
                if !found.contains(&target) {
                    found.push(target);
                }
            }
        }
    }
    out
}
