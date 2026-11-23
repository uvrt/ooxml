//! Owns: a version's flow made again from the same pass's flow of the version before
//! (section 7: per-block facts under their context): the top-level blocks that changed --
//! where the two bodies differ ([`super::align`]: a paragraph moved to another place is the
//! same), or by list label or place in the endnotes -- each widened to the blocks whose
//! items read it (the blocks beside it, a drop cap and the paragraph that carries it); only
//! those made again, every other item taken from the old flow, moved where blocks were
//! added or removed (its paragraph the one at its new place); and the changes between the
//! two flows the paginator restarts from ([`FlowMaker::again`]).
//!
//! Must not: make an item differently than [`FlowMaker::build`] makes it: the result is
//! the flow built from scratch (the relayout property in `tests/integration/relayout.rs` checks the
//! pages laid out from it).

use super::align::{aligned, Window};
use super::{source, FlowItem, FlowMaker, ParaItem, StopItem, TableItem};
use crate::incremental::{same_item, Change};
use crate::notes::NoteRole;
use crate::tables::table_paragraphs;
use ooxml_word_model::model::Block;
use std::ops::Range;
use std::sync::Arc;

/// The same pass's flow of the version before, and what it was made from.
#[derive(Debug, Clone, Copy)]
pub struct OldFlow<'o> {
    /// The flow.
    pub flow: &'o super::Flow,
    /// Its top-level blocks.
    pub body: &'o [Block],
    /// Its paragraphs' list labels, document order (cells included).
    pub labels: &'o [Option<String>],
    /// Per top-level block, what of the endnotes placed in the flow it is.
    pub roles: &'o [Option<NoteRole>],
}

fn paragraphs_of(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(_) => 1,
            Block::Table(t) => table_paragraphs(t),
        })
        .sum()
}

/// Moves an old item to its place in the new flow.
#[derive(Debug, Clone, Copy, Default)]
struct Shift {
    body: isize,
    block: isize,
}

impl Shift {
    fn moved(self, item: &FlowItem) -> FlowItem {
        if self.body == 0 && self.block == 0 {
            return item.clone();
        }
        let body = |k: usize| k.saturating_add_signed(self.body);
        let block = |k: usize| k.saturating_add_signed(self.block);
        match item {
            FlowItem::Para(p) => {
                let drop_cap = p.drop_cap.as_ref().map(|d| {
                    Arc::new(super::DropCap {
                        body: body(d.body),
                        ..(**d).clone()
                    })
                });
                FlowItem::Para(Arc::new(ParaItem {
                    body: body(p.body),
                    block: block(p.block),
                    drop_cap,
                    ..(**p).clone()
                }))
            }
            FlowItem::Table(t) => FlowItem::Table(Arc::new(TableItem {
                body: body(t.body),
                block: block(t.block),
                ..(**t).clone()
            })),
            FlowItem::Stop(s) => FlowItem::Stop(Arc::new(StopItem {
                body: body(s.body),
                block: block(s.block),
                ..(**s).clone()
            })),
        }
    }
}

/// The change between a window's old items `before` (ending at old item `old_end`) and
/// its new ones `made` (starting at new item `start`): their common prefix and suffix
/// left out; `None` where they are alike.
fn window_change(before: &[FlowItem], made: &[FlowItem], start: usize, old_end: usize) -> Option<Change> {
    let prefix = before.iter().zip(made).take_while(|(x, y)| same_item(x, y)).count();
    let room = before.len().min(made.len()) - prefix;
    let suffix = before
        .iter()
        .rev()
        .zip(made.iter().rev())
        .take(room)
        .take_while(|(x, y)| same_item(x, y))
        .count();
    (prefix < before.len().max(made.len()) - suffix).then(|| Change {
        a: start + prefix,
        b_old: old_end - suffix,
        b_new: start + made.len() - suffix,
    })
}

