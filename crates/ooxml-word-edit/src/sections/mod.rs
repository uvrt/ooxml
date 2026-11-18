//! Owns: sections as docx-agent's `edit/sections.py` edits them -- [`SetSection`] (page
//! setup), [`InsertSectionBreak`] and [`RemoveSectionBreak`] -- and what they share: a
//! section named (`s:<paraId>`, `s:<n>`, `s:body`, or the section a body block is in) and
//! its `w:sectPr`'s place.
//!
//! Must not: write but through the sink.

mod breaks;
mod removal;
mod setup;
mod stories;

pub use breaks::{InsertSectionBreak, RemoveSectionBreak};
pub(crate) use setup::canonical as element_canonical;
pub use setup::SectionValues;
pub use stories::{LinkToPrevious, SetStoryText, StoryEdited, UnlinkFromPrevious};

use crate::blocks::resolve_block;
use crate::doc::Key;
use crate::names::W16DU;
use crate::policy::RevisionPolicy;
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::stamp_document;
use crate::work::{scope_above, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::{section, span_ends, AddressError};
use ooxml_word_model::docx::sections::{section_id, sections, Section};
use ooxml_word_model::docx::{at_path, Docx};
use std::sync::Arc;

/// A section found: its place among the sections and the section.
pub(crate) struct Found {
    pub(crate) index: usize,
    pub(crate) section: Section,
}

impl Found {
    /// The place of the paragraph ending it (`None` for `s:body`).
    pub(crate) fn paragraph_path(&self) -> Option<&[u32]> {
        if self.section.id == "s:body" {
            return None;
        }
        self.section.path.get(..self.section.path.len().checked_sub(2)?)
    }
}

/// The section `identifier` names (docx-agent's `_section_record`): `s:<n>`, `s:<paraId>`
/// (its paragraph's id followed), `s:body`, or the section a body block is in.
pub(crate) fn find_section(doc: &WordDoc, docx: &Docx, identifier: &str) -> Result<Found> {
    let all = sections(docx);
    let wanted = if identifier.starts_with("s:") || identifier.starts_with("s@") {
        let followed = if identifier == "s:body"
            || identifier
                .get(2..)
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
        {
            identifier.to_owned()
        } else {
            let paragraph = format!("p{}", identifier.get(1..).unwrap_or(""));
            section_id(&doc.ids().follow(&paragraph))
        };
        let found = section(docx, &followed).or_else(|e| section(docx, identifier).map_err(|_| e));
        found.map_err(|e| EditError::Address(AddressError::not_found(e.message, e.options)))?
    } else {
        section_of(doc, docx, identifier, &all)?
    };
    let index = all.iter().position(|s| s.id == wanted).ok_or_else(|| {
        EditError::Address(AddressError::not_found(
            format!("no section {}", repr_str(identifier)),
            Vec::new(),
        ))
    })?;
    let section = all
        .into_iter()
        .nth(index)
        .ok_or_else(|| EditError::Path(docx.main.clone()))?;
    Ok(Found { index, section })
}

/// The id docx-agent's `_canonical_section` gives `identifier`: `s:<n>` the section's own,
/// `s:<paraId>` its paragraph's alias followed, anything else as given.
pub(crate) fn canonical(doc: &WordDoc, found: &Found, identifier: &str) -> String {
    let number = identifier
        .strip_prefix("s:")
        .is_some_and(|n| !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit()) && !n.starts_with('0'));
    if number {
        return found.section.id.clone();
    }
    if (identifier.starts_with("s:") || identifier.starts_with("s@")) && identifier != "s:body" {
        return section_id(&doc.ids().follow(&format!("p{}", identifier.get(1..).unwrap_or(""))));
    }
    identifier.to_owned()
}

/// The id of the section a body block is in (docx-agent's `section_of`).
fn section_of(doc: &WordDoc, docx: &Docx, identifier: &str, all: &[Section]) -> Result<String> {
    if span_ends(identifier).map_err(EditError::Address)?.is_some() {
        return Err(EditError::Address(AddressError::invalid(
            format!("{identifier} is a span: give one block, or a section (s:2, s:body)"),
            Vec::new(),
        )));
    }
    if docx.story(identifier).is_some() {
        return Err(EditError::Address(AddressError::invalid(
            format!("{identifier} is a story: give a block in the body, or a section (s:2, s:body)"),
            Vec::new(),
        )));
    }
    let (part, block) = resolve_block(doc, docx, identifier)?;
    let body = docx.body().ok_or_else(|| EditError::NoPart(docx.main.clone()))?;
    if part != docx.main {
        return Err(EditError::Address(AddressError::not_found(
            format!("{identifier} is not in the body"),
            Vec::new(),
        )));
    }
    let at = body
        .ids
        .paragraph_at(&block.path)
        .or_else(|| body.ids.paragraphs_within(&block.path).into_iter().next())
        .map(|p| p.index);
    let place = |s: &Section| {
        let path = s.path.get(..s.path.len().saturating_sub(2))?;
        (s.id != "s:body")
            .then(|| body.ids.paragraph_at(path).map(|p| p.index))
            .flatten()
    };
    let found = match at {
        Some(at) => all.iter().find(|s| place(s).is_none_or(|k| k >= at)),
        None => all
            .iter()
            .find(|s| s.id == "s:body" || s.path.as_ref() >= block.path.as_slice()),
    };
    found
        .or(all.last())
        .map(|s| s.id.clone())
        .ok_or_else(|| EditError::Refused("the document has no section".to_owned()))
}

