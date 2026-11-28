//! Owns: a chart in a Word drawing (`c:chart`), as docx2svg's `chart.py` draws one: its
//! part related from the part the drawing sits in, laid out by `ooxml-chart` under Word's
//! measured rules -- the document theme's minor face and accents, `tx1` text, a chart
//! style's grey lines, the sizes and faces Word gives titles -- with colours through the
//! document's theme and colour map (a chart's `c:clrMapOvr` over it), painted by
//! `ooxml-render` at 96 px an inch with Word's line box ([`WordMeasurer`]) and scaled onto
//! the page's device grid.
//!
//! Must not: read the document's own XML, or write SVG beyond what the renderer returns.

use super::colours::{Said, WordColours, DEFAULT_LINE_EMU};
use super::write::{Primitive, PxBox};
use ooxml_chart::layout::{
    accent_colours, chart_text_body, drawable_plots, lay_out, sources::default_font_size, ChartInput, ChartStyle,
    Resolve,
};
use ooxml_chart::model::Chart;
use ooxml_colour::SourceColour;
use ooxml_drawingml::drawable::{
    self, ArrowEnd, Colour, Element as Drawable, Fill, Group, Outline, Paragraph, ParagraphProperties, TextBody,
    TextRun, Transform,
};
use ooxml_drawingml::source;
use ooxml_drawingml::text::{RunProperties as SourceRun, TextBody as SourceBody};
use ooxml_fonts::{office_metrics_for, FontMapping};
use ooxml_render::measure::{Faces, TableMeasurer, TextMeasurer};
use ooxml_render::{render_element, Painter};
use ooxml_word_layout::paint::svg::number;
use ooxml_word_model::theme::FontScheme;
use ooxml_xml::view::{child, children, elements, local};
use ooxml_xml::Element;
use std::cell::RefCell;

/// EMU a point.
const EMU_PER_POINT: f64 = 12_700.0;
/// The face of a title whose paragraph states no `a:defRPr` (`TITLE_FACE`).
const TITLE_FACE: &str = "Arial";
/// The axis, tick and gridline colour under Word 365's chart style.
const CHART_STYLE_LINE: &str = "#898989";
/// `c14:style`'s namespace.
const C14: &str = "http://schemas.microsoft.com/office/drawing/2007/8/2/chart";

/// The renderers' measure with Word's line box: a face's own ascent and descent, where
/// PowerPoint's is 1.2 em (`WordTextMeasurer`).
pub struct WordMeasurer(pub TableMeasurer);

fn face_metrics(faces: Faces<'_>) -> Option<&'static ooxml_fonts::OfficeFace> {
    faces
        .latin
        .and_then(office_metrics_for)
        .or_else(|| faces.east_asian.and_then(office_metrics_for))
}

impl TextMeasurer for WordMeasurer {
    fn width(&self, text: &str, size_pt: f64, bold: bool, faces: Faces<'_>) -> f64 {
        self.0.width(text, size_pt, bold, faces)
    }

    fn kern_between(&self, left: &str, right: &str, size_pt: f64, bold: bool, faces: Faces<'_>) -> f64 {
        self.0.kern_between(left, right, size_pt, bold, faces)
    }

    fn line_height_ratio(&self, faces: Faces<'_>) -> f64 {
        face_metrics(faces).map_or_else(
            || self.0.line_height_ratio(faces),
            |m| (f64::from(m.ascender) + f64::from(m.descender.unsigned_abs())) / f64::from(m.units_per_em),
        )
    }

    fn ascender_ratio(&self, faces: Faces<'_>) -> f64 {
        face_metrics(faces).map_or_else(
            || self.0.ascender_ratio(faces),
            |m| f64::from(m.ascender) / f64::from(m.units_per_em),
        )
    }
}

/// A theme typeface (`+mn-lt`) as the document theme's face (`theme_typeface`).
pub fn theme_typeface(fonts: Option<&FontScheme>, typeface: Option<&str>) -> Option<String> {
    let typeface = typeface.filter(|t| !t.is_empty())?;
    if !typeface.starts_with('+') {
        return Some(typeface.to_owned());
    }
    let fonts = fonts?;
    let set = if typeface.starts_with("+mj") {
        &fonts.major
    } else {
        &fonts.minor
    };
    let found = match typeface.get(typeface.len().saturating_sub(2)..) {
        Some("lt") => &set.latin,
        Some("ea") => &set.east_asian,
        Some("cs") => &set.complex_script,
        _ => return None,
    };
    (!found.is_empty()).then(|| found.to_string())
}

/// Per paragraph of the `c:title` under `owner`, whether it states an `a:defRPr` at all.
fn title_defaults(owner: Option<&Element>) -> Vec<bool> {
    let title = owner.and_then(|o| {
        if local(o) == "title" {
            Some(o)
        } else {
            child(o, "title")
        }
    });
    let rich = title.and_then(|t| child(t, "tx")).and_then(|t| child(t, "rich"));
    rich.map(|r| {
        children(r, "p")
            .map(|p| child(p, "pPr").and_then(|p| child(p, "defRPr")).is_some())
            .collect()
    })
    .unwrap_or_default()
}

/// Whether the chart states Word 365's chart style (`c14:style`).
fn has_chart_style(root: &Element) -> bool {
    fn any(e: &Element) -> bool {
        (local(e) == "style" && e.name().namespace() == Some(C14)) || elements(e).any(any)
    }
    any(root)
}

/// The resolution of one chart's colours, titles and faces (Word's).
struct WordChart<'a> {
    colours: WordColours<'a>,
    fonts: Option<&'a FontScheme>,
    style: ChartStyle,
    /// Each axis title's text body (by address) with its paragraphs' stated defaults.
    axis_titles: Vec<(*const SourceBody, Vec<bool>)>,
    chart_title: Vec<bool>,
    base: f64,
    said: RefCell<Said>,
}

