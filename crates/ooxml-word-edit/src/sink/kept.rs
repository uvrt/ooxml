//! Owns: the ids a copy keeps ([`EditSink::keep_or_draw`]): a copied element's own
//! paraIds, textIds and drawing ids kept where the document does not use them and never
//! did, the others drawn, as docx-agent's `_stamp` keeps or issues them; and the drawing
//! numbers a copy keeps ([`EditSink::keep_or_draw_drawings`], `_drawing_ids`).
//!
//! Must not: write anything but the ledger.

use super::EditSink;
use crate::doc::{Change, Key};
use crate::Result;
use std::sync::Arc;

impl EditSink<'_, '_> {
    /// Ids for one element of a copy (docx-agent's `_stamp`, `keep_or_issue`): each of
    /// `values` (the copy's own, as a number and as written; `None` where it has none
    /// valid) kept, in upper case, where the document does not use it and never did, the
    /// others drawn from `seed`; the kept ones are recorded as used (`_remember_ids`). Read
    /// and written under `seed`'s key, as a draw is.
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub(crate) fn keep_or_draw(&mut self, seed: &str, values: &[Option<(u32, &str)>]) -> Result<Vec<String>> {
        self.tx.read(Key::Seq(Arc::from(seed)));
        let mut kept: Vec<u32> = Vec::new();
        let chosen: Vec<Option<String>> = values
            .iter()
            .map(|value| {
                let (v, raw) = (*value)?;
                if self.tx.doc().ids().contains(v) || kept.contains(&v) {
                    return None;
                }
                kept.push(v);
                Some(raw.to_uppercase())
            })
            .collect();
        if !kept.is_empty() {
            self.write(Change::Ids(crate::pool::IdsChange {
                values: kept,
                ..Default::default()
            }))?;
        }
        let missing = chosen.iter().filter(|v| v.is_none()).count();
        let mut drawn = if missing == 0 {
            Vec::new()
        } else {
            self.draw(seed, missing)?
        }
        .into_iter();
        Ok(chosen
            .into_iter()
            .map(|v| v.unwrap_or_else(|| drawn.next().unwrap_or_default()))
            .collect())
    }

    /// Drawing numbers for a copy's drawings, in document order (docx-agent's
    /// `Importer._drawing_ids`): each of `values` (`None`: none a whole number) kept where
    /// the document never had it and the copy has not given it yet, else the number after
    /// every one given ([`crate::Ids::highest_drawing`], raised by the kept ones). Read and
    /// written under the drawing numbers' key, as a draw is.
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub(crate) fn keep_or_draw_drawings(&mut self, values: &[Option<i64>]) -> Result<Vec<i64>> {
        if values.is_empty() {
            return Ok(Vec::new());
        }
        self.tx.read(Key::Seq(Arc::from(crate::pool::DRAWING_NUMBERS)));
        let ids = self.tx.doc().ids();
        let mut highest = i64::try_from(ids.highest_drawing()).unwrap_or(i64::MAX);
        let mut given: Vec<i64> = Vec::with_capacity(values.len());
        for value in values {
            let had = |v: i64| u64::try_from(v).is_ok_and(|n| ids.had_drawing(n)) || given.contains(&v);
            let number = match *value {
                Some(v) if !had(v) => v,
                _ => highest.saturating_add(1),
            };
            highest = highest.max(number);
            given.push(number);
        }
        let mut drawings: Vec<u64> = given.iter().filter_map(|v| u64::try_from(*v).ok()).collect();
        drawings.sort_unstable();
        drawings.dedup();
        let change = crate::pool::IdsChange {
            highest_drawing: u64::try_from(highest).unwrap_or(0),
            drawings,
            ..Default::default()
        };
        self.write(Change::Ids(change))?;
        Ok(given)
    }
}
