//! Owns: the chart layout differential's Rust side -- a chart laid out by `ooxml-chart` with
//! the consumer ooxml-common's golden harness is (`tests/goldens.py`: colours through an
//! unthemed colour context under the application's rules, fills and outlines as stated, a
//! title's runs with their own size, weight and colour, `+` faces as Aptos), and its
//! elements and data written as JSON in the form `python/chart_diff.py` dumps the Python
//! scene in (every field of each dataclass, by its Python name).
//!
//! Must not: be used by any library; it is a test instrument.

use crate::dml::model::{class, num, onum, opt, ostr};
use ooxml_chart::layout::{
    accent_colours, chart_text_body, drawable_plots, lay_out, sources::default_font_size, ChartData, ChartInput,
    ChartStyle, Resolve,
};
use ooxml_chart::model::Chart;
use ooxml_chart::rules::{ChartRules, POWERPOINT, WORD};
use ooxml_colour::{default_colour_map, ColourContext, SourceColour};
use ooxml_drawingml::drawable::{
    BodyProperties, Colour, Element, Fill, Geometry, GradientFill, GradientStop, Outline, Paragraph,
    ParagraphProperties, RunProperties, TextBody, TextRun, Transform,
};
use ooxml_drawingml::resolve::colour_context;
use ooxml_drawingml::source::{self, GradientKind};
use ooxml_drawingml::text::{Spacing, TextBody as SourceBody};
use ooxml_numfmt::Locale;
use serde_json::{json, Value as J};

/// The golden harness' consumer.
struct Golden {
    context: ColourContext,
}

impl Golden {
    fn colour(&self, c: &SourceColour) -> Option<Colour> {
        self.context.resolve(c).ok().flatten().map(Colour::from)
    }
}

impl Resolve for Golden {
    fn fill(&self, fill: Option<&source::Fill>) -> Option<Fill> {
        match fill? {
            source::Fill::None => Some(Fill::None),
            source::Fill::Solid(c) => self.colour(c).map(Fill::Solid),
            source::Fill::Gradient(g) => {
                let stops: Vec<GradientStop> = g
                    .stops
                    .iter()
                    .filter_map(|s| {
                        Some(GradientStop {
                            position: s.position,
                            colour: self.colour(&s.colour)?,
                        })
                    })
                    .collect();
                if stops.is_empty() {
                    return None;
                }
                Some(Fill::Gradient(GradientFill {
                    stops,
                    angle: g.angle / 60000.0,
                    gradient_type: if g.kind == GradientKind::Radial {
                        "radial"
                    } else {
                        "linear"
                    }
                    .into(),
                    center_x: g.centre.map(|c| c.0),
                    center_y: g.centre.map(|c| c.1),
                    path: g.path.clone(),
                    focus: g.focus,
                    scaled: g.scaled,
                    rotate_with_shape: g.rotate_with_shape,
                }))
            }
            source::Fill::Pattern(p) => Some(Fill::Pattern {
                preset: p.preset.clone(),
                foreground: self.colour(&p.foreground)?,
                background: self.colour(&p.background)?,
            }),
            _ => None,
        }
    }

