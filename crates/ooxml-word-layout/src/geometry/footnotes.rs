//! Owns: the footnotes a page draws (docx2svg's `_footnotes`, `_note_stack` and
//! `_column_footnotes`): the separator -- the continuation separator's on a page a note
//! goes on to -- and under it the rest of a note from the page before and the notes placed
//! on the page, laid out as a flow of their own and stacked so that the stack's foot is on
//! the text area's foot (under `w:pos="beneathText"`, from where the body's stack ends);
//! the first note's space before stands over the separator.
//!
//! On a page of text columns: below mode 15 each column's notes at its own foot, broken in
//! its width, its separator's line cut at its right edge; in mode 15 the page's note area
//! ([`crate::paginate::NoteArea`]) under one separator over the first column; a
//! one-column section sharing the page keeps its notes across the text area.
//!
//! Must not: decide which notes go on a page or column (the paginator has), or lay out
//! endnotes (they are in the body's flow).

use super::linebox::{baseline_in_box, text_band, LineBox};
use super::lines::{column_left, DrawnLine, LineSource};
use super::stories::{StoryJob, StoryPage};
use super::walk::{walk_between, WalkFrom, Walked};
use super::PageFrame;
use crate::engine::DocumentLayout;
use crate::flow::{Flow, FlowContext, FlowItem, FlowMaker, Labels};
use crate::notes::flowed::note_section;
use crate::notes::{separator, NoteLine, NotesPart};
use crate::paginate::{LaidPage, NoteArea, PageColumn, PlacedLine, PlacedNote};
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use crate::sections::Sections;
use ooxml_word_model::model::{Block, SectionProps};
use std::sync::Arc;

/// One note's part on the page: its id, where it goes on from (a carried note's first
/// line), and the first of its lines not on the page.
type Part = (Option<i64>, Option<NoteLine>, Option<NoteLine>);

/// A walk of the note flow: from `(item, line)` up to `(item, line)`.
type Walk = ((usize, usize), (usize, usize));

/// The parts of a foot's notes, in drawing order (`_note_stack`'s `parts`).
fn parts(carried: &[NoteLine], placed: &[PlacedNote]) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    for (k, line) in carried.iter().enumerate() {
        if k == 0 || carried.get(k - 1).map(|l| l.note) != Some(line.note) {
            out.push((line.note, Some(*line), None));
        }
    }
    for note in placed {
        if note.taken > 0 {
            if let Some(first) = note.lines.first() {
                out.push((first.note, None, note.lines.get(note.taken).copied()));
            }
        }
    }
    out
}

/// A footnote's paragraphs.
fn note_paragraphs(document: &ooxml_word_model::WordDocument, note: Option<i64>) -> Vec<Block> {
    note.and_then(|n| document.footnotes.notes.get(&n))
        .map(|b| b.iter().filter(|b| matches!(b, Block::Paragraph(_))).cloned().collect())
        .unwrap_or_default()
}

/// The separator's blocks and the notes' paragraphs from where each goes on; how many
/// blocks the separator has; and per part where its paragraph 0 would be.
fn note_blocks(
    document: &ooxml_word_model::WordDocument,
    parts: &[Part],
    kind: &str,
) -> (Vec<Block>, usize, Vec<usize>) {
    let mut blocks: Vec<Block> = separator(document, kind, NotesPart::Footnotes)
        .into_iter()
        .filter(|b| matches!(b, Block::Paragraph(_)))
        .collect();
    let separators = blocks.len();
    let mut first_of = Vec::new();
    for (note, first, _) in parts {
        let skip = first.map_or(0, |l| l.paragraph);
        first_of.push(blocks.len() - skip);
        blocks.extend(note_paragraphs(document, *note).into_iter().skip(skip));
    }
    (blocks, separators, first_of)
}

