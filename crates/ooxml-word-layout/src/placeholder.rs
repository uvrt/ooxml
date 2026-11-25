//! Owns: what the layout cannot lay out but carries on past (docx2svg's `paginate.CARRY_ON`):
//! a body paragraph holding a drawing it does not model (a VML shape it cannot read, an
//! embedded object) taken into the flow as a [`Placeholder`] of its estimated lines
//! (`placeholder_item`), so that everything after it is laid out; and those estimated lines
//! (`_estimated_lines`: its text at its first text run's size, a line 1.17 em tall per line
//! of half-em characters wrapped at the column's width, and a line as tall as each inline
//! drawing whose extent it states), which a header's or footer's stand-in is stacked with
//! too.
//!
//! A placeholder's lines are no lines of text: nothing is drawn for them, and the page that
//! holds one says so where it is painted.
//!
//! Must not: measure the paragraph's text, or place anything on a page.

use crate::flow::FlowItem;
use crate::heights::LineHeight;
use crate::input::section_geometry;
use crate::paginate::LaidPage;
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use crate::sections::SectionGeometry;
use crate::vertical::LineRule;
use ooxml_base::Q;
use ooxml_text::breaks::Line;
use ooxml_text::measure::Measured;
use ooxml_word_model::ids::BlockKey;
use ooxml_word_model::model::Paragraph;
use ooxml_word_model::value::{Text, Value};
use ooxml_word_model::{Model, WordDocument};
use std::sync::Arc;

/// What a placeholder stands for (`paginate.Placeholder`): a paragraph the layout cannot
/// lay out, taken into the flow as its estimated lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// Why, a stable label (`drawing`).
    pub reason: &'static str,
    /// What about it is not laid out.
    pub message: &'static str,
    /// Its element's path (`w:body/w:p[2]`).
    pub path: Text,
    /// Its paragraph's key, beside its path (`None`: one no story's index holds).
    pub key: Option<BlockKey>,
    /// Its height is estimated from its text (not stated in the file).
    pub estimated: bool,
}

/// The reason of a paragraph holding a drawing the layout does not model.
pub const DRAWING: &str = "drawing";

/// What such a paragraph is, as docx2svg says it (`_unlaid`).
const DRAWING_MESSAGE: &str =
    "a drawing this model does not lay out (a VML shape it cannot read, an embedded object, a wrap it does not know)";

impl Placeholder {
    /// The placeholder `paragraph` is drawn as, where the layout cannot lay it out but
    /// carries on past it: one holding a drawing it does not model.
    #[must_use]
    pub fn of(paragraph: &Paragraph) -> Option<Placeholder> {
        let unmodelled = paragraph
            .runs
            .iter()
            .any(|r| r.breaks.iter().any(|(_, kind)| &**kind == "drawing"));
        unmodelled.then(|| Placeholder {
            reason: DRAWING,
            message: DRAWING_MESSAGE,
            path: paragraph.path.clone(),
            key: paragraph.key,
            estimated: true,
        })
    }

    /// What the layout says of it drawn on a page, `part` the story it is in where it is in
    /// one (`Layout._box`'s message).
    #[must_use]
    pub fn drawn_message(&self, part: Option<&str>) -> String {
        let at = match part {
            Some(part) => format!("{} in {part}", self.path),
            None => self.path.to_string(),
        };
        let size = if self.estimated {
            "its estimated height"
        } else {
            "its stated height"
        };
        format!(
            "{at} is drawn as a placeholder of {size} ({}); the layout carries on after it",
            self.message
        )
    }
}

/// `paragraph`'s lines as a placeholder estimates them, `width` twips wide (`_estimated_lines`).
pub fn estimated_heights(
    document: &WordDocument,
    model: &mut Model,
    paragraph: &Paragraph,
    width: i64,
) -> Vec<LineHeight> {
    let resolved = model.resolve_paragraph(document, paragraph);
    let first = paragraph.runs.iter().position(|r| !r.text.is_empty());
    let half_points = first
        .and_then(|k| resolved.runs.get(k))
        .and_then(|props| props.get("sz").and_then(Value::int))
        .filter(|sz| *sz != 0)
        .unwrap_or(24);
    let em = Q::new(i128::from(half_points), 2) * Q::new(300, 72);
    let chars: usize = paragraph.runs.iter().map(|r| r.text.chars().count()).sum();
    let column = Px::twips(width.max(1)).value().max(em);
    let count = if chars == 0 {
        1
    } else {
        let need = Q::from_integer(i128::try_from(chars).unwrap_or(i128::MAX)) * em * Q::new(1, 2) / column;
        usize::try_from(need.ceil().to_integer()).unwrap_or(1).max(1)
    };
    let mut pitches = vec![em * Q::new(117, 100); count];
    for run in paragraph.runs.iter() {
        for (_, kind) in run.breaks.iter() {
            if let Some(rest) = kind.strip_prefix("drawing:") {
                let cy: i64 = rest.split(':').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                pitches.push(Q::new(i128::from(cy), 12700) * Q::new(300, 72));
            }
        }
    }
    pitches
        .into_iter()
        .map(|h| {
            LineHeight::new(
                Px::from(h),
                Px::from(h),
                Px::ZERO,
                Px::from(h),
                LineRule::Exact,
                0,
                false,
            )
        })
        .collect()
}

/// `paragraph` (in section `section`) measured as a placeholder: its estimated lines at the
/// section's first column's width, none of them holding text.
pub fn placeholder_measured(
    document: &WordDocument,
    model: &mut Model,
    paragraph: &Paragraph,
    section: &SectionGeometry,
) -> MeasuredParagraph {
    let heights = estimated_heights(document, model, paragraph, section.column_width(0));
    let resolved = model.resolve_paragraph(document, paragraph);
    let empty = Line {
        start: 0,
        end: 0,
        forced: false,
        emergency: false,
        hyphenated: false,
    };
    MeasuredParagraph {
        measured: Arc::new(Measured {
            pieces: Vec::new(),
            lines: vec![empty; heights.len()],
        }),
        geometry: Arc::new(section_geometry(document, &resolved.paragraph, section)),
        heights,
        page_breaks: Vec::new(),
        column_breaks: Vec::new(),
        undrawn: Vec::new(),
        auto_hyphen_ends: Vec::new(),
    }
}

/// The placeholders page `laid` holds lines of, each once, in the order placed: its flow
/// item (of `items`) and what it stands for.
pub fn placeholders_on<'f>(items: &'f [FlowItem], laid: &LaidPage) -> Vec<(usize, &'f Placeholder)> {
    let mut out: Vec<(usize, &Placeholder)> = Vec::new();
    for line in &laid.lines {
        if out.last().is_some_and(|(item, _)| *item == line.item) {
            continue;
        }
        if let Some(placeholder) = items
            .get(line.item)
            .and_then(FlowItem::para)
            .and_then(|p| p.placeholder.as_deref())
        {
            out.push((line.item, placeholder));
        }
    }
    out
}
