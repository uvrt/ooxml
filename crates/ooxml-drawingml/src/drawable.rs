//! Owns: the drawable elements -- shapes, connectors and groups with their transforms,
//! geometry, concrete fills and outlines, and resolved text bodies -- that a chart (and, in
//! P3c, a slide's or a document's drawing) is lowered to, equal to ooxml-common's
//! `drawingml/scene.py` elements. Every colour is a concrete `#rrggbb` string as its producer
//! wrote it (the case is kept: it is written into the SVG as it stands), every face a real
//! name, every inherited property merged down; lengths stay in EMU.
//!
//! Must not: lay text out, write SVG or paint (the render crates lower these to a scene),
//! or know where an element came from.

use crate::text::{Spacing, TabStop};

/// A concrete colour: `#rrggbb` as written, and an opacity 0-1.
#[derive(Debug, Clone, PartialEq)]
pub struct Colour {
    /// `#rrggbb`, in the case its producer wrote it.
    pub hex: String,
    /// The opacity.
    pub alpha: f64,
}

impl Colour {
    /// An opaque colour.
    pub fn opaque(hex: &str) -> Colour {
        Colour {
            hex: hex.to_owned(),
            alpha: 1.0,
        }
    }
}

impl From<ooxml_colour::ResolvedColour> for Colour {
    fn from(c: ooxml_colour::ResolvedColour) -> Colour {
        Colour {
            hex: c.hex(),
            alpha: c.alpha,
        }
    }
}

/// A gradient stop.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientStop {
    /// The position, 0-1.
    pub position: f64,
    /// The colour.
    pub colour: Colour,
}

/// A gradient fill.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientFill {
    /// The stops.
    pub stops: Vec<GradientStop>,
    /// The angle, degrees.
    pub angle: f64,
    /// `linear` or `radial`.
    pub gradient_type: String,
    /// A path gradient's centre.
    pub center_x: Option<f64>,
    /// Likewise.
    pub center_y: Option<f64>,
    /// `a:path@path`.
    pub path: Option<String>,
    /// `a:fillToRect`, 0-1 insets.
    pub focus: Option<[f64; 4]>,
    /// `a:lin@scaled`.
    pub scaled: Option<bool>,
    /// `rotWithShape`.
    pub rotate_with_shape: Option<bool>,
}

/// An image fill's tiling.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageTile {
    /// Offsets, EMU.
    pub tx: f64,
    /// Likewise.
    pub ty: f64,
    /// Scales.
    pub sx: f64,
    /// Likewise.
    pub sy: f64,
    /// `none`, `x`, `y` or `xy`.
    pub flip: String,
    /// The alignment.
    pub align: String,
}

/// A concrete fill.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    /// One colour.
    Solid(Colour),
    /// A gradient.
    Gradient(GradientFill),
    /// A picture's data and media type, with its tiling.
    Image {
        /// The data.
        data: String,
        /// The media type.
        mime: String,
        /// The tiling.
        tile: Option<ImageTile>,
    },
    /// A pattern in two colours.
    Pattern {
        /// The preset.
        preset: String,
        /// The foreground.
        foreground: Colour,
        /// The background.
        background: Colour,
    },
    /// No fill.
    None,
}

impl Fill {
    /// A solid fill of `hex`, opaque.
    pub fn solid(hex: &str) -> Fill {
        Fill::Solid(Colour::opaque(hex))
    }
}

/// An arrowhead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowEnd {
    /// Its type.
    pub kind: String,
    /// `sm`, `med` or `lg`.
    pub width: String,
    /// Likewise.
    pub length: String,
}

/// A concrete outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// Its width, EMU.
    pub width: f64,
    /// A solid or gradient fill.
    pub fill: Option<Fill>,
    /// The preset dash.
    pub dash_style: String,
    /// A custom dash.
    pub custom_dash: Option<Vec<f64>>,
    /// The cap.
    pub line_cap: Option<String>,
    /// The join.
    pub line_join: Option<String>,
    /// The head's arrowhead.
    pub head_end: Option<ArrowEnd>,
    /// The tail's arrowhead.
    pub tail_end: Option<ArrowEnd>,
    /// `cmpd`.
    pub compound: Option<String>,
}

