//! Owns: the invariant every version a command makes keeps ([`Stamped`]): no paragraph,
//! row or drawing of any story without a valid and unique id, however the command wrote it
//! -- every block it wrote without one stamped through the sink, by docx-agent's stamping
//! rules (word-model's [`ooxml_word_model::stamp::blocks`]). A
//! document is stamped in memory when it is opened ([`ooxml_word_model::WordPackage`]), and so
//! is a part another editor's version brings (`Change::Arrived`): what is left to stamp here is
//! what a command writes without ids.
//!
//! Must not: write but through the sink, or stamp a block twice (a stamped block's id is
//! valid, and is left alone).

use crate::doc::{Key, WordDoc};
use crate::names::{W14, WP14};
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::stamp::{prefix_at, unstamped};

pub(crate) use ooxml_word_model::stamp::with_attribute;

/// A command, then every block it left without a durable id stamped, in its transaction,
/// when it changed anything. Word drops every paraId of a document holding one paragraph or
/// table row without a valid, unique paraId, or one drawing without its ids, when it saves
/// it (crates/ooxml-oracle/word/observations/paraid-durability.json, factors), and the
/// block ids rest on paraIds: so no version a command makes may hold one. This stamps what
/// a command wrote without ids (a field's update, a part it rewrote), and costs nothing on
/// a stamped story (its index counts what lacks an id).
pub(crate) struct Stamped<'c, C>(pub(crate) &'c C);

impl<C: Command<WordDoc, Error = EditError>> Command<WordDoc> for Stamped<'_, C> {
    type Out = C::Out;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.0.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<C::Out> {
        // every way a part enters the package stamps its blocks (open, an edit, a part
        // another writer put): a command never meets a block without its id
        debug_assert!(every_block_stamped(tx.doc()), "a command met a block without its id");
        let out = self.0.prepare(tx)?;
        if !tx.changes().is_empty() {
            let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
            stamp_written(&mut sink)?;
        }
        Ok(out)
    }

    /// The command's (stamping refuses only a part that is not XML).
    fn read_refusal(&self, tx: &mut Txn<'_, WordDoc>) {
        self.0.read_refusal(tx);
    }
}

/// Whether every paragraph and row of every story of `doc` has a valid and unique id.
fn every_block_stamped(doc: &WordDoc) -> bool {
    doc.docx()
        .is_ok_and(|docx| docx.parts.iter().all(|p| p.ids.fully_stamped()))
}

/// Stamp every block of every story the command wrote without an id, each part's ids drawn
/// through the sink (one draw per seed). A block written without an id was never named:
/// nothing is renamed.
///
/// # Errors
/// When a part is not XML.
fn stamp_written(sink: &mut EditSink<'_, '_>) -> Result<()> {
    let docx = sink.docx()?;
    for story in &docx.parts {
        let part = story.part.as_str();
        // drawings are looked for only in a part whose memoised facts say it has one to stamp
        let drawings = crate::validate::unstamped_drawings(sink.doc(), part)?;
        let Some(found) = unstamped(part, story.root(), &story.ids, drawings) else {
            continue;
        };
        sink.read(Key::part(part));
        let stamp = found.stamp(&mut |seed, count| sink.draw(seed, count))?;
        sink.put(part, story.tree.with_root(stamp.root))?;
    }
    Ok(())
}

/// The prefix the paragraph at `path` of `root` writes `w14` attributes with.
pub(crate) fn w14_prefix(root: &ooxml_xml::Element, path: &[u32]) -> String {
    prefix_at(root, path, W14, "w14")
}

/// The prefix the element at `path` of `root` writes `wp14` attributes with.
pub(crate) fn wp14_prefix(root: &ooxml_xml::Element, path: &[u32]) -> String {
    prefix_at(root, path, WP14, "wp14")
}
