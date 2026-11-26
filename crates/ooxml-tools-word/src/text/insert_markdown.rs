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
//! List items take the look of the paragraph `like` gives for their level, or a style map
//! names for a list role (docx-agent's `_levels_like`: each sample one paragraph, read on the
//! version the edit is applied to; a style map's paragraph read as like, with a note).
//!
//! Refused as not supported yet (nothing changed): rows or a field as the content (`from`),
//! a blob (`blob`), a place or sample given by reference (`$name`; braces reaching the tool
//! are literal text, the host having written its references as their values), tables in a
//! given table's frame (`like` naming a table), and what the command does not write yet
//! (pictures, footnotes).
//!
//! Must not: write the document but through the command, or answer a call it does not
//! support yet, refused ([`Unported`]).

use super::fields::{placed_blocks, with_markdown};
use super::Unported;
use crate::planned::Planned;
use crate::review::{edit_refusal, Editing};
use crate::structure::common::{blocks_ref_as_place, is_place, name_blocks, outcome, Outcome};
use crate::style::sample_look;
use crate::written::{self, Written};
use ooxml_store::Txn;
use ooxml_tools::{ArgEnum, Outcome as Replied, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{
    EditError, InsertMarkdown, LevelLook, ListKind, MarkdownInserted, MarkdownOptions, TextKeep, WordDoc, WriteMap,
};
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
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &WordInsertMarkdown, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
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
    let given = args.markdown.clone().unwrap_or_default();
    let field = editing.written.iter().find(|f| f.field == "from");
    let in_markdown: Vec<&Written> = editing.written.iter().filter(|f| f.field == "markdown").collect();
    let (markdown, facts) = if field.is_none() && args.markdown.is_some() && !in_markdown.is_empty() {
        with_markdown(&given, &in_markdown)
    } else {
        (given, Vec::new())
    };
    // the values written by reference kept in the same step
    let finish = |planned: &Result<(MarkdownInserted, Vec<String>), ToolError>, after: &written::After| {
        let (inserted, notes) = match planned {
            Ok(planned) => planned,
            Err(refusal) => return (ToolResult::failed(refusal.clone()), Vec::new()),
        };
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
        // what the call was read as comes first
        result.warnings.splice(0..0, notes.iter().cloned());
        (result, keeps)
    };
    let done = written::run_keeping(
        editing,
        &Planned(markdown_plan(args, &markdown, editing, &call)),
        finish,
    );
    match done {
        Ok((_, result)) => Ok(result),
        Err(error) => Ok(ToolResult::failed(edit_refusal(error, "", call))),
    }
}

/// The insertion's plan, made in the edit's transaction ([`crate::planned`]): the looks
/// `like` gives each level and the style map's list roles naming a paragraph give theirs
/// (docx-agent's `_levels_like`), each sample read on the version the edit is applied to
/// (a change to it meanwhile is a conflict), then the command; what the call was read as.
fn markdown_plan<'a, 'e: 'a>(
    args: &'a WordInsertMarkdown,
    markdown: &'a str,
    editing: &'a Editing<'e>,
    call: &'a TryCall,
) -> impl Fn(&mut Txn<'_, WordDoc>) -> Result<(InsertMarkdown, Vec<String>), ToolError> + 'a {
    move |tx| {
        let internal = |e: EditError| ToolError::internal(e.to_string(), "word:model");
        let docx = tx.doc().docx().map_err(internal)?;
        let mut like = Vec::new();
        for (level, sample) in (1_i64..).zip(args.like.iter().flatten()) {
            // an empty entry: no look for its level
            if !sample.is_empty() {
                let look = sample_look(tx, &docx, sample, call)?;
                like.push(LevelLook {
                    kind: None,
                    level,
                    look,
                });
            }
        }
        let mut notes = Vec::new();
        let mut pairs: Vec<(String, Option<String>)> = Vec::new();
        for entry in args.style_map.iter().flatten() {
            let element = entry.element.as_str();
            let style = &entry.style;
            match role(entry.element) {
                Some((kind, level)) if is_place(style) && docx.find_style(style, "paragraph").is_none() => {
                    let look = sample_look(tx, &docx, style, call)?;
                    like.push(LevelLook {
                        kind: Some(kind),
                        level,
                        look,
                    });
                    notes.push(format!(
                        "style_map {element}: {style} is a paragraph: read as like, its items looking like it"
                    ));
                }
                _ => pairs.push((element.to_owned(), Some(style.clone()))),
            }
        }
        if let Some(over) = like.iter().find(|l| !(1..=LIKE_LEVELS).contains(&l.level)) {
            let message = format!("like: level {} is 1 to {LIKE_LEVELS}", over.level);
            return Err(ToolError::refusal(Refusal::Refused, message, WayOut::call(call.clone())).at("like"));
        }
        let style_map = WriteMap::from_pairs(&pairs).map_err(|message| {
            ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(call.clone())).at("style_map")
        })?;
        let command = InsertMarkdown {
            markdown: markdown.to_owned(),
            at: Place::parse(&args.at),
            style_map,
            // docx-agent's tool refuses raw HTML and writes emphasis in its styles
            options: MarkdownOptions {
                like,
                ..MarkdownOptions::default()
            },
            policy: editing.policy(),
        };
        Ok((command, notes))
    }
}

