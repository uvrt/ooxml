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
//! With `layout: true` each block (and each block of the notes shown) gets where it is laid
//! out, asked of the version's [`crate::Placer`] once for them all, in the place
//! docx-agent's `state` keeps for it.
//!
//! Must not: lay out (the host's placer does; without one `layout: true` is refused with
//! the call that works), read addresses itself (the address resolver does), or cache
//! between calls.

mod blocks;
mod extras;

pub(crate) use extras::comments_with_anchors;

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
/// The address resolver's refusal of `target`; `refused` for `layout: true` where the
/// version has no placer; `internal` where its placer could not place the blocks; `timeout`
/// when cancelled.
pub fn word_inspect(args: &WordInspect, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    let placer = placer_of(args, version)?;
    let docx = version.docx()?;
    let mut reader = Reader::new(&docx, ViewName::Final);
    let mut first = args.clone();
    first.target = "body".to_owned();
    let refuse = |e| address_refusal(e, "target", try_call(&first));
    let target = followed(args, version);
    let selected = match select(&mut reader, Some(&target)) {
        Ok(selected) => selected,
        Err(e) if e.code == "not_found" && names_place(&target) => return Ok(renamed_place()),
        Err(e) => return Err(refuse(e)),
    };
    cancel.check().map_err(cancelled)?;
    let mut writer = Writer::new(&docx);
    if placer.is_some() {
        writer.placing = Some(Vec::new());
    }
    let mut blocks = Vec::new();
    let mut all = Vec::new();
    let mut cell = false;
    // A cell's blocks are read in their table's style (today's model reads them so).
    let style = ooxml_word_model::address::in_table(&docx, &target)
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
        if let Some(found) = extras::cell(&docx, &target) {
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
    let mut data = Value::Object(data);
    if let (Some(placer), Some(asked)) = (placer, writer.placing.take()) {
        cancel.check().map_err(cancelled)?;
        let answers = placer
            .place(&asked)
            .ok_or_else(|| ToolError::internal("the layout did not place the blocks", "word_inspect:layout"))?;
        crate::placements::fill(&mut data, &answers);
    }
    let mut done = Done::new(format!("Inspected {}", args.target));
    done.unported = unported(&docx);
    done.data = Some(data);
    Ok(done)
}

/// The placer `layout: true` asks (`None` without `layout`); refused where the version has
/// none, with the call that works without.
fn placer_of<'v>(args: &WordInspect, version: &Version<'v>) -> Result<Option<&'v dyn crate::Placer>, ToolError> {
    if args.layout != Some(true) {
        return Ok(None);
    }
    version.placer.map(Some).ok_or_else(|| {
        let mut call = args.clone();
        call.layout = None;
        ToolError::refusal(
            Refusal::Refused,
            "layout: where blocks are placed is read from the Word layout, which this session is not given; inspect without layout",
            WayOut::call(try_call(&call)),
        )
        .at("layout")
    })
}

/// The target read through the session's renames, as docx-agent's view reads it.
fn followed(args: &WordInspect, version: &Version<'_>) -> String {
    version.owned.map_or_else(
        || args.target.clone(),
        |owned| owned.doc.ids().follow_address(&args.target),
    )
}

/// A place id (`t@body/0`) stamping renamed, where this version holds no renames to follow:
/// docx-agent's view follows it -- said, for the host to answer.
fn renamed_place() -> Done<Value> {
    let mut done = Done::new(String::new());
    done.unported.push(crate::RENAMED_PLACE.to_owned());
    done
}

/// Whether `address` is, or is in, a block named by its place (`p@body/3`, `t@body/0/c1,1`).
fn names_place(address: &str) -> bool {
    let block = ooxml_word_model::address::inside_table(address).map_or(address, |(table, _)| table);
    block.starts_with("p@") || block.starts_with("t@")
}

/// What docx-agent gives otherwise: the final view's cells whose end Accept All takes away.
fn unported(docx: &ooxml_word_model::docx::Docx) -> Vec<String> {
    let shared = ooxml_word_model::markdown::read::Shared::of(docx);
    if shared.cell_ends(docx) {
        return vec![crate::CELL_ENDS.to_owned()];
    }
    Vec::new()
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

/// Registers `word_inspect` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.read::<WordInspect>(word_inspect);
}
