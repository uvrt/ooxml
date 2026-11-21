//! Owns: [`StyleEdit`], `word_styles`' edits of the style sheet as docx-agent's `Styles`
//! makes them (`edit/styles.py`, `edit/table_styles.py`, `edit/language.py`): a style
//! added (paragraph, character, or table with its regions), modified (run, paragraph and
//! shading formatting, tab stops, a list level linked or unlinked, a table style's
//! regions), removed (its uses pointed at a replacement), the unused ones purged, the
//! table style new tables get, and the document's (or a style's) language -- each one
//! command, one history step.
//!
//! Must not: write but through the sink, or know a tool's JSON. What an action reads is
//! declared: the style sheet, the settings, the theme and the numbering parts it reads,
//! the whole document where the outcome depends on every story (a style's uses, the text
//! column's width, the runs' own languages).

mod add;
mod default;
mod language;
mod link;
mod modify;
mod read;
mod regions;
mod remove;
mod shading;
mod sheet;
mod tabs;

pub(crate) use default::settings_part;
pub(crate) use language::own_languages;
pub use language::{languages, word_tag, Languages, WORD_TAG_SLOTS};
pub use read::{
    content_uses, declared, find_style, looks_needed, read_paragraph, read_regions, read_run, read_tabs, style_link,
    table_default, usage, Declared, ReadStop, Region, StyleLink, Usage, LOOK_OF, REGIONS,
};
pub use regions::{RegionBorders, RegionValues, BORDER_SIDES, BORDER_STYLES};
pub use shading::read_shading;
pub(crate) use sheet::REL_STYLES;
pub use tabs::{TabStop, TAB_ALIGNMENTS, TAB_LEADERS};

use crate::format::{check, Setting};
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};

/// Why a style edit was refused, in the words docx-agent's tool answers with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleRefusalKind {
    /// No such style (docx-agent's `StyleError`): the tool lists the styles there are.
    NotFound,
    /// A value given wrong (a region's, a language's): the field and the values that do.
    Invalid,
}

/// A style edit's refusal: what, the field it names and the options that would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleRefusal {
    /// Which kind of refusal.
    pub kind: StyleRefusalKind,
    /// The message.
    pub message: String,
    /// The argument it names, if any.
    pub field: Option<String>,
    /// The values that would do.
    pub options: Vec<String>,
}

impl std::fmt::Display for StyleRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A style not found, as docx-agent's `StyleError`.
pub(crate) fn not_found(message: String) -> EditError {
    EditError::Style(StyleRefusal {
        kind: StyleRefusalKind::NotFound,
        message,
        field: None,
        options: Vec::new(),
    })
}

/// A value refused with its field and options (docx-agent's `RegionError`, `LanguageError`).
pub(crate) fn invalid(message: String, field: Option<&str>, options: Vec<String>) -> EditError {
    EditError::Style(StyleRefusal {
        kind: StyleRefusalKind::Invalid,
        message,
        field: field.map(str::to_owned),
        options,
    })
}

/// The formatting a style is given: run and paragraph settings (as `word_format` takes
/// them) and its shading (`Some(None)`: removed).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleFormatting {
    /// Run settings, in order.
    pub run: Vec<Setting>,
    /// Paragraph settings, in order.
    pub paragraph: Vec<Setting>,
    /// Its shading: RRGGBB, a theme colour (lighter or darker), or `None` to remove it.
    pub shading: Option<Option<String>>,
}

impl StyleFormatting {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.run.is_empty() && self.paragraph.is_empty() && self.shading.is_none()
    }
}

/// The list level `modify` links a paragraph style to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListLevel {
    /// Unlink the style (`list: "none"`).
    Unlink,
    /// Link it to `level` of the list `list` names: a list's number, or a style in it.
    Link {
        /// A number (digits) or a paragraph style's name.
        list: String,
        /// The level, 0-8.
        level: i64,
    },
}

