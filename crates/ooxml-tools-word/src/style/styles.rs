//! Owns: `word_styles`' handler (docx-agent's `tools/style.py`, `word_styles`): the list
//! and a style's description read from the document, the call's refusals as docx-agent
//! gives them (a field another action's, nothing to modify, a style that is not a
//! paragraph style for a list level, a list or level the document lacks), the edits
//! through the edit core's [`StyleEdit`], and the replies (a table style's regions, the
//! tables whose look hides one, the document's languages).
//!
//! Must not: change the document but through a command.

use super::style_view::{describe, languages_json, look_warnings, regions_json};
use super::styles_args::{RegionItem, StyleKind, StylesAction, TabItem, WordStyles};
use crate::review::{edit_refusal, Editing};
use crate::structure::common::{call, need, outcome, Outcome};
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{ArgEnum, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{
    languages, word_tag, EditError, ListLevel, Property, RegionBorders, RegionValues, Setting, StyleAction, StyleEdit,
    StyleFormatting, StyleRefusalKind, StyleStep, TabStop, Value as Set,
};
use ooxml_word_model::docx::{style_id, style_kind, style_name, Docx};
use serde_json::{json, Value};

impl From<StyleStep> for Outcome {
    fn from(step: StyleStep) -> Self {
        Outcome {
            id: step.id,
            changed: step.changed,
            created: step.created,
            removed: step.removed,
            renamed: Vec::new(),
            warnings: step.warnings,
        }
    }
}

/// The fields `modify` sets, for its refusal when it sets none.
const STYLE_FIELDS: [&str; 15] = [
    "bold",
    "italic",
    "size",
    "font",
    "color",
    "alignment",
    "space_before",
    "space_after",
    "line_spacing",
    "indent_left",
    "indent_first",
    "keep_with_next",
    "shading",
    "numbering",
    "tabs",
];

/// `word_styles` (docx-agent's): the style sheet listed, described and edited.
#[must_use]
pub fn word_styles(args: &WordStyles, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let again = call(
        "word_styles",
        &[("doc", json!(args.doc)), ("action", json!(args.action.as_str()))],
    );
    run(args, editing, &again).unwrap_or_else(ToolResult::failed)
}

fn invalid(message: String, field: &str, options: &[&str], again: &TryCall) -> ToolError {
    let way = WayOut::names(options.iter().map(|s| (*s).to_owned())).unwrap_or_else(|| WayOut::call(again.clone()));
    ToolError::refusal(Refusal::InvalidArguments, message, way).at(field)
}

fn kind_of(kind: Option<StyleKind>) -> Option<&'static str> {
    kind.map(|k| k.as_str())
}

/// The styles a refusal lists: those named, of `kind` when given.
fn style_names(docx: &Docx, kind: Option<&str>) -> Vec<String> {
    docx.styles()
        .iter()
        .filter(|s| kind.is_none_or(|k| style_kind(s) == k))
        .filter_map(|s| style_name(s).filter(|n| !n.is_empty()).map(str::to_owned))
        .take(50)
        .collect()
}

/// An edit's refusal as `word_styles` gives it.
fn refusal(error: EditError, docx: &Docx, kind: Option<&str>, again: &TryCall) -> ToolError {
    let EditError::Style(refused) = error else {
        return edit_refusal(error, "", again.clone());
    };
    match (refused.kind, refused.field) {
        (StyleRefusalKind::NotFound, None) => {
            let names = style_names(docx, kind);
            let way = WayOut::names(names).unwrap_or_else(|| WayOut::call(again.clone()));
            ToolError::refusal(Refusal::NotFound, refused.message, way).at("name")
        }
        (StyleRefusalKind::NotFound, Some(field)) => {
            ToolError::refusal(Refusal::NotFound, refused.message, WayOut::call(again.clone())).at(&field)
        }
        (StyleRefusalKind::Invalid, field) => {
            let field = field.unwrap_or_else(|| "regions".to_owned());
            let cap = if field == "language" { 30 } else { 50 };
            let way =
                WayOut::names(refused.options.into_iter().take(cap)).unwrap_or_else(|| WayOut::call(again.clone()));
            ToolError::refusal(Refusal::InvalidArguments, refused.message, way).at(&field)
        }
    }
}

