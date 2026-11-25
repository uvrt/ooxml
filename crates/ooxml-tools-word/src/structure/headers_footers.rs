//! Owns: `word_headers_footers` (docx-agent's `tools/structure.py`,
//! `word_headers_footers`): its definition and its edits through the edit core's story
//! commands; a page number after the text, a field (`SetStoryNumbered`).
//!
//! Must not: change the document but through a command.

use super::common::{call, outcome, policy, refused, Outcome};
use crate::review::Editing;
use ooxml_tools::{ArgEnum, ToolArgs, ToolResult};
use ooxml_word_edit::{LinkToPrevious, SetStoryNumbered, SetStoryText, StoryNumber, UnlinkFromPrevious};
use ooxml_word_model::docx::sections::{HeaderFooter, HeaderOrFooter, StoryKind};
use serde_json::{json, Value};

/// Which story.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// The header.
    Header,
    /// The footer.
    Footer,
}

/// The pages a header or footer is for.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryType {
    /// Most pages.
    Default,
    /// The section's first page.
    First,
    /// Even pages.
    Even,
}

/// What `word_headers_footers` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderAction {
    /// Set its text.
    Set,
    /// Remove the section's own.
    Remove,
    /// Link it to the previous section's.
    Link,
    /// Give the section its own.
    Unlink,
}

/// The page number a set adds.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageNumber {
    /// None.
    None,
    /// The bare number after the text.
    AfterText,
    /// 'Page X of Y'.
    #[arg(rename = "page_x_of_y")]
    PageXOfY,
}

/// `word_headers_footers`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_headers_footers",
    description = include_str!("word_headers_footers.md"),
    group = "word_structure",
    mutates
)]
pub struct WordHeadersFooters {
    /// Document id.
    pub doc: String,
    /// Section; s:body is the last.
    #[arg(refs)]
    pub section: String,
    /// Which story.
    pub which: Which,
    /// What to do.
    pub action: HeaderAction,
    /// Pages it is for. Default default (most).
    #[arg(rename = "type")]
    pub kind: Option<StoryType>,
    /// set: the text; \n between paragraphs.
    pub text: Option<String>,
    /// set: after_text, the bare number; page_x_of_y, 'Page X of Y' (' | ' after text).
    pub page_number: Option<PageNumber>,
}

impl StoryType {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.kind().as_str()
    }

    /// The kind it names.
    #[must_use]
    pub fn kind(self) -> StoryKind {
        match self {
            StoryType::Default => StoryKind::Default,
            StoryType::First => StoryKind::First,
            StoryType::Even => StoryKind::Even,
        }
    }
}

/// `word_headers_footers` (docx-agent's `word_headers_footers`): a section's header or
/// footer set, removed, linked to the previous section's or unlinked -- one command.
#[must_use]
pub fn word_headers_footers(args: &WordHeadersFooters, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let section = args.section.clone();
    let target = HeaderFooter {
        part: match args.which {
            Which::Header => HeaderOrFooter::Header,
            Which::Footer => HeaderOrFooter::Footer,
        },
        kind: args.kind.unwrap_or(StoryType::Default).kind(),
    };
    let way = call("word_sections", &[("doc", json!(args.doc)), ("action", json!("list"))]);
    let story = Story {
        section,
        which: target.part.as_str(),
        kind: target.kind.as_str(),
        target,
        way,
    };
    match args.action {
        HeaderAction::Remove | HeaderAction::Link => link(args, editing, &story),
        HeaderAction::Unlink => unlink(editing, &story),
        HeaderAction::Set => set(args, editing, &story),
    }
}

/// The story a call names: its section, which and kind, and the call that lists sections.
struct Story {
    section: String,
    which: &'static str,
    kind: &'static str,
    target: HeaderFooter,
    way: ooxml_tools::TryCall,
}