/// The walks stacked one after the other from `from.top`.
fn stacked(items: &[FlowItem], walks: &[Walk], from: WalkFrom) -> Walked {
    let mut out = Walked::default();
    let mut y = from.top;
    for (start, stop) in walks {
        let walked = walk_between(items, &WalkFrom { top: y, ..from }, *start, *stop);
        y = walked.end;
        out.lines.extend(walked.lines);
        out.rows.extend(walked.rows);
    }
    out.end = y;
    out
}

/// The first note's space before (`before` twips) stands over the separator, not under
/// it: the separator's lines move down by it, rounded.
fn lower_separator(walked: &mut Walked, separators: usize, before: i64) {
    let shift = Px::twips(before).round_half_up();
    for line in walked.lines.iter_mut().filter(|l| l.source.item() < separators) {
        line.baseline += shift;
        line.top += Px::int(shift);
        line.band = (line.band.0 + shift, line.band.1 + shift);
    }
}

/// `props` as one text column `width` twips wide (`column_section`): what a footnote in
/// one of its columns is broken in, its right margin moved in.
fn column_props(props: &SectionProps, width: i64) -> SectionProps {
    let mut one = props.clone();
    let m = &props.margins;
    one.margins.right = props.page_size.width - m.left - m.gutter - width;
    one.column_count = 1;
    one.columns = Arc::from(Vec::new());
    one
}

/// Where a stack of notes goes: its foot on a bottom, or (`beneathText`) its top at a y.
#[derive(Debug, Clone, Copy)]
enum Place {
    Foot(Px),
    Under(Px),
}

/// One stack of notes to draw: its notes, the section it is laid out in, its left edge,
/// the right edge a separator's line is cut at, where it goes and the column it is in.
struct Stack<'a> {
    carried: &'a [NoteLine],
    placed: &'a [PlacedNote],
    props: &'a SectionProps,
    left: Px,
    clip: Option<Px>,
    place: Place,
    column: Option<usize>,
    /// Only the separator (the note area's, in mode 15).
    separator_only: bool,
}

/// `blocks` as a flow of their own on `props`'s page, one section.
pub(super) fn story_flow(job: &mut StoryJob<'_, '_>, blocks: &Arc<[Block]>, props: &SectionProps) -> Flow {
    let sections = Sections::new(&[note_section(props, blocks.len())], false);
    let fcx = FlowContext {
        document: job.cx.document,
        blocks,
        sections: &sections,
        labels: Labels::None,
        faces: job.cx.faces,
        locale: job.cx.locale,
        tables: job.cx.tables,
        roles: &[],
    };
    FlowMaker::new(&fcx, job.model, job.state).build()
}

