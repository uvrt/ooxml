//! Owns: how an EMF+ pen strokes -- its width centred on the outline (at least a device
//! pixel; an inset pen's whole stroke inside a closed figure), its caps (flat, square and
//! round as SVG's, the others and differing start and end caps as filled shapes; an arrow
//! two widths across and sqrt(3) widths long, the line ending one width before it), joins
//! (`Miter` past the limit cut off, `MiterClipped` bevelled), dashes in widths with their
//! offset and dash caps, and compound pens as masks of nested strokes.
//!
//! Must not: fill (`paint` does).

use super::objects::{
    flatten, Pen, CAP_ARROW_ANCHOR, CAP_CUSTOM, CAP_DIAMOND_ANCHOR, CAP_FLAT, CAP_ROUND, CAP_ROUND_ANCHOR, CAP_SQUARE,
    CAP_SQUARE_ANCHOR, CAP_TRIANGLE,
};
use super::paint::Layer;
use super::{Plus, U_WORLD};
use crate::error::Halt;
use crate::geom;
use crate::player::paint::{dashes_for_caps, py_max};
use crate::player::transform_of;
use crate::pymath::{fsum, hypot};
use ooxml_scene::{
    Bounds, Def, Figure, Fill, Group, GroupTransform, LineCap, LineJoin, Matrix, Node, Paint, Path, PathData,
    ResourceKind, Segment, StrokeAttr,
};

const JOIN_MITER: u32 = 0;
const JOIN_BEVEL: u32 = 1;
const JOIN_ROUND: u32 = 2;

