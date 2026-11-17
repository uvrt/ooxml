//! Owns: DrawingML as the XML states it, before a theme or inheritance resolves it -- fills,
//! outlines, shape styles, effects, a picture's effects, transforms, geometry and the theme's
//! format scheme -- equal to ooxml-common's `drawingml/source.py`. `None` means "not stated
//! here: inherit"; no field gets a default the XML did not give it, except where the Python
//! reader gives one (each says so).
//!
//! Must not: resolve a colour (`crate::resolve`) or read XML (`crate::read`).

use ooxml_colour::SourceColour;

/// A rectangle's alignment (`ST_RectAlignment`): `tl`, `t`, `tr`, `l`, `ctr`, `r`, `bl`, `b`, `br`.
pub type Alignment = String;

/// A fill as stated.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    /// `a:solidFill`.
    Solid(SourceColour),
    /// `a:noFill`.
    None,
    /// `a:gradFill`.
    Gradient(GradientFill),
    /// `a:blipFill`: a picture, by relationship.
    Image(ImageFill),
    /// `a:pattFill`.
    Pattern(PatternFill),
    /// `a:grpFill`: the enclosing group's fill.
    Group,
}

/// One gradient stop.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientStop {
    /// The position, 0-1.
    pub position: f64,
    /// The colour.
    pub colour: SourceColour,
}

/// Whether a gradient runs along a line or out from a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientKind {
    /// `a:lin` (or neither shade stated).
    Linear,
    /// `a:path`.
    Radial,
}

/// `a:gradFill`.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientFill {
    /// The stops, in order (at least one).
    pub stops: Vec<GradientStop>,
    /// Linear or path.
    pub kind: GradientKind,
    /// A linear gradient's angle, 1/60000 degree (0 for a path gradient).
    pub angle: f64,
    /// A path gradient's centre, 0-1 of the box.
    pub centre: Option<(f64, f64)>,
    /// `a:path@path`: `circle`, `rect` or `shape`.
    pub path: Option<String>,
    /// `a:fillToRect` as 0-1 insets `(l, t, r, b)`.
    pub focus: Option<[f64; 4]>,
    /// `a:lin@scaled`.
    pub scaled: Option<bool>,
    /// `a:gradFill@rotWithShape`.
    pub rotate_with_shape: Option<bool>,
}

/// `a:tile`: offsets in EMU, scales as fractions, a flip and an alignment.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Horizontal offset.
    pub tx: f64,
    /// Vertical offset.
    pub ty: f64,
    /// Horizontal scale.
    pub sx: f64,
    /// Vertical scale.
    pub sy: f64,
    /// `none`, `x`, `y` or `xy`.
    pub flip: String,
    /// The alignment.
    pub align: Alignment,
}

/// `a:blipFill` as a fill.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageFill {
    /// `a:blip@r:embed`.
    pub blip: String,
    /// `asvg:svgBlip@r:embed`, the vector original.
    pub svg: Option<String>,
    /// `a:tile`.
    pub tile: Option<Tile>,
    /// `a:srcRect` as 0-1 insets.
    pub src_rect: Option<[f64; 4]>,
    /// `a:stretch/a:fillRect` as 0-1 insets.
    pub stretch: Option<[f64; 4]>,
}

/// `a:pattFill`.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternFill {
    /// The preset (`ltDnDiag` where unstated).
    pub preset: String,
    /// The foreground colour.
    pub foreground: SourceColour,
    /// The background colour.
    pub background: SourceColour,
}

/// An arrowhead (`a:headEnd`, `a:tailEnd`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEnd {
    /// `triangle`, `stealth`, `diamond`, `oval` or `arrow`.
    pub kind: String,
    /// `sm`, `med` or `lg`.
    pub width: String,
    /// `sm`, `med` or `lg`.
    pub length: String,
}

/// `a:ln` (and a cell's borders) as stated.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outline {
    /// The width, EMU.
    pub width: Option<f64>,
    /// The line's fill.
    pub fill: Option<Fill>,
    /// A preset dash.
    pub dash: Option<String>,
    /// A custom dash: dash and space pairs, as fractions of the width.
    pub custom_dash: Option<Vec<f64>>,
    /// `butt`, `round` or `square`.
    pub cap: Option<String>,
    /// `round`, `bevel` or `miter`.
    pub join: Option<String>,
    /// The head's arrowhead.
    pub head: Option<LineEnd>,
    /// The tail's arrowhead.
    pub tail: Option<LineEnd>,
    /// `a:ln@cmpd`.
    pub compound: Option<String>,
}

/// A style reference: `a:fillRef`, `a:lnRef`, `a:effectRef` (an index into the format
/// scheme) or `a:fontRef` (a font collection), with its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleRef {
    /// The index (0 for a font collection).
    pub idx: i64,
    /// The reference's colour.
    pub colour: Option<SourceColour>,
    /// `major`, `minor` or `none`, for `a:fontRef`.
    pub collection: Option<String>,
}

/// `p:style`: the shape's references into the theme.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapeStyle {
    /// `a:fillRef`.
    pub fill: Option<StyleRef>,
    /// `a:lnRef`.
    pub line: Option<StyleRef>,
    /// `a:effectRef`.
    pub effect: Option<StyleRef>,
    /// `a:fontRef`.
    pub font: Option<StyleRef>,
}

/// A shadow: blur and distance in EMU, direction in 1/60000 degree.
#[derive(Debug, Clone, PartialEq)]
pub struct Shadow {
    /// Blur radius.
    pub blur: f64,
    /// Distance.
    pub distance: f64,
    /// Direction.
    pub direction: f64,
    /// The colour.
    pub colour: SourceColour,
    /// An outer shadow's alignment (`b` where unstated).
    pub align: Alignment,
    /// An outer shadow's `rotWithShape` (true unless `0`).
    pub rotate_with_shape: bool,
}

