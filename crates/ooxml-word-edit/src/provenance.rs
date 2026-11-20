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
//! Reads: the main part's relationships (where the item is) and the item; the paragraphs
//! the entries name are not read as keys -- an edit that changes one writes it, and its own
//! follow-up then writes the item, which a concurrent follow-up read.
//!
//! Must not: write an entry (docx-agent's tools that place values do), or run but once per
//! command, after it ([`Followed`]).

use crate::doc::{Key, WordDoc};
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_opc::name::rels_for;
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::provenance::{
    self as records, dumps, entry_text, facts, kind_of, target_in_file, text_paragraph, Kind, TextEntry, NS,
    REL_CUSTOM_XML,
};
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::docx::Docx;
use ooxml_xml::{Attribute, Element, Node};
use std::collections::BTreeMap;
use std::sync::Arc;

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
}

/// What becomes of one entry.
enum Fate {
    Kept,
    Dropped,
    /// Its key and its record now.
    Written(String, String),
}

/// The versions an entry is weighed against: the one the edit started from and the one
/// it made, each with its renames.
struct Versions<'v> {
    base: &'v Docx,
    base_aliases: &'v BTreeMap<String, String>,
    now: &'v Docx,
    aliases: &'v BTreeMap<String, String>,
}

/// The records followed after an edit, through `sink`.
///
/// # Errors
/// When the main part's relationships or a story part are not XML.
pub(crate) fn follow(sink: &mut EditSink<'_, '_>) -> Result<()> {
    let now = sink.docx()?;
    sink.read(Key::Part(Arc::from(rels_for(&now.main))));
    let Some(item) = records::item(&now)? else {
        return Ok(());
    };
    sink.read(Key::Part(Arc::from(item.as_str())));
    let Some(tree) = sink.doc().package().tree(&item)? else {
        return Ok(());
    };
    let base = sink.base().docx()?;
    let (base_aliases, aliases) = (sink.base().ids().aliases().clone(), sink.doc().ids().aliases().clone());
    let versions = Versions {
        base: &base,
        base_aliases: &base_aliases,
        now: &now,
        aliases: &aliases,
    };
    let root = tree.root();
    let mut children = Vec::new();
    let (mut changed, mut entries, mut tail) = (false, 0usize, false);
    for node in root.children().iter() {
        match node {
            Node::Element(entry) if entry.name().is(Some(NS), "source") => {
                tail = false;
                match fate(&versions, entry) {
                    Fate::Kept => {
                        children.push(node.clone());
                        entries += 1;
                    }
                    Fate::Dropped => {
                        // lxml's remove takes the element's tail with it
                        (changed, tail) = (true, true);
                    }
                    Fate::Written(key, text) => {
                        children.push(Node::Element(Arc::new(written(entry, &key, &text))));
                        (changed, entries) = (true, entries + 1);
                    }
                }
            }
            Node::Text(_) if tail => tail = false,
            other => {
                tail = false;
                children.push(other.clone());
            }
        }
    }
    if entries == 0 {
        let ids: Vec<String> = now
            .package
            .relationships(&now.main)?
            .iter()
            .filter(|r| r.rel_type == REL_CUSTOM_XML && !r.external && r.target_part.as_deref() == Some(&item))
            .map(|r| r.id.clone())
            .collect();
        return sink.release(&now.main, &ids);
    }
    if changed {
        let root = Element::from_head(Arc::clone(root.head()), ooxml_xml::Children::from_vec(children));
        sink.put(&item, tree.with_root(Arc::new(root)))?;
    }
    Ok(())
}

/// What the edit made of `entry`.
fn fate(versions: &Versions<'_>, entry: &Element) -> Fate {
    let key = entry.attribute(None, "key").unwrap_or_default();
    if !target_in_file(versions.base, versions.base_aliases, key) {
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
    let now = paragraph_text(&paragraph, View::Current);
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

/// `entry` keyed `key`, its text `text` (its attributes in their order, its other children
/// kept).
fn written(entry: &Element, key: &str, text: &str) -> Element {
    let attributes = entry
        .attributes()
        .iter()
        .map(|a| {
            if a.name.namespace().is_none() && a.name.local() == "key" {
                Attribute {
                    name: a.name.clone(),
                    value: Arc::from(key),
                }
            } else {
                a.clone()
            }
        })
        .collect();
    let mut children = vec![Node::Text(Arc::from(text))];
    children.extend(
        entry
            .children()
            .iter()
            .skip_while(|n| !matches!(n, Node::Element(_)))
            .cloned(),
    );
    Element::new(entry.name().clone(), entry.namespaces().to_vec(), attributes, children)
}
