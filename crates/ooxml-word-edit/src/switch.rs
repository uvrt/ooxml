//! Owns: [`SetWordTracking`], Word's own Track Changes switch (`w:trackRevisions` in the
//! settings) turned on or off as docx-agent's `set_word_tracks_changes` writes it: an
//! empty element in its place in the settings' sequence; off is no element; nothing
//! written when the switch already is so.
//!
//! Must not: change how this library's edits are written (the session's tracking mode,
//! a [`crate::RevisionPolicy`], does that).

use crate::doc::{Key, WordDoc};
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::review::xml::{insert_in_order, make};
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::parts::{related, REL_SETTINGS};
use ooxml_xml::Element;
use std::sync::Arc;

/// Turn Word's Track Changes switch on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetWordTracking {
    /// On (else off).
    pub on: bool,
}

/// What setting the switch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordTracking {
    /// Whether it changed the document.
    pub changed: bool,
    /// The switch after the edit.
    pub on: bool,
}

/// Whether a settings root turns Word's Track Changes on (`w:trackRevisions`, its `w:val`
/// not off).
#[must_use]
pub fn tracks_changes(settings: &Element) -> bool {
    settings.find(Some(W), "trackRevisions").is_some_and(|n| {
        !matches!(
            n.attribute(Some(W), "val").unwrap_or("true").to_lowercase().as_str(),
            "0" | "false" | "off"
        )
    })
}

/// The settings part of `doc`.
fn settings_part(doc: &WordDoc) -> Result<Option<String>> {
    let docx = doc.docx()?;
    Ok(related(doc.package(), &docx.main, REL_SETTINGS)?)
}

/// Whether `doc` turns Word's Track Changes on.
///
/// # Errors
/// When the settings part is not XML.
pub fn word_tracks_changes(doc: &WordDoc) -> Result<bool> {
    let Some(part) = settings_part(doc)? else {
        return Ok(false);
    };
    Ok(doc.tree(&part)?.is_some_and(|t| tracks_changes(t.root())))
}

impl Command<WordDoc> for SetWordTracking {
    type Out = WordTracking;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        match settings_part(doc)? {
            Some(_) => Ok(()),
            None => Err(EditError::Refused("the document has no settings part".to_owned())),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<WordTracking> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let part = settings_part(sink.doc())?
            .ok_or_else(|| EditError::Refused("the document has no settings part".to_owned()))?;
        sink.read(Key::Part(Arc::from(part.as_str())));
        if word_tracks_changes(sink.doc())? == self.on {
            return Ok(WordTracking {
                changed: false,
                on: self.on,
            });
        }
        let held = sink.doc().clone();
        let orders = held.shared().facts.orders();
        let mut space = Space::default();
        let work = space
            .work(&held, &part)?
            .ok_or_else(|| EditError::NoPart(part.clone()))?;
        for node in work.children(work.root()).to_vec() {
            if work.is(node, W, "trackRevisions") {
                work.remove(node);
            }
        }
        if self.on {
            let node = make(work, "trackRevisions", &[]);
            let root = work.root();
            insert_in_order(work, root, node, orders);
        }
        space.mark_dirty(&part);
        space.finish(&mut sink)?;
        Ok(WordTracking {
            changed: true,
            on: self.on,
        })
    }
}
