//! Owns: [`Review`], accepting or rejecting tracked changes as docx-agent's `word_changes`
//! does (`tools/review.py` `_selection` and `_settle`, over `revisions/review.py`): the
//! changes chosen by id (a change's or a record's), all of them, or by author, kind and
//! place -- the last one change at a time, as a reviewer settles them -- each settled
//! with the records that go with it, one history step for the whole call; and what it
//! settled ([`Reviewed`]).
//!
//! Must not: word a tool's reply (the tool layer does), or read the clock.

mod apply;
mod blocks;
mod cleanup;
mod describing;
mod edges;
mod grid;
mod part;
pub(crate) mod space;
mod step;
mod within;
pub(crate) mod xml;

pub use blocks::{RemovedBlocks, RowGone, TableGone, Typed};
pub(crate) use cleanup::authors_in_use;

use crate::doc::WordDoc;
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::{scope_paragraphs, span_ends, AddressError};
use ooxml_word_model::docx::changes::{changes, LogicalChange};
use ooxml_word_model::docx::revisions::records;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;
use std::collections::{BTreeMap, BTreeSet};

/// Which changes a review settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// These changes (`rev:` ids: a change's id names all its records, a record's itself).
    Ids(Vec<String>),
    /// Every change.
    All,
    /// The changes by `author`, of `kind`, in `within`, one at a time.
    Matching {
        /// Only this author's.
        author: Option<String>,
        /// Only this kind (`insertion`, `deletion`, `replacement`, `move`, `formatting`,
        /// `table`, `section`).
        kind: Option<String>,
        /// Only in this block or span.
        within: Option<String>,
    },
}

/// Accept or reject tracked changes.
#[derive(Debug, Clone)]
pub struct Review {
    /// Accept (else reject).
    pub accept: bool,
    /// Which.
    pub selection: Selection,
}

/// A revision record as the review counts it: its part, element, `w:id`, author and date.
pub type Mark = (String, String, Option<String>, Option<String>, Option<String>);

/// What a review did.
#[derive(Debug, Clone, Default)]
pub struct Reviewed {
    /// How many revision records it settled (the document's before, less after).
    pub records: usize,
    /// The records settled, each with how many such there were.
    pub settled: BTreeMap<Mark, usize>,
    /// Ids renamed (old, new), in order.
    pub renamed: Vec<(String, String)>,
    /// Ids removed, in order.
    pub removed: Vec<String>,
    /// What it says to look at.
    pub warnings: Vec<String>,
    /// The blocks each step removed.
    pub blocks: Vec<RemovedBlocks>,
    /// How many steps changed something (a step per change, for a selection by author,
    /// kind or place).
    pub steps: usize,
}

/// Every revision record of `docx` as a mark, counted.
pub fn record_marks(docx: &Docx) -> BTreeMap<Mark, usize> {
    let mut out = BTreeMap::new();
    for r in records(docx) {
        let get = |local: &str| r.element.attribute(Some(crate::names::W), local).map(str::to_owned);
        let mark = (
            r.part.clone(),
            r.element.name().local().to_owned(),
            get("id"),
            get("author"),
            get("date"),
        );
        *out.entry(mark).or_insert(0) += 1;
    }
    out
}

/// The record ids `ids` name (`Selection::Ids`): a change's id stands for its records, a
/// record's for itself.
///
/// # Errors
/// [`EditError::Address`] (not found) naming every id the document has no change or
/// record of, the changes' ids as the options.
pub fn wanted_records(docx: &Docx, ids: &[String]) -> Result<Vec<String>> {
    let grouped = changes(docx);
    let known: BTreeSet<String> = records(docx).into_iter().map(|r| r.id).collect();
    let mut wanted: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for id in ids {
        let members: Vec<String> = match grouped.iter().find(|c| c.id() == id) {
            Some(change) => change.records.clone(),
            None if known.contains(id) => vec![id.clone()],
            None => Vec::new(),
        };
        if members.is_empty() {
            missing.push(id.clone());
        }
        for m in members {
            if !wanted.contains(&m) {
                wanted.push(m);
            }
        }
    }
    if !missing.is_empty() {
        let named: Vec<String> = missing.iter().map(|m| ooxml_base::pyfmt::repr_str(m)).collect();
        let listed: Vec<String> = grouped.iter().map(|c| c.id().to_owned()).collect();
        return Err(EditError::Address(AddressError::not_found(
            format!(
                "no change or record {}: the ids are word_changes list's (changes) or list detail records' \
                     (records), as they stand now",
                named.join(", ")
            ),
            listed.into_iter().take(50).collect(),
        )));
    }
    Ok(wanted)
}

/// The paragraphs and records a `within` names (`_within_paragraphs`, and the records
/// `_revisions_within` finds).
///
/// # Errors
/// The address resolver's refusal.
pub fn place_of(docx: &Docx, within: &str) -> Result<Place> {
    // the records first, as docx-agent's _selection reads them first: a refusal is theirs
    let records = records_within(docx, within)?;
    let (part, indexes) = scope_paragraphs(docx, within).map_err(EditError::Address)?;
    let ids: BTreeSet<String> = indexes
        .iter()
        .filter_map(|k| part.ids.paragraphs().get(*k).map(|p| p.id))
        .collect();
    Ok((ids, records))
}

