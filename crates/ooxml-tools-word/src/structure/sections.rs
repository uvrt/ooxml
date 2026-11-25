//! Owns: `word_sections` (docx-agent's `tools/structure.py`, `word_sections`): its
//! definition, its list, and its edits through the edit core's section commands.
//!
//! Must not: change the document but through a command.

use super::common::{call, need, outcome, policy, refused, Outcome};
use crate::describe::sections_json;
use crate::review::Editing;
use ooxml_tools::{ArgEnum, Done, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::units::twips;
use ooxml_word_edit::{InsertSectionBreak, RemoveSectionBreak, SectionEdited, SectionValues, SetSection};
use ooxml_word_model::docx::sections::sections;
use ooxml_word_model::docx::sections::SectionBreakKind;
use serde_json::{json, Value};

/// What `word_sections` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionAction {
    /// List the sections.
    List,
    /// Insert a section break.
    InsertBreak,
    /// Remove a section break.
    RemoveBreak,
    /// Set a section's page setup.
    Set,
}

/// Where a section starts (`w:type`).
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionStart {
    /// On a new page.
    #[arg(rename = "nextPage")]
    NextPage,
    /// On the same page.
    #[arg(rename = "continuous")]
    Continuous,
    /// On the next even page.
    #[arg(rename = "evenPage")]
    EvenPage,
    /// On the next odd page.
    #[arg(rename = "oddPage")]
    OddPage,
    /// In the next column.
    #[arg(rename = "nextColumn")]
    NextColumn,
}

impl SectionStart {
    /// Its name as Word writes it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SectionStart::NextPage => "nextPage",
            SectionStart::Continuous => "continuous",
            SectionStart::EvenPage => "evenPage",
            SectionStart::OddPage => "oddPage",
            SectionStart::NextColumn => "nextColumn",
        }
    }

    /// The section start it names.
    #[must_use]
    pub fn kind(self) -> SectionBreakKind {
        match self {
            SectionStart::NextPage => SectionBreakKind::NextPage,
            SectionStart::Continuous => SectionBreakKind::Continuous,
            SectionStart::EvenPage => SectionBreakKind::EvenPage,
            SectionStart::OddPage => SectionBreakKind::OddPage,
            SectionStart::NextColumn => SectionBreakKind::NextColumn,
        }
    }
}

/// A page's orientation.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Taller than wide.
    Portrait,
    /// Wider than tall.
    Landscape,
}

/// Page margins, in points.
// Today's schema describes these fields not at all: a doc comment would become a description.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct Margins {
    #[arg(minimum = 0, maximum = 1584)]
    pub top: Option<f64>,
    #[arg(minimum = 0, maximum = 1584)]
    pub right: Option<f64>,
    #[arg(minimum = 0, maximum = 1584)]
    pub bottom: Option<f64>,
    #[arg(minimum = 0, maximum = 1584)]
    pub left: Option<f64>,
}

/// `word_sections`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "word_sections",
    description = include_str!("word_sections.md"),
    group = "word_structure",
    mutates,
    places = "sides",
    place_own("after", "before", "section", "target", "where")
)]
pub struct WordSections {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: SectionAction,
    /// insert_break: the block before.
    #[arg(refs)]
    pub after: Option<String>,
    /// insert_break: the block after.
    #[arg(refs)]
    pub before: Option<String>,
    /// remove_break, set: section (s:...; s:body the last).
    #[arg(refs)]
    pub section: Option<String>,
    /// Where the next section starts. Default nextPage.
    pub start: Option<SectionStart>,
    /// set.
    pub orientation: Option<Orientation>,
    /// set.
    #[arg(minimum = 72, maximum = 1584)]
    pub page_width: Option<f64>,
    /// set.
    #[arg(minimum = 72, maximum = 1584)]
    pub page_height: Option<f64>,
    /// set.
    pub margins: Option<Margins>,
    /// set: text columns.
    #[arg(minimum = 1, maximum = 45)]
    pub columns: Option<f64>,
}

impl From<SectionEdited> for Outcome {
    fn from(out: SectionEdited) -> Self {
        Outcome {
            id: out.id,
            changed: out.changed,
            created: out.created,
            removed: out.removed,
            renamed: ooxml_word_model::ids::renames_text(&out.renamed),
            warnings: out.warnings,
        }
    }
}

