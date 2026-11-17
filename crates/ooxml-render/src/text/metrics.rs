//! Owns: a text body's vertical measures (ooxml-common's `textbody` metrics): sizes by run
//! and paragraph, a line's natural height and descent, line and paragraph spacing, the first
//! baseline, a body's height wrapped to a width, and what a stored or fitted autofit does
//! to its sizes.
//!
//! Must not: draw, or break lines (it asks the wrap how many there are).

use super::wrap::{faces, wrap_paragraph, Segment};
use crate::context::Painter;
use crate::measure::{TextMeasurer, PX_PER_PT};
use crate::rules::{Autofit, FirstBaseline};
use crate::units::emu_to_px;
use ooxml_drawingml::drawable::{Paragraph, RunProperties, TextBody};
use ooxml_drawingml::text::Spacing;

/// A body's size where no run states one, points.
pub const DEFAULT_FONT_SIZE_PT: f64 = 18.0;

/// `properties.font_size or default`.
pub fn size_or(p: &RunProperties, default: f64) -> f64 {
    match p.font_size {
        Some(s) if s != 0.0 => s,
        _ => default,
    }
}

/// The first size any run states.
pub fn default_font_size(paragraphs: &[Paragraph]) -> f64 {
    paragraphs
        .iter()
        .flat_map(|p| &p.runs)
        .find_map(|r| r.properties.font_size.filter(|s| *s != 0.0))
        .unwrap_or(DEFAULT_FONT_SIZE_PT)
}

/// The size of a paragraph's first run with text, else its end-of-paragraph size.
pub fn paragraph_font_size(paragraph: &Paragraph, default: f64) -> f64 {
    if let Some(size) = paragraph.runs.iter().find_map(|r| {
        (!r.text.is_empty())
            .then_some(r.properties.font_size)
            .flatten()
            .filter(|s| *s != 0.0)
    }) {
        return size;
    }
    paragraph
        .end_properties
        .as_ref()
        .and_then(|e| e.font_size)
        .filter(|s| *s != 0.0)
        .unwrap_or(default)
}

/// The first size a line's segments state.
pub fn line_font_size(segments: &[Segment<'_>], default: f64) -> f64 {
    segments
        .iter()
        .find_map(|s| s.properties.font_size.filter(|v| *v != 0.0))
        .unwrap_or(default)
}

/// A paragraph's runs as segments.
pub fn run_segments(paragraph: &Paragraph) -> Vec<Segment<'_>> {
    paragraph
        .runs
        .iter()
        .enumerate()
        .map(|(run, r)| Segment {
            text: r.text.clone(),
            run,
            properties: &r.properties,
        })
        .collect()
}

