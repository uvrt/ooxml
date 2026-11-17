//! Owns: a shape tree as the XML states it -- shapes, connectors, pictures, groups, tables
//! and the frames this crate can position but not read (charts, diagrams, objects) -- in
//! z-order, equal to ooxml-common's `drawingml/source_tree.py`. One tree type serves a
//! slide, a layout, a master, a group and a SmartArt drawing.
//!
//! Must not: resolve a placeholder's inheritance (a slide's: `ooxml-slides`), a colour
//! (`crate::resolve`) or a frame's content (a chart: `ooxml-chart`), or read XML.

use crate::source::{BlipEffects, EffectList, Fill, Geometry, GeometrySpec, Outline, ShapeStyle, Tile, Xfrm};
use crate::text::TextBody;

/// What every node says of itself in its non-visual properties.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeInfo {
    /// `cNvPr@name`.
    pub name: Option<String>,
    /// `cNvPr@id`, as stated.
    pub id: Option<String>,
    /// `cNvPr@descr`, where not empty.
    pub alt_text: Option<String>,
    /// `cNvPr@hidden`.
    pub hidden: bool,
}

/// `p:ph`: a placeholder's type and index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// `type`.
    pub kind: Option<String>,
    /// `idx`.
    pub idx: Option<i64>,
}

/// A custom geometry's text rectangle with the geometry its guides are evaluated in.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRect {
    /// The geometry, as written.
    pub geometry: Option<GeometrySpec>,
    /// `l`, `t`, `r`, `b`: guide names or literals.
    pub rect: [String; 4],
}

/// `sp`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shape {
    /// Name, id, description, hidden.
    pub info: NodeInfo,
    /// `p:ph`.
    pub placeholder: Option<Placeholder>,
    /// `a:xfrm`.
    pub xfrm: Option<Xfrm>,
    /// The geometry.
    pub geometry: Option<Geometry>,
    /// The fill.
    pub fill: Option<Fill>,
    /// `a:ln`.
    pub outline: Option<Outline>,
    /// `a:effectLst`.
    pub effects: Option<EffectList>,
    /// `p:style`.
    pub style: Option<ShapeStyle>,
    /// `p:txBody`.
    pub text: Option<TextBody>,
    /// `dsp:txXfrm`: a SmartArt shape's text box.
    pub text_xfrm: Option<Xfrm>,
    /// A custom geometry's `a:rect`.
    pub text_rect: Option<TextRect>,
    /// `cNvPr/a:hlinkClick@r:id`.
    pub hyperlink: Option<String>,
}

/// `cxnSp`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Connector {
    /// Name, id, description, hidden.
    pub info: NodeInfo,
    /// `a:xfrm`.
    pub xfrm: Option<Xfrm>,
    /// The geometry.
    pub geometry: Option<Geometry>,
    /// `a:ln`.
    pub outline: Option<Outline>,
    /// `a:effectLst`.
    pub effects: Option<EffectList>,
    /// `p:style`.
    pub style: Option<ShapeStyle>,
}

/// `pic`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Picture {
    /// Name, id, description, hidden.
    pub info: NodeInfo,
    /// `a:blip@r:embed`.
    pub blip: Option<String>,
    /// `asvg:svgBlip@r:embed`.
    pub svg: Option<String>,
    /// `p:ph`.
    pub placeholder: Option<Placeholder>,
    /// `a:xfrm`.
    pub xfrm: Option<Xfrm>,
    /// The geometry.
    pub geometry: Option<Geometry>,
    /// `a:ln`.
    pub outline: Option<Outline>,
    /// `a:effectLst`.
    pub effects: Option<EffectList>,
    /// The blip's effects.
    pub blip_effects: Option<BlipEffects>,
    /// `a:srcRect` as 0-1 insets.
    pub src_rect: Option<[f64; 4]>,
    /// `a:stretch/a:fillRect` as 0-1 insets.
    pub stretch: Option<[f64; 4]>,
    /// `a:tile`.
    pub tile: Option<Tile>,
    /// `cNvPr/a:hlinkClick@r:id`.
    pub hyperlink: Option<String>,
}

/// `a:tc`.
#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    /// `a:txBody`.
    pub text: Option<TextBody>,
    /// The cell's fill.
    pub fill: Option<Fill>,
    /// `a:lnT`.
    pub border_top: Option<Outline>,
    /// `a:lnB`.
    pub border_bottom: Option<Outline>,
    /// `a:lnL`.
    pub border_left: Option<Outline>,
    /// `a:lnR`.
    pub border_right: Option<Outline>,
    /// `gridSpan` (1 where unstated).
    pub grid_span: i64,
    /// `rowSpan` (1 where unstated).
    pub row_span: i64,
    /// `hMerge`.
    pub h_merge: bool,
    /// `vMerge`.
    pub v_merge: bool,
    /// `marL`, EMU.
    pub margin_left: Option<f64>,
    /// `marR`.
    pub margin_right: Option<f64>,
    /// `marT`.
    pub margin_top: Option<f64>,
    /// `marB`.
    pub margin_bottom: Option<f64>,
    /// `t`, `ctr` or `b` (`t` where unstated; `None` for any other value).
    pub anchor: Option<String>,
}

/// `a:tr`.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRow {
    /// `h`, EMU (0 where unstated).
    pub height: f64,
    /// The cells.
    pub cells: Vec<TableCell>,
}

/// Which regions of the table style apply (`a:tblPr`'s flags).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TableRegions {
    /// `firstRow`.
    pub first_row: bool,
    /// `lastRow`.
    pub last_row: bool,
    /// `firstCol`.
    pub first_col: bool,
    /// `lastCol`.
    pub last_col: bool,
    /// `bandRow`.
    pub band_row: bool,
    /// `bandCol`.
    pub band_col: bool,
}

/// `a:tbl` in a frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    /// The frame's name, id, description, hidden.
    pub info: NodeInfo,
    /// The frame's `xfrm`.
    pub xfrm: Option<Xfrm>,
    /// Column widths, EMU.
    pub columns: Vec<f64>,
    /// The rows.
    pub rows: Vec<TableRow>,
    /// Which style regions apply.
    pub regions: TableRegions,
    /// `a:tableStyleId`, trimmed, where not empty.
    pub style_id: Option<String>,
}

/// `grpSp`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Group {
    /// Name, id, description, hidden.
    pub info: NodeInfo,
    /// The group's placement.
    pub xfrm: Option<Xfrm>,
    /// Its children's space (`chOff`, `chExt`).
    pub child_xfrm: Option<Xfrm>,
    /// The group's fill.
    pub fill: Option<Fill>,
    /// `a:effectLst`.
    pub effects: Option<EffectList>,
    /// The members, in z-order.
    pub children: Vec<Node>,
}

/// A frame this crate positions but does not read: a chart, a diagram, an object, media.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    /// What it holds: `chart`, `diagram`, `chartex`, `ole`, or the graphic data's first
    /// child's local name (`graphicFrame` where there is none).
    pub what: String,
    /// Name, id, description, hidden.
    pub info: NodeInfo,
    /// The frame's `xfrm`.
    pub xfrm: Option<Xfrm>,
    /// The relationship of what can be drawn for it: a chart's part, a diagram's data, an
    /// object's picture.
    pub fallback: Option<String>,
}

/// One member of a shape tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// A shape.
    Shape(Box<Shape>),
    /// A connector.
    Connector(Box<Connector>),
    /// A picture.
    Picture(Box<Picture>),
    /// A table.
    Table(Box<Table>),
    /// A group.
    Group(Box<Group>),
    /// A frame positioned only.
    Frame(Frame),
}
