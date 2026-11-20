//! Owns: footnotes and endnotes as docx-agent's `edit/notes.py` edits them --
//! [`InsertNote`], [`DeleteNote`], [`MoveNote`] and [`EditNote`] -- and what they share:
//! the notes parts (a document's first note brings both, with their separators and the
//! settings naming them: `new_notes_part`), a note by id (`fn:2`, `en:1`), its reference,
//! and the place a reference goes (`_place`: a position, or a range's end, in the body).
//!
//! Must not: write but through the sink; a note's number is one above the largest in its
//! part, and its reference may stand anywhere in the body, so these edits read the notes
//! part and the reference's paragraph, and the whole body where they look a reference up.

mod change;
mod edit;
mod insert;
mod moved;

pub use change::{DeleteNote, MoveNote};
pub use edit::EditNote;
pub use insert::InsertNote;

use crate::names::{W, W14};
use crate::review::space::Space;
use crate::review::xml::{insert_in_order, make};
use crate::set_text::Found;
use crate::sink::EditSink;
use crate::text::atoms;
use crate::text_range::{range, range_id, Span};
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::Limits;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::parts::{related, REL_SETTINGS};
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// What a note edit did, as docx-agent's `EditResult` says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoteEdited {
    /// The note it names (`fn:2`), if any.
    pub id: Option<String>,
    /// Ids it made (the note and its paragraphs).
    pub created: Vec<String>,
    /// Ids it removed.
    pub removed: Vec<String>,
    /// Ids renamed, old to new, in the order made.
    pub renamed: Vec<(String, String)>,
}

/// A footnote or an endnote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    /// A footnote (`fn:`).
    Footnote,
    /// An endnote (`en:`).
    Endnote,
}

impl NoteKind {
    /// Word's name: `footnote`, `endnote`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            NoteKind::Footnote => "footnote",
            NoteKind::Endnote => "endnote",
        }
    }

    pub(crate) fn prefix(self) -> &'static str {
        match self {
            NoteKind::Footnote => "fn",
            NoteKind::Endnote => "en",
        }
    }

    fn other(self) -> NoteKind {
        match self {
            NoteKind::Footnote => NoteKind::Endnote,
            NoteKind::Endnote => NoteKind::Footnote,
        }
    }

    /// The kind a note id (`fn:2`, `en:1`) is of.
    fn of(id: &str) -> NoteKind {
        if id.starts_with("fn:") {
            NoteKind::Footnote
        } else {
            NoteKind::Endnote
        }
    }

    pub(crate) fn rel(self) -> String {
        format!(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/{}s",
            self.name()
        )
    }

    /// The text and reference styles' names.
    fn styles(self) -> (&'static str, &'static str) {
        match self {
            NoteKind::Footnote => ("footnote text", "footnote reference"),
            NoteKind::Endnote => ("endnote text", "endnote reference"),
        }
    }
}

/// The notes part of `kind`, if the document has one.
fn notes_part(doc: &WordDoc, kind: NoteKind) -> Result<Option<String>> {
    let docx = doc.docx()?;
    let found = related(doc.package(), &docx.main, &kind.rel())?;
    Ok(found.filter(|p| doc.package().has_part(p)))
}

/// The notes part of `kind`, made (with the other kind's when it lacks one too) when the
/// document has none (docx-agent's `_notes_part(create=True)`).
pub(crate) fn ensure_notes_part(sink: &mut EditSink<'_, '_>, kind: NoteKind) -> Result<String> {
    if let Some(part) = notes_part(sink.doc(), kind)? {
        return Ok(part);
    }
    let part = new_notes_part(sink, kind)?;
    if notes_part(sink.doc(), kind.other())?.is_none() {
        new_notes_part(sink, kind.other())?;
    }
    Ok(part)
}

