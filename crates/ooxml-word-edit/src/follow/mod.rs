//! Owns: [`FollowSample`], copies made to follow a revised sample in one edit, as
//! docx-agent's `edit/follow.py` makes them (`Document.follow_sample`): the sample and the
//! copies located as the other tools take them (a cell or row address, or a paragraph in a
//! table where a table is wanted, names its table, with a note; an empty address, the
//! sample among its copies and a copy named twice are ignored with a note; an address that
//! names nothing is refused with the document's tables), the document stamped, then each
//! copy given the sample's table, grid, cell, paragraph and run formatting ([`table`],
//! [`paragraph`]), keeping its text, rows, columns, merges and lists; tracked as Word
//! records each (the paragraphs' and runs' as `w:pPrChange` and `w:rPrChange`, the table's
//! as `w:tblPrChange`, `w:tblGridChange` and `w:tcPrChange`), the rows' properties after
//! the record, untracked, as Word does not track them.
//!
//! Must not: write but through the sink, compare (what is left to follow is the tool's
//! comparison of the result), or change a copy's text, rows, columns, merges or lists.

mod command;
mod paragraph;
mod table;
mod take;

use crate::policy::RevisionPolicy;
use crate::sink::Numbered;
use crate::tables::shape::{grid_widths, own_rows};
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::address::resolve;
use ooxml_word_model::address::resolve::unknown;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;
use ooxml_xml::Element;
use std::sync::Arc;
use take::Styles;

/// The warning a tracked edit gives when it followed a row's properties.
const ROWS_UNTRACKED: &str = "Word does not track a row's height, breaking or header: applied untracked";
/// Tables a refusal names at most.
const SHAPES: usize = 20;

/// Copies of a sample table or paragraph made to follow it, one step.
#[derive(Clone)]
pub struct FollowSample {
    /// The sample's address.
    pub sample: String,
    /// The sample's document, at the version pinned for the call, where it is another
    /// (`None`: this one).
    pub source: Option<Arc<Docx>>,
    /// The copies' addresses, as asked.
    pub copies: Vec<String>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

impl std::fmt::Debug for FollowSample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FollowSample")
            .field("sample", &self.sample)
            .field("other_document", &self.source.is_some())
            .field("copies", &self.copies)
            .field("policy", &self.policy)
            .finish()
    }
}

/// What following did to one copy.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CopyFollowed {
    /// The address asked.
    pub asked: String,
    /// The copy's id (after stamping).
    pub id: String,
    /// The table's properties that changed, by `word_format_table`'s names.
    pub table: Vec<String>,
    /// Whether the grid's widths changed.
    pub widths: bool,
    /// Rows, cells, paragraphs and runs whose formatting changed.
    pub rows: usize,
    /// Cells.
    pub cells: usize,
    /// Paragraphs.
    pub paragraphs: usize,
    /// Runs.
    pub runs: usize,
    /// Paragraphs in a list where their sample paragraph is in none, or the other way.
    pub lists: Vec<String>,
    /// Sample styles this document neither has nor would add: name and kind.
    pub missing: Vec<(String, String)>,
}

impl CopyFollowed {
    /// Whether anything changed.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.table.is_empty() || self.widths || self.rows + self.cells + self.paragraphs + self.runs > 0
    }
}

/// What a [`FollowSample`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Followed {
    /// The sample's id as located.
    pub sample: String,
    /// Its id now (after stamping, where it is in this document).
    pub sample_now: String,
    /// Each copy, in the order asked.
    pub copies: Vec<CopyFollowed>,
    /// Ids stamping renamed, old to new.
    pub renamed: ooxml_word_model::ids::Renames,
    /// The notes and warnings.
    pub warnings: Vec<String>,
}

/// What one copy's following changed (docx-agent's `_Log`).
#[derive(Default)]
pub(super) struct Log {
    pub(super) table: Vec<String>,
    pub(super) widths: bool,
    pub(super) rows: usize,
    pub(super) cells: usize,
    pub(super) paragraphs: usize,
    pub(super) runs: usize,
    pub(super) list: Vec<Id>,
}

/// A block located: its part, place, id and element, and whether it is a table.
struct Located {
    part: String,
    path: Vec<u32>,
    id: String,
    element: Arc<Element>,
    table: bool,
}

impl Located {
    fn same_place(&self, other: &Located) -> bool {
        self.part == other.part && self.path == other.path
    }
}