fn head(editing: &Editing<'_>) -> Result<std::sync::Arc<Docx>, ToolError> {
    let snapshot = editing.editor.store().snapshot();
    snapshot
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))
}

fn run(args: &WordStyles, editing: &Editing<'_>, again: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let docx = head(editing)?;
    let mut kind = kind_of(args.kind);
    let action = args.action;
    match action {
        StylesAction::List => return Ok(list(args, &docx)),
        StylesAction::PurgeUnused => {
            let step = edit(editing, StyleAction::PurgeUnused, &docx, kind, again)?;
            return Ok(outcome(&[step.into()], "Purged unused styles".to_owned(), None));
        }
        StylesAction::SetLanguage => {
            let language = need(args.language.as_deref(), "language", again)?;
            return set_language(args, editing, language, again);
        }
        _ => {}
    }
    let (name, regions) = fields(args, again)?;
    match action {
        StylesAction::Describe => {
            let style =
                ooxml_word_edit::find_style(&docx, name, kind).ok_or_else(|| not_found(&docx, kind, name, again))?;
            let data = describe(&docx, &style).map_err(|e| refusal(e, &docx, kind, again))?;
            return Ok(ToolResult::done(ooxml_tools::Done {
                data: Some(data),
                ..ooxml_tools::Done::new(format!("Style {name}"))
            }));
        }
        StylesAction::Remove => {
            let action = StyleAction::Remove {
                name: name.to_owned(),
                kind: kind.map(str::to_owned),
                replacement: args.replacement.clone(),
            };
            let step = edit(editing, action, &docx, kind, again)?;
            return Ok(outcome(&[step.into()], format!("Removed style {name}"), None));
        }
        StylesAction::SetDefault => return set_default(name, kind, editing, &docx, again),
        _ => {}
    }
    let formatting = formatting(args);
    let mut note = None;
    if !regions.is_empty() && action == StylesAction::Add && kind.is_none() {
        kind = Some("table");
        note = Some("regions given: added as a table style (kind table)".to_owned());
    }
    let tabs = args.tabs.as_ref().map(|t| t.iter().map(tab).collect::<Vec<_>>());
    if action == StylesAction::Add {
        return add(args, editing, (name, kind), (formatting, tabs, regions), note, again);
    }
    if !regions.is_empty() {
        return modify_regions(editing, (name, kind), (formatting, tabs, regions), &docx, again);
    }
    modify(args, editing, (name, kind), (formatting, tabs), &docx, again)
}

/// The style named, and the regions given, after refusing a field another action's
/// (`language`, `numbering`, `tabs`, `regions`).
fn fields<'a>(args: &'a WordStyles, again: &TryCall) -> Result<(&'a str, Vec<RegionValues>), ToolError> {
    let action = args.action;
    let name_of = action.as_str();
    if args.language.is_some() {
        return Err(invalid(
            format!("language is set_language's: {name_of} takes none"),
            "language",
            &[],
            again,
        ));
    }
    let name = need(args.name.as_deref(), "name", again)?;
    if args.numbering.is_some() && action != StylesAction::Modify {
        return Err(invalid(
            format!("numbering is modify's: {name_of} takes none"),
            "numbering",
            &[],
            again,
        ));
    }
    let shaping = matches!(action, StylesAction::Add | StylesAction::Modify);
    if args.tabs.is_some() && !shaping {
        return Err(invalid(
            format!("tabs is add's and modify's: {name_of} takes none"),
            "tabs",
            &[],
            again,
        ));
    }
    let regions: Vec<RegionValues> = args.regions.iter().flatten().map(region).collect();
    if !regions.is_empty() && !shaping {
        return Err(invalid(
            format!("regions are add's and modify's (a table style): {name_of} takes none"),
            "regions",
            &[],
            again,
        ));
    }
    Ok((name, regions))
}

