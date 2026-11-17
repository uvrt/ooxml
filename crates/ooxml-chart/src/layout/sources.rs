//! Owns: what the layout reads off the chart's stated text and axes -- a text body's first
//! default run and its size, faces and colour, a run's properties merged over a default,
//! the title's own font rules (`title_states_no_font`, `title_states_defaults`), PowerPoint's
//! automatic title, whether an axis shows its labels, and which tick marks it draws.
//!
//! Must not: resolve a colour or a face (the consumer does), or place anything.

use super::consts::{DEFAULT_CHART_FONT_PT, DEFAULT_TICK_MARK, OFFICE_2007_TICK_MARKS};
use crate::model::{Axis, Chart, ChartText};
use ooxml_drawingml::text::{Paragraph, ParagraphProperties, RunProperties, TextBody, TextRun};

/// The first paragraph default run a body states (`_default_run`).
pub fn default_run(body: Option<&TextBody>) -> Option<&RunProperties> {
    body?
        .paragraphs
        .iter()
        .find_map(|p| p.properties.as_ref()?.default_run.as_ref())
}

/// The body's default size, where stated and not zero.
pub fn text_size(body: Option<&TextBody>) -> Option<f64> {
    default_run(body).and_then(|r| r.size).filter(|s| *s != 0.0)
}

/// The body's default Latin face.
pub fn text_typeface(body: Option<&TextBody>) -> Option<&str> {
    default_run(body).and_then(|r| r.latin.as_deref())
}

/// The body's default East Asian face.
pub fn text_typeface_ea(body: Option<&TextBody>) -> Option<&str> {
    default_run(body).and_then(|r| r.east_asian.as_deref())
}

/// The layers a body's list style states.
fn list_layers(body: Option<&TextBody>) -> Vec<Option<&RunProperties>> {
    let Some(style) = body.and_then(|b| b.list_style.as_ref()) else {
        return Vec::new();
    };
    std::iter::once(style.default.as_ref())
        .chain(style.levels.iter().map(Option::as_ref))
        .flatten()
        .map(|layer| layer.default_run.as_ref())
        .collect()
}

/// Each paragraph's default run (`None` where it states none).
fn paragraph_defaults(body: Option<&TextBody>) -> Vec<Option<&RunProperties>> {
    body.map(|b| {
        b.paragraphs
            .iter()
            .map(|p| p.properties.as_ref().and_then(|pp| pp.default_run.as_ref()))
            .collect()
    })
    .unwrap_or_default()
}

/// Whether a body states a colour anywhere (a default run of any layer, or a run).
pub fn states_colour(body: Option<&TextBody>) -> bool {
    let Some(body) = body else {
        return false;
    };
    let mut layers: Vec<Option<&ParagraphProperties>> = Vec::new();
    if let Some(style) = &body.list_style {
        layers.push(style.default.as_ref());
        layers.extend(style.levels.iter().map(Option::as_ref));
    }
    layers.extend(body.paragraphs.iter().map(|p| p.properties.as_ref()));
    if layers
        .iter()
        .flatten()
        .any(|layer| layer.default_run.as_ref().is_some_and(|r| r.colour.is_some()))
    {
        return true;
    }
    body.paragraphs
        .iter()
        .flat_map(|p| p.runs.iter())
        .any(|run| run.properties.as_ref().is_some_and(|r| r.colour.is_some()))
}

/// `run` over `default`, each property `run` states replacing the default's.
pub fn merged_run(run: Option<&RunProperties>, default: Option<&RunProperties>) -> Option<RunProperties> {
    let (run, default) = match (run, default) {
        (Some(run), Some(default)) => (run, default),
        (run, default) => return run.or(default).cloned(),
    };
    let mut merged = default.clone();
    macro_rules! over {
        ($($field:ident),*) => {$(if run.$field.is_some() { merged.$field = run.$field.clone(); })*};
    }
    over!(
        bold,
        italic,
        underline,
        underline_style,
        strike,
        baseline,
        size,
        latin,
        east_asian,
        complex_script,
        colour,
        highlight,
        outline_width,
        outline_colour,
        hyperlink,
        tooltip,
        lang,
        alt_lang
    );
    Some(merged)
}

/// Whether an axis shows its labels.
pub fn labels_shown(axis: Option<&Axis>) -> bool {
    match axis {
        None => true,
        Some(axis) if axis.delete => false,
        Some(axis) => axis.tick_label_position.as_deref().unwrap_or("nextTo") != "none",
    }
}

/// The major and minor tick marks an axis draws.
pub fn tick_marks(axis: Option<&Axis>, office_2007: bool) -> (String, String) {
    let defaults = if office_2007 {
        OFFICE_2007_TICK_MARKS
    } else {
        (DEFAULT_TICK_MARK, DEFAULT_TICK_MARK)
    };
    let Some(axis) = axis else {
        return (defaults.0.into(), defaults.1.into());
    };
    let kind = |value: Option<&String>, default: &str| match value.map(String::as_str) {
        None => default.to_owned(),
        Some(v @ ("out" | "in" | "cross")) => v.to_owned(),
        Some(_) => "none".to_owned(),
    };
    (
        kind(axis.major_tick_mark.as_ref(), defaults.0),
        kind(axis.minor_tick_mark.as_ref(), defaults.1),
    )
}

