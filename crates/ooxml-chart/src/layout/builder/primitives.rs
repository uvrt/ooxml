//! Owns: the primitives every part of the layout draws with -- a value's position on an
//! axis, an axis line's outline, and the drawable elements themselves: rectangles and preset
//! shapes, polylines and polygons as custom paths, slices, connectors, and one-line text
//! boxes placed by their baselines.
//!
//! Must not: decide where anything goes.

use super::Builder;
use crate::layout::consts::{DEFAULT_AXIS_COLOR, DEFAULT_AXIS_LINE_EMU, EMU_PER_POINT, OFFICE_2007_AXIS_COLOR};
use crate::layout::paths::{path_commands, ring_path, translate_path, Point};
use crate::layout::scale::{py_max, py_min, Scale};
use crate::layout::text::{ChartFont, FontBox};
use crate::layout::three_d::Rect;
use crate::pyfmt::fixed;
use crate::rules::FirstBaseline;
use ooxml_drawingml::drawable::{
    BodyProperties, Colour, Connector, CustomPath, Element, Fill, Geometry, Outline, Paragraph, ParagraphProperties,
    RunProperties, Shape, TextBody, TextRun, Transform,
};
use ooxml_drawingml::source;

/// Chart text's body: no insets, no wrapping (`CHART_TEXT_BODY`).
pub fn chart_text_body() -> BodyProperties {
    BodyProperties {
        anchor: "t".into(),
        margin_left: 0.0,
        margin_right: 0.0,
        margin_top: 0.0,
        margin_bottom: 0.0,
        wrap: "none".into(),
        ..BodyProperties::default()
    }
}

/// The same, wrapping (`CHART_WRAPPED_TEXT_BODY`).
pub fn chart_wrapped_text_body() -> BodyProperties {
    BodyProperties {
        wrap: "square".into(),
        ..chart_text_body()
    }
}

/// The smallest and largest coordinates of `points` (Python's `min` and `max`).
fn extent(points: &[Point]) -> (f64, f64, f64, f64) {
    let first = points.first().copied().unwrap_or_default();
    points
        .iter()
        .skip(1)
        .fold((first.0, first.1, first.0, first.1), |(l, t, r, b), p| {
            (py_min(l, p.0), py_min(t, p.1), py_max(r, p.0), py_max(b, p.1))
        })
}

impl<'a> Builder<'a> {
    /// A value's y on a vertical axis.
    pub fn value_to_y(&self, rect: Rect, value: f64, scale: Scale) -> f64 {
        let span = scale.maximum - scale.minimum;
        if scale.reversed {
            if span <= 0.0 {
                return rect.top;
            }
            return rect.top + (value - scale.minimum) / span * rect.height();
        }
        if span <= 0.0 {
            return rect.bottom;
        }
        rect.bottom - (value - scale.minimum) / span * rect.height()
    }

    /// A value's x on a horizontal axis.
    pub fn value_to_x(&self, rect: Rect, value: f64, scale: Scale) -> f64 {
        let span = scale.maximum - scale.minimum;
        if scale.reversed {
            if span <= 0.0 {
                return rect.right;
            }
            return rect.right - (value - scale.minimum) / span * rect.width();
        }
        if span <= 0.0 {
            return rect.left;
        }
        rect.left + (value - scale.minimum) / span * rect.width()
    }