/// A notes part with its two separators, as Word writes it with a document's first note:
/// its relationship, content type and settings entry (docx-agent's `new_notes_part`).
fn new_notes_part(sink: &mut EditSink<'_, '_>, kind: NoteKind) -> Result<String> {
    let name = kind.name();
    let plural = format!("{name}s");
    let package = sink.doc().package().clone();
    let docx = sink.docx()?;
    let main = docx.main.clone();
    let part = crate::new_part::unused_name(
        &package,
        &format!("word/{plural}.xml"),
        &format!("word/{plural}{{n}}.xml"),
    );
    let paragraph = |what: &str| {
        format!(
            r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:{what}/></w:r></w:p>"#
        )
    };
    let data = format!(
        r#"<w:{plural} xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w14="{W14}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="w14"><w:{name} w:type="separator" w:id="-1">{}</w:{name}><w:{name} w:type="continuationSeparator" w:id="0">{}</w:{name}></w:{plural}>"#,
        paragraph("separator"),
        paragraph("continuationSeparator")
    );
    let tree =
        ooxml_xml::parse(data.as_bytes(), &Limits::default().xml).map_err(|e| EditError::Refused(e.to_string()))?;
    let mut work = Work::of(tree.root(), Vec::new());
    let root = work.root();
    for paragraph in work.iter(root, W, &["p"]) {
        stamp_paragraph(sink, &mut work, paragraph, &part)?;
    }
    let root = work.build().ok_or_else(|| EditError::Path(part.clone()))?;
    let content_type = format!("application/vnd.openxmlformats-officedocument.wordprocessingml.{plural}+xml");
    let made = crate::new_part::add_part(&package, &main, &part, (*root).clone(), &content_type, &kind.rel())?;
    let rels = ooxml_opc::name::rels_for(&main);
    for written in [part.as_str(), ooxml_opc::CONTENT_TYPES_PART, rels.as_str()] {
        if let Some(tree) = made.tree(written)? {
            sink.put(written, tree)?;
        }
    }
    if let Some(settings) = related(sink.doc().package(), &main, REL_SETTINGS)? {
        let held = sink.doc().clone();
        let orders = held.shared().facts.orders();
        let mut space = Space::default();
        if let Some(work) = space.work(&held, &settings)? {
            let root = work.root();
            let node = match work.find(root, W, &format!("{name}Pr")) {
                Some(node) => node,
                None => {
                    let node = make(work, &format!("{name}Pr"), &[]);
                    insert_in_order(work, root, node, orders);
                    node
                }
            };
            if work.find(node, W, name).is_none() {
                for value in ["-1", "0"] {
                    let child = make(work, name, &[("id", value)]);
                    insert_in_order(work, node, child, orders);
                }
            }
            space.mark_dirty(&settings);
        }
        space.finish(sink)?;
    }
    Ok(part)
}

/// Give `paragraph` a paraId and textId drawn from `part`'s sequence (docx-agent's
/// `_ids.generate` and `stamp`).
fn stamp_paragraph(sink: &mut EditSink<'_, '_>, work: &mut Work, paragraph: Id, part: &str) -> Result<()> {
    let drawn = sink.draw(part, 2)?;
    let (Some(para), Some(text)) = (drawn.first(), drawn.get(1)) else {
        return Err(EditError::Path(part.to_owned()));
    };
    for (local, value) in [("paraId", para), ("textId", text)] {
        work.set(paragraph, Arc::new(QName::new(Some("w14"), local, Some(W14))), value);
    }
    Ok(())
}

/// The notes part holding `id` and its note's place there.
fn note_element(doc: &WordDoc, docx: &Docx, id: &str) -> Result<(String, Vec<u32>, Arc<Element>)> {
    for story in &docx.parts {
        let root = story.root();
        let prefix = if root.name().is(Some(W), "footnotes") {
            "fn"
        } else if root.name().is(Some(W), "endnotes") {
            "en"
        } else {
            continue;
        };
        for (k, node) in root.children().iter().enumerate() {
            let Node::Element(note) = node else { continue };
            let normal = matches!(note.attribute(Some(W), "type"), None | Some("normal"));
            let is_note = note.name().is(Some(W), "footnote") || note.name().is(Some(W), "endnote");
            if is_note && normal && format!("{prefix}:{}", note.attribute(Some(W), "id").unwrap_or("")) == id {
                return Ok((
                    story.part.clone(),
                    vec![u32::try_from(k).unwrap_or(u32::MAX)],
                    Arc::clone(note),
                ));
            }
        }
    }
    let _ = doc;
    Err(EditError::Address(ooxml_word_model::address::AddressError::not_found(
        format!("no note {}", ooxml_base::pyfmt::repr_str(id)),
        Vec::new(),
    )))
}

/// The part and place of a note's reference (`w:footnoteReference`), the first in story
/// order outside the notes parts.
fn note_reference(docx: &Docx, id: &str) -> Option<(String, Vec<u32>)> {
    let kind = NoteKind::of(id);
    let number = id.split_once(':').map_or("", |(_, n)| n);
    let local = format!("{}Reference", kind.name());
    for story in &docx.parts {
        let root = story.root();
        if root.name().is(Some(W), "footnotes") || root.name().is(Some(W), "endnotes") {
            continue;
        }
        let mut found = None;
        find_first(
            root,
            &mut Vec::new(),
            &mut |e| e.name().is(Some(W), &local) && e.attribute(Some(W), "id") == Some(number),
            &mut found,
        );
        if let Some(path) = found {
            return Some((story.part.clone(), path));
        }
    }
    None
}

