//! Owns: references found in text and written (ooxml-edit's `textfacts.REFERENCE`,
//! `references`, `expand`, `misread`, `misread_error`, `qualify_text`, `walk_texts` and the
//! dispatcher's `_with_facts`, ported exactly).
//!
//! `REFERENCE` is `(\\?)\{\{([^{}]{1,500})\}\}`, found left to right without overlap: a
//! backslash before it escapes it.
//!
//! Must not: resolve a value (the caller's resolver does).

use super::address::{is_handle, is_py_space, py_strip, qualified, segments, strip_format, Segment};
use super::{fact, options_or, Fact, FactEnv, FactSources, DATASET_PREFIX, MAX_FACTS};
use ooxml_tools::{Failure, Refusal, ToolError, TryCall};
use serde_json::{Map, Value};

/// The longest inside of a reference, in characters.
const INSIDE_MAX: usize = 500;

/// One `{{...}}` found in text: where it is (characters), whether escaped, its inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Its first character (the backslash of an escaped one).
    pub start: usize,
    /// The character after its last.
    pub end: usize,
    /// Whether a backslash escapes it.
    pub escaped: bool,
    /// What is between the braces.
    pub inside: String,
}

/// Every `{{...}}` in `chars`, escaped or not (`REFERENCE.finditer`).
fn found(chars: &[char]) -> Vec<Reference> {
    let braces = |at: usize, c: char| chars.get(at) == Some(&c) && chars.get(at + 1) == Some(&c);
    let inner = |from: usize| -> Option<usize> {
        let run = chars
            .get(from..)?
            .iter()
            .take(INSIDE_MAX + 1)
            .take_while(|c| **c != '{' && **c != '}')
            .count();
        ((1..=INSIDE_MAX).contains(&run) && braces(from + run, '}')).then_some(from + run)
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let escaped = chars.get(at) == Some(&'\\') && braces(at + 1, '{');
        let open = if escaped { at + 1 } else { at };
        let matched = (escaped || braces(at, '{')).then(|| inner(open + 2)).flatten();
        match matched {
            Some(close) => {
                out.push(Reference {
                    start: at,
                    end: close + 2,
                    escaped,
                    inside: chars.get(open + 2..close).unwrap_or_default().iter().collect(),
                });
                at = close + 2;
            }
            None => at += 1,
        }
    }
    out
}

/// The address a reference's inside names, or `None` when it is no reference (`_inside`):
/// its text stripped, when it starts with a known prefix and has more.
fn inside_address(text: &str, known: &[String]) -> Option<String> {
    let body = py_strip(text);
    let address = qualified(body).map_or(body, |(_, rest)| rest);
    known
        .iter()
        .any(|p| address.starts_with(p.as_str()) && address.chars().count() > p.chars().count())
        .then(|| body.to_owned())
}

/// The references in `text` that are not escaped: each and its address (`references`).
#[must_use]
pub fn references(text: &str, known: &[String]) -> Vec<(Reference, String)> {
    let chars: Vec<char> = text.chars().collect();
    found(&chars)
        .into_iter()
        .filter(|r| !r.escaped)
        .filter_map(|r| inside_address(&r.inside, known).map(|a| (r, a)))
        .collect()
}

/// A prefix as a model may write it meaning a known one: a dataset's, a table's (`_MEANT`).
fn meant_prefix(word: &str) -> Option<&'static str> {
    match word.to_lowercase().as_str() {
        "data" | "dataset" | "datasets" | "ds" | "set" => Some("d:"),
        "table" | "tables" | "tbl" | "tab" => Some("t:"),
        _ => None,
    }
}

