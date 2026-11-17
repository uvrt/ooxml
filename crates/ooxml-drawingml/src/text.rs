//! Owns: a text body as the XML states it -- body properties, a list style's levels,
//! paragraphs with their properties, runs (text, breaks and fields in document order) with
//! their properties -- equal to ooxml-common's text types (`drawingml/source_tree.py`, and
//! the bullet and spacing values of `drawingml/scene.py`). `None` means "not stated here".
//!
//! Must not: resolve a list style's inheritance or a theme font (the consumer's: a slide's
//! placeholders, a chart's text), lay text out, or read XML (`crate::read`).

use ooxml_colour::SourceColour;

/// A paragraph's spacing: points (1/100 pt) or a percentage (1/1000 percent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spacing {
    /// `a:spcPts`.
    Points(f64),
    /// `a:spcPct`.
    Percent(f64),
}

/// A tab stop: a position in EMU and an alignment (`l` where unstated).
#[derive(Debug, Clone, PartialEq)]
pub struct TabStop {
    /// The position.
    pub position: f64,
    /// `l`, `ctr`, `r` or `dec` (as stated).
    pub align: String,
}

/// A paragraph's bullet as stated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bullet {
    /// `a:buNone`.
    None,
    /// `a:buChar`: the character (`•` where unstated).
    Char(String),
    /// `a:buAutoNum`: a scheme and where it starts.
    AutoNumber {
        /// The scheme (`arabicPeriod` where unstated or unknown).
        scheme: String,
        /// The first number (1 where unstated).
        start: i64,
    },
    /// `a:buBlip`: a picture, by relationship.
    Picture(String),
}

/// `a:rPr`, `a:defRPr`, `a:endParaRPr` (at least one property stated).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunProperties {
    /// `b`.
    pub bold: Option<bool>,
    /// `i`.
    pub italic: Option<bool>,
    /// Whether underlined (`u` is not `none`; a hyperlink without `u` is).
    pub underline: Option<bool>,
    /// `u` when it is neither `none` nor `sng`.
    pub underline_style: Option<String>,
    /// Whether struck (`strike` is not `noStrike`).
    pub strike: Option<bool>,
    /// `baseline`, percent of the size.
    pub baseline: Option<f64>,
    /// `sz`, points.
    pub size: Option<f64>,
    /// `a:latin@typeface`.
    pub latin: Option<String>,
    /// `a:ea@typeface`.
    pub east_asian: Option<String>,
    /// `a:cs@typeface`.
    pub complex_script: Option<String>,
    /// The fill's colour (`hlink` for a hyperlink that states none).
    pub colour: Option<SourceColour>,
    /// `a:highlight`.
    pub highlight: Option<SourceColour>,
    /// `a:ln@w`, EMU.
    pub outline_width: Option<f64>,
    /// `a:ln`'s fill colour.
    pub outline_colour: Option<SourceColour>,
    /// `a:hlinkClick@r:id`.
    pub hyperlink: Option<String>,
    /// `a:hlinkClick@tooltip`.
    pub tooltip: Option<String>,
    /// `lang`.
    pub lang: Option<String>,
    /// `altLang`.
    pub alt_lang: Option<String>,
}

/// One run: text, a break (`"\n"`) or a field with its cached text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// The text.
    pub text: String,
    /// The run's properties.
    pub properties: Option<RunProperties>,
    /// `a:fld@type`, for a field.
    pub field_type: Option<String>,
}

/// `a:pPr` and the list style's levels (at least one property stated).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParagraphProperties {
    /// `l`, `ctr`, `r` or `just` (`justLow` and `dist` read as `just`).
    pub align: Option<String>,
    /// `lvl`.
    pub level: Option<i64>,
    /// `a:lnSpc`.
    pub line_spacing: Option<Spacing>,
    /// `a:spcBef`.
    pub space_before: Option<Spacing>,
    /// `a:spcAft`.
    pub space_after: Option<Spacing>,
    /// `marL`, EMU.
    pub margin_left: Option<f64>,
    /// `indent`, EMU.
    pub indent: Option<f64>,
    /// The bullet.
    pub bullet: Option<Bullet>,
    /// `a:buFont@typeface`.
    pub bullet_font: Option<String>,
    /// `a:buClr`.
    pub bullet_colour: Option<SourceColour>,
    /// `a:buSzPct@val`, 1/1000 percent.
    pub bullet_size_percent: Option<f64>,
    /// `a:buSzPts@val`, points.
    pub bullet_size_points: Option<f64>,
    /// `a:tabLst`.
    pub tabs: Option<Vec<TabStop>>,
    /// `a:defRPr`.
    pub default_run: Option<RunProperties>,
}

/// One paragraph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paragraph {
    /// Runs, breaks and fields, in order.
    pub runs: Vec<TextRun>,
    /// `a:pPr`.
    pub properties: Option<ParagraphProperties>,
    /// `a:endParaRPr`.
    pub end: Option<RunProperties>,
}

/// `a:lstStyle` and its kin: a default and nine levels (at least one stated).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListStyle {
    /// `a:defPPr`.
    pub default: Option<ParagraphProperties>,
    /// `a:lvl1pPr` to `a:lvl9pPr`.
    pub levels: Box<[Option<ParagraphProperties>; 9]>,
}

/// How a body's text autofits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autofit {
    /// `a:noAutofit`.
    None,
    /// `a:normAutofit`.
    Normal,
    /// `a:spAutoFit`.
    Shape,
}

/// `a:bodyPr` (at least one property stated).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyProperties {
    /// `lIns`, EMU.
    pub inset_left: Option<f64>,
    /// `rIns`.
    pub inset_right: Option<f64>,
    /// `tIns`.
    pub inset_top: Option<f64>,
    /// `bIns`.
    pub inset_bottom: Option<f64>,
    /// `t`, `ctr` or `b` (`just` and `dist` read as `t`).
    pub anchor: Option<String>,
    /// `square` or `none`.
    pub wrap: Option<String>,
    /// The autofit stated.
    pub autofit: Option<Autofit>,
    /// The font scale, 0-1 (1 for an autofit other than `a:normAutofit`).
    pub font_scale: Option<f64>,
    /// The line-spacing reduction, 0-1.
    pub line_reduction: Option<f64>,
    /// `numCol`, at least 1.
    pub columns: Option<i64>,
    /// `vert`.
    pub vert: Option<String>,
    /// `rot`, 1/60000 degree.
    pub rotation: Option<f64>,
    /// `defTabSz`, EMU.
    pub tab_size: Option<f64>,
}

/// A text body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextBody {
    /// The paragraphs.
    pub paragraphs: Vec<Paragraph>,
    /// `a:bodyPr`.
    pub properties: Option<BodyProperties>,
    /// `a:lstStyle`.
    pub list_style: Option<ListStyle>,
}
