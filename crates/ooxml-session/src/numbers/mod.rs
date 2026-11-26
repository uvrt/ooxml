//! Numbers in text that no value holds: `check`'s `include: ["numbers"]` (design phase 3,
//! section 5; ooxml-edit's `numbers`, ported exactly), a session service for every format.
//!
//! Owns: the application's rules ([`NumberRules`]); the texts a format gives a check
//! ([`NumberText`]); the numbers of a text and those left aside ([`find_numbers`]); the index
//! of the session's values ([`Index`]: its datasets, and the tables of the open documents
//! whose format lists them); and the fact ([`report`]): per text, its unverified numbers --
//! no value rounds to them at the decimals written -- with the nearest values and where they
//! came from, and its typed ones -- a value matches, but it was typed -- with the reference
//! that holds it and, where the reference writes the same text, the `replace_text` call that
//! writes it as one; within the rules' caps.
//!
//! Must not: read a document (its format gives the texts), resolve a source (the host gives
//! the datasets and what a reference writes), or keep the index (the host does).

mod index;
mod read;

pub use index::{How, Index, Source, Value as IndexedValue};
pub use read::{find_numbers, parse, shown, Decimal, Found, Numbers};
pub(crate) use read::compiled;

use crate::facts::quote;
use ooxml_base::pyfmt::repr_float;
use ooxml_numfmt::Locale;
use ooxml_tools::sources::{label, Cell};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// `check`'s `include` value that asks for this fact.
pub const INCLUDE: &str = "numbers";

/// What a check says once, when it lists anything: the rule, in a line.
pub const NOTE: &str = "numbers: a number is unverified when no value of the session's datasets or open \
documents' tables rounds to it at the decimals it is written with (2.66 shown as 2,7 matches); typed: a \
value matches, but it was typed -- the fix writes it as a reference. Written by reference and placed with \
from: left out";

/// Python's `str.casefold` as the ported tools read it (lower case, `ß` as `ss`).
pub(crate) fn casefold(text: &str) -> String {
    text.to_lowercase().replace('ß', "ss")
}

/// The application's settings for `check`'s numbers (`NumberRules`; never a tool argument):
/// numbers and phrases that are no figures, whether whole numbers are compared, the texts
/// listed at most and the numbers per list of one text at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberRules {
    /// The numbers never listed, each as [`Decimal::normalized`] writes one (a sign kept).
    pub(crate) numbers: Vec<(String, i64)>,
    /// The phrases whose numbers are left aside, folded.
    pub(crate) phrases: Vec<String>,
    /// Whether whole numbers without a percent sign are compared.
    pub whole_numbers: bool,
    /// The texts listed at most.
    pub texts: usize,
    /// The numbers per list of one text at most.
    pub per_text: usize,
}

impl Default for NumberRules {
    fn default() -> Self {
        NumberRules {
            numbers: Vec::new(),
            phrases: Vec::new(),
            whole_numbers: false,
            texts: 20,
            per_text: 5,
        }
    }
}

/// The fields `NumberRules` takes, as a refusal of others names them.
const RULE_FIELDS: [&str; 4] = ["ignore", "whole_numbers", "texts", "per_text"];

impl NumberRules {
    /// The rules a mapping of `NumberRules`' fields gives (`rules_of`): `null` is the
    /// default.
    ///
    /// # Errors
    /// The application's mistake, in Python's words: a field it lacks, an ignore item that is
    /// no number or phrase, a cap below 1.
    pub fn from_json(given: &Value) -> Result<NumberRules, String> {
        let fields = match given {
            Value::Null => return Ok(NumberRules::default()),
            Value::Object(fields) => fields,
            other => {
                return Err(format!(
                    "numbers takes NumberRules or a mapping of its fields, not {}",
                    type_name(other)
                ))
            }
        };
        if let Some(unknown) = fields.keys().find(|k| !RULE_FIELDS.contains(&k.as_str())) {
            return Err(format!(
                "numbers takes {}: NumberRules.__init__() got an unexpected keyword argument '{unknown}'",
                RULE_FIELDS.join(", ")
            ));
        }
        let mut rules = NumberRules::default();
        match fields.get("ignore") {
            None => {}
            Some(Value::Array(items)) => rules.ignore(items)?,
            Some(one) => rules.ignore(std::slice::from_ref(one))?,
        }
        if let Some(whole) = fields.get("whole_numbers") {
            rules.whole_numbers = truthy(whole);
        }
        for (name, cap) in [("texts", &mut rules.texts), ("per_text", &mut rules.per_text)] {
            if let Some(value) = fields.get(name) {
                *cap = value
                    .as_u64()
                    .filter(|n| *n >= 1)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| format!("{name} is a whole number of at least 1, not {}", py_repr(value)))?;
            }
        }
        Ok(rules)
    }

    /// The ignore list's items split into numbers and phrases (`_split`).
    fn ignore(&mut self, items: &[Value]) -> Result<(), String> {
        for item in items {
            match item {
                Value::Number(n) => {
                    let text = n.as_f64().filter(|_| n.is_f64()).map_or_else(|| n.to_string(), repr_float);
                    self.numbers.push(decimal_key(&text).unwrap_or((text, 0)));
                }
                Value::String(text) if !crate::facts::py_strip(text).is_empty() => {
                    let stripped = crate::facts::py_strip(text);
                    match decimal_key(stripped) {
                        Some(key) => self.numbers.push(key),
                        None => self.phrases.push(casefold(stripped)),
                    }
                }
                other => return Err(format!("ignore lists numbers and phrases, not {}", py_repr(other))),
            }
        }
        Ok(())
    }
}