/// The block `address` names (an alias followed where `doc` keeps it).
fn block(doc: Option<&WordDoc>, docx: &Docx, address: &str) -> std::result::Result<Located, String> {
    let followed = doc.map_or_else(|| address.to_owned(), |d| d.ids().follow(address));
    let found = (followed != address)
        .then(|| ooxml_word_model::address::direct(docx, &followed))
        .flatten()
        .map_or_else(|| resolve(docx, address), Ok)
        .map_err(|e| e.message)?;
    let (story, entry) = found;
    let located = match entry {
        Entry::Table(k) => story.ids.tables().get(k).map(|t| Located {
            part: story.part.clone(),
            path: t.path.to_vec(),
            id: t.id,
            element: t.element,
            table: true,
        }),
        Entry::Paragraph(k) => story.ids.paragraphs().get(k).map(|p| Located {
            part: story.part.clone(),
            path: p.path.to_vec(),
            id: p.id,
            element: p.element,
            table: false,
        }),
    };
    located.ok_or_else(|| unknown(docx, address).message)
}

/// The document's body tables as `t:X (rows x columns)`, those with `like`'s number of
/// columns first (docx-agent's `_shapes`).
fn shapes(docx: &Docx, like: Option<&Arc<Element>>) -> Vec<String> {
    let columns = |element: &Arc<Element>| {
        let work = Work::of(element, Vec::new());
        (
            own_rows(&work, work.root()).len(),
            grid_widths(&work, work.root()).len(),
        )
    };
    let wanted = like.map(|l| columns(l).1);
    let mut found: Vec<(bool, String)> = docx
        .part(&docx.main)
        .map(|story| {
            story
                .ids
                .tables()
                .iter()
                .map(|t| {
                    let (rows, count) = columns(&t.element);
                    (
                        wanted.is_some_and(|w| w != count),
                        format!("{} ({rows} rows x {count} columns)", t.id),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|f| f.0);
    let total = found.len();
    let mut out: Vec<String> = found.into_iter().take(SHAPES).map(|f| f.1).collect();
    if total > SHAPES {
        out.push(format!("{} more (word_read lists them)", total - SHAPES));
    }
    out
}

/// What [`locate`] is asked for: the field named in a refusal, the kind wanted (`None`:
/// either; a paragraph in a table then names its table) and the block whose columns a
/// refusal lists first.
struct Asked<'a> {
    field: &'a str,
    paragraph: bool,
    like: Option<&'a Arc<Element>>,
}

/// The block `identifier` names as the other tools take it (docx-agent's `_locate`).
fn locate(
    doc: Option<&WordDoc>,
    docx: &Docx,
    identifier: &str,
    asked: &Asked<'_>,
    notes: &mut Vec<String>,
) -> Result<Located> {
    let mut address = identifier.trim().to_owned();
    let in_table = address
        .split_once('/')
        .filter(|(t, r)| t.starts_with("t:") && (r.starts_with('c') || r.starts_with('r')))
        .map(|(t, _)| t.to_owned());
    if let Some(table) = in_table {
        notes.push(format!("{address} is in table {table}: the table follows"));
        address = table;
    }
    let mut found = block(doc, docx, &address).map_err(|message| {
        let tables = shapes(docx, asked.like);
        let listed = if tables.is_empty() {
            "none".to_owned()
        } else {
            tables.join(", ")
        };
        EditError::Refused(format!("{}: {message}; the tables here: {listed}", asked.field))
    })?;
    if !found.table && !asked.paragraph {
        let story = docx.part(&found.part);
        let holder = (1..found.path.len())
            .rev()
            .find_map(|n| story.and_then(|s| s.ids.table_at(found.path.get(..n)?)));
        if let Some(table) = holder {
            notes.push(format!(
                "{identifier} is a paragraph in table {}: the table follows",
                table.id
            ));
            found = Located {
                part: found.part,
                path: table.path.to_vec(),
                id: table.id,
                element: table.element,
                table: true,
            };
        }
    }
    Ok(found)
}

/// The place of `node` below the work's root as the document's index reads places: each
/// step counts the text nodes before it.
fn place_below(work: &Work, node: Id) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = node;
    while let Some(parent) = work.parent(at) {
        let mut k = usize::from(!work.text(parent).is_empty());
        for sibling in work.children(parent) {
            if *sibling == at {
                break;
            }
            k += 1 + usize::from(!work.tail(*sibling).is_empty());
        }
        out.push(u32::try_from(k).unwrap_or(u32::MAX));
        at = parent;
    }
    out.reverse();
    out
}

/// One numbering of revision ids after another.
fn joined(first: Option<Numbered>, then: Numbered) -> Numbered {
    match first {
        Some(first) => Numbered {
            base: first.base,
            from: first.from,
            to: then.to,
        },
        None => then,
    }
}
