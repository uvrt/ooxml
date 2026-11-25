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
use crate::spacing::{mark_places, marked_places, space_paragraph, table_of, Context};
use crate::work::{scope_above, Id, Work};
use crate::{Change, Result};
use ooxml_word_model::docx::{at_path, StoryPart};
use ooxml_word_model::ids::Entry;
use ooxml_word_model::typography::{has_marks, Cascade};
use ooxml_xml::Element;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The relationship type of the styles part.
const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";

/// The paragraphs of `work` by their stable place: their `w14:paraId`, or for the root
/// (read from `at` in its part) without one, `@` and its path -- those of a text box's
/// fallback left out (a copy of its choice's, synced when the edit ends). A paragraph
/// without an id is one a table filled in one pass writes unstamped; nothing moves it
/// within the edit.
fn paragraphs(work: &Work, at: Option<&[u32]>) -> Vec<(Id, String)> {
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
        .filter_map(|n| match work.get(n, Some(W14), "paraId") {
            Some(id) => Some((n, id.to_owned())),
            None => at.filter(|_| n == root).map(|path| (n, path_place(path))),
        })
        .collect()
}

/// The place of the paragraph at `path` that has no id.
fn path_place(path: &[u32]) -> String {
    let steps: Vec<String> = path.iter().map(u32::to_string).collect();
    format!("@{}", steps.join("/"))
}

impl EditSink<'_, '_> {
    /// Mark written in `work` (read from `at` in `part`, where it is a paragraph) what the edit wrote in its paragraphs
    /// earlier, so a write there carries it along.
    pub(crate) fn load_written(&mut self, part: &str, at: Option<&[u32]>, work: &mut Work) {
        if !self.written.in_part(part) {
            return;
        }
        for (paragraph, id) in paragraphs(work, at) {
            if let Some(places) = self.written.of(part, &id).cloned() {
                mark_places(work, paragraph, &places);
                self.written.loaded(part, &id);
            }
        }
    }