/// What a style edit does.
#[derive(Debug, Clone, PartialEq)]
pub enum StyleAction {
    /// A new style.
    Add {
        /// Its name.
        name: String,
        /// `paragraph`, `character` or `table`.
        kind: String,
        /// The style it is based on.
        based_on: Option<String>,
        /// The style of the paragraph after.
        next_style: Option<String>,
        /// Its tab stops.
        tabs: Option<Vec<TabStop>>,
        /// A table style's regions.
        regions: Vec<RegionValues>,
        /// Its formatting.
        formatting: StyleFormatting,
    },
    /// A style's formatting changed.
    Modify {
        /// Its name.
        name: String,
        /// Its kind, when said.
        kind: Option<String>,
        /// A list level linked or unlinked.
        numbering: Option<ListLevel>,
        /// Its own tab stops replaced (`[]` removes them).
        tabs: Option<Vec<TabStop>>,
        /// A table style's regions.
        regions: Vec<RegionValues>,
        /// Its formatting.
        formatting: StyleFormatting,
    },
    /// A style removed, its uses pointed at `replacement`.
    Remove {
        /// Its name.
        name: String,
        /// Its kind, when said.
        kind: Option<String>,
        /// The style its uses name instead.
        replacement: Option<String>,
    },
    /// The custom styles nothing uses removed.
    PurgeUnused,
    /// The table style new tables get.
    SetDefault {
        /// Its name.
        name: String,
    },
    /// The document's language, or with `style` that style's alone.
    SetLanguage {
        /// A tag such as `fr-BE`.
        language: String,
        /// The one style to give it.
        style: Option<String>,
    },
}

/// `word_styles`' edit.
#[derive(Debug, Clone)]
pub struct StyleEdit {
    /// What it does.
    pub action: StyleAction,
}

/// What a style edit did, as docx-agent's `EditResult` says it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleStep {
    /// The style's id (`style:<id>`), when there is one.
    pub id: Option<String>,
    /// Whether the document changed.
    pub changed: bool,
    /// Ids made.
    pub created: Vec<String>,
    /// Ids removed.
    pub removed: Vec<String>,
    /// What to know.
    pub warnings: Vec<String>,
}

impl StyleStep {
    fn of(id: Option<String>) -> StyleStep {
        StyleStep {
            id,
            changed: true,
            ..StyleStep::default()
        }
    }
}

impl Command<WordDoc> for StyleEdit {
    type Out = StyleStep;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        match &self.action {
            StyleAction::Add { formatting, .. } | StyleAction::Modify { formatting, .. } => {
                check(&formatting.run)?;
                check(&formatting.paragraph)
            }
            _ => Ok(()),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<StyleStep> {
        // `word_styles` is never tracked (docx-agent's `track=False`)
        let mut sink = EditSink::new(tx, crate::RevisionPolicy::Untracked);
        let mut space = Space::default();
        let step = match &self.action {
            StyleAction::Add {
                name,
                kind,
                based_on,
                next_style,
                tabs,
                regions,
                formatting,
            } => add::add(
                &mut sink,
                &mut space,
                &add::NewStyle {
                    name,
                    kind,
                    based_on: based_on.as_deref(),
                    next_style: next_style.as_deref(),
                    tabs: tabs.as_deref(),
                    regions,
                    formatting,
                },
            )?,
            StyleAction::Modify {
                name,
                kind,
                numbering,
                tabs,
                regions,
                formatting,
            } => modify::modify(
                &mut sink,
                &mut space,
                &modify::Change {
                    name,
                    kind: kind.as_deref(),
                    numbering: numbering.as_ref(),
                    tabs: tabs.as_deref(),
                    regions,
                    formatting,
                },
            )?,
            StyleAction::Remove {
                name,
                kind,
                replacement,
            } => remove::remove(&mut sink, &mut space, name, kind.as_deref(), replacement.as_deref())?,
            StyleAction::PurgeUnused => remove::purge_unused(&mut sink, &mut space)?,
            StyleAction::SetDefault { name } => default::set_default(&mut sink, &mut space, name)?,
            StyleAction::SetLanguage { language, style } => {
                language::set_language(&mut sink, &mut space, language, style.as_deref())?
            }
        };
        // the parts whose XML changed (docx-agent writes the style sheet again unchanged
        // where it marks it dirty; that is lxml's serialization, not a change)
        space.finish_changed(&mut sink)?;
        Ok(step)
    }
}