/// The ids of the records a `within` holds (`_revisions_within`): a story's every record;
/// a table's by its volatile id (`t@...`) every record in it; text (an address with `@`:
/// a range, a position, a paragraph's or cell's text) the records in it, by their
/// paragraph and characters; a block's (paragraph or table) every record in it; else
/// every record in the paragraphs the address covers.
///
/// # Errors
/// The address resolver's refusal.
pub fn records_within(docx: &Docx, within: &str) -> Result<BTreeSet<String>> {
    if let Some(story) = docx.story(within) {
        return Ok(records(docx)
            .into_iter()
            .filter(|r| r.part == story.part)
            .map(|r| r.id)
            .collect());
    }
    let volatile_table = within.starts_with("t@") && ooxml_word_model::address::direct(docx, within).is_some();
    if within.contains('@') && !volatile_table {
        return within::records_in_text(docx, records(docx), within);
    }
    if matches!(span_ends(within), Ok(Some(_))) {
        // one block here, not a span: the resolver refuses it with the span's ends
        ooxml_word_model::address::resolve(docx, within).map_err(EditError::Address)?;
    }
    let (part, indexes) = scope_paragraphs(docx, within).map_err(EditError::Address)?;
    let paths: Vec<Box<[u32]>> = indexes
        .iter()
        .filter_map(|k| part.ids.paragraphs().get(*k).map(|p| p.path))
        .collect();
    let block = ooxml_word_model::address::direct(docx, within).and_then(|(story, entry)| match entry {
        Entry::Paragraph(k) => story.ids.paragraphs().get(k).map(|p| p.path),
        Entry::Table(k) => story.ids.tables().get(k).map(|t| t.path),
    });
    let mut found = BTreeSet::new();
    for r in records(docx).into_iter().filter(|r| r.part == part.part) {
        let inside = match &block {
            Some(path) => r.path.starts_with(path),
            None => paths.iter().any(|p| r.path.starts_with(p)),
        };
        if inside {
            found.insert(r.id);
        }
    }
    Ok(found)
}

/// The paragraphs and records a `within` names.
pub type Place = (BTreeSet<String>, BTreeSet<String>);

/// Whether `change` is one a selection by `author`, `kind` and place settles.
#[must_use]
pub fn matches(change: &LogicalChange, author: Option<&str>, kind: Option<&str>, place: Option<&Place>) -> bool {
    author.is_none_or(|a| change.author.as_deref() == Some(a))
        && kind.is_none_or(|k| change.kind == k)
        && inside(change, place)
}

/// Whether `change` is in the place `(paragraphs, records)`.
fn inside(change: &LogicalChange, place: Option<&Place>) -> bool {
    match place {
        None => true,
        Some((paragraphs, found)) => {
            change.paragraph_ids.iter().any(|p| paragraphs.contains(p))
                || change.records.iter().any(|r| found.contains(r))
        }
    }
}

impl Command<WordDoc> for Review {
    type Out = Reviewed;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let docx = doc.docx()?;
        match &self.selection {
            Selection::Ids(ids) => wanted_records(&docx, ids).map(|_| ()),
            Selection::Matching {
                within: Some(within), ..
            } => place_of(&docx, within).map(|_| ()),
            _ => Ok(()),
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Reviewed> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let docx = sink.docx()?;
        let before = record_marks(&docx);
        let mut steps: Vec<step::Step> = Vec::new();
        match &self.selection {
            Selection::Ids(ids) => {
                let wanted = wanted_records(&docx, ids)?;
                steps.push(step::step(&mut sink, self.accept, Some(&wanted))?);
            }
            Selection::All => steps.push(step::step(&mut sink, self.accept, None)?),
            Selection::Matching { author, kind, within } => {
                let at = within.as_deref().map(|w| place_of(&docx, w)).transpose()?;
                let mut seen: BTreeSet<ChangeKey> = BTreeSet::new();
                loop {
                    let now = sink.docx()?;
                    let next = changes(&now).into_iter().find(|c| {
                        matches(c, author.as_deref(), kind.as_deref(), at.as_ref()) && !seen.contains(&key(c))
                    });
                    let Some(change) = next else { break };
                    seen.insert(key(&change));
                    steps.push(step::step(&mut sink, self.accept, Some(&change.records))?);
                }
            }
        }
        // docx-agent's `_follow_review`: the tables' records kept true
        crate::provenance::follow_review(&mut sink, self.accept)?;
        let after = record_marks(&*sink.docx()?);
        let total = |m: &BTreeMap<Mark, usize>| m.values().sum::<usize>();
        let mut settled = BTreeMap::new();
        for (mark, n) in &before {
            let left = after.get(mark).copied().unwrap_or(0);
            if *n > left {
                settled.insert(mark.clone(), n - left);
            }
        }
        let mut out = Reviewed {
            records: total(&before).saturating_sub(total(&after)),
            settled,
            ..Reviewed::default()
        };
        for s in steps {
            if s.count > 0 {
                out.steps += 1;
            }
            for r in s.removed {
                if !out.removed.contains(&r) {
                    out.removed.push(r);
                }
            }
            for (old, new) in s.renamed {
                match out.renamed.iter_mut().find(|(o, _)| *o == old) {
                    Some(slot) => slot.1 = new,
                    None => out.renamed.push((old, new)),
                }
            }
            for w in s.warnings {
                if !out.warnings.contains(&w) {
                    out.warnings.push(w);
                }
            }
            out.blocks.push(s.blocks);
        }
        Ok(out)
    }
}

/// What tells two changes apart while settling them one at a time: kind, author,
/// records, text and old text.
type ChangeKey = (String, Option<String>, Vec<String>, String, String);

/// A change's [`ChangeKey`].
fn key(c: &LogicalChange) -> ChangeKey {
    (
        c.kind.to_owned(),
        c.author.clone(),
        c.records.clone(),
        c.text.clone(),
        c.old_text.clone(),
    )
}
