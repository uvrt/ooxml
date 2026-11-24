//! Owns: a document's first version as Word opens it, each data-bound plain text control
//! in the line showing its custom XML node. Word fills a bound control from its node on
//! opening and saves it so: in `controls-saved` (`crates/ooxml-oracle/word/observations/
//! e5-word.json`, Word 16.106 for Mac) a control whose content read `Stale cache` and whose
//! node holds `Bound from XML` is saved showing `Bound from XML`, its paragraph's text id
//! renewed. [`opened_store`] writes that, untracked, before the store's first version, so
//! the document's reads, edits and saves all start from what Word shows ([`RefreshBound`]).
//!
//! Only what was measured is refreshed: a plain text control in the line whose content is
//! runs of text and whose node holds text alone. A bound date, list or check box, a control around blocks, and a node holding
//! elements or no text are left as they are, as is every control whose text is its node's
//! (a document without such a control opens exactly as before, its bytes untouched).
//!
//! Must not: write but through the sink; write a node (the node is what is read).

use super::bound::Binding;
use super::fill::{drop_placeholder, fill_inline};
use super::{at_place, located, rewrite, InParagraph, Rewrite};
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Caller, Clock, Command, Store, StoreConfig, Txn};
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// The store of a document whose first version is `doc` as Word opens it: `doc` itself, or,
/// where a bound control's text is not its node's, `doc` with [`RefreshBound`] applied (no
/// history step: it is how the document opens, not an edit).
///
/// # Errors
/// What reading the document or refreshing a control refuses.
pub(crate) fn opened_store(doc: WordDoc, config: StoreConfig, clock: Arc<dyn Clock>) -> Result<Store<WordDoc>> {
    let docx = doc.docx()?;
    if stale(&doc, &docx).is_empty() {
        return Ok(Store::new(doc, config, clock));
    }
    let refreshing = Store::new(doc, config, Arc::clone(&clock));
    refreshing.run(&Caller::anonymous(), &RefreshBound)?;
    // a Word document's ids are drawn from its own sequences, never the store's, so the
    // refreshed version starts a store of its own with nothing to carry over
    let refreshed = refreshing.snapshot().doc().clone();
    Ok(Store::new(refreshed, config, clock))
}

/// Every stale bound plain text control in the line given its node's text, untracked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RefreshBound;

impl Command<WordDoc> for RefreshBound {
    type Out = ();
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<()> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        sink.read_everything();
        let docx = sink.docx()?;
        for (id, value) in stale(sink.doc(), &docx) {
            let docx = sink.docx()?;
            let found = located(&docx, &id)?;
            let at = InParagraph::above(&docx, &found.part, &found.path)
                .ok_or_else(|| EditError::no_element(found.part.clone()))?;
            let inside = found.path.get(at.path.len()..).unwrap_or_default().to_vec();
            let how = Rewrite {
                renew: true,
                ensure_w14: false,
            };
            rewrite(&mut sink, &at, how, |sink, work| {
                let sdt = at_place(work, &inside).ok_or_else(|| EditError::no_element(at.part.clone()))?;
                let wrote = fill_inline(sink, work, sdt, &value)?;
                drop_placeholder(work, sdt);
                Ok(((), wrote))
            })?;
        }
        Ok(())
    }
}

/// Each bound plain text control in the line whose text is not its node's (a node holding
/// text alone, a content of plain runs): its id and the node's text.
fn stale(doc: &WordDoc, docx: &Docx) -> Vec<(String, String)> {
    controls(docx)
        .into_iter()
        .filter_map(|control| {
            if control.kind() != "text" {
                return None;
            }
            let binding = Binding::of(&control.element)?;
            let part = docx.part(&control.part)?;
            if control.level(part) != "inline" {
                return None;
            }
            if !plain_runs(&control.element) {
                return None;
            }
            let value = binding.text(doc, docx).ok().flatten()?;
            (!value.is_empty() && value != control.text(part)).then(|| (control.id.clone(), value))
        })
        .collect()
}

/// Whether the content of `sdt` is runs of text alone (`w:r` holding `w:rPr` and `w:t`), as
/// measured: a content holding a revision, a comment's or bookmark's mark or any other
/// object is left as it is.
fn plain_runs(sdt: &Element) -> bool {
    let is = |e: &Element, local: &str| e.name().is(Some(W), local);
    let Some(content) = sdt.child_elements().find(|e| is(e, "sdtContent")) else {
        return false;
    };
    let only_elements = |e: &Element| {
        e.children()
            .iter()
            .all(|n| matches!(n, Node::Element(_)) || n.as_text().is_some_and(|t| t.trim().is_empty()))
    };
    only_elements(content)
        && content
            .child_elements()
            .all(|run| is(run, "r") && only_elements(run) && run.child_elements().all(|c| is(c, "rPr") || is(c, "t")))
}
