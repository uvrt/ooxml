//! Owns: `word_format`'s handler (docx-agent's `tools/text.py`, `word_format`): the call's
//! settings read as docx-agent splits them (run and paragraph settings in its order, a
//! negative first-line indent a hanging one, `clear` as settings to remove), its refusals
//! (nothing to format or set, a value both set and cleared, a style no document or Word
//! has, a `like` of several paragraphs, `find` items not found once), the edits through the
//! edit core's [`FormatEdit`], and the reply: what changed, the found ranges, what the
//! look did, the outline's moves.
//!
//! Must not: change the document but through a command.

use super::find::found_ranges;
use super::format_args::{Cleared, WordFormat};
use super::moves::moves;
use crate::planned::run_planned;
use crate::review::{edit_refusal, Editing};
use crate::structure::common::{call, outcome, policy, Outcome};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::Txn;
use ooxml_tools::{ArgEnum, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{
    keeps_blocks, known_style, paragraphs_of, read_matching, read_ranges, EditError, FormatEdit, FormatPlan,
    FormatStep, Formatted, Key, Like, ParagraphLook, Property, Setting, Value as Set, WordDoc, KEEPS_BLOCKS, NOT_TAKEN,
};
use ooxml_word_model::address::Address;
use ooxml_word_model::docx::{style_kind, style_name, Docx};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

impl From<FormatStep> for Outcome {
    fn from(step: FormatStep) -> Self {
        Outcome {
            id: Some(step.id.to_string()),
            changed: step.changed,
            created: Vec::new(),
            removed: step.removed,
            renamed: ooxml_word_model::ids::renames_text(&step.renamed),
            warnings: step.warnings,
        }
    }
}

const SETTERS: [&str; 24] = [
    "like",
    "paragraph_style",
    "character_style",
    "bold",
    "italic",
    "underline",
    "strike",
    "size",
    "font",
    "color",
    "highlight",
    "alignment",
    "space_before",
    "space_after",
    "line_spacing",
    "indent_left",
    "indent_right",
    "indent_first",
    "keep_with_next",
    "keep_lines",
    "page_break_before",
    "keep_together",
    "clear_direct",
    "clear",
];

/// `word_format` (docx-agent's): styles, run and paragraph formatting on each target.
#[must_use]
pub fn word_format(args: &WordFormat, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let again = call("word_format", &[("doc", json!(args.doc))]);
    run(args, editing, &again).unwrap_or_else(ToolResult::failed)
}

fn invalid(message: String, field: &str, options: &[&str], again: &TryCall) -> ToolError {
    let way = WayOut::names(options.iter().map(|s| (*s).to_owned())).unwrap_or_else(|| WayOut::call(again.clone()));
    ToolError::refusal(Refusal::InvalidArguments, message, way).at(field)
}

fn run(args: &WordFormat, editing: &Editing<'_>, again: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let targets = args.targets.clone().unwrap_or_default();
    let finds = args.find.clone().unwrap_or_default();
    if targets.is_empty() && finds.is_empty() {
        if args.targets.is_none() && args.find.is_none() {
            let message = "say what to format: targets (ids) or find (text)".to_owned();
            return Err(invalid(message, "targets", &["targets", "find"], again));
        }
        return Ok(ToolResult::nothing_given("formatted", "targets"));
    }
    let given = given(args);
    let clear = args.clear.clone().unwrap_or_default();
    let both: Vec<&str> = clear
        .iter()
        .map(|c| ArgEnum::as_str(*c))
        .filter(|c| given.contains(c))
        .collect();
    if !both.is_empty() {
        let message = format!(
            "{} both set and cleared: give a value or clear it, not both",
            both.join(", ")
        );
        return Err(invalid(message, "clear", &[], again));
    }
    let (mut run, mut paragraph) = split(args);
    for name in &clear {
        cleared(*name, &mut run, &mut paragraph);
    }
    let plan = format_plan(args, (run, paragraph, clear), editing, again);
    let done = run_planned(editing, plan)?.map_err(|e| {
        // docx-agent names `targets` for a target holding no paragraphs or no blocks to keep
        let field = match &e {
            EditError::Address(a)
                if a.message.ends_with("holds no paragraphs") || a.message.ends_with(KEEPS_BLOCKS) =>
            {
                "targets"
            }
            _ => "",
        };
        edit_refusal(e, field, again.clone())
    })?;
    let (out, (count, found, look)) = done.out;
    Ok(reply(count, &found, look.as_ref(), like_of(args), out))
}

/// `like`, where it names something.
fn like_of(args: &WordFormat) -> Option<&str> {
    args.like.as_deref().filter(|l| !l.is_empty())
}

/// What the reply says of a format's plan: how many targets, the ranges `find` found, the
/// sample's look.
type Said = (usize, Vec<String>, Option<ParagraphLook>);

/// `word_format`'s plan, made in the edit's transaction ([`crate::planned`]) after the
/// checks of the arguments alone: the `like` sample's look and the ranges `find` names,
/// read on the version the edit is applied to -- the sample by key, each find by the
/// paragraphs that hold a match and those of what it found -- then the edit.
fn format_plan<'a>(
    args: &'a WordFormat,
    (run, paragraph, clear): (Vec<Setting>, Vec<Setting>, Vec<Cleared>),
    editing: &'a Editing<'a>,
    again: &'a TryCall,
) -> impl Fn(&mut Txn<'_, WordDoc>) -> Result<(FormatEdit, Said), ToolError> + 'a {
    move |tx| {
        let internal = |e: EditError| ToolError::internal(e.to_string(), "word:model");
        let docx = tx.doc().docx().map_err(internal)?;
        let like = like_of(args);
        let look = like.map(|l| sample_look(tx, &docx, l, again)).transpose()?;
        if !sets_anything(args, &run, &paragraph, look.is_some()) {
            let message = "say what to set: a style, a run or a paragraph property".to_owned();
            return Err(invalid(message, "", &SETTERS, again));
        }
        known_styles(args, &docx)?;
        let finds = args.find.clone().unwrap_or_default();
        let found = found_ranges(tx.doc(), &finds, again)?;
        for item in &finds {
            read_matching(tx, &item.text).map_err(internal)?;
        }
        read_ranges(tx, found.iter().map(String::as_str)).map_err(internal)?;
        let mut addresses = args.targets.clone().unwrap_or_default();
        addresses.extend(found.iter().cloned());
        let like_plan = look.clone().map(|look| Like {
            look,
            skip: explicit(args, &paragraph, &clear),
        });
        // the number of the version the edit is applied to: in a host's batch, the version
        // its ops left (numbered as the batch numbers it), else the transaction's base
        let version = if editing.editor.in_batch() {
            editing.editor.current().seq().0
        } else {
            tx.base().seq().0
        };
        let plan = FormatPlan {
            like: like_plan,
            keep_together: args.keep_together,
            taller: taller_before(editing, args.keep_together, (version, tx.doc(), &docx), &addresses),
            clear_direct: args.clear_direct.unwrap_or(false),
            paragraph_style: args.paragraph_style.clone(),
            character_style: args.character_style.clone(),
            paragraph: paragraph.clone(),
            run: run.clone(),
        };
        let command = FormatEdit {
            targets: addresses.iter().map(|a| Address::parse(a)).collect(),
            plan,
            cleared: !clear.is_empty(),
            policy: policy(editing),
        };
        Ok((command, (addresses.len(), found, look)))
    }
}

/// With `keep_together` on, the targets it keeps on one page that the version before the change
/// (`version`, read as `docx`) is drawn taller than a page in, where the document's last
/// check laid that very version out (docx-agent's `_laid_before`: nothing is laid out for
/// it).
fn taller_before(
    editing: &Editing<'_>,
    keep_together: Option<bool>,
    (version, doc, docx): (u64, &WordDoc, &Docx),
    addresses: &[String],
) -> BTreeMap<String, (f64, f64)> {
    let (Some(true), Some(last)) = (keep_together, editing.last_check) else {
        return BTreeMap::new();
    };
    let kept: Vec<(String, Vec<String>)> = addresses
        .iter()
        .map(|a| (a, Address::parse(a)))
        .filter(|(_, read)| keeps_blocks(doc, docx, read))
        .filter_map(|(a, read)| {
            let ids = paragraphs_of(doc, docx, &read)
                .ok()?
                .iter()
                .filter_map(|(part, k)| Some(docx.part(part)?.ids.paragraphs().get(*k)?.id.clone()))
                .collect();
            Some((a.clone(), ids))
        })
        .collect();
    if kept.is_empty() {
        return BTreeMap::new();
    }
    crate::check::taller_than_page(last, version, docx, &kept)
}

/// Whether the call sets anything (docx-agent's test, a style named by an empty string
/// setting nothing).
fn sets_anything(args: &WordFormat, run: &[Setting], paragraph: &[Setting], look: bool) -> bool {
    let named = |s: &Option<String>| s.as_deref().is_some_and(|s| !s.is_empty());
    !run.is_empty()
        || !paragraph.is_empty()
        || named(&args.paragraph_style)
        || named(&args.character_style)
        || args.clear_direct.unwrap_or(false)
        || args.keep_together.is_some()
        || look
}

/// The names of the settings the call gives.
fn given(args: &WordFormat) -> Vec<&'static str> {
    let flags = [
        ("bold", args.bold.is_some()),
        ("italic", args.italic.is_some()),
        ("underline", args.underline.is_some()),
        ("strike", args.strike.is_some()),
        ("size", args.size.is_some()),
        ("font", args.font.is_some()),
        ("color", args.color.is_some()),
        ("highlight", args.highlight.is_some()),
        ("alignment", args.alignment.is_some()),
        ("space_before", args.space_before.is_some()),
        ("space_after", args.space_after.is_some()),
        ("line_spacing", args.line_spacing.is_some()),
        ("indent_left", args.indent_left.is_some()),
        ("indent_right", args.indent_right.is_some()),
        ("indent_first", args.indent_first.is_some()),
        ("keep_with_next", args.keep_with_next.is_some()),
        ("keep_lines", args.keep_lines.is_some()),
        ("page_break_before", args.page_break_before.is_some()),
    ];
    flags.into_iter().filter(|(_, on)| *on).map(|(n, _)| n).collect()
}

