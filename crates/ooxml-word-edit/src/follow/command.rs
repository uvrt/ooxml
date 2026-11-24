//! Owns: [`FollowSample`] run as a command: the sample and copies located and checked, the
//! document stamped, each copy followed on an editable copy of its block and put in place
//! through the sink (tracked records numbered on), what docx-agent's `_edit` does after.
//!
//! Must not: compare, or write but through the sink.

use super::paragraph::follow_paragraph;
use super::table::{follow_rows, follow_table};
use super::{
    joined, locate, place_below, Asked, CopyFollowed, FollowSample, Followed, Located, Log, Styles, ROWS_UNTRACKED,
};
use crate::policy::RevisionPolicy;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::tables::tracker::TableTracker;
use crate::tables::{write, Found, Written};
use crate::work::{scope_above, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::{at_path, Docx};
use std::sync::Arc;

/// The blocks a call names, located: the sample, each copy with the address asked, and the
/// notes on what was taken otherwise or ignored.
struct Named {
    sample: Located,
    copies: Vec<(String, Located)>,
    notes: Vec<String>,
}

impl FollowSample {
    /// The sample and the copies as asked, located (docx-agent's checks before the edit).
    fn located(&self, doc: &WordDoc, docx: &Docx) -> Result<Named> {
        let mut notes = Vec::new();
        let source = self.source.as_deref().unwrap_or(docx);
        let here = self.source.is_none().then_some(doc);
        let asked = Asked {
            field: "sample",
            paragraph: false,
            like: None,
        };
        let sample = locate(here, source, &self.sample, &asked, &mut notes)?;
        let asked = Asked {
            field: "copies",
            paragraph: !sample.table,
            like: Some(&sample.element),
        };
        let mut copies: Vec<(String, Located)> = Vec::new();
        for identifier in &self.copies {
            if identifier.trim().is_empty() {
                notes.push("an empty copy address was ignored".to_owned());
                continue;
            }
            let found = locate(Some(doc), docx, identifier.trim(), &asked, &mut notes)?;
            if self.source.is_none() && found.same_place(&sample) {
                notes.push(format!("{identifier} is the sample itself: nothing to follow"));
                continue;
            }
            if copies.iter().any(|(_, c)| c.same_place(&found)) {
                notes.push(format!("{identifier} was named twice: followed once"));
                continue;
            }
            copies.push((identifier.clone(), found));
        }
        if copies.is_empty() && self.copies.is_empty() {
            let tables = super::shapes(docx, Some(&sample.element));
            let listed = if tables.is_empty() {
                "none".to_owned()
            } else {
                tables.join(", ")
            };
            return Err(EditError::Refused(format!(
                "copies names at least one block made from the sample; tables like it: {listed}"
            )));
        }
        let mut unique: Vec<String> = Vec::new();
        for note in notes {
            if !unique.contains(&note) {
                unique.push(note);
            }
        }
        Ok(Named {
            sample,
            copies,
            notes: unique,
        })
    }

    /// The block `id` (after stamping) in the working version, or in the sample's document.
    fn now(sink: &EditSink<'_, '_>, docx: &Docx, id: &str, here: bool) -> Result<Located> {
        super::block(here.then(|| sink.doc()), docx, id).map_err(EditError::Refused)
    }

    /// One copy followed: located again after stamping, its block's editable copy given
    /// the sample's formatting, recorded when tracked and put in place.
    fn follow_one(
        &self,
        sink: &mut EditSink<'_, '_>,
        (asked, id): (&str, &str),
        sample_id: &str,
        warnings: &mut Vec<String>,
    ) -> Result<CopyFollowed> {
        let docx = sink.docx()?;
        let found = Self::now(sink, &docx, id, true)?;
        let source = self.source.clone().unwrap_or_else(|| Arc::clone(&docx));
        let sample = Self::now(sink, &source, sample_id, self.source.is_none())?;
        sink.read(block_key(&found.part, &found.id));
        let tree = sink
            .doc()
            .tree(&found.part)?
            .ok_or_else(|| EditError::no_part(found.part.clone()))?;
        let element = at_path(tree.root(), &found.path).ok_or_else(|| EditError::no_element(found.part.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &found.path));
        let root = work.root();
        let sample_node = work
            .adopt(&sample.element)
            .ok_or_else(|| EditError::no_element(sample.part.clone()))?;
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let alike = found.table == sample.table;
        let watch = if alike {
            sink.watch_properties(&mut work, &[root])
        } else {
            None
        };
        let tracker = (alike && found.table && watch.is_some()).then(|| TableTracker::new(&mut work, root, true));
        let mut log = Log::default();
        let (rows, missing) = {
            let mut styles = Styles::new(sink, self.source.as_deref());
            let rows = if alike && found.table {
                follow_table(&mut work, root, sample_node, &mut styles, &mut log, orders)?
            } else {
                if alike {
                    follow_paragraph(&mut work, root, sample_node, &mut styles, &mut log, orders)?;
                }
                Vec::new()
            };
            (rows, std::mem::take(&mut styles.missing))
        };
        let recorded = sink.properties_changed(&mut work, watch)?;
        let mut written = Written {
            numbered: recorded.annotations,
            dated: recorded.dated,
            warnings: Vec::new(),
        };
        if let (Some(tracker), RevisionPolicy::Tracked(tracking)) = (tracker, sink.policy().clone()) {
            let ((), numbered, dated) = sink.stamped(tracking, |stamp| {
                tracker.record(&mut work, stamp, true, true, orders);
                Ok(())
            })?;
            written.numbered = Some(joined(written.numbered, numbered));
            written.dated |= dated;
        }
        log.rows = follow_rows(&mut work, &rows, &mut Styles::new(sink, None), orders)?;
        if matches!(sink.policy(), RevisionPolicy::Tracked(_)) && log.rows > 0 && warnings.is_empty() {
            warnings.push(ROWS_UNTRACKED.to_owned());
        }
        let places: Vec<Vec<u32>> = log.list.iter().map(|p| place_below(&work, *p)).collect();
        let lists = put(sink, &found, element, &work, &written, &places)?;
        Ok(CopyFollowed {
            asked: asked.to_owned(),
            id: found.id,
            table: log.table,
            widths: log.widths,
            rows: log.rows,
            cells: log.cells,
            paragraphs: log.paragraphs,
            runs: log.runs,
            lists,
            missing,
        })
    }
}

/// The followed copy put in place where it changed; the ids of its paragraphs at `places`
/// (below it) then.
fn put(
    sink: &mut EditSink<'_, '_>,
    found: &Located,
    element: &Arc<ooxml_xml::Element>,
    work: &Work,
    written: &Written,
    places: &[Vec<u32>],
) -> Result<Vec<String>> {
    let new = work.build().ok_or_else(|| EditError::unbuilt(found.part.clone()))?;
    if new.as_ref() != element.as_ref() {
        let at = Found {
            part: found.part.clone(),
            path: found.path.clone(),
            id: found.id.clone(),
            element: Arc::clone(element),
        };
        write(sink, &at, &found.id, new, written)?;
    }
    let docx = sink.docx()?;
    Ok(places
        .iter()
        .filter_map(|below| {
            let path: Vec<u32> = found.path.iter().chain(below).copied().collect();
            docx.part(&found.part)?.ids.paragraph_at(&path).map(|p| p.id)
        })
        .collect())
}

impl Command<WordDoc> for FollowSample {
    type Out = Followed;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        self.located(doc, &docx).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Followed> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let Named {
            sample,
            copies,
            notes: mut warnings,
        } = self.located(sink.doc(), &docx)?;
        if self.source.is_none() {
            sink.read(block_key(&sample.part, &sample.id));
        }
        let renamed = stamp_document(&mut sink, &docx)?;
        let renamed_to = |id: &str| {
            renamed
                .iter()
                .find(|(old, _)| old == id)
                .map_or_else(|| id.to_owned(), |(_, new)| new.clone())
        };
        let sample_now = if self.source.is_none() {
            renamed_to(&sample.id)
        } else {
            sample.id.clone()
        };
        let mut out = Vec::new();
        for (asked, copy) in &copies {
            let id = renamed_to(&copy.id);
            out.push(self.follow_one(&mut sink, (asked, &id), &sample_now, &mut warnings)?);
        }
        crate::after::after_edit(&mut sink)?;
        Ok(Followed {
            sample: sample.id,
            sample_now,
            copies: out,
            renamed,
            warnings,
        })
    }
}
