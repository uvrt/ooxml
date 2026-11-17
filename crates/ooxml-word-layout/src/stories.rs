//! Owns: the extension point a page's header and footer heights come through
//! ([`StoryHeights`]), and [`StoryTable`], which stacks each section's header and footer
//! of each kind as a page stacks paragraphs -- each story laid out as the body of one
//! section of one column (docx2svg's `paginate.story_document`, `stack_height`,
//! `story_references`, `_stories`).
//!
//! A story's list items are counted from scratch in it; its tables are placed by the
//! table layout as a page places them.
//!
//! A paragraph holding a floating drawing text wraps around is stacked as docx2svg's
//! estimated stand-in for it (`placeholder_item`); a story holding one is reported in
//! [`StoryTable::unsupported`], since Word lays its text out beside the drawing.
//!
//! Must not: draw a story.

use crate::flow::{FlowContext, FlowItem, FlowMaker, FlowState, Labels, ParaItem};
use crate::heights::LineHeight;
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use crate::sections::{Sections, StoryKind};
use crate::vertical::LineRule;
use crate::vertical::{keeps_space_before_at_page_top, page_top_gap, paragraph_gap};
use crate::{faces::Faces, table::TableLayout};
use ooxml_base::Locale;
use ooxml_base::Q;
use ooxml_word_model::model::Paragraph;
use ooxml_word_model::model::{Block, Section, SectionProps};
use ooxml_word_model::value::{Text, Value};
use ooxml_word_model::{Model, WordDocument};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// The heights a page's header and footer reach from their edge distances.
pub trait StoryHeights {
    /// `(header, footer)` of `kind` in `section`.
    fn heights(&self, section: usize, kind: StoryKind) -> (Px, Px);
}

/// No headers or footers.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStories;

impl StoryHeights for NoStories {
    fn heights(&self, _section: usize, _kind: StoryKind) -> (Px, Px) {
        (Px::ZERO, Px::ZERO)
    }
}

/// Each section's header and footer heights by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoryTable {
    by_section: Vec<BTreeMap<StoryKind, (Px, Px)>>,
    /// The stories holding what this layout does not lay out in one, by relationship id.
    pub unsupported: Vec<String>,
}

impl StoryHeights for StoryTable {
    fn heights(&self, section: usize, kind: StoryKind) -> (Px, Px) {
        self.by_section
            .get(section)
            .and_then(|kinds| kinds.get(&kind))
            .copied()
            .unwrap_or((Px::ZERO, Px::ZERO))
    }
}

/// What laying out stories needs.
pub struct StoryContext<'a> {
    /// The document.
    pub document: &'a WordDocument,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale.
    pub locale: &'a Locale,
    /// The table layout.
    pub tables: &'a dyn TableLayout,
}

/// Per section, its header and footer relationship id of each type: what it states, and
/// what it does not state inherited from the section before (`story_references`).
pub fn story_references(sections: &[Section]) -> Vec<(BTreeMap<Text, Text>, BTreeMap<Text, Text>)> {
    let mut headers: BTreeMap<Text, Text> = BTreeMap::new();
    let mut footers: BTreeMap<Text, Text> = BTreeMap::new();
    let mut out = Vec::with_capacity(sections.len());
    for section in sections {
        for (kind, id) in section.properties.headers.iter() {
            headers.insert(kind.clone(), id.clone());
        }
        for (kind, id) in section.properties.footers.iter() {
            footers.insert(kind.clone(), id.clone());
        }
        out.push((headers.clone(), footers.clone()));
    }
    out
}

/// A story's one section: the page's size and margins, one text column.
fn story_section(props: &SectionProps, paragraphs: usize) -> Section {
    let mut one = props.clone();
    one.column_count = 1;
    one.columns = Arc::from(Vec::new());
    one.start_type = None;
    one.headers = Arc::from(Vec::new());
    one.footers = Arc::from(Vec::new());
    one.title_page = false;
    Section {
        properties: Arc::new(one),
        first_paragraph: 0,
        last_paragraph: paragraphs,
    }
}