/// `modify` of a table style's regions: the regions it states after, and the tables whose
/// look hides one.
fn modify_regions(
    editing: &Editing<'_>,
    (name, kind): (&str, Option<&str>),
    (formatting, tabs, regions): (StyleFormatting, Option<Vec<TabStop>>, Vec<RegionValues>),
    docx: &Docx,
    again: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let change = StyleAction::Modify {
        name: name.to_owned(),
        kind: kind.map(str::to_owned),
        numbering: None,
        tabs,
        regions,
        formatting,
    };
    let step = edit(editing, change, docx, kind, again)?;
    let summary = if step.changed {
        format!("Modified style {name}")
    } else {
        format!("No change to style {name}")
    };
    let after = head(editing)?;
    let data = after_regions(&after, name, kind);
    let mut result = outcome(&[step.into()], summary, Some(data));
    result.warnings.extend(look_warnings(&after, name));
    Ok(result)
}

/// A style not found as docx-agent's `StyleError` reads in the tool.
fn not_found(docx: &Docx, kind: Option<&str>, name: &str, again: &TryCall) -> ToolError {
    let kind_word = kind.map(|k| format!("{k} ")).unwrap_or_default();
    let message = format!("the document has no {kind_word}style {}", repr_str(name));
    let way = WayOut::names(style_names(docx, kind)).unwrap_or_else(|| WayOut::call(again.clone()));
    ToolError::refusal(Refusal::NotFound, message, way).at("name")
}

fn edit(
    editing: &Editing<'_>,
    action: StyleAction,
    docx: &Docx,
    kind: Option<&str>,
    again: &TryCall,
) -> Result<StyleStep, ToolError> {
    let command = StyleEdit { action };
    editing
        .editor
        .run(editing.caller, &command)
        .map(|done| done.out)
        .map_err(|e| refusal(e, docx, kind, again))
}

/// The regions a table style states now, as `add` and `modify` reply with them.
fn after_regions(docx: &Docx, name: &str, kind: Option<&str>) -> Value {
    let regions = ooxml_word_edit::find_style(docx, name, kind.or(Some("table")))
        .map(|s| regions_json(&s))
        .unwrap_or_default();
    json!({"regions": regions})
}

fn list(args: &WordStyles, docx: &Docx) -> ToolResult<Value> {
    let kind = kind_of(args.kind);
    let chosen = ooxml_word_edit::table_default(docx);
    let counts = ooxml_word_edit::content_uses(docx);
    let mut rows = Vec::new();
    for style in docx.styles() {
        let Some(name) = style_name(style) else { continue };
        if name.is_empty() || kind.is_some_and(|k| style_kind(style) != k) {
            continue;
        }
        if args.in_use_only == Some(true) {
            // the uses of the style its name finds, as docx-agent counts them
            let counted = docx
                .find_style(name, style_kind(style))
                .map_or(style_id(style), |s| style_id(s));
            if counts.get(counted).copied().unwrap_or(0) == 0 {
                continue;
            }
        }
        let based_on = style
            .find(Some(ooxml_word_model::ids::W), "basedOn")
            .and_then(|b| b.attribute(Some(ooxml_word_model::ids::W), "val"));
        let mut row = json!({"name": name, "kind": style_kind(style), "based_on": based_on});
        if chosen.as_ref().is_some_and(|c| style_id(c) == style_id(style)) {
            if let Some(map) = row.as_object_mut() {
                map.insert("default".to_owned(), json!(true));
            }
        }
        rows.push(row);
    }
    let mut summary = format!("{} style(s)", rows.len());
    if let Some(chosen) = &chosen {
        summary.push_str(&format!("; new tables get {}", style_name(chosen).unwrap_or("None")));
    }
    rows.truncate(200);
    ToolResult::done(ooxml_tools::Done {
        data: Some(Value::Array(rows)),
        ..ooxml_tools::Done::new(summary)
    })
}