/// A tick's extent from its axis line: outward, inward, across, or none.
pub fn tick_span(kind: &str, length: f64, outward: f64) -> Option<(f64, f64)> {
    match kind {
        "out" => Some((0.0, outward * length)),
        "in" => Some((0.0, -outward * length)),
        "cross" => Some((-length, length)),
        _ => None,
    }
}

/// Whether the chart title's rich text states no face anywhere (PowerPoint's Arial rule).
pub fn title_states_no_font(chart: &Chart) -> bool {
    let Some(rich) = chart.title.as_ref().and_then(|t| t.rich.as_ref()) else {
        return false;
    };
    if title_states_defaults(chart) {
        return false;
    }
    let runs = rich
        .paragraphs
        .iter()
        .flat_map(|p| p.runs.iter())
        .map(|r| r.properties.as_ref());
    if list_layers(Some(rich))
        .into_iter()
        .chain(runs)
        .flatten()
        .any(|l| l.latin.as_deref().is_some_and(|f| !f.is_empty()))
    {
        return false;
    }
    let own = chart.title_text_properties.as_ref();
    !list_layers(own)
        .into_iter()
        .chain(paragraph_defaults(own))
        .flatten()
        .any(|l| l.latin.as_deref().is_some_and(|f| !f.is_empty()) || l.size.is_some_and(|s| s != 0.0))
}

/// Whether the chart title's paragraphs state a default run.
pub fn title_states_defaults(chart: &Chart) -> bool {
    chart
        .title
        .as_ref()
        .and_then(|t| t.rich.as_ref())
        .is_some_and(|rich| paragraph_defaults(Some(rich)).iter().any(Option::is_some))
}

/// Which title property is asked about.
#[derive(Debug, Clone, Copy)]
pub enum TitleProperty {
    /// `typeface`.
    Typeface,
    /// `bold`.
    Bold,
}

/// Whether any layer of the title states `property` (and not as an empty string).
pub fn title_states(chart: &Chart, property: TitleProperty) -> bool {
    let rich = chart.title.as_ref().and_then(|t| t.rich.as_ref());
    let own = chart.title_text_properties.as_ref();
    let runs: Vec<Option<&RunProperties>> = rich
        .map(|r| {
            r.paragraphs
                .iter()
                .flat_map(|p| p.runs.iter())
                .map(|r| r.properties.as_ref())
                .collect()
        })
        .unwrap_or_default();
    list_layers(rich)
        .into_iter()
        .chain(paragraph_defaults(rich))
        .chain(runs)
        .chain(list_layers(own))
        .chain(paragraph_defaults(own))
        .flatten()
        .any(|layer| match property {
            TitleProperty::Typeface => layer.latin.as_deref().is_some_and(|f| !f.is_empty()),
            TitleProperty::Bold => layer.bold.is_some(),
        })
}

/// A title's size: its first run's, else its first paragraph default's, else 18 pt.
pub fn title_size(title: Option<&ChartText>) -> f64 {
    if let Some(rich) = title.and_then(|t| t.rich.as_ref()) {
        for paragraph in &rich.paragraphs {
            for run in &paragraph.runs {
                if let Some(size) = run.properties.as_ref().and_then(|p| p.size).filter(|s| *s != 0.0) {
                    return size;
                }
            }
            let stated = paragraph
                .properties
                .as_ref()
                .and_then(|p| p.default_run.as_ref())
                .and_then(|r| r.size);
            if let Some(size) = stated.filter(|s| *s != 0.0) {
                return size;
            }
        }
    }
    18.0
}

/// PowerPoint's automatic title size: the title's own, else 1.2 times the chart's text
/// rounded to the point, else 18 pt.
pub fn powerpoint_auto_title_size(chart: &Chart) -> f64 {
    if let Some(stated) = text_size(chart.title_text_properties.as_ref()) {
        return stated;
    }
    let Some(wide) = text_size(chart.text_properties.as_ref()) else {
        return 18.0;
    };
    let hundredths = crate::pyfmt::round_half_even(wide * 100.0);
    (hundredths * 12.0 / 1000.0 + 0.5).floor()
}

/// PowerPoint's automatic title: `text` bold in the minor face, over the title's own
/// properties, centred.
pub fn powerpoint_auto_title(chart: &Chart, text: &str) -> TextBody {
    let mut properties = RunProperties {
        bold: Some(true),
        latin: Some("+mn-lt".into()),
        size: Some(powerpoint_auto_title_size(chart)),
        ..RunProperties::default()
    };
    if let Some(stated) = default_run(chart.title_text_properties.as_ref()) {
        let size = properties.size;
        properties = merged_run(Some(stated), Some(&properties)).unwrap_or(properties);
        if stated.size == Some(0.0) {
            properties.size = size;
        }
    }
    TextBody {
        paragraphs: vec![Paragraph {
            runs: vec![TextRun {
                text: text.to_owned(),
                properties: Some(properties),
                field_type: None,
            }],
            properties: Some(ParagraphProperties {
                align: Some("ctr".into()),
                ..ParagraphProperties::default()
            }),
            end: None,
        }],
        properties: None,
        list_style: None,
    }
}

/// The chart's default text size.
pub fn default_font_size(chart: &Chart) -> f64 {
    text_size(chart.text_properties.as_ref()).unwrap_or(DEFAULT_CHART_FONT_PT)
}