fn set(property: Property, value: Set) -> Setting {
    Setting {
        property,
        value: Some(value),
    }
}

/// docx-agent's `split_formatting`: the run settings and the paragraph settings, in its
/// order.
fn split(args: &WordFormat) -> (Vec<Setting>, Vec<Setting>) {
    let mut run = Vec::new();
    let flags = [
        (Property::Bold, args.bold),
        (Property::Italic, args.italic),
        (Property::Underline, args.underline),
        (Property::Strike, args.strike),
    ];
    run.extend(flags.into_iter().filter_map(|(p, v)| v.map(|v| set(p, Set::Flag(v)))));
    run.extend(args.size.map(|v| set(Property::Size, Set::Number(v))));
    run.extend(args.font.clone().map(|v| set(Property::Font, Set::Text(v))));
    run.extend(args.color.clone().map(|v| set(Property::Color, Set::Text(v))));
    if let Some(h) = &args.highlight {
        run.push(Setting {
            property: Property::Highlight,
            value: (h != "none").then(|| Set::Text(h.clone())),
        });
    }
    let mut paragraph = Vec::new();
    paragraph.extend(
        args.alignment
            .map(|a| set(Property::Alignment, Set::Text(a.as_str().to_owned()))),
    );
    let numbers = [
        (Property::SpaceBefore, args.space_before),
        (Property::SpaceAfter, args.space_after),
        (Property::LineSpacing, args.line_spacing),
        (Property::IndentLeft, args.indent_left),
        (Property::IndentRight, args.indent_right),
    ];
    paragraph.extend(
        numbers
            .into_iter()
            .filter_map(|(p, v)| v.map(|v| set(p, Set::Number(v)))),
    );
    let flags = [
        (Property::KeepWithNext, args.keep_with_next),
        (Property::KeepTogether, args.keep_lines),
        (Property::PageBreakBefore, args.page_break_before),
    ];
    paragraph.extend(flags.into_iter().filter_map(|(p, v)| v.map(|v| set(p, Set::Flag(v)))));
    if let Some(first) = args.indent_first {
        paragraph.push(if first >= 0.0 {
            set(Property::FirstLine, Set::Number(first))
        } else {
            set(Property::Hanging, Set::Number(-first))
        });
    }
    (run, paragraph)
}

