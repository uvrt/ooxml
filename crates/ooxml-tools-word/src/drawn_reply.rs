//! Owns: a reply naming ids its edits drew as stand-ins, named as they were drawn where
//! the edits landed ([`resolve_drawn`]): an edit prepared on one version and replayed on a
//! newer head draws its new ids there (`ooxml_word_edit::Drawn`), so the ids its handler
//! read while preparing may stand for others.
//!
//! A stand-in that moved is a value the version the edit was prepared on had never given,
//! so in a reply made from that version it names nothing but the edit's own new block:
//! every whole occurrence (eight hex digits, no hex digit beside them) is replaced, in every
//! string of the reply the model or the application reads.
//!
//! Must not: change anything when no stand-in moved (the common case: nothing is walked).

use ooxml_tools::{Outcome, ToolResult};
use serde_json::Value;

/// `result` with each moved stand-in of `moved` (`(stand-in, value drawn)`, upper-case hex)
/// named by its value.
pub fn resolve_drawn(result: &mut ToolResult<Value>, moved: &[(String, String)]) {
    if moved.is_empty() {
        return;
    }
    let swap = |s: &mut String| swap_ids(s, moved);
    result.warnings.iter_mut().for_each(swap);
    for value in &mut result.app.touched {
        swap_value(value, moved);
    }
    for (_, touched) in &mut result.app.touched_by_op {
        touched.iter_mut().for_each(|v| swap_value(v, moved));
    }
    if let Outcome::Done(done) = &mut result.outcome {
        swap(&mut done.summary);
        done.changed.iter_mut().for_each(swap);
        done.created.iter_mut().for_each(swap);
        done.removed.iter_mut().for_each(swap);
        for (old, new) in &mut done.renamed {
            swap(old);
            swap(new);
        }
        for (_, address) in &mut done.refs {
            swap(address);
        }
        if let Some(data) = &mut done.data {
            swap_value(data, moved);
        }
    }
}

/// Every string (and object key) of `value` with the moved stand-ins replaced.
fn swap_value(value: &mut Value, moved: &[(String, String)]) {
    match value {
        Value::String(s) => swap_ids(s, moved),
        Value::Array(items) => items.iter_mut().for_each(|v| swap_value(v, moved)),
        Value::Object(map) => {
            let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            for (mut key, mut v) in entries {
                swap_ids(&mut key, moved);
                swap_value(&mut v, moved);
                map.insert(key, v);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// `text` with each whole occurrence of a moved stand-in (any case) replaced by its value:
/// a run of eight hex digits with no hex digit beside it.
fn swap_ids(text: &mut String, moved: &[(String, String)]) {
    if text.len() < 8 {
        return;
    }
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut run = String::new();
    let mut flush = |run: &mut String, out: &mut String| {
        let value = (run.len() == 8)
            .then(|| moved.iter().find(|(stand_in, _)| stand_in.eq_ignore_ascii_case(run)))
            .flatten();
        match value {
            Some((_, value)) => {
                out.push_str(value);
                changed = true;
            }
            None => out.push_str(run),
        }
        run.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_hexdigit() {
            run.push(ch);
        } else {
            flush(&mut run, &mut out);
            out.push(ch);
        }
    }
    flush(&mut run, &mut out);
    if changed {
        *text = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_stand_ins_are_replaced_and_nothing_else() {
        let moved = vec![("0A0B0C0D".to_owned(), "11223344".to_owned())];
        let mut s = "p:0a0b0c0d, t:0A0B0C0D, x0A0B0C0D, 0A0B0C0D9, é0A0B0C0D".to_owned();
        swap_ids(&mut s, &moved);
        assert_eq!(s, "p:11223344, t:11223344, x11223344, 0A0B0C0D9, é11223344");
        let mut v = serde_json::json!({"blocks": ["p:0A0B0C0D"], "p:0A0B0C0D": 1});
        swap_value(&mut v, &moved);
        assert_eq!(v, serde_json::json!({"blocks": ["p:11223344"], "p:11223344": 1}));
    }
}
