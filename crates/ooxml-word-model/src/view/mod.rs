//! Owns: the views of a document's tracked changes and what each keeps and drops, in one
//! place. [`View::Final`] is Word's Accept All, which is what render shows; [`View::Original`]
//! is Reject All; [`View::Markup`] shows both, with their marks. For each kind of record:
//! - an insertion, a deletion or a move's two ends ([`Revision`]): the content of a
//!   run-level container, a deleted text (`w:delText`);
//! - a paragraph mark's own record ([`View::removes_mark`]), and which paragraphs a removed
//!   mark joins ([`joins`]);
//! - a paragraph's properties change (`w:pPrChange`, [`View::paragraph_properties`]);
//! - a row's record (`w:trPr`), a cell's (`w:cellDel`, `w:cellIns`) and a pending vertical
//!   merge (`w:cellMerge`); a deleted cell whose content stays, and the cells whose end Accept
//!   All takes away ([`cells`]);
//! - a cell's properties change (`w:tcPrChange`, [`View::cell_properties`],
//!   [`View::merge_element`]) and a table's grid as Word rebuilds it from the cells the view
//!   leaves ([`ViewGrid`]).
//!
//! Every reading asks here: the layout's reader (`read`), the tools' text, tables, facts and
//! Markdown readings, and the change grouping. Names are WordprocessingML's in either
//! namespace ([`Ns::W`]).
//!
//! Must not: build a model, word a change, or change a document.

mod cells;
mod grid;
mod joins;

pub use cells::{cell_ends, content_kept, content_kept_in, row_ends, CellEnds};
pub use grid::{Fate, RowShape, ShapeCell, ViewCell, ViewGrid, ViewRow};
pub use joins::{plan as join_plan, Joined, Joining};

use crate::docx::tables::VMerge;
use crate::xmlread::{attr_in, child_in, Ns};
use ooxml_xml::Element;

/// A view of a document's tracked changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    /// Every change accepted (Word's Accept All): insertions in, deletions and a move's
    /// source out. What render shows.
    Final,
    /// Every change rejected (Reject All): deletions in, insertions and a move's destination
    /// out.
    Original,
    /// Both, with their marks.
    Markup,
}

/// A tracked insertion, deletion or move end: `w:ins`, `w:del`, `w:moveTo`, `w:moveFrom`,
/// whether it holds content (a run-level container) or marks a paragraph mark, a run's
/// properties or a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Revision {
    /// `w:ins`.
    Inserted,
    /// `w:del`.
    Deleted,
    /// `w:moveTo`, a move's destination.
    MovedTo,
    /// `w:moveFrom`, a move's source.
    MovedFrom,
}

impl Revision {
    /// The revision `element` is, if it is one.
    #[must_use]
    pub fn of(element: &Element) -> Option<Revision> {
        if !Ns::W.holds(element.name().namespace()) {
            return None;
        }
        Some(match element.name().local() {
            "ins" => Revision::Inserted,
            "del" => Revision::Deleted,
            "moveTo" => Revision::MovedTo,
            "moveFrom" => Revision::MovedFrom,
            _ => return None,
        })
    }

    /// Whether it adds what it marks (an insertion, a move's destination).
    #[must_use]
    pub fn adds(self) -> bool {
        matches!(self, Revision::Inserted | Revision::MovedTo)
    }

    /// Whether it is a move's end.
    #[must_use]
    pub fn moves(self) -> bool {
        matches!(self, Revision::MovedTo | Revision::MovedFrom)
    }
}

impl View {
    /// Whether what `revision` marks is in this view.
    #[must_use]
    pub fn keeps(self, revision: Revision) -> bool {
        match self {
            View::Final => revision.adds(),
            View::Original => !revision.adds(),
            View::Markup => true,
        }
    }

    /// Whether a revision container's content shows in this view; `None` for an element
    /// that is no revision.
    #[must_use]
    pub fn shows(self, container: &Element) -> Option<bool> {
        Revision::of(container).map(|r| self.keeps(r))
    }

    /// Whether a deleted text (`w:delText`) shows.
    #[must_use]
    pub fn shows_deleted_text(self) -> bool {
        self.keeps(Revision::Deleted)
    }

    /// Whether this view removes the paragraph's mark: a record of its mark
    /// (`w:pPr/w:rPr`) it does not keep. A paragraph whose mark is removed joins the next
    /// ([`joins`]).
    #[must_use]
    pub fn removes_mark(self, paragraph: &Element) -> bool {
        child_in(paragraph, Ns::W, "pPr")
            .and_then(|p| child_in(p, Ns::W, "rPr"))
            .is_some_and(|mark| self.drops_any(mark))
    }

    /// Whether this view removes the row: a record of the row (`w:trPr`) it does not keep
    /// (Word 16.106, round 60 R137), or none of its cells left -- in the final view every
    /// cell deleted, or those left gone with their ends ([`cells`]; Word 16.106, lane
    /// ph3-rd1c), in the original every cell inserted (Reject All, Word 16.106, lane
    /// fix-moved-left, `RowRemoved`). The row goes whole, its cells and the tables nested in
    /// them with it.
    #[must_use]
    pub fn removes_row(self, row: &Element) -> bool {
        child_in(row, Ns::W, "trPr").is_some_and(|p| self.drops_any(p))
            || match self {
                View::Final => cells::no_cell_left(row),
                View::Original => cells::every_cell_removed(View::Original, row),
                View::Markup => false,
            }
    }

