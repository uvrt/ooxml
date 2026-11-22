//! Owns: the rows a call's source names (ooxml-edit's `sources.Rows`: a dataset `d:<id>`, a run
//! of its rows, or a table of another open document `t:<name>`), as the host resolved them
//! before the call ([`SourceRows`]: the columns, their type labels, the values, each value's
//! own sources, and where they came from), and what a call reads of them: a column by its
//! name ([`SourceRows::column`]), a value named for a message ([`SourceRows::where_is`]),
//! a value as a cell's text ([`label`]), a column's values' provenance
//! ([`SourceRows::provenance`]).
//!
//! The host resolves and qualifies a source (its dispatcher's `call.source`: an unknown
//! dataset, a row past the last, a source of the call's own document are its refusals) and
//! hands the rows over with the call ([`crate::Others::with_sources`]), keyed by the value as
//! the call gives it.
//!
//! Must not: resolve a source (the host's datasets and other documents do), or read the
//! document.

mod keys;
mod label;
mod pattern;
mod placed;

pub(crate) use keys::{loose_key, natural, Run};
pub(crate) use label::{label, what};
pub(crate) use pattern::{date_pattern, DatePattern};
pub(crate) use placed::{first_rows, fits, placed, runs, thousands, Placed};

use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use ooxml_word_model::docx::casefold;
use serde_json::{Map, Value};

/// A calendar day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day {
    /// The year.
    pub year: i32,
    /// The month, 1-12.
    pub month: u32,
    /// The day, 1-31.
    pub day: u32,
}

impl Day {
    /// The day, where it exists.
    #[must_use]
    pub fn new(year: i32, month: u32, day: u32) -> Option<Day> {
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
            2 => 28,
            _ => return None,
        };
        ((1..=9999).contains(&year) && (1..=days).contains(&day)).then_some(Day { year, month, day })
    }

    /// Monday 0 to Sunday 6 (Python's `date.weekday()`).
    #[must_use]
    pub fn weekday(self) -> usize {
        // days since 1970-01-01 (a Thursday), proleptic Gregorian
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0)
    }
}

/// A time of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// The hour, 0-23.
    pub hour: u32,
    /// The minute.
    pub minute: u32,
    /// The second.
    pub second: u32,
    /// The microsecond.
    pub micro: u32,
}

impl Clock {
    /// The time, where it exists.
    #[must_use]
    pub fn new(hour: u32, minute: u32, second: u32, micro: u32) -> Option<Clock> {
        (hour < 24 && minute < 60 && second < 60 && micro < 1_000_000).then_some(Clock {
            hour,
            minute,
            second,
            micro,
        })
    }
}

/// One value of a source, as the host hands it over.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceValue {
    /// A blank (`None`).
    Blank,
    /// Text.
    Text(String),
    /// A logical value.
    Bool(bool),
    /// A whole number, its digits as Python writes them.
    Int(String),
    /// A finite number with a fraction or an exponent.
    Float(f64),
    /// A date.
    Date(Day),
    /// A date and a time of day.
    DateTime(Day, Clock),
    /// A time of day.
    Time(Clock),
    /// Anything else, as a message names its kind (`a Decimal`, `not a finite number`).
    Other(String),
}

