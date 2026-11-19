//! Owns: the sink's record of the text an edit wrote ([`crate::spacing::Written`]) -- taken
//! from a work as it is handed back ([`EditSink::keep_written`]), put on a later work of
//! the same paragraphs ([`EditSink::load_written`]) -- and the spacing the written text's
//! language wants, applied when the edit ends ([`EditSink::space_written`]).
//!
//! Must not: change what the edit wrote but for that spacing, or look at text the edit did
//! not write.

use super::EditSink;
use crate::names::{MC, W, W14};
use crate::set_text::block_key;
use crate::spacing::{mark_places, marked_places, space_paragraph, Context};
use crate::work::{scope_above, Id, Work};
use crate::Result;
use ooxml_word_model::docx::at_path;
use ooxml_word_model::ids::Entry;
use ooxml_word_model::typography::{has_marks, Cascade};
use ooxml_xml::Element;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The relationship type of the styles part.
const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";

/// The paragraphs of `work` with their `w14:paraId`, those of a text box's fallback left
/// out (a copy of its choice's, synced when the edit ends).
fn paragraphs(work: &Work) -> Vec<(Id, String)> {
    let root = work.root();
    std::iter::once(root)
        .chain(work.descendants(root))
        .filter(|n| work.is(*n, W, "p"))
        .filter(|n| {
            let mut at = work.parent(*n);
            while let Some(a) = at {
                if work.is(a, MC, "Fallback") {
                    return false;
                }
                at = work.parent(a);
            }
            true
        })
        .filter_map(|n| work.get(n, Some(W14), "paraId").map(|id| (n, id.to_owned())))
        .collect()
}

impl EditSink<'_, '_> {
    /// Mark written in `work` (read from `part`) what the edit wrote in its paragraphs
    /// earlier, so a write there carries it along.
    pub(crate) fn load_written(&self, part: &str, work: &mut Work) {
        if !self.written.in_part(part) {
            return;
        }
        for (paragraph, id) in paragraphs(work) {
            if let Some(places) = self.written.of(part, &id).cloned() {
                mark_places(work, paragraph, &places);
            }
        }
    }

    /// Record what `work` (about to replace its place in `part`) marks written, in place of
    /// what was recorded of its paragraphs.
    pub(crate) fn keep_written(&mut self, part: &str, work: &Work) {
        let recorded = self.written.in_part(part);
        if !work.has_marks() && !recorded {
            return;
        }
        // the paragraphs holding a mark, and those recorded before
        let holding: BTreeSet<Id> = work
            .marked_nodes()
            .filter_map(|node| {
                let mut at = work.parent(node);
                while let Some(a) = at.filter(|a| !work.is(*a, W, "p")) {
                    at = work.parent(a);
                }
                at
            })
            .collect();
        for (paragraph, id) in paragraphs(work) {
            if holding.contains(&paragraph) || (recorded && self.written.of(part, &id).is_some()) {
                self.written.set(part, &id, marked_places(work, paragraph));
            }
        }
    }

    /// Give the text the edit wrote the spacing its language wants (docx-agent's
    /// `_space_written`, when the edit ends): the number of spaces written or made U+00A0.
    ///
    /// # Errors
    /// When a part is not XML.
    pub(crate) fn space_written(&mut self) -> Result<usize> {
        if self.written.is_empty() {
            return Ok(0);
        }
        let docx = self.docx()?;
        let mut cascade: Option<Cascade> = None;
        let mut made = 0;
        let recorded: Vec<(String, String)> = self
            .written
            .paragraphs()
            .map(|(part, id)| (part.to_owned(), id.to_owned()))
            .collect();
        for (part, raw) in recorded {
            let Some(story) = docx.part(&part) else { continue };
            let prefix = if story.story == "body" {
                String::new()
            } else {
                format!("{}/", story.story)
            };
            let id = format!("{prefix}p:{raw}");
            let Some(Entry::Paragraph(n)) = story.ids.get(&id) else {
                continue;
            };
            let Some(found) = story.ids.paragraphs().get(n) else {
                continue;
            };
            if !has_marks(&found.element.text_content()) {
                continue;
            }
            let Some(places) = self.written.of(&part, &raw).cloned() else {
                continue;
            };
            let root = story.tree.root();
            let cascade = match &mut cascade {
                Some(c) => &*c,
                None => &*cascade.insert(Cascade::new(docx.styles(), self.doc_defaults(&docx)?)),
            };
            let context = Context {
                cascade,
                table: table_of(root, &found.path),
            };
            let mut work = Work::of(&found.element, scope_above(root, &found.path));
            let spaced = space_paragraph(&mut work, &places, &context);
            if spaced == 0 {
                continue;
            }
            made += spaced;
            let new = work.build().ok_or_else(|| crate::EditError::Path(part.clone()))?;
            self.replace(&part, &found.path, new, vec![block_key(&part, &id)])?;
        }
        Ok(made)
    }

    /// The styles part's `w:docDefaults/w:rPrDefault/w:rPr`; the spacing depends on that
    /// part (read).
    fn doc_defaults(&mut self, docx: &ooxml_word_model::docx::Docx) -> Result<Option<Arc<Element>>> {
        let Some(part) = ooxml_word_model::parts::related(self.doc().package(), &docx.main, REL_STYLES)? else {
            return Ok(None);
        };
        self.read(crate::doc::Key::Part(Arc::from(part.as_str())));
        Ok(self.doc().tree(&part)?.and_then(|tree| {
            tree.root()
                .find(Some(W), "docDefaults")
                .and_then(|d| d.find(Some(W), "rPrDefault"))
                .and_then(|d| d.find(Some(W), "rPr"))
                .cloned()
        }))
    }
}

/// `Some` with the `w:tblPr` of the innermost table above the element at `path` of `root`
/// when there is one.
fn table_of(root: &Arc<Element>, path: &[u32]) -> Option<Option<Arc<Element>>> {
    let mut table = None;
    for k in 0..path.len() {
        let at = at_path(root, path.get(..k).unwrap_or_default())?;
        if at.name().is(Some(W), "tbl") {
            table = Some(at.find(Some(W), "tblPr").cloned());
        }
    }
    table
}
