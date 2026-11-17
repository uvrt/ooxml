//! Owns: how one application draws -- Word's and PowerPoint's drawing rules as data
//! (ooxml-common's `drawingml.rules.DrawingRules`, less the colour rules, which resolution
//! applies before anything reaches the painter). Each field's measurement is documented
//! where ooxml-common keeps it.
//!
//! Must not: draw, or hold any rule as code that branches on an application's name.

use ooxml_fonts::KerningSource;

/// How a gradient is laid over its shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gradients {
    /// Across the shape's box in box units, stops as stated (PowerPoint's, as pptx2svg draws).
    BoundingBox,
    /// Word's measured geometry.
    Office,
}

/// How a preset dash is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dashes {
    /// The preset's dash and gap times the width, the line's cap on every dash.
    StrokeWidth,
    /// Word's: a round cap shortens each dash by the width; a square cap draws butt dashes.
    Office,
}

/// Where a pattern fill's cell is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternPhase {
    /// The shape's corner.
    Shape,
    /// The page's corner, square to the page.
    Page,
}

/// How a custom path is put on its shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomPathStrokes {
    /// By a `scale()` on the path, which scales its outline too.
    Scaled,
    /// The coordinates scaled, the outline at its stated width.
    Stated,
}

/// Where a text body's first baseline stands at or below single spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstBaseline {
    /// The measurer's ascent ratio, scaled with the line.
    Box,
    /// The spaced line box less the face's descent.
    Descent,
}

/// What a text body's autofit does when the file is drawn as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autofit {
    /// What the file stores, applied (PowerPoint's).
    Stored,
    /// Shrink or grow to fit (Word's, unmeasured).
    Fit,
}

/// How bold East Asian text in a face with no bold cut is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticBold {
    /// Asked for as bold.
    Ask,
    /// The upright outline stroked in the text's colour.
    Stroke,
}

/// What a paragraph of only whitespace is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlankParagraphs {
    /// An empty line; an empty paragraph still advances its number.
    Empty,
    /// Text: it draws its bullet; a paragraph of nothing is not numbered.
    Text,
}

/// One application's drawing rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawingRules {
    /// The application's name.
    pub name: &'static str,
    /// Gradients.
    pub gradients: Gradients,
    /// Dashes.
    pub dashes: Dashes,
    /// Arrowheads as SVG markers (`true`) or Word's drawn heads.
    pub arrowhead_markers: bool,
    /// The join of an outline that states none.
    pub default_join: Option<&'static str>,
    /// A pattern's cell registration.
    pub pattern_phase: PatternPhase,
    /// How far inside its text rectangle a shape lays its text, as a fraction of its
    /// outline's width.
    pub text_outline_inset: f64,
    /// Custom paths.
    pub custom_path_strokes: CustomPathStrokes,
    /// The resolution a run's glyphs are drawn on, dots an inch.
    pub text_size_grid: Option<u32>,
    /// The first baseline.
    pub first_baseline: FirstBaseline,
    /// Autofit.
    pub autofit: Autofit,
    /// Which kern pairs a line is laid out with.
    pub kerning: KerningSource,
    /// Synthetic bold.
    pub synthetic_bold: SyntheticBold,
    /// Blank paragraphs.
    pub blank_paragraphs: BlankParagraphs,
}

/// PowerPoint, as pptx2svg reproduces it.
pub const POWERPOINT: DrawingRules = DrawingRules {
    name: "powerpoint",
    gradients: Gradients::BoundingBox,
    dashes: Dashes::StrokeWidth,
    arrowhead_markers: true,
    default_join: None,
    pattern_phase: PatternPhase::Shape,
    text_outline_inset: 0.0,
    custom_path_strokes: CustomPathStrokes::Scaled,
    text_size_grid: None,
    first_baseline: FirstBaseline::Box,
    autofit: Autofit::Stored,
    kerning: KerningSource::POWERPOINT,
    synthetic_bold: SyntheticBold::Stroke,
    blank_paragraphs: BlankParagraphs::Text,
};

/// Word, as docx2svg measured it.
pub const WORD: DrawingRules = DrawingRules {
    name: "word",
    gradients: Gradients::Office,
    dashes: Dashes::Office,
    arrowhead_markers: false,
    default_join: Some("round"),
    pattern_phase: PatternPhase::Page,
    text_outline_inset: 0.5,
    custom_path_strokes: CustomPathStrokes::Stated,
    text_size_grid: Some(300),
    first_baseline: FirstBaseline::Descent,
    autofit: Autofit::Fit,
    kerning: KerningSource::WORD,
    synthetic_bold: SyntheticBold::Ask,
    blank_paragraphs: BlankParagraphs::Empty,
};