impl WordChart<'_> {
    fn colour(&self, c: &SourceColour) -> Option<Colour> {
        self.colours.resolve(c, None, &mut self.said.borrow_mut())
    }

    fn run(&self, props: &SourceRun) -> drawable::RunProperties {
        let colour = props.colour.as_ref().and_then(|c| self.colour(c));
        drawable::RunProperties {
            font_size: props.size,
            font_family: theme_typeface(self.fonts, props.latin.as_deref()),
            font_family_ea: match props.east_asian.as_deref() {
                Some(ea) => theme_typeface(self.fonts, Some(ea)),
                None => self.style.font_family_ea.clone(),
            },
            bold: props.bold == Some(true),
            italic: props.italic == Some(true),
            colour: colour.or_else(|| Some(self.style.colour.clone())),
            ..drawable::RunProperties::default()
        }
    }
}

fn merge(layers: &[Option<&SourceRun>]) -> SourceRun {
    let mut out = SourceRun::default();
    for layer in layers.iter().flatten() {
        out.bold = layer.bold.or(out.bold);
        out.italic = layer.italic.or(out.italic);
        out.size = layer.size.or(out.size);
        out.latin = layer.latin.clone().or(out.latin);
        out.east_asian = layer.east_asian.clone().or(out.east_asian);
        out.colour = layer.colour.clone().or(out.colour);
    }
    out
}

impl Resolve for WordChart<'_> {
    fn fill(&self, fill: Option<&source::Fill>) -> Option<Fill> {
        self.colours.fill(fill?, None, None, &mut self.said.borrow_mut())
    }

    fn outline(&self, outline: Option<&source::Outline>) -> Option<Outline> {
        let outline = outline?;
        let paint = outline.fill.as_ref().and_then(|f| self.fill(Some(f)));
        if paint == Some(Fill::None) {
            return None;
        }
        let end = |e: &source::LineEnd| ArrowEnd {
            kind: e.kind.clone(),
            width: e.width.clone(),
            length: e.length.clone(),
        };
        Some(Outline {
            width: outline.width.unwrap_or(DEFAULT_LINE_EMU),
            fill: paint.filter(|p| matches!(p, Fill::Solid(_) | Fill::Gradient(_))),
            dash_style: outline.dash.clone().unwrap_or_else(|| "solid".into()),
            custom_dash: outline.custom_dash.clone().filter(|d| !d.is_empty()),
            line_cap: outline.cap.clone(),
            line_join: outline.join.clone(),
            head_end: outline.head.as_ref().map(end),
            tail_end: outline.tail.as_ref().map(end),
            compound: outline.compound.clone(),
        })
    }

    fn text(&self, rich: Option<&SourceBody>, text: &str, size: f64, align: &str) -> TextBody {
        let (axis, stated) = match rich {
            None => (false, Vec::new()),
            Some(r) => self
                .axis_titles
                .iter()
                .find(|(p, _)| std::ptr::eq(*p, r))
                .map_or((false, self.chart_title.clone()), |(_, s)| (true, s.clone())),
        };
        let base_size = if axis { self.style.font_size } else { self.base };
        let sources: Vec<Option<&ooxml_drawingml::text::Paragraph>> = match rich {
            Some(r) if !r.paragraphs.is_empty() => r.paragraphs.iter().map(Some).collect(),
            _ => vec![None],
        };
        let mut paragraphs = Vec::new();
        for (index, paragraph) in sources.into_iter().enumerate() {
            let defaults = paragraph
                .and_then(|p| p.properties.as_ref())
                .and_then(|p| p.default_run.as_ref());
            let base = if rich.is_none() || defaults.is_some() || stated.get(index).copied().unwrap_or(false) {
                SourceRun {
                    latin: self.style.font_family.clone(),
                    size: Some(base_size),
                    bold: Some(true),
                    ..SourceRun::default()
                }
            } else {
                SourceRun {
                    latin: Some(TITLE_FACE.into()),
                    size: Some(size),
                    bold: Some(false),
                    ..SourceRun::default()
                }
            };
            let runs: Vec<Option<&ooxml_drawingml::text::TextRun>> = match paragraph {
                Some(p) if !p.runs.is_empty() => p.runs.iter().map(Some).collect(),
                _ => vec![None],
            };
            let runs = runs
                .into_iter()
                .map(|r| {
                    let props = merge(&[Some(&base), defaults, r.and_then(|r| r.properties.as_ref())]);
                    TextRun {
                        text: r.map_or_else(|| text.to_owned(), |r| r.text.clone()),
                        properties: self.run(&props),
                    }
                })
                .collect();
            paragraphs.push(Paragraph {
                runs,
                properties: ParagraphProperties::aligned(align),
                end_properties: None,
            });
        }
        TextBody {
            paragraphs,
            properties: chart_text_body(),
        }
    }

    fn typeface(&self, face: &str) -> Option<String> {
        theme_typeface(self.fonts, Some(face))
    }
}