fn link(args: &WordHeadersFooters, editing: &Editing<'_>, story: &Story) -> ToolResult<Value> {
    let Story {
        section,
        which,
        kind,
        target,
        way,
    } = story;
    let (which, kind) = (*which, *kind);
    let editor = editing.editor;
    let command = LinkToPrevious {
        section: ooxml_word_model::address::Address::parse(&section),
        story: *target,
    };
    match editor.run(editing.caller, &command) {
        Ok(done) if !done.out.changed => {
            let note = format!("{section} had no {kind} {which} of its own: no change");
            let mut result = outcome(&[done.out.into()], note.clone(), None);
            result.warnings.push(note);
            result
        }
        Ok(done) => {
            let summary = if args.action == HeaderAction::Remove {
                format!("Removed the {kind} {which} of {section}")
            } else {
                format!("Linked the {kind} {which} of {section} to the previous section's")
            };
            outcome(&[done.out.into()], summary, None)
        }
        Err(error) => refused(error, way.clone()),
    }
}

fn unlink(editing: &Editing<'_>, story: &Story) -> ToolResult<Value> {
    let Story {
        section,
        which,
        kind,
        target,
        way,
    } = story;
    let (which, kind) = (*which, *kind);
    let editor = editing.editor;
    let command = UnlinkFromPrevious {
        section: ooxml_word_model::address::Address::parse(&section),
        story: *target,
    };
    match editor.run(editing.caller, &command) {
        Ok(done) => {
            let id = done.out.id.clone().unwrap_or_default();
            let changed = done.out.changed;
            let mut result = outcome(
                &[done.out.into()],
                format!("Unlinked: {id}"),
                Some(json!({"story": id})),
            );
            if !changed {
                let summary = format!("{section} had its own {kind} {which} already ({id}): no change");
                if let ooxml_tools::Outcome::Done(made) = &mut result.outcome {
                    made.summary.clone_from(&summary);
                }
                result.warnings.push(summary);
            }
            result
        }
        Err(error) => refused(error, way.clone()),
    }
}

fn set(args: &WordHeadersFooters, editing: &Editing<'_>, story: &Story) -> ToolResult<Value> {
    let Story {
        section,
        which,
        kind,
        target,
        way,
    } = story;
    let (which, kind) = (*which, *kind);
    let editor = editing.editor;
    let command = SetStoryText {
        section: ooxml_word_model::address::Address::parse(&section),
        story: *target,
        text: args.text.clone().unwrap_or_default(),
        policy: policy(editing),
    };
    let summary = format!("Set the {kind} {which} of {section}");
    let number = match args.page_number {
        Some(PageNumber::AfterText) => Some(StoryNumber::Page),
        Some(PageNumber::PageXOfY) => Some(StoryNumber::PageOfPages),
        Some(PageNumber::None) | None => None,
    };
    let Some(number) = number else {
        return match editor.run(editing.caller, &command) {
            Ok(done) => {
                let story = done.out.story.clone();
                let steps: Vec<Outcome> = done.out.steps.into_iter().map(Outcome::from).collect();
                outcome(&steps, summary, Some(json!({"story": story})))
            }
            Err(error) => refused(error, way.clone()),
        };
    };
    let today = ooxml_word_edit::Day::parse(editing.date).unwrap_or(ooxml_word_edit::Day {
        year: 1970,
        month: 1,
        day: 1,
    });
    let numbered = SetStoryNumbered {
        text: command,
        number,
        today,
        layout: editing.edit_layout(),
    };
    match editor.run(editing.caller, &numbered) {
        Ok(done) => {
            let out = done.out;
            let story = out.story.story.clone();
            let mut steps: Vec<Outcome> = out.story.steps.into_iter().map(Outcome::from).collect();
            if let Some(typed) = out.typed {
                steps.push(Outcome {
                    id: typed.id,
                    changed: true,
                    renamed: ooxml_word_model::ids::renames_text(&typed.renamed),
                    ..Outcome::default()
                });
            }
            steps.extend(out.fields.into_iter().map(Outcome::from));
            outcome(&steps, summary, Some(json!({"story": story})))
        }
        Err(error) => refused(error, way.clone()),
    }
}

/// Registers `word_headers_footers` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordHeadersFooters>(word_headers_footers, crate::Hosting::PLAIN.layout());
}
