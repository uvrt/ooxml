//! Owns: `spacers` (docx-agent's `spacers`, round 16 F8): top-level empty paragraphs, a run
//! of them, used as vertical space -- how far the next block would move up without them,
//! the `word_delete` target that removes them, and the space before that gives the next
//! paragraph that space instead. One empty paragraph between two tables (it keeps them
//! apart) or ending the document is not one.
//!
//! Between two paragraphs the space before and the space after collapse to the larger
//! (docx2svg's measured rule), so the gap a run makes is measured from the previous block's
//! text to the next block's.
//!
//! Must not: count a paragraph that draws something or breaks (`_CONTENT`), or one not
//! drawn.

use super::tree::has_content;
use super::{points, round, Block, BlockKind, Facts};
use crate::page_parts::Part;
use crate::page_read::PageRead;
use ooxml_word_model::ids::Anchor;
use std::collections::HashMap;

/// The most space before `word_format` takes, points.
const MOST_SPACE: f64 = 1584.0;

/// A run of empty paragraphs used as space.
#[derive(Debug, Clone, PartialEq)]
pub struct Spacer {
    /// Their ids.
    pub paras: Vec<String>,
    /// The 1-based page the first is drawn on.
    pub page: usize,
    /// How far the next block would move up without them, points.
    pub height_pt: f64,
    /// The block after them, where there is one.
    pub next: Option<String>,
    /// What `word_delete` takes to remove them: an id, or a span of ids.
    pub target: String,
    /// The space before that keeps the next paragraph where it is without them, points,
    /// where it is on their page and a paragraph.
    pub keep_space: Option<f64>,
    /// They reach the foot of their page (the next block is on a later page).
    pub page_end: bool,
}

/// `spacers`: tallest first.
#[must_use]
pub fn spacers(facts: &Facts<'_>) -> Vec<Spacer> {
    let blocks = facts.blocks();
    let empty: Vec<bool> = blocks.iter().map(|block| is_empty(facts, block)).collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < blocks.len() {
        if !empty.get(k).copied().unwrap_or(false) {
            k += 1;
            continue;
        }
        let start = k;
        while empty.get(k).copied().unwrap_or(false) {
            k += 1;
        }
        let before = start.checked_sub(1).and_then(|b| blocks.get(b));
        let after = blocks.get(k);
        let mut run = blocks.get(start..k).unwrap_or_default();
        match after {
            None => run = run.get(..run.len().saturating_sub(1)).unwrap_or_default(),
            Some(after) if run.len() == 1 && before.is_some_and(table) && table(after) => continue,
            Some(_) => {}
        }
        if run.is_empty() {
            continue;
        }
        if let Some(found) = spacer(facts, run, before, after).filter(|s| facts.shown([s.page])) {
            out.push(found);
        }
    }
    out.sort_by(|a, b| b.height_pt.total_cmp(&a.height_pt));
    out
}

fn table(block: &Block<'_>) -> bool {
    block.kind == BlockKind::Table
}

/// One page's contribution to `spacers` (the page tier's, [`crate::page_entry`]): its body
/// lines' characters that are not white space, by paragraph (its anchor) and line number
/// (a line drawn twice counts as drawn last).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InkedPart(HashMap<Anchor, Vec<(usize, usize)>>);

impl InkedPart {
    /// The characters that are not white space of the body line `number` of the paragraph
    /// `anchor`, where the page draws it.
    fn of_line(&self, anchor: Anchor, number: usize) -> Option<usize> {
        let lines = self.0.get(&anchor)?;
        lines.iter().rev().find(|(n, _)| *n == number).map(|(_, inked)| *inked)
    }
}

impl Part<PageRead> for InkedPart {
    fn of(read: &PageRead) -> InkedPart {
        let mut out: HashMap<Anchor, Vec<(usize, usize)>> = HashMap::new();
        for line in read.texts.iter().filter(|l| l.story.is_none()) {
            if let Some(anchor) = line.place.anchor() {
                out.entry(anchor).or_default().push((line.number, line.inked));
            }
        }
        InkedPart(out)
    }

    fn weight_of(read: &PageRead) -> usize {
        let each = std::mem::size_of::<(Anchor, Vec<(usize, usize)>)>() + 2 * std::mem::size_of::<(usize, usize)>();
        let lines = read.texts.iter().filter(|l| l.story.is_none()).count();
        std::mem::size_of::<InkedPart>() + lines * each
    }
}

/// Whether a top-level block is an empty paragraph drawn (`_empty`).
fn is_empty(facts: &Facts<'_>, block: &Block<'_>) -> bool {
    if block.kind != BlockKind::Paragraph || has_content(block.element) {
        return false;
    }
    let lines = facts.lines_of(&block.path);
    if lines.is_empty() {
        return false;
    }
    !lines.iter().any(|d| {
        facts
            .entry(d.page - 1)
            .and_then(|e| e.inked().of_line(d.line.anchor?, d.line.number))
            .is_some_and(|n| n > 0)
    })
}

/// A top-level paragraph's space before or after, points (0 for a table).
fn space(facts: &Facts<'_>, block: &Block<'_>, before: bool) -> f64 {
    if block.kind != BlockKind::Paragraph {
        return 0.0;
    }
    let effective = facts.paragraph(block.element);
    if before {
        effective.space_before
    } else {
        effective.space_after
    }
}

/// One run of spacers (`_spacer`).
fn spacer(
    facts: &Facts<'_>,
    run: &[Block<'_>],
    before: Option<&Block<'_>>,
    after: Option<&Block<'_>>,
) -> Option<Spacer> {
    let (first, last) = (run.first()?, run.last()?);
    let (page, top) = facts.first_line(first)?;
    let paras: Vec<String> = run.iter().map(|b| b.id().to_owned()).collect();
    let target = if run.len() == 1 {
        first.id().to_owned()
    } else {
        format!("{}..{}", first.id(), last.id())
    };
    let previous_after = before.map_or(0.0, |b| space(facts, b, false));
    let above = previous_after.max(space(facts, first, true));
    let following = after.and_then(|a| facts.first_line(a));
    let mut out = Spacer {
        paras,
        page,
        height_pt: 0.0,
        next: after.map(|a| a.id().to_owned()),
        target,
        keep_space: None,
        page_end: false,
    };
    match (following, after) {
        (Some((next_page, next_top)), Some(after)) if next_page == page => {
            let gap = round(above + points(next_top - top), 1);
            out.height_pt = round(gap - previous_after.max(space(facts, after, true)), 1);
            if after.kind == BlockKind::Paragraph && gap <= MOST_SPACE {
                out.keep_space = Some(gap);
            }
        }
        _ => {
            let bottom = run
                .iter()
                .flat_map(|b| facts.lines_of(&b.path))
                .filter(|d| d.page == page)
                .map(|d| d.line.top + d.line.pitch)
                .max()?;
            out.height_pt = round(above + points(bottom - top), 1);
            out.page_end = true;
        }
    }
    Some(out)
}