    /// Record what `work` (about to replace its place in `part`) marks written, in place of
    /// what was recorded of its paragraphs that hold marks or were loaded into it; a
    /// paragraph recorded but not loaded (a neighbour the work holds) keeps its record.
    pub(crate) fn keep_written(&mut self, part: &str, at: Option<&[u32]>, work: &Work) {
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
        for (paragraph, id) in paragraphs(work, at) {
            let loaded = recorded && self.written.take_loaded(part, &id);
            if holding.contains(&paragraph) || loaded {
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
        if self.written.is_empty() || self.doc().as_given() {
            return Ok(0);
        }
        let docx = self.docx()?;
        let mut cascade: Option<Cascade> = None;
        let mut made = 0;
        let mut changed: Vec<(String, Vec<u32>)> = Vec::new();
        let recorded: Vec<(String, String)> = self
            .written
            .paragraphs()
            .map(|(part, id)| (part.to_owned(), id.to_owned()))
            .collect();
        for (part, raw) in recorded {
            let Some(story) = docx.part(&part) else { continue };
            let Some(found) = locate(story, &raw) else { continue };
            if !has_marks(&found.element.text_content()) {
                continue;
            }
            let Some(places) = self.written.of(&part, &raw).cloned() else {
                continue;
            };
            // the tree as spaced so far (a paragraph holding a text box spaced before it):
            // spacing moves nothing, so the place found stays
            let Some(tree) = self.doc().tree(&part)? else { continue };
            let root = tree.root();
            let Some(element) = at_path(root, &found.path) else {
                continue;
            };
            let cascade = match &mut cascade {
                Some(c) => &*c,
                None => &*cascade.insert(Cascade::new(docx.styles(), self.doc_defaults(&docx)?)),
            };
            let context = Context {
                cascade,
                table: table_of(root, &found.path),
            };
            let mut work = Work::of(element, scope_above(root, &found.path));
            let spaced = space_paragraph(&mut work, &places, &context);
            if spaced == 0 {
                continue;
            }
            made += spaced;
            let new = work.build().ok_or_else(|| crate::EditError::unbuilt(part.clone()))?;
            self.replace(&part, &found.path, new, vec![found.key])?;
            changed.push((part, found.path));
        }
        // a text box's: its fallback holds the same (docx-agent syncs the boxes around each
        // paragraph it spaced, innermost first)
        for (part, path) in changed {
            for k in (0..path.len()).rev() {
                let Some(tree) = self.doc().tree(&part)? else { break };
                let above = path.get(..k).unwrap_or_default();
                let Some(holder) = at_path(tree.root(), above).filter(|h| h.name().is(Some(MC), "AlternateContent"))
                else {
                    continue;
                };
                let holder = Arc::clone(holder);
                crate::textbox::sync_holder(self, &part, above, &holder)?;
            }
        }
        if made > 0 {
            self.write(Change::Spaced(made))?;
        }
        Ok(made)
    }

    /// The styles part's `w:docDefaults/w:rPrDefault/w:rPr`; the spacing depends on that
    /// part (read).
    fn doc_defaults(&mut self, docx: &ooxml_word_model::docx::Docx) -> Result<Option<Arc<Element>>> {
        let Some(part) = ooxml_word_model::parts::related(self.doc().package(), &docx.main, REL_STYLES)? else {
            return Ok(None);
        };
        self.read(crate::doc::Key::part(part.as_str()));
        Ok(self.doc().tree(&part)?.and_then(|tree| {
            tree.root()
                .find(Some(W), "docDefaults")
                .and_then(|d| d.find(Some(W), "rPrDefault"))
                .and_then(|d| d.find(Some(W), "rPr"))
                .cloned()
        }))
    }
}

/// A recorded paragraph found in the working version: its path, its element and the key
/// it is written under.
struct Located {
    path: Vec<u32>,
    element: Arc<Element>,
    key: crate::doc::Key,
}

/// The paragraph of `story` recorded as `raw` (a `w14:paraId`, or `@` and a path): through
/// the part's id index, else -- a paragraph in a text box -- the first with that paraId in
/// the part outside a fallback, written under its nearest indexed ancestor's key (else the
/// part's); one recorded by path, the paragraph there.
fn locate(story: &StoryPart, raw: &str) -> Option<Located> {
    let found = match raw.strip_prefix('@') {
        Some(path) => {
            let path: Vec<u32> = path.split('/').filter_map(|k| k.parse().ok()).collect();
            story.ids.paragraph_at(&path)
        }
        None => {
            let prefix = if story.story == "body" {
                String::new()
            } else {
                format!("{}/", story.story)
            };
            match story.ids.get(&format!("{prefix}p:{raw}")) {
                Some(Entry::Paragraph(n)) => story.ids.paragraphs().get(n),
                _ => None,
            }
        }
    };
    if let Some(found) = found {
        return Some(Located {
            path: found.path.to_vec(),
            element: found.element,
            key: block_key(&story.part, &found.id),
        });
    }
    let root = story.tree.root();
    let (path, element) = match raw.strip_prefix('@') {
        Some(path) => {
            let path: Vec<u32> = path.split('/').filter_map(|k| k.parse().ok()).collect();
            let element = at_path(root, &path).filter(|e| e.name().is(Some(W), "p"))?;
            (path, Arc::clone(element))
        }
        None => by_para_id(root, raw, &mut Vec::new())?,
    };
    let key = (0..path.len())
        .rev()
        .find_map(|k| story.ids.paragraph_at(path.get(..k)?))
        .map_or_else(
            || crate::doc::Key::part(story.part.as_str()),
            |p| block_key(&story.part, &p.id),
        );
    Some(Located { path, element, key })
}

/// The first `w:p` below `element` whose `w14:paraId` is `raw`, outside `mc:Fallback`.
fn by_para_id(element: &Arc<Element>, raw: &str, path: &mut Vec<u32>) -> Option<(Vec<u32>, Arc<Element>)> {
    for (k, node) in element.children().iter().enumerate() {
        let Some(child) = node.as_element() else { continue };
        if child.name().is(Some(MC), "Fallback") {
            continue;
        }
        path.push(u32::try_from(k).ok()?);
        if child.name().is(Some(W), "p") && child.attribute(Some(W14), "paraId") == Some(raw) {
            return Some((path.clone(), Arc::clone(child)));
        }
        if let Some(found) = by_para_id(child, raw, path) {
            return Some(found);
        }
        path.pop();
    }
    None
}
