//! Owns: one paragraph measured for the page stack ([`MeasuredParagraph`]): the text
//! engine's pieces and lines at its column's width, each line's height, and what follows
//! from the lines alone -- the lines a page or column break ends, the lines that draw
//! nothing, the lines ending in an automatic hyphen (docx2svg's `lines.broken`,
//! `shares_of`, `objects_of` and `paginate.flow`'s per-paragraph part).
//!
//! Must not: decide the paragraph's spacing, borders or keeps (`crate::flow`), or place it.

use crate::faces::Faces;
use crate::heights::{shares, HeightRules, LineHeight};
use crate::input::{label_props, list_level, text_paragraph, RunFormatter};
use crate::px::Px;
use crate::vertical::{HeightItem, LineRule};
use ooxml_base::{Locale, Q};
use ooxml_text::{Geometry, Measured, Measurer, PieceKind};
use ooxml_word_model::model::Paragraph;
use ooxml_word_model::resolve::CharacterFormat;
use ooxml_word_model::value::Value;
use ooxml_word_model::{ResolvedParagraph, WordDocument};
use std::sync::Arc;

/// Why a paragraph has no measure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unmeasured {
    /// The text engine refused it (a face or advance unknown, a drawing not laid out).
    Text(String),
    /// A line has no face with metrics.
    NoFaceMetrics,
}

/// A paragraph measured: its pieces and lines, each line's height and what follows from
/// the lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredParagraph {
    /// The text engine's pieces and lines.
    pub measured: Arc<Measured>,
    /// Each line's height.
    pub heights: Vec<LineHeight>,
    /// Lines a page break ends (a column break too, in a one-column section).
    pub page_breaks: Vec<usize>,
    /// Lines a column break ends, in a section of several columns.
    pub column_breaks: Vec<usize>,
    /// Lines that draw nothing: no character but spaces, and not the last.
    pub undrawn: Vec<usize>,
    /// Lines ending in an automatic hyphen.
    pub auto_hyphen_ends: Vec<usize>,
}

/// What measuring a paragraph needs besides the paragraph.
pub struct MeasureContext<'a> {
    /// The document.
    pub document: &'a WordDocument,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale measures are keyed under.
    pub locale: &'a Locale,
}

/// A character's format as an item of a line's height.
pub fn height_item(format: &CharacterFormat) -> HeightItem {
    HeightItem {
        face: Some(ooxml_text::FaceKey {
            family: Arc::from(&*format.face),
            bold: format.bold,
            italic: format.italic,
        }),
        half_points: format.half_points,
        position: format.position,
        border: format.border,
        picture: None,
    }
}

/// The label's items of the first line's height: the level's text, its numbers as `1`,
/// in the label's format, or its picture bullet (`lines.label_format`).
fn label_items(document: &WordDocument, resolved: &ResolvedParagraph) -> Vec<HeightItem> {
    let Some(level) = list_level(document, &resolved.paragraph) else {
        return Vec::new();
    };
    let Some(text) = level.text.as_deref().filter(|t| !t.is_empty()) else {
        return Vec::new();
    };
    let props = label_props(&resolved.mark, level);
    let mut formatter = RunFormatter::new(document, &props);
    if let Some(id) = level.picture_bullet {
        let Some((_, height)) = document.picture_bullets.get(&id).and_then(|b| b.pixels) else {
            return Vec::new();
        };
        let format = formatter.format(' ').clone();
        let units = u32::try_from(format.half_points)
            .ok()
            .and_then(|hp| ooxml_text::pieces::picture_bullet_units(hp, height).ok());
        let Some(units) = units else { return Vec::new() };
        let (n, d) = units.parts();
        let mut item = height_item(&format);
        item.face = None;
        item.border = None;
        item.picture = Some(Px::from(Q::new(n, d) * Px::layout_unit()));
        return vec![item];
    }
    let numbered = numbers_as_one(text);
    numbered.chars().map(|c| height_item(formatter.format(c))).collect()
}

/// `re.sub(r"%\d", "1", text)`.
fn numbers_as_one(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' && chars.peek().is_some_and(char::is_ascii_digit) {
            chars.next();
            out.push('1');
        } else {
            out.push(c);
        }
    }
    out
}

/// One paragraph to measure: the paragraph, its resolved formats, the geometry its lines
/// break at, whether its section has several columns and its list label's text.
pub struct ParagraphJob<'a> {
    /// The paragraph.
    pub paragraph: &'a Paragraph,
    /// Its resolved formats.
    pub resolved: &'a ResolvedParagraph,
    /// Where its lines start and end.
    pub geometry: &'a Geometry,
    /// Its section has several columns.
    pub columns: bool,
    /// Its list label's text.
    pub label: Option<&'a str>,
}