impl SourceValue {
    /// A value as the host writes one in JSON: `null`, a logical value, a number, text,
    /// `{"date": [y, m, d]}`, `{"datetime": [y, m, d, h, min, s, us]}`, `{"time": [h, min,
    /// s, us]}`, `{"int": "digits"}` (a whole number past 64 bits) or `{"other": kind}`.
    fn from_json(value: &Value) -> Option<SourceValue> {
        Some(match value {
            Value::Null => SourceValue::Blank,
            Value::Bool(b) => SourceValue::Bool(*b),
            Value::String(s) => SourceValue::Text(s.clone()),
            Value::Number(n) if n.is_i64() || n.is_u64() => SourceValue::Int(n.to_string()),
            Value::Number(n) => SourceValue::Float(n.as_f64()?),
            Value::Object(map) => {
                let (kind, given) = map.iter().next()?;
                let numbers = || -> Option<Vec<u32>> {
                    given
                        .as_array()?
                        .iter()
                        .map(|n| n.as_u64().and_then(|n| u32::try_from(n).ok()))
                        .collect()
                };
                match kind.as_str() {
                    "date" => match numbers()?.as_slice() {
                        [y, m, d] => SourceValue::Date(Day::new(i32::try_from(*y).ok()?, *m, *d)?),
                        _ => return None,
                    },
                    "datetime" => match numbers()?.as_slice() {
                        [y, m, d, h, min, s, us] => SourceValue::DateTime(
                            Day::new(i32::try_from(*y).ok()?, *m, *d)?,
                            Clock::new(*h, *min, *s, *us)?,
                        ),
                        _ => return None,
                    },
                    "time" => match numbers()?.as_slice() {
                        [h, min, s, us] => SourceValue::Time(Clock::new(*h, *min, *s, *us)?),
                        _ => return None,
                    },
                    "int" => SourceValue::Int(given.as_str()?.to_owned()),
                    "other" => SourceValue::Other(given.as_str()?.to_owned()),
                    _ => return None,
                }
            }
            Value::Array(_) => return None,
        })
    }

    /// Whether the value is a blank or empty text (`value is None or value == ""`).
    #[must_use]
    pub fn is_blank(&self) -> bool {
        match self {
            SourceValue::Blank => true,
            SourceValue::Text(t) => t.is_empty(),
            _ => false,
        }
    }
}

/// The rows a source named, as the host resolved them (ooxml-edit's `Rows`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceRows {
    /// The source as the call gives it (`t:stock`, `d:sales/#2:#5`): what the call is
    /// looked up by.
    pub given: String,
    /// The source as resolved (`d:sales/#2:#5`, `d2/t:stock`).
    pub address: String,
    /// The columns' names.
    pub columns: Vec<String>,
    /// The columns' type labels (`date`, `datetime`, `time`, ...), where the source has them.
    pub types: Option<Vec<Option<String>>>,
    /// The values, row by row.
    pub rows: Vec<Vec<SourceValue>>,
    /// Each value's own sources (a workbook's cells), where the source has them.
    pub sources: Option<Vec<Vec<Vec<String>>>>,
    /// The host's handle of the document the values came from (`d2`).
    pub doc: Option<String>,
    /// That document's name (`stock.xlsx`).
    pub name: Option<String>,
    /// Its version the values are of.
    pub version: Option<i64>,
    /// Whether it has changed since (a dataset), or the values came from invalidated caches.
    pub stale: bool,
    /// The formula caches the values were read from, counted by status.
    pub cache: Option<Map<String, Value>>,
}

impl SourceRows {
    /// One source's rows as a host hands them over in JSON: `{"given", "address",
    /// "columns", "types", "rows", "sources", "doc", "name", "version", "stale", "cache"}`,
    /// values as [`SourceValue`] reads them; `None` for anything else.
    pub(crate) fn from_json(value: &Value) -> Option<SourceRows> {
        let text = |name: &str| -> Option<Option<String>> {
            match value.get(name) {
                None | Some(Value::Null) => Some(None),
                Some(found) => Some(Some(found.as_str()?.to_owned())),
            }
        };
        let strings = |found: &Value| -> Option<Vec<String>> {
            found
                .as_array()?
                .iter()
                .map(|s| s.as_str().map(str::to_owned))
                .collect()
        };
        let columns = strings(value.get("columns")?)?;
        let rows = value
            .get("rows")?
            .as_array()?
            .iter()
            .map(|row| row.as_array()?.iter().map(SourceValue::from_json).collect())
            .collect::<Option<Vec<Vec<SourceValue>>>>()?;
        let types = match value.get("types") {
            None | Some(Value::Null) => None,
            Some(found) => Some(
                found
                    .as_array()?
                    .iter()
                    .map(|t| match t {
                        Value::Null => Some(None),
                        other => other.as_str().map(|s| Some(s.to_owned())),
                    })
                    .collect::<Option<Vec<_>>>()?,
            ),
        };
        let sources = match value.get("sources") {
            None | Some(Value::Null) => None,
            Some(found) => Some(
                found
                    .as_array()?
                    .iter()
                    .map(|row| row.as_array()?.iter().map(strings).collect())
                    .collect::<Option<Vec<Vec<Vec<String>>>>>()?,
            ),
        };
        Some(SourceRows {
            given: value.get("given")?.as_str()?.to_owned(),
            address: value.get("address")?.as_str()?.to_owned(),
            columns,
            types,
            rows,
            sources,
            doc: text("doc")?,
            name: text("name")?,
            version: value.get("version").and_then(Value::as_i64),
            stale: value.get("stale").and_then(Value::as_bool).unwrap_or(false),
            cache: value.get("cache").and_then(Value::as_object).cloned(),
        })
    }

