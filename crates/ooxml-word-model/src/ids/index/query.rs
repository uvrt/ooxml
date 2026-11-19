//! Owns: the index's queries by place that reads ask without walking the part -- the
//! paragraphs below a place (a cell's, a comment's), the blocks below a stretch of an
//! element's children (what an edit replaces), and the paragraph just after or before a
//! place (where a range marker between blocks stands) -- each in O(log n) plus the
//! segments it lands in.
//!
//! Must not: name blocks any other way than the index does.

use super::segment::Kind;
use super::{Count, Located, ParagraphId, PartIds, TableId};
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

    /// The paragraphs and the tables (nested ones included) below the children
    /// `from..=to` of the element at `parent`, each in document order: O(log n) plus the
    /// blocks there when `parent` is the block container or inside it (a stretch an edit
    /// replaces), a walk of the segments otherwise.
    #[must_use]
    pub fn within_stretch(&self, parent: &[u32], (from, to): (u32, u32)) -> (Vec<ParagraphId>, Vec<TableId>) {
        let mut out = (Vec::new(), Vec::new());
        let c = self.container.len();
        if parent == &*self.container {
            // every block of the container children from..=to: their segments, whole
            let Ok(mut seen) = self.segments.prefix((from as usize).min(self.segments.len())) else {
                return out;
            };
            for k in from as usize..=to as usize {
                let Some(seg) = self.segments.get(k) else { break };
                if let Some(at) = self.segment(k, &seen) {
                    self.blocks_of(&at, |_| true, &mut out);
                }
                seen = seen.combine(&Count::of(seg));
            }
            return out;
        }
        let inside = |base: &[u32], rel: &[u32]| {
            let mut place = base.iter().chain(rel);
            base.len() + rel.len() > parent.len()
                && place.by_ref().take(parent.len()).eq(parent.iter())
                && place.next().is_some_and(|k| (from..=to).contains(k))
        };
        if parent.len() > c && parent.starts_with(&self.container) {
            // inside one container child: its segment's blocks
            let Some(k) = parent.get(c).map(|k| *k as usize) else {
                return out;
            };
            let Ok(before) = self.segments.prefix(k) else {
                return out;
            };
            if let Some(at) = self.segment(k, &before) {
                let base = at.base.clone();
                self.blocks_of(&at, |rel| inside(&base, rel), &mut out);
            }
            return out;
        }
        for at in self.located() {
            let base = at.base.clone();
            self.blocks_of(&at, |rel| inside(&base, rel), &mut out);
        }
        out
    }

    /// The paragraphs and tables of `at` whose place below it `keep` takes, into `out`.
    fn blocks_of(&self, at: &Located<'_>, keep: impl Fn(&[u32]) -> bool, out: &mut (Vec<ParagraphId>, Vec<TableId>)) {
        let kept = |k: &&u32| at.body.blocks.get(**k as usize).is_some_and(|b| keep(&b.rel));
        let paragraphs = at.body.paragraphs.iter().filter(kept);
        out.0
            .extend(paragraphs.filter_map(|k| self.paragraph_in(at, *k as usize)));
        let tables = at.body.tables.iter().filter(kept);
        out.1.extend(tables.filter_map(|k| self.table_in(at, *k as usize)));
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
            let before = self
                .segments
                .prefix(k.min(self.segments.len()))
                .unwrap_or_else(|_| Count::empty());
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
