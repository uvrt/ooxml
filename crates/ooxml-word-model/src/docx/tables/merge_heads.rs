//! Owns: a vertical merge as Accept All leaves it when rows it spans are pending deletions
//! (Word 16.106, round 60 R137, `row-deletions-word.json`): the rows go whole; where the
//! row holding the merge's first cell goes, the first kept cell continuing it starts the
//! merge (`restart`), with its own (empty) content -- the first cell's text goes with its
//! row -- or, with no kept cell continuing after it, stands alone (no merge). Rows between
//! kept cells of a merge going leave the merge as it was, also where their cell in that
//! column is no part of it (as a tracked merge across a pending row deletion writes it:
//! R137b, `MergeAcross`, `TrackedMergeAcross`).
//!
//! Must not: change a table, or decide which rows a view keeps or a cell's merge in it (the
//! caller says, from [`crate::view::View`]).

use super::cells_at;
use ooxml_xml::Element;
use std::collections::HashMap;

/// A cell's part in a vertical merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VMerge {
    /// Not merged.
    None,
    /// The merge's first cell (`w:vMerge w:val="restart"`).
    Restart,
    /// A cell continuing the merge above it (`w:vMerge`, `w:val="continue"`).
    Continue,
}

/// The cells continuing a merge in kept rows that the view makes a merge's first cell
/// ([`VMerge::Restart`]) or a cell of their own ([`VMerge::None`]), because the rows above
/// them in their merge are not `kept`; keyed by `(row, cell)`, the row's place in `rows`
/// and the cell's among its row's cells (row-level wrappers seen through) -- by place, not
/// by element: a parsed tree shares equal subtrees, and empty cells continuing a merge are
/// equal. Empty where every row is kept.
#[must_use]
pub fn merge_heads(
    rows: &[&Element],
    kept: &dyn Fn(&Element) -> bool,
    merge: &dyn Fn(&Element) -> VMerge,
) -> HashMap<(usize, usize), VMerge> {
    let mut out = HashMap::new();
    if rows.iter().all(|r| kept(r)) {
        return out;
    }
    // per grid column: whether a kept row holds a cell of the merge there so far, and the
    // head made there still waiting for a cell continuing it
    let mut held: HashMap<usize, bool> = HashMap::new();
    let mut waiting: HashMap<usize, (usize, usize)> = HashMap::new();
    for (r, row) in rows.iter().enumerate() {
        let row_kept = kept(row);
        for (n, cell) in cells_at(row, &[]).into_iter().enumerate() {
            let me = (r, n);
            match merge(&cell.element) {
                // a cell of a row that goes leaves the merge around it as it was, unless
                // it is the merge's first cell
                VMerge::Continue | VMerge::None if !row_kept => {}
                VMerge::Continue => {
                    if held.get(&cell.column).copied().unwrap_or(true) {
                        if let Some(head) = waiting.remove(&cell.column) {
                            out.insert(head, VMerge::Restart);
                        }
                    } else {
                        out.insert(me, VMerge::None);
                        waiting.insert(cell.column, me);
                        held.insert(cell.column, true);
                    }
                }
                VMerge::Restart | VMerge::None => {
                    waiting.remove(&cell.column);
                    held.insert(cell.column, row_kept);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{merge_heads, VMerge};
    use crate::view::View;
    use ooxml_base::XmlLimits;
    use ooxml_xml::Element;

    /// Heads of a table of `rows` (each `(deleted, [merge of the first cell])`, a second
    /// cell plain): per row, the first cell's head if any.
    fn heads(rows: &[(bool, &str)]) -> Vec<Option<VMerge>> {
        let xml: String = rows
            .iter()
            .map(|(deleted, merge)| {
                let del = if *deleted { "<w:trPr><w:del/></w:trPr>" } else { "" };
                format!("<w:tr>{del}<w:tc><w:tcPr>{merge}</w:tcPr><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr>")
            })
            .collect();
        let doc =
            format!(r#"<w:tbl xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{xml}</w:tbl>"#);
        let tree = ooxml_xml::parse(doc.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        let rows: Vec<&Element> = tree.root().child_elements().map(|r| &**r).collect();
        let kept = |r: &Element| r.find(Some(crate::ids::W), "trPr").is_none();
        let found = merge_heads(&rows, &kept, &|c| View::Final.merge(c));
        (0..rows.len()).map(|r| found.get(&(r, 0)).copied()).collect()
    }

    const RESTART: &str = r#"<w:vMerge w:val="restart"/>"#;
    const CONTINUE: &str = "<w:vMerge/>";

    #[test]
    fn the_first_kept_cell_of_a_merge_whose_first_row_goes_starts_it() {
        // Word 16.106's Accept All (round 60, R137): MergeStart, MergeStartTwo, MergeMiddle,
        // MergeEnd, MergeAbove
        let start = heads(&[(false, ""), (true, RESTART), (false, CONTINUE), (false, CONTINUE)]);
        assert_eq!(start, [None, None, Some(VMerge::Restart), None]);
        let alone = heads(&[(false, ""), (true, RESTART), (true, CONTINUE), (false, CONTINUE)]);
        assert_eq!(alone, [None, None, None, Some(VMerge::None)]);
        for kept_merge in [
            [(false, ""), (false, RESTART), (true, CONTINUE), (false, CONTINUE)],
            [(false, ""), (false, RESTART), (false, CONTINUE), (true, CONTINUE)],
            [(true, ""), (false, RESTART), (false, CONTINUE), (false, CONTINUE)],
        ] {
            assert_eq!(heads(&kept_merge), [None; 4]);
        }
        // MergeAcross, TrackedMergeAcross (R137b): the deleted row's cell is no part of the
        // merge around it; the merge stays
        let across = heads(&[(false, ""), (false, RESTART), (true, ""), (false, CONTINUE)]);
        assert_eq!(across, [None; 4]);
        let tracked = [
            (false, r#"<w:cellMerge w:vMerge="rest"/>"#),
            (true, ""),
            (false, r#"<w:cellMerge w:vMerge="cont"/>"#),
        ];
        assert_eq!(heads(&tracked), [None; 3]);
    }
}
