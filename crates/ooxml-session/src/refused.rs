//! Owns: the refusal of an id a version of a Word document removed -- docx-agent's words
//! (round 37: which change removed it, the version it made as the host numbers it, who made
//! it, and the way on; round 38: a change made before the document was saved and opened
//! again says so) -- built from the document's retired-id record ([`crate::retired`]) for a
//! call the tool refused as naming an id it lacks (`not_found`). The ids are read from the
//! call's arguments as docx-agent's view reads an address it lacks: a drawing's `d:<n>` at an
//! argument's start, a block id (`p:`, `t:`, `tr:`, `s:` and eight hex digits, a story's
//! name before it) anywhere in it.
//!
//! Must not: change what the call did (nothing), or word any other refusal.

use crate::retired::{Retired, RetiredBy};
use ooxml_tools::{ErrorCode, Refusal, ToolError, TryCall, WayOut};
use ooxml_word_model::ids::HeldId;
use serde_json::{Map, Value};

/// The tool a refusal of a removed id says to read the document again with.
const READ_AGAIN: &str = "word_read";

/// `error` as the refusal of a removed id, where it is a `not_found` and `arguments` name
/// an id `retired` holds; `None` otherwise (the tool's refusal stands).
pub(crate) fn removed(retired: &Retired, arguments: &Map<String, Value>, error: &ToolError) -> Option<ToolError> {
    if error.code() != ErrorCode::Refusal(Refusal::NotFound) {
        return None;
    }
    let mut strings = Vec::new();
    arguments.values().for_each(|value| strings_of(value, &mut strings));
    let message = strings.into_iter().find_map(|identifier| named(retired, identifier))?;
    let mut read = Map::new();
    if let Some(doc) = arguments.get("doc").and_then(Value::as_str) {
        read.insert("doc".into(), Value::from(doc));
    }
    let call = TryCall {
        tool: READ_AGAIN.to_owned(),
        arguments: read,
        fill: Vec::new(),
    };
    let refusal = ToolError::refusal(Refusal::NotFound, message, WayOut::call(call));
    Some(match error.field() {
        Some(field) => refusal.at(field),
        None => refusal,
    })
}

/// What naming a removed id in `identifier` says: its drawing, else its first block id the
/// record holds (one the host numbered).
fn named(retired: &Retired, identifier: &str) -> Option<String> {
    if let Some((named, number)) = drawing(identifier) {
        if let Some(said) = said(retired, HeldId::Drawing(number)) {
            return Some(message(&named, None, &said));
        }
    }
    blocks(identifier).into_iter().find_map(|(named, value)| {
        let said = said(retired, HeldId::Block(value))?;
        Some(message(&named, Some(identifier), &said))
    })
}

/// Who removed `id` and in which version, as the host numbers it.
struct Said {
    version: u64,
    who: Option<String>,
    earlier: bool,
}

fn said(retired: &Retired, id: HeldId) -> Option<Said> {
    match retired.by(id)? {
        RetiredBy::Step { seq, caller, tool } => Some(Said {
            version: retired.number_of(seq)?,
            who: tool.map(|tool| who(&tool, caller.as_deref())),
            earlier: false,
        }),
        RetiredBy::Earlier { version, said } => Some(Said {
            version,
            who: said,
            earlier: true,
        }),
    }
}

/// A step's tool and its caller's context, as docx-agent's tool layer says them:
/// `word_delete (agent=a, turn=3)`.
fn who(tool: &str, caller: Option<&str>) -> String {
    let context: Vec<String> = caller
        .and_then(|c| serde_json::from_str::<Map<String, Value>>(c).ok())
        .map(|map| {
            map.iter()
                .map(|(key, value)| format!("{key}={}", py_str(value)))
                .collect()
        })
        .unwrap_or_default();
    if context.is_empty() {
        tool.to_owned()
    } else {
        format!("{tool} ({})", context.join(", "))
    }
}

/// A JSON value as Python's `str` writes what `json.loads` read of it.
fn py_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Null => "None".to_owned(),
        other => other.to_string(),
    }
}

