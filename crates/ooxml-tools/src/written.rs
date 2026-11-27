//! Owns: the values a host wrote into a call's arguments for references (`{{d:...}}`) before
//! the call ran ([`Written`]: ooxml-edit's `textfacts.Fact`, the host's `call.facts`), and
//! what a host hands a call of the values it resolved for it ([`CallValues`]: those values,
//! the rows of the sources the call names, or the host's refusals of them, and where a
//! source naming one value finds it: [`OneValue`]; and, through the values channel, what the
//! application resolved of the documents its own host holds: [`HeldValues`]).
//!
//! Must not: resolve a reference or a source (the host does), or keep a value in a document
//! (the format does).

use crate::datasets::Dataset;
use crate::sources::{SourceRows, Sources};
use crate::{ErrorCode, ToolError, WayOut};
use serde_json::{Map, Value};

/// One value a host wrote for a reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    /// The argument it went into (`text`, `items[0].text`, `replace`).
    pub field: String,
    /// Its first character in the argument's text.
    pub start: usize,
    /// The character after its last.
    pub end: usize,
    /// The value written.
    pub text: String,
    /// What the document keeps of it (an `ooxml_edit.provenance` fact's fields).
    pub record: Map<String, Value>,
    /// What a reply says of it (`Fact.brief()`: the reference, the value, its sources).
    pub brief: Map<String, Value>,
    /// The value's own Markdown, where its source holds one apart from its text (a workbook
    /// cell's emphasis and list lines: docx-agent's `field_text.reference_markdown`, read by
    /// the host); `word_insert_markdown` writes it as that.
    pub markdown: Option<String>,
}

impl Written {
    /// The values a host hands over as JSON: a list of `{"field", "start", "end", "text",
    /// "record", "brief"}` (and the value's own `"markdown"`, where it has one); `None` for
    /// anything else.
    #[must_use]
    pub fn list_from_json(value: &Value) -> Option<Vec<Written>> {
        value.as_array()?.iter().map(Written::from_json).collect()
    }

    fn from_json(value: &Value) -> Option<Written> {
        let index = |name: &str| value.get(name)?.as_u64().and_then(|n| usize::try_from(n).ok());
        Some(Written {
            field: value.get("field")?.as_str()?.to_owned(),
            start: index("start")?,
            end: index("end")?,
            text: value.get("text")?.as_str()?.to_owned(),
            record: value.get("record")?.as_object()?.clone(),
            brief: value.get("brief")?.as_object()?.clone(),
            markdown: match value.get("markdown") {
                None | Some(Value::Null) => None,
                Some(found) => Some(found.as_str()?.to_owned()),
            },
        })
    }

    /// The item index of a field `items[k].text`; `None` for another field.
    #[must_use]
    pub fn item(&self) -> Option<usize> {
        self.field.strip_prefix("items[")?.strip_suffix("].text")?.parse().ok()
    }

    /// Whether it is all of `argument` but white space around it.
    #[must_use]
    pub fn is_whole(&self, argument: &str) -> bool {
        let chars: Vec<char> = argument.chars().collect();
        let blank = |part: Option<&[char]>| part.is_some_and(|p| p.iter().all(|c| py_space(*c)));
        blank(chars.get(..self.start)) && blank(chars.get(self.end..))
    }
}

/// Python's `str.isspace` for one character: Unicode white space and the four information
/// separators.
#[must_use]
pub fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// What a host hands a call of the values it resolved for it before the call ran.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallValues {
    /// The values written into its arguments for references, each where it went.
    pub written: Vec<Written>,
    /// The rows of the sources it names, keyed by the value as the call gives it, or the
    /// host's refusals of them.
    pub sources: Sources,
    /// Each source naming one value (`d:notes/#12/body`), as the host read it: the rows it
    /// reads and where the value is in them (its text and record are in [`Self::written`],
    /// its refusal in [`Sources::refused`], under the same field).
    pub one_values: Vec<OneValue>,
    /// The tables of the other open documents, each as a `from` names it, where a call
    /// reads sources with another document open (a refusal names the one meant).
    pub tables: Vec<HeldTable>,
    /// What the application resolved for the call of each document its own host holds
    /// (design phase 3, the values channel; Phase 4 deletes it).
    pub held: Vec<HeldValues>,
    /// What the call asked of those documents that was not in [`Self::held`].
    pub missed: Missed,
}

/// What the application resolved for one call of a document its own host holds: the rows
/// each address the call may read gives, or the refusal of it, and the tables the document
/// lists (its format's `value_sources`; what a check of numbers reads and a refusal offers).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeldValues {
    /// The document (`d2`).
    pub doc: String,
    /// Each address resolved (`t:Sales`), and its rows or its refusal.
    pub answers: Vec<(String, Result<Dataset, ToolError>)>,
    /// The tables it lists, `None` where its format lists none.
    pub value_sources: Option<Vec<String>>,
}

/// What a call asked of the documents the application's own host holds that was not
/// resolved for it: `(doc, address)`, in the order asked, each once; every copy shares them
/// (the application resolves them and makes the call again). The default notes nothing: a
/// call of a session holding no such document ([`Missed::noting`] notes).
#[derive(Debug, Clone, Default)]
pub struct Missed(Option<Noted>);

