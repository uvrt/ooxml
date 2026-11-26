//! Owns: `replace_text` on a Word document (docx-agent's handler of the shared tool): the
//! matches of the text as written, or of a Python regular expression (`regex`, matched by
//! [`ooxml_tools::pyre`], each replacement its template expanded), found in every story's
//! current text or in the paragraphs `range` holds; refused when there are none or,
//! expecting one, several (the matches listed as the way out); the replacement one
//! [`ReplaceText`] command (one history step), tracked as each match deleted and its
//! replacement inserted; the reply as docx-agent's makes it.
//!
//! Must not: write the document but through the command, plan on a version but the edit's
//! transaction's, or answer a form it does not support yet but by refusing it
//! (`not_supported`): a deck's slides, an empty find, styled text in `replace`, and a
//! body-only call while other stories match too.

use super::{by_reference, merge_renamed, not_supported, text_unported, Unported};
use crate::find_text::{match_context, story_ok};
use crate::planned::Planned;
use crate::refusal::address_refusal;
use crate::review::edit_refusal;
use crate::{written, Editing};
use ooxml_store::Txn;
use ooxml_tools::pyfmt::repr;
use ooxml_tools::pyre::PyRegex;
use ooxml_tools::{ArgEnum, Done, IntOrStr, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::find::{Literal, Pattern};
use ooxml_word_edit::{
    read_pattern, read_ranges, read_scope, EditError, Keep, RangesEdited, ReplaceText as Replace, WordDoc,
};
use ooxml_word_model::address::{scope_paragraphs, Address};
use ooxml_word_model::docx::{Docx, StoryPart};
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
/// listed with its context); `refused` (nothing changed) for what is not supported
/// yet.
#[must_use]
pub fn replace_text(args: &ReplaceText, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &ReplaceText, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    crate::review::stopped(editing.cancel)?;
    if let Some(refusal) = form_not_supported(args) {
        return Err(refusal);
    }
    if args.range.as_deref().is_some_and(by_reference) {
        return Err(Unported::Reference.refusal(replace_call(args), &["range"]));
    }
    if let Some(unported) = replace_unported(args) {
        return Err(unported.refusal(replace_call(args), &["replace"]));
    }
    // the pattern is the call's alone: refused before any version is read
    let pattern = pattern_of(args)?;
    let argument = |field: &str| (field == "replace").then(|| args.replace.clone());
    // the values written by reference kept in the same step
    let finish = |out: &Result<(RangesEdited, Vec<Match>), ToolError>, after: &written::After| match out {
        Ok((out, matches)) => {
            let result = replied(out, matches);
            let keeps = written::reply_keeps(after, editing, &result, &[], &argument);
            (result, keeps)
        }
        Err(refusal) => (ToolResult::failed(refusal.clone()), Vec::new()),
    };
    let (_, result) = written::run_keeping(editing, &Planned(replace_plan(args, &pattern, editing)), finish)
        .map_err(|e| edit_refusal(e, "", find_call(args)))?;
    Ok(result)
}

/// The replace's plan, made in the edit's transaction ([`crate::planned`]): the scope `range`
/// names (read through the version's renames), the matches and the refusals they make
/// (none, more than one where one is expected, others in stories the call leaves) found on
/// the version the edit is applied to, the paragraphs that hold a match and those of the
/// scope read there -- so a match made or taken away, or a block put into the scope or
/// taken out of it meanwhile, is a conflict, and the plan is made again.
fn replace_plan<'a>(
    args: &'a ReplaceText,
    pattern: &'a Arc<dyn Pattern>,
    editing: &'a Editing<'a>,
) -> impl Fn(&mut Txn<'_, WordDoc>) -> Result<(Replace, Vec<Match>), ToolError> + 'a {
    move |tx| {
        let internal = |e: EditError| ToolError::internal(e.to_string(), "word:model");
        let docx = tx.doc().docx().map_err(internal)?;
        read_pattern(tx, Arc::clone(pattern));
        let within = range_of(args).map(|range| Address::from(tx.doc().ids().follow_address(range)));
        let scope = match &within {
            Some(within) => {
                let mut call = replace_call(args);
                call.arguments.remove("range");
                let scope = scope_paragraphs(&docx, within).map_err(|e| address_refusal(e, "range", call))?;
                read_scope(tx, within).map_err(internal)?;
                Some(scope)
            }
            None => None,
        };
        let (matches, all) = found(&docx, args, pattern.as_ref(), scope)?;
        read_ranges(tx, matches.iter().map(|(_, address, _)| address.as_str())).map_err(internal)?;
        let find = repr(&Value::from(args.find.as_str()));
        let inside = range_of(args).map(|r| format!(" in {r}")).unwrap_or_default();
        if matches.is_empty() && all > 0 {
            // every match is in a story the call leaves: the call that covers them
            let mut call = replace_call(args);
            call.arguments.insert("stories".into(), Value::from("all"));
            return Err(ToolError::refusal(
                Refusal::NotFound,
                format!("{find} occurs nowhere in the body{inside}; {all} place(s) in other stories match: stories \"all\" replaces them"),
                WayOut::call(call),
            ));
        }
        if matches.is_empty() {
            return Err(ToolError::refusal(
                Refusal::NotFound,
                format!("{find} occurs nowhere{inside}"),
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
            let mut call = replace_call(args);
            call.arguments.insert("stories".into(), Value::from("all"));
            return Err(not_supported(
                "replace_text in the body only while other stories match",
                call,
                &[],
            ));
        }
        let command = Replace {
            pattern: Arc::clone(pattern),
            across: false,
            within,
            count: (args.expect == Expect::One).then_some(1),
            keep: Keep::First,
            policy: editing.policy(),
        };
        Ok((command, matches))
    }
}

/// The range the call names, if any (an empty one is none: rule 1's placeholder).
fn range_of(args: &ReplaceText) -> Option<&str> {
    args.range.as_deref().filter(|r| !r.is_empty())
}

/// What replaces each match: the text as written, or with `regex` a Python pattern whose
/// replacement is a template (`\1`, `\g<name>`), each refused, at its field, where it is not
/// matched or expanded here as Python's `re` would.
fn pattern_of(args: &ReplaceText) -> Result<Arc<dyn Pattern>, ToolError> {
    if args.regex != Some(true) {
        let literal =
            Literal::new(&args.find, true, false, &args.replace).map_err(|e| edit_refusal(e, "", find_call(args)))?;
        return Ok(Arc::new(literal));
    }
    let regex = PyRegex::new(&args.find, false).map_err(|why| regex_refusal(args, "find", &why.0))?;
    // the template read once, whatever the text holds (no pattern here matches "")
    regex
        .find_expanded("", &args.replace)
        .map_err(|why| regex_refusal(args, "replace", &why.0))?;
    Ok(Arc::new(PythonPattern {
        regex,
        template: args.replace.clone(),
    }))
}

/// A Python regular expression and the template each match's replacement is expanded from
/// (`match.expand`).
#[derive(Debug)]
struct PythonPattern {
    regex: PyRegex,
    template: String,
}

impl Pattern for PythonPattern {
    fn matches(&self, text: &str) -> Result<Vec<(usize, usize, String)>, String> {
        self.regex.find_expanded(text, &self.template).map_err(|why| why.0)
    }
}

/// `regex` refused at `field` for `why`, with the call that finds the text as written.
fn regex_refusal(args: &ReplaceText, field: &str, why: &str) -> ToolError {
    let what = if field == "replace" { "replacement" } else { "pattern" };
    ToolError::refusal(
        Refusal::Refused,
        format!("regex: this {what} is not matched here as Python matches it ({why}); find the text as it is written"),
        WayOut::call(replace_call(args)),
    )
    .at(field)
}

/// The reply of a replace that made `out` at `matches`.
fn replied(out: &RangesEdited, matches: &[Match]) -> ToolResult<Value> {
    let count = out.count;
    let mut paragraphs: Vec<String> = Vec::new();
    for (id, ..) in matches {
        if !paragraphs.contains(id) {
            paragraphs.push(id.clone());
        }
    }
    let mut reply = Done::new(format!("Replaced {count} occurrence(s)"));
    merge_renamed(&mut reply.renamed, &ooxml_word_model::ids::renames_text(&out.renamed));
    // the paragraphs by the ids they have now (a match's paragraph may have been stamped)
    let now: Vec<String> = paragraphs
        .iter()
        .map(|p| {
            reply
                .renamed
                .iter()
                .find(|(old, _)| old == p)
                .map_or_else(|| p.clone(), |(_, new)| new.clone())
        })
        .collect();
    let listed: Vec<&String> = now.iter().take(LISTED).collect();
    reply.data = Some(json!({"count": count, "paragraphs": listed}));
    reply.changed = now;
    ToolResult::done(reply)
}

/// A match: its paragraph's id, its range's address, its context.
type Match = (String, String, String);

/// The matches in the stories the call covers (in `scope`, the paragraphs its range holds,
/// when given), in document order, and how many there are in every story (in the scope).
///
/// # Errors
/// The pattern's refusal: a search that gives up.
fn found(
    docx: &Docx,
    args: &ReplaceText,
    pattern: &dyn Pattern,
    scope: Option<(&StoryPart, Vec<usize>)>,
) -> Result<(Vec<Match>, usize), ToolError> {
    let every = args.stories == Some(ReplaceStories::All);
    let scopes: Vec<(&StoryPart, Vec<usize>)> = match scope {
        Some(scope) => vec![scope],
        None => docx
            .parts
            .iter()
            .map(|p| (&**p, (0..p.ids.paragraphs().len()).collect()))
            .collect(),
    };
    let mut all = 0;
    let mut matches = Vec::new();
    for (part, places) in scopes {
        let texts = part.texts();
        for k in places {
            let Some(line) = texts.get(k).filter(|line| pattern.may_match(line)) else {
                continue;
            };
            let Some(entry) = part.ids.paragraphs().get(k) else {
                continue;
            };
            let text: Vec<char> = line.chars().collect();
            let found = pattern.matches(line).map_err(|why| regex_refusal(args, "find", &why))?;
            for (start, end, _) in found {
                all += 1;
                let address = format!("{}@{start}:{end}", entry.id);
                if every || story_ok(&address) {
                    let context = match_context(&text, start, end);
                    matches.push((entry.id.clone(), address, context));
                }
            }
        }
    }
    Ok((matches, all))
}

/// What of `replace` is not supported yet: styled text. With `regex`, a backslash is the
/// template's (a group, an escape), not an escape of the text.
fn replace_unported(args: &ReplaceText) -> Option<Unported> {
    if args.regex == Some(true) {
        return text_unported(&args.replace.replace('\\', ""));
    }
    text_unported(&args.replace)
}

/// The forms of `args` not supported yet, refused with the call without them: a deck's
/// slides, an empty `find`.
fn form_not_supported(args: &ReplaceText) -> Option<ToolError> {
    let (form, fill) = if args.slides.is_some() {
        ("replace_text slides", &[][..])
    } else if args.find.is_empty() {
        ("replace_text empty find", &["find"][..])
    } else {
        return None;
    };
    Some(not_supported(form, replace_call(args), fill))
}

/// `replace_text` of `args`' text and replacement, in the range and stories it names,
/// without the forms not supported yet (and without `regex`: the text as written).
fn replace_call(args: &ReplaceText) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    arguments.insert("find".into(), Value::from(args.find.as_str()));
    arguments.insert("replace".into(), Value::from(args.replace.as_str()));
    if args.expect == Expect::All {
        arguments.insert("expect".into(), Value::from("all"));
    }
    if let Some(range) = range_of(args) {
        arguments.insert("range".into(), Value::from(range));
    }
    if args.stories == Some(ReplaceStories::All) {
        arguments.insert("stories".into(), Value::from("all"));
    }
    TryCall {
        tool: "replace_text".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

/// `find_text` of the same text, in the same range: where it occurs.
fn find_call(args: &ReplaceText) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    arguments.insert("text".into(), Value::from(args.find.as_str()));
    if args.regex == Some(true) {
        arguments.insert("regex".into(), Value::from(true));
    }
    if let Some(range) = range_of(args) {
        arguments.insert("range".into(), Value::from(range));
    }
    if args.stories == Some(ReplaceStories::All) {
        arguments.insert("stories".into(), Value::from("all"));
    }
    TryCall {
        tool: "find_text".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

/// Registers `replace_text` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.edit::<ReplaceText>(replace_text);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::planned::held::{args, calling, compare, markdown, marked, paragraph, raced, shape};
    use ooxml_store::Caller;
    use ooxml_word_edit::Editor;

    /// The fixture with three paragraphs at its end, each holding 2026 (and Charlie a date).
    fn years() -> Editor {
        let editor = marked();
        let text = "Alpha 2026 here.\n\nBravo 2026 and 2026.\n\nCharlie 2026-11 and 2025-03.";
        editor.run(&Caller::anonymous(), &markdown("end", text)).unwrap();
        editor
    }

    /// The text of the body's paragraph that starts with `start` (the body as [`shape`]
    /// gives it: the tests set no formatting).
    fn text_of(editor: &Editor, start: &str) -> String {
        let body = shape(editor);
        body.lines().find(|t| t.starts_with(start)).unwrap().to_owned()
    }

    fn done(result: &ToolResult<Value>) -> Value {
        match &result.outcome {
            ooxml_tools::Outcome::Done(done) => done.data.clone().unwrap(),
            _ => panic!("{:?}", result.to_json()),
        }
    }

    fn failed(result: &ToolResult<Value>) -> ToolError {
        match &result.outcome {
            ooxml_tools::Outcome::Failed(error) => error.clone(),
            _ => panic!("{:?}", result.to_json()),
        }
    }

    #[test]
    fn a_range_replaces_only_what_it_holds() {
        let editor = years();
        let span = format!("{}..{}", paragraph(&editor, "Alpha"), paragraph(&editor, "Bravo"));
        let args: ReplaceText = args(json!({"find": "2026", "replace": "2027", "expect": "all", "range": span}));
        let data = done(&calling(&editor, |editing| replace_text(&args, editing)));
        assert_eq!(data["count"], 3);
        assert_eq!(text_of(&editor, "Alpha"), "Alpha 2027 here.");
        assert_eq!(text_of(&editor, "Bravo"), "Bravo 2027 and 2027.");
        assert_eq!(text_of(&editor, "Charlie"), "Charlie 2026-11 and 2025-03.");
    }

    #[test]
    fn expect_one_counts_the_range_alone() {
        // 2026 occurs in three paragraphs, once in Alpha: one match there
        let editor = years();
        let range = paragraph(&editor, "Alpha");
        let args: ReplaceText = args(json!({"find": "2026", "replace": "2027", "range": range}));
        assert_eq!(
            done(&calling(&editor, |editing| replace_text(&args, editing)))["count"],
            1
        );
        let args: ReplaceText = args_of(&editor, "Bravo", json!({"find": "2026", "replace": "x"}));
        let refused = failed(&calling(&editor, |editing| replace_text(&args, editing)));
        assert_eq!(refused.code().as_str(), "ambiguous", "{refused:?}");
    }

    fn args_of(editor: &Editor, start: &str, mut value: Value) -> ReplaceText {
        value["range"] = Value::from(paragraph(editor, start));
        args(value)
    }

    #[test]
    fn a_regex_expands_its_groups_where_it_matches() {
        let editor = years();
        let args = args_of(
            &editor,
            "Charlie",
            json!({"find": r"(\d{4})-(?P<month>\d{2})", "replace": r"\g<month>/\1", "expect": "all", "regex": true}),
        );
        assert_eq!(
            done(&calling(&editor, |editing| replace_text(&args, editing)))["count"],
            2
        );
        assert_eq!(text_of(&editor, "Charlie"), "Charlie 11/2026 and 03/2025.");
        assert_eq!(text_of(&editor, "Alpha"), "Alpha 2026 here.");
    }

    #[test]
    fn a_pattern_or_template_python_would_not_read_alike_is_refused_at_its_field() {
        let editor = years();
        for (find, replace, field) in [("a*", "x", "find"), (r"(\d+)", r"\2", "replace")] {
            let args: ReplaceText = args(json!({"find": find, "replace": replace, "regex": true}));
            let refused = failed(&calling(&editor, |editing| replace_text(&args, editing)));
            assert_eq!(refused.field(), Some(field), "{refused:?}");
            assert!(refused.message().starts_with("regex:"), "{refused:?}");
        }
    }

    #[test]
    fn a_range_naming_nothing_is_refused_with_the_call_without_it() {
        let editor = years();
        let args: ReplaceText = args(json!({"find": "2026", "replace": "x", "range": "p:0BADF00D"}));
        let refused = failed(&calling(&editor, |editing| replace_text(&args, editing)));
        assert_eq!(refused.code().as_str(), "not_found", "{refused:?}");
        assert!(refused.message().contains("p:0BADF00D"), "{refused:?}");
        assert_eq!(refused.field(), Some("range"));
    }

    #[test]
    fn a_match_put_into_the_range_meanwhile_is_replaced_too() {
        // the race's plan saw three matches in the span; the paragraph put into it meanwhile
        // (beside Alpha, a block the scope reads) is a conflict, and the plan made again
        // replaces its match as the call made after it does (both editors are made alike, so
        // the span's ids are the same in each)
        let made = years();
        let span = format!("{}..{}", paragraph(&made, "Alpha"), paragraph(&made, "Bravo"));
        let args: ReplaceText = args(json!({"find": "2026", "replace": "2027", "expect": "all", "range": span}));
        let pattern = pattern_of(&args).unwrap();
        let compared = compare(
            years,
            |editor| markdown(&format!("after:{}", paragraph(editor, "Alpha")), "Inside 2026 too."),
            |editor, between| {
                calling(editor, |editing| {
                    raced(editor, replace_plan(&args, &pattern, editing), between)
                })
            },
            |editing| replace_text(&args, editing),
        );
        compared.assert_same_edit();
        assert!(compared.serial.contains("Inside 2027 too."), "{}", compared.serial);
    }

    #[test]
    fn exactly_one_match_is_checked_where_the_replace_commits() {
        // a second match put meanwhile: the replace, prepared again, refuses as ambiguous,
        // as it does called after it -- not the first of the two replaced
        let args: ReplaceText = args(json!({"find": "this year", "replace": "last year"}));
        let compared = compare(
            marked,
            |_| markdown("end", "Also this year."),
            |editor, between| {
                calling(editor, |editing| {
                    raced(
                        editor,
                        replace_plan(&args, &pattern_of(&args).unwrap(), editing),
                        between,
                    )
                })
            },
            |editing| replace_text(&args, editing),
        );
        compared.assert_same_refusal();
        assert!(compared.serial.contains("14% this year."), "{}", compared.serial);
    }
}
