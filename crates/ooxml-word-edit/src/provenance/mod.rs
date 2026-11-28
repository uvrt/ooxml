//! Owns: what docx-agent does to its records of where placed values came from at the end
//! of every edit (`edit/provenance.py`, run by `Document._edit` and `batch`), in the
//! edit's own history step: entries whose table, chart or paragraph the version the edit
//! started from no longer has are dropped (`_prune_provenance` as the edit begins); each
//! paragraph's text facts and labels follow its text (`_follow_text_facts`: moved with the
//! text, dropped where the edit touched them, kept under the paragraph's new id, gone with
//! the last of them); entries whose object the edit removed are dropped
//! (`_prune_provenance` as it ends); and the item, its properties and its relationship go
//! together when no entry is left.
//!
//! Reads: by condition, the item the main part's relationships name ([`item::ItemNamed`]); the
//! item; the paragraphs the entries name are not read as keys -- an edit that changes one
//! writes it, and its own follow-up then writes the item, which a concurrent follow-up read.
//!
//! A review moves or drops the tables' records as well ([`follow_review`], run by the
//! review itself before this).
//!
//! Values written by reference and labels are kept on their paragraphs by [`KeepText`]
//! (the tool that wrote them says where), the item made where there is none; a chart's
//! record is replaced or dropped by the edit that changed its data ([`store_record`]).
//!
//! Must not: decide where a value went, or run the follow-up but once per command, after
//! it ([`Followed`]).

mod item;
mod keep;
mod made;
mod review;

pub use keep::{KeepText, TextKeep};
pub(crate) use review::follow_review;

use crate::doc::WordDoc;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::{EditError, Result};
use item::{Draft, Fate};
use ooxml_provenance::dumps;
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::provenance::{
    self as records, entry_text, facts, kind_of, target_in_file, text_paragraph, Kind, TextEntry,
};
use ooxml_word_model::docx::text::paragraph_text;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::view::View;
use ooxml_xml::Element;
use std::collections::{BTreeMap, HashSet};

/// A command, then the records followed in its transaction when it changed anything.
pub(crate) struct Followed<'c, C>(pub(crate) &'c C);

impl<C: Command<WordDoc, Error = EditError>> Command<WordDoc> for Followed<'_, C> {
    type Out = C::Out;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.0.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<C::Out> {
        let out = self.0.prepare(tx)?;
        if !tx.changes().is_empty() {
            follow(&mut EditSink::new(tx, RevisionPolicy::Untracked))?;
        }
        Ok(out)
    }

    /// The command's (following refuses only a part that is not XML).
    fn read_refusal(&self, tx: &mut Txn<'_, WordDoc>) {
        self.0.read_refusal(tx);
    }
}

/// The entry of `kind` keyed `key` holding `text` now, made (with the item, where the
/// document has none) or replaced where it stands; with `None`, dropped (docx-agent's
/// `store_provenance`). The edit's follow-up then prunes and releases as after every edit.
///
/// # Errors
/// When the content types, the main part's relationships or the item are not XML.
pub(crate) fn store_record(sink: &mut EditSink<'_, '_>, kind: &str, key: &str, text: Option<&str>) -> Result<()> {
    let docx = sink.docx()?;
    let mut draft = Draft::open(sink, &docx)?;
    match (text, draft.as_mut()) {
        (None, None) => return Ok(()),
        (None, Some(found)) => found.drop_entry(kind, key),
        (Some(text), Some(found)) => found.store(kind, key, text),
        (Some(text), None) => {
            made::make_item(sink, &docx, key)?;
            let made = sink.docx()?;
            draft = Draft::open(sink, &made)?;
            if let Some(found) = draft.as_mut() {
                found.store(kind, key, text);
            }
        }
    }
    match draft {
        Some(found) => found.finish(sink, &docx),
        None => Ok(()),
    }
}

