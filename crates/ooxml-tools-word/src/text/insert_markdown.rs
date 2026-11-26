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
//! Refused as not supported yet (nothing changed): rows or a field as the content (`from`),
//! a blob (`blob`), a place given by reference (`$name`; braces reaching the tool are literal
//! text, the host having written its references as their values), list items or tables like given blocks (`like`, a style map naming a
//! paragraph), and what the command does not write yet (pictures, footnotes, `replace:`).
//!
//! Must not: write the document but through the command, or answer a call it does not
//! support yet, refused ([`Unported`]).

use super::fields::{placed_blocks, with_markdown};
use super::Unported;
use crate::review::{edit_refusal, Editing};
use crate::structure::common::{blocks_ref_as_place, is_place, name_blocks, outcome, Outcome};
use crate::written::{self, Written};
use ooxml_tools::{ArgEnum, Outcome as Replied, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{InsertMarkdown, MarkdownInserted, MarkdownOptions, TextKeep, WriteMap};
use ooxml_word_model::address::Place;
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
/// HTML, a style it lacks); `refused` (nothing changed) for what is not supported yet.
#[must_use]
pub fn word_insert_markdown(args: &WordInsertMarkdown, editing: &Editing<'_>) -> ToolResult<Value> {
    adapted(args, editing)
        .and_then(|(read, value)| run(read.as_ref().unwrap_or(args), editing, value.as_ref()))
        .unwrap_or_else(ToolResult::failed)
}

/// The call as the tool takes it (docx-agent's adapter): a Markdown blob read as its
/// Markdown, a `from` naming one text field read as its Markdown, with that value as it is
/// written ([`super::field_from`]); `None`: as given.
///
/// # Errors
/// The blob's or the field's refusal.
fn adapted(
    args: &WordInsertMarkdown,
    editing: &Editing<'_>,
) -> Result<(Option<WordInsertMarkdown>, Option<Written>), ToolError> {
    let read = from_blob(args, editing)?;
    let given = read.as_ref().unwrap_or(args);
    Ok(match super::field_from::field_from(given, editing, &try_call(given))? {
        Some((field, value)) => (Some(field), Some(value)),
        None => (read, None),
    })
}

/// The call with its Markdown blob read as its Markdown (docx-agent's adapter: the blob is
/// the host's, its text the Markdown), where it gives a blob and neither Markdown nor
/// `from`, and the host handed the blobs over; `None` otherwise.
///
/// # Errors
/// `not_found` for a blob there is not; `invalid_arguments` for one that is not UTF-8 text.
fn from_blob(args: &WordInsertMarkdown, editing: &Editing<'_>) -> Result<Option<WordInsertMarkdown>, ToolError> {
    let (Some(handle), None, None, Some(blobs)) = (&args.blob, &args.markdown, &args.source, editing.blobs) else {
        return Ok(None);
    };
    let call = try_call(args);
    let found = crate::blobs::blob(blobs, handle, &call)?;
    let text = String::from_utf8(found.data.to_vec()).map_err(|_| {
        ToolError::refusal(
            Refusal::InvalidArguments,
            format!("{handle} is not UTF-8 text: Markdown is given as text"),
            WayOut::call(call.clone()),
        )
        .at("blob")
    })?;
    let mut read = args.clone();
    read.markdown = Some(text);
    read.blob = None;
    Ok(Some(read))
}

fn run(
    args: &WordInsertMarkdown,
    editing: &Editing<'_>,
    from: Option<&Written>,
) -> Result<ToolResult<Value>, ToolError> {
    crate::review::stopped(editing.cancel)?;
    if let Some(name) = args.ref_name.as_deref().filter(|r| is_place(r)) {
        return Err(blocks_ref_as_place(name, &args.at));
    }
    if let Some(unported) = unported(args) {
        let mut call = try_call(args);
        if !call.arguments.contains_key("markdown") {
            call.arguments.insert("markdown".into(), Value::from("A paragraph."));
        }
        return Err(unported.refusal(call, &["markdown"]));
    }
    let call = try_call(args);
    let pairs: Vec<(String, Option<String>)> = args
        .style_map
        .iter()
        .flatten()
        .map(|e| (e.element.as_str().to_owned(), Some(e.style.clone())))
        .collect();
    let style_map = WriteMap::from_pairs(&pairs).map_err(|message| {
        ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(call.clone())).at("style_map")
    })?;
    let given = args.markdown.clone().unwrap_or_default();
    let field = from.or_else(|| editing.written.iter().find(|f| f.field == "from"));
    let in_markdown: Vec<&Written> = editing.written.iter().filter(|f| f.field == "markdown").collect();
    let (markdown, facts) = if field.is_none() && args.markdown.is_some() && !in_markdown.is_empty() {
        with_markdown(&given, &in_markdown)
    } else {
        (given, Vec::new())
    };
    let command = InsertMarkdown {
        markdown: markdown.clone(),
        at: Place::parse(&args.at),
        style_map,
        // docx-agent's tool refuses raw HTML and writes emphasis in its styles
        options: MarkdownOptions::default(),
        policy: editing.policy(),
    };
    // the values written by reference kept in the same step
    let finish = |inserted: &MarkdownInserted, after: &written::After| {
        let mut result = reply(args, inserted.clone());
        let keeps = if editing.written.is_empty() {
            Vec::new()
        } else {
            let made = Made {
                markdown: &markdown,
                field,
                facts: &facts,
                blocks: &inserted.blocks,
            };
            keep_facts(args, editing, &made, after, &mut result)
        };
        (result, keeps)
    };
    match written::run_keeping(editing, &command, finish) {
        Ok((_, result)) => Ok(result),
        Err(error) => Ok(ToolResult::failed(edit_refusal(error, "", call))),
    }
}