    /// Where the values came from, for a result, never the values (`Rows.describe`).
    #[must_use]
    pub fn describe(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("source".into(), Value::String(self.address.clone()));
        out.insert("doc".into(), self.doc.clone().map_or(Value::Null, Value::String));
        out.insert("version".into(), self.version.map_or(Value::Null, Value::from));
        if self.stale {
            out.insert("stale".into(), Value::Bool(true));
        }
        if let Some(cache) = self.cache.as_ref().filter(|c| !c.is_empty()) {
            out.insert("cache".into(), Value::Object(cache.clone()));
        }
        out
    }

    /// The position of the column `name`: exactly, else the one equal case aside;
    /// `not_found` with the columns there are (`Rows.column`).
    ///
    /// # Errors
    /// No such column.
    pub fn column(&self, name: &str, field: &str, retry: &TryCall) -> Result<usize, ToolError> {
        if let Some(k) = self.columns.iter().position(|c| c == name) {
            return Ok(k);
        }
        let folded = casefold(name);
        let found: Vec<usize> = self
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| casefold(c) == folded)
            .map(|(k, _)| k)
            .collect();
        if let [one] = found.as_slice() {
            return Ok(*one);
        }
        Err(ToolError::refusal(
            Refusal::NotFound,
            format!("{} has no column {}", self.address, repr_str(name)),
            WayOut::names(self.columns.iter().cloned()).unwrap_or_else(|| WayOut::call(retry.clone())),
        )
        .at(field))
    }

    /// The column's type label, where it has one (`Rows.type`).
    #[must_use]
    pub fn type_of(&self, column: usize) -> Option<&str> {
        self.types.as_ref()?.get(column)?.as_deref()
    }

    /// One value named for a message: its sources (the first two), else its place in the
    /// source, row from 1 (`Rows.where`).
    #[must_use]
    pub fn where_is(&self, row: usize, column: usize) -> String {
        let name = repr_str(self.columns.get(column).map_or("", String::as_str));
        let found = self
            .sources
            .as_ref()
            .and_then(|s| s.get(row)?.get(column))
            .filter(|s| !s.is_empty());
        match found {
            Some(found) => {
                let mut shown = found.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
                if found.len() > 2 {
                    shown.push_str(&format!(" and {} more", found.len() - 2));
                }
                format!("{shown} (row {} of {}, column {name})", row + 1, self.address)
            }
            None => format!("row {} of {}, column {name}", row + 1, self.address),
        }
    }

    /// Where a column's values at `rows` (from 0, in the order placed) came from: the
    /// column, the rows (from 1, runs as `1-4`) and their sources, each once
    /// (`Rows.provenance`).
    #[must_use]
    pub fn provenance(&self, column: usize, rows: &[usize]) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert(
            "column".into(),
            Value::String(self.columns.get(column).cloned().unwrap_or_default()),
        );
        out.insert("rows".into(), Value::String(runs(rows)));
        if let Some(sources) = &self.sources {
            let mut seen = Vec::<String>::new();
            for row in rows {
                for source in sources.get(*row).and_then(|r| r.get(column)).into_iter().flatten() {
                    if !seen.contains(source) {
                        seen.push(source.clone());
                    }
                }
            }
            out.insert("sources".into(), Value::from(seen));
        }
        out
    }

    /// The value at `row`, `column`.
    #[must_use]
    pub fn value(&self, row: usize, column: usize) -> &SourceValue {
        self.rows
            .get(row)
            .and_then(|r| r.get(column))
            .unwrap_or(&SourceValue::Blank)
    }
}

