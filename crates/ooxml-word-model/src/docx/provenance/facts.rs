//! Owns: how a paragraph's text facts and labels follow an edit of its text
//! (`ooxml_edit.provenance`: `place`, `offset`, `facts_found`, `edit_span`,
//! `facts_after_edit`, `labels_after_edit`), places counted in characters as Python counts
//! them. A fact is true while its characters are its text; a label outlives an edit of its
//! own text (stretched over it).
//!
//! Must not: read a document (the caller gives the texts).

use serde_json::{Map, Value};

/// A text as Python indexes it: by character.
type Chars = [char];

/// `(p, at)` of the character `offset` of `text`: its paragraph (paragraphs joined by
/// `\n`) and its place in it.
fn place(text: &Chars, offset: usize) -> (usize, usize) {
    let before = text.get(..offset).unwrap_or(text);
    let p = before.iter().filter(|c| **c == '\n').count();
    let line = before.iter().rposition(|c| *c == '\n').map_or(0, |k| k + 1);
    (p, offset - line)
}

/// A whole number of at least 0 as JSON gives it (`true` is none).
fn whole(value: Option<&Value>) -> Option<usize> {
    value.and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())
}

/// The character of `text` that paragraph `p`'s character `at` is; `None` when there is no
/// such paragraph, or `p` and `at` are not whole numbers.
fn offset(text: &Chars, p: Option<&Value>, at: Option<&Value>) -> Option<usize> {
    let (p, at) = (whole(p)?, whole(at)?);
    let mut start = 0;
    for (k, line) in text.split(|c| *c == '\n').enumerate() {
        if k == p {
            return (at <= line.len()).then_some(start + at);
        }
        start += line.len() + 1;
    }
    None
}

/// A fact's (or label's) object, its text's characters and its first character in `text`,
/// when its text is in `text` where its `p` and `at` say.
fn found_at<'f>(found: &'f Value, text: &Chars) -> Option<(&'f Map<String, Value>, Vec<char>, usize)> {
    let object = found.as_object()?;
    let own: Vec<char> = object.get("text")?.as_str()?.chars().collect();
    let start = offset(text, object.get("p"), object.get("at"))?;
    let end = start + own.len();
    (!own.is_empty() && text.get(start..end) == Some(&own[..])).then_some((object, own, start))
}

/// The facts whose `text` is in `text` where their `p` and `at` say.
#[must_use]
pub fn facts_found(facts: &[Value], text: &str) -> Vec<Value> {
    let text: Vec<char> = text.chars().collect();
    facts.iter().filter(|f| found_at(f, &text).is_some()).cloned().collect()
}

/// What changed from `old` to `new`, as one span: `(start, old_end, new_end)`.
fn edit_span(old: &Chars, new: &Chars) -> (usize, usize, usize) {
    let limit = old.len().min(new.len());
    let start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(limit - start)
        .take_while(|(a, b)| a == b)
        .count();
    (start, old.len() - tail, new.len() - tail)
}

/// `object` with `p` and `at` set (in their places) to the place of `offset` in `text`.
fn moved(object: &Map<String, Value>, text: &Chars, offset: usize) -> Value {
    let (p, at) = place(text, offset);
    let mut out = object.clone();
    out.insert("p".into(), Value::from(p));
    out.insert("at".into(), Value::from(at));
    Value::Object(out)
}

/// The facts of `old` still true in `new` (the same text after an edit), at their new
/// places: one the edit did not touch stays, moved with the text before it; one it touched
/// or borders, or that is not where it says, goes.
#[must_use]
pub fn facts_after_edit(facts: &[Value], old: &str, new: &str) -> Vec<Value> {
    if old == new {
        return facts_found(facts, old);
    }
    let (old, new): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let (start, old_end, new_end) = edit_span(&old, &new);
    let mut out = Vec::new();
    for fact in facts {
        let Some((object, own, first)) = found_at(fact, &old) else {
            continue;
        };
        let last = first + own.len();
        if last < start {
            out.push(fact.clone());
        } else if first > old_end {
            out.push(moved(object, &new, first + new_end - old_end));
        }
    }
    out
}

/// The labels of `old` after an edit made it `new`: one the edit left alone stays (moved
/// with the text before it); one that is all of `old` is all of `new`; one the edit falls
/// within (an insertion at its edge too) stretches over the edited text; one the edit
/// reaches past an edge of goes.
#[must_use]
pub fn labels_after_edit(labels: &[Value], old: &str, new: &str) -> Vec<Value> {
    if old == new {
        return facts_found(labels, old);
    }
    let (old, new): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
    let (start, old_end, new_end) = edit_span(&old, &new);
    let mut out = Vec::new();
    for label in labels {
        let Some((object, own, first)) = found_at(label, &old) else {
            continue;
        };
        let last = first + own.len();
        let with_text = |text: &Chars| {
            let mut out = object.clone();
            out.insert("text".into(), Value::from(text.iter().collect::<String>()));
            Value::Object(out)
        };
        if first == 0 && last == old.len() {
            if !new.is_empty() {
                out.push(with_text(&new));
            }
        } else if old_end < first || (old_end == first && start < first) {
            out.push(moved(object, &new, first + new_end - old_end));
        } else if start > last || (start == last && old_end > last) {
            out.push(label.clone());
        } else if first <= start && old_end <= last {
            let end = (last + new_end).saturating_sub(old_end).min(new.len());
            let text = new.get(first..end).unwrap_or_default();
            if !text.is_empty() && !text.contains(&'\n') {
                out.push(with_text(text));
            }
        }
    }
    out
}
