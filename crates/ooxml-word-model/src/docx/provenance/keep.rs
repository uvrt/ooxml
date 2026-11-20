//! Owns: what a paragraph's text entry holds once new facts or labels are kept on it
//! (docx-agent's `ProvenanceOps._keep_text`): the new ones that are where they say in the
//! paragraph's text now, beside the ones it kept (followed to the text now) that they do
//! not overlap, in the order of their places.
//!
//! Must not: read a document or change one (the editing crate finds the paragraph and
//! writes the entry).

use super::facts::{facts_after_edit, facts_found, labels_after_edit};
use super::TextEntry;
use serde_json::Value;

/// A paragraph's entry after a keep.
#[derive(Debug, Clone, PartialEq)]
pub enum Kept {
    /// Nothing kept: labels were given and none is where it says (docx-agent's `False`).
    Refused,
    /// No fact or label is left: the entry goes, if there is one.
    Cleared,
    /// The entry's record now.
    Entry(TextEntry),
}

/// The entry of a paragraph whose text is `now` after `facts` and `labels` (each `p` set
/// to 0, `at` its place in `now`) are kept on it, `old` the entry it had (`None`: none, or
/// one that holds no text record).
#[must_use]
pub fn kept(old: Option<&TextEntry>, now: &str, facts: &[Value], labels: &[Value]) -> Kept {
    let new = facts_found(&first_paragraph(facts), now);
    let new_labels = facts_found(&first_paragraph(labels), now);
    if !labels.is_empty() && new_labels.is_empty() {
        return Kept::Refused;
    }
    let (old_facts, old_labels) = old.map_or_else(Default::default, |entry| {
        (
            facts_after_edit(&entry.facts, &entry.paragraph, now),
            labels_after_edit(&entry.labels, &entry.paragraph, now),
        )
    });
    let spans: Vec<(u64, u64)> = new.iter().chain(&new_labels).map(span).collect();
    let clear = |f: &Value| {
        let (first, last) = span(f);
        !spans.iter().any(|&(start, end)| first < end && start < last)
    };
    let facts = merged(old_facts.into_iter().filter(|f| clear(f)), new);
    let merged_labels = merged(old_labels.into_iter().filter(|f| clear(f)), new_labels);
    if facts.is_empty() && merged_labels.is_empty() {
        return Kept::Cleared;
    }
    Kept::Entry(TextEntry {
        paragraph: now.to_owned(),
        facts,
        labels: merged_labels,
    })
}

/// `{**f, "p": 0}` of each.
fn first_paragraph(given: &[Value]) -> Vec<Value> {
    given
        .iter()
        .map(|f| match f {
            Value::Object(object) => {
                let mut out = object.clone();
                out.insert("p".into(), Value::from(0));
                Value::Object(out)
            }
            other => other.clone(),
        })
        .collect()
}

/// A fact's characters, `at` to `at` and its text's length (one found where it says has
/// both).
fn span(found: &Value) -> (u64, u64) {
    let at = found.get("at").and_then(Value::as_u64).unwrap_or(0);
    let length = found
        .get("text")
        .and_then(Value::as_str)
        .map_or(0, |t| t.chars().count());
    (at, at + length as u64)
}

/// The kept ones then the new ones, in the order of their places (stable, as Python's
/// `sorted`).
fn merged(kept: impl Iterator<Item = Value>, new: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = kept.chain(new).collect();
    out.sort_by_key(|f| span(f).0);
    out
}

#[cfg(test)]
mod tests {
    use super::{kept, Kept};
    use crate::docx::provenance::TextEntry;
    use serde_json::{json, Value};

    fn fact(at: u64, text: &str, source: &str) -> Value {
        json!({"source": source, "text": text, "p": 0, "at": at})
    }

    #[test]
    fn new_facts_replace_the_ones_they_overlap() {
        let old = TextEntry {
            paragraph: "Got 12 and 3.5.".into(),
            facts: vec![fact(4, "12", "d:a"), fact(11, "3.5", "d:b")],
            labels: Vec::new(),
        };
        let given = [json!({"source": "d:c", "text": "12", "at": 4})];
        let Kept::Entry(entry) = kept(Some(&old), "Got 12 and 3.5.", &given, &[]) else {
            panic!("kept");
        };
        assert_eq!(
            entry.facts,
            vec![
                json!({"source": "d:c", "text": "12", "at": 4, "p": 0}),
                fact(11, "3.5", "d:b")
            ]
        );
    }

    #[test]
    fn a_fact_not_where_it_says_is_left_out() {
        let given = [json!({"source": "d:c", "text": "13", "at": 4})];
        assert_eq!(kept(None, "Got 12.", &given, &[]), Kept::Cleared);
        let labels = [json!({"source": "d:c", "text": "High", "at": 0})];
        assert_eq!(kept(None, "Low", &[], &labels), Kept::Refused);
    }
}
