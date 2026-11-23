//! Owns: laying a new version's pages out from an old version's (docx2svg's `relayout`,
//! the prototype's `incr`): the change between two flows by their items' content
//! ([`diff`]), and pagination restarted at the first old page whose decision read the
//! first changed item, stopped where a new page starts where an old one did past the
//! change (pages after it moved, not laid out again) ([`repaginate`]).
//!
//! Must not: decide anything a full pagination would not: the result equals paginating
//! the new flow from its start (the property `tests/integration/relayout.rs` checks).

use crate::flow::FlowItem;
use crate::paginate::{LaidPage, LineStore, PageContext, Pagination, Paginator, Position, Resume};
use ooxml_word_model::model::Block;
use std::sync::Arc;

/// Items `[a - shift, b_old)` of the old flow are `[a, b_new)` of the new one, `shift`
/// being what the changes before it added (`b_new - b_old` of the one before); the rest
/// are the same. `a` and `b_new` index the new flow, `b_old` the old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// The first changed item.
    pub a: usize,
    /// One past the last changed item in the old flow.
    pub b_old: usize,
    /// One past the last changed item in the new flow.
    pub b_new: usize,
}

/// Whether two items lay out alike wherever they stand: the same lines (a shared measure,
/// or an equal one where the model read the block again) and the same spacing, borders, keeps and section facts. Their places in the body
/// (block indices) may differ.
pub fn same_item(old: &FlowItem, new: &FlowItem) -> bool {
    match (old, new) {
        (FlowItem::Para(a), FlowItem::Para(b)) => {
            (Arc::ptr_eq(&a.measured, &b.measured) || a.measured == b.measured)
                && a.section == b.section
                && a.before == b.before
                && a.after == b.after
                && a.contextual == b.contextual
                && a.style == b.style
                && a.border == b.border
                && a.top_border == b.top_border
                && a.keep_next == b.keep_next
                && a.keep_lines == b.keep_lines
                && a.page_break_before == b.page_break_before
                && a.widow == b.widow
                && a.section_start == b.section_start
                && a.section_mark == b.section_mark
                && a.notes == b.notes
                && a.role == b.role
                && a.continuation == b.continuation
                && a.anchors == b.anchors
                && same_source(a.source.as_deref(), b.source.as_deref())
                && a.drop_cap == b.drop_cap
                && a.note_forms == b.note_forms
        }
        (FlowItem::Table(a), FlowItem::Table(b)) => {
            Arc::ptr_eq(&a.flow, &b.flow)
                && a.section == b.section
                && a.section_start == b.section_start
                && a.notes == b.notes
        }
        (FlowItem::Stop(a), FlowItem::Stop(b)) => a.section == b.section && a.reason == b.reason,
        _ => false,
    }
}

/// Whether two items' sources break their lines alike (their paragraphs perhaps at other
/// places, [`ooxml_word_model::model::Paragraph::lays_out_as`]).
fn same_source(a: Option<&crate::rebreak::ParaSource>, b: Option<&crate::rebreak::ParaSource>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => std::ptr::eq(a, b) || a.lays_out_as(b),
        (None, None) => true,
        _ => false,
    }
}

/// The change between two flows: their common prefix and suffix.
pub fn diff(old: &[FlowItem], new: &[FlowItem]) -> Change {
    let mut a = 0;
    while let (Some(x), Some(y)) = (old.get(a), new.get(a)) {
        if !same_item(x, y) {
            break;
        }
        a += 1;
    }
    let (mut bo, mut bn) = (old.len(), new.len());
    while bo > a && bn > a {
        match (old.get(bo - 1), new.get(bn - 1)) {
            (Some(x), Some(y)) if same_item(x, y) => {
                bo -= 1;
                bn -= 1;
            }
            _ => break,
        }
    }
    Change {
        a,
        b_old: bo,
        b_new: bn,
    }
}

/// What a relayout did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelayoutStats {
    /// The change laid out again.
    pub change: Option<Change>,
    /// The first laid page laid out again.
    pub restart: usize,
    /// Pages laid out again.
    pub laid: usize,
    /// The old laid page the new pages lined up with again (`None`: ran to the end).
    pub joined: Option<usize>,
}

