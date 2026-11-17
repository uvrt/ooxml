//! Owns: [`SetText`], a paragraph's whole text rewritten as docx-agent's `set_text` does
//! (`Document._set_text`, `_write_text`): nothing written when the text is already that;
//! otherwise the document stamped first (every block without a durable id gets one), the
//! text written through the sink by the revision policy -- untracked keeping each surviving
//! character's formatting, tracked as word-level deletions and insertions -- and, for a
//! paragraph that was stamped before, a new `w14:textId`.
//!
//! Must not: write but through the sink, or read outside what it records reading: the
//! paragraph's own block, the ledger seeds it draws from, the annotation ids when tracked.

use crate::doc::{Key, WordDoc};
use crate::names::W14;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::stamping::{new_text_id, stamp_document, w14_prefix};
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
struct Found {
    part: String,
    path: Box<[u32]>,
    id: String,
    state: IdState,
    element: Arc<ooxml_xml::Element>,
}

/// The paragraph `target` names now: an alias followed while its target exists; a cell's
/// first paragraph.
fn find(doc: &WordDoc, docx: &Docx, target: &str) -> Result<Found> {
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
        .paragraphs
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
    let table = found.part.ids.tables.get(found.table).ok_or_else(refuse)?;
    let (row_path, _) = table.rows.get(found.row).ok_or_else(refuse)?;
    let row = at_path(found.part.root(), row_path).ok_or_else(refuse)?;
    let cell = cell_at(row, column).ok_or_else(refuse)?;
    let mut cell_path = row_path.to_vec();
    cell_path.push(cell);
    let paragraph = found
        .part
        .ids
        .paragraphs
        .iter()
        .find(|p| p.path.len() == cell_path.len() + 1 && p.path.starts_with(&cell_path))
        .ok_or_else(refuse)?;
    Ok(Found {
        part: found.part.part.clone(),
        path: paragraph.path.clone(),
        id: paragraph.id.clone(),
        state: paragraph.state,
        element: Arc::clone(&paragraph.element),
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
        let wrote = sink.set_paragraph_text(&found.part, &mut work, &self.text)?;
        if !wrote.released.is_empty() {
            return Err(EditError::Unsupported(
                "releasing the relationships of content a tracked edit removed outright (phase P2b)".to_owned(),
            ));
        }
        if found.state == IdState::Ok {
            let text_id = new_text_id(&mut sink, &found.part)?;
            let prefix = w14_prefix(tree.root(), &found.path);
            let name = Arc::new(QName::new(Some(&prefix), "textId", Some(W14)));
            let root = work.root();
            work.set(root, name, &text_id);
            sink.ensure_w14(&found.part, false, &[])?;
        }
        let new = work.build().ok_or_else(|| EditError::Path(found.part.clone()))?;
        sink.replace(&found.part, &found.path, new, vec![block_key(&found.part, &id)])?;
        crate::textbox::sync(&mut sink)?;
        crate::revision_ids::make_unique(&mut sink, &found.part, &found.path, &found.element)?;
        Ok(TextEdited {
            id,
            changed: true,
            renamed,
            part: found.part,
        })
    }
}