/// docx-agent's words (`edit.document._deleted_message`).
fn message(named: &str, identifier: Option<&str>, said: &Said) -> String {
    let version = said.version;
    let mut by = match &said.who {
        Some(who) => format!(" by {who}, the change that made version {version}"),
        None => format!(" in the change that made version {version}"),
    };
    if said.earlier {
        by.push_str(", before the document was saved and opened again");
    }
    let head = identifier
        .filter(|i| *i != named)
        .map(|i| format!("{i}: "))
        .unwrap_or_default();
    format!(
        "{head}{named} was deleted{by}. An id is never given to another block: read that part of the \
         document again ({READ_AGAIN}) for the ids there now"
    )
}

/// The strings of a call's arguments, in order (an address may be at any depth).
fn strings_of<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => out.push(text),
        Value::Array(items) => items.iter().for_each(|item| strings_of(item, out)),
        Value::Object(map) => map.values().for_each(|item| strings_of(item, out)),
        _ => {}
    }
}

/// A drawing's id at the start of `text` (`d:<number>`, not followed by a digit or `#`).
fn drawing(text: &str) -> Option<(String, u64)> {
    let rest = text.strip_prefix("d:")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || rest.as_bytes().get(digits) == Some(&b'#') {
        return None;
    }
    let number = rest.get(..digits)?.parse().ok()?;
    Some((format!("d:{}", rest.get(..digits)?), number))
}

/// Each block id in `text` (docx-agent's `_GONE_BLOCK`: at the start or after `/`, `:` or
/// `.`, a story's name and `/` before it or not, then `p:`, `t:`, `tr:` or `s:` and eight
/// hex digits not followed by another): as named, and its value.
fn blocks(text: &str) -> Vec<(String, u32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    while start < b.len() {
        let after_mark = start == 0 || b.get(start - 1).is_some_and(|c| b"/:.".contains(c));
        let found = if after_mark { block_at(b, start) } else { None };
        match found {
            Some((end, value)) => {
                if let Some(named) = text.get(start..end) {
                    out.push((named.to_owned(), value));
                }
                start = end;
            }
            None => start += 1,
        }
    }
    out
}

/// A block id starting at `start` (a story's name first, or not): its end and value.
fn block_at(b: &[u8], start: usize) -> Option<(usize, u32)> {
    let word = b
        .get(start..)?
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
        .count();
    if word > 0 && b.get(start + word) == Some(&b'/') {
        if let Some(found) = kind_at(b, start + word + 1) {
            return Some(found);
        }
    }
    kind_at(b, start)
}

/// `p:`, `t:`, `tr:` or `s:` and eight hex digits at `at`, not followed by another.
fn kind_at(b: &[u8], at: usize) -> Option<(usize, u32)> {
    ["p:", "t:", "tr:", "s:"].iter().find_map(|kind| {
        let hex = at + kind.len();
        let end = hex + 8;
        let digits = b.get(hex..end)?;
        let ok = b.get(at..hex)? == kind.as_bytes()
            && digits.iter().all(u8::is_ascii_hexdigit)
            && !b.get(end).is_some_and(u8::is_ascii_hexdigit);
        let value = u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
        ok.then_some((end, value))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_ids_are_read_as_the_view_reads_them() {
        let found = |t: &str| blocks(t).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(found("p:0BADF00D@3:9"), ["p:0BADF00D"]);
        assert_eq!(found("header1/p:0BADF00D"), ["header1/p:0BADF00D"]);
        assert_eq!(found("h:p:0BADF00D..t:1234ABCD/r1"), ["p:0BADF00D", "t:1234ABCD"]);
        assert_eq!(found("tr:0000000A"), ["tr:0000000A"]);
        assert!(found("hp:0BADF00D").is_empty());
        assert!(found("p:0BADF00D1").is_empty());
        assert_eq!(drawing("d:12"), Some(("d:12".to_owned(), 12)));
        assert_eq!(drawing("d:12#2"), None);
        assert_eq!(drawing("x d:12"), None);
    }

    #[test]
    fn a_step_is_said_with_its_tool_and_context() {
        assert_eq!(
            who("word_delete", Some(r#"{"agent":"a","turn":3}"#)),
            "word_delete (agent=a, turn=3)"
        );
        assert_eq!(who("word_delete", None), "word_delete");
        let said = Said {
            version: 4,
            who: None,
            earlier: true,
        };
        assert_eq!(
            message("p:0BADF00D", Some("p:0BADF00D@2"), &said),
            "p:0BADF00D@2: p:0BADF00D was deleted in the change that made version 4, before the document was \
             saved and opened again. An id is never given to another block: read that part of the document \
             again (word_read) for the ids there now"
        );
    }
}