fn set_default(
    name: &str,
    kind: Option<&str>,
    editing: &Editing<'_>,
    docx: &Docx,
    again: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    if !matches!(kind, None | Some("table")) {
        return Err(invalid(
            "set_default sets the table style new tables get: kind table".to_owned(),
            "kind",
            &["table"],
            again,
        ));
    }
    let step = edit(
        editing,
        StyleAction::SetDefault { name: name.to_owned() },
        docx,
        Some("table"),
        again,
    )?;
    let summary = if step.changed {
        let after = head(editing)?;
        let chosen = ooxml_word_edit::table_default(&after);
        format!(
            "New tables get {}",
            chosen.as_ref().and_then(|c| style_name(c)).unwrap_or("None")
        )
    } else {
        format!("No change: new tables get {name}")
    };
    Ok(outcome(&[step.into()], summary, None))
}

fn set_language(
    args: &WordStyles,
    editing: &Editing<'_>,
    language: &str,
    again: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let docx = head(editing)?;
    let kind = kind_of(args.kind);
    let name = args.name.as_deref();
    let action = StyleAction::SetLanguage {
        language: language.to_owned(),
        style: name.map(str::to_owned),
    };
    let step = edit(editing, action, &docx, kind, again)?;
    let after = head(editing)?;
    let found = languages(&after);
    let data = languages_json(&found);
    let slot = word_tag(language).map(|(_, s, _)| s).unwrap_or("val");
    let tag: Vec<(&str, String)> = match name {
        Some(n) => found
            .styles
            .iter()
            .find(|(s, _)| s == n)
            .map(|(_, t)| t.clone())
            .unwrap_or_default(),
        None => found.default.clone(),
    };
    let what = name.map_or_else(|| "The document".to_owned(), |n| format!("Style {n}"));
    let summary = if step.changed {
        let shown = tag
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| tag.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join(", "));
        format!("{what}: language {shown}")
    } else {
        format!("No change: {}", step.warnings.first().cloned().unwrap_or_default())
    };
    Ok(outcome(&[step.into()], summary, Some(data)))
}

fn add(
    args: &WordStyles,
    editing: &Editing<'_>,
    (name, kind): (&str, Option<&str>),
    (formatting, tabs, regions): (StyleFormatting, Option<Vec<TabStop>>, Vec<RegionValues>),
    note: Option<String>,
    again: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let docx = head(editing)?;
    let made = kind.unwrap_or("paragraph");
    let action = StyleAction::Add {
        name: name.to_owned(),
        kind: made.to_owned(),
        based_on: args.based_on.clone(),
        next_style: args.next_style.clone(),
        tabs,
        regions,
        formatting,
    };
    let step = edit(editing, action, &docx, kind, again)?;
    let data = (made == "table").then(|| after_regions(&head(editing).unwrap_or(docx), name, Some("table")));
    let mut result = outcome(&[step.into()], format!("Added style {name}"), data);
    if let Some(note) = note {
        result.warnings.insert(0, note);
    }
    Ok(result)
}

fn modify(
    args: &WordStyles,
    editing: &Editing<'_>,
    (name, kind): (&str, Option<&str>),
    (formatting, tabs): (StyleFormatting, Option<Vec<TabStop>>),
    docx: &Docx,
    again: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let numbering = args.numbering.as_ref().map(|n| {
        let given = n.list.trim();
        if given.eq_ignore_ascii_case("none") {
            (ListLevel::Unlink, n.level.is_some())
        } else {
            (
                ListLevel::Link {
                    list: given.to_owned(),
                    level: n.level.unwrap_or(0),
                },
                false,
            )
        }
    });
    if let Some((_, true)) = numbering {
        return Err(invalid(
            "list \"none\" unlinks: give no level with it".to_owned(),
            "numbering.level",
            &[],
            again,
        ));
    }
    if formatting.is_empty() && tabs.is_none() && numbering.is_none() {
        return Err(invalid(
            "modify needs formatting to set, numbering or tabs".to_owned(),
            "",
            &STYLE_FIELDS,
            again,
        ));
    }
    let numbering = numbering.map(|(n, _)| n);
    if numbering.is_some() {
        let style = ooxml_word_edit::find_style(docx, name, kind).ok_or_else(|| not_found(docx, kind, name, again))?;
        let theirs = style_kind(&style);
        if theirs != "paragraph" {
            return Err(invalid(
                format!(
                    "{} is a {theirs} style: only a paragraph style is linked to a list level",
                    repr_str(name)
                ),
                "name",
                &[],
                again,
            ));
        }
    }
    let summary = match &numbering {
        None => None,
        Some(ListLevel::Unlink) => Some(String::new()),
        Some(ListLevel::Link { list, level }) => Some(format!("linked to level {level} of list {list}")),
    };
    let action = StyleAction::Modify {
        name: name.to_owned(),
        kind: kind.map(str::to_owned),
        numbering: numbering.clone(),
        tabs,
        regions: Vec::new(),
        formatting,
    };
    let step = edit(editing, action, docx, kind, again)?;
    let summary = match summary {
        None => format!("Modified style {name}"),
        Some(linked) if !linked.is_empty() => format!("Modified style {name}: {linked}"),
        Some(_) if step.changed => format!("Modified style {name}: unlinked from its list level"),
        Some(_) => format!("Modified style {name}: numbers nothing, left as it was"),
    };
    Ok(outcome(&[step.into()], summary, None))
}

