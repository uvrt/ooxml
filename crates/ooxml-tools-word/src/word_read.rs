//! `word_read`: the document as Markdown with an id comment before each block, paged
//! (today's `word_read`, `docx_agent/tools/read.py`).
//!
//! Owns: the tool's definition, its paging (a page breaks at a line end in its second
//! half; the cursor is `c<offset>`), and the refusals of its arguments.
//!
//! Must not: write Markdown itself (`ooxml_word_model::markdown` does) or cache between
//! calls.

use crate::describe::cancelled;
use ooxml_base::Cancel;
use ooxml_opc::Package;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolArgs, ToolError, TryCall, WayOut};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::markdown::{to_markdown, RangeError, Stories, ViewName};
use serde_json::{json, Value};

/// Which view of the revisions to read.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Changes accepted.
    Final,
    /// Changes, comments and fields in place.
    Markup,
    /// Changes rejected.
    Original,
}

/// Which stories to read.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Story {
    /// The body.
    Body,
    /// The body, then headers, footers and notes.
    All,
}

/// `word_read`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "word_read", description = include_str!("word_read.md"), group = "core")]
pub struct WordRead {
    /// Document id.
    pub doc: String,
    /// Block, cell, span p:A..t:B (p:A.. to end), heading h:p:A or section s:2. Default body.
    #[arg(refs)]
    pub range: Option<String>,
    /// final: changes accepted; markup: changes, comments, fields in place. Default final.
    pub view: Option<View>,
    /// all adds headers, footers, notes. Default body.
    pub stories: Option<Story>,
    /// next_cursor of the previous page.
    pub cursor: Option<String>,
}

/// The reply budget a page is cut to: docx-agent's `max_result_chars` less room for the
/// envelope.
const PAGE: usize = 24_000 - 1_500;

/// `word_read` of a Word document `name` at one version of its package.
///
/// # Errors
/// `invalid_arguments` for a range with `stories=all` or a cursor this tool did not give;
/// `not_found` for a range that names nothing (with the ways out); `timeout` when
/// cancelled.
pub fn word_read(
    args: &WordRead,
    package: &Package,
    name: &str,
    cancel: &dyn Cancel,
) -> Result<Done<Value>, ToolError> {
    let stories = match args.stories {
        Some(Story::All) => Stories::All,
        _ => Stories::Body,
    };
    if args.range.is_some() && stories == Stories::All {
        let call = try_call(json!({"doc": args.doc, "stories": "all"}));
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "stories=all reads whole stories: give no range with it",
            WayOut::call(call),
        )
        .at("range"));
    }
    let view = match args.view {
        Some(View::Markup) => ViewName::Markup,
        Some(View::Original) => ViewName::Original,
        _ => ViewName::Final,
    };
    cancel.check().map_err(cancelled)?;
    let docx = Docx::open(package).map_err(|e| ToolError::internal(e.to_string(), "word_read:model"))?;
    let markdown = to_markdown(&docx, args.range.as_deref(), view, stories).map_err(|e| range_refusal(e, &args.doc))?;
    cancel.check().map_err(cancelled)?;
    let (text, next) = page_text(&args.doc, &markdown, args.cursor.as_deref(), PAGE.max(2000))?;
    let shown = args.range.clone().unwrap_or_else(|| "the document".to_owned());
    let view_name = match view {
        ViewName::Final => "final",
        ViewName::Markup => "markup",
        ViewName::Original => "original",
    };
    let _ = name;
    let mut done = Done::new(format!("Read {shown} ({view_name})"));
    done.data = Some(Value::from(text));
    done.next_cursor = next;
    done.total = Some(markdown.chars().count() as u64);
    Ok(done)
}

/// One page of `text` from `cursor` and the next page's cursor: a page breaks at a line
/// end in its second half where there is one.
fn page_text(doc: &str, text: &str, cursor: Option<&str>, limit: usize) -> Result<(String, Option<String>), ToolError> {
    let chars: Vec<char> = text.chars().collect();
    let start = match cursor.filter(|c| !c.is_empty()) {
        None => 0,
        Some(c) => c
            .strip_prefix('c')
            .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|d| d.parse::<usize>().ok())
            .ok_or_else(|| bad_cursor(doc, format!("cursor {} is not one this tool gave", py_repr(c))))?,
    };
    if start > chars.len() {
        return Err(bad_cursor(
            doc,
            format!("cursor {} is past the end", py_repr(cursor.unwrap_or(""))),
        ));
    }
    let end = start + limit;
    if end >= chars.len() {
        return Ok((chars.get(start..).unwrap_or(&[]).iter().collect(), None));
    }
    let from = start + limit / 2;
    let end = chars
        .get(from..end)
        .and_then(|window| window.iter().rposition(|c| *c == '\n'))
        .map_or(end, |k| from + k + 1);
    Ok((
        chars.get(start..end).unwrap_or(&[]).iter().collect(),
        Some(format!("c{end}")),
    ))
}

/// A cursor refused: the way out is the first page.
fn bad_cursor(doc: &str, message: String) -> ToolError {
    ToolError::refusal(
        Refusal::InvalidArguments,
        message,
        WayOut::call(try_call(json!({"doc": doc}))),
    )
    .at("cursor")
}

/// `word_read` called with `arguments`.
pub(crate) fn try_call(arguments: Value) -> TryCall {
    TryCall {
        tool: "word_read".to_owned(),
        arguments: match arguments {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        },
        fill: Vec::new(),
    }
}

/// Python's `repr` of a short string.
fn py_repr(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// A range refused: the ids that would do, and the call that reads the whole document.
fn range_refusal(error: RangeError, doc: &str) -> ToolError {
    let code = if error.code == "not_found" {
        Refusal::NotFound
    } else {
        Refusal::InvalidArguments
    };
    let call = try_call(json!({"doc": doc}));
    let way_out = match WayOut::names(error.options) {
        Some(options) => options.and_call(call),
        None => WayOut::call(call),
    };
    ToolError::refusal(code, error.message, way_out).at("range")
}
