//! `describe`: the document at a glance (today's `word_describe`).
//!
//! Owns: the tool's definition (one schema for every kind, as the shared `describe` has),
//! and its data keyed and ordered as docx-agent's reply is: the fields the package parts
//! give ([`package`]) and those the Word model's reading gives ([`document`]).
//!
//! Must not: lay out (`pages`, the Rust layout's switch, phase PL step L4), validate
//! (`problems`, phase P2), or read state the session does not hold: those fields are
//! [`NOT_DESCRIBED`].

mod document;
pub(crate) use document::sections_json;
mod package;

pub(crate) use document::short;

use crate::version::Version;
use ooxml_base::{BaseError, Cancel};
use ooxml_opc::OpcError;
use ooxml_tools::{Done, Failure, ToolArgs, ToolError};
use ooxml_word_model::parts::{main_part, story_parts};
use package::Reader;
use serde_json::{Map, Value};

/// `describe`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "describe", description = include_str!("../describe.md"))]
pub struct Describe {
    /// Document id.
    pub doc: String,
}

/// The keys of today's data that this handler gives, in their order there (`tagged` and
/// `headings_note` only where today's has them).
pub const DESCRIBED: &[&str] = &[
    "name",
    "compatibility_mode",
    "properties",
    "tracking",
    "headings",
    "sections",
    "styles_in_use",
    "comments",
    "revisions",
    "fields",
    "tables",
    "drawings",
    "notes",
    "controls",
    "tagged",
    "headings_note",
];

/// The keys of today's data that it does not give yet, and why (the hand-back's list).
pub const NOT_DESCRIBED: &[(&str, &str)] = &[
    ("pages", "the Word layout (phase PL, step L4)"),
    ("problems", "validation on open (phase P2)"),
];

/// The compatibility mode of a document whose settings name none: Word 2007's.
const DEFAULT_COMPATIBILITY_MODE: i64 = 12;

/// The core properties `describe` shows, in the order docx-agent reads them.
const PROPERTIES: [(&str, &str, &str); 3] = [
    ("title", DC, "title"),
    ("subject", DC, "subject"),
    ("author", DC, "creator"),
];
const DC: &str = "http://purl.org/dc/elements/1.1/";

/// `describe` of a Word document `name` at one version of its package. Cancellation is
/// checked once per part read and between the model's fields.
///
/// # Errors
/// `timeout` when `cancel` asks to stop; `internal` when a part it reads is not XML or the
/// package has no main part.
pub fn describe(_args: &Describe, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    let (package, name) = (version.package, version.name);
    let reader = Reader { package, cancel };
    let main = main_part(package)
        .map_err(opc)?
        .ok_or_else(|| ToolError::internal("not a Word package: no main document part", "describe:main"))?;
    let docx = version.docx()?;
    let read = document::Reading::new(&docx);
    let mut data = Map::new();
    data.insert("name".into(), Value::from(name));
    data.insert(
        "compatibility_mode".into(),
        Value::from(reader.compatibility_mode(&main)?),
    );
    data.insert("properties".into(), Value::Object(reader.properties()?));
    // The Rust session keeps no tracking state yet: no session tracks changes (phase P2).
    data.insert("tracking".into(), Value::Null);
    cancel.check().map_err(cancelled)?;
    let headings = read.headings();
    let full = headings.len() >= 80;
    data.insert("headings".into(), Value::Array(headings.into_iter().take(80).collect()));
    data.insert("sections".into(), read.sections());
    data.insert("styles_in_use".into(), read.styles_in_use());
    data.insert("comments".into(), reader.comments(&main)?);
    cancel.check().map_err(cancelled)?;
    data.insert("revisions".into(), read.revisions());
    data.insert("fields".into(), read.fields());
    data.insert("tables".into(), read.tables());
    data.insert("drawings".into(), read.drawings());
    let mut parts = vec![main.clone()];
    parts.extend(story_parts(package, &main).map_err(opc)?);
    let (notes, controls) = reader.notes_and_controls(&parts)?;
    data.insert("notes".into(), Value::Object(notes));
    data.insert("controls".into(), Value::from(controls));
    if let Some(tagged) = read.tagged() {
        data.insert("tagged".into(), tagged);
    }
    if full {
        data.insert(
            "headings_note".into(),
            Value::from("the first 80 headings; word_read a range for the rest"),
        );
    }
    let mut done = Done::new(format!("Described {name}"));
    done.data = Some(Value::Object(data));
    Ok(done)
}

/// A whole number as Python's `int(text)` reads the forms Word writes (spaces around, a
/// sign); `None` where it raises.
pub(super) fn python_int(text: &str) -> Option<i64> {
    let text = text.trim();
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

pub(super) fn opc(error: OpcError) -> ToolError {
    ToolError::internal(error.to_string(), "describe:package")
}

pub(super) fn cancelled(error: BaseError) -> ToolError {
    match error {
        BaseError::Cancelled => ToolError::failure(
            Failure::Timeout,
            "describe stopped before it finished (the call was cancelled or ran past its deadline); nothing changed",
        ),
        other => ToolError::internal(other.to_string(), "describe:cancel"),
    }
}

#[cfg(test)]
mod tests {
    use super::python_int;

    #[test]
    fn whole_numbers_read_as_python_reads_them() {
        assert_eq!(python_int(" 15 "), Some(15));
        assert_eq!(python_int("+14"), Some(14));
        assert_eq!(python_int("-1"), Some(-1));
        assert_eq!(python_int(""), None);
        assert_eq!(python_int("15.0"), None);
        assert_eq!(python_int("x"), None);
    }
}