/// A value as Python's `repr` writes it, for an application's mistake.
fn py_repr(value: &Value) -> String {
    match value {
        Value::String(s) => ooxml_base::pyfmt::repr_str(s),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Null => "None".to_owned(),
        other => other.to_string(),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        _ => "NoneType",
    }
}

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

/// `text` as `Decimal(text).normalize()` compares it -- `(digits, exponent)`, a negative
/// one's digits after `-`, zero as `("0", 0)` whatever its sign -- or `None` where Python's
/// `Decimal` reads no number (`iso 9001`).
fn decimal_key(text: &str) -> Option<(String, i64)> {
    let (negative, body) = match text.strip_prefix(['-', '+']) {
        Some(rest) => (text.starts_with('-'), rest),
        None => (false, text),
    };
    let folded = body.to_ascii_lowercase();
    if matches!(folded.as_str(), "inf" | "infinity" | "nan" | "snan") {
        return Some((format!("{}{folded}", if negative { "-" } else { "" }), 0));
    }
    let (mantissa, exponent) = match folded.split_once('e') {
        Some((m, e)) => (m, e.parse::<i64>().ok()?),
        None => (folded.as_str(), 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |s: &str| s.chars().all(|c| ooxml_base::pystr::decimal_value(c).is_some());
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return None;
    }
    let ascii: String = whole
        .chars()
        .chain(fraction.chars())
        .filter_map(|c| ooxml_base::pystr::decimal_value(c).and_then(|d| char::from_digit(d, 10)))
        .collect();
    let (kept, power) = read::normalized(&ascii, exponent - i64::try_from(fraction.chars().count()).ok()?);
    let sign = if negative && kept != "0" { "-" } else { "" };
    Some((format!("{sign}{kept}"), power))
}

/// One text a check of numbers reads, as a format gives it (`NumberText`): its address as
/// the format's tools take it, its own text (paragraphs joined by `\n`), the character spans
/// written by reference (left out), the `replace_text` arguments that name where it is (for
/// the fix), and what a reader is told of its place (`{"page": 3}`).
#[derive(Debug, Clone, PartialEq)]
pub struct NumberText {
    /// Its address.
    pub at: String,
    /// Its own text.
    pub text: String,
    /// The `(start, end)` character spans written by reference.
    pub written: Vec<(usize, usize)>,
    /// The `replace_text` arguments that name where it is.
    pub scope: Map<String, Value>,
    /// What a reader is told of its place.
    pub place: Map<String, Value>,
}

/// What a report reads beside the index: the call's locale, its document's name, and what a
/// reference writes (`Call.fact`).
pub trait NumbersEnv {
    /// The call's locale.
    fn locale(&self) -> &Locale;
    /// The call's document, as the call names it.
    fn doc(&self) -> Option<&str>;
    /// The text the reference `address` (with its format) writes; `None` where it is refused.
    fn fact(&self, address: &str) -> Option<String>;
}

/// A value as a reader compares it with the text (`_value_text`): as its format shows it
/// (or in General), a percentage when quoted as one.
fn value_text(value: &IndexedValue, how: How, locale: &Locale) -> String {
    if let Some((decimals, pct, divide)) = value.format {
        let mut format = "0".to_owned();
        if decimals > 0 {
            format.push('.');
            format.push_str(&"0".repeat(decimals));
        }
        if pct || how == How::Hundred {
            format.push('%');
            return crate::facts::number_text(&Cell::Float(value.number), &format, locale).unwrap_or_default();
        }
        #[allow(clippy::cast_precision_loss)]
        let shown = value.number / divide as f64;
        return crate::facts::number_text(&Cell::Float(shown), &format, locale).unwrap_or_default();
    }
    if how == How::Hundred {
        let hundred = times_hundred(value.number);
        return label(&Cell::Float(hundred), locale, None, None).unwrap_or_default() + "%";
    }
    label(&Cell::Float(value.number), locale, None, None).unwrap_or_else(|| repr_float(value.number))
}

/// `float(Decimal(repr(number)) * 100)`.
fn times_hundred(number: f64) -> f64 {
    let repr = repr_float(number);
    let scaled = match repr.split_once('e') {
        Some((mantissa, exponent)) => format!("{mantissa}e{}", exponent.parse::<i64>().unwrap_or(0) + 2),
        None => format!("{repr}e2"),
    };
    scaled.parse().unwrap_or(number * 100.0)
}

/// The reference of one value, as a model writes it (`_address`): its source, the row by its
/// key -- the first column's label, when no other row has it; of a document's table only when
/// the table is keyed by its first column -- or by position, the column.
fn address(index: &Index, value: &IndexedValue) -> String {
    let Some(source) = index.sources.get(value.source) else {
        return String::new();
    };
    let dataset = &source.dataset;
    let keys = index.keys(value.source);
    let key = keys.get(value.row).cloned().flatten();
    let keyed = source.shown.starts_with(crate::facts::DATASET_PREFIX) || dataset.keys() > 0;
    let unique = key.as_deref().filter(|k| {
        keyed
            && value.column != 0
            && keys.iter().filter(|o| o.as_deref() == Some(*k)).count() == 1
            && !numbered(k)
    });
    let part = unique.map_or_else(|| format!("#{}", value.row + 1), quote);
    let column = dataset.columns().get(value.column).map(String::as_str).unwrap_or_default();
    format!("{}/{part}/{}", source.shown, quote(column))
}

/// Whether `key` is `#` and digits (`re.fullmatch(r"#\d+", key)`).
fn numbered(key: &str) -> bool {
    key.strip_prefix('#')
        .is_some_and(|d| !d.is_empty() && d.chars().all(ooxml_base::pystr::is_decimal))
}

/// The first two sources of one value (`_sources_of`).
fn sources_of(index: &Index, value: &IndexedValue) -> Vec<String> {
    index
        .sources
        .get(value.source)
        .and_then(|s| s.dataset.sources())
        .and_then(|rows| rows.get(value.row))
        .and_then(|row| row.get(value.column))
        .map(|found| found.iter().take(2).cloned().collect())
        .unwrap_or_default()
}

/// `matched` without a value whose sources an earlier one shares (`_distinct`): a dataset's
/// value and the table cell it was derived from are one value (the dataset's, first).
fn distinct(index: &Index, matched: Vec<(usize, How)>) -> Vec<(usize, How)> {
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (i, how) in matched {
        let Some(value) = index.values.get(i) else { continue };
        let mut own: Vec<String> = index
            .sources
            .get(value.source)
            .and_then(|s| s.dataset.sources())
            .and_then(|rows| rows.get(value.row))
            .and_then(|row| row.get(value.column))
            .cloned()
            .unwrap_or_default();
        own.push(address(index, value));
        if own.iter().any(|o| seen.contains(o)) {
            continue;
        }
        seen.extend(own);
        out.push((i, how));
    }
    out
}

/// The number format that writes a value as `number` is written (`_format_for`), or `None`.
fn format_for(number: &Found, how: How) -> Option<String> {
    if number.scale.is_some() || number.percent == Some(1000) {
        return None;
    }
    let mut format = if number.grouped { "#,##0" } else { "0" }.to_owned();
    if number.decimals > 0 {
        format.push('.');
        format.push_str(&"0".repeat(number.decimals));
    }
    if number.percent.is_some() && how == How::Hundred {
        format.push('%');
    }
    Some(format)
}

/// One typed number's place in its text, and the texts of its scope (`_fix`'s arguments).
struct Typed<'a> {
    item: &'a NumberText,
    number: &'a Found,
    siblings: &'a [&'a NumberText],
}

