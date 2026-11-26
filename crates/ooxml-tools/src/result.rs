//! What a tool call returns: [`ToolResult`], the envelope the model reads, serialised
//! within the reply budget (tool design rule 4), and the fields only the application reads.
//!
//! Owns: the envelope's JSON key for key as today's `results.py` writes it (`Result.to_json`)
//! and its text within a budget (`Result.to_text`): what did not fit is said, never cut
//! silently, with the call that would ask for it where the tool gave one
//! ([`Done::ask`]); a no-op as a success with a note, never an error (rule 3); and the
//! application-only fields ([`AppOnly`]: `touched`, timing, memory, budget, the files the
//! call gives the application), never in the model's JSON.
//!
//! Must not: know providers (the adapters turn the text and images into content blocks)
//! or describe images (their descriptors come from the renderer, as opaque JSON).

use crate::error::{ToolError, TryCall};
use serde_json::{Map, Value};
use std::fmt::Write as _;

/// The result's data as JSON.
pub trait ReplyData {
    /// The data as the model reads it.
    fn to_json(&self) -> Value;
}

impl ReplyData for Value {
    fn to_json(&self) -> Value {
        self.clone()
    }
}

/// What a successful call did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Done<T> {
    /// One line on what happened.
    pub summary: String,
    /// Ids changed.
    pub changed: Vec<String>,
    /// Ids created.
    pub created: Vec<String>,
    /// Ids removed.
    pub removed: Vec<String>,
    /// Ids renamed: `(old, new)`.
    pub renamed: Vec<(String, String)>,
    /// Refs this call defined: `(name, address)`.
    pub refs: Vec<(String, String)>,
    /// Datasets this call defined.
    pub datasets: Vec<String>,
    /// The checks after a change.
    pub checks: Map<String, Value>,
    /// The data.
    pub data: Option<T>,
    /// The next page's cursor.
    pub next_cursor: Option<String>,
    /// The total, where the data is a page of it.
    pub total: Option<u64>,
    /// The images' descriptors (the images travel beside the JSON).
    pub images: Vec<Value>,
    /// The call that asks for what did not fit in the budget (the next page, a narrower
    /// range); without one, the reply says to ask for less.
    pub ask: Option<TryCall>,
    /// The files the call gives the application (`save_document`'s package): never in the
    /// model's JSON; [`ToolResult::done`] moves them into [`AppOnly::files`].
    pub files: Vec<File>,
    /// What the model is warned of (an argument read otherwise than given);
    /// [`ToolResult::done`] moves them into [`ToolResult::warnings`].
    pub warnings: Vec<String>,
}

/// A file a call gives the application, its bytes beside the reply (tool design rule 6:
/// the model is told its name, format and size, never its content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Its name, as the call gives it (`q3-review.docx`).
    pub name: String,
    /// Its format, as the call names it (`docx`, `markdown`).
    pub format: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

impl<T> Done<T> {
    /// A result with its summary, nothing else yet.
    #[must_use]
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            changed: Vec::new(),
            created: Vec::new(),
            removed: Vec::new(),
            renamed: Vec::new(),
            refs: Vec::new(),
            datasets: Vec::new(),
            checks: Map::new(),
            data: None,
            next_cursor: None,
            total: None,
            images: Vec::new(),
            ask: None,
            files: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// Where a call's time went, in seconds, for the application.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Timing {
    /// The whole call.
    pub total: f64,
    /// Waiting for its documents.
    pub lock_wait: f64,
    /// The rest.
    pub work: f64,
    /// The thread's own CPU, where the platform's clock gives it (`None`: not measured).
    pub cpu: Option<f64>,
    /// How long it held its documents' locks.
    pub held: f64,
}