/// Dash patterns in pen widths (`DashStyle` 1 to 4), measured on Windows.
fn dash_pattern(dash: i32) -> Option<&'static [f64]> {
    match dash {
        1 => Some(&[3.0, 1.0]),
        2 => Some(&[1.0, 1.0]),
        3 => Some(&[3.0, 1.0, 1.0, 1.0]),
        4 => Some(&[3.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
        _ => None,
    }
}

impl Plus<'_, '_> {
    /// The pen's width in the shape's units (the space of `m`), at least one device pixel.
    fn pen_width(&self, pen: &Pen, m: &Matrix) -> f64 {
        let scale = Some(m.mean_scale()).filter(|s| *s != 0.0).unwrap_or(1.0);
        let mut width = if pen.matrix.is_identity() {
            pen.width
        } else {
            pen.width * pen.matrix.mean_scale()
        };
        if pen.unit != U_WORLD {
            width = width * self.unit(pen.unit, 0) / scale;
        }
        py_max(width, 1.0 / scale)
    }

    /// Strokes world-space figures with a pen, in the current transform and clip.
    pub fn stroke(&mut self, figures: Vec<Figure>, pen: Option<Pen>) -> Result<(), Halt> {
        let Some(pen) = pen.filter(|_| !figures.is_empty()) else {
            return Ok(());
        };
        let m = self.matrix();
        let width = self.pen_width(&pen, &m);
        let Some(layers) = self.paint_layers(&pen.brush, &m, true)?.filter(|l| !l.is_empty()) else {
            return Ok(());
        };
        let paint = layers.last().copied().unwrap_or(Layer {
            paint: Paint::None,
            opacity: None,
        });
        let inset = pen.align == 1 && figures.iter().all(|f| f.closed);
        let drawn_width = if inset { 2.0 * width } else { width };
        let mut attrs = vec![StrokeAttr::Paint(paint.paint)];
        if let Some(o) = paint.opacity {
            attrs.push(StrokeAttr::Opacity(o));
        }
        attrs.push(StrokeAttr::Width(drawn_width));
        let has_open = figures.iter().any(|f| !f.closed);
        let decorate = pen.start_cap != pen.end_cap || ![CAP_FLAT, CAP_SQUARE, CAP_ROUND].contains(&pen.start_cap);
        let mut cap = match pen.start_cap {
            _ if decorate => LineCap::Butt,
            CAP_SQUARE => LineCap::Square,
            CAP_ROUND => LineCap::Round,
            _ => LineCap::Butt,
        };
        if pen.start_cap == CAP_CUSTOM || pen.end_cap == CAP_CUSTOM {
            self.p.skip("EmfPlus custom line caps (drawn flat)");
        }
        let original = figures.clone();
        let figures: Vec<Figure> = if pen.start_cap == CAP_ARROW_ANCHOR || pen.end_cap == CAP_ARROW_ANCHOR {
            let at_start = if pen.start_cap == CAP_ARROW_ANCHOR { width } else { 0.0 };
            let at_end = if pen.end_cap == CAP_ARROW_ANCHOR { width } else { 0.0 };
            figures
                .into_iter()
                .map(|f| if f.closed { f } else { shortened(&f, at_start, at_end) })
                .collect()
        } else {
            figures
        };
        let (dash_attrs, flat_ends, dash_cap) = self.dashes(&pen, width, has_open);
        cap = dash_cap.unwrap_or(cap);
        attrs.extend(dash_attrs);
        if cap != LineCap::Butt {
            attrs.push(StrokeAttr::Cap(cap));
        }
        match pen.join {
            JOIN_ROUND => attrs.push(StrokeAttr::Join(LineJoin::Round)),
            JOIN_BEVEL => attrs.push(StrokeAttr::Join(LineJoin::Bevel)),
            join => {
                if join == JOIN_MITER {
                    attrs.push(StrokeAttr::Join(LineJoin::MiterClip));
                }
                attrs.push(StrokeAttr::MiterLimit(py_max(pen.miter, 1.0)));
            }
        }
        let data = PathData::new(figures);
        let body = self.stroke_body(&pen, &data, attrs, drawn_width, inset, flat_ends)?;
        let node = match transform_of(&m) {
            Some(t) => Node::Group(Group {
                transform: Some(GroupTransform::Matrix(t)),
                children: vec![body],
                ..Group::default()
            }),
            None => body,
        };
        self.write(node)?;
        if decorate && has_open {
            self.caps(&original, &pen, width, &m, paint)?;
        }
        Ok(())
    }

    /// The dash attributes, whether the dashes' round or triangle caps end flat at the
    /// line's own ends, and the cap the dashes ask for.
    fn dashes(&mut self, pen: &Pen, width: f64, has_open: bool) -> (Vec<StrokeAttr>, bool, Option<LineCap>) {
        let dashes: Option<Vec<f64>> = if let Some(p) = dash_pattern(pen.dash) {
            Some(p.iter().map(|v| v * width).collect())
        } else if pen.dash == 5 && !pen.dash_array.is_empty() {
            Some(pen.dash_array.iter().map(|v| py_max(*v, 0.0) * width).collect())
        } else {
            None
        };
        let Some(dashes) = dashes.filter(|d| !d.is_empty() && fsum(d.iter().copied()) > 0.0) else {
            return (Vec::new(), false, None);
        };
        let flat_ends = (pen.dash_cap == CAP_ROUND || pen.dash_cap == CAP_TRIANGLE) && has_open;
        let (cap, dashes) = match pen.dash_cap {
            CAP_ROUND => (LineCap::Round, dashes_for_caps(dashes, width, true)),
            CAP_TRIANGLE => {
                // A triangle dash cap, drawn as a flat end half as long (the same area).
                self.p.skip("EmfPlus triangle dash caps (drawn flat, half as long)");
                (LineCap::Butt, dashes_for_caps(dashes, width / 2.0, true))
            }
            _ => (LineCap::Butt, dashes_for_caps(dashes, width, false)),
        };
        let mut attrs = vec![StrokeAttr::Dashes(dashes)];
        if pen.dash_offset != 0.0 {
            attrs.push(StrokeAttr::DashOffset(pen.dash_offset * width));
        }
        (attrs, flat_ends, Some(cap))
    }

    fn stroke_body(
        &mut self,
        pen: &Pen,
        data: &PathData,
        attrs: Vec<StrokeAttr>,
        width: f64,
        inset: bool,
        flat_ends: bool,
    ) -> Result<Node, Halt> {
        let mut path = Path {
            data: data.clone(),
            fill: Some(Fill::of(Paint::None)),
            stroke: attrs.clone(),
            ..Path::default()
        };
        if inset {
            path.clip = Some(self.clip_path(data.clone(), None, false)?);
        }
        let mut body = Node::Path(path);
        let compound = bands(&pen.compound);
        if !pen.compound.is_empty() && compound.is_none() {
            self.p
                .skip("EmfPlus compound pens that are not symmetric (drawn as one stroke)");
        }
        if let Some(edges) = compound.filter(|e| !e.is_empty()) {
            body = self.compound(body, data, &attrs, width, &edges)?;
        }
        if flat_ends {
            body = self.flat_ends(body, data, &attrs, width)?;
        }
        Ok(body)
    }

    /// Round dash caps end flat at the line's own ends (measured): the dashes masked by the
    /// undashed line with flat ends.
    fn flat_ends(&mut self, body: Node, data: &PathData, attrs: &[StrokeAttr], width: f64) -> Result<Node, Halt> {
        let id = self.p.new_id(ResourceKind::Mask);
        let mut stroke = vec![
            StrokeAttr::Paint(Paint::Color(ooxml_scene::Rgb(255, 255, 255))),
            StrokeAttr::Width(width),
        ];
        stroke.extend(
            attrs
                .iter()
                .filter(|a| matches!(a, StrokeAttr::Join(_) | StrokeAttr::MiterLimit(_)))
                .cloned(),
        );
        self.p.define(Def::Mask {
            id,
            region: region(&data.figures, width),
            paths: vec![Path {
                data: data.clone(),
                fill: Some(Fill::of(Paint::None)),
                stroke,
                ..Path::default()
            }],
        })?;
        Ok(masked(body, id))
    }

    /// A compound pen: the stroke masked by nested centred strokes, white and black in turn,
    /// one for each edge of its bands (exact for a symmetric array).
    fn compound(
        &mut self,
        body: Node,
        data: &PathData,
        attrs: &[StrokeAttr],
        width: f64,
        edges: &[f64],
    ) -> Result<Node, Halt> {
        let id = self.p.new_id(ResourceKind::Mask);
        let rest: Vec<StrokeAttr> = attrs
            .iter()
            .skip_while(|a| !matches!(a, StrokeAttr::Width(_)))
            .skip(1)
            .map(|a| match a {
                StrokeAttr::Dashes(v) => StrokeAttr::InertDashes(v.clone()),
                StrokeAttr::DashOffset(v) => StrokeAttr::InertDashOffset(*v),
                other => other.clone(),
            })
            .collect();
        let paths = edges
            .iter()
            .enumerate()
            .map(|(i, edge)| {
                let c = if i % 2 == 0 { 255 } else { 0 };
                let mut stroke = vec![
                    StrokeAttr::Paint(Paint::Color(ooxml_scene::Rgb(c, c, c))),
                    StrokeAttr::Width((1.0 - 2.0 * edge) * width),
                ];
                stroke.extend(rest.iter().cloned());
                Path {
                    data: data.clone(),
                    fill: Some(Fill::of(Paint::None)),
                    stroke,
                    ..Path::default()
                }
            })
            .collect();
        self.p.define(Def::Mask {
            id,
            region: region(&data.figures, width),
            paths,
        })?;
        Ok(masked(body, id))
    }

    /// Line caps SVG has no `stroke-linecap` for (and differing start and end caps), as
    /// filled shapes at the ends of the open figures.
    fn caps(&mut self, figures: &[Figure], pen: &Pen, width: f64, m: &Matrix, paint: Layer) -> Result<(), Halt> {
        let mut shapes = Vec::new();
        for fig in figures.iter().filter(|f| !f.closed) {
            let Some(((p0, d0), (p1, d1))) = ends(fig) else {
                continue;
            };
            for (cap, point, direction) in [(pen.start_cap, p0, d0), (pen.end_cap, p1, d1)] {
                if let Some(shape) = cap_shape(cap, point, direction, width) {
                    shapes.push(shape);
                }
            }
        }
        if shapes.is_empty() {
            return Ok(());
        }
        self.write(Node::Path(Path {
            data: PathData::new(shapes),
            transform: transform_of(m),
            fill: Some(paint.fill(false)),
            ..Path::default()
        }))
    }
}

fn masked(body: Node, id: ooxml_scene::ResourceId) -> Node {
    Node::Group(Group {
        mask: Some(id),
        children: vec![body],
        ..Group::default()
    })
}

/// A compound array's edges from the outside in, when it is symmetric about the pen's
/// centre line.
fn bands(compound: &[f64]) -> Option<Vec<f64>> {
    let mut values: Vec<f64> = compound.iter().map(|v| v.clamp(0.0, 1.0)).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n < 2 || n % 2 == 1 {
        return None;
    }
    let symmetric = (0..n).all(|i| {
        let (a, b) = (
            values.get(i).copied().unwrap_or(0.0),
            values.get(n - 1 - i).copied().unwrap_or(0.0),
        );
        (a + b - 1.0).abs() <= 1e-3
    });
    symmetric.then(|| values.into_iter().take(n / 2).collect())
}

/// A mask's region: the figures' box and a margin wider than any stroke on them.
fn region(figures: &[Figure], width: f64) -> Bounds {
    let pts: Vec<(f64, f64)> = figures.iter().flat_map(|f| flatten(f, 2)).collect();
    let min = |v: &mut dyn Iterator<Item = f64>| v.fold(f64::INFINITY, |a, b| if b < a { b } else { a });
    let max = |v: &mut dyn Iterator<Item = f64>| v.fold(f64::NEG_INFINITY, |a, b| if b > a { b } else { a });
    let (x0, x1) = (min(&mut pts.iter().map(|p| p.0)), max(&mut pts.iter().map(|p| p.0)));
    let (y0, y1) = (min(&mut pts.iter().map(|p| p.1)), max(&mut pts.iter().map(|p| p.1)));
    let pad = 2.0 * width + 1.0;
    Bounds {
        x: x0 - pad,
        y: y0 - pad,
        width: x1 - x0 + 2.0 * pad,
        height: y1 - y0 + 2.0 * pad,
    }
}

type End = ((f64, f64), (f64, f64));

/// `(start, outward direction)` and `(end, outward direction)` of an open figure.
fn ends(fig: &Figure) -> Option<(End, End)> {
    let pts = flatten(fig, 6);
    if pts.len() < 2 {
        return None;
    }
    let direction = |a: (f64, f64), b: (f64, f64)| {
        let (dx, dy) = (a.0 - b.0, a.1 - b.1);
        let length = hypot(dx, dy);
        (length != 0.0).then(|| (dx / length, dy / length))
    };
    let first = *pts.first()?;
    let last = *pts.last()?;
    let start = pts.iter().skip(1).find_map(|q| direction(first, *q))?;
    let end = pts.iter().rev().skip(1).find_map(|q| direction(last, *q))?;
    Some(((first, start), (last, end)))
}

fn cap_shape(cap: i32, point: (f64, f64), direction: (f64, f64), width: f64) -> Option<Figure> {
    let ((px, py), (dx, dy)) = (point, direction);
    let (nx, ny) = (-dy, dx);
    let h = width / 2.0;
    // Square and triangle caps reach a little back over the line, so that no seam shows.
    let e = h / 4.0;
    let (px, py) = (px - dx * e, py - dy * e);
    match cap {
        CAP_SQUARE => {
            let h2 = h + e;
            let pts = [
                (px + nx * h, py + ny * h),
                (px + nx * h + dx * h2, py + ny * h + dy * h2),
                (px - nx * h + dx * h2, py - ny * h + dy * h2),
                (px - nx * h, py - ny * h),
            ];
            return geom::polyline(&pts, true);
        }
        CAP_ROUND => {
            let (px, py) = (px + dx * e, py + dy * e);
            return Some(geom::ellipse(px - h, py - h, px + h, py + h));
        }
        CAP_TRIANGLE => {
            let pts = [
                (px + nx * h, py + ny * h),
                (px + dx * (h + e), py + dy * (h + e)),
                (px - nx * h, py - ny * h),
            ];
            return geom::polyline(&pts, true);
        }
        _ => {}
    }
    let (px, py) = (px + dx * e, py + dy * e);
    match cap {
        CAP_ARROW_ANCHOR => {
            // Measured: two widths across, sqrt(3) widths long, the tip on the end point.
            let length = 3f64.sqrt() * width;
            let (bx, by) = (px - dx * length, py - dy * length);
            let pts = [
                (px, py),
                (bx + nx * width, by + ny * width),
                (bx - nx * width, by - ny * width),
            ];
            geom::polyline(&pts, true)
        }
        CAP_ROUND_ANCHOR => Some(geom::ellipse(px - width, py - width, px + width, py + width)),
        CAP_SQUARE_ANCHOR => Some(geom::rect(px - width, py - width, px + width, py + width)),
        CAP_DIAMOND_ANCHOR => {
            let pts = [
                (px + dx * width, py + dy * width),
                (px + nx * width, py + ny * width),
                (px - dx * width, py - dy * width),
                (px - nx * width, py - ny * width),
            ];
            geom::polyline(&pts, true)
        }
        _ => None,
    }
}

/// An open figure with its first and last straight segments shortened.
fn shortened(fig: &Figure, at_start: f64, at_end: f64) -> Figure {
    let mut out = fig.clone();
    if let (true, Some(Segment::Line(x1, y1))) = (at_start != 0.0, out.segments.first().copied()) {
        let (x0, y0) = fig.start;
        let length = hypot(x1 - x0, y1 - y0);
        if length > at_start {
            let t = at_start / length;
            out.start = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
        }
    }
    if let (true, Some(Segment::Line(x1, y1))) = (at_end != 0.0, out.segments.last().copied()) {
        let n = out.segments.len();
        let (x0, y0) = if n > 1 {
            out.segments.get(n - 2).map_or(out.start, Segment::end)
        } else {
            out.start
        };
        let length = hypot(x1 - x0, y1 - y0);
        if length > at_end {
            let t = at_end / length;
            if let Some(last) = out.segments.last_mut() {
                *last = Segment::Line(x1 - (x1 - x0) * t, y1 - (y1 - y0) * t);
            }
        }
    }
    out
}