/// The call's formatting as docx-agent's `split_formatting` splits it (run, then paragraph
/// settings in its order, a negative first-line indent a hanging one) and its shading.
fn formatting(args: &WordStyles) -> StyleFormatting {
    let set = |property, value| Setting {
        property,
        value: Some(value),
    };
    let mut run = Vec::new();
    run.extend(args.bold.map(|v| set(Property::Bold, Set::Flag(v))));
    run.extend(args.italic.map(|v| set(Property::Italic, Set::Flag(v))));
    run.extend(args.size.map(|v| set(Property::Size, Set::Number(v))));
    run.extend(args.font.clone().map(|v| set(Property::Font, Set::Text(v))));
    run.extend(args.color.clone().map(|v| set(Property::Color, Set::Text(v))));
    let mut paragraph = Vec::new();
    paragraph.extend(
        args.alignment
            .map(|a| set(Property::Alignment, Set::Text(a.as_str().to_owned()))),
    );
    for (property, value) in [
        (Property::SpaceBefore, args.space_before),
        (Property::SpaceAfter, args.space_after),
        (Property::LineSpacing, args.line_spacing),
        (Property::IndentLeft, args.indent_left),
    ] {
        paragraph.extend(value.map(|v| set(property, Set::Number(v))));
    }
    paragraph.extend(args.keep_with_next.map(|v| set(Property::KeepWithNext, Set::Flag(v))));
    if let Some(first) = args.indent_first {
        paragraph.push(if first >= 0.0 {
            set(Property::FirstLine, Set::Number(first))
        } else {
            set(Property::Hanging, Set::Number(-first))
        });
    }
    let shading = args
        .shading
        .as_ref()
        .map(|s| (!s.trim().eq_ignore_ascii_case("none")).then(|| s.clone()));
    StyleFormatting {
        run,
        paragraph,
        shading,
    }
}

fn tab(item: &TabItem) -> TabStop {
    TabStop {
        position: item.position,
        alignment: item.alignment.map(|a| a.as_str().to_owned()),
        leader: item.leader.map(|l| l.as_str().to_owned()),
    }
}

fn region(item: &RegionItem) -> RegionValues {
    RegionValues {
        region: item.region.as_str().to_owned(),
        shading: item.shading.clone(),
        color: item.color.clone(),
        bold: item.bold,
        italic: item.italic,
        size: item.size,
        alignment: item.alignment.map(|a| a.as_str().to_owned()),
        borders: item.borders.as_ref().map(|b| RegionBorders {
            style: Some(b.style.as_str().to_owned()),
            sides: b.sides.iter().flatten().map(|s| s.as_str().to_owned()).collect(),
            color: b.color.clone(),
            width: b.width,
        }),
        band_size: item.band_size,
        clear: item.clear,
    }
}

/// Registers `word_styles` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordStyles>(word_styles, crate::Hosting::PLAIN);
}