    fn outline(&self, outline: Option<&source::Outline>) -> Option<Outline> {
        let outline = outline?;
        let paint = outline.fill.as_ref().and_then(|f| self.fill(Some(f)));
        if matches!(paint, Some(Fill::None)) {
            return None;
        }
        let end = |e: &source::LineEnd| ooxml_drawingml::drawable::ArrowEnd {
            kind: e.kind.clone(),
            width: e.width.clone(),
            length: e.length.clone(),
        };
        Some(Outline {
            width: outline.width.unwrap_or(9525.0),
            fill: paint.filter(|p| matches!(p, Fill::Solid(_) | Fill::Gradient(_))),
            dash_style: outline
                .dash
                .clone()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "solid".into()),
            custom_dash: outline.custom_dash.clone().filter(|d| !d.is_empty()),
            line_cap: outline.cap.clone(),
            line_join: outline.join.clone(),
            head_end: outline.head.as_ref().map(end),
            tail_end: outline.tail.as_ref().map(end),
            compound: outline.compound.clone(),
        })
    }

    fn text(&self, rich: Option<&SourceBody>, text: &str, size: f64, align: &str) -> TextBody {
        let mut paragraphs: Vec<Paragraph> = Vec::new();
        for paragraph in rich.map(|r| r.paragraphs.as_slice()).unwrap_or(&[]) {
            let runs = paragraph
                .runs
                .iter()
                .map(|run| {
                    let p = run.properties.as_ref();
                    TextRun {
                        text: run.text.clone(),
                        properties: RunProperties {
                            font_size: Some(p.and_then(|p| p.size).filter(|s| *s != 0.0).unwrap_or(size)),
                            font_family: p.and_then(|p| p.latin.clone()),
                            bold: p.is_some_and(|p| p.bold == Some(true)),
                            italic: p.is_some_and(|p| p.italic == Some(true)),
                            colour: p.and_then(|p| p.colour.as_ref()).and_then(|c| self.colour(c)),
                            ..RunProperties::default()
                        },
                    }
                })
                .collect();
            paragraphs.push(Paragraph {
                runs,
                properties: ParagraphProperties::aligned(align),
                end_properties: None,
            });
        }
        if !paragraphs.iter().any(|p| !p.runs.is_empty()) {
            paragraphs = vec![Paragraph {
                runs: vec![TextRun {
                    text: text.to_owned(),
                    properties: RunProperties {
                        font_size: Some(size),
                        ..RunProperties::default()
                    },
                }],
                properties: ParagraphProperties::aligned(align),
                end_properties: None,
            }];
        }
        TextBody {
            paragraphs,
            properties: chart_text_body(),
        }
    }

    fn typeface(&self, face: &str) -> Option<String> {
        Some(if face.starts_with('+') {
            "Aptos".to_owned()
        } else {
            face.to_owned()
        })
    }
}

fn colour_json(c: &Colour) -> J {
    class("ResolvedColor", vec![("hex", json!(c.hex)), ("alpha", num(c.alpha))])
}

fn fill_json(f: &Fill) -> J {
    match f {
        Fill::Solid(c) => class("SolidFill", vec![("color", colour_json(c)), ("type", json!("solid"))]),
        Fill::None => class("NoFill", vec![("type", json!("none"))]),
        Fill::Pattern {
            preset,
            foreground,
            background,
        } => class(
            "PatternFill",
            vec![
                ("preset", json!(preset)),
                ("foreground_color", colour_json(foreground)),
                ("background_color", colour_json(background)),
                ("type", json!("pattern")),
            ],
        ),
        Fill::Gradient(g) => class(
            "GradientFill",
            vec![
                (
                    "stops",
                    J::Array(
                        g.stops
                            .iter()
                            .map(|s| {
                                class(
                                    "GradientStop",
                                    vec![("position", num(s.position)), ("color", colour_json(&s.colour))],
                                )
                            })
                            .collect(),
                    ),
                ),
                ("angle", num(g.angle)),
                ("gradient_type", json!(g.gradient_type)),
                ("center_x", onum(g.center_x)),
                ("center_y", onum(g.center_y)),
                ("type", json!("gradient")),
                ("path", ostr(g.path.as_ref())),
                (
                    "focus",
                    opt(g.focus.as_ref(), |f| J::Array(f.iter().map(|v| num(*v)).collect())),
                ),
                ("scaled", json!(g.scaled)),
                ("rotate_with_shape", json!(g.rotate_with_shape)),
            ],
        ),
        Fill::Image { .. } => json!({"@": "ImageFill"}),
    }
}

