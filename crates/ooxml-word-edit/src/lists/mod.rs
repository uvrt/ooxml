//! Owns: [`ListEdit`], docx-agent's list edits on the paragraphs some addresses hold, one
//! per paragraph in order as `word_lists` makes them (`edit/numbering.py`'s `ListOps`):
//! a paragraph added to Word's own bullet or numbered list (joining the list right before
//! it), taken out of its list, moved to a level, its list restarted or continued, a level's
//! format changed -- each step stamping as an edit does, writing the paragraphs' and the
//! numbering part's changes (tracked: the paragraphs' old properties in `w:pPrChange`s)
//! and removing the instances and definitions it left unused.
//!
//! Must not: write but through the sink, or know a tool's JSON. Every step reads the whole
//! document: an instance is removed when no paragraph of any story names it.

pub(crate) mod numbering;
mod paragraph;
mod read;
mod steps;

pub(crate) use numbering::reap;
pub use paragraph::LevelFormat;
pub(crate) use read::style_list as style_list_of;
pub use read::{numbered_by_style, unnumbered_styles_like};

use crate::policy::RevisionPolicy;
use crate::stamping::stamp_document;
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::scope_paragraphs;
use ooxml_word_model::docx::numbering::Membership;
use ooxml_word_model::docx::Docx;
use read::Place;

/// What a [`ListEdit`] does to its paragraphs.
#[derive(Debug, Clone, PartialEq)]
pub enum ListAction {
    /// Make each a list item of Word's own `bullet` or `number` list at `level`.
    Add {
        /// `bullet` or `number`.
        kind: String,
        /// From 0.
        level: i64,
    },
    /// Take each out of its list.
    Remove,
    /// Move each to a level.
    Level(i64),
    /// Restart the first one's list there, from a number.
    Restart(i64),
    /// Continue an earlier list from each target's first paragraph, in order: the list of
    /// the paragraph named, or the nearest earlier one over the same definition (a list
    /// continued by an earlier target counts as that earlier list).
    Continue(Option<String>),
    /// Change a level of the first one's list.
    Format(LevelFormat),
}

impl ListAction {
    /// Whether the action takes a step on the `k`th paragraph of the targets (`first`: the
    /// first its target holds): every paragraph for the item actions, each target's first
    /// for continue, the first of all for restart and format.
    fn steps(&self, k: usize, first: bool) -> bool {
        match self {
            ListAction::Add { .. } | ListAction::Remove | ListAction::Level(_) => true,
            ListAction::Continue(_) => first,
            ListAction::Restart(_) | ListAction::Format(_) => k == 0,
        }
    }
}

/// docx-agent's list edits on the paragraphs `targets` hold.
#[derive(Debug, Clone)]
pub struct ListEdit {
    /// Paragraphs, spans, tables, cells, sections: every paragraph they hold.
    pub targets: Vec<String>,
    /// What to do.
    pub action: ListAction,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What one paragraph's step did (docx-agent's `EditResult`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListStep {
    /// The paragraph's id after the step.
    pub id: ooxml_word_model::ids::BlockId,
    /// Whether it changed anything.
    pub changed: bool,
    /// The instances and definitions it removed (`num:3`, `abstractNum:2`).
    pub removed: Vec<String>,
    /// Ids its stamping renamed, old to new.
    pub renamed: ooxml_word_model::ids::Renames,
    /// What it notes.
    pub warnings: Vec<String>,
}

/// What a [`ListEdit`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListsEdited {
    /// Each step that changed something, in order.
    pub steps: Vec<ListStep>,
    /// Paragraphs `remove` found in no list (left as they were).
    pub listless: Vec<String>,
    /// Every target paragraph after the edit: its id now and its list.
    pub lists: Vec<(String, Option<Membership>)>,
}

/// The paragraphs `address` holds, in document order: an id followed when it was renamed,
/// and so a row's or cell's table, or a span's ends.
///
/// # Errors
/// [`EditError::Address`] when it names nothing, or nothing with paragraphs.
pub fn paragraphs_of(doc: &WordDoc, docx: &Docx, address: &str) -> Result<Vec<(String, usize)>> {
    let followed = doc.ids().follow_address(address);
    if let Some(found) = table_paragraphs(docx, &followed) {
        return Ok(found);
    }
    let found = scope_paragraphs(docx, &followed).or_else(|e| {
        if followed == address {
            Err(e)
        } else {
            scope_paragraphs(docx, address)
        }
    });
    let (part, indexes) = found.map_err(EditError::Address)?;
    if indexes.is_empty() {
        return Err(EditError::Address(ooxml_word_model::address::AddressError::invalid(
            format!("{address} holds no paragraphs"),
            Vec::new(),
        )));
    }
    Ok(indexes.into_iter().map(|k| (part.part.clone(), k)).collect())
}

