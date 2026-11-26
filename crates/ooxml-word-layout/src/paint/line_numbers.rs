//! Owns: line numbers (`w:lnNumType`, docx2svg's `_line_numbers`, measured by
//! `make_line_number_probe.py`): which of the body's lines are counted -- every line of
//! the body's own paragraphs, an empty one's too, not a table's, a header's, a footer's, a
//! note's, nor a paragraph's under `w:suppressLineNumbers` -- the count from `w:start` + 1,
//! restarting at each page (`newPage`) or section (`newSection`) or going on
//! (`continuous`), a number drawn where it is a multiple of `w:countBy`
//! ([`LineNumbers`], made once per layout); and each number's span, in the `LineNumber`
//! character style, its digits on whole device px, ending `w:distance` (360 twips
//! automatic) left of the text column's edge on the line's baseline ([`number_span`]).
//!
//! A section of several text columns is not numbered (not measured).
//!
//! Must not: place lines (the geometry has) or count a line twice.

use super::{PaintContext, Span, SpanKind};
use crate::engine::DocumentLayout;
use crate::geometry::LineSource;
use crate::input::RunFormatter;
use crate::naming::{paragraph_item, ItemName};
use crate::px::Px;
use ooxml_text::pieces::note_number_units;
use ooxml_text::FaceKey;
use ooxml_word_model::ids::{Anchor, Sub};
use ooxml_word_model::model::{Block, Paragraph, Run, RunProps};
use ooxml_word_model::value::{Text, Value};
use ooxml_word_model::Model;
use std::sync::Arc;

/// One number to draw beside a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineNumber {
    /// The line's paragraph: what the number is anchored on (its key; its path where no
    /// story's index holds it).
    pub name: ItemName,
    /// The line's number in its paragraph.
    pub line: usize,
    /// Its baseline.
    pub baseline: i64,
    /// The number.
    pub count: i64,
    /// The section that numbers it.
    pub section: usize,
}

/// The numbers each page draws beside its lines, in the order they are counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineNumbers {
    pages: Vec<Vec<LineNumber>>,
}

impl LineNumbers {
    /// The line numbers of `layout`: none where no section numbers its lines.
    pub fn of(layout: &DocumentLayout, model: &mut Model) -> LineNumbers {
        let sections = &layout.document.sections;
        if !sections.iter().any(|s| s.properties.line_numbering.is_some()) {
            return LineNumbers::default();
        }
        let mut out = LineNumbers {
            pages: vec![Vec::new(); layout.page_count()],
        };
        let mut count: Option<i64> = None;
        let mut last_section: Option<usize> = None;
        for (k, page) in out.pages.iter_mut().enumerate() {
            let Some(geometry) = layout.page_geometry(k) else {
                continue;
            };
            if geometry.info.blank {
                continue;
            }
            let mut lines: Vec<(i64, usize, &Arc<Paragraph>, usize)> = geometry
                .lines
                .iter()
                .filter_map(|l| {
                    let (paragraph, section) = numbered_paragraph(layout, l.source)?;
                    Some((l.baseline, l.number, paragraph, section))
                })
                .collect();
            lines.sort_by_key(|l| l.0);
            let mut first_on_page = true;
            for (baseline, number, paragraph, section) in lines {
                let Some(numbering) = sections.get(section).and_then(|s| {
                    let props = &s.properties;
                    props.line_numbering.as_ref().filter(|_| props.column_count <= 1)
                }) else {
                    continue;
                };
                let resolved = model.resolve_paragraph_props(&layout.document, paragraph);
                if resolved.get("suppressLineNumbers").is_some_and(Value::truthy) {
                    continue;
                }
                let restart = match &*numbering.restart {
                    "newSection" => last_section != Some(section),
                    "continuous" => false,
                    _ => first_on_page,
                };
                let from = match count {
                    Some(c) if !restart => c,
                    _ => numbering.start,
                };
                let n = from + 1;
                count = Some(n);
                first_on_page = false;
                last_section = Some(section);
                if numbering.count_by > 0 && n % numbering.count_by != 0 {
                    continue;
                }
                page.push(LineNumber {
                    name: paragraph_item(paragraph, Sub::Whole),
                    line: number,
                    baseline,
                    count: n,
                    section,
                });
            }
        }
        out
    }