/// What in a story this layout does not lay out: a floating drawing text wraps around.
fn unsupported_in(blocks: &[Block]) -> Option<&'static str> {
    let wraps = |p: &ooxml_word_model::model::Paragraph| {
        p.runs.iter().any(|r| r.anchors.iter().any(crate::floating::moves_text))
    };
    blocks
        .iter()
        .any(|b| matches!(b, Block::Paragraph(p) if wraps(p)))
        .then_some("a floating drawing text wraps around")
}

/// A story's paragraphs' list labels, document order (cells included), counted from
/// scratch in the story (`flow`'s `ListCounters` over a story document).
pub fn story_labels(model: &mut Model, document: &WordDocument, blocks: &[Block]) -> Vec<Option<String>> {
    let (paragraphs, _) = crate::flow::body_paragraphs(blocks);
    let mut counters = ooxml_word_model::labels::Counters::default();
    paragraphs
        .iter()
        .map(|p| {
            let pp = model.resolve_paragraph_props(document, p);
            let num_id = pp.get("numPr.numId").and_then(Value::int).unwrap_or(0);
            if num_id == 0 {
                return None;
            }
            let ilvl = pp.get("numPr.ilvl").and_then(Value::int).unwrap_or(0);
            counters.item(&document.context.numbering, num_id, ilvl)
        })
        .collect()
}

/// How far a story's items reach stacked from its edge distance (`stack_height`): the
/// first paragraph's space before kept, the gaps and borders between them, and the last
/// paragraph's space after; a table's rows as placed. A stop ends the stack.
pub fn stack_height(items: &[FlowItem], mode15: bool) -> Px {
    let mut y = Px::ZERO;
    let mut previous: Option<&FlowItem> = None;
    let mut first = true;
    for item in items {
        let p = match item {
            FlowItem::Para(p) => p,
            FlowItem::Table(t) => {
                // Its rows as a page places them, from where the stack is.
                if !first {
                    y += crate::paginate::table_gap(previous, t);
                }
                let request = crate::table::TableRequest {
                    y,
                    bottom: y + Px::int(10_000_000),
                    row: 0,
                    part: Vec::new(),
                    first_on_page: first,
                    mode15,
                    split_kept: false,
                    stop: None,
                };
                match t.flow.place(&request) {
                    Ok(laid) => y = laid.end,
                    Err(_) => break,
                }
                first = false;
                previous = Some(item);
                continue;
            }
            FlowItem::Stop(_) => break,
        };
        for (number, height) in p.measured.heights.iter().enumerate() {
            if number == 0 {
                let gap = if first {
                    let keeps = keeps_space_before_at_page_top(true, false, mode15);
                    page_top_gap(p.before, keeps, 0).0
                } else {
                    match previous {
                        Some(FlowItem::Para(before)) => {
                            let same = p.style == before.style;
                            let own = if p.contextual && same { 0 } else { p.before };
                            let after = if before.contextual && same { 0 } else { before.after };
                            paragraph_gap(after, own).0
                        }
                        _ => Px::twips(p.before),
                    }
                };
                y += gap + p.top_border;
            }
            y += height.pitch;
            first = false;
        }
        y += p.border;
        previous = Some(item);
    }
    if let Some(FlowItem::Para(p)) = previous {
        y += Px::twips(p.after);
    }
    y
}

type HeightKey = (Text, i64, i64, [i64; 7]);

/// How far a header or footer reaches from its edge distance: its blocks laid out as the
/// body of one section of one column with the page's size and margins, stacked
/// (`story_height`).
pub fn story_height(
    cx: &StoryContext<'_>,
    model: &mut Model,
    state: &mut FlowState,
    blocks: &[Block],
    props: &SectionProps,
) -> Px {
    let document = cx.document;
    let paragraphs = blocks.iter().filter(|b| matches!(b, Block::Paragraph(_))).count();
    let sections = Sections::new(&[story_section(props, paragraphs)], false);
    let labels = story_labels(model, document, blocks);
    let fcx = FlowContext {
        document,
        blocks,
        sections: &sections,
        labels: Labels::Given(&labels),
        faces: cx.faces,
        locale: cx.locale,
        tables: cx.tables,
        roles: &[],
    };
    let flow = FlowMaker::new(&fcx, model, state).build();
    let width = sections.get(0).map_or(0, |s| s.column_width(0));
    let items: Vec<FlowItem> = flow
        .items
        .into_iter()
        .map(|item| match (&item, &item.para().and_then(|p| blocks.get(p.body))) {
            (FlowItem::Para(p), Some(Block::Paragraph(paragraph))) if wraps_text(paragraph) => {
                FlowItem::Para(Arc::new(stand_in(document, model, paragraph, p, width)))
            }
            _ => item,
        })
        .collect();
    stack_height(&items, crate::input::rules(document).mode15())
}