/// A line is as tall as its tallest run, points.
pub fn line_natural_height(segments: &[Segment<'_>], default: f64, scale: f64, m: &dyn TextMeasurer) -> f64 {
    let mut tallest = 0.0_f64;
    for s in segments {
        let size = size_or(s.properties, default) * scale;
        tallest = tallest.max(size * m.line_height_ratio(faces(s.properties)));
    }
    if tallest > 0.0 {
        tallest
    } else {
        default * scale * 1.2
    }
}

/// The descent of a line's tallest run, px.
pub fn line_descent(segments: &[Segment<'_>], default: f64, scale: f64, m: &dyn TextMeasurer) -> f64 {
    let (mut tallest, mut descent) = (0.0, 0.0);
    for s in segments {
        let size = size_or(s.properties, default) * scale;
        let f = faces(s.properties);
        let height = size * m.line_height_ratio(f);
        if height > tallest {
            tallest = height;
            descent = height - size * m.ascender_ratio(f);
        }
    }
    descent * PX_PER_PT
}

/// The descent of a paragraph's runs, or of its end-of-paragraph properties, px.
pub fn paragraph_descent(paragraph: &Paragraph, default: f64, scale: f64, m: &dyn TextMeasurer) -> f64 {
    let segments = run_segments(paragraph);
    if !segments.is_empty() {
        return line_descent(&segments, default, scale, m);
    }
    let empty = RunProperties::default();
    let end = paragraph.end_properties.as_ref().unwrap_or(&empty);
    line_descent(
        &[Segment {
            text: String::new(),
            run: 0,
            properties: end,
        }],
        default,
        scale,
        m,
    )
}

fn first_faces(paragraphs: &[Paragraph]) -> Option<&RunProperties> {
    paragraphs
        .iter()
        .flat_map(|p| &p.runs)
        .map(|r| &r.properties)
        .find(|p| p.font_family.is_some() || p.font_family_ea.is_some())
}

/// The line ratio of the first run naming a face.
pub fn default_line_ratio(paragraphs: &[Paragraph], m: &dyn TextMeasurer) -> f64 {
    first_faces(paragraphs).map_or(1.2, |p| m.line_height_ratio(faces(p)))
}

/// The ascender ratio of the first run naming a face.
pub fn default_ascender_ratio(paragraphs: &[Paragraph], m: &dyn TextMeasurer) -> f64 {
    first_faces(paragraphs).map_or(1.0, |p| m.ascender_ratio(faces(p)))
}

/// A paragraph's single-line height before its line spacing, points.
pub fn paragraph_natural_height(
    paragraph: &Paragraph,
    default: f64,
    scale: f64,
    ratio: f64,
    m: &dyn TextMeasurer,
) -> f64 {
    let has_text = paragraph.runs.iter().any(|r| !r.text.is_empty());
    if let Some(size) = paragraph
        .end_properties
        .as_ref()
        .and_then(|e| e.font_size)
        .filter(|s| *s != 0.0)
    {
        if !has_text {
            return size * scale * ratio;
        }
    }
    let height = line_natural_height(&run_segments(paragraph), default, scale, m);
    if height > 0.0 {
        height
    } else {
        default * scale * ratio
    }
}

/// `a:lnSpc` applied to a natural height, px.
pub fn line_height_px(paragraph: &Paragraph, natural_pt: f64, reduction: f64) -> f64 {
    match paragraph.properties.line_spacing {
        Some(Spacing::Points(v)) => (v / 100.0) * PX_PER_PT * (1.0 - reduction),
        Some(Spacing::Percent(v)) => natural_pt * PX_PER_PT * (v / 100_000.0).max(0.5) * (1.0 - reduction),
        None => natural_pt * PX_PER_PT * 1.0 * (1.0 - reduction),
    }
}

/// `a:spcBef` or `a:spcAft`, px.
pub fn spacing_px(spacing: Spacing, natural_pt: f64) -> f64 {
    match spacing {
        Spacing::Points(v) => (v / 100.0) * PX_PER_PT,
        Spacing::Percent(v) => natural_pt * (v / 100_000.0) * PX_PER_PT,
    }
}

/// How far below the top of the text the first baseline sits, px.
pub fn first_baseline_px(
    paragraph: &Paragraph,
    size_pt: f64,
    ascender: f64,
    reduction: f64,
    painter: &Painter<'_>,
) -> f64 {
    let fonts = paragraph
        .runs
        .iter()
        .map(|r| &r.properties)
        .find(|p| p.font_family.is_some() || p.font_family_ea.is_some())
        .map(faces)
        .unwrap_or_default();
    let natural_pt = size_pt * painter.measurer.line_height_ratio(fonts);
    let line_px = line_height_px(paragraph, natural_pt, reduction);
    let natural_px = natural_pt * PX_PER_PT;
    if natural_px <= 0.0 {
        return size_pt * ascender * PX_PER_PT;
    }
    let factor = line_px / natural_px;
    if factor > 1.0 {
        return 0.75 * line_px;
    }
    if painter.rules.first_baseline == FirstBaseline::Descent {
        return line_px - (natural_px - size_pt * ascender * PX_PER_PT);
    }
    size_pt * ascender * factor * PX_PER_PT
}

/// The height `paragraphs` take wrapped to `width`, px.
pub fn estimate_height(
    paragraphs: &[Paragraph],
    default: f64,
    wrap: bool,
    width: f64,
    spacing: (f64, f64),
    m: &dyn TextMeasurer,
) -> f64 {
    let (reduction, scale) = spacing;
    let mut total = 0.0;
    let ratio = default_line_ratio(paragraphs, m);
    let mut previous_after = 0.0;
    let scaled_default = default * scale;
    for (index, paragraph) in paragraphs.iter().enumerate() {
        let has_text = paragraph.runs.iter().any(|r| !r.text.is_empty());
        let natural = paragraph_natural_height(paragraph, default, scale, ratio, m);
        let line = line_height_px(paragraph, natural, reduction);
        let count = if wrap && has_text {
            let w = width - emu_to_px(paragraph.properties.margin_left.unwrap_or(0.0));
            wrap_paragraph(paragraph, w, scaled_default, scale, m).len()
        } else {
            1
        };
        total += count as f64 * line;
        if index > 0 {
            total += previous_after + spacing_px(paragraph.properties.space_before, natural);
        }
        previous_after = spacing_px(paragraph.properties.space_after, natural);
    }
    total
}

/// A run's size under a stored `fontScale`: rounded to a whole point, half up.
fn scaled_size(size: f64, scale: f64) -> f64 {
    (size * scale + 0.5 + 1e-9).trunc().max(1.0)
}

/// `normAutofit` as PowerPoint draws a file it has not re-fitted: what it stores, applied.
pub fn stored_autofit(body: &TextBody, autofit: Autofit) -> Option<TextBody> {
    let props = &body.properties;
    if autofit != Autofit::Stored || props.auto_fit != "normAutofit" {
        return None;
    }
    let (scale, reduction) = (props.font_scale, props.line_reduction);
    if scale == 1.0 && reduction == 0.0 {
        return None;
    }
    let stated = body
        .paragraphs
        .iter()
        .flat_map(|p| &p.runs)
        .any(|r| r.properties.font_size.is_some_and(|s| s != 0.0));
    let sized = |p: &RunProperties| -> RunProperties {
        let mut out = p.clone();
        if scale == 1.0 {
            return out;
        }
        match p.font_size.filter(|s| *s != 0.0) {
            Some(s) => out.font_size = Some(scaled_size(s, scale)),
            None if !stated => out.font_size = Some(scaled_size(DEFAULT_FONT_SIZE_PT, scale)),
            None => {}
        }
        out
    };
    let mut out = body.clone();
    for paragraph in &mut out.paragraphs {
        for run in &mut paragraph.runs {
            run.properties = sized(&run.properties);
        }
        paragraph.end_properties = paragraph.end_properties.as_ref().map(&sized);
        if reduction != 0.0 && !matches!(paragraph.properties.line_spacing, Some(Spacing::Points(_))) {
            let percent = match paragraph.properties.line_spacing {
                Some(Spacing::Percent(v)) => v,
                _ => 100_000.0,
            };
            paragraph.properties.line_spacing = Some(Spacing::Percent(percent - reduction * 100_000.0));
        }
    }
    out.properties.font_scale = 1.0;
    out.properties.line_reduction = 0.0;
    Some(out)
}

/// `normAutofit` fitted: shrink until the text fits, in a few passes.
pub fn shrink_to_fit(
    paragraphs: &[Paragraph],
    default: f64,
    scale: f64,
    reduction: f64,
    room: (f64, f64),
    m: &dyn TextMeasurer,
) -> f64 {
    let (width, height) = room;
    if height <= 0.0 {
        return scale;
    }
    let min_scale = scale * 0.1;
    let mut current = scale;
    for _ in 0..5 {
        let needed = estimate_height(paragraphs, default, true, width, (reduction, current), m);
        if needed <= height {
            break;
        }
        current = (current * (height / needed)).max(min_scale);
        if current <= min_scale {
            break;
        }
    }
    current
}
