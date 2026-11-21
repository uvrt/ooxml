//! Owns: `word_insert_markdown` (docx-agent's `word_insert_markdown`): its definition and
//! its Markdown written as the document's own blocks through the edit core's
//! [`InsertMarkdown`] (one history step), the reply naming the blocks made (`data.blocks`),
//! each named by the call's `ref`. Values the host wrote by reference into the Markdown
//! (`editing.written`, field `markdown`) -- or the one field of one row `from` named, which
//! the host hands over as the Markdown with its value (field `from`) -- are kept as
//! docx-agent's `_fields_placed` and `keep_written` keep them: a value with Markdown of its
//! own written as it, the blocks each value standing alone made kept with it as their source
//! and named in the reply's `data.facts` ([`super::fields`]), every value found in the
//! paragraphs written kept on its place.
//!
//! The Markdown may be a blob's text (`blob`, UTF-8), and its pictures' sources blobs, by
//! handle or name (docx-agent's `images_from`), read before anything changes in the order
//! docx-agent reads them: a refusal of a picture comes after the place's.
//!
//! Left to the host (`unported`, nothing changed): rows or a field as the content (`from`),
//! a blob or a picture's blob where the host hands over none, references not yet written
//! (`{{...}}`) and a place given by reference (`$name`), list items or tables like given
//! blocks (`like`, a style map naming a paragraph), and what the command does not write yet
//! (footnotes, `replace:`).
//!
//! Must not: write the document but through the command, or answer a call it does not
//! port otherwise than by saying so ([`Unported`]).

use super::fields::{placed_blocks, with_markdown};
use super::Unported;
use crate::review::Editing;
use crate::structure::common::{blocks_ref_as_place, is_place, name_blocks, outcome, refused, Outcome};
use crate::written::{self, Written};
use ooxml_tools::{ArgEnum, Outcome as Replied, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{
    read_pictures, InsertMarkdown, MarkdownInserted, MarkdownOptions, PictureRefusal, Pictures, WriteMap,
};
use serde_json::{json, Map, Value};

/// A Markdown construct a style map names a style for.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleElement {
    /// Body paragraphs.
    Paragraph,
    /// Heading 1.
    H1,
    /// Heading 2.
    H2,
    /// Heading 3.
    H3,
    /// Heading 4.
    H4,
    /// Heading 5.
    H5,
    /// Heading 6.
    H6,
    /// A block quote.
    Quote,
    /// A code block.
    Code,
    /// Inline code.
    InlineCode,
    /// A bulleted item.
    Bullet,
    /// A bulleted item, level 2.
    Bullet2,
    /// A bulleted item, level 3.
    Bullet3,
    /// A numbered item.
    Number,
    /// A numbered item, level 2.
    Number2,
    /// A numbered item, level 3.
    Number3,
    /// A table's style.
    Table,
    /// A table cell's paragraphs.
    TableCell,
    /// The paragraphs of a centred column's cells.
    TableCellCenter,
    /// The paragraphs of a right-aligned column's cells.
    TableCellRight,
    /// A footnote's text.
    Footnote,
    /// A footnote reference.
    FootnoteReference,
    /// Emphasis.
    Emphasis,
    /// Strong.
    Strong,
    /// A link.
    Link,
    /// A caption.
    Caption,
}

/// One construct's style.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct StyleMapEntry {
    /// Markdown element.
    pub element: StyleElement,
    /// The document's style name for it.
    pub style: String,
}

/// `word_insert_markdown`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_insert_markdown",
    description = include_str!("word_insert_markdown.md"),
    group = "word_text",
    mutates,
    exactly_one("markdown", "blob", "from"),
    places = "blocks",
    place_own("at")
)]
pub struct WordInsertMarkdown {
    /// Document id.
    pub doc: String,
    /// end, `after:<id>`, `before:<id>` or `replace:<id>[..<id>]`.
    #[arg(description = "end, after:<id>, before:<id> or replace:<id>[..<id>].")]
    pub at: String,
    /// Markdown.
    #[arg(facts)]
    pub markdown: Option<String>,
    /// Or a Markdown blob handle.
    pub blob: Option<String>,
    /// Or rows of `d:<id>` or `t:<table>` as a table; one text field as Markdown.
    #[arg(
        rename = "from",
        sources,
        refused_alias = "markdown_from",
        description = "Or rows of d:<id> or t:<table> as a table; one text field (d:<id>/#12/<col>) as Markdown."
    )]
    pub source: Option<String>,
    /// from's columns, in order. Default all.
    #[arg(alias = "columns")]
    pub from_columns: Option<Vec<String>>,
    /// from's header texts; [] none. Default column names.
    pub header: Option<Vec<String>>,
    /// Dates of from as e.g. d mmmm yyyy or dd/mm/yyyy.
    pub date_format: Option<String>,
    /// Styles differing from the defaults (h1 Heading 1, paragraph Normal...).
    pub style_map: Option<Vec<StyleMapEntry>>,
    /// Paragraphs list items look like, by level; a table: its frame, empty cells joining merges.
    #[arg(single_as_list)]
    pub like: Option<Vec<String>>,
    /// Retry key: a repeat with the same key makes nothing new.
    pub key: Option<String>,
    /// Ref name for the new blocks ($name).
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
}