/// The place of the first element below `element` that `wanted` takes.
fn find_first(
    element: &Element,
    path: &mut Vec<u32>,
    wanted: &mut dyn FnMut(&Element) -> bool,
    found: &mut Option<Vec<u32>>,
) {
    for (k, node) in element.children().iter().enumerate() {
        if found.is_some() {
            return;
        }
        let Node::Element(child) = node else { continue };
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        if wanted(child) {
            *found = Some(path.clone());
        } else {
            find_first(child, path, wanted, found);
        }
        path.pop();
    }
}

/// Where a note's reference goes (docx-agent's `_place`): a position, or a range's end, in
/// a body paragraph, not inside a field's result.
fn place(doc: &WordDoc, docx: &Docx, at: &str) -> Result<(Found, usize)> {
    let found = range(doc, docx, at)?;
    let id = range_id(&found);
    let span = Span::of(doc, docx, &found)?;
    if span.part != docx.main {
        return Err(EditError::Refused(
            "a note's reference goes in the body (Word has no note in a header, a footer, a note or a comment)"
                .to_owned(),
        ));
    }
    let offset = span.end;
    let last = span
        .paragraphs
        .into_iter()
        .last()
        .ok_or_else(|| EditError::Path(span.part.clone()))?;
    let work = Work::of(&last.element, Vec::new());
    let items = atoms(&work, work.root());
    if offset > items.len() {
        return Err(EditError::Refused(format!("{id} is outside its paragraph's text")));
    }
    let inside = offset > 0
        && items.get(offset).and_then(|a| a.field).is_some()
        && items.get(offset - 1).and_then(|a| a.field) == items.get(offset).and_then(|a| a.field);
    if inside {
        return Err(EditError::Refused(format!("{id} is inside a field's result")));
    }
    Ok((last, offset))
}

/// The run holding only `reference` (split out of a run that holds more): docx-agent's
/// `_isolate_reference`.
fn isolate_reference(work: &mut Work, reference: Id) -> Option<Id> {
    let run = work.parent(reference)?;
    let children: Vec<Id> = work
        .children(run)
        .iter()
        .copied()
        .filter(|c| work.is_element(*c) && !work.is(*c, W, "rPr"))
        .collect();
    if children == [reference] {
        return Some(run);
    }
    let properties = work.find(run, W, "rPr");
    let at = children.iter().position(|c| *c == reference)?;
    let alone = make(work, "r", &[]);
    if let Some(p) = properties {
        let copy = work.deep_copy(p);
        work.append(alone, copy);
    }
    let after: Vec<Id> = children.get(at + 1..).unwrap_or_default().to_vec();
    if !after.is_empty() {
        let tail = make(work, "r", &[]);
        if let Some(p) = properties {
            let copy = work.deep_copy(p);
            work.append(tail, copy);
        }
        for child in after {
            work.append(tail, child);
        }
        work.add_next(run, tail);
    }
    work.append(alone, reference);
    work.add_next(run, alone);
    if at == 0 {
        work.remove(run);
    }
    Some(alone)
}

/// Take a reference out with its run when the run holds nothing else, and the emptied
/// revision containers around it (docx-agent's `_remove_reference`).
fn remove_reference(work: &mut Work, reference: Id) {
    let run = work.parent(reference);
    work.remove(reference);
    let Some(run) = run else { return };
    let holds = work
        .children(run)
        .iter()
        .any(|c| work.is_element(*c) && !work.is(*c, W, "rPr"));
    if holds {
        return;
    }
    let mut parent = work.parent(run);
    work.remove(run);
    while let Some(p) = parent.filter(|p| work.is_any(*p, W, &["ins", "del"]) && work.children(*p).is_empty()) {
        parent = work.parent(p);
        work.remove(p);
    }
}

/// The largest note number in a notes part, plus one (at least one).
pub(crate) fn next_number(root: &Element, kind: NoteKind) -> i64 {
    root.child_elements()
        .filter(|n| n.name().is(Some(W), kind.name()))
        .filter_map(|n| n.attribute(Some(W), "id"))
        .filter(|v| {
            let digits = v.trim_start_matches('-');
            !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
        })
        .filter_map(|v| v.parse::<i64>().ok())
        .chain([0])
        .max()
        .unwrap_or(0)
        + 1
}
