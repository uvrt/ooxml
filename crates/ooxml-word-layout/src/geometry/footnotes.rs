//! Owns: the footnotes at a page's foot as drawn (docx2svg's `_footnotes` and
//! `_note_stack`, on a page of one column): the separator -- the continuation separator's
//! on a page a note goes on to -- and under it the rest of a note from the page before and
//! the notes placed on the page, laid out as a flow of their own and stacked so that the
//! stack's foot is on the text area's foot; the first note's space before stands over the
//! separator.
//!
//! Must not: decide which notes go on a page (the paginator has), or lay out notes in
//! columns, under `beneathText`, or of endnotes (endnotes are in the body's flow).

use super::stories::{StoryJob, StoryPage};
use super::walk::{walk_between, WalkFrom, Walked};
use super::PageFrame;
use crate::engine::DocumentLayout;
use crate::flow::{FlowContext, FlowItem, FlowMaker, Labels};
use crate::notes::flowed::note_section;
use crate::notes::{separator, NoteLine, NotesPart};
use crate::paginate::LaidPage;
use crate::px::Px;
use crate::sections::Sections;
use ooxml_word_model::model::Block;
use std::sync::Arc;

/// One note's part on the page: its id, where it goes on from (a carried note's first
/// line), and the first of its lines not on the page.
type Part = (Option<i64>, Option<NoteLine>, Option<NoteLine>);

/// A walk of the note flow: from `(item, line)` up to `(item, line)`.
type Walk = ((usize, usize), (usize, usize));

/// The parts of the page's notes, in drawing order (`_note_stack`'s `parts`).
fn parts(laid: &LaidPage) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    for (k, line) in laid.carried.iter().enumerate() {
        if k == 0 || laid.carried.get(k - 1).map(|l| l.note) != Some(line.note) {
            out.push((line.note, Some(*line), None));
        }
    }
    for placed in &laid.notes {
        if placed.taken > 0 {
            if let Some(first) = placed.lines.first() {
                out.push((first.note, None, placed.lines.get(placed.taken).copied()));
            }
        }
    }
    out
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
        let paragraphs: Vec<Block> = note
            .and_then(|n| document.footnotes.notes.get(&n))
            .map(|b| b.iter().filter(|b| matches!(b, Block::Paragraph(_))).cloned().collect())
            .unwrap_or_default();
        let skip = first.map_or(0, |l| l.paragraph);
        first_of.push(blocks.len() - skip);
        blocks.extend(paragraphs.into_iter().skip(skip));
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

/// Whether a page of columns has notes: a line referencing one, or a note going on.
fn column_notes(layout: &DocumentLayout, laid: &LaidPage) -> bool {
    !laid.carried.is_empty()
        || laid.lines.iter().any(|l| {
            layout
                .flow
                .items
                .get(l.item)
                .and_then(FlowItem::para)
                .is_some_and(|p| !p.notes.is_empty())
        })
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

/// The footnotes page `laid` draws at its foot, as a story stacked there; `None` where it
/// has none, or holds them in a way this does not draw (`Err` with why).
pub fn page_footnotes(
    job: &mut StoryJob<'_, '_>,
    layout: &DocumentLayout,
    frame: &PageFrame,
    laid: &LaidPage,
) -> Result<Option<StoryPage>, &'static str> {
    let document = job.cx.document;
    let section = layout.sections.get(frame.section).ok_or("no section")?;
    if layout.sections.any_multi_column() && column_notes(layout, laid) {
        // A page of columns keeps its notes at its columns' feet.
        return Err("footnotes in columns");
    }
    let parts = parts(laid);
    if parts.is_empty() {
        return Ok(None);
    }
    if section.props.footnotes.position.as_deref() == Some("beneathText") {
        return Err("footnotes beneath the text");
    }
    let mode15 = layout.mode15;
    let carried = !laid.carried.is_empty();
    let kind = if carried && !mode15 {
        "continuationSeparator"
    } else {
        "separator"
    };
    let (blocks, separators, first_of) = note_blocks(document, &parts, kind);
    let blocks: Arc<[Block]> = blocks.into();
    let sections = Sections::new(&[note_section(&section.props, blocks.len())], false);
    let fcx = FlowContext {
        document,
        blocks: &blocks,
        sections: &sections,
        labels: Labels::None,
        faces: job.cx.faces,
        locale: job.cx.locale,
        tables: job.cx.tables,
        roles: &[],
    };
    let flow = FlowMaker::new(&fcx, job.model, job.state).build();
    if flow.items.iter().any(|i| matches!(i, FlowItem::Stop(_))) {
        return Err("a footnote that cannot be laid out");
    }
    let items = &flow.items;
    let last = parts.last().and_then(|p| p.2);
    let end = match (last, first_of.last()) {
        (Some(l), Some(f)) => (f + l.paragraph, l.line),
        _ => (items.len(), 0),
    };
    let walks: Vec<Walk> = match laid.carried.first().filter(|l| l.line > 0) {
        Some(l) => vec![((0, 0), (separators, 0)), ((separators, l.line), end)],
        None => vec![((0, 0), end)],
    };
    let stack = |top: Px| {
        let from = WalkFrom {
            top,
            left: frame.left,
            mode15,
            bottom: None,
        };
        stacked(items, &walks, from)
    };
    let mut height = stack(Px::ZERO).end;
    if let (None, Some(FlowItem::Para(p))) = (last, items.last()) {
        height += Px::twips(p.after);
    }
    let mut walked = stack(frame.bottom - height);
    let before = match (carried, items.get(separators)) {
        (false, Some(FlowItem::Para(p))) => p.before,
        _ => 0,
    };
    if before != 0 {
        lower_separator(&mut walked, separators, before);
    }
    Ok(Some(StoryPage {
        kind: "footnotes",
        part: Arc::from(""),
        blocks,
        flow,
        walked,
    }))
}
