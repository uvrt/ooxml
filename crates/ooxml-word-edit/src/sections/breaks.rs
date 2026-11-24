//! Owns: [`InsertSectionBreak`] and [`RemoveSectionBreak`], as docx-agent's
//! `insert_section_break` and `remove_section_break` make them: a break is a paragraph of
//! its own holding a copy of the split section's `w:sectPr`, its properties the paragraph's
//! it is put before; the split section takes the break's kind and gives its header and
//! footer references to the new one (tracked: its kind a recorded change, its references
//! kept). A removed break's mark is deleted: the paragraph joins the one after it (an empty
//! one simply goes), what the section stated that the next does not is handed on, the rest
//! released; tracked, the mark is a deletion.
//!
//! Must not: write but through the sink.

use super::setup::{apply, kinds, record_change, SectionValues};
use super::SectionEdited;
use crate::blocks::resolve_block;
use crate::delete::{editable_opening, Opening};
use crate::doc::Key;
use crate::names::{W, W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::stamping::{stamp_document, w14_prefix};
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::sections::{section_id, sections};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// A section break between two blocks of the body.
#[derive(Debug, Clone)]
pub struct InsertSectionBreak {
    /// The block before it; or
    pub after: Option<String>,
    /// the block after it.
    pub before: Option<String>,
    /// How the section after it starts (`nextPage`...).
    pub kind: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// The break ending a section removed.
#[derive(Debug, Clone)]
pub struct RemoveSectionBreak {
    /// The section.
    pub section: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

const BLOCKS: [&str; 4] = ["p", "tbl", "sdt", "customXml"];
pub(crate) const REFERENCES: [&str; 2] = ["headerReference", "footerReference"];

/// The block after `element` among its siblings (none past the body's `w:sectPr`).
pub(crate) fn next_block(work: &Work, element: Id) -> Option<Id> {
    let mut node = work.next(element);
    while let Some(n) = node {
        if work.is_any(n, W, &BLOCKS) {
            return Some(n);
        }
        if work.is(n, W, "sectPr") {
            return None;
        }
        node = work.next(n);
    }
    None
}

/// The place of the element after the one at `path`, among its siblings.
pub(crate) fn next_sibling_path(root: &Arc<Element>, path: &[u32]) -> Option<Vec<u32>> {
    let (&index, above) = path.split_last()?;
    let parent = if above.is_empty() { root } else { at_path(root, above)? };
    parent
        .children()
        .iter()
        .enumerate()
        .skip(index as usize + 1)
        .find_map(|(k, n)| match n {
            Node::Element(_) => {
                let mut out = above.to_vec();
                out.push(u32::try_from(k).ok()?);
                Some(out)
            }
            _ => None,
        })
}

/// A break paragraph's properties: the paragraph's it is put before, without its section
/// break, list membership or revision records (docx-agent's `_break_properties`).
fn break_properties(work: &mut Work, template: Option<Id>) -> Id {
    let source = template.and_then(|t| work.find(t, W, "pPr"));
    let Some(source) = source else {
        return make(work, "pPr", &[]);
    };
    let properties = work.deep_copy(source);
    for child in work.children(properties).to_vec() {
        if work.is_any(child, W, &["sectPr", "pPrChange", "numPr"]) {
            work.remove(child);
        } else if work.is(child, W, "rPr") {
            for mark in work.children(child).to_vec() {
                if work.is_any(mark, W, &["ins", "del", "moveFrom", "moveTo", "rPrChange"]) {
                    work.remove(mark);
                }
            }
            if work.children(child).is_empty() {
                work.remove(child);
            }
        }
    }
    properties
}

/// A new paragraph with fresh ids drawn from `part`'s sequence.
fn fresh_paragraph(sink: &mut EditSink<'_, '_>, work: &mut Work, part: &str, prefix: &str) -> Result<(Id, String)> {
    let paragraph = make(work, "p", &[]);
    let drawn = sink.draw(part, 2)?;
    let (Some(para), Some(text)) = (drawn.first(), drawn.get(1)) else {
        return Err(EditError::no_element(part.to_owned()));
    };
    for (local, value) in [("paraId", para), ("textId", text)] {
        work.set(paragraph, Arc::new(QName::new(Some(prefix), local, Some(W14))), value);
    }
    Ok((paragraph, para.clone()))
}

impl InsertSectionBreak {
    fn anchor(&self, doc: &WordDoc, docx: &Docx) -> Result<Vec<u32>> {
        let anchor = match (&self.after, &self.before) {
            (Some(a), None) | (None, Some(a)) => a,
            _ => return Err(EditError::Refused("give exactly one of after= or before=".to_owned())),
        };
        let (part, block) = resolve_block(doc, docx, anchor)?;
        if part != docx.main {
            return Err(EditError::Refused(
                "a section break goes between blocks of the body".to_owned(),
            ));
        }
        let body = docx.body().ok_or_else(|| EditError::no_part(part.clone()))?;
        let above = block.path.split_last().map(|(_, a)| a.to_vec()).unwrap_or_default();
        let parent = at_path(body.root(), &above);
        if !parent.is_some_and(|p| p.name().is(Some(W), "body")) {
            return Err(EditError::Refused(format!(
                "{anchor} is not a block of the body (a section break cannot stand in a table, a content control or a note)"
            )));
        }
        Ok(block.path)
    }
}

impl Command<WordDoc> for InsertSectionBreak {
    type Out = SectionEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.anchor(doc, &*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<SectionEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl InsertSectionBreak {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<SectionEdited> {
        sink.read_everything();
        let docx = sink.docx()?;
        let anchor_path = self.anchor(sink.doc(), &docx)?;
        let renamed = stamp_document(&mut *sink, &docx)?;
        let docx = sink.docx()?;
        let main = docx.main.clone();
        let tree = sink
            .doc()
            .tree(&main)?
            .ok_or_else(|| EditError::no_part(main.clone()))?;
        let root = tree.root();
        let after = self.after.is_some();
        let sect_path = split_path(&docx, root, &anchor_path, after)?;
        let paths = [anchor_path.clone(), sect_path.clone()];
        let wanted: [&[u32]; 2] = [&anchor_path, &sect_path];
        let path_error = || EditError::no_element(main.clone());
        // the anchor, its neighbours and the section's end read whole; the body between sealed
        let (mut work, parent, stretch, found) =
            editable_opening(root, &paths, &wanted, Opening::Ends).ok_or_else(path_error)?;
        let (&anchor, &sect) = (
            found.first().ok_or_else(path_error)?,
            found.get(1).ok_or_else(path_error)?,
        );
        let following = if after { next_block(&work, anchor) } else { Some(anchor) };
        let template = following
            .filter(|f| work.is(*f, W, "p"))
            .or_else(|| Some(anchor).filter(|a| work.is(*a, W, "p")));
        let (paragraph, filler, created) = make_break(
            sink,
            &mut work,
            (anchor, sect, following, template),
            (&main, &prefix_of(root, &anchor_path), after),
        )?;
        let para = created.first().cloned().unwrap_or_default();
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let before: Vec<(String, Id)> = kinds(&work, sect)
            .into_iter()
            .map(|(k, id)| (k, work.deep_copy(id)))
            .collect();
        let values = SectionValues {
            start: Some(self.kind.clone()),
            ..SectionValues::default()
        };
        apply(&mut work, sect, &values, orders);
        let (numbered, dated) = track_break(&mut *sink, &mut work, (paragraph, filler, sect), &before, orders)?;
        let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
        let draws = crate::delete::draws_of(&nodes, numbered);
        let count = (stretch.1 - stretch.0) as usize + 1;
        sink.splice_stretch(
            &docx,
            &main,
            (&parent, stretch.0 as usize, count),
            nodes,
            &[Key::Annotations],
            draws,
        )?;
        sink.ensure_w14(&main, false, &[])?;
        if dated {
            sink.ensure_w14(&main, false, &[("w16du", W16DU)])?;
        }
        if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
            sink.people(Some(&tracking.author), Some(&tracking.author))?;
        }
        let id = section_id(&para);
        let mut made = vec![id.clone()];
        made.extend(created);
        Ok(SectionEdited {
            id: Some(id),
            changed: true,
            created: made,
            renamed,
            removed: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

/// The place of the `w:sectPr` of the section a break beside `anchor_path` splits: the one
/// the block after the break is in, else the last.
fn split_path(docx: &Docx, root: &Arc<Element>, anchor_path: &[u32], after: bool) -> Result<Vec<u32>> {
    let following = if after {
        next_sibling_path(root, anchor_path)
    } else {
        Some(anchor_path.to_vec())
    };
    let all = sections(docx);
    let split = following
        .as_ref()
        .and_then(|f| all.iter().find(|s| s.id == "s:body" || s.path.as_ref() > f.as_slice()))
        .or(all.last())
        .ok_or_else(|| EditError::Refused("the document has no section".to_owned()))?;
    Ok(split.path.to_vec())
}

/// The place of `path`'s paragraph's `w14` prefix.
fn prefix_of(root: &Arc<Element>, path: &[u32]) -> String {
    w14_prefix(root, path)
}

/// The break paragraph put beside `anchor` -- a copy of `sect` in properties like
/// `template`'s -- and, with no block after it, an empty paragraph after it: both nodes and
/// their ids.
fn make_break(
    sink: &mut EditSink<'_, '_>,
    work: &mut Work,
    (anchor, sect, following, template): (Id, Id, Option<Id>, Option<Id>),
    (main, prefix, after): (&str, &str, bool),
) -> Result<(Id, Option<Id>, Vec<String>)> {
    let new_sect = work.deep_copy(sect);
    for node in work.children(new_sect).to_vec() {
        if work.is(node, W, "sectPrChange") {
            work.remove(node);
        }
    }
    let (paragraph, para) = fresh_paragraph(&mut *sink, work, main, prefix)?;
    let properties = break_properties(work, template);
    work.append(properties, new_sect);
    work.append(paragraph, properties);
    if after {
        work.add_next(anchor, paragraph);
    } else {
        work.add_previous(anchor, paragraph);
    }
    let mut created = vec![format!("p:{para}")];
    let mut filler = None;
    if following.is_none() {
        let (made, id) = fresh_paragraph(&mut *sink, work, main, prefix)?;
        let filler_properties = break_properties(work, template);
        if !work.children(filler_properties).is_empty() {
            work.append(made, filler_properties);
        }
        work.add_next(paragraph, made);
        created.push(format!("p:{id}"));
        filler = Some(made);
    }
    Ok((paragraph, filler, created))
}

/// Tracked, the break's (and filler's) marks inserted and the split section's kind a
/// recorded change; untracked, the split section's references given to the new one.
fn track_break(
    sink: &mut EditSink<'_, '_>,
    work: &mut Work,
    (paragraph, filler, sect): (Id, Option<Id>, Id),
    before: &[(String, Id)],
    orders: &Orders,
) -> Result<(Option<crate::sink::Numbered>, bool)> {
    let RevisionPolicy::Tracked(tracking) = sink.policy().clone() else {
        for node in work.children(sect).to_vec() {
            if work.is_any(node, W, &REFERENCES) {
                work.remove(node);
            }
        }
        return Ok((None, false));
    };
    let ((), numbered, dated) = sink.stamped(tracking, |stamp| {
        crate::marks::mark(work, paragraph, "ins", stamp, orders);
        if let Some(filler) = filler {
            crate::marks::mark(work, filler, "ins", stamp, orders);
        }
        record_change(work, sect, before, stamp, orders)
    })?;
    Ok((Some(numbered), dated))
}

/// Word's order of the references in a `w:sectPr`: even, default header; even, default
/// footer; first header, first footer -- first among its children.
pub(crate) fn order_references(work: &mut Work, sect: Id) {
    let key = |work: &Work, node: Id| {
        let which = if work.is(node, W, "headerReference") { 0 } else { 1 };
        match (which, work.get(node, Some(W), "type").unwrap_or("default")) {
            (0, "even") => 0,
            (0, "default") => 1,
            (1, "even") => 2,
            (1, "default") => 3,
            (0, "first") => 4,
            (1, "first") => 5,
            _ => 9,
        }
    };
    let mut references: Vec<Id> = work
        .children(sect)
        .iter()
        .copied()
        .filter(|n| work.is_any(*n, W, &REFERENCES))
        .collect();
    references.sort_by_key(|n| key(work, *n));
    for (k, node) in references.into_iter().enumerate() {
        work.insert(sect, k, node);
    }
}

/// `sect`'s own reference of `which` and `kind`.
pub(crate) fn own_reference(work: &Work, sect: Id, which: &str, kind: &str) -> Option<Id> {
    let local = format!("{which}Reference");
    work.children(sect)
        .iter()
        .copied()
        .find(|n| work.is(*n, W, &local) && work.get(*n, Some(W), "type").unwrap_or("default") == kind)
}
