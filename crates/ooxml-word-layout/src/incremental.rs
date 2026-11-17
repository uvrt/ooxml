//! Owns: laying a new version's pages out from an old version's (docx2svg's `relayout`,
//! the prototype's `incr`): the change between two flows by their items' content
//! ([`diff`]), and pagination restarted at the first old page whose decision read the
//! first changed item, stopped where a new page starts where an old one did past the
//! change (pages after it moved, not laid out again) ([`repaginate`]).
//!
//! Must not: decide anything a full pagination would not: the result equals paginating
//! the new flow from its start (the property `tests/relayout.rs` checks).

use crate::flow::FlowItem;
use crate::paginate::{LaidPage, PageContext, Pagination, Paginator, Position, Resume};
use std::sync::Arc;

/// Items `[a, b_old)` of the old flow are `[a, b_new)` of the new one; the rest are the
/// same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// The first changed item.
    pub a: usize,
    /// One past the last changed item in the old flow.
    pub b_old: usize,
    /// One past the last changed item in the new flow.
    pub b_new: usize,
}

/// Whether two items lay out alike wherever they stand: the same lines (shared measure)
/// and the same spacing, borders, keeps and section facts. Their places in the body
/// (block indices) may differ.
pub fn same_item(old: &FlowItem, new: &FlowItem) -> bool {
    match (old, new) {
        (FlowItem::Para(a), FlowItem::Para(b)) => {
            Arc::ptr_eq(&a.measured, &b.measured)
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
        }
        (FlowItem::Table(a), FlowItem::Table(b)) => {
            Arc::ptr_eq(&a.flow, &b.flow) && a.section == b.section && a.section_start == b.section_start
        }
        (FlowItem::Stop(a), FlowItem::Stop(b)) => a.section == b.section && a.reason == b.reason,
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

/// An old page moved by the change: its items by `items`, its place by `pages`, its
/// number by `numbers`.
fn moved(page: &LaidPage, items: isize, pages: isize, numbers: i64) -> LaidPage {
    let mut out = page.clone();
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
    out
}

/// Where old page `q`'s own blank pages (if any) start among the old pages.
fn base_of(old: &Pagination, q: usize) -> usize {
    q.checked_sub(1)
        .and_then(|k| old.pages.get(k))
        .map_or(0, |b| b.page + 1)
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

/// The old page a new page starting at `state` lines up with, past the change.
fn joins(old: &Pagination, change: Change, delta: isize, parity: bool, state: &Resume) -> Option<usize> {
    if state.position.item < change.b_new {
        return None;
    }
    let old_position = shift(&state.position, -delta);
    let q = old.pages.binary_search_by(|page| page.start.cmp(&old_position)).ok()?;
    let page = old.pages.get(q)?;
    let page_shift = i64::try_from(state.page).unwrap_or(0) - i64::try_from(base_of(old, q)).unwrap_or(0);
    let number_shift = state.number - page.number_before;
    let odd = page_shift.rem_euclid(2) != 0 || number_shift.rem_euclid(2) != 0;
    (!(parity && odd)).then_some(q)
}

/// The old pages from `q` on, moved to follow the new pages that end at `state`.
fn splice_rest(out: &mut Pagination, old: &Pagination, q: usize, state: &Resume, delta: isize) {
    let Some(join) = old.pages.get(q) else { return };
    let base = base_of(old, q);
    let page_shift = isize::try_from(state.page).unwrap_or(0) - isize::try_from(base).unwrap_or(0);
    let number_shift = state.number - join.number_before;
    out.pages.extend(
        old.pages
            .iter()
            .skip(q)
            .map(|page| moved(page, delta, page_shift, number_shift)),
    );
    out.info.extend(old.info.iter().skip(base).map(|info| {
        let mut i = *info;
        i.number += number_shift;
        i
    }));
    out.stopped = old.stopped.clone().map(|mut s| {
        s.item = s.item.saturating_add_signed(delta);
        s
    });
}

/// The new flow's pages from the old flow's: the pages before the change kept, the
/// change's pages laid out again, the rest moved once a page starts where an old one did
/// (by an even number of pages where parity decides something).
pub fn repaginate(cx: &PageContext<'_>, old: &Pagination, change: Change) -> (Pagination, RelayoutStats) {
    let target = Position::at(change.a, 0);
    let p = old
        .pages
        .partition_point(|page| page.reach < target)
        .min(old.pages.len().saturating_sub(1));
    let resume = old.pages.get(p).map(|page| Resume {
        position: page.start.clone(),
        page: page.page,
        number: page.number_before,
    });
    let delta = isize::try_from(change.b_new).unwrap_or(0) - isize::try_from(change.b_old).unwrap_or(0);
    let parity = parity_decides(cx);
    let mut joined: Option<usize> = None;
    let mut until = |state: &Resume| -> bool {
        joined = joins(old, change, delta, parity, state);
        joined.is_some()
    };
    let (fresh, stop) = Paginator::new(cx).run(resume, &mut until);
    let first_page = old.pages.get(p).map_or(0, |page| page.page);
    let mut out = Pagination {
        pages: old.pages.get(..p).unwrap_or_default().to_vec(),
        info: old.info.get(..first_page).unwrap_or_default().to_vec(),
        stopped: fresh.stopped.clone(),
    };
    let laid = fresh.pages.len();
    out.pages.extend(fresh.pages);
    out.info.extend(fresh.info);
    if let (Some(q), Some(state)) = (joined, stop) {
        splice_rest(&mut out, old, q, &state, delta);
    }
    (
        out,
        RelayoutStats {
            restart: p,
            laid,
            joined,
        },
    )
}