/// The separator and the notes of `stack`, stacked (`_note_stack`); `None` where there is
/// nothing to draw.
fn note_stack(job: &mut StoryJob<'_, '_>, mode15: bool, stack: &Stack<'_>) -> Result<Option<StoryPage>, &'static str> {
    let document = job.cx.document;
    let parts = parts(stack.carried, stack.placed);
    if parts.is_empty() && !stack.separator_only {
        return Ok(None);
    }
    let carried = !stack.carried.is_empty();
    let kind = if carried && !mode15 {
        "continuationSeparator"
    } else {
        "separator"
    };
    let (blocks, separators, first_of) = note_blocks(document, &parts, kind);
    let blocks: Arc<[Block]> = blocks.into();
    let flow = story_flow(job, &blocks, stack.props);
    if flow.items.iter().any(|i| matches!(i, FlowItem::Stop(_))) {
        return Err("a footnote that cannot be laid out");
    }
    let items = &flow.items;
    let last = parts.last().and_then(|p| p.2);
    let end = match (last, first_of.last()) {
        (Some(l), Some(f)) => (f + l.paragraph, l.line),
        _ => (items.len(), 0),
    };
    let walks: Vec<Walk> = match stack.carried.first().filter(|l| l.line > 0) {
        Some(l) => vec![((0, 0), (separators, 0)), ((separators, l.line), end)],
        None => vec![((0, 0), end)],
    };
    let from = |top: Px| WalkFrom {
        top,
        left: stack.left,
        mode15,
        bottom: None,
    };
    let mut walked = match stack.place {
        Place::Under(y) => stacked(items, &walks, from(y)),
        Place::Foot(bottom) => {
            let mut height = stacked(items, &walks, from(Px::ZERO)).end;
            if let (None, Some(FlowItem::Para(p))) = (last, items.last()) {
                height += Px::twips(p.after);
            }
            let mut walked = stacked(items, &walks, from(bottom - height));
            let before = match (carried, items.get(separators)) {
                (false, Some(FlowItem::Para(p))) => p.before,
                _ => 0,
            };
            if before != 0 {
                lower_separator(&mut walked, separators, before);
            }
            walked
        }
    };
    if stack.column.is_some() {
        for line in &mut walked.lines {
            line.column = stack.column;
        }
    }
    Ok(Some(StoryPage {
        kind: "footnotes",
        part: Arc::from(""),
        blocks,
        flow,
        walked,
        clip: stack.clip,
        broken: crate::paginate::LineStore::new(),
    }))
}

/// The footnotes page `laid` draws: at its foot, or on a page of columns at its columns'
/// feet or in its note area; empty where it has none, `Err` with why where it holds them
/// in a way this does not draw.
pub fn page_footnotes(
    job: &mut StoryJob<'_, '_>,
    layout: &DocumentLayout,
    frame: &PageFrame,
    laid: &LaidPage,
) -> Result<Vec<StoryPage>, &'static str> {
    if laid.columns.iter().any(|c| c.multi) {
        return column_footnotes(job, layout, frame, laid);
    }
    let section = layout.sections.get(frame.section).ok_or("no section")?;
    let stack = Stack {
        carried: &laid.carried,
        placed: &laid.notes,
        props: &section.props,
        left: frame.left,
        clip: None,
        place: place(&section.props, laid, frame.bottom),
        column: None,
        separator_only: false,
    };
    Ok(note_stack(job, layout.mode15, &stack)?.into_iter().collect())
}

/// Where a stack not cut at a column's edge goes: its foot on `bottom`, or under
/// `w:pos="beneathText"` straight under the body's last line.
fn place(props: &SectionProps, laid: &LaidPage, bottom: Px) -> Place {
    if props.footnotes.position.as_deref() == Some("beneathText") {
        Place::Under(laid.body_end)
    } else {
        Place::Foot(bottom)
    }
}

/// A text column's left and right edges (`column_edges`).
fn column_edges(layout: &DocumentLayout, frame: &PageFrame, offset: i64, width: i64) -> (Px, Px) {
    let left = column_left(layout, frame, offset);
    (left, left + Px::twips(width))
}

/// Whether a column has notes at its foot.
fn noted(column: &PageColumn) -> bool {
    !column.notes.is_empty() || !column.carried.is_empty()
}

/// The footnotes of a page of text columns (`_column_footnotes`), then those of a
/// one-column section sharing the page, across its text area.
fn column_footnotes(
    job: &mut StoryJob<'_, '_>,
    layout: &DocumentLayout,
    frame: &PageFrame,
    laid: &LaidPage,
) -> Result<Vec<StoryPage>, &'static str> {
    let mut out = Vec::new();
    if layout.mode15 {
        if let Some(area) = &laid.note_area {
            out.extend(note_area(job, layout, (frame, laid), area)?);
        }
    } else {
        for column in laid.columns.iter().filter(|c| c.multi && noted(c)) {
            let section = layout.sections.get(column.section).ok_or("no section")?;
            let (left, right) = column_edges(layout, frame, column.offset, column.width);
            let props = column_props(&section.props, column.width);
            let stack = Stack {
                carried: &column.carried,
                placed: &column.notes,
                props: &props,
                left,
                clip: Some(right),
                place: Place::Foot(frame.bottom),
                column: Some(column.column),
                separator_only: false,
            };
            out.extend(note_stack(job, false, &stack)?);
        }
    }
    // A one-column section sharing the page: its notes across the text area, at the
    // page's foot (not measured where the columns' notes are drawn too: an approximation).
    for column in laid.columns.iter().filter(|c| !c.multi && noted(c)) {
        let section = layout.sections.get(column.section).ok_or("no section")?;
        let stack = Stack {
            carried: &column.carried,
            placed: &column.notes,
            props: &section.props,
            left: frame.left,
            clip: None,
            place: place(&section.props, laid, frame.bottom),
            column: None,
            separator_only: false,
        };
        out.extend(note_stack(job, layout.mode15, &stack)?);
    }
    Ok(out)
}