/// The `replace_text` call that writes `typed`'s number as the reference `address:format`
/// (`shown`: the address as the model writes it), when the reference writes the same number
/// -- `writes` saying how, when the text wrote it otherwise -- else `None` (`_fix`).
fn fix(env: &dyn NumbersEnv, typed: &Typed<'_>, address: &str, shown: &str, format: &str) -> Option<Map<String, Value>> {
    let written = env.fact(&format!("{address}:{format}"))?;
    let number = typed.number;
    let percent = format.ends_with('%');
    let start = if number.negative { number.start } else { number.digits.0 };
    let end = if percent { number.end } else { number.digits.1 };
    if written.starts_with('-') != number.negative {
        return None;
    }
    let all = NumberRules {
        whole_numbers: true,
        ..NumberRules::default()
    };
    let (again, _) = find_numbers(&written, env.locale(), &all, &[]).ok()?;
    match again.as_slice() {
        [one] if one.value.normalized() == number.value.normalized() && one.decimals == number.decimals => {}
        _ => return None,
    }
    let text = &typed.item.text;
    let replaced = format!("{{{{{shown}:{format}}}}}");
    let line_start = text.get(..start).and_then(|b| b.rfind('\n')).map_or(0, |n| n + 1);
    let line_end = text.get(end..).and_then(|a| a.find('\n')).map_or(text.len(), |n| end + n);
    for width in [12, 24, 48, 1_000_000] {
        let snippet = snippet_around(text, (start, end), (line_start, line_end), width);
        let (left, right) = snippet;
        let found = text.get(left..right).unwrap_or_default();
        let times: usize = typed.siblings.iter().map(|o| o.text.matches(found).count()).sum();
        if times == 1 {
            return Some(fix_call(env, typed, (left, right), (start, end), &replaced, &written));
        }
    }
    None
}

