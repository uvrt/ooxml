//! Owns: a `SEQ` field counted as docx2svg's `sequence_switches` and `sequence_step` count
//! one: its identifier and switches, and its text with the sequence's count moved on.
//!
//! Must not: find the fields to count (the caller walks its story in document order).

use super::{number_text, Uncomputable};
use std::collections::{HashMap, HashSet};

fn uncomputable(why: impl Into<String>) -> Uncomputable {
    Uncomputable(why.into())
}

/// The tokens of a `SEQ` instruction: quoted strings, switches, words.
pub fn tokens(instruction: &str) -> Vec<String> {
    let chars: Vec<char> = instruction.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(&c) = chars.get(k) {
        if c.is_whitespace() {
            k += 1;
        } else if c == '"' {
            let end = chars.iter().skip(k + 1).position(|d| *d == '"').map(|p| k + 1 + p);
            match end {
                Some(end) => {
                    out.push(chars.get(k..=end).unwrap_or_default().iter().collect());
                    k = end + 1;
                }
                None => k += 1,
            }
        } else if c == '\\' {
            match chars.get(k + 1) {
                Some(d) if !d.is_whitespace() => {
                    out.push(format!("\\{d}"));
                    k += 2;
                }
                _ => k += 1,
            }
        } else {
            let end = chars
                .iter()
                .skip(k)
                .position(|d| d.is_whitespace() || *d == '"' || *d == '\\')
                .map_or(chars.len(), |p| k + p);
            out.push(chars.get(k..end).unwrap_or_default().iter().collect());
            k = end;
        }
    }
    out
}

/// A switch and its argument.
pub type Switch = (char, Option<String>);

/// A `SEQ` instruction's identifier and its switches in order (`sequence_switches`).
pub fn sequence_switches(instruction: &str) -> (Option<String>, Vec<Switch>) {
    let tokens = tokens(instruction);
    let mut name = None;
    let mut found = Vec::new();
    let mut k = usize::from(tokens.first().is_some_and(|t| t.eq_ignore_ascii_case("SEQ")));
    while let Some(token) = tokens.get(k) {
        k += 1;
        if let Some(rest) = token.strip_prefix('\\') {
            let c = rest.chars().next().unwrap_or(' ');
            let key = if "#*@!".contains(c) { c } else { c.to_ascii_lowercase() };
            let mut value = None;
            if "#*@rs".contains(key) {
                if let Some(next) = tokens.get(k) {
                    value = Some(next.trim_matches('"').to_owned());
                    k += 1;
                }
            }
            found.push((key, value));
        } else if name.is_none() {
            name = Some(token.trim_matches('"').to_owned());
        }
    }
    (name, found)
}

/// One `SEQ` counted (`sequence_step`).
pub fn sequence_step(
    instruction: &str,
    name: &str,
    found: &[Switch],
    counts: &mut HashMap<String, i64>,
    restarted: &HashSet<String>,
) -> Result<String, Uncomputable> {
    if name.is_empty() {
        return Err(uncomputable("a SEQ field names no sequence"));
    }
    if restarted.contains(name) {
        return Err(uncomputable(format!(
            "the sequence {name:?} restarts at heading levels (\\s)"
        )));
    }
    let flag = |key: char| found.iter().find(|(k, _)| *k == key);
    if let Some((_, restart)) = flag('r') {
        let value = restart
            .as_deref()
            .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
            .and_then(|r| r.parse().ok())
            .ok_or_else(|| uncomputable("\\r takes a number"))?;
        counts.insert(name.to_owned(), value);
    } else if flag('c').is_none() {
        *counts.entry(name.to_owned()).or_insert(0) += 1;
    }
    let value = counts.get(name).copied().unwrap_or(0);
    if flag('h').is_some() {
        return Ok(String::new());
    }
    let keys: Vec<char> = found.iter().map(|(k, _)| *k).collect();
    if let Some(star) = keys.iter().position(|k| *k == '*') {
        if keys.iter().skip(star).any(|k| *k == '#') {
            return Err(uncomputable("a \\# after a \\* is Word's error text"));
        }
    }
    number_text(instruction, value, None)
}
