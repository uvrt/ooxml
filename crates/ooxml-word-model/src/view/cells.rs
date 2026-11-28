//! Owns: a table's cells in the final view (Accept All; measured, docx2svg's ROADMAP
//! "Revisions -- measured" R.3, round 54 R110): whether a deleted cell's content stays (it
//! widens the cell before it), and a row's cell ends -- a cell whose last paragraph mark is
//! removed takes the next cell in; the last cell of such a chain, its end removed with no
//! cell after it, goes with its text.
//!
//! Must not: read a cell's properties or build a model.

use super::{Revision, View};
use crate::xmlread::{child_in, Ns};
use ooxml_xml::Element;
use std::collections::HashSet;

/// Run content a reader sees (docx-agent's `review._SHOWN`).
const SHOWN: [&str; 10] = [
    "t",
    "tab",
    "br",
    "cr",
    "drawing",
    "pict",
    "object",
    "sym",
    "noBreakHyphen",
    "ptab",
];

/// Whether a deleted cell keeps anything in the final view (docx-agent's `_content_kept`):
/// content outside deletions and moves away, or a paragraph mark of its own the final view
/// keeps. Such a cell widens the cell before it.
#[must_use]
pub fn content_kept(cell: &Element) -> bool {
    fn shown(node: &Element, gone: bool) -> bool {
        node.child_elements().any(|e| {
            let gone = gone || Revision::of(e).is_some_and(|r| !View::Final.keeps(r));
            let visible = Ns::W.holds(e.name().namespace())
                && SHOWN.contains(&e.name().local())
                && !(e.name().local() == "t" && crate::docx::text::leading_text(e).is_empty());
            (visible && !gone) || shown(e, gone)
        })
    }
    fn marks(node: &Element) -> bool {
        node.child_elements().any(|e| {
            if Ns::W.named(e, "tc") {
                return false;
            }
            (Ns::W.named(e, "p") && !View::Final.removes_mark(e)) || marks(e)
        })
    }
    shown(cell, false) || marks(cell)
}

/// Whether the last block of a cell (or of a control's content) is a paragraph whose mark
/// the final view removes.
fn last_mark_removed(container: &Element) -> bool {
    let blocky = |e: &Element| {
        e.child_elements()
            .any(|n| ["p", "tbl", "sdt", "customXml"].iter().any(|b| Ns::W.named(n, b)))
    };
    let nodes: Vec<&std::sync::Arc<Element>> = container.child_elements().collect();
    for node in nodes.into_iter().rev() {
        if Ns::W.named(node, "p") {
            return View::Final.removes_mark(node);
        }
        if Ns::W.named(node, "tbl") {
            return false;
        }
        if Ns::W.named(node, "sdt") {
            if let Some(content) = child_in(node, Ns::W, "sdtContent").filter(|c| blocky(c)) {
                return last_mark_removed(content);
            }
        } else if Ns::W.named(node, "customXml") && blocky(node) {
            return last_mark_removed(node);
        }
    }
    false
}

/// A row's cells in the final view, from the cells it shows (those it does not remove).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellEnds {
    /// Each cell kept, by its place among the cells shown, with the cells it takes in after
    /// it (its last mark removed, and theirs but the last's).
    pub groups: Vec<Vec<usize>>,
    /// The cells gone: those taken in, and every cell of a chain whose last cell's end is
    /// removed with no cell after it.
    pub gone: HashSet<usize>,
}

impl CellEnds {
    /// Whether accepting takes any cell's end away here: a cell taken in, or one gone.
    #[must_use]
    pub fn any(&self) -> bool {
        !self.gone.is_empty() || self.groups.iter().any(|g| g.len() > 1)
    }

    /// The cells removed with their text: gone, and not taken into a cell before them.
    pub fn lost(&self) -> impl Iterator<Item = usize> + '_ {
        let taken: HashSet<usize> = self.groups.iter().flat_map(|g| g.iter().skip(1).copied()).collect();
        let mut lost: Vec<usize> = self.gone.iter().copied().filter(|n| !taken.contains(n)).collect();
        lost.sort_unstable();
        lost.into_iter()
    }
}

/// The cell ends of a row whose cells shown are `cells`, in order.
#[must_use]
pub fn cell_ends(cells: &[&Element]) -> CellEnds {
    let mut out = CellEnds::default();
    let removed = |k: usize| cells.get(k).is_some_and(|c| last_mark_removed(c));
    let mut n = 0;
    while n < cells.len() {
        let mut group = vec![n];
        while group.last().is_some_and(|&g| removed(g) && g + 1 < cells.len()) {
            let next = group.last().map_or(n, |g| g + 1);
            group.push(next);
        }
        let last = group.last().copied().unwrap_or(n);
        if removed(last) {
            out.gone.extend(group.iter().copied());
        } else {
            out.gone.extend(group.iter().skip(1).copied());
            out.groups.push(group);
        }
        n = last + 1;
    }
    out
}

/// The cell ends of `row` in the final view, from its cells (row-level wrappers seen
/// through) less those the view removes.
#[must_use]
pub fn row_ends(row: &Element) -> CellEnds {
    let cells = crate::docx::tables::cells_at(row, &[]);
    let shown: Vec<&Element> = cells
        .iter()
        .map(|c| &*c.element)
        .filter(|c| !View::Final.removes_cell(c))
        .collect();
    cell_ends(&shown)
}

/// Whether the final view leaves none of a row's cells, which Word's Accept All removes the
/// row for (Word 16.106, lane ph3-rd1c): every cell deleted, its content kept or not, or the
/// cells left gone with their ends -- a deleted cell whose content stays has no cell before
/// it to widen then. A row of no cells at all is left as it is.
pub(super) fn no_cell_left(row: &Element) -> bool {
    !crate::docx::tables::cells_at(row, &[]).is_empty() && row_ends(row).groups.is_empty()
}