/// `a:glow`.
#[derive(Debug, Clone, PartialEq)]
pub struct Glow {
    /// Radius, EMU.
    pub radius: f64,
    /// The colour.
    pub colour: SourceColour,
}

/// `a:effectLst` (at least one effect).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectList {
    /// `a:outerShdw`.
    pub outer_shadow: Option<Shadow>,
    /// `a:innerShdw`.
    pub inner_shadow: Option<Shadow>,
    /// `a:glow`.
    pub glow: Option<Glow>,
    /// `a:softEdge`'s radius, EMU.
    pub soft_edge: Option<f64>,
}

/// A picture's effects, from its `a:blip` (at least one).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlipEffects {
    /// `a:grayscl`.
    pub grayscale: bool,
    /// `a:biLevel`'s threshold, 0-1.
    pub bi_level: Option<f64>,
    /// `a:blur`: radius in EMU, and whether it grows.
    pub blur: Option<(f64, bool)>,
    /// `a:lum`: brightness and contrast, -1 to 1.
    pub luminance: Option<(f64, f64)>,
    /// `a:duotone`'s two colours.
    pub duotone: Option<(SourceColour, SourceColour)>,
    /// `a:clrChange`: from, to.
    pub colour_change: Option<(SourceColour, SourceColour)>,
    /// `a:alphaModFix`'s opacity, where below 1.
    pub alpha: Option<f64>,
}

/// `a:xfrm`: offset and extent in EMU, rotation in 1/60000 degree, flips.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Xfrm {
    /// Offset x.
    pub x: f64,
    /// Offset y.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
    /// Rotation.
    pub rotation: f64,
    /// `flipH`.
    pub flip_h: bool,
    /// `flipV`.
    pub flip_v: bool,
}

/// A custom geometry's path, evaluated at its own size into SVG path data.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomPath {
    /// The path's width.
    pub width: f64,
    /// The path's height.
    pub height: f64,
    /// SVG path data in the path's own space.
    pub commands: String,
}

/// A shape's geometry.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// `a:prstGeom`: a preset and its adjustments (`name -> value`, in first-stated order).
    Preset {
        /// The preset's name (`rect` where unstated).
        name: String,
        /// The adjustments stated as `val N`.
        adjust: Vec<(String, f64)>,
    },
    /// `a:custGeom`, evaluated (at least one path).
    Custom(Vec<CustomPath>),
}

/// A path command as written, for a consumer that evaluates the path at another size: a
/// letter (`M`, `L`, `A`, `Q`, `C`, `Z`) and its operands (guide names or literals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecCommand {
    /// The command letter.
    pub letter: char,
    /// Its operands as written.
    pub operands: Vec<String>,
}

/// One path of a geometry, as written.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecPath {
    /// `@fill`: `norm` where unstated.
    pub fill: String,
    /// Whether it is stroked (`@stroke` is not `0` or `false`).
    pub stroke: bool,
    /// `(@w, @h)` when the path has its own space.
    pub space: Option<(f64, f64)>,
    /// The commands.
    pub commands: Vec<SpecCommand>,
}

/// A geometry as written, unevaluated (ooxml-common's `parse_geometry_spec`).
#[derive(Debug, Clone, PartialEq)]
pub enum GeometrySpec {
    /// A preset: its name and adjustments as formulas, by name.
    Preset {
        /// The preset's name.
        name: String,
        /// `name -> formula`, in first-stated order.
        adjust: Vec<(String, String)>,
    },
    /// A custom geometry: adjustments, guides and paths.
    Custom {
        /// `a:avLst` as `(name, formula)`.
        adjustments: Vec<(String, String)>,
        /// `a:gdLst` as `(name, formula)`.
        guides: Vec<(String, String)>,
        /// The paths.
        paths: Vec<SpecPath>,
    },
}

/// `a:fmtScheme`: what a style reference's index selects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormatScheme {
    /// `a:fillStyleLst`.
    pub fill_styles: Vec<Fill>,
    /// `a:lnStyleLst`.
    pub line_styles: Vec<Outline>,
    /// `a:effectStyleLst` (an entry without effects is `None`).
    pub effect_styles: Vec<Option<EffectList>>,
    /// `a:bgFillStyleLst`.
    pub background_fill_styles: Vec<Fill>,
}

impl FormatScheme {
    /// The fill style `idx` selects: 1 to 3 the fill styles, 1001 to 1003 the background fill
    /// styles; 0, 1000 and anything out of range none (measured on PowerPoint and Word).
    pub fn fill_style(&self, idx: i64) -> Option<&Fill> {
        let (styles, index) = if idx >= 1000 {
            (&self.background_fill_styles, idx - 1001)
        } else {
            (&self.fill_styles, idx - 1)
        };
        usize::try_from(index).ok().and_then(|i| styles.get(i))
    }

    /// The line style `idx` selects (1-based; 0 none).
    pub fn line_style(&self, idx: i64) -> Option<&Outline> {
        usize::try_from(idx - 1).ok().and_then(|i| self.line_styles.get(i))
    }

    /// The effect style `idx` selects (1-based; 0 none), `None` too for an entry with none.
    pub fn effect_style(&self, idx: i64) -> Option<&EffectList> {
        usize::try_from(idx - 1)
            .ok()
            .and_then(|i| self.effect_styles.get(i))
            .and_then(Option::as_ref)
    }
}