    /// An axis line's outline: the stated one where it has a fill, none for `a:noFill`,
    /// else the default 0.5 pt line.
    pub fn axis_outline(&self, outline: Option<&source::Outline>) -> Option<Outline> {
        if super::series::no_fill_outline(outline) {
            return None;
        }
        let resolved = outline.and_then(|o| self.resolve.outline(Some(o)));
        if let Some(found) = resolved.filter(|o| o.fill.is_some()) {
            return Some(found);
        }
        let colour = self
            .style
            .line_colour
            .clone()
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| {
                (if self.style.office_2007 {
                    OFFICE_2007_AXIS_COLOR
                } else {
                    DEFAULT_AXIS_COLOR
                })
                .to_owned()
            });
        Some(Outline::solid(DEFAULT_AXIS_LINE_EMU, Colour::opaque(&colour)))
    }

    /// A shape of preset geometry over `rect`.
    pub fn rect(&mut self, rect: Rect, fill: Option<Fill>, outline: Option<Outline>, preset: &str) {
        self.elements.push(Element::Shape(Box::new(Shape {
            transform: Transform::of(
                rect.left * EMU_PER_POINT,
                rect.top * EMU_PER_POINT,
                rect.width() * EMU_PER_POINT,
                rect.height() * EMU_PER_POINT,
            ),
            geometry: Geometry::preset(preset),
            fill,
            outline,
            text: None,
        })));
    }

    /// A custom path in its own box.
    fn path(
        &mut self,
        points: &[Point],
        commands: impl FnOnce(f64, f64) -> String,
        fill: Option<Fill>,
        outline: Option<Outline>,
    ) {
        let (left, top, right, bottom) = extent(points);
        let width = py_max(right - left, 1e-6);
        let height = py_max(bottom - top, 1e-6);
        let commands = commands(left, top);
        self.elements.push(Element::Shape(Box::new(Shape {
            transform: Transform::of(
                left * EMU_PER_POINT,
                top * EMU_PER_POINT,
                width * EMU_PER_POINT,
                height * EMU_PER_POINT,
            ),
            geometry: Geometry::Custom(vec![CustomPath {
                width,
                height,
                commands,
            }]),
            fill,
            outline,
            text: None,
        })));
    }

    /// A line through `points` (smoothed or not), as one path.
    pub fn polyline(&mut self, points: &[Point], outline: &Outline, smooth: bool) {
        let local = |left: f64, top: f64| {
            let shifted: Vec<Point> = points.iter().map(|&(x, y)| (x - left, y - top)).collect();
            path_commands(&shifted, smooth)
        };
        self.path(points, local, Some(Fill::None), Some(outline.clone()));
    }

    /// A closed polygon.
    pub fn polygon(&mut self, points: &[Point], fill: Option<Fill>, outline: Option<Outline>) {
        let local = |left: f64, top: f64| {
            let parts: Vec<String> = points
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| {
                    format!(
                        "{} {} {}",
                        if i == 0 { "M" } else { "L" },
                        fixed(x - left, 4),
                        fixed(y - top, 4)
                    )
                })
                .collect();
            format!("{} Z", parts.join(" "))
        };
        self.path(points, local, fill, outline);
    }

    /// A straight segment, its box flipped where it rises to the right; nothing without an
    /// outline.
    pub fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, outline: Option<&Outline>) {
        let Some(outline) = outline else { return };
        self.elements.push(Element::Connector(Box::new(Connector {
            transform: Transform {
                flip_v: (x1 - x0) * (y1 - y0) < 0.0,
                ..Transform::of(
                    py_min(x0, x1) * EMU_PER_POINT,
                    py_min(y0, y1) * EMU_PER_POINT,
                    (x1 - x0).abs() * EMU_PER_POINT,
                    (y1 - y0).abs() * EMU_PER_POINT,
                )
            },
            geometry: Geometry::preset("line"),
            outline: Some(outline.clone()),
        })));
    }

    /// A pie or doughnut slice.
    pub fn slice(&mut self, slice: &Slice, fill: Option<Fill>, outline: Option<Outline>) {
        let commands = ring_path(slice.cx, slice.cy, slice.inner, slice.outer, slice.start, slice.sweep);
        let left = slice.cx - slice.outer;
        let top = slice.cy - slice.outer;
        let size = slice.outer * 2.0;
        let local = translate_path(&commands, -left, -top);
        self.elements.push(Element::Shape(Box::new(Shape {
            transform: Transform::of(
                left * EMU_PER_POINT,
                top * EMU_PER_POINT,
                size * EMU_PER_POINT,
                size * EMU_PER_POINT,
            ),
            geometry: Geometry::Custom(vec![CustomPath {
                width: size,
                height: size,
                commands: local,
            }]),
            fill,
            outline,
            text: None,
        })));
    }

    /// One run of chart text in `font`.
    pub fn label_body(&self, text: &str, font: &ChartFont, align: &str, colour: Option<Colour>) -> TextBody {
        let colour = colour
            .or_else(|| font.colour.clone())
            .unwrap_or_else(|| self.style.colour.clone());
        TextBody {
            paragraphs: vec![Paragraph {
                runs: vec![TextRun {
                    text: text.to_owned(),
                    properties: RunProperties {
                        font_size: Some(font.size()),
                        font_family: font.family.clone(),
                        font_family_ea: font.family_ea.clone(),
                        colour: Some(colour),
                        ..RunProperties::default()
                    },
                }],
                properties: ParagraphProperties::aligned(align),
                end_properties: None,
            }],
            properties: chart_text_body(),
        }
    }

    /// Where the renderer puts a one-line label's baseline below its box's top.
    pub fn first_baseline(&self, box_: &FontBox) -> f64 {
        match self.rules.first_baseline {
            FirstBaseline::Descent => box_.ascent,
            FirstBaseline::Box => box_.first_baseline(),
        }
    }

    /// A one-line text box placed so its baseline lands at `baseline`.
    pub fn text(&mut self, body: TextBody, left: f64, width: f64, baseline: f64, box_: &FontBox) {
        let top = baseline - self.first_baseline(box_);
        self.elements.push(Element::Shape(Box::new(Shape {
            transform: Transform::of(
                left * EMU_PER_POINT,
                top * EMU_PER_POINT,
                py_max(width, 1.0) * EMU_PER_POINT,
                (box_.line_height() * 1.5) * EMU_PER_POINT,
            ),
            geometry: Geometry::preset("rect"),
            fill: None,
            outline: None,
            text: Some(body),
        })));
    }
}

/// A slice of a ring: its centre, radii, start angle and sweep (degrees).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slice {
    pub cx: f64,
    pub cy: f64,
    pub inner: f64,
    pub outer: f64,
    pub start: f64,
    pub sweep: f64,
}
