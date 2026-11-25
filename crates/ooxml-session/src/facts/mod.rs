//! Text facts (design phase 3, section 5; ooxml-edit's `textfacts`, ported exactly): a
//! source's value written into a text argument by reference, and where it came from.
//!
//! Owns: a reference in text, `{{` + a source address naming one value + `}}`
//! (`{{d:by_topic/door/mean}}`, `{{t:checks/N-1/rating:0.0}}`, `{{d2/t:x/#3/y}}`) found
//! ([`references`]) and replaced by the value in the call's locale ([`expand`], over the
//! declared text paths: [`expand_arguments`]), at most [`MAX_FACTS`] per text, escaped ones
//! (`\{{`) written literally; braces shaped like a reference that names no source refused
//! with the nearest one ([`misread`], [`misread_error`]); a reference's address qualified
//! with its document ([`qualify_text`]); one value picked from the rows a source names
//! ([`pick`]) and written ([`value::value_text`]), and the record kept of it ([`record`],
//! [`Fact`]); a document's own references (`{{n:7}}`, the prefixes its format resolves:
//! [`own_name`], [`own_record`], [`not_own`]).
//!
//! The host gives what a reference reads through [`FactSources`]: the rows a source names
//! (its datasets, other documents' tables) and the document's own references.
//!
//! Must not: hold datasets or documents, or know a format: its prefixes come from the
//! formats loaded ([`known_prefixes`]).

mod address;
mod expand;
mod value;

pub use address::{
    bad_format, date_suffix, format_error, is_date_pattern, py_strip, qualified, quote, segments, split, strip_format,
    Segment, FORMS,
};
pub use expand::{expand, expand_arguments, misread, misread_error, qualify_text, references, walk_texts, Reference};
pub use value::{date_of, is_number, number_text, value_text};

use address::unqualified;
use ooxml_base::pyfmt::repr_str;
use ooxml_numfmt::Locale;
use ooxml_tools::sources::{label, SourceRows};
use ooxml_tools::{ErrorCode, Refusal, ToolError, TryCall, WayOut};
use serde_json::{Map, Value};

/// How many values one argument may reference.
pub const MAX_FACTS: usize = 50;

/// The dataset prefix.
pub const DATASET_PREFIX: &str = "d:";

/// One reference written (`textfacts.Fact`).
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    /// The argument (`items[0].text`).
    pub field: String,
    /// Its value's first character in the argument's text as the handler gets it.
    pub start: usize,
    /// The character after its last.
    pub end: usize,
    /// The value written.
    pub text: String,
    /// The reference as given, without a document handle or its format.
    pub address: String,
    /// What the document keeps of it ([`record`]).
    pub record: Map<String, Value>,
}

impl Fact {
    /// What a result says of it: the reference, the value written, its sources (`brief`).
    #[must_use]
    pub fn brief(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("from".into(), Value::from(self.address.as_str()));
        out.insert("text".into(), Value::from(self.text.as_str()));
        for key in ["sources", "stale"] {
            if let Some(found) = self.record.get(key).filter(|v| truthy(v)) {
                out.insert(key.into(), found.clone());
            }
        }
        out
    }
}

/// Python's truth of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// What a reference reads: given by the host.
pub trait FactSources {
    /// The rows `source` names (`d:by_item`, `d2/t:checks/N-1`, `t:checks`): a dataset, or
    /// rows of a document the call holds, refused as the host refuses a source.
    ///
    /// # Errors
    /// The host's refusal of a source that names no rows.
    fn rows(&self, source: &str, field: &str) -> Result<SourceRows, ToolError>;

    /// The document's own reference `prefix` + `name` (a `$ref` in `name` the host's to
    /// resolve), as its format writes it: the text and the fields the format keeps; refused
    /// where the call's document takes no such reference ([`not_own`]).
    ///
    /// # Errors
    /// The format's refusal, or [`not_own`].
    fn own(&self, prefix: &str, name: &str, field: &str) -> Result<(String, Map<String, Value>), ToolError>;

