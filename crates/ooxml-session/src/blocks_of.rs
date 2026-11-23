//! Owns: which of the body's top-level blocks holds an id at a version
//! ([`Opened::block_of`], docx-agent's `Document.block_of`, round 22 U1) and the top-level
//! blocks of a span ([`Opened::blocks_between`]): renamed ids followed; a nested paragraph,
//! a table's row or cell, a range, a bookmark, a content control or a drawing gives the
//! block it is in; a footnote or endnote (`fn:`, `en:`) the block holding its reference; a
//! comment (`c:`) the block its range starts in; a section (`s:`) the block holding its
//! break. `None` for what is not in the body.
//!
//! Must not: change the document, or resolve a `$name` ref (the host's: refused here).

use crate::{Opened, SessionError};
use ooxml_word_model::address::resolve::section_paragraphs;
use ooxml_word_model::address::{scope_paragraphs, section, span_ends, AddressError};
use ooxml_word_model::docx::drawings::all_drawings;
use ooxml_word_model::docx::{Docx, StoryPart};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;
use std::sync::Arc;

/// Why an id names no block: one the document lacks (`NotFound`, Python's `KeyError`), or
/// one this does not take (`Refused`: a ref, a span, an end outside the body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockRefused {
    /// The document has no such id.
    NotFound(String),
    /// An id of a form this does not take here.
    Refused(String),
}

impl From<AddressError> for BlockRefused {
    fn from(error: AddressError) -> Self {
        if error.code == "not_found" {
            BlockRefused::NotFound(error.message)
        } else {
            BlockRefused::Refused(error.message)
        }
    }
}

/// A block's answer, or why the id names none.
pub type BlockFound<T> = Result<T, BlockRefused>;

impl Opened {
    /// The id of the body's top-level block (a paragraph or a table) holding `id`; `None`
    /// for what is not in the body (a header's paragraph, a note's own text, a note or
    /// comment no body block references, the body's last section) or a document that is
    /// not a Word document.
    ///
    /// # Errors
    /// When the version cannot be read.
    pub fn block_of(&self, id: &str) -> Result<BlockFound<Option<String>>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(Ok(None));
        };
        let docx = word.docx()?;
        let followed = word.ids().follow(id);
        Ok(block_in(&docx, &followed))
    }

    /// The ids of the body's top-level blocks from the one holding `first` through the one
    /// holding `last` ([`Opened::block_of`]), in order.
    ///
    /// # Errors
    /// When the version cannot be read.
    pub fn blocks_between(&self, first: &str, last: &str) -> Result<BlockFound<Vec<String>>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(Ok(Vec::new()));
        };
        let docx = word.docx()?;
        let mut ends = Vec::with_capacity(2);
        for given in [first, last] {
            match block_in(&docx, &word.ids().follow(given)) {
                Ok(Some(block)) => ends.push(block),
                Ok(None) => {
                    return Ok(Err(BlockRefused::Refused(format!(
                        "{given} is not in the body: blocks_between spans the body's blocks"
                    ))))
                }
                Err(refused) => return Ok(Err(refused)),
            }
        }
        let span = ends.join("..");
        Ok(scope_paragraphs(&docx, &span)
            .map_err(BlockRefused::from)
            .map(|(part, indexes)| top_blocks(part, &indexes)))
    }
}

/// [`Opened::block_of`] of `id` in `docx`.
fn block_in(docx: &Docx, id: &str) -> BlockFound<Option<String>> {
    if id.starts_with('$') {
        return Err(BlockRefused::Refused(format!(
            "{id} is a ref: refs live in the tools' session, which resolves one first"
        )));
    }
    if matches!(span_ends(id), Ok(Some(_))) {
        return Err(BlockRefused::Refused(format!(
            "{id:?} is a span of blocks: blocks_between takes its two ends"
        )));
    }
    let Some(body) = docx.body() else {
        return Ok(None);
    };
    if docx.story(id).is_some() {
        return Ok(None);
    }
    if let Some(found) = marked(docx, body, id)? {
        return Ok(found);
    }
    if id.starts_with("s:") || id.starts_with("s@") {
        let section_id = section(docx, id)?;
        let paragraphs = section_paragraphs(docx, &section_id);
        let last = body.ids.paragraphs().len().checked_sub(1);
        // the body's last section's break is the body's own: no block holds it
        return Ok(match paragraphs.last() {
            Some(k) if Some(*k) != last => top_blocks(body, &[*k]).into_iter().next(),
            _ => None,
        });
    }
    if id.starts_with("d:") {
        let drawings = all_drawings(docx);
        let drawing = drawings
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| BlockRefused::NotFound(format!("no drawing {id}")))?;
        return Ok((drawing.part == body.part)
            .then(|| top_place(body, &drawing.path).and_then(|top| block_id(body, top)))
            .flatten());
    }
    let (part, indexes) = scope_paragraphs(docx, id)?;
    if part.part != body.part {
        return Ok(None);
    }
    Ok(top_blocks(part, indexes.get(..1).unwrap_or_default())
        .into_iter()
        .next())
}