/// `word_sections` (docx-agent's `word_sections`): list the sections; insert or remove a
/// break; set a section's page setup -- each one command.
#[must_use]
pub fn word_sections(args: &WordSections, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let list = call("word_sections", &[("doc", json!(args.doc)), ("action", json!("list"))]);
    match run(args, editing, &list) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

fn run(args: &WordSections, editing: &Editing<'_>, list: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let editor = editing.editor;
    match args.action {
        SectionAction::List => {
            let head = editor.current();
            let docx = head
                .doc()
                .docx()
                .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
            let mut done = Done::new("Sections");
            done.data = Some(sections_json(&docx, &sections(&docx)));
            Ok(ToolResult::done(done))
        }
        SectionAction::InsertBreak => {
            let (after, before) = sides(args.after.as_deref(), args.before.as_deref(), editing, list)?;
            let command = InsertSectionBreak {
                after: after.map(ooxml_word_model::address::Address::from),
                before: before.map(ooxml_word_model::address::Address::from),
                kind: args.start.unwrap_or(SectionStart::NextPage).kind(),
                policy: policy(editing),
            };
            Ok(match editor.run(editing.caller, &command) {
                Ok(done) => {
                    let id = done.out.id.clone().unwrap_or_default();
                    let summary = format!("Inserted a section break; the section before it is {id}");
                    outcome(&[done.out.into()], summary, Some(json!({"section": id})))
                }
                Err(error) => refused(error, list.clone()),
            })
        }
        SectionAction::RemoveBreak | SectionAction::Set => {
            let section = need(args.section.as_deref(), "section", list)?.to_owned();
            if args.action == SectionAction::RemoveBreak {
                let command = RemoveSectionBreak {
                    section: ooxml_word_model::address::Address::parse(&section),
                    policy: policy(editing),
                };
                return Ok(match editor.run(editing.caller, &command) {
                    Ok(done) => outcome(&[done.out.into()], format!("Removed the break ending {section}"), None),
                    Err(error) => refused(error, list.clone()),
                });
            }
            set(args, editing, list, section)
        }
    }
}

/// `set`: the section's page setup.
fn set(
    args: &WordSections,
    editing: &Editing<'_>,
    list: &TryCall,
    section: String,
) -> Result<ToolResult<Value>, ToolError> {
    let margins = args.margins.as_ref();
    let values = SectionValues {
        start: args.start.map(|s| s.name().to_owned()),
        orientation: args.orientation.map(|o| match o {
            Orientation::Portrait => "portrait".to_owned(),
            Orientation::Landscape => "landscape".to_owned(),
        }),
        page_width: args.page_width.map(twips),
        page_height: args.page_height.map(twips),
        margins: [
            margins.and_then(|m| m.top).map(twips),
            margins.and_then(|m| m.right).map(twips),
            margins.and_then(|m| m.bottom).map(twips),
            margins.and_then(|m| m.left).map(twips),
        ],
        #[allow(clippy::cast_possible_truncation)]
        columns: args.columns.map(|c| c as i64),
    };
    let names = values.names();
    if names.is_empty() {
        let way_out = WayOut::names([
            "start",
            "orientation",
            "page_width",
            "page_height",
            "margins",
            "columns",
        ])
        .unwrap_or_else(|| WayOut::call(list.clone()));
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "set needs a property to set",
            way_out,
        ));
    }
    let command = SetSection {
        section: ooxml_word_model::address::Address::parse(&section),
        values,
        policy: policy(editing),
    };
    Ok(match editing.editor.run(editing.caller, &command) {
        Ok(done) => {
            let mut edits: Vec<Outcome> = vec![done.out.into()];
            edits.push(Outcome {
                id: Some(section.clone()),
                changed: true,
                ..Outcome::default()
            });
            outcome(&edits, format!("Set {} of {section}", names.join(", ")), None)
        }
        Err(error) => refused(error, list.clone()),
    })
}

/// `after`/`before` as docx-agent's `_place` reads them: exactly one; `after` may say its
/// side (`before:X`, `after:X`) or be `end` (after the body's last block).
fn sides(
    after: Option<&str>,
    before: Option<&str>,
    editing: &Editing<'_>,
    list: &TryCall,
) -> Result<(Option<String>, Option<String>), ToolError> {
    match (after, before) {
        (Some(after), None) => {
            if after == "end" {
                let head = editing.editor.current();
                let docx = head
                    .doc()
                    .docx()
                    .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
                let last = body_blocks(&docx).into_iter().last().unwrap_or_default();
                return Ok((Some(last), None));
            }
            if let Some(anchor) = after.strip_prefix("before:") {
                return Ok((None, Some(anchor.to_owned())));
            }
            Ok((Some(after.strip_prefix("after:").unwrap_or(after).to_owned()), None))
        }
        (None, Some(before)) => Ok((None, Some(before.to_owned()))),
        _ => {
            let way_out =
                WayOut::names(["after", "before", "after_section"]).unwrap_or_else(|| WayOut::call(list.clone()));
            Err(ToolError::refusal(
                Refusal::InvalidArguments,
                "give exactly one of after, before",
                way_out,
            ))
        }
    }
}

/// The ids of the body's top-level blocks (docx-agent's `blocks()` ids), in order.
pub(crate) fn body_blocks(docx: &ooxml_word_model::docx::Docx) -> Vec<String> {
    let found = ooxml_word_model::docx::controls::controls(docx);
    ooxml_word_model::docx::outline::body_blocks(docx, &found)
        .into_iter()
        .filter_map(|b| b.id)
        .collect()
}

/// Registers `word_sections` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordSections>(word_sections, crate::Hosting::PLAIN);
}