    /// A run of a source's own addresses written compactly by its format (`merge_sources`);
    /// as they are by default.
    fn merge(&self, rows: &SourceRows, sources: Vec<String>) -> Vec<String> {
        let _ = rows;
        sources
    }
}

/// What a reference is read with: the prefixes the formats loaded resolve, the session's
/// dataset ids, the call's locale and the call itself (a refusal's way out).
#[derive(Clone, Copy)]
pub struct FactEnv<'a> {
    /// `d:`, the row prefixes and own prefixes of the formats loaded, longest first
    /// ([`known_prefixes`]).
    pub known: &'a [String],
    /// The own prefixes of the formats loaded, longest first.
    pub own: &'a [String],
    /// The dataset ids there are (the call's own first), for braces misread.
    pub datasets: &'a [String],
    /// The call's locale.
    pub locale: &'a Locale,
    /// The call as given, the way out of a refusal that names no option.
    pub retry: &'a TryCall,
}

/// `d:`, the prefixes the formats loaded resolve to rows (`row`) and their own references'
/// (`own`), longest first (`known_prefixes`); equal lengths in alphabetical order.
#[must_use]
pub fn known_prefixes(row: &[&str], own: &[&str]) -> Vec<String> {
    let mut found: Vec<String> = std::iter::once(DATASET_PREFIX)
        .chain(
            row.iter()
                .copied()
                .filter(|p| !p.is_empty() && !p.starts_with(DATASET_PREFIX)),
        )
        .chain(own.iter().copied())
        .map(str::to_owned)
        .collect();
    found.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b)));
    found.dedup();
    found
}

/// The own-reference prefix `address` starts with (after a document handle, if it has
/// one), the longest (`own_prefix`).
#[must_use]
pub fn own_prefix<'a>(address: &str, own: &'a [String]) -> Option<&'a str> {
    let body = unqualified(py_strip(address));
    let mut sorted: Vec<&String> = own.iter().collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
    sorted
        .into_iter()
        .find(|p| body.starts_with(p.as_str()))
        .map(String::as_str)
}

/// A way out naming `options`, else the call itself.
pub(crate) fn options_or(options: Vec<String>, retry: &TryCall) -> WayOut {
    WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone()))
}

/// The text a reference's `address` writes and its record (the dispatcher's `Call.fact`):
/// one value of a source, through its number format or date pattern, or a thing of the
/// call's own document.
///
/// # Errors
/// The source's refusal; a suffix that is no format ([`format_error`]); the value's.
pub fn fact(
    address: &str,
    field: &str,
    env: &FactEnv<'_>,
    sources: &dyn FactSources,
) -> Result<(String, Map<String, Value>), ToolError> {
    if let Some(prefix) = own_prefix(address, env.own) {
        let name = own_name(address, prefix, field, env.retry)?;
        let (text, fields) = sources.own(prefix, &name, field)?;
        let record = own_record(prefix, &name, &text, fields);
        return Ok((text, record));
    }
    fact_of(address, field, env, sources).map_err(|error| {
        let refused = matches!(
            error.code(),
            ErrorCode::Refusal(Refusal::NotFound | Refusal::InvalidArguments)
        );
        if let Some(suffix) = bad_format(address).filter(|_| refused) {
            return format_error(address, &suffix, Some(field));
        }
        if error.field().is_none() {
            error.at(field)
        } else {
            error
        }
    })
}

