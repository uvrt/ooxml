//! Owns: `word_insert_markdown` (docx-agent's `word_insert_markdown`): its definition and
//! its Markdown written as the document's own blocks through the edit core's
//! [`InsertMarkdown`] (one history step), the reply naming the blocks made (`data.blocks`),
//! each named by the call's `ref`.
//!
//! Left to the host (`unported`, nothing changed): rows or a field as the content (`from`),
//! a blob (`blob`), values written by reference (`{{...}}`) and a place given by reference
//! (`$name`), list items or tables like given blocks (`like`, a style map naming a
//! paragraph), and what the command does not write yet (pictures, footnotes, `replace:`).
//!
//! Must not: write the document but through the command, or answer a call it does not
//! port otherwise than by saying so ([`Unported`]).

use super::Unported;
use crate::review::{edit_refusal, Editing};
use crate::structure::common::{blocks_ref_as_place, is_place, name_blocks, outcome, refused, Outcome};
use ooxml_tools::{ArgEnum, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{InsertMarkdown, MarkdownInserted, MarkdownOptions, WriteMap};
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
    if let Some(unported) = unported(args) {
        return Ok(unported.reply());
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
    let command = InsertMarkdown {
        markdown: args.markdown.clone().unwrap_or_default(),
        at: args.at.clone(),
        style_map,
        // docx-agent's tool refuses raw HTML and writes emphasis in its styles
        options: MarkdownOptions::default(),
        policy: editing.policy(),
    };
    Ok(match editing.editor.run(editing.caller, &command) {
        Ok(done) => reply(args, done.out),
        Err(error @ ooxml_word_edit::EditError::Unsupported(_)) => refused(error, call),
        Err(error) => ToolResult::failed(edit_refusal(error, "", call)),
    })
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
    r.edit::<WordInsertMarkdown>(word_insert_markdown, crate::Hosting::SERVED);
}