/// Measure a paragraph: its pieces and lines from the text engine (memoised by content in
/// `measurer`), each line's height, and what follows from its lines.
pub fn measure_paragraph(
    cx: &MeasureContext<'_>,
    measurer: &mut Measurer,
    job: &ParagraphJob<'_>,
) -> Result<MeasuredParagraph, Unmeasured> {
    let input = text_paragraph(cx.document, job.paragraph, job.resolved, job.label);
    let measured = measurer
        .measure(cx.faces, &input, job.geometry, job.columns, cx.locale)
        .map_err(|e| Unmeasured::Text(e.to_string()))?;
    let (heights, undrawn) = line_heights(cx, job, &measured)?;
    let (pieces, lines) = (&measured.pieces, &measured.lines);
    let last_char = |l: &ooxml_text::Line| l.end.checked_sub(1).and_then(|k| pieces.get(k)).map_or('\0', |p| p.ch);
    let forced_by = |c: char| -> Vec<usize> {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.forced && last_char(l) == c)
            .map(|(k, _)| k)
            .collect()
    };
    let mut page_breaks = forced_by('\u{c}');
    let column_breaks = if job.columns { forced_by('\u{b}') } else { Vec::new() };
    if !job.columns {
        page_breaks.extend(forced_by('\u{b}'));
        page_breaks.sort_unstable();
    }
    let auto_hyphen_ends = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.hyphenated && l.end > l.start && pieces.get(l.end - 1).is_some_and(|p| p.auto))
        .map(|(k, _)| k)
        .collect();
    Ok(MeasuredParagraph {
        heights,
        page_breaks,
        column_breaks,
        undrawn,
        auto_hyphen_ends,
        measured,
    })
}

/// Each line's height, and the lines that draw nothing.
fn line_heights(
    cx: &MeasureContext<'_>,
    job: &ParagraphJob<'_>,
    measured: &Measured,
) -> Result<(Vec<LineHeight>, Vec<usize>), Unmeasured> {
    let (document, resolved) = (cx.document, job.resolved);
    let pp = &resolved.paragraph;
    let rule = LineRule::parse(pp.get("spacing.lineRule").and_then(Value::str));
    let line = pp.get("spacing.line").and_then(Value::int).unwrap_or(240);
    let mut rules = HeightRules::new(cx.faces, rule, line, crate::input::rules(document).mode15());
    let mark = height_item(RunFormatter::new(document, &resolved.mark).format(' '));
    let label = label_items(document, resolved);
    let runs: Vec<(usize, bool, &str)> = job
        .paragraph
        .runs
        .iter()
        .zip(resolved.runs.iter())
        .enumerate()
        .map(|(k, (run, props))| (k, props.get("vanish").is_some_and(Value::truthy), &*run.text))
        .collect();
    let mut formatters: Vec<RunFormatter<'_>> = resolved
        .runs
        .iter()
        .map(|props| RunFormatter::new(document, props))
        .collect();
    let (pieces, lines) = (&measured.pieces, &measured.lines);
    let shares = shares(pieces, lines, &runs);
    let mut heights = Vec::with_capacity(lines.len());
    for (number, (line, share)) in lines.iter().zip(&shares).enumerate() {
        let chars: Vec<(HeightItem, char)> = share
            .iter()
            .filter_map(|&(run, ch)| Some((height_item(formatters.get_mut(run)?.format(ch)), ch)))
            .collect();
        let label = if number == 0 { label.as_slice() } else { &[] };
        let on_line = pieces.get(line.start..line.end).unwrap_or_default();
        let objects: Vec<i64> = on_line
            .iter()
            .filter(|p| p.kind == PieceKind::Object)
            .map(|p| p.height)
            .collect();
        let height = if objects.is_empty() {
            rules.line_height(&chars, &mark, label)
        } else {
            let mut object_runs: Vec<usize> = on_line
                .iter()
                .filter(|p| p.kind == PieceKind::Object)
                .filter_map(|p| usize::try_from(p.run).ok())
                .collect();
            object_runs.sort_unstable();
            object_runs.dedup();
            let run_items: Vec<HeightItem> = object_runs
                .iter()
                .filter_map(|r| Some(height_item(formatters.get_mut(*r)?.format(' '))))
                .collect();
            rules.object_line_height(&chars, &mark, label, &objects, &run_items)
        };
        heights.push(height.ok_or(Unmeasured::NoFaceMetrics)?);
    }
    let undrawn = shares
        .iter()
        .enumerate()
        .filter(|(k, share)| k + 1 < shares.len() && share.iter().all(|(_, c)| c.is_whitespace()))
        .map(|(k, _)| k)
        .collect();
    Ok((heights, undrawn))
}

#[cfg(test)]
mod tests {
    use super::numbers_as_one;

    #[test]
    fn label_numbers() {
        assert_eq!(numbers_as_one("%1.%2)"), "1.1)");
        assert_eq!(numbers_as_one("100%"), "100%");
        assert_eq!(numbers_as_one("%a"), "%a");
    }
}