/// One value of a source (`Call._fact`).
fn fact_of(
    address: &str,
    field: &str,
    env: &FactEnv<'_>,
    sources: &dyn FactSources,
) -> Result<(String, Map<String, Value>), ToolError> {
    let (source, mut parts, format) = split(address);
    let mut selected = false;
    let rows = if source.starts_with(DATASET_PREFIX) {
        sources.rows(&source, field)?
    } else if parts.len() >= 2 {
        let (segment, quoted) = parts.remove(0);
        let numbered = segment.strip_prefix('#').is_some_and(|n| {
            n.starts_with(|c: char| ('1'..='9').contains(&c)) && n.bytes().all(|b| b.is_ascii_digit())
        });
        let raw = if quoted || !numbered { quote(&segment) } else { segment };
        selected = true;
        sources.rows(&format!("{source}/{raw}"), field)?
    } else {
        sources.rows(&source, field)?
    };
    let (row, column) = pick(&rows, &parts, address, field, env, selected)?;
    let text = value_text(
        rows.value(row, column),
        env.locale,
        (rows.type_of(column), format.as_deref()),
        &rows.where_is(row, column),
        field,
        env.retry,
    )?;
    let record = record(&rows, row, column, address, &text, format.as_deref(), sources);
    Ok((text, record))
}

/// The one value `segments` ([`split`]'s) names in `rows`: `(row, column)`;
/// `row_selected`: the source gave the row already, so one segment is a column (`pick`).
///
/// # Errors
/// More than one value named (`invalid_arguments`, an example); a row or column not found.
pub fn pick(
    rows: &SourceRows,
    segments: &[Segment],
    address: &str,
    field: &str,
    env: &FactEnv<'_>,
    row_selected: bool,
) -> Result<(usize, usize), ToolError> {
    let count = rows.rows.len();
    let (row_part, column_part) = match (row_selected, segments) {
        (true, _) => (None, segments.first()),
        (false, [row, column]) => (Some(row), Some(column)),
        (false, [column]) => (None, Some(column)),
        (false, []) => (None, None),
        (false, _) => {
            let said = format!("{address} has {} parts", segments.len() + 1);
            return Err(not_one(address, &said, rows, field, None, env.retry));
        }
    };
    let row = match row_part {
        None if count != 1 => {
            let said = format!("{} has {count} rows", rows.address);
            return Err(not_one(
                address,
                &said,
                rows,
                field,
                example(rows, env.locale, None),
                env.retry,
            ));
        }
        None => 0,
        Some((part, quoted)) => row_of(rows, part, *quoted, field, env)?,
    };
    let column = match column_part {
        None if rows.columns.len() != 1 => {
            let said = format!("{} has {} columns", rows.address, rows.columns.len());
            return Err(not_one(
                address,
                &said,
                rows,
                field,
                example(rows, env.locale, Some(row)),
                env.retry,
            ));
        }
        None => 0,
        Some((name, _)) => rows.column(name, field, env.retry)?,
    };
    Ok((row, column))
}

/// A reference to one value of `rows`, for a refusal's example (`_example`).
fn example(rows: &SourceRows, locale: &Locale, row: Option<usize>) -> Option<String> {
    let first = rows.rows.first()?;
    rows.columns.first()?;
    let row = row.unwrap_or(0);
    let values = rows.rows.get(row).unwrap_or(first);
    let key = values
        .first()
        .and_then(|v| label(v, locale, rows.type_of(0), None))
        .unwrap_or_else(|| format!("#{}", row + 1));
    let column = if rows.columns.len() > 1 {
        rows.columns.last()
    } else {
        rows.columns.first()
    }?;
    Some(format!("{}/{}/{}", rows.address, quote(&key), quote(column)))
}

/// The refusal of a reference that names more than one value (`_not_one`).
fn not_one(
    address: &str,
    said: &str,
    rows: &SourceRows,
    field: &str,
    example: Option<String>,
    retry: &TryCall,
) -> ToolError {
    let base = &rows.address;
    let shown = example.map_or_else(|| format!("{{{{{base}/<key>/<column>}}}}"), |e| format!("{{{{{e}}}}}"));
    let options = rows
        .columns
        .iter()
        .take(10)
        .map(|c| format!("{base}/<key>/{}", quote(c)))
        .collect();
    ToolError::refusal(
        Refusal::InvalidArguments,
        format!(
            "{{{{{address}}}}} is not one value ({said}): a reference names one, e.g. {shown}; a table or chart takes \
             rows with from"
        ),
        options_or(options, retry),
    )
    .at(field)
}

