//! Owns: which of the body's top-level blocks holds an id at a version
//! ([`BlocksOf::block_of`], docx-agent's `Document.block_of`, round 22 U1) and the top-level
//! blocks of a span ([`BlocksOf::blocks_between`]): renamed ids followed; a nested paragraph,
//! a table's row or cell, a range, a bookmark, a content control or a drawing gives the
//! block it is in; a footnote or endnote (`fn:`, `en:`) the block holding its reference; a
//! comment (`c:`, `c#`) the block its range starts in; a run, a hyperlink, a field or a
//! revision the block it is in; a section (`s:`) the block holding its break; a paragraph
//! in a block-level content control the control. `None` for what is not in the body.
//!
//! Must not: change the document, or resolve a `$name` ref (the host's: refused here).

use crate::WordVersion;
use ooxml_session::{Opened, SessionError};
use ooxml_word_edit::WordDoc;
use ooxml_word_model::address::resolve::section_paragraphs;
use ooxml_word_model::address::{scope_paragraphs, section, span_ends, Address, AddressError};
use ooxml_word_model::docx::drawings::all_drawings;
use ooxml_word_model::docx::{Docx, StoryPart};
use ooxml_word_model::ids::W;
use ooxml_xml::Element;

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

/// The blocks a Word version's ids name.
pub trait BlocksOf {
    /// The id of the body's top-level block (a paragraph or a table) holding `id`; `None`
    /// for what is not in the body (a header's paragraph, a note's own text, a note or
    /// comment no body block references, the body's last section) or a document that is
    /// not a Word document.
    ///
    /// # Errors
    /// When the version cannot be read.
    fn block_of(&self, id: &str) -> Result<BlockFound<Option<String>>, SessionError>;

    /// The ids of the body's top-level blocks from the one holding `first` through the one
    /// holding `last` ([`BlocksOf::block_of`]), in order.
    ///
    /// # Errors
    /// When the version cannot be read.
    fn blocks_between(&self, first: &str, last: &str) -> Result<BlockFound<Vec<String>>, SessionError>;
}

impl BlocksOf for Opened {
    fn block_of(&self, id: &str) -> Result<BlockFound<Option<String>>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(Ok(None));
        };
        let docx = word.docx().map_err(SessionError::format)?;
        let followed = word.ids().follow(id);
        Ok(block_in(word, &docx, &followed))
    }

    fn blocks_between(&self, first: &str, last: &str) -> Result<BlockFound<Vec<String>>, SessionError> {
        let Some(word) = self.word() else {
            return Ok(Ok(Vec::new()));
        };
        let docx = word.docx().map_err(SessionError::format)?;
        let mut ends = Vec::with_capacity(2);
        for given in [first, last] {
            match block_in(word, &docx, &word.ids().follow(given)) {
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
        Ok(scope_paragraphs(&docx, &Address::from(span))
            .map_err(BlockRefused::from)
            .map(|(part, indexes)| top_blocks(&docx, part, &indexes)))
    }
}

/// [`BlocksOf::block_of`] of `id` in `docx` (a reading of `word`).
fn block_in(word: &WordDoc, docx: &Docx, id: &str) -> BlockFound<Option<String>> {
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
    if let Some(found) = held(word, docx, body, id)? {
        return Ok(found);
    }
    if id.starts_with("s:") || id.starts_with("s@") {
        let section_id = section(docx, id)?;
        let paragraphs = section_paragraphs(docx, &section_id);
        let last = body.ids.paragraphs().len().checked_sub(1);
        // the body's last section's break is the body's own: no block holds it
        return Ok(match paragraphs.last() {
            Some(k) if Some(*k) != last => top_blocks(docx, body, &[*k]).into_iter().next(),
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
            .then(|| top_id(docx, body, &drawing.path))
            .flatten());
    }
    if (id.starts_with("p:") || id.starts_with("t:")) && body.ids.get(id).is_none() {
        // a block of another story: not in the body
        if docx.parts.iter().any(|p| p.ids.get(id).is_some()) {
            return Ok(None);
        }
    }
    let (part, indexes) = scope_paragraphs(docx, &Address::parse(id))?;
    if part.part != body.part {
        return Ok(None);
    }
    let Some(first) = indexes.first() else {
        return Err(BlockRefused::NotFound(format!("{id} names no paragraph or table")));
    };
    Ok(top_blocks(docx, part, &[*first]).into_iter().next())
}

/// The block of what a block holds named by the block (a run `P/r0`, a hyperlink
/// `hl:P/0`), or listed by the document (a field, a revision); `None` for an id of another
/// form.
fn held(word: &WordDoc, docx: &Docx, body: &StoryPart, id: &str) -> BlockFound<Option<Option<String>>> {
    if let Some((paragraph, run)) = id.rsplit_once("/r") {
        if !run.is_empty() && run.bytes().all(|b| b.is_ascii_digit()) && paragraph.starts_with("p") {
            return block_in(word, docx, paragraph).map(Some);
        }
    }
    if let Some(rest) = id.strip_prefix("hl:") {
        let paragraph = rest.rsplit_once('/').map_or(rest, |(p, _)| p);
        return block_in(word, docx, paragraph).map(Some);
    }
    if id.starts_with("fld:") {
        let fields = ooxml_word_edit::fields(word).map_err(|e| BlockRefused::Refused(e.to_string()))?;
        let field = fields
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| BlockRefused::NotFound(format!("no field {id}")))?;
        return match &field.paragraph {
            Some(paragraph) => block_in(word, docx, paragraph).map(Some),
            None => Ok(Some(None)),
        };
    }
    if id.starts_with("rev:") {
        let records = ooxml_word_model::docx::revisions::records(docx);
        let record = records
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| BlockRefused::NotFound(format!("no revision {id}")))?;
        return Ok(Some(
            (record.part == body.part)
                .then(|| top_id(docx, body, &record.path))
                .flatten(),
        ));
    }
    Ok(None)
}