/// `c:txPr`'s first `a:defRPr`'s size, where a paragraph states one.
fn stated_text_size(chart: &Chart) -> Option<f64> {
    let body = chart.text_properties.as_ref()?;
    body.paragraphs
        .iter()
        .find_map(|p| p.properties.as_ref().and_then(|p| p.default_run.as_ref()))
        .and_then(|d| d.size.filter(|s| *s != 0.0))
}

/// What drawing a chart reads: the colours, the theme's faces.
pub struct ChartJob<'a> {
    /// The document's colours.
    pub colours: &'a WordColours<'a>,
    /// The theme's faces.
    pub fonts: Option<&'a FontScheme>,
    /// The locale numbers are written in.
    pub locale: &'a ooxml_numfmt::Locale,
}

/// The chart's colours (its `c:clrMapOvr` over the document's map) and its text and
/// series defaults in Word (`chart_style`): the theme's minor face, the size the chart
/// states or 10 pt, `tx1`, the accents, and a chart style's grey lines.
fn chart_style<'a>(
    job: &ChartJob<'a>,
    chart: &Chart,
    root: &Element,
    said: &mut Said,
) -> (WordColours<'a>, ChartStyle) {
    let mut colours = job.colours.clone();
    if let Some(over) = &chart.color_map_override {
        for (k, v) in over {
            colours.slots.insert(k.clone(), v.clone());
        }
    }
    let minor = job.fonts.map(|f| &f.minor);
    let text_colour = colours
        .resolve(
            &SourceColour::Scheme {
                slot: "tx1".into(),
                transforms: Vec::new(),
            },
            None,
            said,
        )
        .unwrap_or_else(|| Colour::opaque("#000000"));
    let accent_said = RefCell::new(Vec::new());
    let accents = accent_colours(|key| {
        let slot = SourceColour::Scheme {
            slot: key.into(),
            transforms: Vec::new(),
        };
        colours.resolve(&slot, None, &mut accent_said.borrow_mut())
    });
    said.extend(accent_said.take());
    let styled = has_chart_style(root);
    let style = ChartStyle {
        font_family: minor.map(|m| m.latin.to_string()).filter(|s| !s.is_empty()),
        font_size: default_font_size(chart),
        colour: text_colour,
        accents,
        font_family_ea: minor.map(|m| m.east_asian.to_string()).filter(|s| !s.is_empty()),
        office_2007: false,
        line_colour: styled.then(|| CHART_STYLE_LINE.to_owned()),
    };
    (colours, style)
}