/// `property` cleared: its setting's value removed in its place, or added last.
fn put_cleared(settings: &mut Vec<Setting>, property: Property) {
    match settings.iter_mut().find(|s| s.property == property) {
        Some(found) => found.value = None,
        None => settings.push(Setting { property, value: None }),
    }
}

fn cleared(name: Cleared, run: &mut Vec<Setting>, paragraph: &mut Vec<Setting>) {
    let property = match name {
        Cleared::IndentFirst => {
            put_cleared(paragraph, Property::FirstLine);
            put_cleared(paragraph, Property::Hanging);
            return;
        }
        Cleared::Bold => Property::Bold,
        Cleared::Italic => Property::Italic,
        Cleared::Underline => Property::Underline,
        Cleared::Strike => Property::Strike,
        Cleared::Size => Property::Size,
        Cleared::Font => Property::Font,
        Cleared::Color => Property::Color,
        Cleared::Highlight => Property::Highlight,
        Cleared::Alignment => Property::Alignment,
        Cleared::SpaceBefore => Property::SpaceBefore,
        Cleared::SpaceAfter => Property::SpaceAfter,
        Cleared::LineSpacing => Property::LineSpacing,
        Cleared::IndentLeft => Property::IndentLeft,
        Cleared::IndentRight => Property::IndentRight,
        Cleared::KeepWithNext => Property::KeepWithNext,
        Cleared::KeepLines => Property::KeepTogether,
        Cleared::PageBreakBefore => Property::PageBreakBefore,
    };
    put_cleared(if property.is_run() { run } else { paragraph }, property);
}