/// The row `part` names: `#3`, or the row whose key -- its first column's value as a table
/// cell writes it, else Python's text of it, else one equal case aside -- is `part` (`_row`).
fn row_of(rows: &SourceRows, part: &str, quoted: bool, field: &str, env: &FactEnv<'_>) -> Result<usize, ToolError> {
    let base = &rows.address;
    let count = rows.rows.len();
    let position = part.strip_prefix('#').filter(|n| {
        !quoted
            && (1..=7).contains(&n.len())
            && n.starts_with(|c: char| ('1'..='9').contains(&c))
            && n.bytes().all(|b| b.is_ascii_digit())
    });
    if let Some(index) = position.and_then(|n| n.parse::<usize>().ok()) {
        if index > count {
            let message = format!("{base} has {count} rows; there is no row #{index}");
            return Err(ToolError::refusal(Refusal::NotFound, message, WayOut::call(env.retry.clone())).at(field));
        }
        return Ok(index - 1);
    }
    let Some(first_column) = rows.columns.first() else {
        let message = format!("{base} has no columns");
        return Err(ToolError::refusal(Refusal::NotFound, message, WayOut::call(env.retry.clone())).at(field));
    };
    let keys: Vec<Option<String>> = (0..count)
        .map(|i| label(rows.value(i, 0), env.locale, rows.type_of(0), None))
        .collect();
    let mut found: Vec<usize> = (0..count)
        .filter(|i| {
            keys.get(*i).is_some_and(|k| k.as_deref() == Some(part))
                || python_str(rows.value(*i, 0)).as_deref() == Some(part)
        })
        .collect();
    if found.is_empty() {
        let folded = casefold(part);
        found = (0..count)
            .filter(|i| {
                keys.get(*i)
                    .and_then(Option::as_deref)
                    .is_some_and(|k| casefold(k) == folded)
            })
            .collect();
    }
    match found.as_slice() {
        [one] => Ok(*one),
        [] => {
            let message = format!(
                "{base} has no row whose key ({}) is {}",
                repr_str(first_column),
                repr_str(part)
            );
            let options = keys
                .iter()
                .flatten()
                .take(20)
                .map(|k| format!("{base}/{}", quote(k)))
                .collect();
            Err(ToolError::refusal(Refusal::NotFound, message, options_or(options, env.retry)).at(field))
        }
        many => {
            let message = format!(
                "{} rows of {base} have the key {}: name the row by position",
                many.len(),
                repr_str(part)
            );
            let options = many.iter().take(10).map(|i| format!("{base}/#{}", i + 1)).collect();
            Err(ToolError::refusal(Refusal::Ambiguous, message, options_or(options, env.retry)).at(field))
        }
    }
}

/// Python's `str.casefold` for what keys hold (lower case, `ß` as `ss`).
fn casefold(text: &str) -> String {
    text.to_lowercase().replace('ß', "ss")
}

/// Python's `str(value)` of a source's value; `None` for a blank, or a value whose text
/// Python's would not be known here.
fn python_str(value: &ooxml_tools::sources::Cell) -> Option<String> {
    use ooxml_tools::sources::Cell;
    let clock = |c: &ooxml_tools::sources::Clock| {
        let base = format!("{:02}:{:02}:{:02}", c.hour, c.minute, c.second);
        if c.micro == 0 {
            base
        } else {
            format!("{base}.{:06}", c.micro)
        }
    };
    let day = |d: &ooxml_tools::sources::Day| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day);
    match value {
        Cell::Blank | Cell::Other(_) => None,
        Cell::Text(t) => Some(t.clone()),
        Cell::Bool(b) => Some(if *b { "True" } else { "False" }.to_owned()),
        Cell::Int(digits) => Some(digits.clone()),
        Cell::Float(f) => Some(ooxml_base::pyfmt::repr_float(*f)),
        Cell::Date(d) => Some(day(d)),
        Cell::DateTime(d, c) => Some(format!("{} {}", day(d), clock(c))),
        Cell::Time(c) => Some(clock(c)),
    }
}

