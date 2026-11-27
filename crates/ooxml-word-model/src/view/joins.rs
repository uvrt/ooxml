//! Owns: which paragraphs are one in a view (measured in Word's Accept All, every
//! compatibility mode; docx2svg's `_join`; round 60 R146): a paragraph whose mark the view
//! removes joins the next paragraph, whatever block holds it -- a chain into the last; the
//! first paragraph of a content control's content; in the final view, the first paragraph
//! of the table after it, unless the paragraph ends a section (a section break stays, and
//! the paragraphs before it join it). The plan for one container's blocks, by their places
//! in it; the readers make the joined paragraphs.
//!
//! Must not: read XML or build a paragraph.

use super::View;

/// One block of a container, as a join reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joining {
    /// A paragraph.
    Paragraph {
        /// Whether the view removes its mark ([`View::removes_mark`]).
        removed: bool,
        /// Whether it ends a section (its `w:pPr/w:sectPr`).
        section: bool,
        /// Whether it shows anything in the view: text, or a section break. Read only in
        /// the original view, where paragraphs that join nothing and show nothing go (the
        /// markup view removes no mark).
        shown: bool,
    },
    /// A table.
    Table,
    /// A content control's start (block level): its content follows, up to its [`Joining::Close`].
    Open,
    /// A content control's end.
    Close,
}

/// One block of a container in a view, the blocks it is made of by their places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Joined {
    /// Paragraphs that are one, named by the last (as Word keeps it).
    Paragraph(Vec<usize>),
    /// The table at `at`, with the paragraphs joined into its first cell's first paragraph.
    Table {
        /// The table's place.
        at: usize,
        /// The paragraphs joined into it, in order.
        joins: Vec<usize>,
    },
    /// The control start at `at`, with the paragraphs before it that join its first
    /// paragraph (none where the control holds no paragraph that ends their chain).
    Open {
        /// The start's place.
        at: usize,
        /// The paragraphs joining into it, in order.
        joins: Vec<usize>,
    },
    /// The control end at its place.
    Close(usize),
}

/// The blocks of a container in `view`: `blocks` joined as the view joins them.
#[must_use]
pub fn plan(view: View, blocks: &[Joining]) -> Vec<Joined> {
    let mut out = Vec::new();
    let mut pending: Vec<usize> = Vec::new();
    // Paragraph chains ended so far, and for each control open its start's place in `out`
    // with the count when it took the pending paragraphs: a control that ends before any
    // chain ends holds no paragraph of them (they join what comes after it).
    let mut ended = 0usize;
    let mut open: Vec<Option<(usize, usize)>> = Vec::new();
    let shown = |p: &[usize]| {
        p.iter()
            .any(|k| matches!(blocks.get(*k), Some(Joining::Paragraph { shown: true, .. })))
    };
    for (k, block) in blocks.iter().enumerate() {
        match *block {
            Joining::Paragraph { removed, .. } => {
                pending.push(k);
                if !removed {
                    out.push(Joined::Paragraph(std::mem::take(&mut pending)));
                    ended += 1;
                }
            }
            Joining::Open => {
                let joined = (!pending.is_empty()).then_some((out.len(), ended));
                open.push(joined);
                out.push(Joined::Open {
                    at: k,
                    joins: if joined.is_some() { pending.clone() } else { Vec::new() },
                });
            }
            Joining::Close => {
                if let Some(Some((start, at))) = open.pop() {
                    if at == ended {
                        if let Some(Joined::Open { joins, .. }) = out.get_mut(start) {
                            joins.clear();
                        }
                    }
                }
                out.push(Joined::Close(k));
            }
            Joining::Table => {
                ended += usize::from(!pending.is_empty());
                let mut joins = Vec::new();
                if !pending.is_empty() && view == View::Final {
                    let cut = pending
                        .iter()
                        .rposition(|p| matches!(blocks.get(*p), Some(Joining::Paragraph { section: true, .. })));
                    joins = match cut {
                        Some(cut) => {
                            let rest = pending.split_off(cut + 1);
                            out.push(Joined::Paragraph(std::mem::take(&mut pending)));
                            rest
                        }
                        None => std::mem::take(&mut pending),
                    };
                } else if !pending.is_empty() {
                    if shown(&pending) {
                        out.push(Joined::Paragraph(std::mem::take(&mut pending)));
                    }
                    pending.clear();
                }
                out.push(Joined::Table { at: k, joins });
            }
        }
    }
    if !pending.is_empty() && (view != View::Original || shown(&pending)) {
        out.push(Joined::Paragraph(pending));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{plan, Joined, Joining};
    use crate::view::View;

    const fn p(removed: bool, section: bool) -> Joining {
        Joining::Paragraph {
            removed,
            section,
            shown: true,
        }
    }

    #[test]
    fn a_removed_mark_joins_the_next_paragraph_or_the_table_after_it() {
        let blocks = [
            p(true, false),
            p(true, false),
            p(false, false),
            p(true, false),
            Joining::Table,
        ];
        assert_eq!(
            plan(View::Final, &blocks),
            [
                Joined::Paragraph(vec![0, 1, 2]),
                Joined::Table { at: 4, joins: vec![3] }
            ]
        );
        // outside the final view a paragraph joins no table
        assert_eq!(
            plan(View::Original, &blocks),
            [
                Joined::Paragraph(vec![0, 1, 2]),
                Joined::Paragraph(vec![3]),
                Joined::Table { at: 4, joins: vec![] }
            ]
        );
    }

    #[test]
    fn a_section_break_stays_before_a_table_and_takes_the_paragraphs_before_it() {
        let blocks = [
            p(true, false),
            p(true, true),
            p(true, false),
            Joining::Table,
            p(true, false),
        ];
        assert_eq!(
            plan(View::Final, &blocks),
            [
                Joined::Paragraph(vec![0, 1]),
                Joined::Table { at: 3, joins: vec![2] },
                Joined::Paragraph(vec![4])
            ]
        );
    }

    #[test]
    fn a_control_takes_the_chain_only_where_a_paragraph_in_it_ends_it() {
        let ends_inside = [p(true, false), Joining::Open, p(false, false), Joining::Close];
        assert_eq!(
            plan(View::Final, &ends_inside),
            [
                Joined::Open { at: 1, joins: vec![0] },
                Joined::Paragraph(vec![0, 2]),
                Joined::Close(3)
            ]
        );
        let empty = [p(true, false), Joining::Open, Joining::Close, p(false, false)];
        assert_eq!(
            plan(View::Final, &empty),
            [
                Joined::Open { at: 1, joins: vec![] },
                Joined::Close(2),
                Joined::Paragraph(vec![0, 3])
            ]
        );
    }
}