fn outline_json(o: &Outline) -> J {
    let end = |e: &ooxml_drawingml::drawable::ArrowEnd| {
        class(
            "ArrowEndpoint",
            vec![
                ("type", json!(e.kind)),
                ("width", json!(e.width)),
                ("length", json!(e.length)),
            ],
        )
    };
    class(
        "Outline",
        vec![
            ("width", num(o.width)),
            ("fill", opt(o.fill.as_ref(), fill_json)),
            ("dash_style", json!(o.dash_style)),
            (
                "custom_dash",
                opt(o.custom_dash.as_ref(), |d| {
                    J::Array(d.iter().map(|v| num(*v)).collect())
                }),
            ),
            ("line_cap", ostr(o.line_cap.as_ref())),
            ("line_join", ostr(o.line_join.as_ref())),
            ("head_end", opt(o.head_end.as_ref(), end)),
            ("tail_end", opt(o.tail_end.as_ref(), end)),
            ("compound", ostr(o.compound.as_ref())),
        ],
    )
}

fn transform_json(t: &Transform) -> J {
    class(
        "Transform",
        vec![
            ("offset_x", num(t.offset_x)),
            ("offset_y", num(t.offset_y)),
            ("extent_width", num(t.extent_width)),
            ("extent_height", num(t.extent_height)),
            ("rotation", num(t.rotation)),
            ("flip_h", json!(t.flip_h)),
            ("flip_v", json!(t.flip_v)),
        ],
    )
}

fn geometry_json(g: &Geometry) -> J {
    match g {
        Geometry::Preset { name, adjust } => {
            let pairs: Vec<J> = adjust.iter().map(|(k, v)| json!([k, num(*v)])).collect();
            class(
                "PresetGeometry",
                vec![
                    ("preset", json!(name)),
                    ("adjust_values", json!({"{}": pairs})),
                    ("type", json!("preset")),
                ],
            )
        }
        Geometry::Custom(paths) => class(
            "CustomGeometry",
            vec![
                (
                    "paths",
                    J::Array(
                        paths
                            .iter()
                            .map(|p| {
                                class(
                                    "CustomGeometryPath",
                                    vec![
                                        ("width", num(p.width)),
                                        ("height", num(p.height)),
                                        ("commands", json!(p.commands)),
                                    ],
                                )
                            })
                            .collect(),
                    ),
                ),
                ("type", json!("custom")),
            ],
        ),
    }
}

fn spacing_json(s: &Spacing) -> J {
    match s {
        Spacing::Points(v) => class("PointsSpacing", vec![("value", num(*v)), ("type", json!("pts"))]),
        Spacing::Percent(v) => class("PercentSpacing", vec![("value", num(*v)), ("type", json!("pct"))]),
    }
}

fn run_json(p: &RunProperties) -> J {
    class(
        "RunProperties",
        vec![
            ("font_size", onum(p.font_size)),
            ("font_family", ostr(p.font_family.as_ref())),
            ("font_family_ea", ostr(p.font_family_ea.as_ref())),
            ("font_family_cs", ostr(p.font_family_cs.as_ref())),
            ("bold", json!(p.bold)),
            ("italic", json!(p.italic)),
            ("underline", json!(p.underline)),
            ("underline_style", ostr(p.underline_style.as_ref())),
            ("strikethrough", json!(p.strikethrough)),
            ("color", opt(p.colour.as_ref(), colour_json)),
            ("baseline", num(p.baseline)),
            ("hyperlink", J::Null),
            ("outline", J::Null),
            ("highlight", opt(p.highlight.as_ref(), colour_json)),
        ],
    )
}