fn shift(p: &Position, by: isize) -> Position {
    Position {
        item: p.item.saturating_add_signed(by),
        line: p.line,
        part: p.part.clone(),
    }
}

/// An old page moved by the change: its items by `by.items` (its drawings' paragraphs
/// by `shift.blocks`), its place by `shift.pages`, its number by `shift.numbers`.
fn moved(page: &LaidPage, by: Shift) -> LaidPage {
    let Shift {
        items,
        blocks,
        pages,
        numbers,
    } = by;
    let mut out = page.clone();
    for band in &mut out.bands {
        band.key.block = band.key.block.saturating_add_signed(blocks);
    }
    for wrap in &mut out.wraps {
        wrap.drop_cap = wrap.drop_cap.map(|k| k.saturating_add_signed(items));
        wrap.table = wrap.table.map(|k| k.saturating_add_signed(items));
        if let Some((key, _)) = &mut wrap.drawing {
            key.block = key.block.saturating_add_signed(blocks);
        }
    }
    out.start = shift(&page.start, items);
    out.end = shift(&page.end, items);
    out.reach = shift(&page.reach, items);
    out.page = page.page.saturating_add_signed(pages);
    out.number_before += numbers;
    for line in &mut out.lines {
        line.item = line.item.saturating_add_signed(items);
    }
    for row in &mut out.rows {
        row.item = row.item.saturating_add_signed(items);
    }
    out.lines_before = shifted(&page.lines_before, items);
    out.broken = shifted(&page.broken, items);
    out
}

/// Where old page `q`'s own blank pages (if any) start among the old pages.
fn base_of(old: &Pagination, q: usize) -> usize {
    q.checked_sub(1)
        .and_then(|k| old.pages.get(k))
        .map_or(0, |b| b.page + 1 + usize::from(b.continued.is_some()))
}

/// Where a page's number or place has a parity that decides something -- blank pages
/// before an odd or even section, even pages' stories: the pages after the change are
/// then moved only by an even number of pages and of page numbers.
fn parity_decides(cx: &PageContext<'_>) -> bool {
    cx.sections.even_and_odd
        || cx
            .sections
            .list
            .iter()
            .any(|s| matches!(s.props.start_type.as_deref(), Some("oddPage" | "evenPage")))
}

/// What decides whether old pages may move to other places: a parity (blank pages, even
/// pages' stories), or page stories of their own (headers and footers of their own heights,
/// by their pages' places).
#[derive(Debug, Clone, Copy)]
struct Placed {
    parity: bool,
    fixed: bool,
}

/// The old page a new page starting at `state` lines up with, past the change.
fn joins(old: &Pagination, change: Change, delta: isize, placed: Placed, state: &Resume) -> Option<usize> {
    // A page starting at a paragraph's first line reads the item before it (its space
    // after, at the page's top): that item must be past the change too.
    let reads_before = state.position.line == 0 && state.position.part.is_empty();
    if state.position.item < change.b_new + usize::from(reads_before) {
        return None;
    }
    let old_position = shift(&state.position, -delta);
    let q = old.pages.binary_search_by(|page| page.start.cmp(&old_position)).ok()?;
    let page = old.pages.get(q)?;
    if page.carried != state.carry || shifted(&page.lines_before, delta) != state.lines {
        // The page's foot opens with other lines of a note, or its paragraphs are broken
        // otherwise.
        return None;
    }
    let page_shift = i64::try_from(state.page).unwrap_or(0) - i64::try_from(base_of(old, q)).unwrap_or(0);
    let number_shift = state.number - page.number_before;
    let odd = page_shift.rem_euclid(2) != 0 || number_shift.rem_euclid(2) != 0;
    let refused = (placed.parity && odd) || (placed.fixed && page_shift != 0);
    (!refused).then_some(q)
}