/// What the values written by reference are found by: the Markdown written, the `from`
/// field or the values in the Markdown, the blocks made.
struct Made<'a> {
    markdown: &'a str,
    field: Option<&'a Written>,
    facts: &'a [Written],
    blocks: &'a [String],
}

/// The note when the values written by reference could not be lined up with the blocks.
const NOT_LINED_UP: &str =
    "the values written by reference could not be lined up with the blocks made: word_read the new blocks to see which is which";

/// Round 45, R65: the reply's `facts` -- the `from` field, or each value in the Markdown
/// (`facts`, their places in `markdown`, the text written) with the blocks a value
/// standing alone made, kept as their source -- and a note first where they could not be
/// lined up (docx-agent's `_fields_placed`); then every value found where it went
/// (`keep_written`, on the call's own Markdown). The keeps.
fn keep_facts(
    args: &WordInsertMarkdown,
    editing: &Editing<'_>,
    made: &Made<'_>,
    after: &written::After,
    result: &mut ToolResult<Value>,
) -> Vec<TextKeep> {
    let Made {
        markdown,
        field,
        facts,
        blocks,
    } = *made;
    let mut keeps = Vec::new();
    let said = if let Some(field) = field {
        keeps.extend(written::whole_keeps(after, field, blocks));
        Some(vec![written::brief(field, Some(blocks))])
    } else if facts.is_empty() {
        None
    } else {
        let (found, lined_up) = placed_blocks(markdown, facts, blocks);
        let mut said = Vec::new();
        for (k, fact) in facts.iter().enumerate() {
            let made = found.get(&k);
            if let Some(made) = made {
                keeps.extend(written::whole_keeps(after, fact, made));
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
        keeps.extend(written::written_keeps(after, editing, done, &[], &argument));
        if let Some(said) = said {
            let mut data = match done.data.take() {
                Some(Value::Object(data)) => data,
                _ => Map::new(),
            };
            data.insert("facts".into(), Value::Array(said));
            done.data = Some(Value::Object(data));
        }
    }
    keeps
}

/// What of the call is the host's: data it holds, values or places by reference, blocks
/// whose look new ones take.
fn unported(args: &WordInsertMarkdown) -> Option<Unported> {
    let from_data = args.source.is_some()
        || args.blob.is_some()
        || args.from_columns.is_some()
        || args.header.is_some()
        || args.date_format.is_some();
    if from_data {
        return Some(Unported::HostData);
    }
    if args.at.contains('$') {
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
        renamed: ooxml_word_model::ids::renames_text(&out.renamed),
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
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.edit::<WordInsertMarkdown>(word_insert_markdown);
}
