//! Owns: the display list -- a [`Scene`]'s root viewport, its paint servers, clips and masks
//! ([`Def`]), and its drawing ([`Node`]: paths, rectangles, text runs with positioned
//! characters, images, groups and nested viewports) -- with every value typed.
//!
//! The attributes of a node are fields, written in one fixed order by the SVG writer, except
//! a stroke's: its attributes are a list ([`StrokeAttr`]) kept in the order the producer gave
//! them, which SVG gives no meaning but the reference output fixes.
//!
//! Must not: know any OOXML or metafile record, or write SVG (the writer does).

use crate::geometry::{Matrix, PathData};

/// A colour, 8 bits a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// What kind of resource an id names; its letter is part of the id (`c3`, `h4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// A clip path (`c`).
    Clip,
    /// A hatch pattern (`h`).
    Hatch,
    /// An image pattern (`p`).
    Pattern,
    /// A gradient (`g`).
    Gradient,
    /// A mask (`k`).
    Mask,
}

impl ResourceKind {
    /// The id's letter.
    pub fn letter(self) -> char {
        match self {
            ResourceKind::Clip => 'c',
            ResourceKind::Hatch => 'h',
            ResourceKind::Pattern => 'p',
            ResourceKind::Gradient => 'g',
            ResourceKind::Mask => 'k',
        }
    }
}

/// A resource's id within its [`Content`]: written `{prefix}{letter}{n}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceId {
    /// Its kind.
    pub kind: ResourceKind,
    /// Its number, unique in the content whatever the kind.
    pub n: u32,
}

/// What a fill or stroke paints with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    /// Nothing (`none`).
    None,
    /// A colour.
    Color(Rgb),
    /// A paint server: a pattern or gradient of the content's [`Def`]s.
    Server(ResourceId),
}

/// A fill: its paint, an opacity and the even-odd rule, each written only when present.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    /// `fill`.
    pub paint: Paint,
    /// `fill-opacity`.
    pub opacity: Option<f64>,
    /// `fill-rule="evenodd"`.
    pub even_odd: bool,
}

impl Fill {
    /// A fill of `paint` alone.
    pub fn of(paint: Paint) -> Fill {
        Fill {
            paint,
            opacity: None,
            even_odd: false,
        }
    }
}

/// A stroke's line cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    /// `butt`.
    Butt,
    /// `round`.
    Round,
    /// `square`.
    Square,
}

/// A stroke's line join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    /// `miter`.
    Miter,
    /// `miter-clip`.
    MiterClip,
    /// `round`.
    Round,
    /// `bevel`.
    Bevel,
}

/// One attribute of a stroke.
#[derive(Debug, Clone, PartialEq)]
pub enum StrokeAttr {
    /// `stroke`.
    Paint(Paint),
    /// `stroke-opacity`.
    Opacity(f64),
    /// `stroke-width`.
    Width(f64),
    /// `stroke-linecap`.
    Cap(LineCap),
    /// `stroke-linejoin`.
    Join(LineJoin),
    /// `stroke-miterlimit`.
    MiterLimit(f64),
    /// `stroke-dasharray`.
    Dashes(Vec<f64>),
    /// `stroke-dashoffset`.
    DashOffset(f64),
    /// A dash array kept but switched off (`data-d`): a compound pen's mask strokes are
    /// solid, with the dashes the pen's own stroke carries.
    InertDashes(Vec<f64>),
    /// A dash offset kept but switched off (`data-o`).
    InertDashOffset(f64),
}

/// A path element.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path {
    /// `d`.
    pub data: PathData,
    /// `transform`.
    pub transform: Option<Matrix>,
    /// The fill; `None` writes no fill attribute at all (SVG's default black, or a clip
    /// path's child).
    pub fill: Option<Fill>,
    /// `clip-rule="evenodd"` (a clip path's child).
    pub clip_even_odd: bool,
    /// The stroke's attributes, in order; empty for none.
    pub stroke: Vec<StrokeAttr>,
    /// `clip-path`.
    pub clip: Option<ResourceId>,
}

/// Axis-aligned cells filled alike, as one path in SVG's short relative form
/// (`M x y h w v h h -w z` a cell): a hatch tile's pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Cells {
    /// Each cell: `x, y, width, height`.
    pub cells: Vec<(f64, f64, f64, f64)>,
    /// Their fill.
    pub fill: Fill,
}

/// A rectangle element.
#[derive(Debug, Clone, PartialEq)]
pub struct Rect {
    /// `x`, `y`, `width`, `height`.
    pub x: f64,
    /// `y`.
    pub y: f64,
    /// `width`.
    pub width: f64,
    /// `height`.
    pub height: f64,
    /// `fill`.
    pub fill: Fill,
}

/// A text decoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoration {
    /// `underline`.
    Underline,
    /// `line-through`.
    LineThrough,
}

/// A text run's font.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    /// `font-size`.
    pub size: f64,
    /// `font-family`: the stack as written, unescaped.
    pub family: Option<String>,
    /// `font-weight="bold"`.
    pub bold: bool,
    /// `font-style="italic"`.
    pub italic: bool,
    /// `text-decoration`, in order.
    pub decorations: Vec<Decoration>,
}