/// A kept item whose paragraph the model moved to another place (`body`'s block there is
/// the same paragraph at its new path), given that paragraph: its lines are broken from it.
fn at_place(item: FlowItem, body: &[Block]) -> FlowItem {
    let FlowItem::Para(p) = &item else { return item };
    let (Some(source), Some(Block::Paragraph(now))) = (&p.source, body.get(p.body)) else {
        return item;
    };
    if Arc::ptr_eq(&source.paragraph, now) {
        return item;
    }
    let source = crate::rebreak::ParaSource {
        paragraph: Arc::clone(now),
        ..(**source).clone()
    };
    FlowItem::Para(Arc::new(ParaItem {
        source: Some(Arc::new(source)),
        ..(**p).clone()
    }))
}

/// The body index of an item's first block.
fn body_of(item: &FlowItem) -> usize {
    match item {
        FlowItem::Para(p) => p.drop_cap.as_ref().map_or(p.body, |d| d.body),
        FlowItem::Table(t) => t.body,
        FlowItem::Stop(s) => s.body,
    }
}

impl FlowMaker<'_, '_> {
    /// Whether top-level block `body` is a drop cap the flow lays out.
    fn is_drop_cap(&mut self, body: usize) -> bool {
        !self.drop_caps(body..body + 1).is_empty()
    }

    /// The windows widened to the blocks whose items read a changed one, and those whose
    /// list labels or place in the endnotes changed added; merged where they meet.
    fn widened(&mut self, old: &OldFlow<'_>, found: Vec<Window>) -> Vec<Window> {
        let new_len = self.cx.blocks.len();
        let mut windows = found;
        // Blocks outside the windows are the same blocks; their labels or roles may not be.
        let mut extra = Vec::new();
        let (mut shift, mut pshift) = (0isize, 0isize);
        let mut at = 0;
        for w in windows.iter().copied().chain(std::iter::once(Window {
            a: new_len,
            b_old: old.body.len(),
            b_new: new_len,
        })) {
            for k in at..w.a {
                let old_k = k.saturating_add_signed(-shift);
                let first = self.firsts.get(k).copied().unwrap_or(0);
                let next = self.firsts.get(k + 1).copied().unwrap_or(self.paragraphs.len());
                let labels_differ = (first..next).any(|p| {
                    let o = p.saturating_add_signed(-pshift);
                    self.cx.labels.get(p) != old.labels.get(o).and_then(Option::as_deref)
                });
                let role_differs = self.cx.roles.get(k) != old.roles.get(old_k);
                if labels_differ || role_differs {
                    extra.push(Window {
                        a: k,
                        b_old: old_k + 1,
                        b_new: k + 1,
                    });
                }
            }
            if w.a >= new_len {
                break;
            }
            let new_paragraphs = self.firsts.get(w.b_new).copied().unwrap_or(self.paragraphs.len())
                - self.firsts.get(w.a).copied().unwrap_or(self.paragraphs.len());
            let old_start = w.a.saturating_add_signed(-shift);
            let old_paragraphs = paragraphs_of(old.body.get(old_start..w.b_old).unwrap_or_default());
            // What the windows up to this one added: `b_new` indexes the new body, `b_old` the old.
            shift = isize::try_from(w.b_new).unwrap_or(0) - isize::try_from(w.b_old).unwrap_or(0);
            pshift += isize::try_from(new_paragraphs).unwrap_or(0) - isize::try_from(old_paragraphs).unwrap_or(0);
            at = w.b_new;
        }
        windows.extend(extra);
        windows.sort_by_key(|w| w.a);
        // Each widened to the blocks beside it, and past a drop cap at either end.
        let mut out: Vec<Window> = Vec::new();
        for w in windows {
            let (mut a, mut b_old, mut b_new) = (w.a, w.b_old, w.b_new);
            let grow_a = usize::from(a > 0);
            a -= grow_a;
            while a > 0 && self.is_drop_cap(a - 1) {
                a -= 1;
            }
            let grow_b = usize::from(b_new < new_len && b_old < old.body.len());
            b_old += grow_b;
            b_new += grow_b;
            while b_new > 0 && b_new < new_len && b_old < old.body.len() && self.is_drop_cap(b_new - 1) {
                b_old += 1;
                b_new += 1;
            }
            match out.last_mut() {
                Some(last) if a <= last.b_new => {
                    if b_new >= last.b_new {
                        last.b_new = b_new;
                        last.b_old = b_old;
                    }
                }
                _ => out.push(Window { a, b_old, b_new }),
            }
        }
        out
    }