/// The bytes of the words around `at` -- `width` characters each side, out to whole words,
/// within its line.
fn snippet_around(text: &str, at: (usize, usize), line: (usize, usize), width: usize) -> (usize, usize) {
    let space = crate::facts::is_py_space;
    let mut left = at.0;
    for (i, _) in text.get(line.0..at.0).unwrap_or_default().char_indices().rev().take(width) {
        left = line.0 + i;
    }
    let mut right = at.1;
    for (n, c) in text.get(at.1..line.1).unwrap_or_default().chars().enumerate() {
        if n >= width {
            break;
        }
        right += c.len_utf8();
    }
    while left > line.0 {
        match text.get(..left).and_then(|b| b.chars().next_back()) {
            Some(c) if !space(c) => left -= c.len_utf8(),
            _ => break,
        }
    }
    while right < line.1 {
        match text.get(right..).and_then(|a| a.chars().next()) {
            Some(c) if !space(c) => right += c.len_utf8(),
            _ => break,
        }
    }
    (left, right)
}

/// The fix's call: `replace_text` of the snippet `(left, right)` with the number at `at`
/// written as `replaced`.
fn fix_call(
    env: &dyn NumbersEnv,
    typed: &Typed<'_>,
    (left, right): (usize, usize),
    (start, end): (usize, usize),
    replaced: &str,
    written: &str,
) -> Map<String, Value> {
    let text = &typed.item.text;
    let mut arguments = Map::new();
    if let Some(doc) = env.doc() {
        arguments.insert("doc".into(), Value::from(doc));
    }
    let snippet = text.get(left..right).unwrap_or_default();
    let replace = format!(
        "{}{replaced}{}",
        text.get(left..start).unwrap_or_default(),
        text.get(end..right).unwrap_or_default()
    );
    arguments.insert("find".into(), Value::from(snippet));
    arguments.insert("replace".into(), Value::from(replace));
    arguments.insert("expect".into(), Value::from("one"));
    for (key, value) in &typed.item.scope {
        arguments.insert(key.clone(), value.clone());
    }
    let mut out = Map::new();
    out.insert("tool".into(), Value::from("replace_text"));
    out.insert("arguments".into(), Value::Object(arguments));
    if text.get(start..end) != Some(written) {
        out.insert("writes".into(), Value::from(written));
    }
    out
}

/// What one text's numbers say: its unverified and typed entries.
struct Listed {
    unverified: Vec<Value>,
    typed: Vec<Value>,
}

