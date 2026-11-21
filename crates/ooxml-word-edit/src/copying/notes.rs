//! Owns: the footnotes and endnotes a copy's references name, copied as docx-agent's
//! importer copies them (`_notes`, `_stamp_notes`): per kind, the references in document
//! order, each given the next number of this document's notes part (made, with the other
//! kind's, where it has none) and a copy of its note there -- the note's content as the
//! source shows it now, with its styles, lists, relationships, bookmarks, drawing numbers
//! and control ids brought like a block's -- and once the blocks are stamped, every paragraph and row of a copied
//! note given new ids. A reference to a note the source lacks stays as it is; references
//! of a kind the source has no part for are dropped.
//!
//! As docx-agent's, a note's comment markers are copied as they are (`_notes` does not
//! bring comments).
//!
//! Must not: write a notes part but through the copy's [`Space`].

use super::content::Bringer;
use super::view::records_view;
use crate::names::{W, W14, WP};
use crate::notes::{ensure_notes_part, next_number, NoteKind};
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// The source's notes by id.
type ById = HashMap<String, Arc<Element>>;

/// What the notes of a copy gave.
#[derive(Default)]
pub(super) struct NotesCopied {
    /// Each note copied: its notes part and its copy there.
    made: Vec<(String, Id)>,
    /// Each source note and its copy (`fn:2` -> `fn:5`), each after its controls' ids
    /// changed (`sdt#7` -> `sdt#12`), in order.
    pub(super) copied: Vec<(String, String)>,
}

/// The notes `elements` (copied blocks in `work`) refer to, copied into this document's
/// notes parts, held in `stories`.
///
/// # Errors
/// What bringing its content refuses; a part that is not XML.
pub(super) fn bring(
    sink: &mut EditSink<'_, '_>,
    bringer: &mut Bringer<'_>,
    stories: &mut Space,
    (source, work): (&Docx, &mut Work),
    elements: &[Id],
) -> Result<NotesCopied> {
    let mut out = NotesCopied::default();
    for kind in [NoteKind::Footnote, NoteKind::Endnote] {
        let name = kind.name();
        let reference = format!("{name}Reference");
        let references: Vec<Id> = elements
            .iter()
            .flat_map(|e| work.iter(*e, W, &[reference.as_str()]))
            .collect();
        if references.is_empty() {
            continue;
        }
        let Some((source_part, notes)) = source_notes(source, kind)? else {
            for node in references {
                work.remove(node);
            }
            continue;
        };
        let part = ensure_notes_part(sink, kind)?;
        let doc = sink.doc().clone();
        let tree = doc.tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
        let mut number = next_number(tree.root(), kind);
        let into = stories
            .work(&doc, &part)?
            .ok_or_else(|| EditError::NoPart(part.clone()))?;
        for node in references {
            let old = work.get(node, Some(W), "id").unwrap_or("").to_owned();
            let Some(note) = notes.get(&old) else { continue };
            let copy = into
                .adopt_copy(note)
                .ok_or_else(|| EditError::Path(source_part.clone()))?;
            let id = number.to_string();
            let attribute = into.qname(W, "id", "w");
            into.set(copy, attribute, &id);
            let attribute = work.qname(W, "id", "w");
            work.set(node, attribute, &id);
            let content: Vec<Id> = into
                .children(copy)
                .iter()
                .copied()
                .filter(|c| into.is_element(*c))
                .collect();
            for &child in &content {
                records_view(into, child);
            }
            bringer.content(sink, into, &content, (&source_part, &part))?;
            bringer.bookmarks(sink, into, &content)?;
            crate::imported::keep_or_renumber_drawings(sink, into, &content)?;
            let controls = bringer.controls(sink, into, &content)?;
            out.copied.extend(controls);
            let root = into.root();
            into.append(root, copy);
            out.made.push((part.clone(), copy));
            let prefix = kind.prefix();
            out.copied.push((format!("{prefix}:{old}"), format!("{prefix}:{id}")));
            number += 1;
        }
        stories.mark_dirty(&part);
    }
    Ok(out)
}

