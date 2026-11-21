//! Owns: the comments a copy's markers name, copied as docx-agent's importer copies them
//! (`_comments`): each comment the source has given the next annotation id (its markers
//! following it), a marker of a comment the source lacks dropped (a reference with its
//! run where the run holds little else); each comment copied into this document's
//! comments part (made where missing) with its styles, lists and relationships brought
//! like a block's and its paragraphs given new ids, its author listed in the people part;
//! its `commentEx` entry made from the source's (done, and its parent where that was
//! copied too), and every comment then given its entries in the modern parts.
//!
//! As docx-agent's, a copied comment's drawings keep their numbers and ids, and its
//! bookmarks and controls theirs (`_comments` brings neither).
//!
//! Must not: write a comment part but through the copy's [`Space`].

use super::content::Bringer;
use crate::comments::{append, complete, ensure_part, last_para, part_of, Kind, REL_COMMENTS, REL_EXTENDED, W15};
use crate::names::{W, W14};
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// The markers a comment has in a block.
const MARKERS: [&str; 3] = ["commentRangeStart", "commentRangeEnd", "commentReference"];

/// The source's elements (comments, `commentEx` entries) by a key.
type ByKey = HashMap<String, Arc<Element>>;

/// What copying a block's comments gave.
#[derive(Default)]
pub(super) struct CommentsCopied {
    /// Each source comment and its copy (`comment#3` -> `comment#12`), in order.
    pub(super) copied: Vec<(String, String)>,
    /// The comments part, when one was written.
    pub(super) part: Option<String>,
}

/// The comments `elements` (copied blocks in `work`) mark, copied into this document's
/// comment parts, held in `stories`.
///
/// # Errors
/// What bringing its content refuses; a part that is not XML.
pub(super) fn bring(
    sink: &mut EditSink<'_, '_>,
    bringer: &mut Bringer<'_>,
    stories: &mut Space,
    (source, work): (&Docx, &mut Work),
    elements: &[Id],
) -> Result<CommentsCopied> {
    let markers: Vec<Id> = elements.iter().flat_map(|e| work.iter(*e, W, &MARKERS)).collect();
    if markers.is_empty() {
        return Ok(CommentsCopied::default());
    }
    let source_part = part_of(&source.package, &source.main, REL_COMMENTS);
    let by_id = match &source_part {
        Some(part) => comments_of(source, part)?,
        None => HashMap::new(),
    };
    let mapping = renumber(sink, work, &markers, &by_id)?;
    let Some(source_part) = source_part.filter(|_| !mapping.is_empty()) else {
        return Ok(CommentsCopied::default());
    };
    let main = sink.docx()?.main.clone();
    let part = ensure_part(sink, &main, Kind::Comments)?;
    let mut copier = Copier {
        parts: (source_part, part.clone()),
        seed: format!("{part}\0import"),
        paras: HashMap::new(),
    };
    let mut made: Vec<(Id, String)> = Vec::new();
    let mut copied = Vec::new();
    for (old, new) in &mapping {
        let comment = by_id.get(old).ok_or_else(|| EditError::Path(copier.parts.0.clone()))?;
        made.push(copier.copy(sink, bringer, stories, comment, new)?);
        copied.push((format!("comment#{old}"), format!("comment#{new}")));
    }
    let extended = source_extended(source)?;
    let entries = ensure_part(sink, &main, Kind::Extended)?;
    for (copy, old_last) in made {
        let attributes = copier.entry(stories, &extended, copy, &old_last)?;
        let attributes: Vec<(&str, &str)> = attributes.iter().map(|(n, v)| (*n, v.as_str())).collect();
        append(sink, stories, &entries, (W15, "w15", "commentEx"), &attributes)?;
    }
    complete(sink, stories, &main, &part)?;
    Ok(CommentsCopied {
        copied,
        part: Some(part),
    })
}

/// Each marker of a comment the source has given that comment's new id (the next
/// annotation id, once per comment), the others dropped; old id to new, in order.
fn renumber(
    sink: &mut EditSink<'_, '_>,
    work: &mut Work,
    markers: &[Id],
    by_id: &ByKey,
) -> Result<Vec<(String, String)>> {
    let mut mapping: Vec<(String, String)> = Vec::new();
    for &node in markers {
        let old = work.get(node, Some(W), "id").unwrap_or("").to_owned();
        if !by_id.contains_key(&old) {
            drop_marker(work, node);
            continue;
        }
        let new = match mapping.iter().find(|(o, _)| *o == old) {
            Some((_, new)) => new.clone(),
            None => {
                let (value, _) = sink.marker_id()?;
                mapping.push((old.clone(), value.to_string()));
                value.to_string()
            }
        };
        let attribute = work.qname(W, "id", "w");
        work.set(node, attribute, &new);
    }
    Ok(mapping)
}