/// A table's paragraphs as docx-agent's `paragraphs_of` takes them: each grid position's
/// cell's, row by row (a merged cell's at each position it covers); `None` when `address`
/// is not a table's id.
fn table_paragraphs(docx: &Docx, address: &str) -> Option<Vec<(String, usize)>> {
    use ooxml_word_model::address::{is_text_range, resolve, span_ends};
    if is_text_range(address) || !matches!(span_ends(address), Ok(None)) {
        return None;
    }
    let (story, entry) = resolve(docx, address).ok()?;
    let ooxml_word_model::ids::Entry::Table(t) = entry else {
        return None;
    };
    let table = story.ids.tables().get(t)?;
    let grid = ooxml_word_model::docx::tables::TableGrid::new(story, &table);
    let mut out = Vec::new();
    for row in 0..grid.row_count() {
        for column in 0..grid.column_count() {
            out.extend(
                grid.cell_paragraphs(row, column)
                    .into_iter()
                    .map(|k| (story.part.clone(), k)),
            );
        }
    }
    Some(out)
}

/// Stamping done by the first step that edits: the renames it made.
pub(crate) struct Stamped {
    renamed: Option<ooxml_word_model::ids::Renames>,
}

impl Stamped {
    /// Stamp the document if no step did yet; this step's renames.
    pub(crate) fn once(&mut self, sink: &mut crate::sink::EditSink<'_, '_>) -> Result<ooxml_word_model::ids::Renames> {
        if self.renamed.is_some() {
            return Ok(Vec::new());
        }
        let docx = sink.docx()?;
        let renamed = stamp_document(sink, &docx)?;
        self.renamed = Some(renamed.clone());
        Ok(renamed)
    }
}

impl Command<WordDoc> for ListEdit {
    type Out = ListsEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        for target in &self.targets {
            paragraphs_of(doc, &docx, target)?;
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<ListsEdited> {
        let mut sink = crate::sink::EditSink::new(tx, self.policy.clone());
        sink.read_everything();
        let docx = sink.docx()?;
        let mut places = Vec::new();
        // each paragraph, and whether it is the first its target holds
        let mut firsts = Vec::new();
        for target in &self.targets {
            for (k, (part, index)) in paragraphs_of(sink.doc(), &docx, target)?.into_iter().enumerate() {
                places.push(Place { part, index });
                firsts.push(k == 0);
            }
        }
        // every stepped paragraph checked before the first step: "not in a list" anywhere in the
        // target comes before a step's own refusal, whose way out would not work for the target
        let stepped: Vec<&Place> = places
            .iter()
            .zip(&firsts)
            .enumerate()
            .filter(|(k, (_, first))| self.action.steps(*k, **first))
            .map(|(_, (place, _))| place)
            .collect();
        for place in &stepped {
            steps::precheck(&docx, place, &self.action)?;
        }
        let mut stamped = Stamped { renamed: None };
        let mut out = ListsEdited::default();
        for place in stepped {
            match steps::step(&mut sink, &mut stamped, place, &self.action)? {
                steps::Stepped::Done(step) => out.steps.push(step),
                steps::Stepped::Listless(id) => out.listless.push(id),
            }
        }
        if !out.steps.is_empty() {
            crate::textbox::sync(&mut sink)?;
            crate::revision_ids::make_unique(&mut sink)?;
        }
        let docx = sink.docx()?;
        for place in &places {
            if let Some(read::At { id, .. }) = read::at(&docx, place) {
                out.lists.push((id, read::list_at(&docx, place)));
            }
        }
        Ok(out)
    }
}

/// The first paragraph `address` holds (an id given by the caller).
fn paragraphs_of_place(docx: &Docx, address: &str) -> Result<Place> {
    let (part, indexes) = scope_paragraphs(docx, address).map_err(EditError::Address)?;
    let index = indexes.first().copied().ok_or_else(|| {
        EditError::Address(ooxml_word_model::address::AddressError::invalid(
            format!("{address} holds no paragraphs"),
            Vec::new(),
        ))
    })?;
    Ok(Place {
        part: part.part.clone(),
        index,
    })
}