    /// The numbers page `page` (0-based, blank pages counted) draws.
    pub fn on(&self, page: usize) -> &[LineNumber] {
        self.pages.get(page).map_or(&[], Vec::as_slice)
    }
}

/// The top-level body paragraph a line is of and its section, where the line can be
/// numbered: a body paragraph's own line or its drop cap's, not an endnote placed in the
/// flow.
fn numbered_paragraph(layout: &DocumentLayout, source: LineSource) -> Option<(&Arc<Paragraph>, usize)> {
    let (item, body) = match source {
        LineSource::Flow { item } => {
            let p = layout.flow.items.get(item)?.para()?;
            (p, p.body)
        }
        LineSource::DropCap { item } => {
            let p = layout.flow.items.get(item)?.para()?;
            (p, p.drop_cap.as_ref()?.body)
        }
        LineSource::Cell { .. } | LineSource::Opening { .. } => return None,
    };
    if layout.roles.get(body).is_some_and(Option::is_some) {
        return None;
    }
    match layout.document.body.get(body)? {
        Block::Paragraph(p) => Some((p, item.section)),
        Block::Table(_) => None,
    }
}

/// The span of `number` drawn for a page whose text column starts at `column_left`; `None`
/// where its face has no advance for a digit.
pub fn number_span(
    cx: &PaintContext<'_>,
    layout: &DocumentLayout,
    model: &mut Model,
    number: &LineNumber,
    column_left: Px,
) -> Option<Span> {
    let document = &layout.document;
    let styled = document
        .context
        .styles
        .as_deref()
        .is_some_and(|s| s.styles.contains_key("LineNumber"));
    let run = Run {
        text: Arc::from("0"),
        properties: styled.then(|| {
            Arc::new(RunProps {
                style_id: Some(Text::from("LineNumber")),
                ..RunProps::default()
            })
        }),
        ..Run::default()
    };
    let paragraph = Paragraph {
        runs: Arc::from(vec![run]),
        ..Paragraph::default()
    };
    let resolved = model.resolve_paragraph(document, &paragraph);
    let props = resolved.runs.first()?;
    let text: Vec<char> = number.count.to_string().chars().collect();
    let format = RunFormatter::new(document, props).format(*text.first()?).clone();
    let key = FaceKey {
        family: Arc::clone(&format.face),
        bold: format.bold,
        italic: format.italic,
    };
    let half_points = u32::try_from(format.half_points).ok()?;
    let mut widths = Vec::new();
    for ch in &text {
        let advance = cx.faces.advance(&key, *ch)?;
        let units = note_number_units(advance.advance, advance.units_per_em, half_points).ok()?;
        widths.push(super::spans::px_at(Px::ZERO, units));
    }
    let section = document.sections.get(number.section)?;
    let distance = section
        .properties
        .line_numbering
        .as_ref()
        .and_then(|n| n.distance)
        .unwrap_or(360);
    let end = Px::int((column_left - Px::twips(distance)).round_half_up());
    let mut x = widths.iter().fold(end, |x, w| x - *w);
    let mut xs = Vec::with_capacity(widths.len());
    for w in widths {
        xs.push(x.into());
        x += w;
    }
    Some(Span {
        name: number_name(number),
        chars: text,
        xs,
        y: number.baseline,
        end: x,
        face: Arc::clone(&format.face),
        bold: format.bold,
        italic: format.italic,
        half_points: format.half_points,
        color: super::spans::colour(props.get("color").and_then(Value::str)),
        underline: None,
        strike: false,
        double_strike: false,
        highlight: None,
        shading: None,
        kind: SpanKind::LineNumber,
        // docx2svg's line-number span leaves its line's baseline unset.
        line_baseline: 0,
        vertical_align: None,
        effects: None,
    })
}

/// What a line's number is drawn for: its section's line numbering, anchored on the line's
/// paragraph (by the path it is made with where no story's index holds the paragraph).
fn number_name(number: &LineNumber) -> ItemName {
    let anchor = number
        .name
        .anchor()
        .zip(u32::try_from(number.section).ok())
        .map(|(paragraph, section)| Anchor {
            block: paragraph.block,
            sub: Sub::LineNumbers(section),
        });
    ItemName::of(anchor, || format!("w:sectPr[{}]/w:lnNumType", number.section + 1))
}