/// Copies comments from the source's comments part into this document's.
struct Copier {
    /// The source's comments part and this document's.
    parts: (String, String),
    /// The sequence the copies' paragraph ids are drawn from.
    seed: String,
    /// Each copied paragraph's source paraId (upper case) and its new one.
    paras: HashMap<String, String>,
}

impl Copier {
    /// `comment` copied as comment `new`, its content brought and its paragraphs given new
    /// ids, appended to the comments part, its author listed; the copy and the source's
    /// last paraId.
    fn copy(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        bringer: &mut Bringer<'_>,
        stories: &mut Space,
        comment: &Arc<Element>,
        new: &str,
    ) -> Result<(Id, String)> {
        let (source_part, part) = (self.parts.0.clone(), self.parts.1.clone());
        let doc = sink.doc().clone();
        let into = stories
            .work(&doc, &part)?
            .ok_or_else(|| EditError::NoPart(part.clone()))?;
        let copy = into
            .adopt_copy(comment)
            .ok_or_else(|| EditError::Path(source_part.clone()))?;
        let attribute = into.qname(W, "id", "w");
        into.set(copy, attribute, new);
        let old_last = last_para(into, copy);
        let content: Vec<Id> = into
            .children(copy)
            .iter()
            .copied()
            .filter(|c| into.is_element(*c))
            .collect();
        bringer.content(sink, into, &content, (&source_part, &part))?;
        for paragraph in into.iter(copy, W, &["p"]) {
            let drawn = sink.draw(&self.seed, 2)?;
            let (Some(para), Some(text)) = (drawn.first(), drawn.get(1)) else {
                return Err(EditError::Path(part.clone()));
            };
            let into = stories.get_mut(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
            if let Some(raw) = into.get(paragraph, Some(W14), "paraId").filter(|r| !r.is_empty()) {
                self.paras.insert(raw.to_uppercase(), para.clone());
            }
            for (local, value) in [("paraId", para), ("textId", text)] {
                let attribute = into.qname(W14, local, "w14");
                into.set(paragraph, attribute, value);
            }
        }
        let into = stories.get_mut(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
        let root = into.root();
        into.append(root, copy);
        let author = into.get(copy, Some(W), "author").unwrap_or("").to_owned();
        if !author.is_empty() {
            sink.people(Some(&author), None)?;
        }
        Ok((copy, old_last))
    }

    /// The `commentEx` entry of the copy `copy`, from the source's entry of its last
    /// paragraph (`old_last`): its last paraId, its parent's copy where that was copied,
    /// and whether it is done (`0` where the source says nothing).
    fn entry(
        &self,
        stories: &Space,
        extended: &ByKey,
        copy: Id,
        old_last: &str,
    ) -> Result<Vec<(&'static str, String)>> {
        let into = stories
            .get(&self.parts.1)
            .ok_or_else(|| EditError::NoPart(self.parts.1.clone()))?;
        let entry = extended.get(&old_last.to_uppercase());
        let mut attributes = vec![("paraId", last_para(into, copy))];
        let parent = entry
            .and_then(|e| e.attribute(Some(W15), "paraIdParent"))
            .and_then(|p| self.paras.get(&p.to_uppercase()));
        if let Some(parent) = parent {
            attributes.push(("paraIdParent", parent.clone()));
        }
        let done = entry
            .and_then(|e| e.attribute(Some(W15), "done"))
            .filter(|d| !d.is_empty())
            .unwrap_or("0");
        attributes.push(("done", done.to_owned()));
        Ok(attributes)
    }
}

/// The source's comments by id (the last of an id).
fn comments_of(source: &Docx, part: &str) -> Result<ByKey> {
    let Some(tree) = source.package.tree(part)? else {
        return Ok(HashMap::new());
    };
    Ok(tree
        .root()
        .child_elements()
        .filter(|c| c.name().is(Some(W), "comment"))
        .map(|c| (c.attribute(Some(W), "id").unwrap_or("").to_owned(), Arc::clone(c)))
        .collect())
}

/// The source's `commentEx` entries by upper-case `paraId` (the last of one).
fn source_extended(source: &Docx) -> Result<ByKey> {
    let Some(part) = part_of(&source.package, &source.main, REL_EXTENDED) else {
        return Ok(HashMap::new());
    };
    let Some(tree) = source.package.tree(&part)? else {
        return Ok(HashMap::new());
    };
    Ok(tree
        .root()
        .child_elements()
        .filter(|e| e.name().is(Some(W15), "commentEx"))
        .map(|e| {
            let para = e.attribute(Some(W15), "paraId").unwrap_or("").to_uppercase();
            (para, Arc::clone(e))
        })
        .collect())
}

/// A marker of a comment the source lacks removed: a reference with its run where the run
/// holds at most one other child.
fn drop_marker(work: &mut Work, node: Id) {
    let parent = work.parent(node);
    if work.is(node, W, "commentReference") {
        if let Some(run) = parent.filter(|r| work.is(*r, W, "r") && work.children(*r).len() <= 2) {
            work.remove(run);
            return;
        }
    }
    work.remove(node);
}
