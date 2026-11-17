//! Owns: Word's arrowheads (ooxml-common's `fill.render_arrowheads`): each head drawn as a
//! shape of its own at an open path's end, sized by the line's width, in the outline's
//! colour, and how far the line is cut back under it so the shaft stops behind the tip --
//! measured on Word (the rules' `arrowhead_markers: false`).
//!
//! Must not: find a path's ends or cut it back (that is [`crate::paths`]), or draw
//! PowerPoint's markers (that is [`crate::fill::markers`]).

use crate::paths::PathEnd;
use crate::units::emu_to_px_at;
use ooxml_drawingml::drawable::{ArrowEnd, Fill, Outline};
use ooxml_scene::markup::{Command, Element, Op, Sep, Tag, Value};

/// A head's width or length as a multiple of the line's width.
fn size_factor(size: &str) -> f64 {
    match size {
        "sm" => 2.0,
        "lg" => 5.0,
        _ => 3.0,
    }
}

/// The width a thinner line's head is sized by, pt (measured at 1 pt: an `sm` head is 4 pt
/// long, `lg` 10).
const MINIMUM_UNIT_PT: f64 = 2.0;
/// Where a stealth head's notch is, as a fraction of its length from the tip.
const STEALTH_NOTCH: f64 = 0.6;

/// The heads of an open path, and how far to cut the path back at its start and its end.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Arrowheads {
    /// The heads, painted in the outline's colour.
    pub elements: Vec<Element>,
    /// How far to cut the path back at its first point.
    pub start: f64,
    /// Likewise at its last point.
    pub end: f64,
}

/// The paint every head shares: the outline's colour (a gradient's first stop) and alpha.
fn paint(outline: &Outline) -> (String, f64) {
    match &outline.fill {
        Some(Fill::Solid(c)) => (c.hex.clone(), c.alpha),
        Some(Fill::Gradient(g)) => g
            .stops
            .first()
            .map_or(("#000000".to_owned(), 1.0), |s| (s.colour.hex.clone(), s.colour.alpha)),
        _ => ("#000000".to_owned(), 1.0),
    }
}

/// One head's frame: its end, the head's length and half its width, the unit and the line's
/// width.
struct Head<'a> {
    at: &'a PathEnd,
    length: f64,
    half: f64,
    unit: f64,
    width: f64,
    colour: &'a str,
    alpha: f64,
}

impl Head<'_> {
    /// The point `along` the line out of the path and `across` it (to the left of out).
    fn point(&self, along: f64, across: f64) -> Vec<f64> {
        let PathEnd { x, y, dx, dy } = *self.at;
        let (nx, ny) = (-dy, dx);
        vec![x + dx * along + nx * across, y + dy * along + ny * across]
    }

    fn polygon(&self, corners: &[(f64, f64)]) -> Element {
        let mut commands: Vec<Command> = corners
            .iter()
            .enumerate()
            .map(|(i, (along, across))| Command::new(if i == 0 { 'M' } else { 'L' }, self.point(*along, *across)))
            .collect();
        commands.push(Command::new('Z', Vec::new()));
        self.filled(Element::empty(Tag::Path).with("d", Value::Path(commands)))
    }

    fn filled(&self, element: Element) -> Element {
        let element = element.word("fill", self.colour);
        if self.alpha < 1.0 {
            element.num("fill-opacity", self.alpha)
        } else {
            element
        }
    }

    /// The head and the setback, for a head of `kind`; `None` for a kind that draws none.
    fn draw(&self, kind: &str) -> Option<(Element, f64)> {
        let (length, half) = (self.length, self.half);
        Some(match kind {
            "triangle" => (
                self.polygon(&[(0.0, 0.0), (-length, half), (-length, -half)]),
                (length - self.unit / 2.0).max(0.0),
            ),
            "stealth" => (
                self.polygon(&[
                    (0.0, 0.0),
                    (-length, half),
                    (-length * STEALTH_NOTCH, 0.0),
                    (-length, -half),
                ]),
                length / 2.0,
            ),
            "diamond" => (
                self.polygon(&[(length / 2.0, 0.0), (0.0, half), (-length / 2.0, 0.0), (0.0, -half)]),
                0.0,
            ),
            "oval" => (self.oval(), 0.0),
            "arrow" => self.chevron(),
            _ => return None,
        })
    }

    fn oval(&self) -> Element {
        let PathEnd { x, y, dx, dy } = *self.at;
        let angle = dy.atan2(dx).to_degrees();
        self.filled(
            Element::empty(Tag::Ellipse)
                .num("cx", x)
                .num("cy", y)
                .num("rx", self.length / 2.0)
                .num("ry", self.half)
                .with(
                    "transform",
                    Value::Transform(vec![Op::RotateAbout(angle, x, y, Sep::Space)]),
                ),
        )
    }

    /// An open chevron stroked at the line's width, its outer tip on the point.
    fn chevron(&self) -> (Element, f64) {
        let (length, half, width) = (self.length, self.half, self.width);
        let angle = half.atan2(length);
        let spread = if angle == 0.0 { 1e-9 } else { angle };
        let inset = (width / 2.0) / spread.sin();
        let d = vec![
            Command::new('M', self.point(-inset - length, half)),
            Command::new('L', self.point(-inset, 0.0)),
            Command::new('L', self.point(-inset - length, -half)),
        ];
        let mut path = Element::empty(Tag::Path)
            .with("d", Value::Path(d))
            .word("fill", "none")
            .word("stroke", self.colour)
            .num("stroke-width", width)
            .word("stroke-linecap", "round")
            .word("stroke-linejoin", "miter");
        if self.alpha < 1.0 {
            path = path.num("stroke-opacity", self.alpha);
        }
        (path, inset + width / 2.0)
    }
}

/// Word's arrowheads for an open path whose ends are `ends` (its first point, then its last,
/// each with the unit direction out of the path), at `dpi`. A head not stated, or of a kind
/// that draws nothing, sets nothing back.
pub fn render_arrowheads(outline: Option<&Outline>, ends: &[PathEnd; 2], dpi: f64) -> Arrowheads {
    let Some(outline) = outline else {
        return Arrowheads::default();
    };
    if outline.head_end.is_none() && outline.tail_end.is_none() {
        return Arrowheads::default();
    }
    let (colour, alpha) = paint(outline);
    let width = emu_to_px_at(outline.width, dpi);
    let unit = width.max(MINIMUM_UNIT_PT * dpi / 72.0);
    let mut out = Arrowheads::default();
    let [first, last] = ends;
    for (index, (end, at)) in [(&outline.head_end, first), (&outline.tail_end, last)]
        .into_iter()
        .enumerate()
    {
        let drawn = end.as_ref().and_then(|end: &ArrowEnd| {
            let head = Head {
                at,
                length: size_factor(&end.length) * unit,
                half: size_factor(&end.width) * unit / 2.0,
                unit,
                width,
                colour: &colour,
                alpha,
            };
            head.draw(&end.kind)
        });
        let setback = drawn.map_or(0.0, |(element, setback)| {
            out.elements.push(element);
            setback
        });
        if index == 0 {
            out.start = setback;
        } else {
            out.end = setback;
        }
    }
    out
}
