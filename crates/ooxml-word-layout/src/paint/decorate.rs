//! Owns: the rectangles Word fills for a span's decorations, on whole device pixels
//! (docx2svg's `decoration_rects`, measured there): highlight and run shading across the
//! line's text rows; single, words, double, thick, dotted and dash underlines from the
//! face's `post` table; single and double strikethrough from its `OS/2` table -- and a note
//! separator's line (`_separator`); a line's decorations (`_decorate`), and its automatic
//! text drawn white on a dark cell.
//!
//! Must not: decide what a span is or where its line is.

use super::spans::PageOut;
use super::{PaintContext, Rule, Span};
use crate::px::Px;
use ooxml_base::Q;
use ooxml_fonts::Decorations;
use ooxml_text::FaceKey;
use ooxml_word_model::resolve::CharacterFormat;
use std::sync::Arc;

/// The pattern of a dotted and a dashed underline: px on, and the period.
const DASHES: [(&str, i64, i64); 2] = [("dotted", 6, 12), ("dash", 16, 24)];

/// The span's pen extent, trailing spaces at its line's end left out (`_extent`).
fn extent(span: &Span, line_end: bool) -> Option<(Px, Px)> {
    let mut last = span.chars.len().checked_sub(1)?;
    if line_end {
        while span.chars.get(last).is_some_and(|c| c.is_whitespace()) {
            last = last.checked_sub(1)?;
        }
    }
    let end = if last + 1 == span.chars.len() {
        span.end
    } else {
        span.x(last + 1)?
    };
    Some((span.x(0)?, end))
}

/// Every word's pen extent (spaces left out).
fn words(span: &Span) -> Vec<(Px, Px)> {
    let mut out = Vec::new();
    let mut start = None;
    for (k, ch) in span.chars.iter().enumerate() {
        let x = span.x(k).unwrap_or(span.end);
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                out.push((s, x));
            }
        } else if start.is_none() {
            start = Some(x);
        }
    }
    if let Some(s) = start {
        out.push((s, span.end));
    }
    out
}

/// The size decorations scale with: the drawn size in whole device px.
fn face_px(half_points: i64) -> Q {
    Q::from_integer(i128::from(
        Px::points(Q::new(i128::from(half_points), 2)).round_half_up(),
    ))
}

fn whole(v: i64) -> Px {
    Px::int(v)
}

fn rule(kind: &'static str, span: &Span, at: (i64, i64), size: (i64, i64)) -> Rule {
    Rule {
        kind,
        path: span.path.clone(),
        x: whole(at.0),
        y: whole(at.1),
        width: whole(size.0),
        height: whole(size.1),
        color: span.color.clone(),
        style: None,
    }
}

/// A rule of `style` (an underline's or a strike's).
fn styled(rule: Rule, style: &str) -> Rule {
    Rule {
        style: Some(style.to_owned()),
        ..rule
    }
}

fn round(x: Q) -> i64 {
    Px::from(x).round_half_up()
}

/// The rectangles of a span's decorations; `band` the line's text rows.
pub(super) fn decoration_rects(
    span: &Span,
    decorations: Option<&Decorations>,
    line_end: bool,
    band: Option<(i64, i64)>,
) -> Vec<Rule> {
    let mut out = Vec::new();
    let Some((a, b)) = extent(span, line_end) else {
        return out;
    };
    let (x0, x1) = (a.round_half_up(), b.round_half_up());
    if let Some(colour) = span.highlight.map(str::to_owned).or_else(|| span.shading.clone()) {
        let band = band.unwrap_or((span.line_baseline, span.line_baseline));
        let fx0 = span.x(0).map_or(x0, Px::round_half_up);
        let fx1 = span.end.round_half_up();
        let kind = if span.highlight.is_some() {
            "highlight"
        } else {
            "shading"
        };
        out.push(Rule {
            color: Some(colour),
            ..rule(kind, span, (fx0, band.0), (fx1 - fx0, band.1 - band.0))
        });
    }
    let Some(d) = decorations else { return out };
    let scale = face_px(span.half_points) / Q::from_integer(i128::from(d.units_per_em.max(1)));
    if let (Some(style), Some(position), Some(thickness)) =
        (span.underline.as_deref(), d.underline_position, d.underline_thickness)
    {
        let position = -Q::from_integer(i128::from(position)) * scale;
        let exact_thick = Q::from_integer(i128::from(thickness)) * scale;
        let thick = round(exact_thick).max(1);
        let top = span.y + round(position);
        underline(&mut out, span, style, (x0, x1), (position, exact_thick, thick, top));
    }
    if let (true, Some(position), Some(size)) = (
        span.strike || span.double_strike,
        d.strikeout_position,
        d.strikeout_size,
    ) {
        let thick = round(Q::from_integer(i128::from(size)) * scale).max(1);
        let top = span.y - round(Q::from_integer(i128::from(position)) * scale);
        if span.double_strike {
            out.push(styled(
                rule("strike", span, (x0, top - thick), (x1 - x0, thick)),
                "double",
            ));
            out.push(styled(
                rule("strike", span, (x0, top + thick), (x1 - x0, thick)),
                "double",
            ));
        } else {
            out.push(styled(rule("strike", span, (x0, top), (x1 - x0, thick)), "single"));
        }
    }
    out
}

