//! Owns: `word_lists` (docx-agent's `tools/style.py`, `word_lists`): its definition, its
//! refusal of `remove` where a style numbers the paragraphs (the styles that would do, the
//! lists to join), and its edits through the edit core's [`ListEdit`].
//!
//! Must not: change the document but through a command.

use crate::planned::run_planned;
use crate::review::{edit_refusal, Editing};
use crate::structure::common::{call, need, outcome, policy, Outcome};
use ooxml_store::Txn;
use ooxml_tools::{python_dumps, ArgEnum, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::units::twips;
use ooxml_word_edit::{
    numbered_by_style, paragraphs_of, unnumbered_styles_like, LevelFormat, ListAction, ListEdit, ListStep, ListsEdited,
    WordDoc,
};
use ooxml_word_model::address::Address;
use ooxml_word_model::docx::numbering::{membership, Lists};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::W;
use serde_json::{json, Value};

/// What `word_lists` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListsAction {
    /// Add to a list.
    Add,
    /// Take out of the list.
    Remove,
    /// Set the level.
    Level,
    /// Restart numbering.
    Restart,
    /// Continue numbering.
    Continue,
    /// Change a level's format.
    Format,
}

/// A list's kind.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// Bullets.
    Bullet,
    /// Numbers.
    Number,
}

/// What follows a level's number.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    /// A tab.
    Tab,
    /// A space.
    Space,
    /// Nothing.
    Nothing,
}

/// `word_lists`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(name = "word_lists", description = include_str!("word_lists.md"), group = "word_style", mutates)]
pub struct WordLists {
    /// Document id.
    pub doc: String,
    /// Paragraphs or spans.
    #[arg(min_items = 1, refs)]
    pub targets: Vec<String>,
    /// What to do.
    pub action: ListsAction,
    /// add. Default bullet.
    pub kind: Option<ListKind>,
    /// add, level, format: level from 0.
    #[arg(minimum = 0, maximum = 8)]
    pub level: Option<i64>,
    /// restart: first number (default 1); format: start value.
    #[arg(minimum = 0, maximum = 32767)]
    pub start: Option<i64>,
    /// format: decimal, lowerLetter, upperRoman, bullet...
    pub number_format: Option<String>,
    /// format: label, e.g. %1) or a bullet.
    pub label: Option<String>,
    /// format: number's position.
    #[arg(minimum = -1584, maximum = 1584)]
    pub number_at: Option<f64>,
    /// format: text indent.
    #[arg(minimum = -1584, maximum = 1584)]
    pub text_at: Option<f64>,
    /// format: after the number.
    pub follow: Option<Follow>,
    /// format: tab stop.
    #[arg(minimum = -1584, maximum = 1584)]
    pub tab_at: Option<f64>,
    /// remove: also where the style numbers. Default false.
    pub force: Option<bool>,
    /// continue: a paragraph of the list to join.
    #[arg(refs)]
    pub join: Option<String>,
}