/// The changes between two flows: where they have as many items, each run of items that
/// differ in place; otherwise one change, between their common prefix and suffix.
pub fn changes(old: &[FlowItem], new: &[FlowItem]) -> Vec<Change> {
    if old.len() != new.len() {
        let one = diff(old, new);
        return if one.a == one.b_old && one.a == one.b_new {
            Vec::new()
        } else {
            vec![one]
        };
    }
    let mut out: Vec<Change> = Vec::new();
    for (k, (x, y)) in old.iter().zip(new).enumerate() {
        if same_item(x, y) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.b_new == k => {
                last.b_old = k + 1;
                last.b_new = k + 1;
            }
            _ => out.push(Change {
                a: k,
                b_old: k + 1,
                b_new: k + 1,
            }),
        }
    }
    out
}

/// How the old pages are moved to follow the new ones: by items, blocks, pages and
/// numbers.
#[derive(Debug, Clone, Copy)]
struct Shift {
    items: isize,
    /// How far the paragraphs' blocks (document order, cells included) moved: a
    /// drawing's key names its paragraph by its block.
    blocks: isize,
    pages: isize,
    numbers: i64,
}

/// The first old laid page whose decision read `target` (the last where none did).
fn first_reaching(old: &Pagination, from: usize, target: &Position) -> usize {
    let k = from
        + old
            .pages
            .get(from..)
            .unwrap_or_default()
            .partition_point(|page| page.reach < *target);
    k.min(old.pages.len().saturating_sub(1))
}

/// Old laid pages `from..to` (and their blank pages' info) moved by `shift`.
fn copy_moved(out: &mut Pagination, old: &Pagination, from: usize, to: usize, shift: Shift) {
    let to = to.min(old.pages.len());
    out.pages
        .extend(old.pages.get(from..to).unwrap_or_default().iter().map(|page| {
            if shift.items == 0 && shift.blocks == 0 && shift.pages == 0 && shift.numbers == 0 {
                // A page that stays where it was is the old one.
                Arc::clone(page)
            } else {
                Arc::new(moved(page, shift))
            }
        }));
    let info_to = if to < old.pages.len() {
        base_of(old, to)
    } else {
        old.info.len()
    };
    out.info.extend(
        old.info
            .get(base_of(old, from)..info_to)
            .unwrap_or_default()
            .iter()
            .map(|info| {
                let mut i = *info;
                i.number += shift.numbers;
                i
            }),
    );
}

/// The new flow's pages from the old flow's, given the changes between them: the pages
/// before the first change kept; from the first page whose decision read a change, pages
/// laid out again until one starts where an old one did past that change; the old pages
/// from there moved until the next change is read, and so on.
pub fn repaginate(
    cx: &PageContext<'_>,
    (old, old_items): (&Pagination, &[FlowItem]),
    changes: &[Change],
) -> (Pagination, RelayoutStats) {
    let Some(first) = changes.first() else {
        return (old.clone(), RelayoutStats::default());
    };
    let placed = Placed {
        parity: parity_decides(cx),
        // A page whose header or footer has heights of its own keeps them by its place.
        fixed: !cx.page_stories.is_empty(),
    };
    let mut p = clean_restart(old, first_reaching(old, 0, &Position::at(first.a, 0)), first.a);
    let mut out = Pagination {
        pages: old.pages.get(..p).unwrap_or_default().to_vec(),
        info: old.info.get(..base_of(old, p)).unwrap_or_default().to_vec(),
        stopped: None,
    };
    let mut shift = Shift {
        items: 0,
        blocks: 0,
        pages: 0,
        numbers: 0,
    };
    let mut stats = RelayoutStats {
        change: Some(*first),
        restart: p,
        ..RelayoutStats::default()
    };
    let mut i = 0;
    loop {
        let resume = old.pages.get(p).map(|page| Resume {
            position: shift_position(&page.start, shift.items),
            page: base_of(old, p).saturating_add_signed(shift.pages),
            number: page.number_before + shift.numbers,
            carry: std::sync::Arc::clone(&page.carried),
            lines: shifted(&page.lines_before, shift.items),
        });
        let change = changes.get(i).copied().unwrap_or(*first);
        // What every change up to this one added: `b_new` indexes the new flow, `b_old` the
        // old one.
        let items = isize::try_from(change.b_new).unwrap_or(0) - isize::try_from(change.b_old).unwrap_or(0);
        let mut joined = None;
        let mut until = |state: &Resume| {
            joined = joins(old, change, items, placed, state);
            joined.is_some()
        };
        let (fresh, stop) = Paginator::new(cx).run(resume, &mut until);
        stats.laid += fresh.pages.len();
        out.pages.extend(fresh.pages);
        out.info.extend(fresh.info);
        out.stopped = fresh.stopped;
        let (Some(q), Some(state)) = (joined, stop) else { break };
        stats.joined = Some(q);
        let start = old.pages.get(q).map_or(0, |page| page.start.item);
        shift = Shift {
            items,
            blocks: block_shift((old_items, cx.items), start, items),
            pages: isize::try_from(state.page).unwrap_or(0) - isize::try_from(base_of(old, q)).unwrap_or(0),
            numbers: state.number - old.pages.get(q).map_or(0, |page| page.number_before),
        };
        while changes.get(i).is_some_and(|c| c.b_new <= state.position.item) {
            i += 1;
        }
        match changes.get(i) {
            Some(c) => {
                // The next change, where the old pages' items stand.
                let target = shift_position(&Position::at(c.a, 0), -shift.items);
                let n = clean_restart(old, first_reaching(old, q, &target), target.item).max(q);
                copy_moved(&mut out, old, q, n, shift);
                p = n;
            }
            None => {
                copy_moved(&mut out, old, q, old.pages.len(), shift);
                out.stopped = old.stopped.clone().map(|mut s| {
                    s.item = s.item.saturating_add_signed(shift.items);
                    s
                });
                break;
            }
        }
    }
    (out, stats)
}