/// An underline of `style`: `(position, exact thickness, thickness, top)` of the single one.
fn underline(out: &mut Vec<Rule>, span: &Span, style: &str, (x0, x1): (i64, i64), metrics: (Q, Q, i64, i64)) {
    let (position, exact_thick, thick, top) = metrics;
    if style == "words" {
        for (a, b) in words(span) {
            let (a, b) = (a.round_half_up(), b.round_half_up());
            out.push(styled(rule("underline", span, (a, top), (b - a, thick)), style));
        }
    } else if matches!(style, "double" | "thick" | "wavyDouble") || style.ends_with("Heavy") {
        let first_offset = i64::try_from((position / Q::from_integer(2)).ceil().to_integer()).unwrap_or(0);
        let first = span.y + first_offset;
        let height = (round(position + exact_thick / Q::from_integer(2)) - first_offset).max(5);
        if matches!(style, "double" | "wavyDouble") {
            let half = (thick + 1) / 2;
            out.push(styled(rule("underline", span, (x0, first), (x1 - x0, half)), style));
            out.push(styled(
                rule("underline", span, (x0, first + height), (x1 - x0, half)),
                style,
            ));
        } else {
            out.push(styled(rule("underline", span, (x0, first), (x1 - x0, height)), style));
        }
    } else if let Some((_, on, period)) = DASHES.iter().find(|(name, ..)| *name == style) {
        let mut start = x0;
        while start < x1 {
            let end = (start + on).min(x1);
            out.push(styled(
                rule("underline", span, (start, top), (end - start, thick)),
                style,
            ));
            start += period;
        }
    } else {
        out.push(styled(rule("underline", span, (x0, top), (x1 - x0, thick)), style));
    }
}

/// A note separator's line: where its run's face would strike through its text, as thick,
/// from `x` 2,880 twips long, or to `end` (the continuation separator's); `(x, y, width,
/// height)`, `None` where the face has no strikeout metrics.
pub(super) fn separator_rule(
    decorations: Option<&Decorations>,
    format: &CharacterFormat,
    (x, end, clip): (Px, Option<Px>, Option<Px>),
    baseline: i64,
) -> Option<(Px, Px, Px, Px)> {
    let d = decorations?;
    let (position, size) = (d.strikeout_position?, d.strikeout_size?);
    let scale = face_px(format.half_points) / Q::from_integer(i128::from(d.units_per_em.max(1)));
    let thick = round(Q::from_integer(i128::from(size)) * scale).max(1);
    let top = baseline - round(Q::from_integer(i128::from(position)) * scale);
    let x0 = x.round_half_up();
    let x1 = match (end, clip) {
        (Some(end), _) => end.round_half_up(),
        // Below mode 15, in a column narrower than the line, cut at its right edge.
        (None, Some(clip)) => (x + Px::points(Q::from_integer(144)))
            .round_half_up()
            .min(clip.round_half_up()),
        (None, None) => (x + Px::points(Q::from_integer(144))).round_half_up(),
    };
    Some((whole(x0), whole(top), whole(x1 - x0), whole(thick)))
}

/// Automatic text on a dark cell fill drawn white, where the paragraph has no background:
/// the spans with no colour, highlight or shading of their own.
pub(super) fn white_text(spans: &mut [Span]) {
    for span in spans
        .iter_mut()
        .filter(|s| s.color.is_none() && s.highlight.is_none() && s.shading.is_none())
    {
        span.color = Some("FFFFFF".to_owned());
    }
}

/// The decorations of a line's spans (`_decorate`).
pub(super) fn decorate(cx: &PaintContext<'_>, spans: &[Span], band: (i64, i64), out: &mut PageOut) {
    let drawn: Vec<&Span> = spans.iter().filter(|s| !s.chars.is_empty()).collect();
    for (k, span) in drawn.iter().enumerate() {
        let key = FaceKey {
            family: Arc::clone(&span.face),
            bold: span.bold,
            italic: span.italic,
        };
        let decorations = cx.faces.decorations(&key);
        out.rules.extend(decoration_rects(
            span,
            decorations.as_ref(),
            k + 1 == drawn.len(),
            Some(band),
        ));
    }
}