/// A word and a colon at a reference's start, after a document's handle if it has one
/// (`_PREFIXED`, `(?:(d[0-9]+)/)?([A-Za-z][A-Za-z_]{0,15})\s*:\s*(.*)$`): the handle, the
/// word, what follows, and whether white space came between the colon and it.
fn prefixed(body: &str) -> Option<(Option<&str>, &str, &str, bool)> {
    let (handle, after) = match body.split_once('/') {
        Some((handle, rest)) if is_handle(handle) => (Some(handle), rest),
        _ => (None, body),
    };
    let word_len = after
        .char_indices()
        .take_while(|(i, c)| {
            if *i == 0 {
                c.is_ascii_alphabetic()
            } else {
                c.is_ascii_alphabetic() || *c == '_'
            }
        })
        .count();
    if word_len == 0 || word_len > 16 {
        return None;
    }
    let (word, rest) = after.split_at(word_len);
    let rest = rest.trim_start_matches(is_py_space).strip_prefix(':')?;
    let value = rest.trim_start_matches(is_py_space);
    Some((handle, word, value, value.len() != rest.len()))
}

/// A row by position or every row: a path segment no template text holds (`_ROW_SEGMENT`).
fn row_segment(segment: &Segment) -> bool {
    !segment.1
        && (segment.0 == "*"
            || segment
                .0
                .strip_prefix('#')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
}

/// `text`'s segments when it is a path: two or more, the first one word (`_path`).
fn path(text: &str) -> Option<Vec<Segment>> {
    let parts = segments(py_strip(text));
    let (first, quoted) = parts.first()?;
    if parts.len() < 2 || first.is_empty() || (!quoted && first.chars().any(is_py_space)) {
        return None;
    }
    Some(parts)
}

/// The references `{{inside}}` most likely meant, nearest first, when it has a reference's
/// shape but no source prefix a loaded format resolves; empty: the braces stay text
/// (`misread`). A reference returned may hold a placeholder (`<table>`).
#[must_use]
pub fn misread(inside: &str, known: &[String], datasets: &[String]) -> Vec<String> {
    let body = py_strip(inside);
    if body.is_empty() || inside_address(body, known).is_some() {
        return Vec::new();
    }
    let is_dataset = |id: &str| datasets.iter().any(|d| d == id);
    if let Some((handle, word, rest, spaced)) = prefixed(body) {
        let lead = handle.map_or(String::new(), |h| format!("{h}/"));
        let rest = py_strip(rest);
        let wanted = format!("{}:", word.to_lowercase());
        if let Some(own) = known.iter().find(|p| p.to_lowercase() == wanted) {
            return if rest.is_empty() {
                Vec::new()
            } else {
                vec![format!("{lead}{own}{rest}")]
            };
        }
        let meant = meant_prefix(word);
        let found = if spaced { None } else { path(rest) };
        let rowish = found.as_ref().is_some_and(|p| {
            p.first().is_some_and(|(first, _)| is_dataset(first)) || p.iter().skip(1).any(row_segment)
        });
        let single = meant.is_some() && !rest.is_empty() && !rest.chars().any(is_py_space);
        if !(single || rowish) {
            return Vec::new();
        }
        if let Some(meant) = meant.filter(|m| known.iter().any(|p| p == m)) {
            return vec![format!("{lead}{meant}{rest}")];
        }
        return [DATASET_PREFIX, "t:"]
            .iter()
            .filter(|p| known.iter().any(|k| k == *p))
            .map(|p| format!("{lead}{p}{rest}"))
            .collect();
    }
    let Some(parts) = path(body) else {
        return Vec::new();
    };
    let handle = parts.first().is_some_and(|(first, quoted)| !quoted && is_handle(first));
    let rest = if handle {
        body.split_once('/').map_or(body, |(_, r)| r)
    } else {
        body
    };
    let first = parts.get(usize::from(handle)).map_or("", |(f, _)| f.as_str());
    let rows = parts.iter().skip(1).any(row_segment);
    if !(handle || rows || is_dataset(first)) {
        return Vec::new();
    }
    let lead = parts.first().map_or("", |(f, _)| f.as_str());
    if handle && is_dataset(lead) {
        return vec![format!("{DATASET_PREFIX}{body}")];
    }
    if is_dataset(first) || !handle {
        return vec![format!("{DATASET_PREFIX}{rest}")];
    }
    let mut out: Vec<String> = known
        .iter()
        .filter(|p| *p == "t:")
        .map(|p| format!("{lead}/{p}<table>/{rest}"))
        .collect();
    out.push(format!("{DATASET_PREFIX}<dataset>/{rest}"));
    out
}

/// What a source prefix names, as a refusal says it.
fn prefix_name(prefix: &str) -> Option<&'static str> {
    match prefix {
        "d:" => Some("a dataset"),
        "t:" => Some("a table of an open document"),
        _ => None,
    }
}