/// How far old item `start`'s paragraph blocks moved, it being item `start + items` now.
fn block_shift((old, new): (&[FlowItem], &[FlowItem]), start: usize, items: isize) -> isize {
    let block = |list: &[FlowItem], k: usize| list.get(k).map(|i| isize::try_from(i.block()).unwrap_or(0));
    match (block(new, start.saturating_add_signed(items)), block(old, start)) {
        (Some(now), Some(was)) => now - was,
        _ => 0,
    }
}

/// The page a relayout restarts at, at or before `p`: one whose state holds no paragraph
/// broken again at or past the change at item `a` (the old flow's lines of it).
fn clean_restart(old: &Pagination, mut p: usize, a: usize) -> usize {
    while p > 0
        && old
            .pages
            .get(p)
            .is_some_and(|page| page.lines_before.range(a..).next().is_some())
    {
        p -= 1;
    }
    p
}

/// A line store with its items moved by `by`.
fn shifted(lines: &LineStore, by: isize) -> LineStore {
    lines
        .iter()
        .map(|(k, v)| (k.saturating_add_signed(by), std::sync::Arc::clone(v)))
        .collect()
}

fn shift_position(p: &Position, by: isize) -> Position {
    shift(p, by)
}

fn same_block(x: &Block, y: &Block) -> bool {
    match (x, y) {
        (Block::Paragraph(p), Block::Paragraph(q)) => Arc::ptr_eq(p, q),
        (Block::Table(t), Block::Table(u)) => Arc::ptr_eq(t, u),
        _ => false,
    }
}

/// The top-level blocks of two versions' bodies that differ by `Arc` (the model shares an
/// unchanged block's): where both have as many, each run of them in place; otherwise one
/// change between the common prefix and suffix. `a` and `b_new` index the new body, `b_old`
/// the old one.
pub fn block_changes(old: &[Block], new: &[Block]) -> Vec<Change> {
    if old.len() == new.len() {
        let mut out: Vec<Change> = Vec::new();
        for (k, (x, y)) in old.iter().zip(new).enumerate() {
            if same_block(x, y) {
                continue;
            }
            match out.last_mut() {
                Some(c) if c.b_new == k => {
                    c.b_old = k + 1;
                    c.b_new = k + 1;
                }
                _ => out.push(Change {
                    a: k,
                    b_old: k + 1,
                    b_new: k + 1,
                }),
            }
        }
        return out;
    }
    let prefix = old.iter().zip(new).take_while(|(x, y)| same_block(x, y)).count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(x, y)| same_block(x, y))
        .count();
    vec![Change {
        a: prefix,
        b_old: old.len() - suffix,
        b_new: new.len() - suffix,
    }]
}