/// `word_insert_markdown` on the document `editing` names: the Markdown written at `at` as
/// one command. A failed reply is `invalid_arguments` for a `ref` holding an address or a
/// style map the edit core cannot read, the edit's refusal (a place the document lacks, raw
/// HTML, a style it lacks); `unported` (nothing changed) for what is the host's.
#[must_use]
pub fn word_insert_markdown(args: &WordInsertMarkdown, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &WordInsertMarkdown, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    crate::review::stopped(editing.cancel)?;
    if let Some(name) = args.ref_name.as_deref().filter(|r| is_place(r)) {
        return Err(blocks_ref_as_place(name, &args.at));
    }
    if let Some(unported) = unported(args, editing) {
        return Ok(unported.reply());
    }
    let call = try_call(args);
    let given = match (&args.blob, editing.blobs) {
        (Some(handle), Some(blobs)) => blob_text(blobs, handle, &call)?,
        _ => args.markdown.clone().unwrap_or_default(),
    };
    let pairs: Vec<(String, Option<String>)> = args
        .style_map
        .iter()
        .flatten()
        .map(|e| (e.element.as_str().to_owned(), Some(e.style.clone())))
        .collect();
    let style_map = WriteMap::from_pairs(&pairs).map_err(|message| {
        ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(call.clone())).at("style_map")
    })?;
    let field = editing.written.iter().find(|f| f.field == "from");
    let in_markdown: Vec<&Written> = editing.written.iter().filter(|f| f.field == "markdown").collect();
    let (markdown, facts) = if field.is_none() && args.markdown.is_some() && !in_markdown.is_empty() {
        with_markdown(&given, &in_markdown)
    } else {
        (given, Vec::new())
    };
    let mut command = InsertMarkdown {
        markdown: markdown.clone(),
        at: args.at.clone(),
        style_map,
        // docx-agent's tool refuses raw HTML and writes emphasis in its styles
        options: MarkdownOptions::default(),
        policy: editing.policy(),
    };
    match pictures(&command, editing, &call) {
        Ok(Some(pictures)) => command.options.pictures = pictures,
        Ok(None) => return Ok(Unported::HostData.reply()),
        Err(reply) => return Ok(*reply),
    }
    let inserted = match editing.editor.run(editing.caller, &command) {
        Ok(done) => done.out,
        Err(error) => return Ok(refused(error, call)),
    };
    let blocks = inserted.blocks.clone();
    let mut result = reply(args, inserted);
    if !editing.written.is_empty() {
        keep_facts(args, editing, &markdown, field, &facts, &blocks, &mut result)?;
    }
    Ok(result)
}

/// The note when the values written by reference could not be lined up with the blocks.
const NOT_LINED_UP: &str =
    "the values written by reference could not be lined up with the blocks made: word_read the new blocks to see which is which";

/// Round 45, R65: the reply's `facts` -- the `from` field, or each value in the Markdown
/// (`facts`, their places in `markdown`, the text written) with the blocks a value
/// standing alone made, kept as their source -- and a note first where they could not be
/// lined up (docx-agent's `_fields_placed`); then every value found where it went
/// (`keep_written`, on the call's own Markdown).
fn keep_facts(
    args: &WordInsertMarkdown,
    editing: &Editing<'_>,
    markdown: &str,
    field: Option<&Written>,
    facts: &[Written],
    blocks: &[String],
    result: &mut ToolResult<Value>,
) -> Result<(), ToolError> {
    let after = written::after_edit(editing)?;
    let mut keeps = Vec::new();
    let said = if let Some(field) = field {
        keeps.extend(written::whole_keeps(&after, field, blocks));
        Some(vec![written::brief(field, Some(blocks))])
    } else if facts.is_empty() {
        None
    } else {
        let (found, lined_up) = placed_blocks(markdown, facts, blocks);
        let mut said = Vec::new();
        for (k, fact) in facts.iter().enumerate() {
            let made = found.get(&k);
            if let Some(made) = made {
                keeps.extend(written::whole_keeps(&after, fact, made));
            }
            said.push(written::brief(fact, made.map(Vec::as_slice)));
        }
        if !lined_up {
            result.warnings.insert(0, NOT_LINED_UP.to_owned());
        }
        Some(said)
    };
    if let Replied::Done(done) = &mut result.outcome {
        let argument = |name: &str| (name == "markdown").then(|| args.markdown.clone()).flatten();
        keeps.extend(written::written_keeps(&after, editing, done, &[], &argument));
        if let Some(said) = said {
            let mut data = match done.data.take() {
                Some(Value::Object(data)) => data,
                _ => Map::new(),
            };
            data.insert("facts".into(), Value::Array(said));
            done.data = Some(Value::Object(data));
        }
    }
    written::keep(editing, keeps, &try_call(args))
}

