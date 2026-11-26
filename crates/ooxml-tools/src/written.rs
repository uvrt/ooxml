//! Owns: the values a host wrote into a call's arguments for references (`{{d:...}}`) before
//! the call ran ([`Written`]: ooxml-edit's `textfacts.Fact`, the host's `call.facts`), and
//! what a host hands a call of the values it resolved for it ([`CallValues`]: those values,
//! the rows of the sources the call names, or the host's refusals of them, and where a
//! source naming one value finds it: [`OneValue`]).
//!
//! Must not: resolve a reference or a source (the host does), or keep a value in a document
//! (the format does).

use crate::sources::{SourceRows, Sources};
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
    /// These with `other`'s added (the application's beside the host's).
    pub fn merge(&mut self, other: CallValues) {
        self.written.extend(other.written);
        self.sources.rows.extend(other.sources.rows);
        self.sources.refused.extend(other.sources.refused);
        self.one_values.extend(other.one_values);
        self.tables.extend(other.tables);
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