fn paragraph_json(p: &Paragraph) -> J {
    let pp = &p.properties;
    let properties = class(
        "ParagraphProperties",
        vec![
            ("alignment", ostr(pp.alignment.as_ref())),
            ("line_spacing", opt(pp.line_spacing.as_ref(), spacing_json)),
            ("space_before", spacing_json(&pp.space_before)),
            ("space_after", spacing_json(&pp.space_after)),
            ("level", json!(pp.level)),
            ("bullet", J::Null),
            ("bullet_font", ostr(pp.bullet_font.as_ref())),
            ("bullet_color", opt(pp.bullet_colour.as_ref(), colour_json)),
            ("bullet_size_pct", onum(pp.bullet_size_percent)),
            ("bullet_size_points", onum(pp.bullet_size_points)),
            ("margin_left", onum(pp.margin_left)),
            ("indent", onum(pp.indent)),
            ("tab_stops", json!([])),
        ],
    );
    let runs: Vec<J> = p
        .runs
        .iter()
        .map(|r| {
            class(
                "TextRun",
                vec![("text", json!(r.text)), ("properties", run_json(&r.properties))],
            )
        })
        .collect();
    class(
        "Paragraph",
        vec![
            ("runs", J::Array(runs)),
            ("properties", properties),
            ("end_para_run_properties", opt(p.end_properties.as_ref(), run_json)),
        ],
    )
}

fn body_properties_json(b: &BodyProperties) -> J {
    class(
        "BodyProperties",
        vec![
            ("anchor", json!(b.anchor)),
            ("margin_left", num(b.margin_left)),
            ("margin_right", num(b.margin_right)),
            ("margin_top", num(b.margin_top)),
            ("margin_bottom", num(b.margin_bottom)),
            ("wrap", json!(b.wrap)),
            ("auto_fit", json!(b.auto_fit)),
            ("font_scale", num(b.font_scale)),
            ("ln_spc_reduction", num(b.line_reduction)),
            ("num_col", json!(b.columns)),
            ("vert", json!(b.vert)),
            ("rotation", num(b.rotation)),
            ("default_tab_size", num(b.default_tab_size)),
        ],
    )
}

fn body_json(b: &TextBody) -> J {
    class(
        "TextBody",
        vec![
            (
                "paragraphs",
                J::Array(b.paragraphs.iter().map(paragraph_json).collect()),
            ),
            ("body_properties", body_properties_json(&b.properties)),
        ],
    )
}

/// One element as the Python dumps a scene element.
pub fn element(e: &Element) -> J {
    match e {
        Element::Shape(s) => class(
            "ShapeElement",
            vec![
                ("transform", transform_json(&s.transform)),
                ("geometry", geometry_json(&s.geometry)),
                ("fill", opt(s.fill.as_ref(), fill_json)),
                ("outline", opt(s.outline.as_ref(), outline_json)),
                ("text_body", opt(s.text.as_ref(), body_json)),
                ("text_transform", J::Null),
                ("text_rect", J::Null),
                ("effects", J::Null),
                ("placeholder_type", J::Null),
                ("placeholder_idx", J::Null),
                ("alt_text", J::Null),
                ("hyperlink", J::Null),
                ("element_id", J::Null),
                ("element_path", J::Null),
                ("type", json!("shape")),
            ],
        ),
        Element::Connector(c) => class(
            "ConnectorElement",
            vec![
                ("transform", transform_json(&c.transform)),
                ("geometry", geometry_json(&c.geometry)),
                ("outline", opt(c.outline.as_ref(), outline_json)),
                ("effects", J::Null),
                ("alt_text", J::Null),
                ("element_id", J::Null),
                ("element_path", J::Null),
                ("type", json!("connector")),
            ],
        ),
        Element::Group(g) => class(
            "GroupElement",
            vec![
                ("transform", transform_json(&g.transform)),
                ("child_transform", transform_json(&g.child_transform)),
                ("children", J::Array(g.children.iter().map(element).collect())),
            ],
        ),
    }
}