/// The text of the blob `handle` names, refused where it is not UTF-8.
fn blob_text(blobs: &dyn crate::Blobs, handle: &str, call: &TryCall) -> Result<String, ToolError> {
    let found = crate::blobs::blob(blobs, handle, call)?;
    String::from_utf8(found.data.to_vec()).map_err(|_| {
        let message = format!("{handle} is not UTF-8 text");
        ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(call.clone())).at("blob")
    })
}

/// Why the host could not hand a picture over.
enum Hosted {
    /// It hands over no blobs: the call is the host's.
    Unported,
    /// Its refusal.
    Refused(ToolError),
}

/// The bytes of the pictures the command's Markdown shows (`None`: a picture's blob, which
/// the host hands over none of); a refusal of one is the reply, after the place's.
fn pictures(
    command: &InsertMarkdown,
    editing: &Editing<'_>,
    call: &TryCall,
) -> Result<Option<Pictures>, Box<ToolResult<Value>>> {
    // every picture's syntax starts `![`: Markdown without it shows none
    if !command.markdown.contains("![") {
        return Ok(Some(Pictures::new()));
    }
    let hosted = |src: &str| match editing.blobs {
        Some(blobs) => crate::blobs::image(blobs, src, call).map_err(Hosted::Refused),
        None => Err(Hosted::Unported),
    };
    let refusal = match read_pictures(&command.markdown, hosted) {
        Ok(pictures) => return Ok(Some(pictures)),
        Err(PictureRefusal::Host(Hosted::Unported)) => return Ok(None),
        Err(PictureRefusal::Host(Hosted::Refused(error))) => ToolResult::failed(error),
        Err(PictureRefusal::Edit(error)) => refused(error, call.clone()),
    };
    let first = command.check_place(editing.editor.store().snapshot().doc());
    Err(Box::new(match first {
        Err(error) => refused(error, call.clone()),
        Ok(()) => refusal,
    }))
}

/// What of the call is the host's: data it holds (a blob only where it hands over none),
/// values or places by reference, blocks whose look new ones take.
fn unported(args: &WordInsertMarkdown, editing: &Editing<'_>) -> Option<Unported> {
    let from_data = args.source.is_some()
        || (args.blob.is_some() && editing.blobs.is_none())
        || args.from_columns.is_some()
        || args.header.is_some()
        || args.date_format.is_some();
    if from_data {
        return Some(Unported::HostData);
    }
    let by_reference = args.at.contains('$') || args.markdown.as_deref().is_some_and(|m| m.contains("{{"));
    if by_reference {
        return Some(Unported::Reference);
    }
    // a style map naming a paragraph for a construct is read as like (docx-agent's `_levels_like`)
    let like = args.like.as_ref().is_some_and(|l| !l.is_empty())
        || args.style_map.iter().flatten().any(|e| is_place(&e.style));
    like.then_some(Unported::Like)
}

/// The reply, as docx-agent's `outcome` and `name_blocks` make it of the insertion.
fn reply(args: &WordInsertMarkdown, out: MarkdownInserted) -> ToolResult<Value> {
    let edit = Outcome {
        id: out.first,
        changed: out.changed,
        created: out.created,
        removed: out.removed,
        renamed: out.renamed,
        warnings: out.warnings,
    };
    let summary = format!("Inserted {} block(s)", out.blocks.len());
    let mut result = outcome(&[edit], summary, Some(json!({"blocks": out.blocks})));
    name_blocks(&mut result, args.ref_name.as_ref(), &out.blocks);
    result
}

/// `word_insert_markdown` called with `args` (its text arguments).
fn try_call(args: &WordInsertMarkdown) -> TryCall {
    let mut arguments = Map::new();
    for (key, value) in [
        ("doc", Some(&args.doc)),
        ("at", Some(&args.at)),
        ("markdown", args.markdown.as_ref()),
        ("ref", args.ref_name.as_ref()),
    ] {
        if let Some(v) = value {
            arguments.insert(key.into(), Value::from(v.as_str()));
        }
    }
    TryCall {
        tool: "word_insert_markdown".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

/// Registers `word_insert_markdown` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordInsertMarkdown>(word_insert_markdown, crate::Hosting::SERVED.blobs().keeps_written());
}