    /// The flow made again from `old`, the same pass's flow of the version before, and the
    /// changes between the two; `None` where it must be built whole (the body's drawings
    /// that text wraps beside, or its drop caps, changed whether there are any).
    pub fn again(mut self, old: &OldFlow<'_>) -> Option<(super::Flow, Vec<Change>)> {
        let windows = self.widened(old, aligned(old.body, self.cx.blocks));
        // Each window's old items and its drop caps.
        let mut drops: Vec<Vec<usize>> = Vec::with_capacity(windows.len());
        let mut old_ranges: Vec<Range<usize>> = Vec::with_capacity(windows.len());
        let mut shift = 0isize;
        for w in &windows {
            drops.push(self.drop_caps(w.a..w.b_new));
            let old_a = w.a.saturating_add_signed(-shift);
            let items = &old.flow.items;
            let from = items.partition_point(|i| body_of(i) < old_a);
            let to = items.partition_point(|i| body_of(i) < w.b_old);
            old_ranges.push(from..to);
            // What the windows up to this one added: `b_new` indexes the new body, `b_old` the old.
            shift = isize::try_from(w.b_new).unwrap_or(0) - isize::try_from(w.b_old).unwrap_or(0);
        }
        let drops_outside = old.flow.items.iter().enumerate().any(|(k, item)| {
            item.para().is_some_and(|p| p.drop_cap.is_some()) && !old_ranges.iter().any(|r| r.contains(&k))
        });
        let wraps = source::wraps(self.cx.blocks) || drops_outside || drops.iter().any(|d| !d.is_empty());
        if wraps != old.flow.wraps {
            return None;
        }
        self.wraps = wraps;
        self.separators = old.flow.separators.clone();
        self.note_items = old.flow.note_items.clone();
        self.notes_at = old.flow.notes_at.clone();
        let mut items: Vec<FlowItem> = Vec::with_capacity(old.flow.items.len());
        let mut changes: Vec<Change> = Vec::new();
        let mut shift = Shift::default();
        let mut next_old = 0;
        for ((w, range), window_drops) in windows.iter().zip(&old_ranges).zip(&drops) {
            let kept = old.flow.items.get(next_old..range.start).unwrap_or_default();
            items.extend(kept.iter().map(|i| at_place(shift.moved(i), self.cx.blocks)));
            let made = self.items_in(w.a..w.b_new, window_drops);
            let before = old.flow.items.get(range.clone()).unwrap_or_default();
            changes.extend(window_change(before, &made, items.len(), range.end));
            let old_paragraphs = paragraphs_of(
                old.body
                    .get(w.a.saturating_add_signed(-shift.body)..w.b_old)
                    .unwrap_or_default(),
            );
            let new_paragraphs = paragraphs_of(self.cx.blocks.get(w.a..w.b_new).unwrap_or_default());
            items.extend(made);
            shift.body = isize::try_from(w.b_new).unwrap_or(0) - isize::try_from(w.b_old).unwrap_or(0);
            shift.block += isize::try_from(new_paragraphs).unwrap_or(0) - isize::try_from(old_paragraphs).unwrap_or(0);
            next_old = range.end;
        }
        let rest = old.flow.items.get(next_old..).unwrap_or_default();
        items.extend(rest.iter().map(|i| at_place(shift.moved(i), self.cx.blocks)));
        let flow = super::Flow {
            items,
            separators: self.separators,
            note_items: self.note_items,
            notes_at: self.notes_at,
            wraps: self.wraps,
        };
        Some((flow, changes))
    }
}