/// The host's refusal of a source it resolved to no rows (an unknown dataset), which the
/// tool words with what its document says of the value (the docx format's `source_hint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefused {
    /// The source as the call gives it.
    pub given: String,
    /// The host's message.
    pub message: String,
    /// The host's options (the datasets there are).
    pub options: Vec<String>,
}

/// The sources a call names, as the host resolved them: their rows, or its refusals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sources {
    /// The rows of each source resolved.
    pub rows: Vec<SourceRows>,
    /// The sources refused.
    pub refused: Vec<SourceRefused>,
}

impl Sources {
    /// The sources a host hands over as JSON: a list, each as [`SourceRows::list_from_json`]
    /// reads one, or `{"given", "refused": {"message", "options"}}`; `None` for anything
    /// else.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Sources> {
        let mut out = Sources::default();
        for item in value.as_array()? {
            match item.get("refused") {
                Some(refused) => out.refused.push(SourceRefused {
                    given: item.get("given")?.as_str()?.to_owned(),
                    message: refused.get("message")?.as_str()?.to_owned(),
                    options: refused
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|o| o.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                        .unwrap_or_default(),
                }),
                None => out.rows.push(SourceRows::from_json(item)?),
            }
        }
        Some(out)
    }
}

/// The refusal of a source the host refused, as the dispatcher words it: with the hint of a
/// drawing's id given for a dataset (the docx format's `source_hint`).
pub(crate) fn refused_source(
    refused: &SourceRefused,
    docx: Option<&ooxml_word_model::docx::Docx>,
    retry: &TryCall,
) -> ToolError {
    let given = &refused.given;
    let drawing_like = given.strip_prefix("d:").is_some_and(|rest| {
        let (number, member) = rest.split_once('#').map_or((rest, None), |(n, m)| (n, Some(m)));
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit());
        digits(number) && member.is_none_or(digits)
    });
    let mut message = refused.message.clone();
    if drawing_like {
        let here = docx.is_some_and(|d| {
            ooxml_word_model::docx::drawings::all_drawings(d)
                .iter()
                .any(|x| x.id == *given)
        });
        let what = if here {
            "is a drawing in this document"
        } else {
            "is a drawing's id (none here)"
        };
        message.push_str(&format!(
            "; {given} {what}, not a dataset: a source is a dataset d:<id>, its id starting with a letter, or a table \
             t:<name>"
        ));
    }
    ToolError::refusal(
        Refusal::NotFound,
        message,
        WayOut::names(refused.options.iter().cloned()).unwrap_or_else(|| WayOut::call(retry.clone())),
    )
    .at("from")
}

/// The refusal of a source the host handed no rows for: a dataset as ooxml-edit's
/// session refuses an unknown one, anything else as a source naming nothing.
pub(crate) fn no_rows(given: &str, field: &str, retry: &TryCall) -> ToolError {
    let message = if given.starts_with("d:") {
        let id = given.trim_start_matches("d:");
        let id = id.split(['/', '[']).next().unwrap_or(id);
        format!("no dataset d:{id}")
    } else {
        format!("no rows for {}", repr_str(given))
    };
    ToolError::refusal(Refusal::NotFound, message, WayOut::call(retry.clone())).at(field)
}

#[cfg(test)]
mod tests;
