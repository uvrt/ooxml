//! Owns: what a version laid out, its facts and a page painted hold in bytes
//! ([`layout_weight`], [`facts_weight`], [`paint_weight`]): a measure for a memory budget, not an allocator's count. A version's
//! weight is what it alone holds -- what dropping it would give back now: its own
//! containers, and each shared part (a measured paragraph, a page, a block) only where no
//! other holder is left (the engine's memos, the versions laid out from it or before it).
//! So a version kept while the engine moves on weighs more as its parts stop being
//! shared, and a version just laid out from the one before weighs what it changed.
//!
//! Must not: walk anything twice per call, or allocate in proportion to what it weighs.

use ooxml_word_layout::facts::{BlockKeeps, LayoutFacts, LineFact, PageFacts};
use ooxml_word_layout::flow::{Flow, FlowItem};
use ooxml_word_layout::paginate::{LaidPage, Pagination};
use ooxml_word_layout::paint::PagePaint;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::model::Block;
use std::mem::size_of;
use std::sync::Arc;

/// What a block of the model takes, roughly (its paragraph or table with its runs): the
/// figure the model's memo weighs a block at.
const BLOCK: usize = 1024;
/// What a table's measure takes per row, roughly (its cells' measured paragraphs).
const TABLE_ROW: usize = 4096;

/// Whether `value` is held by its holder alone.
fn alone<T: ?Sized>(value: &Arc<T>) -> bool {
    Arc::strong_count(value) == 1
}

/// What `layout` alone holds, bytes, roughly.
pub fn layout_weight(layout: &DocumentLayout) -> usize {
    let passes: usize = layout
        .earlier
        .iter()
        .map(|p| flow_weight(&p.flow) + pagination_weight(&p.pagination) + blocks_weight(&p.body))
        .sum();
    let lists = if alone(&layout.labels) {
        layout
            .labels
            .iter()
            .map(|l| 24 + l.as_ref().map_or(0, String::len))
            .sum()
    } else {
        0
    };
    let roles = if alone(&layout.roles) {
        layout.roles.len() * 16
    } else {
        0
    };
    flow_weight(&layout.flow)
        + pagination_weight(&layout.pagination)
        + blocks_weight(&layout.document.body)
        + passes
        + lists
        + roles
}

fn flow_weight(flow: &Flow) -> usize {
    let own = flow.items.capacity() * size_of::<FlowItem>();
    let items: usize = flow
        .items
        .iter()
        .map(|item| match item {
            FlowItem::Para(p) if alone(p) => {
                let measured = if alone(&p.measured) {
                    let m = &p.measured;
                    let text = if alone(&m.measured) {
                        m.measured.pieces.capacity() * size_of::<ooxml_text::Piece>()
                            + m.measured.lines.capacity() * size_of::<ooxml_text::Line>()
                    } else {
                        0
                    };
                    size_of::<ooxml_word_layout::paragraph::MeasuredParagraph>()
                        + m.heights.capacity() * size_of::<ooxml_word_layout::heights::LineHeight>()
                        + text
                } else {
                    0
                };
                size_of::<ooxml_word_layout::flow::ParaItem>() + measured
            }
            FlowItem::Table(t) if alone(t) => {
                size_of::<ooxml_word_layout::flow::TableItem>()
                    + if alone(&t.flow) { t.flow.rows() * TABLE_ROW } else { 0 }
            }
            FlowItem::Stop(s) if alone(s) => size_of::<ooxml_word_layout::flow::StopItem>(),
            _ => 0,
        })
        .sum();
    own + items
}

fn pagination_weight(pagination: &Pagination) -> usize {
    let own = pagination.pages.capacity() * size_of::<Arc<LaidPage>>()
        + pagination.info.capacity() * size_of::<ooxml_word_layout::paginate::PageInfo>();
    let pages: usize = pagination
        .pages
        .iter()
        .filter(|page| alone(page))
        .map(|page| page_weight(page))
        .sum();
    own + pages
}

fn page_weight(page: &LaidPage) -> usize {
    size_of::<LaidPage>()
        + page.lines.capacity() * size_of::<ooxml_word_layout::paginate::PlacedLine>()
        + page.rows.capacity() * size_of::<ooxml_word_layout::paginate::PlacedTableRow>()
        + page.bands.capacity() * size_of::<ooxml_word_layout::paginate::Band>()
}

fn blocks_weight(body: &Arc<[Block]>) -> usize {
    if !alone(body) {
        return 0;
    }
    let blocks: usize = body
        .iter()
        .filter(|block| match block {
            Block::Paragraph(p) => alone(p),
            Block::Table(t) => alone(t),
        })
        .count();
    body.len() * size_of::<Block>() + blocks * BLOCK
}

/// What a layout's facts hold, bytes, roughly: each page's lines and the keys of the
/// blocks it draws (each key its own string, and its anchor), and each block's keeps.
pub fn facts_weight(facts: &LayoutFacts) -> usize {
    let pages: usize = facts
        .pages
        .iter()
        .map(|p| {
            size_of::<PageFacts>()
                + p.lines.capacity() * size_of::<LineFact>()
                + p.blocks.capacity() * size_of::<ooxml_word_model::ids::Anchor>()
        })
        .sum();
    pages + facts.keeps.len() * (size_of::<usize>() + size_of::<BlockKeeps>() + 32)
}

/// What a page's paint holds, bytes, roughly: its lines with their spans' characters and
/// positions, its rules, pictures and drawings.
pub fn paint_weight(paint: &PagePaint) -> usize {
    let spans: usize = paint
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| {
            size_of::<ooxml_word_layout::paint::Span>()
                + span.chars.capacity() * size_of::<char>()
                + span.xs.capacity() * size_of::<ooxml_word_layout::CompactPx>()
                + span.path.len()
                + span.face.len()
        })
        .sum();
    let lines: usize = paint
        .lines
        .iter()
        .map(|line| size_of::<ooxml_word_layout::paint::PaintedLine>() + line.path.len())
        .sum();
    size_of::<PagePaint>()
        + spans
        + lines
        + paint.rules.capacity() * size_of::<ooxml_word_layout::paint::Rule>()
        + paint.pictures.capacity() * size_of::<ooxml_word_layout::paint::Drawn>()
        + paint.drawings.capacity() * size_of::<ooxml_word_layout::paint::drawings::DrawingSlot>()
}