/// The record kept for the table `key` (docx-agent's `provenance().get(key)`), read through
/// `sink` as an edit of the records reads them.
///
/// # Errors
/// When the content types, the main part's relationships or the item are not XML.
pub(crate) fn table_record(
    sink: &mut EditSink<'_, '_>,
    key: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>> {
    let docx = sink.docx()?;
    Draft::open(sink, &docx)?;
    Ok(records::provenance(&docx)?.get(key).cloned())
}

/// What an entry is weighed against: the keys of the entries whose object the version the
/// edit started from no longer had (pruned as the edit began: an entry the edit itself
/// wrote is not), and the version the edit made, with its renames.
struct Versions<'v> {
    stale: &'v HashSet<String>,
    now: &'v Docx,
    aliases: &'v BTreeMap<String, String>,
}

/// The records followed after an edit, through `sink`.
///
/// # Errors
/// When the main part's relationships or a story part are not XML.
pub(crate) fn follow(sink: &mut EditSink<'_, '_>) -> Result<()> {
    let now = sink.docx()?;
    let Some(mut draft) = Draft::open(sink, &now)? else {
        return Ok(());
    };
    let stale = stale(&*sink.base().docx()?, sink.base().ids().aliases())?;
    let aliases = sink.doc().ids().aliases().clone();
    let versions = Versions {
        stale: &stale,
        now: &now,
        aliases: &aliases,
    };
    draft.settle(|entry| fate(&versions, entry));
    draft.finish(sink, &now)
}

/// The keys of `base`'s entries whose table, chart or paragraph `base` does not have.
///
/// # Errors
/// When the main part's relationships are not XML.
fn stale(base: &Docx, aliases: &BTreeMap<String, String>) -> Result<HashSet<String>> {
    let Some(name) = records::item(base)? else {
        return Ok(HashSet::new());
    };
    let Some(tree) = base.package.tree(&name)? else {
        return Ok(HashSet::new());
    };
    Ok(records::entries(tree.root())
        .map(|e| e.attribute(None, "key").unwrap_or_default())
        .filter(|key| !target_in_file(base, aliases, key))
        .map(str::to_owned)
        .collect())
}

/// What the edit made of `entry`.
fn fate(versions: &Versions<'_>, entry: &Element) -> Fate {
    let key = entry.attribute(None, "key").unwrap_or_default();
    if versions.stale.contains(key) {
        return Fate::Dropped;
    }
    let mut fate = Fate::Kept;
    if entry.attribute(None, "kind") == Some("text") {
        if let Some(followed) = followed(versions, key, &entry_text(entry)) {
            fate = followed;
        }
    }
    let now = match &fate {
        Fate::Dropped => return Fate::Dropped,
        Fate::Written(name, _) => name.as_str(),
        Fate::Kept => key,
    };
    if target_in_file(versions.now, versions.aliases, now) {
        fate
    } else {
        Fate::Dropped
    }
}

/// A text entry after the edit (docx-agent's `_follow_text_facts` for one entry): `None`
/// where it stays as it is -- it holds no record, its paragraph is gone (pruned after), or
/// neither the paragraph's text nor its id changed.
fn followed(versions: &Versions<'_>, key: &str, text: &str) -> Option<Fate> {
    let found = TextEntry::read(text)?;
    let (paragraph, name) = text_paragraph(versions.now, versions.aliases, key)?;
    let now = paragraph_text(&paragraph, View::Final);
    if now == found.paragraph && name == key {
        return None;
    }
    let after = TextEntry {
        facts: facts::facts_after_edit(&found.facts, &found.paragraph, &now),
        labels: facts::labels_after_edit(&found.labels, &found.paragraph, &now),
        paragraph: now,
    };
    if (after.facts.is_empty() && after.labels.is_empty()) || kind_of(&name) != Some(Kind::Text) {
        return Some(Fate::Dropped);
    }
    Some(Fate::Written(name, dumps(&after.record())))
}