    /// Whether this view removes the cell: its deletion (`w:cellDel`) in the final view,
    /// its insertion (`w:cellIns`) in the original. A deleted cell whose content stays
    /// is merged into a cell beside it ([`content_kept_in`], [`ViewGrid`]).
    #[must_use]
    pub fn removes_cell(self, cell: &Element) -> bool {
        let record = match self {
            View::Final => "cellDel",
            View::Original => "cellIns",
            View::Markup => return false,
        };
        child_in(cell, Ns::W, "tcPr").is_some_and(|p| child_in(p, Ns::W, record).is_some())
    }

    /// A cell's part in a vertical merge in this view: in the final view a pending tracked
    /// merge (`w:cellMerge`) as accepting leaves it, otherwise its `w:vMerge` in this view
    /// ([`View::merge_element`]).
    #[must_use]
    pub fn merge(self, cell: &Element) -> VMerge {
        if self == View::Final {
            let tracked = child_in(cell, Ns::W, "tcPr").and_then(|p| child_in(p, Ns::W, "cellMerge"));
            if let Some(tracked) = tracked {
                return match attr_in(tracked, Some(Ns::W), "vMerge") {
                    Some("rest") => VMerge::Restart,
                    Some("cont") => VMerge::Continue,
                    _ => VMerge::None,
                };
            }
        }
        match self.merge_element(cell).map(|m| attr_in(m, Some(Ns::W), "val")) {
            Some(None | Some("continue")) => VMerge::Continue,
            Some(Some("restart")) => VMerge::Restart,
            _ => VMerge::None,
        }
    }

    /// The paragraph properties (`w:pPr`) this view shows, and whether they are the ones a
    /// properties change recorded: in the original view, the `w:pPr` a `w:pPrChange` holds
    /// (the caller keeps the mark's `w:rPr` and the `w:sectPr`, which a change does not
    /// record); otherwise `properties` itself.
    #[must_use]
    pub fn paragraph_properties(self, properties: &Element) -> (&Element, bool) {
        if self == View::Original {
            if let Some(old) = child_in(properties, Ns::W, "pPrChange").and_then(|c| child_in(c, Ns::W, "pPr")) {
                return (old, true);
            }
        }
        (properties, false)
    }

    /// Whether a child of `holder` is a revision this view does not keep.
    fn drops_any(self, holder: &Element) -> bool {
        holder
            .child_elements()
            .filter_map(|c| Revision::of(c))
            .any(|r| !self.keeps(r))
    }
}

#[cfg(test)]
mod tests {
    use super::{Revision, View};
    use ooxml_base::XmlLimits;

    fn element(xml: &str) -> std::sync::Arc<ooxml_xml::Element> {
        let doc = xml.replace(
            "<w:p>",
            r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        );
        let tree = ooxml_xml::parse(doc.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        std::sync::Arc::clone(tree.root())
    }

    #[test]
    fn each_view_keeps_its_side_of_every_revision() {
        let all = [
            Revision::Inserted,
            Revision::Deleted,
            Revision::MovedTo,
            Revision::MovedFrom,
        ];
        let kept = |view: View| all.iter().map(|r| view.keeps(*r)).collect::<Vec<_>>();
        assert_eq!(kept(View::Final), [true, false, true, false]);
        assert_eq!(kept(View::Original), [false, true, false, true]);
        assert_eq!(kept(View::Markup), [true; 4]);
        assert!(!View::Final.shows_deleted_text());
        assert!(View::Original.shows_deleted_text() && View::Markup.shows_deleted_text());
    }

    #[test]
    fn a_mark_is_removed_by_the_view_that_drops_its_record() {
        let deleted = element("<w:p><w:pPr><w:rPr><w:del/></w:rPr></w:pPr></w:p>");
        let moved_in = element("<w:p><w:pPr><w:rPr><w:b/><w:moveTo/></w:rPr></w:pPr></w:p>");
        let plain = element("<w:p><w:pPr><w:rPr><w:b/></w:rPr></w:pPr></w:p>");
        assert!(View::Final.removes_mark(&deleted) && !View::Original.removes_mark(&deleted));
        assert!(!View::Final.removes_mark(&moved_in) && View::Original.removes_mark(&moved_in));
        assert!(!View::Markup.removes_mark(&deleted) && !View::Markup.removes_mark(&moved_in));
        assert!(!View::Final.removes_mark(&plain) && !View::Original.removes_mark(&plain));
    }

    #[test]
    fn a_strict_document_names_the_same_revisions() {
        let tree = ooxml_xml::parse(
            br#"<w:del xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main"/>"#,
            &XmlLimits::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(Revision::of(tree.root()), Some(Revision::Deleted));
        let other = ooxml_xml::parse(br#"<x:del xmlns:x="urn:other"/>"#, &XmlLimits::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(Revision::of(other.root()), None);
    }
}