/// The list levels a look may be given for (1 the top).
const LIKE_LEVELS: i64 = 9;

/// The list items a style map's construct is (docx-agent's `ROLES`): their kind and level.
fn role(element: StyleElement) -> Option<(ListKind, i64)> {
    Some(match element {
        StyleElement::Bullet => (ListKind::Bullet, 1),
        StyleElement::Bullet2 => (ListKind::Bullet, 2),
        StyleElement::Bullet3 => (ListKind::Bullet, 3),
        StyleElement::Number => (ListKind::Number, 1),
        StyleElement::Number2 => (ListKind::Number, 2),
        StyleElement::Number3 => (ListKind::Number, 3),
        _ => return None,
    })
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

/// What of the call is the host's: data it holds, values, places or samples by reference,
/// a table whose frame new tables take.
fn unported(args: &WordInsertMarkdown) -> Option<Unported> {
    let from_data = args.source.is_some()
        || args.blob.is_some()
        || args.from_columns.is_some()
        || args.header.is_some()
        || args.date_format.is_some();
    if from_data {
        return Some(Unported::HostData);
    }
    let samples = || args.like.iter().flatten().filter(|s| !s.is_empty());
    if args.at.contains('$') || samples().any(|s| s.starts_with('$')) {
        return Some(Unported::Reference);
    }
    // a table's frame (docx-agent's `_table_like`: a table's id, not a cell's or a span's)
    let table = |s: &str| s.rsplit('/').next().is_some_and(|last| last.starts_with("t:")) && !s.contains("..");
    samples().any(|s| table(s)).then_some(Unported::Like)
}

/// How many paragraphs' kept settings a replacement lists; the rest are counted.
const KEPT_CAP: usize = 30;

/// The reply, as docx-agent's `outcome` and `name_blocks` make it of the insertion (at
/// `replace:`, what the new paragraphs kept of the replaced ones: `data.kept`).
fn reply(args: &WordInsertMarkdown, out: MarkdownInserted) -> ToolResult<Value> {
    let edit = Outcome {
        id: out.first,
        changed: out.changed,
        created: out.created,
        removed: out.removed,
        renamed: ooxml_word_model::ids::renames_text(&out.renamed),
        warnings: out.warnings,
    };
    let mut summary = format!("Inserted {} block(s)", out.blocks.len());
    let mut data = Map::new();
    data.insert("blocks".into(), json!(out.blocks));
    if !out.kept.is_empty() {
        // what the new paragraphs kept of the replaced ones (rule 4: capped, the rest counted)
        summary.push_str(&format!("; {} kept the replaced paragraphs' settings", out.kept.len()));
        let listed: Vec<Value> = out
            .kept
            .iter()
            .take(KEPT_CAP)
            .map(|k| json!({"block": k.block, "from": k.from, "settings": k.settings}))
            .collect();
        data.insert("kept".into(), Value::Array(listed));
        if out.kept.len() > KEPT_CAP {
            data.insert("kept_left_out".into(), Value::from(out.kept.len() - KEPT_CAP));
        }
    }
    let mut result = outcome(&[edit], summary, Some(Value::Object(data)));
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
