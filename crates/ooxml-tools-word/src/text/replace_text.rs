//! Owns: `replace_text` on a Word document (docx-agent's handler of the shared tool): the
//! matches found in every story's current text, refused when there are none or, expecting
//! one, several (the matches listed as the way out); the replacement one
//! [`ReplaceText`] command (one history step); the reply as docx-agent's makes it.
//!
//! Must not: write the document but through the command, or answer a call it does not
//! port otherwise than by saying so ([`Unported`]): a regex, a range, a deck's slides, and
//! a body-only call while other stories match too (docx-agent replaces those paragraph by
//! paragraph) are the host's still.

use super::{merge_renamed, text_unported, Unported};
use crate::find_text::{literal_matches, match_context, story_ok};
use crate::review::edit_refusal;
use crate::Editing;
use ooxml_tools::pyfmt::repr;
use ooxml_tools::{ArgEnum, Done, IntOrStr, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::find::Literal;
use ooxml_word_edit::{Keep, ReplaceText as Replace};
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::docx::Docx;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// How many matches a call expects.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Exactly one.
    One,
    /// Every one.
    All,
}

/// Which stories a search covers.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceStories {
    /// The body.
    Body,
    /// Every story.
    All,
    /// A deck's layouts.
    Layouts,
    /// A deck's masters.
    Masters,
}

/// `replace_text`'s arguments, shared by every format.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(name = "replace_text", description = include_str!("replace_text.md"), mutates)]
pub struct ReplaceText {
    /// Document id.
    pub doc: String,
    /// Text to replace.
    pub find: String,
    /// Replacement; with regex, \1 is a group.
    #[arg(facts)]
    pub replace: String,
    /// Matches expected.
    #[arg(default = "one")]
    pub expect: Expect,
    /// Decks: slide numbers from 1, or ids (s:256, $ref). Default all.
    #[arg(kinds("pptx"), refs)]
    pub slides: Option<Vec<IntOrStr<1>>>,
    /// Documents: only within this block or span (p:A..t:B).
    #[arg(kinds("docx"))]
    pub range: Option<String>,
    /// Documents: body (default) or all stories. Decks: body is the slides; layouts, masters, or all.
    #[arg(
        kinds("pptx", "docx"),
        kind_value("layouts", "pptx"),
        kind_value("masters", "pptx"),
        kind_description("pptx", "Body (default): the slides; layouts, masters, or all."),
        kind_description("docx", "Body (default) or all stories.")
    )]
    pub stories: Option<ReplaceStories>,
    /// find is a Python regex. Default false.
    pub regex: Option<bool>,
}

/// Matches a refusal lists.
const LISTED: usize = 50;

/// `replace_text` on the Word document `editing` names. A failed reply is `not_found`
/// when the text occurs nowhere, `ambiguous` when one is expected and several match (each
/// listed with its context); `unported` (nothing changed) for what the Rust core does not
/// do yet.
#[must_use]
pub fn replace_text(args: &ReplaceText, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &ReplaceText, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    crate::review::stopped(editing.cancel)?;
    if args.regex == Some(true) || args.range.is_some() || args.slides.is_some() || args.find.is_empty() {
        return Ok(Unported::Form.reply());
    }
    if let Some(unported) = text_unported(&args.replace) {
        return Ok(unported.reply());
    }
    let head = editing.editor.store().snapshot();
    let docx = head
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let (matches, all) = found(&docx, args);
    let find = repr(&Value::from(args.find.as_str()));
    if matches.is_empty() {
        return Err(ToolError::refusal(
            Refusal::NotFound,
            format!("{find} occurs nowhere"),
            WayOut::call(find_call(args)),
        ));
    }
    if args.expect == Expect::One && matches.len() > 1 {
        let options = matches
            .iter()
            .take(LISTED)
            .map(|(_, address, context)| Value::from(format!("{address} {}", repr(&Value::from(context.as_str())))))
            .collect();
        let way = WayOut::options(options).unwrap_or_else(|| WayOut::call(find_call(args)));
        return Err(ToolError::refusal(
            Refusal::Ambiguous,
            format!(
                "{} places match {find}; narrow it with range or more words",
                matches.len()
            ),
            way,
        ));
    }
    if args.stories != Some(ReplaceStories::All) && matches.len() != all {
        return Ok(Unported::Form.reply());
    }
    let pattern = Literal::new(&args.find, true, false, &args.replace)
        .map_err(|e| edit_refusal(e, "find", find_call(args)))?;
    let command = Replace {
        pattern: Arc::new(pattern),
        across: false,
        count: (args.expect == Expect::One).then_some(1),
        keep: Keep::First,
        policy: editing.policy(),
    };
    let done = editing
        .editor
        .run(editing.caller, &command)
        .map_err(|e| edit_refusal(e, "find", find_call(args)))?;
    let count = done.out.count;
    let mut paragraphs: Vec<String> = Vec::new();
    for (id, ..) in &matches {
        if !paragraphs.contains(id) {
            paragraphs.push(id.clone());
        }
    }
    let mut reply = Done::new(format!("Replaced {count} occurrence(s)"));
    merge_renamed(&mut reply.renamed, &done.out.renamed);
    let listed: Vec<&String> = paragraphs.iter().take(LISTED).collect();
    reply.data = Some(json!({"count": count, "paragraphs": listed}));
    reply.changed = paragraphs
        .iter()
        .map(|p| {
            reply
                .renamed
                .iter()
                .find(|(old, _)| old == p)
                .map_or_else(|| p.clone(), |(_, new)| new.clone())
        })
        .collect();
    Ok(ToolResult::done(reply))
}

/// A match: its paragraph's id, its range's address, its context.
type Match = (String, String, String);

/// The matches in the stories the call covers, in document order, and how many there are
/// in every story.
fn found(docx: &Docx, args: &ReplaceText) -> (Vec<Match>, usize) {
    let needle: Vec<char> = args.find.chars().collect();
    let every = args.stories == Some(ReplaceStories::All);
    let mut all = 0;
    let mut matches = Vec::new();
    for part in &docx.parts {
        for entry in part.ids.paragraphs().iter() {
            let text: Vec<char> = paragraph_text(&entry.element, View::Current).chars().collect();
            for start in literal_matches(&text, &needle) {
                all += 1;
                let address = format!("{}@{start}:{}", entry.id, start + needle.len());
                if every || story_ok(&address) {
                    let context = match_context(&text, start, start + needle.len());
                    matches.push((entry.id.clone(), address, context));
                }
            }
        }
    }
    (matches, all)
}

/// `find_text` of the same text: where it occurs.
fn find_call(args: &ReplaceText) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    arguments.insert("text".into(), Value::from(args.find.as_str()));
    if args.stories == Some(ReplaceStories::All) {
        arguments.insert("stories".into(), Value::from("all"));
    }
    TryCall {
        tool: "find_text".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}