/// Whether a story paragraph holds a floating drawing text wraps around: docx2svg lays
/// it out as an estimated stand-in ([`stand_in`]).
fn wraps_text(paragraph: &Paragraph) -> bool {
    paragraph
        .runs
        .iter()
        .any(|r| r.anchors.iter().any(crate::floating::moves_text))
}

/// A story paragraph holding a floating drawing text wraps around as docx2svg stacks it
/// (`placeholder_item`, `_estimated_lines`): its text at its first text run's size, a
/// line 1.17 em tall per line of half-em characters wrapped at the column's width, and a
/// line as tall as each inline drawing whose extent it states; its own space before and
/// after, no borders, no style.
fn stand_in(
    document: &WordDocument,
    model: &mut Model,
    paragraph: &Paragraph,
    item: &ParaItem,
    width: i64,
) -> ParaItem {
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
    let heights = pitches
        .into_iter()
        .map(|h| LineHeight {
            pitch: Px::from(h),
            text_above: Px::from(h),
            text_below: Px::ZERO,
            natural: Px::from(h),
            rule: LineRule::Exact,
            line: 0,
            mixed: false,
            object_drop: None,
        })
        .collect();
    let measured = MeasuredParagraph {
        heights,
        page_breaks: Vec::new(),
        column_breaks: Vec::new(),
        undrawn: Vec::new(),
        auto_hyphen_ends: Vec::new(),
        ..(*item.measured).clone()
    };
    let spacing = |key: &str| resolved.paragraph.get(key).and_then(Value::int).unwrap_or(0);
    ParaItem {
        measured: Arc::new(measured),
        before: spacing("spacing.before"),
        after: spacing("spacing.after"),
        contextual: false,
        style: None,
        border: Px::ZERO,
        top_border: Px::ZERO,
        keep_next: false,
        keep_lines: false,
        page_break_before: false,
        widow: Some(false),
        ..item.clone()
    }
}

impl StoryTable {
    /// Every section's header and footer heights by kind.
    pub fn compute(cx: &StoryContext<'_>, model: &mut Model, state: &mut FlowState) -> StoryTable {
        let document = cx.document;
        let mut table = StoryTable::default();
        let mut memo: HashMap<HeightKey, Px> = HashMap::new();
        for (index, (headers, footers)) in story_references(&document.sections).into_iter().enumerate() {
            let Some(section) = document.sections.get(index) else {
                continue;
            };
            let props = &section.properties;
            let mut height = |references: &BTreeMap<Text, Text>, kind: StoryKind| -> Px {
                let Some(id) = references.get(kind.name()) else {
                    return Px::ZERO;
                };
                let Some(blocks) = document.stories.get(id).filter(|b| !b.is_empty()) else {
                    return Px::ZERO;
                };
                let m = &props.margins;
                let key = (
                    id.clone(),
                    props.page_size.width,
                    props.page_size.height,
                    [m.top, m.right, m.bottom, m.left, m.header, m.footer, m.gutter],
                );
                if let Some(found) = memo.get(&key) {
                    return *found;
                }
                if let Some(why) = unsupported_in(blocks) {
                    table.unsupported.push(format!("{id}: {why}"));
                }
                let found = story_height(cx, model, state, blocks, props);
                memo.insert(key, found);
                found
            };
            let mut kinds = BTreeMap::new();
            for kind in StoryKind::ALL {
                kinds.insert(kind, (height(&headers, kind), height(&footers, kind)));
            }
            table.by_section.push(kinds);
        }
        table
    }
}
