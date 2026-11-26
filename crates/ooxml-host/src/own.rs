//! Owns: the host's own tools' definitions (ooxml-edit's `SESSION_TOOLS` and `new_document`,
//! their schemas as today's, so one acceptance takes their calls), and the seam the host
//! runs the session tools through ([`SessionTools`]): `open_document`, `new_document`,
//! `close_document` and `read_blob` are run by an implementation the host is made with
//! ([`crate::HostTools`]); `batch` the host runs itself (`crate::batch`). [`NotYet`] refuses
//! a tool with the way out until its port.
//!
//! Must not: run a format's tool, or change the session's stores but through the
//! [`HostSession`] the seam is handed.

use crate::catalogue::Own;
use crate::session::HostSession;
use ooxml_tools::{ArgEnum, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use serde_json::{Map, Value};

/// The kinds a new document may be.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    /// A deck.
    Pptx,
    /// A document.
    Docx,
    /// A workbook.
    Xlsx,
}

/// `open_document`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "open_document",
    description = "Open an input blob as a deck or document. Returns its doc id and a summary; describe it next.",
    documents(),
    batchable = false,
    role = "any"
)]
pub struct OpenDocument {
    /// Blob handle, e.g. b1.
    pub blob: String,
}

/// `new_document`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "new_document",
    description = "Make a new deck or document, optionally from a template blob (its layouts, styles and theme, not its content). Returns its doc id.",
    documents(),
    route = "kind",
    batchable = false,
    role = "edit"
)]
pub struct NewDocument {
    /// pptx: a deck; docx: a document; xlsx: a workbook.
    pub kind: NewKind,
    /// Template blob handle (.potx, .dotx...).
    pub template_blob: Option<String>,
    /// Decks 16:9 (default) or 4:3; documents A4, Letter...
    pub size: Option<String>,
    /// Title property.
    pub title: Option<String>,
    /// Author property.
    pub author: Option<String>,
    /// Name, e.g. plan.pptx. Default new.<kind>.
    pub name: Option<String>,
}

/// `close_document`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "close_document",
    description = "Close a document, discarding unsaved changes. The session holds a few documents at a time: close a source once you have taken what you need.",
    group = "shared_misc",
    batchable = false,
    role = "any"
)]
pub struct CloseDocument {
    /// Document id.
    pub doc: String,
}

/// `read_blob`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "read_blob",
    description = "Read a text input blob (CSV, Markdown, plain text, JSON) a page at a time. Open documents with open_document; place images with a picture tool.",
    group = "shared_misc",
    documents(),
    batchable = false,
    role = "any"
)]
pub struct ReadBlob {
    /// Blob handle, e.g. b2.
    pub blob: String,
    /// next_cursor of the previous page.
    pub cursor: Option<String>,
}

/// One op of a `batch`.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct BatchOp {
    // No description: today's definition gives the op's tool none.
    pub tool: String,
    /// That tool's arguments.
    pub arguments: Map<String, Value>,
}

/// `batch`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "batch",
    description = "Run calls to several tools in order as one undo step, checks once at the end; a failing op is skipped with its error. Not for render, save_document or session tools.",
    documents(),
    mutates,
    strict = false,
    batchable = false,
    role = "any"
)]
pub struct Batch {
    /// The calls in order, at most 200; later ops may use earlier refs.
    #[arg(min_items = 1)]
    pub ops: Vec<BatchOp>,
    /// All or none.
    pub atomic: Option<bool>,
}

/// One call of a session tool, as the host hands it to the seam: its arguments taken as
/// meant, the session, and the call that would be made again.
pub struct OwnCall<'a> {
    /// The tool.
    pub own: Own,
    /// Its arguments, taken as meant.
    pub arguments: &'a Map<String, Value>,
    /// The session the call runs in.
    pub session: &'a HostSession,
    /// The caller's context, where the call has one.
    pub caller: Option<&'a Map<String, Value>>,
    /// The call as given, for a refusal's way out.
    pub retry: &'a TryCall,
}

/// What runs the session tools the host does not run itself.
pub trait SessionTools: Send + Sync {
    /// Run `call`. Never fails: a refusal is the result's error.
    fn run(&self, call: &OwnCall<'_>) -> ToolResult<Value>;
}

/// A session tool before its port: refused as not supported yet, with
/// the call that would work in the Python host.
#[derive(Debug, Clone, Copy, Default)]
pub struct NotYet;

impl SessionTools for NotYet {
    fn run(&self, call: &OwnCall<'_>) -> ToolResult<Value> {
        let name = &call.retry.tool;
        ToolResult::failed(
            ToolError::refusal(
                Refusal::Refused,
                format!("{name} is not supported in the Rust host yet; the application runs it"),
                WayOut::call(call.retry.clone()),
            )
            .with_detail("not_supported", Value::from(format!("{name} (Rust host)"))),
        )
    }
}

/// The specs of the host's own tools the seam runs, each with its kind.
#[must_use]
pub fn specs() -> Vec<(Own, ooxml_tools::ToolSpec)> {
    vec![
        (Own::NewDocument, NewDocument::spec()),
        (Own::OpenDocument, OpenDocument::spec()),
        (Own::CloseDocument, CloseDocument::spec()),
        (Own::Batch, Batch::spec()),
        (Own::ReadBlob, ReadBlob::spec()),
    ]
}