/// What a document keeps of one value written (an `ooxml_edit.provenance` record without
/// `v`): the reference as given (no document handle, no format), the document's name, its
/// version, the format, the text written, its sources, whether stale (`record`).
#[must_use]
pub fn record(
    rows: &SourceRows,
    row: usize,
    column: usize,
    address: &str,
    text: &str,
    format: Option<&str>,
    sources: &dyn FactSources,
) -> Map<String, Value> {
    let given = strip_format(address);
    let mut out = Map::new();
    out.insert("source".into(), Value::from(unqualified(given)));
    if let Some(name) = rows.name.as_deref().filter(|n| !n.is_empty()) {
        out.insert("document".into(), Value::from(name));
    }
    if let Some(version) = rows.version {
        out.insert("version".into(), Value::from(version));
    }
    if let Some(format) = format.filter(|f| !f.is_empty()) {
        out.insert("format".into(), Value::from(format));
    }
    out.insert("text".into(), Value::from(text));
    if let Some(found) = rows.sources.as_ref() {
        let own = found.get(row).and_then(|r| r.get(column)).cloned().unwrap_or_default();
        let mut unique: Vec<String> = Vec::with_capacity(own.len());
        for source in own {
            if !unique.contains(&source) {
                unique.push(source);
            }
        }
        let merged = if unique.is_empty() {
            unique
        } else {
            sources.merge(rows, unique)
        };
        out.insert("sources".into(), Value::from(merged));
    }
    if rows.stale {
        out.insert("stale".into(), Value::Bool(true));
    }
    out
}

/// The name a reference to the document's own `prefix` gives (`7` of `{{n:7}}`), refused
/// with the form when it carries a document handle or a number format (`own_name`).
///
/// # Errors
/// `invalid_arguments` at `field`, as above.
pub fn own_name(address: &str, prefix: &str, field: &str, retry: &TryCall) -> Result<String, ToolError> {
    let body = py_strip(address);
    let refuse =
        |message: String| ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(retry.clone())).at(field);
    if let Some((_, rest)) = qualified(body) {
        return Err(refuse(format!(
            "{{{{{body}}}}}: a {prefix} reference names a thing of the document it is written into; leave the \
                 document out: {{{{{rest}}}}}"
        )));
    }
    let name = py_strip(body.get(prefix.len()..).unwrap_or(""));
    if let Some(found) = address::format_at(name) {
        let bare = name.get(..found.at).unwrap_or(name);
        return Err(refuse(format!(
            "{{{{{body}}}}}: a {prefix} reference takes no number format; write {{{{{prefix}{bare}}}}}"
        )));
    }
    Ok(name.to_owned())
}

/// What a document keeps of an own reference written: `source` (the prefix and the name
/// resolved), `text`, and the format's `fields` (`own_record`).
#[must_use]
pub fn own_record(prefix: &str, name: &str, text: &str, fields: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("source".into(), Value::from(format!("{prefix}{name}")));
    out.insert("text".into(), Value::from(text));
    out.extend(fields);
    out
}

/// The refusal of an own reference in a document whose format does not resolve its
/// prefix: in the format's words (`said`), else naming the kinds whose documents do
/// (`not_own`).
#[must_use]
pub fn not_own(
    address: &str,
    prefix: &str,
    kind: Option<&str>,
    takers: &[&str],
    field: &str,
    said: Option<&str>,
    retry: &TryCall,
) -> ToolError {
    let why = said.map_or_else(
        || {
            let takers = if takers.is_empty() {
                "other".to_owned()
            } else {
                takers.join(" and ")
            };
            format!(
                "{prefix} references are for {takers} documents; a {} takes none",
                kind.unwrap_or("document like this")
            )
        },
        |said| py_strip(said).trim_end_matches('.').to_owned(),
    );
    ToolError::refusal(
        Refusal::InvalidArguments,
        format!(
            "{{{{{}}}}}: {why}. Write the text itself, or \\{{{{ for the braces",
            py_strip(address)
        ),
        WayOut::call(retry.clone()),
    )
    .at(field)
}