/// A note's or comment's block: the body block holding its reference (a note) or where its
/// range starts (a comment, else its reference); `Some(None)` where the body references it
/// nowhere, `None` for an id of another form.
fn marked(docx: &Docx, body: &StoryPart, id: &str) -> BlockFound<Option<Option<String>>> {
    let (marks, number, part): (&[&str], &str, &str) = if let Some(n) = id.strip_prefix("fn:") {
        (&["footnoteReference"], n, "footnotes")
    } else if let Some(n) = id.strip_prefix("en:") {
        (&["endnoteReference"], n, "endnotes")
    } else if let Some(n) = id.strip_prefix("c:") {
        (&["commentRangeStart", "commentReference"], n, "comments")
    } else {
        return Ok(None);
    };
    for mark in marks {
        let mut path = Vec::new();
        if let Some(found) = find_mark(body.root(), mark, number, &mut path) {
            return Ok(Some(top_place(body, &found).and_then(|top| block_id(body, top))));
        }
    }
    let known = docx.parts.iter().any(|p| p.story == part && has_note(p.root(), number));
    if known {
        Ok(Some(None))
    } else {
        Err(BlockRefused::NotFound(format!("no {} {id}", &part[..part.len() - 1])))
    }
}

/// Whether a notes or comments part has the entry numbered `number`.
fn has_note(root: &Element, number: &str) -> bool {
    root.children()
        .iter()
        .filter_map(|c| c.as_element())
        .any(|e| e.attribute(Some(W), "id") == Some(number))
}

/// The place of the first `w:<mark>` numbered `number` under `node`, `path` its place.
fn find_mark(node: &Element, mark: &str, number: &str, path: &mut Vec<u32>) -> Option<Vec<u32>> {
    for (k, child) in node.children().iter().enumerate() {
        let Some(element) = child.as_element() else { continue };
        path.push(u32::try_from(k).ok()?);
        if element.name().namespace() == Some(W)
            && element.name().local() == mark
            && element.attribute(Some(W), "id") == Some(number)
        {
            return Some(path.clone());
        }
        if let Some(found) = find_mark(element, mark, number, path) {
            return Some(found);
        }
        path.pop();
    }
    None
}

/// The top-level blocks the paragraphs `indexes` of `story` are in, each once, in order.
fn top_blocks(story: &StoryPart, indexes: &[usize]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for &k in indexes {
        let Some(paragraph) = story.ids.paragraphs().get(k) else {
            continue;
        };
        let id = top_place(story, &paragraph.path).and_then(|top| block_id(story, top));
        if let Some(id) = id.filter(|id| out.last() != Some(id)) {
            out.push(id);
        }
    }
    out
}

/// The place of the top-level block at or above `place`: the first paragraph or table on
/// its way below the body (past block-level content controls).
fn top_place<'p>(story: &StoryPart, place: &'p [u32]) -> Option<&'p [u32]> {
    let mut node: &Arc<Element> = story.root();
    for (end, &k) in place.iter().enumerate() {
        node = node.children().get(k as usize)?.as_element()?;
        let local = node.name().local();
        if end >= 1 && node.name().namespace() == Some(W) && (local == "p" || local == "tbl") {
            return place.get(..=end);
        }
    }
    None
}

/// The id of the paragraph or table at `place` in `story`.
fn block_id(story: &StoryPart, place: &[u32]) -> Option<String> {
    story.ids.at(place).and_then(|entry| story.ids.id_of(entry))
}