/// docx-agent's `_explicit`: the parts of the sample's look the call sets itself.
fn explicit(args: &WordFormat, paragraph: &[Setting], clear: &[Cleared]) -> Vec<&'static str> {
    let mut skip: Vec<&'static str> = Vec::new();
    let mut add = |part: &'static str| {
        if !skip.contains(&part) {
            skip.push(part);
        }
    };
    if args.paragraph_style.is_some() {
        add("style");
    }
    for setting in paragraph {
        match setting.property {
            Property::KeepWithNext => add("keep_with_next"),
            Property::KeepTogether => add("keep_lines"),
            Property::PageBreakBefore => add("page_break_before"),
            _ => {}
        }
    }
    if args.keep_together.is_some() {
        add("keep_lines");
        add("keep_with_next");
    }
    for name in clear {
        match name {
            Cleared::KeepWithNext => add("keep_with_next"),
            Cleared::KeepLines => add("keep_lines"),
            Cleared::PageBreakBefore => add("page_break_before"),
            _ => {}
        }
    }
    skip
}

/// [`known`] for the paragraph and character styles the call names.
fn known_styles(args: &WordFormat, docx: &Docx) -> Result<(), ToolError> {
    for (name, kind, field) in [
        (&args.paragraph_style, "paragraph", "paragraph_style"),
        (&args.character_style, "character", "character_style"),
    ] {
        if let Some(name) = name {
            known(docx, name, kind, field)?;
        }
    }
    Ok(())
}

/// `not_found` with the document's styles of `kind` unless `name` names one (docx-agent's
/// `known_style`).
fn known(docx: &Docx, name: &str, kind: &str, field: &str) -> Result<(), ToolError> {
    let Err(error) = known_style(docx, name, kind) else {
        return Ok(());
    };
    let options: Vec<String> = docx
        .styles()
        .iter()
        .filter(|s| style_kind(s) == kind)
        .filter_map(|s| style_name(s).filter(|n| !n.is_empty()).map(str::to_owned))
        .take(50)
        .collect();
    let message = match error {
        EditError::Refused(m) => m,
        other => other.to_string(),
    };
    let way = WayOut::names(options).unwrap_or_else(|| WayOut::call(call("word_format", &[])));
    Err(ToolError::refusal(Refusal::NotFound, message, way).at(field))
}

/// The look of the paragraph `like` names (docx-agent's `_sample_look`): one paragraph,
/// or a range, cell or span of one; several refused with them (rule 2).
fn sample_look(
    tx: &mut Txn<'_, WordDoc>,
    docx: &Docx,
    like: &str,
    again: &TryCall,
) -> Result<ParagraphLook, ToolError> {
    let found = paragraphs_of(tx.doc(), docx, &Address::parse(like)).map_err(|e| {
        let message = match e {
            EditError::Address(a) if a.message.ends_with("holds no paragraphs") => {
                format!("like: {}", a.message)
            }
            EditError::Address(a) => format!(
                "like {} names no paragraph ({}): give a paragraph id, p:<id>",
                repr_str(like),
                a.message
            ),
            other => format!("like: {other}"),
        };
        ToolError::refusal(Refusal::NotFound, message, WayOut::call(again.clone())).at("like")
    })?;
    let paragraphs: Vec<_> = found
        .iter()
        .filter_map(|(part, k)| docx.part(part)?.ids.paragraphs().get(*k))
        .collect();
    let [one] = paragraphs.as_slice() else {
        let shown: Vec<String> = paragraphs.iter().take(50).map(|p| p.id.clone()).collect();
        let first = shown.first().cloned().unwrap_or_default();
        let message = format!(
            "like names one paragraph to look like, and {like} holds {}: give one of them, e.g. like: {}",
            paragraphs.len(),
            repr_str(&first)
        );
        let way = WayOut::names(shown).unwrap_or_else(|| WayOut::call(again.clone()));
        return Err(ToolError::refusal(Refusal::InvalidArguments, message, way).at("like"));
    };
    // the look is the sample's own: a change to it meanwhile is a conflict
    if let Some((part, _)) = found.first() {
        tx.read(Key::block(part.as_str(), one.id.as_str()));
    }
    Ok(ParagraphLook::of(&one.id, &one.element))
}