impl From<ListStep> for Outcome {
    fn from(step: ListStep) -> Self {
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

/// `word_lists` (docx-agent's `word_lists`): every paragraph the targets hold added to a
/// list, taken out, moved to a level; each target's first one's list continued; the first
/// one's list restarted or a level of it formatted -- one command.
#[must_use]
pub fn word_lists(args: &WordLists, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let again = call(
        "word_lists",
        &[
            ("doc", json!(args.doc)),
            ("targets", json!(args.targets)),
            ("action", json!(args.action.as_str())),
        ],
    );
    match run(args, editing, &again) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

fn run(args: &WordLists, editing: &Editing<'_>, again: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    Ok(match run_planned(editing, lists_plan(args, editing, again))? {
        Ok(done) => reply(args, done.out.0),
        Err(error) => ToolResult::failed(edit_refusal(error, "", again.clone())),
    })
}

/// `word_lists`' plan, made in the edit's transaction ([`crate::planned`]): the targets'
/// paragraphs, the action (`continue`'s list) and `remove`'s refusal of paragraphs their
/// style numbers, on the version the edit is applied to ([`ListEdit`] reads all of it).
fn lists_plan<'a>(
    args: &'a WordLists,
    editing: &'a Editing<'a>,
    again: &'a TryCall,
) -> impl Fn(&mut Txn<'_, WordDoc>) -> Result<(ListEdit, ()), ToolError> + 'a {
    move |tx| {
        let doc = tx.doc();
        let docx = doc
            .docx()
            .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
        let targets: Vec<Address> = args.targets.iter().map(|t| Address::parse(t)).collect();
        let mut paragraphs = Vec::new();
        for target in &targets {
            let found = paragraphs_of(doc, &docx, target).map_err(|e| edit_refusal(e, "", again.clone()))?;
            paragraphs.extend(found);
        }
        let action = action(args, doc, &docx, again)?;
        if args.action == ListsAction::Remove && !args.force.unwrap_or(false) {
            refuse_style_numbered(&docx, &paragraphs, again)?;
        }
        let command = ListEdit {
            targets,
            action,
            policy: policy(editing),
        };
        Ok((command, ()))
    }
}

/// The edit core's action for the call (docx-agent's arguments read as it reads them).
fn action(args: &WordLists, doc: &WordDoc, docx: &Docx, again: &TryCall) -> Result<ListAction, ToolError> {
    Ok(match args.action {
        ListsAction::Add => ListAction::Add {
            kind: args.kind.unwrap_or(ListKind::Bullet).as_str().to_owned(),
            level: args.level.unwrap_or(0),
        },
        ListsAction::Remove => ListAction::Remove,
        ListsAction::Level => ListAction::Level(*need(args.level.as_ref(), "level", again)?),
        ListsAction::Restart => ListAction::Restart(args.start.unwrap_or(1)),
        ListsAction::Continue => ListAction::Continue(joined(doc, docx, args.join.as_deref(), again)?),
        ListsAction::Format => ListAction::Format(LevelFormat {
            level: args.level,
            format: args.number_format.clone(),
            text: args.label.clone(),
            start: args.start,
            number_at: args.number_at.map(twips),
            text_at: args.text_at.map(twips),
            follow: args.follow.map(|f| f.as_str().to_owned()),
            tab_at: args.tab_at.map(twips),
        }),
    })
}

/// `join`'s first paragraph's id, or none -- an empty value too (rule 1; docx-agent's
/// `_joined`).
fn joined(doc: &WordDoc, docx: &Docx, join: Option<&str>, again: &TryCall) -> Result<Option<String>, ToolError> {
    let Some(join) = join.map(str::trim).filter(|j| !j.is_empty()) else {
        return Ok(None);
    };
    let found = paragraphs_of(doc, docx, &Address::parse(join)).map_err(|e| edit_refusal(e, "", again.clone()))?;
    let id = found.first().and_then(|(part, k)| {
        let story = docx.part(part)?;
        Some(story.ids.paragraphs().get(*k)?.id.clone())
    });
    id.map(Some).ok_or_else(|| {
        ToolError::refusal(
            Refusal::NotFound,
            format!(
                "join {} holds no paragraph: name a paragraph of the list to go on from",
                ooxml_tools::pyfmt::repr_str(join)
            ),
            WayOut::call(again.clone()),
        )
        .at("join")
    })
}

/// The reply as docx-agent's `word_lists` writes it.
fn reply(args: &WordLists, out: ListsEdited) -> ToolResult<Value> {
    let count = out.lists.len();
    let lists: Vec<Value> = out
        .lists
        .iter()
        .take(50)
        .map(|(id, list)| {
            let list = list.as_ref().map_or(Value::Null, |m| {
                let kind = if m.format.as_deref() == Some("bullet") {
                    "bullet"
                } else {
                    "number"
                };
                json!({"kind": kind, "level": m.level, "num_id": m.num_id})
            });
            json!({"id": id, "list": list})
        })
        .collect();
    let edits: Vec<Outcome> = out.steps.into_iter().map(Outcome::from).collect();
    let mut result = outcome(
        &edits,
        format!("{}: {count} paragraph(s)", args.action.as_str()),
        Some(Value::Array(lists)),
    );
    if args.action != ListsAction::Continue && args.join.as_deref().is_some_and(|j| !j.trim().is_empty()) {
        result
            .warnings
            .push(format!("join is continue's: left out of {}", args.action.as_str()));
    }
    if !out.listless.is_empty() {
        let mut named = out.listless.iter().take(10).cloned().collect::<Vec<_>>().join(", ");
        if out.listless.len() > 10 {
            named.push_str(&format!(" and {} more", out.listless.len() - 10));
        }
        result.warnings.push(format!(
            "{} paragraph(s) were not in a list, left as they were: {named}",
            out.listless.len()
        ));
    }
    result
}

/// The refusal of `remove` for paragraphs whose style numbers them: which, the styles to
/// give them instead, the lists to join, or `force` (docx-agent's `_refuse_style_numbered`).
fn refuse_style_numbered(docx: &Docx, paragraphs: &[(String, usize)], again: &TryCall) -> Result<(), ToolError> {
    let mut numbered: Vec<(String, Vec<String>)> = Vec::new();
    for (part, k) in paragraphs {
        let Some(paragraph) = docx.part(part).and_then(|s| s.ids.paragraphs().get(*k)) else {
            continue;
        };
        if let Some(style) = numbered_by_style(docx, &paragraph.element) {
            match numbered.iter_mut().find(|(s, _)| *s == style) {
                Some(slot) => slot.1.push(paragraph.id.clone()),
                None => numbered.push((style, vec![paragraph.id.clone()])),
            }
        }
    }
    if numbered.is_empty() {
        return Ok(());
    }
    let (mut parts, mut options) = (Vec::new(), Vec::<String>::new());
    for (style_id, ids) in numbered.iter().take(5) {
        let fits = unnumbered_styles_like(docx, style_id, 6);
        let mut shown = ids.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        if ids.len() > 5 {
            shown.push_str(&format!(" and {} more", ids.len() - 5));
        }
        let tail = if fits.is_empty() {
            " (no style like it numbers nothing: word_styles add one based on it)".to_owned()
        } else {
            format!(" (styles like it that number nothing: {})", fits.join(", "))
        };
        let name = style_label(docx, style_id);
        parts.push(format!(
            "{shown}: style {} numbers them{tail}",
            ooxml_tools::pyfmt::repr_str(&name)
        ));
        for fit in fits {
            if !options.contains(&fit) {
                options.push(fit);
            }
        }
    }
    let mut message = parts.join("; ")
        + ". Taking them out of the list would override their style: give them a style that numbers nothing \
           (word_format paragraph_style), or force true to override (no marker)";
    let ids: Vec<String> = numbered.iter().flat_map(|(_, ids)| ids.iter().cloned()).collect();
    let joins = joins(docx, paragraphs, &ids);
    if !joins.is_empty() {
        let calls: Vec<String> = joins
            .iter()
            .map(|c| format!("word_format {}", python_dumps(c)))
            .collect();
        message.push_str(&format!(
            ". To drop their own numbering and join an earlier list at their level instead, keeping a marker: {}",
            calls.join("; ")
        ));
        let mut first: Vec<String> = joins
            .iter()
            .map(|c| {
                format!(
                    "word_format like {}",
                    c.get("like").and_then(Value::as_str).unwrap_or("")
                )
            })
            .collect();
        first.append(&mut options);
        options = first;
    }
    options.push("force".to_owned());
    options.truncate(50);
    let way_out = WayOut::names(options).unwrap_or_else(|| WayOut::call(again.clone()));
    Err(ToolError::refusal(Refusal::InvalidArguments, message, way_out).at("targets"))
}

/// A style's name, or its id where it has none (docx-agent's `s.name or s.id`).
fn style_label(docx: &Docx, style_id: &str) -> String {
    docx.styles()
        .iter()
        .find(|s| s.attribute(Some(W), "styleId") == Some(style_id))
        .and_then(|s| ooxml_word_model::docx::style_name(s).filter(|n| !n.is_empty()))
        .unwrap_or(style_id)
        .to_owned()
}

/// For paragraphs numbered by a list of their own, the `word_format` calls that make them
/// items of the nearest earlier list at their level (`like` an item of it), grouped by
/// that item; at most five (docx-agent's `_joins`).
fn joins(docx: &Docx, paragraphs: &[(String, usize)], ids: &[String]) -> Vec<Value> {
    let lists = Lists::read(docx);
    let mut calls: Vec<(String, Vec<String>)> = Vec::new();
    for id in ids {
        let Some((part, k)) = paragraphs.iter().find(|(part, k)| {
            docx.part(part)
                .and_then(|s| s.ids.paragraphs().get(*k))
                .is_some_and(|p| p.id == *id)
        }) else {
            continue;
        };
        let Some(story) = docx.part(part) else { continue };
        let Some(paragraph) = story.ids.paragraphs().get(*k) else {
            continue;
        };
        let Some(own) = membership(docx, &lists, &paragraph.element).filter(|m| !m.from_style) else {
            continue;
        };
        if let Some(item) = earlier_item(docx, story, &paragraph.path, &own, &lists) {
            match calls.iter_mut().find(|(i, _)| *i == item) {
                Some(slot) => slot.1.push(id.clone()),
                None => calls.push((item, vec![id.clone()])),
            }
        }
    }
    calls
        .into_iter()
        .take(5)
        .map(|(item, targets)| json!({"targets": targets, "like": item}))
        .collect()
}

/// The nearest paragraph before the one at `path` in its container that is an item of
/// another list at `own`'s level (docx-agent's `_earlier_item`).
fn earlier_item(
    docx: &Docx,
    story: &ooxml_word_model::docx::StoryPart,
    path: &[u32],
    own: &ooxml_word_model::docx::numbering::Membership,
    lists: &Lists,
) -> Option<String> {
    let (&last, above) = path.split_last()?;
    let parent = ooxml_word_model::docx::at_path(story.root(), above)?;
    for k in (0..last as usize).rev() {
        let Some(ooxml_xml::Node::Element(node)) = parent.children().get(k) else {
            continue;
        };
        if !node.name().is(Some(W), "p") {
            continue;
        }
        let Some(found) = membership(docx, lists, node) else {
            continue;
        };
        if found.num_id != own.num_id && found.level == own.level {
            let mut at = above.to_vec();
            at.push(u32::try_from(k).ok()?);
            return story.ids.paragraph_at(&at).map(|p| p.id);
        }
    }
    None
}

/// Registers `word_lists` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordLists>(word_lists, crate::Hosting::PLAIN);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::planned::held::{args, calling, compare, opened, paragraph, raced};
    use ooxml_word_edit::{RevisionPolicy, SetParagraphStyle};

    #[test]
    fn the_numbered_style_check_is_made_where_the_remove_commits() {
        // the item given a style that numbers it meanwhile: the remove, prepared again,
        // refuses as it does called after it -- not the list taken off and the style left
        let open = || opened("lists-and-styles.docx");
        let item = paragraph(&open(), "First item");
        let args: WordLists = args(json!({"targets": [item], "action": "remove"}));
        let again = call("word_lists", &[]);
        let compared = compare(
            open,
            |editor| SetParagraphStyle {
                target: Address::parse(&paragraph(editor, "First item")),
                style: Some("List Bullet".to_owned()),
                policy: RevisionPolicy::Untracked,
            },
            |editor, between| {
                calling(editor, |editing| {
                    raced(editor, lists_plan(&args, editing, &again), between)
                })
            },
            |editing| word_lists(&args, editing),
        );
        compared.assert_same_refusal();
    }
}