/// Fields only the application reads; never in the model's JSON.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppOnly {
    /// What the call changed, one entry per thing, in the format's own addresses.
    pub touched: Vec<Value>,
    /// The zip parts the call changed, `{"doc", "part"}` each, by document and then name:
    /// what a co-authoring client uploads after the call.
    pub changed_parts: Vec<Value>,
    /// A batch's `touched` per op: `(op index, touched)`.
    pub touched_by_op: Vec<(usize, Vec<Value>)>,
    /// The tool's name.
    pub tool: String,
    /// Where the time went.
    pub timing: Option<Timing>,
    /// What the process holds, in MB, where a handler measures it.
    pub memory: Option<Map<String, Value>>,
    /// The most characters the result may take when sent, where that is more than the
    /// sender's limit.
    pub budget: Option<usize>,
    /// Whether the call found what it asked already true (rule 3).
    pub unchanged: bool,
    /// The files the call gives the application ([`Done::files`]).
    pub files: Vec<File>,
    /// What a call that panicked said (its reply is an `internal` failure), for the
    /// application's log; `None` for every call that did not.
    pub panic: Option<String>,
}

/// How the call went.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome<T> {
    /// It did what was asked (or found it already true).
    Done(Box<Done<T>>),
    /// It did not.
    Failed(ToolError),
}

/// The envelope every call returns.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult<T = Value> {
    /// The document the call was about.
    pub doc: Option<String>,
    /// Its version after the call.
    pub version: Option<u64>,
    /// How it went.
    pub outcome: Outcome<T>,
    /// Notes for the model: what was read as what, what was left as it was.
    pub warnings: Vec<String>,
    /// For the application only.
    pub app: AppOnly,
}

/// The reply budget's default, as `Limits.max_result_chars`.
pub const MAX_RESULT_CHARS: usize = 24_000;

impl<T: ReplyData> ToolResult<T> {
    /// A success.
    #[must_use]
    pub fn done(mut done: Done<T>) -> Self {
        let files = std::mem::take(&mut done.files);
        let warnings = std::mem::take(&mut done.warnings);
        Self {
            doc: None,
            version: None,
            outcome: Outcome::Done(Box::new(done)),
            warnings,
            app: AppOnly {
                files,
                ..AppOnly::default()
            },
        }
    }

    /// A failure.
    #[must_use]
    pub fn failed(error: ToolError) -> Self {
        Self {
            doc: None,
            version: None,
            outcome: Outcome::Failed(error),
            warnings: Vec::new(),
            app: AppOnly::default(),
        }
    }

    /// Something already true: a success that changed nothing, with a note saying why
    /// (rule 3). It never fails a batch.
    #[must_use]
    pub fn no_op(summary: impl Into<String>, note: impl Into<String>) -> Self {
        let mut result = Self::done(Done::new(summary));
        result.warnings.push(note.into());
        result.app.unchanged = true;
        result
    }

    /// A call whose list of work is empty: nothing changes, and a warning says so
    /// (`done`: what was not done, `"added"`).
    #[must_use]
    pub fn nothing_given(done: &str, field: &str) -> Self {
        Self::no_op(
            format!("Nothing {done}: {field} is empty"),
            format!("{field} is empty: nothing was {done}"),
        )
    }