impl Outline {
    /// A solid outline of `width` EMU in `colour`.
    pub fn solid(width: f64, colour: Colour) -> Outline {
        Outline {
            width,
            fill: Some(Fill::Solid(colour)),
            dash_style: "solid".into(),
            custom_dash: None,
            line_cap: None,
            line_join: None,
            head_end: None,
            tail_end: None,
            compound: None,
        }
    }
}

/// Where an element stands: its offset and extent (EMU), rotation (degrees) and flips.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Transform {
    /// Left.
    pub offset_x: f64,
    /// Top.
    pub offset_y: f64,
    /// Width.
    pub extent_width: f64,
    /// Height.
    pub extent_height: f64,
    /// Rotation, degrees clockwise.
    pub rotation: f64,
    /// Mirrored left to right.
    pub flip_h: bool,
    /// Mirrored top to bottom.
    pub flip_v: bool,
}

impl Transform {
    /// A box at `(x, y)` of `width` by `height`, EMU, unrotated.
    pub fn of(x: f64, y: f64, width: f64, height: f64) -> Transform {
        Transform {
            offset_x: x,
            offset_y: y,
            extent_width: width,
            extent_height: height,
            ..Transform::default()
        }
    }
}

/// One path of a custom geometry: its own coordinate space and its commands.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomPath {
    /// The path space's width.
    pub width: f64,
    /// Its height.
    pub height: f64,
    /// SVG-like commands (`M x y L x y C ... Z`).
    pub commands: String,
}

/// A shape's outline.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// A preset by name, with its adjust values in order.
    Preset {
        /// The preset's name.
        name: String,
        /// Adjust values stated.
        adjust: Vec<(String, f64)>,
    },
    /// Custom paths.
    Custom(Vec<CustomPath>),
}

impl Geometry {
    /// A preset with no adjust values.
    pub fn preset(name: &str) -> Geometry {
        Geometry::Preset {
            name: name.to_owned(),
            adjust: Vec::new(),
        }
    }
}

/// A run's resolved text outline.
#[derive(Debug, Clone, PartialEq)]
pub struct TextOutline {
    /// Its width, EMU.
    pub width: f64,
    /// Its colour.
    pub colour: Colour,
}

/// A hyperlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyperlink {
    /// The target.
    pub url: String,
    /// The tooltip.
    pub tooltip: Option<String>,
}

/// A run's resolved properties.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunProperties {
    /// The size, points.
    pub font_size: Option<f64>,
    /// The Latin face.
    pub font_family: Option<String>,
    /// The East Asian face.
    pub font_family_ea: Option<String>,
    /// The complex-script face.
    pub font_family_cs: Option<String>,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underlined.
    pub underline: bool,
    /// The underline's style where not single.
    pub underline_style: Option<String>,
    /// Struck through.
    pub strikethrough: bool,
    /// The colour.
    pub colour: Option<Colour>,
    /// The baseline shift, percent.
    pub baseline: f64,
    /// The hyperlink.
    pub hyperlink: Option<Hyperlink>,
    /// The text outline.
    pub outline: Option<TextOutline>,
    /// The highlight.
    pub highlight: Option<Colour>,
}

/// A run of text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextRun {
    /// The text.
    pub text: String,
    /// Its properties.
    pub properties: RunProperties,
}

/// A resolved bullet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bullet {
    /// None.
    None,
    /// A character.
    Char(String),
    /// An automatic number: its scheme and first number.
    AutoNumber {
        /// The scheme.
        scheme: String,
        /// The first number.
        start: i64,
    },
    /// A picture: its data and media type.
    Picture {
        /// The image data.
        data: String,
        /// Its media type.
        mime: String,
    },
}