/// What a [`Missed`] noted, shared by its copies.
type Noted = std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>;

impl Missed {
    /// One that notes what the call misses.
    #[must_use]
    pub fn noting() -> Missed {
        Missed(Some(std::sync::Arc::default()))
    }

    /// The call asked for `address` of `doc`, not resolved.
    pub fn note(&self, doc: &str, address: &str) {
        let Some(found) = &self.0 else { return };
        let mut found = found.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !found.iter().any(|(d, a)| d == doc && a == address) {
            found.push((doc.to_owned(), address.to_owned()));
        }
    }

    /// What was noted so far.
    #[must_use]
    pub fn noted(&self) -> Vec<(String, String)> {
        self.0
            .as_ref()
            .map(|found| found.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone())
            .unwrap_or_default()
    }
}

impl PartialEq for Missed {
    /// (What a call missed is no part of what it was handed.)
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl HeldValues {
    /// The values of each held document, read from JSON: a list of `{"doc", "answers":
    /// [{"address", "rows": <a dataset, Dataset::from_json>} | {"address", "refused":
    /// {"code", "message", "options", "details"}}], "value_sources": [...] | null}`.
    ///
    /// # Errors
    /// A field of the wrong shape, with why.
    pub fn list_from_json(value: &Value) -> Result<Vec<HeldValues>, String> {
        value
            .as_array()
            .ok_or("held values are a JSON list")?
            .iter()
            .map(HeldValues::from_json)
            .collect()
    }

    fn from_json(value: &Value) -> Result<HeldValues, String> {
        let doc = value
            .get("doc")
            .and_then(Value::as_str)
            .ok_or("held values name their document (doc)")?;
        let answers = value
            .get("answers")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(answer_of).collect::<Result<Vec<_>, String>>())
            .transpose()?
            .unwrap_or_default();
        let value_sources = match value.get("value_sources") {
            None | Some(Value::Null) => None,
            Some(Value::Array(items)) => Some(
                items
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned).ok_or("value_sources are text"))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Some(_) => return Err("value_sources is a list".into()),
        };
        Ok(HeldValues {
            doc: doc.to_owned(),
            answers,
            value_sources,
        })
    }
}

/// One answer of [`HeldValues::list_from_json`].
fn answer_of(value: &Value) -> Result<(String, Result<Dataset, ToolError>), String> {
    let address = value
        .get("address")
        .and_then(Value::as_str)
        .ok_or("an answer names its address")?
        .to_owned();
    if let Some(rows) = value.get("rows") {
        let dataset = Dataset::from_json(rows).map_err(|e| format!("{address}: {}", e.0))?;
        return Ok((address, Ok(dataset)));
    }
    let refused = value.get("refused").ok_or("an answer has rows or refused")?;
    let named = refused
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("invalid_arguments");
    let code = ErrorCode::ALL
        .into_iter()
        .find(|c| c.as_str() == named)
        .ok_or_else(|| format!("{named:?} is no error code"))?;
    let message = refused
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let options = refused
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut error = match code {
        ErrorCode::Refusal(refusal) => ToolError::refusal(
            refusal,
            message,
            WayOut::options(options).unwrap_or_else(|| WayOut::one(Value::Null)),
        ),
        ErrorCode::Failure(failure) => {
            let error = ToolError::failure(failure, message);
            match WayOut::options(options) {
                Some(way_out) => error.with_way_out(way_out),
                None => error,
            }
        }
    };
    if let Some(details) = refused.get("details").and_then(Value::as_object) {
        for (key, value) in details {
            error = error.with_detail(key, value.clone());
        }
    }
    Ok((address, Err(error)))
}

/// A table of another open document, as a `from` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldTable {
    /// The table as a `from` names it (`t:stock`, `d2/t:stock`).
    pub name: String,
    /// Its document's handle (`d2`).
    pub doc: String,
    /// Its document's kind (`xlsx`).
    pub kind: String,
}

impl CallValues {
    /// These with `other`'s added (the application's beside the host's); what the call
    /// missed noted where `other` notes it.
    pub fn merge(&mut self, other: CallValues) {
        self.written.extend(other.written);
        self.sources.rows.extend(other.sources.rows);
        self.sources.refused.extend(other.sources.refused);
        self.one_values.extend(other.one_values);
        self.tables.extend(other.tables);
        self.held.extend(other.held);
        // (the application's notes what the call missed: what it hands, the call's)
        self.missed = other.missed;
    }
}

/// One value a declared source names, as the host read it (docx-agent's `_rows_and_cell`).
#[derive(Debug, Clone, PartialEq)]
pub struct OneValue {
    /// The source as the call gives it (qualified with its document).
    pub given: String,
    /// The argument it is in (`from`, `cells[2].from`).
    pub field: String,
    /// The address as a message shows it: without its document handle or format
    /// (`d:notes/#12/body`).
    pub shown: String,
    /// The rows it reads.
    pub rows: SourceRows,
    /// The value's row in them.
    pub row: usize,
    /// Its column.
    pub column: usize,
    /// The value's own Markdown, where its source holds one apart from its text.
    pub markdown: Option<String>,
}