/// Where a text run is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// `middle`.
    Middle,
    /// `end`.
    End,
}

/// A text element: characters at their positions (`x` and `y` lists), on one line.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    /// `x`: one value, or one a character.
    pub x: Vec<f64>,
    /// `y`: likewise.
    pub y: Vec<f64>,
    /// `transform`.
    pub transform: Option<Matrix>,
    /// Its fill.
    pub fill: Fill,
    /// Its font.
    pub font: Font,
    /// `text-anchor`.
    pub anchor: Option<Anchor>,
    /// The characters (whitespace preserved).
    pub content: String,
}

/// An image's bytes and type, written as a `data:` URI.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageData {
    /// The media type (`image/png`).
    pub mime: String,
    /// The encoded image.
    pub bytes: Vec<u8>,
}

/// An image element.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// `x`.
    pub x: Option<f64>,
    /// `y`.
    pub y: Option<f64>,
    /// `width`.
    pub width: f64,
    /// `height`.
    pub height: f64,
    /// `preserveAspectRatio="none"`: stretched to the box.
    pub stretch: bool,
    /// Drawn with nearest-neighbour sampling (`image-rendering`).
    pub pixelated: bool,
    /// The picture.
    pub data: ImageData,
}

/// A group's transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GroupTransform {
    /// `matrix(...)`.
    Matrix(Matrix),
    /// `translate(x y)`, then `rotate(angle)` when there is one (degrees, clockwise).
    TranslateRotate {
        /// The translation.
        x: f64,
        /// The translation.
        y: f64,
        /// The rotation.
        angle: Option<f64>,
    },
}

/// A group element.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    /// `transform`.
    pub transform: Option<GroupTransform>,
    /// `opacity`.
    pub opacity: Option<f64>,
    /// `clip-path`.
    pub clip: Option<ResourceId>,
    /// `mask`.
    pub mask: Option<ResourceId>,
    /// What it holds.
    pub children: Vec<Node>,
}

/// A rectangle: `x y width height`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// A nested viewport: `content`'s view box stretched over the box.
#[derive(Debug, Clone, PartialEq)]
pub struct Viewport {
    /// `x`.
    pub x: Option<f64>,
    /// `y`.
    pub y: Option<f64>,
    /// `width`.
    pub width: f64,
    /// `height`.
    pub height: f64,
    /// `viewBox`.
    pub view_box: Bounds,
    /// `overflow="hidden"`.
    pub clipped: bool,
    /// What it shows, with its own resources.
    pub content: Content,
}

/// One drawing element.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// A path.
    Path(Path),
    /// Cells of one fill.
    Cells(Cells),
    /// A rectangle.
    Rect(Rect),
    /// A text run.
    Text(Text),
    /// An image.
    Image(Image),
    /// A group.
    Group(Group),
    /// A nested viewport.
    Viewport(Viewport),
}

/// A gradient stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    /// `offset`.
    pub offset: f64,
    /// `stop-color`.
    pub color: Rgb,
    /// `stop-opacity`.
    pub opacity: Option<f64>,
}

/// A resource: a clip path, pattern, gradient or mask, all in user space.
#[derive(Debug, Clone, PartialEq)]
pub enum Def {
    /// A clip path, inside `parent` when there is one.
    ClipPath {
        /// Its id.
        id: ResourceId,
        /// The clip it is inside.
        parent: Option<ResourceId>,
        /// Its paths (their union).
        paths: Vec<Path>,
    },
    /// A tile repeated from the origin.
    Pattern {
        /// Its id.
        id: ResourceId,
        /// The tile's size.
        width: f64,
        /// The tile's size.
        height: f64,
        /// `patternTransform`.
        transform: Option<Matrix>,
        /// The tile's drawing.
        children: Vec<Node>,
    },
    /// A linear gradient from `(x1, y1)` to `(x2, y2)`.
    LinearGradient {
        /// Its id.
        id: ResourceId,
        /// The start.
        x1: f64,
        /// The start.
        y1: f64,
        /// The end.
        x2: f64,
        /// The end.
        y2: f64,
        /// `gradientTransform`.
        transform: Option<Matrix>,
        /// Its stops.
        stops: Vec<Stop>,
    },
    /// A luminance mask over a region.
    Mask {
        /// Its id.
        id: ResourceId,
        /// The region.
        region: Bounds,
        /// Its paths.
        paths: Vec<Path>,
    },
}

/// Resources and drawing that share one id space.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Content {
    /// What every id of this content starts with.
    pub id_prefix: String,
    /// The resources, in order.
    pub defs: Vec<Def>,
    /// The drawing, in order.
    pub body: Vec<Node>,
}

/// What made a scene, written on its root so a later pass can recognise it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A metafile player (`data-ooxml-metafile="gdi"`).
    Metafile,
}

/// The root viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Root {
    /// `width`.
    pub width: f64,
    /// `height`.
    pub height: f64,
    /// `viewBox`.
    pub view_box: Bounds,
    /// `preserveAspectRatio="none"`.
    pub stretch: bool,
    /// What made it.
    pub origin: Option<Origin>,
}

/// A drawing: its root viewport and content.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The root.
    pub root: Root,
    /// The content.
    pub content: Content,
}