/// The chart part `root` laid out in `at` and painted (`chart_element` and `render`); the
/// resources added to `painter`; `None` where it cannot be drawn (warned).
pub fn draw(
    job: &ChartJob<'_>,
    root: &Element,
    (at, path): (&PxBox, &str),
    painter: &mut Painter<'_>,
    said: &mut Said,
) -> Option<Vec<Primitive>> {
    let Some(chart) = ooxml_chart::chart_space(root) else {
        said.push("chart-unreadable:no c:chart".into());
        return None;
    };
    let plots = drawable_plots(&chart);
    if plots.is_empty() {
        let mut kinds: Vec<&str> = chart.plots.iter().map(|p| p.kind.as_str()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        let kinds = if kinds.is_empty() {
            "nothing".to_owned()
        } else {
            kinds.join(",")
        };
        said.push(format!("chart-unsupported-type:{kinds}"));
        return None;
    }
    let emu = |v: ooxml_word_layout::px::Px| super::shapes::to_f64(v.value()) * 914_400.0 / 300.0;
    let (width_emu, height_emu) = (emu(at.width), emu(at.height));
    if width_emu <= 0.0 || height_emu <= 0.0 {
        said.push("chart-unreadable:zero-sized".into());
        return None;
    }
    let (colours, style) = chart_style(job, &chart, root, said);
    let styled = style.line_colour.is_some();
    let plot_area = child(root, "chart").and_then(|c| child(c, "plotArea"));
    let axis_nodes: Vec<&Element> = plot_area
        .map(|p| {
            elements(p)
                .filter(|e| ooxml_chart::model::AXIS_ELEMENTS.contains(&local(e)))
                .collect()
        })
        .unwrap_or_default();
    let axis_titles = chart
        .axes
        .iter()
        .zip(axis_nodes)
        .filter_map(|(axis, node)| {
            let rich = axis.title.as_ref()?.rich.as_ref()?;
            Some((std::ptr::from_ref(rich), title_defaults(Some(node))))
        })
        .collect();
    let base = stated_text_size(&chart).map_or(if styled { 18.0 } else { 10.0 }, |s| 1.2 * s);
    let resolver = WordChart {
        colours,
        fonts: job.fonts,
        style: style.clone(),
        axis_titles,
        chart_title: title_defaults(child(root, "chart").and_then(|c| child(c, "title"))),
        base,
        said: RefCell::new(Vec::new()),
    };
    let input = ChartInput {
        chart: &chart,
        plot: plots.first().copied()?,
        plots: plots.clone(),
        width_pt: width_emu / EMU_PER_POINT,
        height_pt: height_emu / EMU_PER_POINT,
        style,
        resolve: &resolver,
        rules: &ooxml_chart::rules::WORD,
        locale: job.locale,
        default_title: None,
    };
    let laid = lay_out(&input);
    said.extend(resolver.said.take());
    layout_said(&laid, said);
    let frame = Transform::of(0.0, 0.0, width_emu, height_emu);
    let group = Drawable::Group(Box::new(Group {
        transform: frame,
        child_transform: frame,
        children: laid.elements,
        ..Group::default()
    }));
    Some(vec![render(&group, at, path, painter)])
}

/// What the layout says it did not draw as Word does.
fn layout_said(laid: &ooxml_chart::layout::Laid, said: &mut Said) {
    if !laid.flattened_three_d.is_empty() {
        said.push(format!("chart-3d-flattened:{}", laid.flattened_three_d.join(",")));
    }
    if laid.default_title_wanted {
        said.push("chart-title-not-drawn:title in Word's own words".into());
    }
    let mut reasons: Vec<&str> = Vec::new();
    for r in &laid.axis_titles_not_drawn {
        if !reasons.contains(&r.as_str()) {
            reasons.push(r);
        }
    }
    for r in reasons {
        let why = match r {
            "default" => "axis title in Word's own words",
            "chart" => "axis title in a chart of this kind",
            "placement" => "axis title turned or placed otherwise",
            other => other,
        };
        said.push(format!("chart-axis-title-not-drawn:{why}"));
    }
}

/// `element` (its box at the origin) painted at `at`, its resources on `painter`'s page
/// under ids that page has not used (`chart.render`).
pub fn render(element: &Drawable, at: &PxBox, path: &str, painter: &mut Painter<'_>) -> Primitive {
    let rules = ooxml_render::WORD;
    let measurer = WordMeasurer(TableMeasurer { kerning: rules.kerning });
    let mut chart = Painter::new(rules, &measurer);
    chart.font_mapping = FontMapping::new(Vec::new());
    chart.scope_ids(painter.id_scope());
    chart.resume_ids(painter.ids());
    let markup = render_element(element, &mut chart).to_svg();
    painter.resume_ids(chart.ids());
    for def in chart.defs {
        painter.add_def(def);
    }
    Primitive::Markup {
        path: path.to_owned(),
        markup,
        transform: Some(format!(
            "translate({} {}) scale(3.125)",
            number(at.x.value()),
            number(at.y.value())
        )),
    }
}
