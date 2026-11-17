//! Owns: the index's queries by place that reads ask without walking the part -- the
//! paragraphs below a place (a cell's, a comment's), and the paragraph just after or
//! before a place (where a range marker between blocks stands) -- each in O(log n) plus
//! the segment it lands in.
//!
//! Must not: name blocks any other way than the index does.

use super::segment::Kind;
use super::{Count, Located, ParagraphId, PartIds};
use ooxml_derive::Monoid;

impl PartIds {
    /// The paragraphs whose place starts with `prefix` (a cell's, a comment's, a table's),
    /// in document order.
    #[must_use]
    pub fn paragraphs_within(&self, prefix: &[u32]) -> Vec<ParagraphId> {
        let c = self.container.len();
        if prefix.len() > c && prefix.starts_with(&self.container) {
            let Some(k) = prefix.get(c).map(|k| *k as usize) else {
                return Vec::new();
            };
            let Ok(before) = self.segments.prefix(k) else {
                return Vec::new();
            };
            let Some(at) = self.segment(k, &before) else {
                return Vec::new();
            };
            let rest = prefix.get(c + 1..).unwrap_or_default();
            return self.paragraphs_of(&at, |rel| rel.starts_with(rest));
        }
        let mut out = Vec::new();
        for at in self.located() {
            let base = at.base.clone();
            out.extend(self.paragraphs_of(&at, |rel| {
                base.iter().chain(rel).take(prefix.len()).eq(prefix.iter()) && base.len() + rel.len() >= prefix.len()
            }));
        }
        out
    }

    fn paragraphs_of(&self, at: &Located<'_>, keep: impl Fn(&[u32]) -> bool) -> Vec<ParagraphId> {
        at.body
            .paragraphs
            .iter()
            .map(|k| *k as usize)
            .filter(|k| at.body.blocks.get(*k).is_some_and(|b| keep(&b.rel)))
            .filter_map(|k| self.paragraph_in(at, k))
            .collect()
    }

    /// How many paragraphs have a place before `marker` (`inclusive`: or at it).
    fn paragraphs_before(&self, marker: &[u32], inclusive: bool) -> usize {
        let counts = |at: &Located<'_>| {
            at.body
                .paragraphs
                .iter()
                .filter_map(|k| at.body.blocks.get(*k as usize))
                .filter(|b| b.kind == Kind::P)
                .filter(|b| {
                    let place: Vec<u32> = at.base.iter().chain(b.rel.iter()).copied().collect();
                    if inclusive {
                        place.as_slice() <= marker
                    } else {
                        place.as_slice() < marker
                    }
                })
                .count()
        };
        let total = self.total();
        let mut out = counts(&self.outer_before()) + counts(&self.outer_after());
        let c = self.container.len();
        if marker.len() > c && marker.starts_with(&self.container) {
            let k = marker.get(c).map_or(0, |k| *k as usize);
            let before = self.segments.prefix(k.min(self.segments.len())).unwrap_or_else(|_| Count::empty());
            out += before.paragraphs;
            if let Some(at) = self.segment(k, &before) {
                out += counts(&at);
            }
        } else if marker > &*self.container {
            out += total.paragraphs;
        }
        out
    }

    /// The first paragraph whose place is after `marker`.
    #[must_use]
    pub fn paragraph_after(&self, marker: &[u32]) -> Option<ParagraphId> {
        self.paragraphs().get(self.paragraphs_before(marker, true))
    }

    /// The last paragraph whose place is before `marker`.
    #[must_use]
    pub fn paragraph_before(&self, marker: &[u32]) -> Option<ParagraphId> {
        let n = self.paragraphs_before(marker, false);
        self.paragraphs().get(n.checked_sub(1)?)
    }
}