/// A section's page setup set.
#[derive(Debug, Clone)]
pub struct SetSection {
    /// The section.
    pub section: String,
    /// What to set.
    pub values: SectionValues,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a section edit did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SectionEdited {
    /// The section's id after the edit (the new one's, for a break inserted).
    pub id: Option<String>,
    /// Whether it changed the document.
    pub changed: bool,
    /// Ids it made.
    pub created: Vec<String>,
    /// Ids it removed.
    pub removed: Vec<String>,
    /// Ids renamed, old to new, in the order made.
    pub renamed: Vec<(String, String)>,
    /// What it turned on that the caller did not ask for.
    pub warnings: Vec<String>,
}

impl Command<WordDoc> for SetSection {
    type Out = SectionEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        find_section(doc, &*doc.docx()?, &self.section).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<SectionEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl SetSection {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<SectionEdited> {
        let docx = sink.docx()?;
        let found = find_section(sink.doc(), &docx, &self.section)?;
        let main = docx.main.clone();
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let mut trial = Work::of(&found.section.properties, Vec::new());
        let root = trial.root();
        let was = setup::canonical(&trial, root);
        setup::apply(&mut trial, root, &self.values, orders);
        if setup::canonical(&trial, root) == was {
            return Ok(SectionEdited {
                id: Some(canonical(sink.doc(), &found, &self.section)),
                ..SectionEdited::default()
            });
        }
        let key = match found.paragraph_path().and_then(|p| docx.body()?.ids.paragraph_at(p)) {
            Some(p) => crate::set_text::block_key(&main, &p.id),
            None => Key::Part(Arc::from(main.as_str())),
        };
        sink.read(key);
        let renamed = stamp_document(&mut *sink, &docx)?;
        let tree = sink.doc().tree(&main)?.ok_or_else(|| EditError::NoPart(main.clone()))?;
        let path = found.section.path.to_vec();
        let element = at_path(tree.root(), &path).ok_or_else(|| EditError::Path(main.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &path));
        let sect = work.root();
        let before: Vec<(String, crate::work::Id)> = setup::kinds(&work, sect)
            .into_iter()
            .map(|(k, id)| (k, work.deep_copy(id)))
            .collect();
        setup::apply(&mut work, sect, &self.values, orders);
        let mut numbered = None;
        let mut dated = false;
        if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
            let ((), n, d) = sink.stamped(tracking, |stamp| {
                setup::record_change(&mut work, sect, &before, stamp, orders)
            })?;
            numbered = Some(n);
            dated = d;
        }
        let new = work.build().ok_or_else(|| EditError::Path(main.clone()))?;
        let draws = numbered.map(|n| annotation_draws(&new, n, 0)).unwrap_or_default();
        let docx = sink.docx()?;
        let mut keys = vec![
            match found.paragraph_path().and_then(|p| docx.body()?.ids.paragraph_at(p)) {
                Some(p) => crate::set_text::block_key(&main, &p.id),
                None => Key::Part(Arc::from(main.as_str())),
            },
        ];
        if numbered.is_some() {
            keys.push(Key::Annotations);
        }
        sink.replace_drawing(&main, &path, new, keys, draws)?;
        if dated {
            sink.ensure_w14(&main, false, &[("w16du", W16DU)])?;
        }
        if let (RevisionPolicy::Tracked(tracking), Some(n)) = (sink.policy().clone(), numbered) {
            sink.people(
                (n.to > n.from).then_some(tracking.author.as_str()),
                Some(&tracking.author),
            )?;
        }
        let id = match found.paragraph_path().and_then(|p| docx.body()?.ids.paragraph_at(p)) {
            Some(p) => section_id(&p.id),
            None => canonical(sink.doc(), &found, &self.section),
        };
        Ok(SectionEdited {
            id: Some(id),
            changed: true,
            renamed,
            ..SectionEdited::default()
        })
    }
}
