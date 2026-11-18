//! Owns: [`SetText`], a paragraph's whole text rewritten as docx-agent's `set_text` does
//! (`Document._set_text`, `_write_text`): nothing written when the text is already that;
//! otherwise the document stamped first (every block without a durable id gets one), the
//! text written through the sink by the revision policy -- untracked keeping each surviving
//! character's formatting, tracked as word-level deletions and insertions -- and, for a
//! paragraph that was stamped before, a new `w14:textId`.
//!
//! Must not: write but through the sink, or read outside what it records reading: the
//! paragraph's own block, the ledger seeds it draws from, the annotation ids when tracked.

use crate::doc::{Draw, DrawFrom, Key, WordDoc};
use crate::names::{W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::stamping::{stamp_document, w14_prefix};
use crate::text::{paragraph_text, write::check_text};
use crate::work::{scope_above, Work};
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::resolve::cell_at;
use ooxml_word_model::address::{direct, in_table, unknown};
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_word_model::ids::{Entry, IdState};
use ooxml_xml::QName;
use std::sync::Arc;

/// Rewrite one paragraph's text.
#[derive(Debug, Clone)]
pub struct SetText {
    /// The paragraph (`p:...`, `p@body/3`, a renamed id) or a cell (`t:X/c1,1`: its first
    /// paragraph).
    pub target: String,
    /// The whole new text: no `\n` (`\v` is a line break, `\t` a tab).
    pub text: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a set_text did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdited {
    /// The paragraph's id after the edit.
    pub id: String,
    /// Whether anything changed.
    pub changed: bool,
    /// Ids the edit's stamping renamed, old to new, in the order made.
    pub renamed: Vec<(String, String)>,
    /// The paragraph's part.
    pub part: String,
}

/// A paragraph found: its part, place, id and how its id stands.
pub(crate) struct Found {
    pub(crate) part: String,
    pub(crate) path: Box<[u32]>,
    pub(crate) id: String,
    pub(crate) state: IdState,
    pub(crate) element: Arc<ooxml_xml::Element>,
}

/// The paragraph `target` names now: an alias followed while its target exists; a cell's
/// first paragraph.
pub(crate) fn find(doc: &WordDoc, docx: &Docx, target: &str) -> Result<Found> {
    let followed = doc.ids().follow(target);
    let block = (followed != target)
        .then(|| direct(docx, &followed))
        .flatten()
        .or_else(|| direct(docx, target));
    let (story, entry) = match block {
        Some(found) => found,
        None => return cell_paragraph(docx, target),
    };
    let Entry::Paragraph(k) = entry else {
        return Err(EditError::Refused(format!("{target} is not a paragraph")));
    };
    let paragraph = story
        .ids
        .paragraphs()
        .get(k)
        .ok_or_else(|| EditError::Refused(format!("{target} is not a paragraph")))?;
    Ok(Found {
        part: story.part.clone(),
        path: paragraph.path.clone(),
        id: paragraph.id.clone(),
        state: paragraph.state,
        element: Arc::clone(&paragraph.element),
    })
}

/// The first paragraph of the cell `target` names (`t:X/c<row>,<col>`).
fn cell_paragraph(docx: &Docx, target: &str) -> Result<Found> {
    let refuse = || EditError::Address(unknown(docx, target));
    let found = in_table(docx, target).map_err(EditError::Address)?.ok_or_else(refuse)?;
    let column = found
        .column
        .ok_or_else(|| EditError::Refused(format!("{target} is a row, not a paragraph")))?;
    let table = found.part.ids.tables().get(found.table).ok_or_else(refuse)?;
    let (row_path, _) = table.rows.get(found.row).ok_or_else(refuse)?;
    let row = at_path(found.part.root(), row_path).ok_or_else(refuse)?;
    let cell = cell_at(row, column).ok_or_else(refuse)?;
    let mut cell_path = row_path.to_vec();
    cell_path.push(cell);
    let paragraph = found
        .part
        .ids
        .paragraphs_within(&cell_path)
        .into_iter()
        .find(|p| p.path.len() == cell_path.len() + 1)
        .ok_or_else(refuse)?;
    Ok(Found {
        part: found.part.part.clone(),
        path: paragraph.path.clone(),
        id: paragraph.id.clone(),
        state: paragraph.state,
        element: Arc::clone(&paragraph.element),
    })
}

/// Whether a paragraph holds an element whose `w:id` is an annotation id.
fn paragraph_has_annotations(paragraph: &ooxml_xml::Element) -> bool {
    paragraph.child_elements().any(|c| {
        (c.name().namespace() == Some(crate::names::W) && c.attribute(Some(crate::names::W), "id").is_some())
            || paragraph_has_annotations(c)
    })
}

/// The key a block is read and written under: its durable id.
fn block_key(part: &str, id: &str) -> Key {
    Key::Block {
        part: Arc::from(part),
        id: Arc::from(id),
    }
}

impl Command<WordDoc> for SetText {
    type Out = TextEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        check_text(&self.text)?;
        find(doc, &*doc.docx()?, &self.target).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<TextEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let found = find(sink.doc(), &docx, &self.target)?;
        sink.read(block_key(&found.part, &found.id));
        let mut work = Work::of(&found.element, Vec::new());
        if paragraph_text(&work, work.root()) == self.text {
            return Ok(TextEdited {
                id: found.id,
                changed: false,
                renamed: Vec::new(),
                part: found.part,
            });
        }
        let renamed = stamp_document(&mut sink, &docx)?;
        let id = renamed
            .iter()
            .find(|(old, _)| *old == found.id)
            .map_or_else(|| found.id.clone(), |(_, new)| new.clone());
        let tree = sink
            .doc()
            .tree(&found.part)?
            .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
        let element = at_path(tree.root(), &found.path).ok_or_else(|| EditError::Path(found.part.clone()))?;
        work = Work::of(element, scope_above(tree.root(), &found.path));
        let wrote = sink.set_paragraph_text(&mut work, &self.text)?;
        // a paragraph stamped before gets a new text id, drawn where the change is applied
        let mut draws = Vec::new();
        if found.state == IdState::Ok {
            let prefix = w14_prefix(tree.root(), &found.path);
            draws.push(Draw {
                from: DrawFrom::Seed(Arc::from(format!("{}\0text", found.part))),
                at: Vec::new(),
                attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
            });
        }
        let new = work.build().ok_or_else(|| EditError::Path(found.part.clone()))?;
        if let Some(numbered) = wrote.annotations {
            draws.extend(crate::sink::annotation_draws(&new, numbered));
        }
        let mut keys = vec![block_key(&found.part, &id)];
        if paragraph_has_annotations(&found.element) || paragraph_has_annotations(&new) {
            keys.push(Key::Annotations); // the ids in use, which tracked edits number from
        }
        sink.replace_drawing(&found.part, &found.path, new, keys, draws)?;
        // the part's root declares what the paragraph now uses (docx-agent: the revisions'
        // `w16du`, then the text id's `w14`), once the paragraph is in place
        if wrote.dated {
            sink.ensure_w14(&found.part, false, &[("w16du", W16DU)])?;
        }
        if found.state == IdState::Ok {
            sink.ensure_w14(&found.part, false, &[])?;
        }
        crate::textbox::sync(&mut sink)?;
        crate::revision_ids::make_unique(&mut sink)?;
        if let RevisionPolicy::Tracked(tracking) = &self.policy {
            let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
            sink.people(gave.then_some(tracking.author.as_str()), Some(&tracking.author))?;
            sink.release(&found.part, &wrote.released)?;
        }
        Ok(TextEdited {
            id,
            changed: true,
            renamed,
            part: found.part,
        })
    }
}