/// One text's numbers compared with the index.
fn compared(env: &dyn NumbersEnv, index: &Index, found: &[Found], item: &NumberText, siblings: &[&NumberText]) -> Listed {
    let mut listed = Listed {
        unverified: Vec::new(),
        typed: Vec::new(),
    };
    for number in found {
        let matched = index.matches(number);
        if matched.is_empty() {
            listed.unverified.push(unverified(env, index, number));
            continue;
        }
        let matched = distinct(index, matched);
        let mut addresses: Vec<String> = Vec::new();
        for (i, _) in &matched {
            if let Some(value) = index.values.get(*i) {
                let found = address(index, value);
                if !addresses.contains(&found) {
                    addresses.push(found);
                }
            }
        }
        let typed = Typed { item, number, siblings };
        listed.typed.push(typed_entry(env, index, &typed, &matched, &addresses));
    }
    listed
}

/// An unverified number with its nearest values.
fn unverified(env: &dyn NumbersEnv, index: &Index, number: &Found) -> Value {
    let mut near = Vec::new();
    for (i, how, _) in index.nearest(number, 2) {
        let Some(value) = index.values.get(i) else { continue };
        let mut close = Map::new();
        close.insert("value".into(), Value::from(value_text(value, how, env.locale())));
        close.insert("from".into(), Value::from(address(index, value)));
        let sources = sources_of(index, value);
        if !sources.is_empty() {
            close.insert("sources".into(), Value::from(sources));
        }
        near.push(Value::Object(close));
    }
    let mut entry = Map::new();
    entry.insert("text".into(), Value::from(number.text.as_str()));
    entry.insert("near".into(), Value::Array(near));
    Value::Object(entry)
}

/// A typed number: the reference that holds it, or the first three of several and their
/// count; with one, its sources and the fix.
fn typed_entry(
    env: &dyn NumbersEnv,
    index: &Index,
    typed: &Typed<'_>,
    matched: &[(usize, How)],
    addresses: &[String],
) -> Value {
    let mut entry = Map::new();
    entry.insert("text".into(), Value::from(typed.number.text.as_str()));
    let first = addresses.first().cloned().unwrap_or_default();
    entry.insert("from".into(), Value::from(first.as_str()));
    if addresses.len() > 1 {
        entry.insert("from".into(), Value::from(addresses.iter().take(3).cloned().collect::<Vec<_>>()));
        entry.insert("matches".into(), Value::from(addresses.len()));
        return Value::Object(entry);
    }
    let Some((value, how)) = matched.first().and_then(|(i, how)| Some((index.values.get(*i)?, *how))) else {
        return Value::Object(entry);
    };
    let sources = sources_of(index, value);
    if !sources.is_empty() {
        entry.insert("sources".into(), Value::from(sources));
    }
    let source = index.sources.get(value.source);
    let format = format_for(typed.number, how);
    if let (Some(format), Some(source)) = (format, source) {
        let full = format!("{}{}", source.address, first.get(source.shown.len()..).unwrap_or_default());
        if let Some(fix) = fix(env, typed, &full, &first, &format) {
            entry.insert("fix".into(), Value::Object(fix));
        }
    }
    Value::Object(entry)
}

/// The key the texts of one scope are grouped by (`_scope_key`).
fn scope_key(item: &NumberText) -> String {
    let mut pairs: Vec<(&String, &Value)> = item.scope.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    serde_json::to_string(&pairs).unwrap_or_default()
}

