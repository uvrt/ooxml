//! `word_read`: the document as Markdown with an id comment before each block, paged
//! (today's `word_read`, `docx_agent/tools/read.py`).
//!
//! Owns: the tool's definition, its paging (a page breaks at a line end in its second
//! half; the cursor is `c<offset>`), and the refusals of its arguments.
//!
//! Must not: write Markdown itself (`ooxml_word_model::markdown` does) or cache between
//! calls.

use crate::describe::cancelled;
use crate::paging::page_text;
use crate::version::Version;
use ooxml_base::Cancel;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolArgs, ToolError, TryCall, WayOut};
use ooxml_word_model::address::expand_heading;
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

/// Room left for the envelope when a page is cut to the reply's characters.
const MARGIN: usize = 1_500;

/// `word_read` of a Word document `name` at one version of its package.
///
/// # Errors
/// `invalid_arguments` for a range with `stories=all` or a cursor this tool did not give;
/// `not_found` for a range that names nothing (with the ways out); `timeout` when
/// cancelled.
pub fn word_read(args: &WordRead, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
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
    let docx = version.docx()?;
    // A heading's range is read, and named, as the span it is (today's `resolve`).
    // ... through the session's renames (a place id stamping renamed), as docx-agent's view
    let range = args
        .range
        .as_deref()
        .map(|r| expand_heading(&docx, &version.follow(r)))
        .transpose()
        .map_err(|e| range_refusal(e, &args.doc))?;
    let markdown = to_markdown(&docx, range.as_deref(), view, stories).map_err(|e| range_refusal(e, &args.doc))?;
    let unported = unported(&docx, view);
    cancel.check().map_err(cancelled)?;
    let first_page = try_call(json!({"doc": args.doc}));
    let (text, next) = page_text(
        &markdown,
        args.cursor.as_deref(),
        version.limits.result_chars.saturating_sub(MARGIN).max(2000),
        first_page,
    )?;
    let shown = range.unwrap_or_else(|| "the document".to_owned());
    let view_name = match view {
        ViewName::Final => "final",
        ViewName::Markup => "markup",
        ViewName::Original => "original",
    };
    let mut done = Done::new(format!("Read {shown} ({view_name})"));
    done.data = Some(Value::from(text));
    done.next_cursor = next;
    done.total = Some(markdown.chars().count() as u64);
    done.unported = unported;
    Ok(done)
}

/// What of a reading in `view` docx-agent writes otherwise: a cell's end that Accept All
/// takes away, in the final view.
fn unported(docx: &ooxml_word_model::docx::Docx, view: ViewName) -> Vec<String> {
    let shared = ooxml_word_model::markdown::read::Shared::of(docx);
    if view == ViewName::Final && shared.cell_ends(docx) {
        return vec![crate::CELL_ENDS.to_owned()];
    }
    Vec::new()
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

/// A range refused: the ids that would do, and the call that reads the whole document.
fn range_refusal(error: RangeError, doc: &str) -> ToolError {
    crate::refusal::address_refusal(error, "range", try_call(json!({"doc": doc})))
}

/// Registers `word_read` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.read::<WordRead>(word_read);
}
