//! Owns: `check`'s definition, byte-equal to today's (ooxml-edit's shared `check`: one
//! schema for every kind, a deck's arguments and a document's side by side), and the
//! arguments' types.
//!
//! Must not: hold a handler.

use ooxml_tools::{ArgEnum, IntOrStr, ToolArgs};

/// What a check reports.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckInclude {
    /// A deck's text that does not fit.
    Fit,
    /// A deck's shapes that collide.
    Collisions,
    /// A deck's facts.
    Facts,
    /// A deck's design facts.
    Design,
    /// Validation problems.
    Validate,
    /// A document's pages since the last check.
    Reflow,
    /// A document's fields.
    Fields,
    /// A document's layout facts.
    Layout,
    /// The application's findings.
    App,
    /// Numbers in text that no open workbook's value holds.
    Numbers,
}

/// How a copy's numbered lists count.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyLists {
    /// Each copy's lists as its sample's.
    Sample,
    /// Numbered lists count on from the copy before.
    RunOn,
}

/// What a copy was made from.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct CopyReference {
    /// Its document. Default doc.
    pub doc: Option<String>,
    /// Its block or span.
    pub block: String,
}

/// A copy compared with its reference.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct CheckCopy {
    /// The copy's block or span.
    pub blocks: String,
    /// What it was made from.
    pub reference: CopyReference,
    /// run_on: numbered lists count on from the copy before.
    #[arg(
        value_alias("runs_on", "run_on"),
        value_alias("run on", "run_on"),
        value_alias("run-on", "run_on"),
        value_alias("continue", "run_on")
    )]
    pub lists: Option<CopyLists>,
}

/// A colour a scale's step gives text.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepText {
    /// Black or white, whichever reads.
    Auto,
    /// White.
    White,
    /// Black.
    Black,
}

/// One step of a scale. Its fields are undescribed, as today's are (`style_by_value`'s
/// definition describes them).
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct ScaleStep {
    pub fill: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub value: Option<IntOrStr<0>>,
    pub text: Option<StepText>,
    pub label: Option<String>,
}

/// What a scale's targets show.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleShow {
    /// The value.
    Value,
    /// The step's label.
    Label,
}

/// A scale whose steps style values (`style_by_value`'s). Its fields are undescribed, as
/// today's are.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq)]
pub struct CheckScale {
    pub steps: Vec<ScaleStep>,
    pub targets: Vec<String>,
    pub name: Option<String>,
    pub show: Option<ScaleShow>,
}

/// `check`'s arguments, shared by every format.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "check",
    description = include_str!("check.md"),
    documents("doc", "copies[].reference.doc")
)]
pub struct Check {
    /// Document id.
    pub doc: String,
    /// Decks: slide numbers from 1, or ids (s:256, $ref). Default all.
    #[arg(kinds("pptx"), refs)]
    pub slides: Option<Vec<IntOrStr<1>>>,
    /// Documents: page numbers from 1. Default all.
    #[arg(kinds("docx"), minimum = 1)]
    pub pages: Option<Vec<i64>>,
    /// Default: fit, collisions, validate (decks); validate, reflow, fields (documents).
    #[arg(
        kind_value("fit", "pptx"),
        kind_value("collisions", "pptx"),
        kind_value("facts", "pptx"),
        kind_value("design", "pptx"),
        kind_value("reflow", "docx"),
        kind_value("fields", "docx"),
        kind_value("numbers", "pptx", "docx"),
        kind_description("pptx", "Default: fit, collisions, validate."),
        kind_description("docx", "Default: validate, reflow, fields.")
    )]
    pub include: Option<Vec<CheckInclude>>,
    /// Decks: also text boxes that overlap where their text does not. Default false.
    #[arg(kinds("pptx"))]
    pub boxes: Option<bool>,
    /// Decks: also the gaps, padding and frame clearances below this, pt; 8 is usual.
    #[arg(kinds("pptx"), minimum = 0, maximum = 200)]
    pub spacing: Option<f64>,
    /// Decks: also text below this contrast ratio with what is behind it; 4.5 is WCAG's.
    #[arg(kinds("pptx"), minimum = 1, maximum = 21)]
    pub contrast: Option<f64>,
    /// Documents: also text below this, pt.
    #[arg(kinds("docx"), minimum = 1, maximum = 400)]
    pub min_size: Option<f64>,
    /// Documents: the allowed paragraph and table styles; others listed.
    #[arg(kinds("docx"))]
    pub styles: Option<Vec<String>>,
    /// Documents: blocks, spans p:A..t:B or sections s:2; their facts, pages.
    #[arg(kinds("docx"), single_as_list)]
    pub range: Option<Vec<String>>,
    /// Documents: copies compared with their reference.
    #[arg(kinds("docx"))]
    pub copies: Option<Vec<CheckCopy>>,
    /// As style_by_value's: its values with the wrong step or none.
    #[arg(kinds("pptx", "docx"))]
    pub scale: Option<CheckScale>,
}
