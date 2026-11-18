//! Owns: `word_sections`' definition (docx-agent's `tools/structure.py`, `word_sections`).
//!
//! Must not: change the document but through a command.

use ooxml_tools::{ArgEnum, ToolArgs};

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