    /// Whether the call failed.
    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self.outcome, Outcome::Failed(_))
    }

    /// The envelope as the model reads it.
    #[must_use]
    pub fn to_json(&self) -> Map<String, Value> {
        let mut out = Map::new();
        let strings = |items: &[String]| Value::Array(items.iter().map(|s| Value::from(s.as_str())).collect());
        let pairs = |items: &[(String, String)]| -> Value {
            Value::Object(
                items
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
                    .collect(),
            )
        };
        out.insert("ok".into(), Value::Bool(!self.is_error()));
        out.insert("doc".into(), self.doc.as_deref().map_or(Value::Null, Value::from));
        out.insert("version".into(), self.version.map_or(Value::Null, Value::from));
        let done = match &self.outcome {
            Outcome::Failed(error) => {
                out.insert("error".into(), error.to_json());
                if !self.warnings.is_empty() {
                    out.insert("warnings".into(), strings(&self.warnings));
                }
                return out;
            }
            Outcome::Done(done) => done,
        };
        out.insert("summary".into(), Value::from(done.summary.as_str()));
        out.insert("changed".into(), strings(&done.changed));
        out.insert("created".into(), strings(&done.created));
        out.insert("removed".into(), strings(&done.removed));
        out.insert("renamed".into(), pairs(&done.renamed));
        out.insert("warnings".into(), strings(&self.warnings));
        out.insert("checks".into(), Value::Object(done.checks.clone()));
        out.insert(
            "data".into(),
            done.data.as_ref().map_or(Value::Null, ReplyData::to_json),
        );
        out.insert(
            "next_cursor".into(),
            done.next_cursor.as_deref().map_or(Value::Null, Value::from),
        );
        if !done.refs.is_empty() {
            out.insert("refs".into(), pairs(&done.refs));
        }
        if !done.datasets.is_empty() {
            out.insert("datasets".into(), strings(&done.datasets));
        }
        if let Some(total) = done.total {
            out.insert("total".into(), Value::from(total));
        }
        if !done.images.is_empty() {
            out.insert("images".into(), Value::Array(done.images.clone()));
        }
        out
    }

    /// The envelope as compact JSON within `limit` characters (or the result's own
    /// [`AppOnly::budget`] where that is more). What does not fit is said: first the data
    /// goes, with `truncated` saying how long it was and how to ask for it (`try_call`, the
    /// tool's [`Done::ask`], where it gave one); then the other lists become their counts;
    /// at last only the call's status and `truncated` stay.
    #[must_use]
    pub fn to_text(&self, limit: Option<usize>) -> String {
        let limit = match (limit, self.app.budget) {
            (Some(limit), Some(budget)) => Some(limit.max(budget)),
            (limit, _) => limit,
        };
        let envelope = self.to_json();
        let text = compact(&Value::Object(envelope.clone()));
        let length = text.chars().count();
        let Some(limit) = limit.filter(|limit| length > *limit) else {
            return text;
        };
        let mut envelope = envelope;
        envelope.insert("data".into(), Value::Null);
        let truncated =
            format!("the result was {length} characters, over the {limit} limit; ask for less (a range or a page)");
        envelope.insert("truncated".into(), Value::from(truncated.as_str()));
        if let Outcome::Done(done) = &self.outcome {
            if let Some(ask) = &done.ask {
                envelope.insert("try_call".into(), ask.to_json());
            }
        }
        let mut text = compact(&Value::Object(envelope.clone()));
        if text.chars().count() > limit {
            for key in [
                "checks", "warnings", "changed", "created", "removed", "renamed", "refs", "datasets",
            ] {
                let count = match envelope.get(key) {
                    Some(Value::Array(items)) if !items.is_empty() => items.len(),
                    Some(Value::Object(map)) if !map.is_empty() => map.len(),
                    _ => continue,
                };
                let mut omitted = Map::new();
                omitted.insert("omitted".into(), Value::from(count));
                envelope.insert(key.into(), Value::Object(omitted));
            }
            text = compact(&Value::Object(envelope));
        }
        if text.chars().count() <= limit {
            return text;
        }
        let mut last = Map::new();
        last.insert("ok".into(), Value::Bool(!self.is_error()));
        last.insert("doc".into(), self.doc.as_deref().map_or(Value::Null, Value::from));
        last.insert("version".into(), self.version.map_or(Value::Null, Value::from));
        last.insert("truncated".into(), Value::from(truncated));
        python_dumps(&Value::Object(last))
    }
}

/// Compact JSON, non-ASCII kept, as `json.dumps(..., separators=(",", ":"),
/// ensure_ascii=False)` writes it.
#[must_use]
pub fn compact(value: &Value) -> String {
    value.to_string()
}

/// JSON as `json.dumps(value)` writes it by default: `", "` and `": "` between, non-ASCII
/// escaped.
#[must_use]
pub fn python_dumps(value: &Value) -> String {
    let mut out = String::new();
    dump(value, &mut out);
    out
}

fn dump(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => dump_str(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                dump(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                dump_str(key, out);
                out.push_str(": ");
                dump(item, out);
            }
            out.push('}');
        }
        Value::Number(n) => out.push_str(&crate::pyfmt::number(n)),
        other => out.push_str(&other.to_string()),
    }
}

fn dump_str(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    // Writing to a String cannot fail.
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}
