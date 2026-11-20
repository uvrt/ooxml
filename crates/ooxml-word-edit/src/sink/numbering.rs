//! Owns: how an edit numbers its revision records and range markers (annotation ids), as
//! docx-agent's `Stamp` does -- each write numbers from one above the largest annotation
//! id the document holds as it is then -- with ids that are stand-ins, drawn again where
//! the change is applied ([`crate::DrawFrom::Annotation`]: a [`Change::Number`] plus an
//! offset), so revision ids are no conflict between transactions.
//!
//! An edit's ids count on from its last one while any id it gave is not yet written into
//! the working version (a record built in a [`crate::work::Work`] and put in place later is
//! in no tree the version holds). Once every id it gave is written, a write that removed
//! the edit's or another write's records outright (one's own insertion deleted) can leave
//! the document's largest id below the next one: the edit then numbers again from the
//! working version (another [`Change::Number`]), reusing those ids as docx-agent does.
//!
//! Must not: number from the working version while an id it gave may be unwritten.

use super::{EditSink, Numbered};
use crate::doc::{Change, Draw, DrawFrom, Key, WordDoc};
use crate::policy::Stamp;
use crate::validate::annotation_ceiling;
use crate::{EditError, Result};
use ooxml_store::ReadPredicate;
use std::sync::Arc;

/// An edit's numbering: the first id of its latest [`Change::Number`], the next id it
/// gives, and one above the largest it wrote.
#[derive(Debug, Default)]
pub(super) struct Numbering {
    /// `(base, next)` once the edit began numbering.
    at: Option<(i64, i64)>,
    /// One above the largest id a write carried (an annotation draw of the latest base).
    written_to: i64,
}

impl Numbering {
    /// Note the annotation ids a splice's `draws` write.
    pub(super) fn written(&mut self, draws: &[Draw]) {
        let Some((base, _)) = self.at else { return };
        for draw in draws {
            if let DrawFrom::Annotation(offset) = draw.from {
                self.written_to = self.written_to.max(base + i64::from(offset) + 1);
            }
        }
    }
}

impl EditSink<'_, '_> {
    /// The next annotation id this edit gives: one above the largest annotation id in use
    /// in any story of the working version as the edit begins numbering, or again once
    /// every id it gave is written (see the module's notes); else the one after its last.
    ///
    /// # Errors
    /// When a story part is not XML.
    pub fn next_annotation_id(&mut self) -> Result<i64> {
        let pending = self.numbered.at.filter(|(_, next)| self.numbered.written_to < *next);
        if let Some((_, next)) = pending {
            return Ok(next);
        }
        let first = annotation_ceiling(self.tx.doc())? + 1;
        if let Some((_, next)) = self.numbered.at.filter(|(_, next)| first >= *next) {
            return Ok(next);
        }
        self.write(Change::Number)?;
        self.numbered.at = Some((first, first));
        self.numbered.written_to = first;
        Ok(first)
    }

    /// The edit's ids given up to `next` (one above the last), counted from `from`.
    fn given(&mut self, from: i64, next: i64) -> Numbered {
        let base = self.numbered.at.map_or(from, |(base, _)| base);
        self.numbered.at = Some((base, next));
        Numbered { base, from, to: next }
    }

    /// One annotation id for a range marker this edit writes (a bookmark's, a comment's):
    /// its value on this version and its offset from the edit's first id, for a
    /// [`Draw`] that numbers it again where the change is applied.
    ///
    /// # Errors
    /// When a story part is not XML.
    pub(crate) fn marker_id(&mut self) -> Result<(i64, u32)> {
        let value = self.next_annotation_id()?;
        let numbered = self.given(value, value + 1);
        let offset = u32::try_from(value - numbered.base)
            .map_err(|_| EditError::Refused("too many annotation ids".to_owned()))?;
        Ok((value, offset))
    }

    /// Revision records an edit of its own writes (`write` given the [`Stamp`]), numbered
    /// on from this edit's next id; the ids it gave, and whether a record carries a UTC
    /// date.
    ///
    /// # Errors
    /// `write`'s, or when a story part is not XML.
    pub(crate) fn stamped<R>(
        &mut self,
        tracking: crate::policy::Tracking,
        write: impl FnOnce(&mut Stamp) -> Result<R>,
    ) -> Result<(R, Numbered, bool)> {
        let from = self.next_annotation_id()?;
        let mut stamp = Stamp::new(tracking, from);
        let out = write(&mut stamp)?;
        let numbered = self.given(from, stamp.next());
        Ok((out, numbered, stamp.dated))
    }

    /// A tracked write through a [`Stamp`] numbering on from this edit's next id.
    pub(super) fn tracked(
        &mut self,
        tracking: crate::policy::Tracking,
        write: impl FnOnce(&mut Stamp) -> Result<Vec<String>>,
    ) -> Result<(Vec<String>, bool, Numbered)> {
        let from = self.next_annotation_id()?;
        let mut stamp = Stamp::new(tracking, from);
        let released = write(&mut stamp)?;
        let numbered = self.given(from, stamp.next());
        Ok((released, stamp.dated, numbered))
    }

    /// Depend on the largest annotation id in use as the edit's version holds it, by
    /// condition ([`AnnotationCeiling`]): for an edit that writes ids it numbered where they
    /// are not drawn again when the change is applied (a part put whole).
    ///
    /// # Errors
    /// When a story part is not XML.
    pub(crate) fn read_annotation_ceiling(&mut self) -> Result<()> {
        let ceiling = annotation_ceiling(self.tx.doc())?;
        self.read_where(Arc::new(AnnotationCeiling { ceiling }));
        Ok(())
    }
}

/// The largest annotation id in use, read by condition: a commit since the edit's snapshot
/// that leaves it otherwise on the head -- ids numbered, or the holder of the largest
/// removed -- is a conflict, so ids numbered from it are those the serial order gives.
#[derive(Debug)]
pub(crate) struct AnnotationCeiling {
    ceiling: i64,
}

impl ReadPredicate<WordDoc> for AnnotationCeiling {
    fn depends(&self, head: &WordDoc, _key: &Key) -> bool {
        // the head's tally is read as it is published: no walk here
        annotation_ceiling(head).map_or(true, |ceiling| ceiling != self.ceiling)
    }
}