/// A paragraph's resolved properties.
#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphProperties {
    /// `l`, `ctr`, `r` or `just`.
    pub alignment: Option<String>,
    /// The line spacing.
    pub line_spacing: Option<Spacing>,
    /// Space before.
    pub space_before: Spacing,
    /// Space after.
    pub space_after: Spacing,
    /// The level.
    pub level: i64,
    /// The bullet.
    pub bullet: Option<Bullet>,
    /// The bullet's face.
    pub bullet_font: Option<String>,
    /// The bullet's colour.
    pub bullet_colour: Option<Colour>,
    /// The bullet's size, 1/1000 percent.
    pub bullet_size_percent: Option<f64>,
    /// The bullet's size, points.
    pub bullet_size_points: Option<f64>,
    /// The left margin, EMU.
    pub margin_left: Option<f64>,
    /// The first line's indent, EMU.
    pub indent: Option<f64>,
    /// Tab stops.
    pub tab_stops: Vec<TabStop>,
}

impl Default for ParagraphProperties {
    fn default() -> Self {
        ParagraphProperties {
            alignment: None,
            line_spacing: None,
            space_before: Spacing::Percent(0.0),
            space_after: Spacing::Percent(0.0),
            level: 0,
            bullet: None,
            bullet_font: None,
            bullet_colour: None,
            bullet_size_percent: None,
            bullet_size_points: None,
            margin_left: None,
            indent: None,
            tab_stops: Vec::new(),
        }
    }
}

impl ParagraphProperties {
    /// Properties stating only an alignment.
    pub fn aligned(alignment: &str) -> ParagraphProperties {
        ParagraphProperties {
            alignment: Some(alignment.to_owned()),
            ..ParagraphProperties::default()
        }
    }
}

/// A paragraph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paragraph {
    /// Its runs.
    pub runs: Vec<TextRun>,
    /// Its properties.
    pub properties: ParagraphProperties,
    /// The end-of-paragraph run's properties.
    pub end_properties: Option<RunProperties>,
}

/// A body's resolved properties.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyProperties {
    /// `t`, `ctr` or `b`.
    pub anchor: String,
    /// Left inset, EMU.
    pub margin_left: f64,
    /// Right inset.
    pub margin_right: f64,
    /// Top inset.
    pub margin_top: f64,
    /// Bottom inset.
    pub margin_bottom: f64,
    /// `square` or `none`.
    pub wrap: String,
    /// `noAutofit`, `normAutofit` or `spAutofit`.
    pub auto_fit: String,
    /// The font scale.
    pub font_scale: f64,
    /// The line-spacing reduction.
    pub line_reduction: f64,
    /// Columns.
    pub columns: i64,
    /// The text direction.
    pub vert: String,
    /// Rotation, degrees.
    pub rotation: f64,
    /// The default tab size, EMU.
    pub default_tab_size: f64,
}

impl Default for BodyProperties {
    fn default() -> Self {
        BodyProperties {
            anchor: "t".into(),
            margin_left: 91_440.0,
            margin_right: 91_440.0,
            margin_top: 45_720.0,
            margin_bottom: 45_720.0,
            wrap: "square".into(),
            auto_fit: "noAutofit".into(),
            font_scale: 1.0,
            line_reduction: 0.0,
            columns: 1,
            vert: "horz".into(),
            rotation: 0.0,
            default_tab_size: 914_400.0,
        }
    }
}

/// A resolved text body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextBody {
    /// Its paragraphs.
    pub paragraphs: Vec<Paragraph>,
    /// Its body properties.
    pub properties: BodyProperties,
}

/// A shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// Where it stands.
    pub transform: Transform,
    /// Its outline geometry.
    pub geometry: Geometry,
    /// Its fill.
    pub fill: Option<Fill>,
    /// Its outline.
    pub outline: Option<Outline>,
    /// Its text.
    pub text: Option<TextBody>,
}

/// A connector.
#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    /// Where it stands.
    pub transform: Transform,
    /// Its geometry.
    pub geometry: Geometry,
    /// Its outline.
    pub outline: Option<Outline>,
}

/// A group: children mapped from its child space onto its box.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Its box.
    pub transform: Transform,
    /// Its children's space.
    pub child_transform: Transform,
    /// Its children.
    pub children: Vec<Element>,
}

/// A drawable element.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    /// A shape.
    Shape(Shape),
    /// A connector.
    Connector(Connector),
    /// A group.
    Group(Group),
}