/// A note's or comment's block: the body block holding its reference (a note) or where its
/// range starts (a comment, else its reference); `Some(None)` where the body references it
/// nowhere, `None` for an id of another form.
fn marked(docx: &Docx, body: &StoryPart, id: &str) -> BlockFound<Option<Option<String>>> {
    let (marks, number, part): (&[&str], &str, &str) = if let Some(n) = id.strip_prefix("fn:") {
        (&["footnoteReference"], n, "footnotes")
    } else if let Some(n) = id.strip_prefix("en:") {
        (&["endnoteReference"], n, "endnotes")
    } else if id.starts_with("c:") || id.starts_with("c#") {
        let comments = ooxml_word_model::docx::comments::comments(docx);
        let number = comments
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.w_id.clone())
            .ok_or_else(|| BlockRefused::NotFound(format!("no comment {id}")))?;
        let marks = ["commentRangeStart", "commentReference"];
        for mark in marks {
            let mut path = Vec::new();
            if let Some(found) = find_mark(body.root(), mark, &number, &mut path) {
                return Ok(Some(top_id(docx, body, &found)));
            }
        }
        return Ok(Some(None));
    } else {
        return Ok(None);
    };
    for mark in marks {
        let mut path = Vec::new();
        if let Some(found) = find_mark(body.root(), mark, number, &mut path) {
            return Ok(Some(top_id(docx, body, &found)));
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
fn top_blocks(docx: &Docx, story: &StoryPart, indexes: &[usize]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for &k in indexes {
        let Some(paragraph) = story.ids.paragraphs().get(k) else {
            continue;
        };
        let id = top_id(docx, story, &paragraph.path);
        if let Some(id) = id.filter(|id| out.last() != Some(id)) {
            out.push(id);
        }
    }
    out
}

/// The id of the top-level block at or above `place` in `story`: the body's child on its
/// way there -- a paragraph, a table, or a block-level content control (its `cc:` id).
fn top_id(docx: &Docx, story: &StoryPart, place: &[u32]) -> Option<String> {
    let top = place.get(..2)?;
    let node = ooxml_word_model::docx::at_path(story.root(), top)?;
    if node.name().is(Some(W), "sdt") {
        let controls = ooxml_word_model::docx::controls::controls(docx);
        return controls
            .into_iter()
            .find(|c| c.part == story.part && *c.path == *top)
            .map(|c| c.id);
    }
    story.ids.at(top).and_then(|entry| story.ids.id_of(entry))
}