/// The page's note area in mode 15: one separator over the first column, its foot at the
/// area's top, and the notes' lines where the area's columns placed them.
fn note_area(
    job: &mut StoryJob<'_, '_>,
    layout: &DocumentLayout,
    (frame, laid): (&PageFrame, &LaidPage),
    area: &NoteArea,
) -> Result<Vec<StoryPage>, &'static str> {
    let section = layout.sections.get(area.section).ok_or("no section")?;
    let top = frame.bottom - area.height;
    let first_width = section.column_width(0);
    let (left, _) = column_edges(layout, frame, 0, first_width);
    let props = column_props(&section.props, first_width);
    let stack = Stack {
        carried: &[],
        placed: &[],
        props: &props,
        left,
        clip: None,
        place: place(&section.props, laid, top),
        column: Some(0),
        separator_only: true,
    };
    let mut out: Vec<StoryPage> = note_stack(job, true, &stack)?.into_iter().collect();
    let blocks: Arc<[Block]> = area
        .notes
        .iter()
        .flat_map(|n| note_paragraphs(job.cx.document, Some(*n)))
        .collect();
    let flow = story_flow(job, &blocks, &section.props);
    let mut walked = Walked::default();
    for placed in &area.lines {
        let left = column_left(layout, frame, placed.offset);
        let measured = area.broken.get(&placed.item).map(|r| &r.measured);
        walked
            .lines
            .extend(area_line(&flow.items, (placed, measured), left, top));
    }
    out.push(StoryPage {
        kind: "footnotes",
        part: Arc::from(""),
        blocks,
        flow,
        walked,
        clip: None,
        broken: area.broken.clone(),
    });
    Ok(out)
}

/// A line of the note area as its columns placed it -- broken again there (`again`), or
/// as its flow broke it -- the area `top` px below the page's top.
fn area_line(
    items: &[FlowItem],
    (placed, again): (&PlacedLine, Option<&Arc<MeasuredParagraph>>),
    left: Px,
    top: Px,
) -> Option<DrawnLine> {
    let item = items.get(placed.item)?.para()?;
    let measured = again.unwrap_or(&item.measured);
    if measured.undrawn.contains(&placed.line) {
        return None;
    }
    let height = measured.heights.get(placed.line)?;
    let space_after = if placed.line + 1 == measured.heights.len() {
        let same_next = items
            .get(placed.item + 1)
            .and_then(FlowItem::para)
            .is_some_and(|next| next.style == item.style);
        let own_after = if item.contextual && same_next { 0 } else { item.after };
        item.border + Px::twips(own_after)
    } else {
        Px::ZERO
    };
    let line_top = placed.top + top;
    let b = LineBox::new(*height, placed.space_before, space_after);
    Some(DrawnLine {
        source: LineSource::Flow { item: placed.item },
        block: Some(item.block),
        body: Some(item.body),
        number: placed.line,
        top: line_top,
        pitch: placed.pitch,
        baseline: baseline_in_box(line_top - b.space_before, &b),
        band: text_band(line_top - b.space_before, &b),
        column: placed.column,
        left,
        white_text: false,
    })
}