/// The fact (`report`): per text, its unverified numbers and its typed ones, within `rules`'
/// caps, compared with `index` (`notes`: what could not be read for it); `narrower`: the
/// arguments that narrow the check, named when something did not fit.
///
/// # Errors
/// The patterns' failure to compile (an `internal` failure).
pub fn report(
    env: &dyn NumbersEnv,
    index: &Index,
    mut notes: Vec<String>,
    items: &[NumberText],
    rules: &NumberRules,
    narrower: &[&str],
) -> Result<Map<String, Value>, String> {
    let mut texts: Vec<Value> = Vec::new();
    let (mut left_out, mut unverified_count, mut typed_count) = (0_usize, 0_usize, 0_usize);
    let mut aside: BTreeMap<String, usize> = BTreeMap::new();
    let keys: Vec<String> = items.iter().map(scope_key).collect();
    for (item, key) in items.iter().zip(&keys) {
        let (found, skipped) = find_numbers(&item.text, env.locale(), rules, &item.written)?;
        for (reason, count) in skipped {
            *aside.entry(reason).or_default() += count;
        }
        if found.is_empty() || index.is_empty() {
            continue;
        }
        let siblings: Vec<&NumberText> = items.iter().zip(&keys).filter(|(_, k)| *k == key).map(|(i, _)| i).collect();
        let Listed { unverified, typed } = compared(env, index, &found, item, &siblings);
        unverified_count += unverified.len();
        typed_count += typed.len();
        if unverified.is_empty() && typed.is_empty() {
            continue;
        }
        if texts.len() >= rules.texts {
            left_out += 1;
            continue;
        }
        texts.push(Value::Object(listed_text(item, unverified, typed, rules.per_text)));
    }
    let mut data = Map::new();
    data.insert("unverified".into(), Value::from(unverified_count));
    data.insert("typed".into(), Value::from(typed_count));
    data.insert("values".into(), Value::from(index.len()));
    if left_out > 0 {
        data.insert("texts_left_out".into(), Value::from(left_out));
    }
    if !aside.is_empty() {
        data.insert("left_aside".into(), Value::Object(aside.into_iter().map(|(k, v)| (k, Value::from(v))).collect()));
    }
    if index.is_empty() {
        notes.push(
            "no values to compare with: no dataset is defined and no open document lists tables (open the \
             document that holds them, or define a dataset)"
                .to_owned(),
        );
    }
    Ok(finish(data, texts, notes, left_out, narrower))
}

/// One text listed: where, its unverified and typed numbers within the cap, and how many
/// more there are.
fn listed_text(item: &NumberText, unverified: Vec<Value>, typed: Vec<Value>, per_text: usize) -> Map<String, Value> {
    let mut listed = Map::new();
    listed.insert("at".into(), Value::from(item.at.as_str()));
    for (key, value) in &item.place {
        listed.insert(key.clone(), value.clone());
    }
    let mut more = 0;
    for (name, list) in [("unverified", unverified), ("typed", typed)] {
        if list.is_empty() {
            continue;
        }
        more += list.len().saturating_sub(per_text);
        listed.insert(name.into(), Value::Array(list.into_iter().take(per_text).collect()));
    }
    if more > 0 {
        listed.insert("more".into(), Value::from(more));
    }
    listed
}

/// The fact's texts, notes and hints after its counts.
fn finish(
    mut data: Map<String, Value>,
    texts: Vec<Value>,
    notes: Vec<String>,
    left_out: usize,
    narrower: &[&str],
) -> Map<String, Value> {
    let fixes = texts
        .iter()
        .flat_map(|t| t.get("typed").and_then(Value::as_array).into_iter().flatten())
        .filter(|entry| entry.get("fix").is_some())
        .count();
    let more = texts.iter().any(|t| t.get("more").is_some());
    let listed = !texts.is_empty();
    // (texts before texts_left_out and left_aside, as Python's dict is built)
    let mut ordered = Map::new();
    for key in ["unverified", "typed", "values"] {
        if let Some(value) = data.remove(key) {
            ordered.insert(key.into(), value);
        }
    }
    ordered.insert("texts".into(), Value::Array(texts));
    ordered.append(&mut data);
    if !notes.is_empty() {
        ordered.insert("notes".into(), Value::from(notes));
    }
    if listed {
        ordered.insert("note".into(), Value::from(NOTE));
    }
    if fixes > 1 {
        ordered.insert(
            "fix_hint".into(),
            Value::from(format!(
                "{fixes} fixes: send them as one batch (ops of replace_text), one undo step"
            )),
        );
    }
    if listed && (left_out > 0 || more) {
        let named = if narrower.is_empty() {
            String::new()
        } else {
            format!(" ({})", narrower.join(", "))
        };
        ordered.insert(
            "left_out_hint".into(),
            Value::from(format!("check fewer at a time to see the rest{named}")),
        );
    }
    ordered
}

/// The fact's summary (`summary`).
#[must_use]
pub fn summary(data: &Map<String, Value>) -> String {
    let count = |key: &str| data.get(key).and_then(Value::as_u64).unwrap_or(0);
    let texts = data.get("texts").and_then(Value::as_array).map_or(0, Vec::len) + usize::try_from(count("texts_left_out")).unwrap_or(0);
    format!(
        "{} unverified and {} typed number(s) in {texts} text(s), against {} value(s)",
        count("unverified"),
        count("typed"),
        count("values")
    )
}

#[cfg(test)]
mod tests;