/// The refusal of braces [`misread`] read as a reference: the nearest references, the
/// prefixes there are, and how braces are written as text (`misread_error`).
#[must_use]
pub fn misread_error(
    inside: &str,
    meant: &[String],
    known: &[String],
    field: &str,
    datasets: &[String],
    retry: &TryCall,
) -> ToolError {
    let mut by_length: Vec<&String> = known.iter().collect();
    by_length.sort_by_key(|p| p.chars().count());
    let forms: Vec<String> = by_length
        .iter()
        .map(|p| prefix_name(p).map_or_else(|| (*p).clone(), |name| format!("{p} ({name})")))
        .collect();
    let mut options: Vec<String> = meant.iter().filter(|m| !m.contains('<')).cloned().collect();
    let mut ids: Vec<&String> = datasets.iter().collect();
    ids.sort();
    for id in ids.into_iter().take(10) {
        let option = format!("{DATASET_PREFIX}{id}");
        if !options.contains(&option) {
            options.push(option);
        }
    }
    let near = meant
        .iter()
        .map(|m| format!("{{{{{m}}}}}"))
        .collect::<Vec<_>>()
        .join(" or ");
    let shown = py_strip(inside);
    ToolError::refusal(
        Refusal::InvalidArguments,
        format!(
            "{{{{{shown}}}}} looks like a reference but names no source: a reference starts with {}; did you mean \
             {near}? To keep the braces as text, put a backslash before them: \\{{{{{shown}}}}}",
            forms.join(", ")
        ),
        options_or(options, retry),
    )
    .at(field)
}

/// `text` with each reference's address through `qualify` (`t:x/...` to `d2/t:x/...`);
/// datasets, a document's own references (prefixes `own`) and escaped references as they
/// are (`qualify_text`).
///
/// # Errors
/// `qualify`'s.
pub fn qualify_text(
    text: &str,
    known: &[String],
    own: &[String],
    mut qualify: impl FnMut(&str) -> Result<String, ToolError>,
) -> Result<String, ToolError> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for reference in found(&chars) {
        let Some(address) = inside_address(&reference.inside, known) else {
            continue;
        };
        if reference.escaped || address.starts_with(DATASET_PREFIX) || super::own_prefix(&address, own).is_some() {
            continue;
        }
        out.extend(chars.get(at..reference.start).unwrap_or_default());
        out.push_str("{{");
        out.push_str(&qualify(&address)?);
        out.push_str("}}");
        at = reference.end;
    }
    out.extend(chars.get(at..).unwrap_or_default());
    Ok(out)
}