fn data_json(d: &ChartData) -> J {
    let series: Vec<J> = d
        .series
        .iter()
        .map(|s| {
            class(
                "ChartSeries",
                vec![
                    ("name", ostr(s.name.as_ref())),
                    ("values", J::Array(s.values.iter().map(|v| onum(*v)).collect())),
                    ("categories", json!(s.categories)),
                    ("color", opt(s.colour.as_ref(), colour_json)),
                    ("format_code", ostr(s.format_code.as_ref())),
                ],
            )
        })
        .collect();
    let axis = d.value_axis.map(|a| {
        class(
            "ChartAxisScale",
            vec![
                ("minimum", num(a.minimum)),
                ("maximum", num(a.maximum)),
                ("major_unit", num(a.major_unit)),
            ],
        )
    });
    let view = d.view_3d.as_ref().map(|v| {
        class(
            "Chart3DView",
            vec![
                ("rot_x", onum(v.rot_x)),
                ("rot_y", onum(v.rot_y)),
                ("depth_percent", onum(v.depth_percent)),
                ("h_percent", onum(v.h_percent)),
                ("right_angle_axes", json!(v.right_angle_axes)),
                ("perspective", onum(v.perspective)),
            ],
        )
    });
    class(
        "ChartData",
        vec![
            ("kind", json!(d.kind)),
            ("series", J::Array(series)),
            ("categories", json!(d.categories)),
            ("title", ostr(d.title.as_ref())),
            ("grouping", ostr(d.grouping.as_ref())),
            ("bar_direction", ostr(d.bar_direction.as_ref())),
            ("value_axis", axis.unwrap_or(J::Null)),
            ("legend_position", ostr(d.legend_position.as_ref())),
            ("three_d", json!(d.three_d)),
            ("view_3d", view.unwrap_or(J::Null)),
        ],
    )
}

/// One case: a chart laid out in a `width` x `height` pt frame under `rules`.
pub struct Case<'c> {
    /// The chart.
    pub chart: &'c Chart,
    /// The frame's width, points.
    pub width: f64,
    /// Its height.
    pub height: f64,
    /// `powerpoint` or `word`.
    pub rules: &'c str,
    /// The locale.
    pub locale: Locale,
}

/// A case laid out, as `chart_diff.py` dumps the Python's: the notes, elements and data,
/// or the notes alone where nothing is drawn.
pub fn case(case: &Case<'_>) -> J {
    let rules: &ChartRules = if case.rules == "word" { &WORD } else { &POWERPOINT };
    let context = colour_context(
        None,
        default_colour_map(),
        if case.rules == "word" {
            ooxml_colour::WORD
        } else {
            ooxml_colour::POWERPOINT
        },
    );
    let golden = Golden { context };
    let plots = drawable_plots(case.chart);
    let Some(&plot) = plots.first() else {
        let mut kinds: Vec<&str> = case.chart.plots.iter().map(|p| p.kind.as_str()).collect();
        kinds.sort_unstable();
        return json!({"notes": [format!("not drawn: {}", kinds.join(","))]});
    };
    let tx1 = golden
        .colour(&SourceColour::scheme("tx1"))
        .unwrap_or_else(|| Colour::opaque("#000000"));
    let style = ChartStyle {
        font_family: Some("Aptos".into()),
        font_size: default_font_size(case.chart),
        colour: tx1,
        accents: accent_colours(|key| golden.colour(&SourceColour::scheme(key))),
        font_family_ea: None,
        office_2007: false,
        line_colour: None,
    };
    let input = ChartInput {
        chart: case.chart,
        plot,
        plots: plots.clone(),
        width_pt: case.width,
        height_pt: case.height,
        style,
        resolve: &golden,
        rules,
        locale: &case.locale,
        default_title: None,
    };
    let laid = lay_out(&input);
    let mut notes: Vec<String> = Vec::new();
    if !laid.flattened_three_d.is_empty() {
        notes.push(format!("flattened: {}", laid.flattened_three_d.join(",")));
    }
    if laid.default_title_wanted {
        notes.push("default title wanted".into());
    }
    notes.extend(
        laid.axis_titles_not_drawn
            .iter()
            .map(|r| format!("axis title not drawn: {r}")),
    );
    json!({
        "notes": notes,
        "elements": J::Array(laid.elements.iter().map(element).collect()),
        "data": data_json(&laid.data),
    })
}