/// The reply as docx-agent's `word_format` writes it.
fn reply(
    count: usize,
    found: &[String],
    look: Option<&ParagraphLook>,
    like: Option<&str>,
    out: Formatted,
) -> ToolResult<Value> {
    let any_changed = out.steps.iter().any(|s| s.changed);
    let edits: Vec<Outcome> = out.steps.into_iter().map(Outcome::from).collect();
    let mut data = Map::new();
    if !found.is_empty() {
        data.insert("ranges".into(), json!(found.iter().take(50).collect::<Vec<_>>()));
    }
    let mut nothing_set = false;
    if let (Some(look), Some(done)) = (look, &out.like) {
        let set: Map<String, Value> = done
            .set
            .iter()
            .take(50)
            .map(|(id, parts)| (id.clone(), json!(parts)))
            .collect();
        let mut alike = json!({"sample": look.sample, "set": set,
            "alike": done.alike.iter().take(50).collect::<Vec<_>>(), "not_taken": NOT_TAKEN});
        let left = done.set.len().saturating_sub(50) + done.alike.len().saturating_sub(50);
        if left > 0 {
            if let Some(map) = alike.as_object_mut() {
                map.insert("left_out".into(), json!(left));
            }
        }
        data.insert("like".into(), alike);
        nothing_set = done.set.is_empty();
    }
    let data = (!data.is_empty()).then_some(Value::Object(data));
    let mut result = outcome(&edits, format!("Formatted {count} target(s)"), data);
    if let (Some(look), Some(like)) = (look, like) {
        if look.sample != like {
            result
                .warnings
                .push(format!("like {like} read as its paragraph {}", look.sample));
        }
        if nothing_set && !any_changed {
            result.warnings.push(format!(
                "every target already looks like {}: nothing changed",
                look.sample
            ));
        }
    }
    if let Some((before, after)) = &out.outline {
        let renamed: Vec<(String, String)> = edits.iter().flat_map(|e| e.renamed.iter().cloned()).collect();
        result.warnings.extend(moves(before, after, &renamed, 5));
    }
    result
}

/// Registers `word_format` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.edit::<WordFormat>(word_format);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::planned::held::{args, calling, compare, marked, paragraph, raced, shifted};

    /// The plan of `args`, which clear nothing.
    fn raced_format(
        args: &WordFormat,
        editor: &ooxml_word_edit::Editor,
        between: &impl ooxml_store::Command<WordDoc, Error = EditError>,
    ) -> (Option<ToolError>, u64) {
        let again = call("word_format", &[]);
        let (run, paragraph) = split(args);
        calling(editor, |editing| {
            raced(
                editor,
                format_plan(args, (run, paragraph, Vec::new()), editing, &again),
                between,
            )
        })
    }

    #[test]
    fn text_found_to_format_is_found_where_it_is_when_the_format_commits() {
        let args: WordFormat = args(json!({"find": [{"text": "this year"}], "bold": true}));
        let compared = compare(
            marked,
            shifted,
            |editor, between| raced_format(&args, editor, between),
            |editing| word_format(&args, editing),
        );
        compared.assert_same_edit();
        assert!(compared.serial.contains("14% [b]this year."), "{}", compared.serial);
    }

    #[test]
    fn the_like_sample_s_look_is_read_where_the_format_commits() {
        // the sample kept with the next meanwhile: the target looks like it as it is then
        // (italic too, so that the format writes on either version: an edit that writes
        // nothing is no conflict)
        let editor = marked();
        let (target, sample) = (paragraph(&editor, "Plain bold"), paragraph(&editor, "Visit"));
        let args: WordFormat = args(json!({"targets": [target], "like": sample, "italic": true}));
        let kept = |_: &ooxml_word_edit::Editor| FormatEdit {
            targets: vec![Address::parse(&sample)],
            plan: FormatPlan {
                paragraph: vec![set(Property::KeepWithNext, Set::Flag(true))],
                ..FormatPlan::default()
            },
            cleared: false,
            policy: ooxml_word_edit::RevisionPolicy::Untracked,
        };
        let compared = compare(
            marked,
            kept,
            |editor, between| raced_format(&args, editor, between),
            |editing| word_format(&args, editing),
        );
        compared.assert_same_edit();
        let target = compared.serial.lines().find(|l| l.contains("Plain")).unwrap();
        assert!(target.contains("[keepNext]"), "{}", compared.serial);
    }
}