/// `text` with every reference replaced by `resolve(address, field)`'s text, and escaped
/// ones unescaped; the [`Fact`] of each, its place in the new text. With `datasets` (the
/// ids there are), braces shaped like a reference that is none are refused ([`misread`]),
/// before anything is resolved (`expand`).
///
/// # Errors
/// [`misread_error`]; more than [`MAX_FACTS`] references (`limit`); `resolve`'s.
pub fn expand(
    text: &str,
    field: &str,
    known: &[String],
    datasets: Option<&[String]>,
    retry: &TryCall,
    mut resolve: impl FnMut(&str, &str) -> Result<(String, Map<String, Value>), ToolError>,
) -> Result<(String, Vec<Fact>), ToolError> {
    let chars: Vec<char> = text.chars().collect();
    let all = found(&chars);
    if let Some(datasets) = datasets {
        for reference in all.iter().filter(|r| !r.escaped) {
            let meant = misread(&reference.inside, known, datasets);
            if !meant.is_empty() {
                return Err(misread_error(&reference.inside, &meant, known, field, datasets, retry));
            }
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut facts = Vec::new();
    let (mut at, mut length, mut count) = (0, 0, 0);
    for reference in all {
        let address = inside_address(&reference.inside, known);
        let misread_escape =
            reference.escaped && datasets.is_some_and(|d| !misread(&reference.inside, known, d).is_empty());
        if address.is_none() && !misread_escape {
            continue;
        }
        let before = chars.get(at..reference.start).unwrap_or_default();
        out.extend(before);
        length += before.len();
        at = reference.end;
        if reference.escaped {
            // `\{{...}}`: the text, without the backslash
            let literal = chars.get(reference.start + 1..reference.end).unwrap_or_default();
            out.extend(literal);
            length += literal.len();
            continue;
        }
        let Some(address) = address else { continue };
        count += 1;
        if count > MAX_FACTS {
            return Err(
                ToolError::failure(Failure::Limit, format!("more than {MAX_FACTS} references in one text"))
                    // (the count stops at the first over the limit: no `given`)
                    .with_detail("limit", Value::from(MAX_FACTS))
                    .at(field),
            );
        }
        let (written, record) = resolve(&address, field)?;
        let given = strip_format(qualified(&address).map_or(&address, |(_, rest)| rest)).to_owned();
        let size = written.chars().count();
        facts.push(Fact {
            field: field.to_owned(),
            start: length,
            end: length + size,
            text: written.clone(),
            address: given,
            record,
        });
        out.push_str(&written);
        length += size;
    }
    out.extend(chars.get(at..).unwrap_or_default());
    Ok((out, facts))
}

/// `node` with `visit(text, field)` applied to every string at the argument path `parts`
/// (`items[].text` split at the dots) -- in a list of lists too (`walk_texts`).
///
/// # Errors
/// `visit`'s.
pub fn walk_texts(
    node: &Value,
    parts: &[&str],
    field: &str,
    visit: &mut dyn FnMut(&str, &str) -> Result<String, ToolError>,
) -> Result<Value, ToolError> {
    let Some((part, rest)) = parts.split_first() else {
        return leaf(node, field, visit);
    };
    let (name, many) = part.strip_suffix("[]").map_or((*part, false), |n| (n, true));
    let Some(value) = node.as_object().and_then(|map| map.get(name)) else {
        return Ok(node.clone());
    };
    let path = if field.is_empty() {
        name.to_owned()
    } else {
        format!("{field}.{name}")
    };
    let walked = match value {
        Value::Array(items) if many => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let at = format!("{path}[{i}]");
                    if rest.is_empty() {
                        leaf(item, &at, visit)
                    } else {
                        walk_texts(item, rest, &at, visit)
                    }
                })
                .collect::<Result<_, _>>()?,
        ),
        _ => walk_texts(value, rest, &path, visit)?,
    };
    let mut map = node.as_object().cloned().unwrap_or_default();
    map.insert(name.to_owned(), walked);
    Ok(Value::Object(map))
}

fn leaf(
    value: &Value,
    field: &str,
    visit: &mut dyn FnMut(&str, &str) -> Result<String, ToolError>,
) -> Result<Value, ToolError> {
    match value {
        Value::String(text) => Ok(Value::String(visit(text, field)?)),
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| leaf(item, &format!("{field}[{i}]"), visit))
                .collect::<Result<_, _>>()?,
        )),
        other => Ok(other.clone()),
    }
}

/// `arguments` with every reference in a declared text (`paths`, the tool's facts paths)
/// replaced by the value it names, escaped ones unescaped, and what was written where
/// (the dispatcher's `_with_facts`). Refused whole when one names no single value.
///
/// # Errors
/// The first refusal of a reference.
pub fn expand_arguments(
    arguments: &Value,
    paths: &[String],
    env: &FactEnv<'_>,
    sources: &dyn FactSources,
) -> Result<(Value, Vec<Fact>), ToolError> {
    let mut facts = Vec::new();
    let mut out = arguments.clone();
    let mut visit = |text: &str, field: &str| -> Result<String, ToolError> {
        if !text.contains("{{") {
            return Ok(text.to_owned());
        }
        let (written, found) = expand(text, field, env.known, Some(env.datasets), env.retry, |address, at| {
            fact(address, at, env, sources)
        })?;
        facts.extend(found);
        Ok(written)
    };
    for path in paths {
        let parts: Vec<&str> = path.split('.').collect();
        out = walk_texts(&out, &parts, "", &mut visit)?;
    }
    Ok((out, facts))
}
