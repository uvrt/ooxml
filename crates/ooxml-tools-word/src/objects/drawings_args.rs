//! Owns: the definitions of `word_drawings` and `word_controls` (docx-agent's
//! `tools/objects.py`): their arguments, byte-equal to today's (`word_drawings` as the
//! toolbox without an asset catalogue defines it).
//!
//! Must not: change the document.

use super::format_table_args::Across;
use ooxml_tools::{ArgEnum, ToolArgs};

/// What `word_drawings` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingAction {
    /// List the drawings.
    List,
    /// Insert a picture.
    InsertPicture,
    /// Insert a text box.
    InsertTextBox,
    /// Insert a shape.
    InsertShape,
    /// Move a floating drawing.
    Move,
    /// Resize a drawing.
    Resize,
    /// Float a drawing.
    Float,
    /// Put a drawing in line with text.
    Inline,
    /// Set wrapping, order or alt text.
    Set,
}

/// What a drawing's `x` and `align` measure from.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Against {
    /// The column.
    Column,
    /// The margin.
    Margin,
    /// The page.
    Page,
}

/// How text wraps a floating drawing.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    /// Square.
    Square,
    /// Tight.
    Tight,
    /// Top and bottom.
    TopAndBottom,
    /// In front of the text.
    Front,
    /// Behind the text.
    Behind,
}

/// A drawing's z-order change.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZOrder {
    /// To the front.
    Front,
    /// To the back.
    Back,
    /// One forward.
    Forward,
    /// One backward.
    Backward,
}

/// `word_drawings`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(
    name = "word_drawings",
    description = include_str!("word_drawings.md"),
    group = "word_objects",
    mutates,
    places = "picture",
    place_own("at", "target"),
    place_find
)]
pub struct WordDrawings {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: DrawingAction,
    /// The drawing (d:...).
    #[arg(refs)]
    pub target: Option<String>,
    /// insert_picture: image blob handle, e.g. b2.
    pub image: Option<String>,
    /// insert_*: position, or after:/before: a block (a paragraph of its own).
    #[arg(refs)]
    pub at: Option<String>,
    /// insert_*: text occurring once to anchor after.
    pub find: Option<String>,
    /// move, float, insert_*: offset.
    #[arg(minimum = -1584, maximum = 1584)]
    pub x: Option<f64>,
    /// move, float, insert_*: offset.
    #[arg(minimum = -1584, maximum = 1584)]
    pub y: Option<f64>,
    /// float, insert_*: across, instead of x; own paragraph: its alignment.
    pub align: Option<Across>,
    /// What x and align measure from. Default column.
    pub against: Option<Against>,
    /// resize, insert_*.
    #[arg(minimum = 1, maximum = 1584, alias = "size_pt")]
    pub width: Option<f64>,
    /// resize, insert_text_box, insert_shape.
    #[arg(minimum = 1, maximum = 1584)]
    pub height: Option<f64>,
    /// float, set, insert_*: text wrapping; inserts float with it.
    pub wrap: Option<Wrap>,
    /// set: z-order.
    pub z_order: Option<ZOrder>,
    /// insert_text_box, insert_shape: the text inside.
    pub text: Option<String>,
    /// insert_shape: geometry, e.g. rect, ellipse.
    pub preset: Option<String>,
    /// insert_picture (needed), set: text for screen readers.
    pub alt_text: Option<String>,
    /// insert_picture: tint an SVG, theme colour or RRGGBB.
    pub color: Option<String>,
    /// insert_*: ref name for $name.
    #[arg(rename = "ref")]
    pub reference: Option<String>,
    /// Retry key: a repeat with the same key makes nothing new.
    pub key: Option<String>,
}

/// What `word_controls` does.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    /// List the content controls.
    List,
    /// Insert one.
    Insert,
    /// Fill one as typing would.
    Fill,
    /// Remove one, keeping its content.
    Remove,
}

/// A content control's type.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlType {
    /// Plain text.
    Text,
    /// Rich text.
    #[arg(rename = "rich-text")]
    RichText,
    /// A drop-down list.
    #[arg(rename = "drop-down")]
    DropDown,
    /// A combo box.
    #[arg(rename = "combo-box")]
    ComboBox,
    /// A date.
    Date,
    /// A checkbox.
    Checkbox,
}

/// `word_controls`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_controls",
    description = include_str!("word_controls.md"),
    group = "word_objects",
    mutates,
    places = "mark",
    place_own("at"),
    place_find
)]
pub struct WordControls {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: ControlAction,
    /// fill, remove: the control (cc:...).
    #[arg(refs)]
    pub control: Option<String>,
    /// insert: position, or find.
    #[arg(refs)]
    pub at: Option<String>,
    /// insert: text occurring once; goes after it.
    pub find: Option<String>,
    /// insert. Default text.
    #[arg(rename = "type")]
    pub kind: Option<ControlType>,
    /// insert drop-down, combo-box: the choices.
    pub items: Option<Vec<String>>,
    /// fill: text, choice, date or true/false.
    pub value: Option<String>,
    /// insert: a tag for programs.
    pub tag: Option<String>,
    /// insert: the title people see.
    pub title: Option<String>,
}