impl NotesCopied {
    /// Every paragraph and row of each copied note given new ids from its part's import
    /// sequence (`_stamp_notes`), the paragraphs' added to `created`; the notes parts that
    /// hold a drawing.
    ///
    /// # Errors
    /// When the ledger cannot be written.
    pub(super) fn stamp(
        &self,
        sink: &mut EditSink<'_, '_>,
        stories: &mut Space,
        created: &mut Vec<String>,
    ) -> Result<Vec<(String, bool)>> {
        let docx = sink.docx()?;
        let mut parts: Vec<(String, bool)> = Vec::new();
        for (part, note) in &self.made {
            let story = docx.part(part).map_or_else(String::new, |s| s.story.clone());
            let seed = format!("{part}\0import");
            let work = stories.get_mut(part).ok_or_else(|| EditError::NoPart(part.clone()))?;
            let nodes: Vec<Id> = work.iter(*note, W, &["p", "tr"]);
            for node in nodes {
                let drawn = sink.draw(&seed, 2)?;
                let (Some(para), Some(text)) = (drawn.first(), drawn.get(1)) else {
                    return Err(EditError::Path(part.clone()));
                };
                let work = stories.get_mut(part).ok_or_else(|| EditError::NoPart(part.clone()))?;
                for (local, value) in [("paraId", para), ("textId", text)] {
                    let attribute = work.qname(W14, local, "w14");
                    work.set(node, attribute, value);
                }
                if work.is(node, W, "p") {
                    created.push(format!("{story}/p:{para}"));
                }
            }
            let work = stories.get(part).ok_or_else(|| EditError::NoPart(part.clone()))?;
            let drawings = !work.iter(*note, WP, &["inline", "anchor"]).is_empty();
            match parts.iter_mut().find(|(p, _)| p == part) {
                Some(found) => found.1 |= drawings,
                None => parts.push((part.clone(), drawings)),
            }
        }
        Ok(parts)
    }
}

impl NotesCopied {
    /// Tracked, each copied note's content recorded as inserted as the importer's `_track`
    /// records it: each paragraph's content, and its mark but the last's; each table's rows
    /// and paragraphs. The notes parts a record is dated in.
    ///
    /// # Errors
    /// When the annotation ids cannot be read.
    pub(super) fn track(&self, sink: &mut EditSink<'_, '_>, stories: &mut Space) -> Result<Vec<String>> {
        let RevisionPolicy::Tracked(tracking) = sink.policy().clone() else {
            return Ok(Vec::new());
        };
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let mut dated: Vec<String> = Vec::new();
        for (part, note) in &self.made {
            let work = stories.get_mut(part).ok_or_else(|| EditError::NoPart(part.clone()))?;
            let content: Vec<Id> = work
                .children(*note)
                .iter()
                .copied()
                .filter(|c| work.is_element(*c))
                .collect();
            let ((), _, d) = sink.stamped(tracking.clone(), |stamp| {
                for (k, &element) in content.iter().enumerate() {
                    if work.is(element, W, "p") {
                        crate::markdown::insert_content(work, element, stamp);
                        if k + 1 < content.len() {
                            crate::marks::mark(work, element, "ins", stamp, orders);
                        }
                    } else {
                        crate::markdown::track_table(work, element, stamp, orders);
                    }
                }
                Ok(())
            })?;
            if d && !dated.contains(part) {
                dated.push(part.clone());
            }
        }
        Ok(dated)
    }
}

/// The source's notes part of `kind` and its notes by id (the last of an id), if it has
/// one.
fn source_notes(source: &Docx, kind: NoteKind) -> Result<Option<(String, ById)>> {
    let Some(part) = ooxml_word_model::parts::related(&source.package, &source.main, &kind.rel())? else {
        return Ok(None);
    };
    let Some(tree) = source.package.tree(&part)? else {
        return Ok(None);
    };
    let notes = tree
        .root()
        .child_elements()
        .filter(|n| n.name().is(Some(W), kind.name()))
        .map(|n| (n.attribute(Some(W), "id").unwrap_or("").to_owned(), Arc::clone(n)))
        .collect();
    Ok(Some((part, notes)))
}
