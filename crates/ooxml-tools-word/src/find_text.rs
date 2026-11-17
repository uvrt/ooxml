//! `find_text` for Word documents: every match of a text, in document order (the body, then
//! headers, footers, notes and comments), each as a text range with its context (today's
//! shared `find_text`, `docx_agent/tools/shared.py`, over `Document.find`).
//!
//! Owns: the tool's definition (one schema for every kind, as the shared tool has: a
//! deck's `slides`, a workbook's `range`), its matching, paging and refusals.
//!
//! Must not: resolve addresses itself (`ooxml_word_model::address` does) or cache between
//! calls. A Python regular expression is matched only where it is plain text: the Rust
//! core has no engine with Python `re`'s semantics (a dependency to approve), so a pattern
//! with any of its special characters is refused with the call that finds it as text.

use crate::describe::{cancelled, short};
use crate::paging::page_list;
use crate::refusal::address_refusal;
use crate::version::Version;
use ooxml_base::Cancel;
use ooxml_tools::{ArgEnum, Done, Failure, IntOrStr, Refusal, ToolArgs, ToolError, TryCall, WayOut};
use ooxml_word_model::address::scope_paragraphs;
use ooxml_word_model::docx::text::{paragraph_text, View};
use serde_json::{json, Map, Value};

/// Which stories a search covers.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindStories {
    /// The body.
    Body,
    /// Every story.
    All,
    /// A deck's layouts.
    Layouts,
    /// A deck's masters.
    Masters,
}

/// `find_text`'s arguments, shared by every format.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(name = "find_text", description = include_str!("find_text.md"))]
pub struct FindText {
    /// Document id.
    pub doc: String,
    /// Text to find.
    pub text: String,
    /// text is a Python regex. Default false.
    pub regex: Option<bool>,
    /// Decks: slide numbers from 1, or ids (s:256, $ref). Default all.
    #[arg(kinds("pptx"), refs)]
    pub slides: Option<Vec<IntOrStr<1>>>,
    /// Documents: only within this block or span (p:A..t:B). Workbooks: a sheet, range or table.
    #[arg(
        kinds("docx", "xlsx"),
        kind_description("docx", "Only within this block or span (p:A..t:B)."),
        kind_description("xlsx", "Only within this sheet, range or table.")
    )]
    pub range: Option<String>,
    /// Documents: body (default) or all stories. Decks: body is the slides; layouts, masters, or all.
    #[arg(
        kinds("pptx", "docx"),
        kind_value("layouts", "pptx"),
        kind_value("masters", "pptx"),
        kind_description("pptx", "Body (default): the slides; layouts, masters, or all."),
        kind_description("docx", "Body (default) or all stories."),
        kind_neutral("body", "all")
    )]
    pub stories: Option<FindStories>,
    /// next_cursor of the previous page.
    pub cursor: Option<String>,
}

/// Matches on a page, as `Limits.max_list_items`.
const PAGE: usize = 50;

/// Characters of context on each side of a match.
const CONTEXT: usize = 20;

/// Python `re`'s special characters: a pattern with none of them matches as its text.
const SPECIAL: &[char] = &['.', '^', '$', '*', '+', '?', '{', '}', '[', ']', '\\', '|', '(', ')'];

/// `find_text` on a Word document at one version.
///
/// # Errors
/// `unit` for nothing to find; the address resolver's refusal of `range`; `refused` for
/// a regular expression the core cannot match; `invalid_arguments` for a cursor this tool
/// did not give; `timeout` when cancelled.
pub fn find_text(args: &FindText, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    if args.regex == Some(true) && args.text.contains(SPECIAL) {
        let mut call = args.clone();
        call.regex = None;
        return Err(ToolError::refusal(
            Refusal::Refused,
            "regex: the Rust core matches a pattern only as plain text (no special characters) so far; find the words themselves",
            WayOut::call(try_call(&call)),
        )
        .at("regex"));
    }
    if args.text.is_empty() {
        return Err(ToolError::failure(Failure::Unit, "nothing to find").at("text"));
    }
    let docx = version.docx()?;
    let docx = &*docx;
    let scopes = match args.range.as_deref() {
        None => docx
            .parts
            .iter()
            .map(|p| (&**p, (0..p.ids.paragraphs.len()).collect()))
            .collect(),
        Some(range) => {
            let mut call = args.clone();
            call.range = None;
            vec![scope_paragraphs(docx, range).map_err(|e| address_refusal(e, "range", try_call(&call)))?]
        }
    };
    let every = args.stories == Some(FindStories::All);
    let needle: Vec<char> = args.text.chars().collect();
    let mut rows = Vec::new();
    for (part, paragraphs) in scopes {
        cancel.check().map_err(cancelled)?;
        for k in paragraphs {
            let Some(entry) = part.ids.paragraphs.get(k) else {
                continue;
            };
            let text: Vec<char> = paragraph_text(&entry.element, View::Current).chars().collect();
            for start in matches(&text, &needle) {
                let address = format!("{}@{start}:{}", entry.id, start + needle.len());
                if !(every || story_ok(&address)) {
                    continue;
                }
                rows.push(row(&address, &text, start, start + needle.len()));
            }
        }
    }
    let mut first = args.clone();
    first.cursor = None;
    let (shown, total, next) = page_list(&rows, args.cursor.as_deref(), PAGE, try_call(&first))?;
    let mut done = Done::new(format!("{total} match(es)"));
    done.data = Some(Value::Array(shown));
    done.next_cursor = next;
    done.total = Some(total as u64);
    Ok(done)
}

/// Where `needle` starts in `text`, left to right and not overlapping (as `re.finditer`
/// finds an escaped literal).
fn matches(text: &[char], needle: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + needle.len() <= text.len() {
        if text.get(at..at + needle.len()) == Some(needle) {
            out.push(at);
            at += needle.len();
        } else {
            at += 1;
        }
    }
    out
}

/// A match's row: its address, kind, text and context (`before[text]after`).
fn row(address: &str, text: &[char], start: usize, end: usize) -> Value {
    let piece = |a: usize, b: usize| -> String { text.get(a..b).unwrap_or(&[]).iter().collect() };
    let found = piece(start, end);
    let context = format!(
        "{}[{found}]{}",
        piece(start.saturating_sub(CONTEXT), start),
        piece(end, (end + CONTEXT).min(text.len()))
    );
    json!({"address": address, "kind": "text", "text": short(&found, 80), "context": context})
}

/// Whether a match's address is in the body (docx-agent's `_story_ok` for `stories=body`).
fn story_ok(address: &str) -> bool {
    let block = address.split('@').next().unwrap_or(address);
    !(block.contains('/') && !address.starts_with("p@body/"))
}

/// `find_text` called with `args`.
fn try_call(args: &FindText) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    arguments.insert("text".into(), Value::from(args.text.as_str()));
    if let Some(regex) = args.regex {
        arguments.insert("regex".into(), Value::from(regex));
    }
    if let Some(range) = &args.range {
        arguments.insert("range".into(), Value::from(range.as_str()));
    }
    if args.stories == Some(FindStories::All) {
        arguments.insert("stories".into(), Value::from("all"));
    }
    if let Some(cursor) = &args.cursor {
        arguments.insert("cursor".into(), Value::from(cursor.as_str()));
    }
    TryCall {
        tool: "find_text".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}
