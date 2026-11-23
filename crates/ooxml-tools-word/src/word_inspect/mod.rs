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
//! out, asked once for them all -- of the host's [`crate::Placer`] where it gives one, else
//! of the call's own layout of the version (`placing`) -- in the place docx-agent's `state`
//! keeps for it.
//!
//! Must not: lay out but through the call's layout (without a placer or a layout
//! `layout: true` is refused with the call that works), read addresses itself (the address
//! resolver does), or cache between calls.

mod blocks;
mod extras;
mod placing;

pub(crate) use extras::comments_with_anchors;

use crate::describe::cancelled;
use crate::pages::NotLaid;
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
    if let Some(refusal) = not_supported(&docx, args) {
        return Err(refusal);
    }
    let mut reader = Reader::new(&docx, ViewName::Final);
    let mut first = args.clone();
    first.target = "body".to_owned();
    let refuse = |e| address_refusal(e, "target", try_call(&first));
    let target = version.follow(&args.target);
    let selected = select(&mut reader, Some(&target)).map_err(refuse)?;
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
        let answers = placed(args, placer, &asked, version, &docx, cancel)?;
        crate::placements::fill(&mut data, &answers);
    }
    let mut done = Done::new(format!("Inspected {}", args.target));
    done.data = Some(data);
    Ok(done)
}

/// Where the blocks `asked` are placed, as `placer` says.
fn placed(
    args: &WordInspect,
    placer: Placing<'_>,
    asked: &[Vec<String>],
    version: &Version<'_>,
    docx: &ooxml_word_model::docx::Docx,
    cancel: &dyn Cancel,
) -> Result<Vec<Map<String, Value>>, ToolError> {
    match placer {
        Placing::Host(placer) => placer
            .place(asked)
            .ok_or_else(|| ToolError::internal("the layout did not place the blocks", "word_inspect:layout")),
        Placing::Own(layout, limit) => {
            let stop = limit.map_or(cancel, |l| l.cancel);
            let laid = placing::Laid::new(layout, version.package, docx, stop)
                .map_err(|why| not_laid(args, why, limit, cancel))?;
            Ok(laid.place(asked))
        }
    }
}

/// Who says where blocks are placed: the host's placer, or the call's layout of the version.
#[derive(Clone, Copy)]
enum Placing<'v> {
    Host(&'v dyn crate::Placer),
    Own(&'v dyn crate::WordLayout, Option<crate::LayoutLimit<'v>>),
}

/// Who `layout: true` asks (`None` without `layout`): the host's placer where it gives one,
/// else the call's own layout of the version; refused where the call has neither, with the
/// call that works without.
fn placer_of<'v>(args: &WordInspect, version: &Version<'v>) -> Result<Option<Placing<'v>>, ToolError> {
    if args.layout != Some(true) {
        return Ok(None);
    }
    if let Some(placer) = version.placer {
        return Ok(Some(Placing::Host(placer)));
    }
    let own = version
        .owned
        .and_then(|owned| owned.layout.map(|layout| Placing::Own(layout, owned.layout_limit)));
    own.map(Some).ok_or_else(|| {
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

/// Why the call's layout placed no blocks: the call's own stop as a timeout; the layout's
/// limit passed, or a version it does not lay out, refused with the call that works without.
fn not_laid(args: &WordInspect, why: NotLaid, limit: Option<crate::LayoutLimit<'_>>, cancel: &dyn Cancel) -> ToolError {
    if let Err(stopped) = cancel.check() {
        return cancelled(stopped);
    }
    if why.in_note() {
        let mut call = args.clone();
        call.layout = None;
        let form = format!("word_inspect layout: {}", crate::pages::UNLAID_NOTE);
        return ToolError::not_supported(&form, WayOut::call(try_call(&call))).at("layout");
    }
    let said = match (why, limit) {
        (NotLaid::Stopped, Some(limit)) => format!(
            "laying the document out took longer than the layout limit ({} s)",
            limit.timeout.as_secs_f64()
        ),
        (NotLaid::Stopped, None) => "laying the document out was stopped".to_owned(),
        (reason, _) => format!("the Word layout does not lay this version out yet ({reason})"),
    };
    let mut call = args.clone();
    call.layout = None;
    ToolError::refusal(
        Refusal::Refused,
        format!("layout: {said}; inspect without layout"),
        WayOut::call(try_call(&call)),
    )
    .at("layout")
}

/// An inspection not supported yet: of a document with cells whose end Accept All takes
/// away, refused with reading the target in the markup view as the way out.
fn not_supported(docx: &ooxml_word_model::docx::Docx, args: &WordInspect) -> Option<ToolError> {
    let shared = ooxml_word_model::markdown::read::Shared::of(docx);
    if !shared.cell_ends(docx) {
        return None;
    }
    let read = json!({"doc": args.doc, "range": args.target, "view": "markup"});
    Some(crate::refusal::not_supported(
        &format!("word_inspect of {}", crate::CELL_ENDS),
        "word_read",
        read,
        &[],
    ))
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
