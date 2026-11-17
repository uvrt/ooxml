//! `word_inspect`: the exact formatting of blocks (today's `word_inspect`,
//! `docx_agent/tools/read.py`, over docx-agent's `state`, `docx_agent/state.py`, schema
//! `docx-agent/state` version 1): each paragraph's style, Markdown construct, list, text,
//! effective formatting and runs, links, bookmarks, notes, comments, revisions, pictures,
//! drawings, controls and fields; each table's cells with their fill and drawn text colour;
//! the notes, comments and revisions the blocks reference, the styles they use, and for a
//! cell its own properties.
//!
//! Owns: the tool's definition and the reply's shape, key for key as today's.
//!
//! Must not: lay out (`layout: true` waits for the Rust layout's switch, phase PL step
//! L4, and is refused with the call that works), read addresses itself (the address
//! resolver does), or cache between calls.

mod blocks;
mod extras;

use crate::describe::cancelled;
use crate::refusal::address_refusal;
use crate::version::Version;
use blocks::Writer;
use ooxml_base::Cancel;
use ooxml_tools::{Done, Refusal, ToolArgs, ToolError, TryCall, WayOut};
use ooxml_word_model::markdown::read::Reader;
use ooxml_word_model::markdown::select::{select, Selected};
use ooxml_word_model::markdown::ViewName;
use serde_json::{json, Map, Value};

/// `word_inspect`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "word_inspect", description = include_str!("word_inspect.md"), group = "word_text")]
pub struct WordInspect {
    /// Document id.
    pub doc: String,
    /// A block, cell t:X/c1,1, span p:A..t:B (p:A.. to the end) or section.
    #[arg(refs)]
    pub target: String,
    /// Also each block's page, top and bottom. Default false.
    pub layout: Option<bool>,
}

/// `word_inspect` on a Word document at one version.
///
/// # Errors
/// The address resolver's refusal of `target`; `refused` for `layout: true` (the Rust
/// layout is not switched on yet); `timeout` when cancelled.
pub fn word_inspect(args: &WordInspect, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    if args.layout == Some(true) {
        let mut call = args.clone();
        call.layout = None;
        return Err(ToolError::refusal(
            Refusal::Refused,
            "layout: where blocks are placed is read from the Word layout, which the Rust core does not serve yet; inspect without layout",
            WayOut::call(try_call(&call)),
        )
        .at("layout"));
    }
    let docx = version.docx()?;
    let mut reader = Reader::new(&docx, ViewName::Final);
    let mut first = args.clone();
    first.target = "body".to_owned();
    let refuse = |e| address_refusal(e, "target", try_call(&first));
    let selected = select(&mut reader, Some(&args.target)).map_err(refuse)?;
    cancel.check().map_err(cancelled)?;
    let mut writer = Writer::new(&docx);
    let mut blocks = Vec::new();
    let mut all = Vec::new();
    let mut cell = false;
    // A cell's blocks are read in their table's style (today's model reads them so).
    let style = ooxml_word_model::address::in_table(&docx, &args.target)
        .ok()
        .flatten()
        .filter(|found| found.column.is_some())
        .and_then(|found| {
            found
                .part
                .ids
                .tables()
                .get(found.table)
                .map(|t| ooxml_word_model::markdown::read::table_style(&t.element))
        });
    for found in selected {
        match found {
            Selected::Notes(part) => writer.notes_of_part(&part),
            Selected::Records {
                records, cell: alone, ..
            } => {
                cell |= alone;
                let within = if alone { style.as_deref() } else { None };
                blocks.extend(writer.blocks(&mut reader, &records, within));
                all.extend(records);
            }
        }
    }
    writer.referenced_notes(&all);
    cancel.check().map_err(cancelled)?;
    let mut data = Map::new();
    if cell {
        if let Some(found) = extras::cell(&docx, &args.target) {
            data.insert("cell".into(), found);
        }
    }
    let notes = writer.notes(&mut reader);
    let comments = writer.comments(&reader);
    let revisions = writer.revisions();
    let styles = writer.styles();
    for (key, value) in [
        ("styles", styles),
        ("blocks", Value::Array(blocks)),
        ("notes", notes),
        ("comments", comments),
        ("revisions", revisions),
    ] {
        if !is_empty(&value) {
            data.insert(key.into(), value);
        }
    }
    let mut done = Done::new(format!("Inspected {}", args.target));
    done.data = Some(Value::Object(data));
    Ok(done)
}

/// Whether Python reads `value` as false (`if state.get(key)`).
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// `word_inspect` called with `args`.
fn try_call(args: &WordInspect) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    arguments.insert("target".into(), Value::from(args.target.as_str()));
    if let Some(layout) = args.layout {
        arguments.insert("layout".into(), json!(layout));
    }
    TryCall {
        tool: "word_inspect".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}
