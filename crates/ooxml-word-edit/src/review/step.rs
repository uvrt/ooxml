//! Owns: one review step -- docx-agent's `RevisionOps._review` (`revisions/review.py`): the
//! records asked for, the document stamped first, each part's records expanded and
//! applied ([`PartReview`]), joined paragraphs' ids aliased to the paragraph they went
//! into, then the cleanups across parts (notes and comments whose references went, unused
//! numbering, relationships released, text boxes' fallbacks, the people part, comments
//! describing a structural table change left with none, data-bound controls' nodes) and
//! what it removed ([`super::blocks`]).
//!
//! Must not: choose records (the command does) or write but through the sink.

use super::blocks::{blocks_gone, blocks_now, RemovedBlocks};
use super::part::PartReview;
use super::space::Space;
use crate::comments::delete::{comment_part_relationships, delete, none_left};
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::work::Id;
use crate::{EditError, Result};
use ooxml_word_model::docx::changes::change_groups;
use ooxml_word_model::docx::comments::comments;
use ooxml_word_model::docx::revisions::{records, Record};
use ooxml_word_model::docx::Docx;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What one step did.
#[derive(Debug, Clone, Default)]
pub(crate) struct Step {
    /// How many records it was asked to settle.
    pub(crate) count: usize,
    /// Renames: the stamping's (old to new) and the tables a removed first row renamed.
    pub(crate) renamed: Vec<(String, String)>,
    /// Every id removed.
    pub(crate) removed: Vec<String>,
    /// What it says to look at (a row's last cell's text moved out).
    pub(crate) warnings: Vec<String>,
    /// The blocks it removed.
    pub(crate) blocks: RemovedBlocks,
}

/// Settle the records `wanted` (record ids; `None`: every record) by `accept`.
///
/// # Errors
/// [`EditError::Refused`] for an id the document has no record of.
pub(crate) fn step(sink: &mut EditSink<'_, '_>, accept: bool, wanted: Option<&[String]>) -> Result<Step> {
    let docx0 = sink.docx()?;
    let all = records(&docx0);
    let selected: Vec<Record> = match wanted {
        None => all,
        Some(ids) => {
            let mut out = Vec::new();
            for id in ids {
                let found: Vec<&Record> = all.iter().filter(|r| r.id == *id).collect();
                if found.is_empty() {
                    return Err(EditError::Refused(format!("no revision {id:?}")));
                }
                out.extend(found.into_iter().cloned());
            }
            out
        }
    };
    if selected.is_empty() {
        return Ok(Step::default());
    }
    let mut parts: Vec<String> = Vec::new();
    for r in &selected {
        if !parts.contains(&r.part) {
            parts.push(r.part.clone());
        }
    }
    // a review reads the whole document: what it settles, the records' groups, the
    // numbering and authors left, the notes and comments referenced
    sink.read_everything();
    let known = blocks_now(&docx0);
    let renames = stamp_document(sink, &docx0)?;
    let docx = sink.docx()?;
    let mut run = Run {
        accept,
        held: sink.doc().clone(),
        placeholder_texts: super::placeholders::placeholder_texts(sink.doc(), &docx.main)?,
        space: Space::default(),
        gathered: Gathered::default(),
    };
    let before = Before {
        nums: super::cleanup::referenced_nums(sink.doc(), &docx, &run.space)?,
        authors: super::cleanup::authors_in_use(&docx),
        ids: crate::revision_ids::document_counts(&docx),
        known,
        renames,
        count: selected.len(),
    };
    for part in &parts {
        let mine: Vec<&Record> = selected.iter().filter(|r| r.part == *part).collect();
        run.review_part(sink, &docx, part, &mine)?;
    }
    run.finish(sink, &docx, before)
}

/// One step's run: its copies and what its parts' reviews gathered.
struct Run {
    accept: bool,
    /// The version the step reviews (holding the document's shared tables).
    held: crate::doc::WordDoc,
    /// The glossary's placeholder texts, for the controls a review empties.
    placeholder_texts: super::placeholders::PlaceholderTexts,
    space: Space,
    gathered: Gathered,
}

/// What the step read before it changed anything.
struct Before {
    known: Vec<super::blocks::Known>,
    renames: Vec<(String, String)>,
    nums: BTreeSet<i64>,
    authors: BTreeSet<String>,
    /// Every revision id's count as the step began (its renumbering of repeats after).
    ids: HashMap<String, usize>,
    count: usize,
}

/// What the parts' reviews gathered for the cleanups.
#[derive(Default)]
struct Gathered {
    removed_ids: Vec<String>,
    joined: HashMap<String, Vec<(Id, Id)>>,
    released: BTreeMap<String, BTreeSet<String>>,
    dropped_comments: BTreeSet<String>,
    dropped_notes: BTreeSet<(String, String)>,
    moved_out: Vec<(String, String, Vec<Id>)>,
}

impl Run {
    /// Review one part's records.
    fn review_part(&mut self, sink: &mut EditSink<'_, '_>, docx: &Docx, part: &str, mine: &[&Record]) -> Result<()> {
        let (space, gathered, accept) = (&mut self.space, &mut self.gathered, self.accept);
        let texts = &self.placeholder_texts;
        let orders = self.held.shared().facts.orders();
        let Some(story) = docx.part(part) else {
            return Err(EditError::NoPart(part.to_owned()));
        };
        let groups = record_groups(docx, part, mine);
        space.work(sink.doc(), part)?;
        let elements: Vec<Id> = mine.iter().filter_map(|r| space.locate(part, &r.path)).collect();
        let mut group_of: HashMap<Id, Vec<Id>> = HashMap::new();
        for g in &groups {
            let ids: Vec<Id> = g.iter().filter_map(|p| space.locate(part, p)).collect();
            for id in &ids {
                group_of.insert(*id, ids.clone());
            }
        }
        let ids_before: HashMap<Id, String> = story
            .ids
            .paragraphs()
            .iter()
            .filter_map(|p| Some((space.locate(part, &p.path)?, p.id)))
            .collect();
        let work = space.get_mut(part).ok_or_else(|| EditError::NoPart(part.to_owned()))?;
        let mut review = PartReview::new(work, accept, (orders, texts), group_of);
        let group = review.expand(&elements);
        review.run(&group);
        for p in &review.removed_paragraphs {
            if let Some(id) = ids_before.get(p) {
                gathered.removed_ids.push(id.clone());
            }
        }
        let mut aliases = Vec::new();
        for (gone, survivor) in &review.joined {
            if let (Some(a), Some(b)) = (ids_before.get(gone), ids_before.get(survivor)) {
                aliases.push((a.clone(), b.clone()));
            }
        }
        gathered
            .joined
            .entry(part.to_owned())
            .or_default()
            .extend(review.joined.iter().copied());
        if !review.released.is_empty() {
            gathered
                .released
                .entry(part.to_owned())
                .or_default()
                .extend(review.released.iter().cloned());
        }
        for (text, paragraphs) in &review.moved_out {
            gathered
                .moved_out
                .push((part.to_owned(), text.clone(), paragraphs.clone()));
        }
        gathered
            .dropped_comments
            .extend(review.dropped_comments.iter().cloned());
        gathered.dropped_notes.extend(review.dropped_notes.iter().cloned());
        space.mark_dirty(part);
        for alias in aliases {
            sink.rename(vec![alias])?;
        }
        Ok(())
    }

    /// The cleanups after the parts' reviews, and the step's result.
    fn finish(self, sink: &mut EditSink<'_, '_>, docx: &Docx, before: Before) -> Result<Step> {
        let Run {
            mut space, gathered, ..
        } = self;
        let Before {
            known,
            renames,
            nums: before_nums,
            authors,
            ids,
            count,
        } = before;
        clean(sink, docx, &mut space, &gathered, &before_nums, &authors, &ids)?;
        let after = sink.docx()?;
        let now = |part: &str, path: &[u32]| -> Option<String> {
            let story = after.part(part)?;
            story.ids.at(path).and_then(|e| story.ids.id_of(e))
        };
        let blocks = blocks_gone(&space, &known, &gathered.joined, &now);
        let warnings = gathered
            .moved_out
            .iter()
            .map(|(part, text, ps)| {
                let said: Vec<String> = ps
                    .iter()
                    .filter_map(|p| space.path_of(part, *p))
                    .filter_map(|path| now(part, &path))
                    .collect();
                moved_out_warning(text, &said)
            })
            .collect();
        let named: BTreeSet<String> = blocks
            .flat
            .iter()
            .cloned()
            .chain(
                blocks
                    .flat
                    .iter()
                    .filter_map(|i| renames.iter().find(|(old, _)| old == i).map(|(_, new)| new.clone())),
            )
            .collect();
        let mut removed = blocks.flat.clone();
        removed.extend(gathered.removed_ids.iter().filter(|i| !named.contains(*i)).cloned());
        let mut renamed: Vec<(String, String)> =
            renames.into_iter().filter(|(old, _)| !removed.contains(old)).collect();
        for (old, new) in &blocks.tables_renamed {
            match renamed.iter_mut().find(|(o, _)| o == old) {
                Some(slot) => slot.1 = new.clone(),
                None => renamed.push((old.clone(), new.clone())),
            }
        }
        if !blocks.tables_renamed.is_empty() {
            sink.rename(blocks.tables_renamed.clone())?;
        }
        Ok(Step {
            count,
            renamed,
            removed,
            warnings,
            blocks,
        })
    }
}

/// The part's records grouped as `word_changes` lists them (a table's structure edits),
/// read only when a cell's record is settled (docx-agent's `_edit_of`): each group's
/// records' places.
fn record_groups(docx: &Docx, part: &str, mine: &[&Record]) -> Vec<Vec<Box<[u32]>>> {
    let structural = mine.iter().any(|r| {
        r.element.name().namespace() == Some(crate::names::W)
            && matches!(
                r.element.name().local(),
                "cellIns" | "cellDel" | "cellMerge" | "tcPrChange"
            )
    });
    if !structural {
        return Vec::new();
    }
    let by_id: HashMap<String, Box<[u32]>> = records(docx)
        .into_iter()
        .filter(|r| r.part == part)
        .map(|r| (r.id, r.path))
        .collect();
    change_groups(docx)
        .into_iter()
        .map(|g| g.iter().filter_map(|id| by_id.get(id).cloned()).collect::<Vec<_>>())
        .filter(|g: &Vec<Box<[u32]>>| !g.is_empty())
        .collect()
}

/// The cleanups across parts after the parts' reviews, then the copies written.
fn clean(
    sink: &mut EditSink<'_, '_>,
    docx: &Docx,
    space: &mut Space,
    gathered: &Gathered,
    before_nums: &BTreeSet<i64>,
    authors: &BTreeSet<String>,
    ids_before: &HashMap<String, usize>,
) -> Result<()> {
    let doc = sink.doc().clone();
    let mut released = gathered.released.clone();
    if !gathered.dropped_notes.is_empty() {
        for (part, ids) in super::cleanup::drop_notes(&doc, docx, space, &gathered.dropped_notes)? {
            released.entry(part).or_default().extend(ids);
        }
    }
    let mut drop_comment_parts = false;
    let listed = comments(docx);
    let mut gone = BTreeSet::new();
    for w_id in &gathered.dropped_comments {
        if let Some(c) = listed.iter().find(|c| c.w_id.as_deref() == Some(w_id.as_str())) {
            delete(&doc, docx, space, &listed, &c.id.clone(), &mut gone)?;
            drop_comment_parts |= none_left(&doc, docx, space)?;
        }
    }
    for id in super::describing::resolved(&doc, docx, space, &listed, &gone)? {
        delete(&doc, docx, space, &listed, &id, &mut gone)?;
        drop_comment_parts |= none_left(&doc, docx, space)?;
    }
    super::cleanup::reap_numbering(&doc, docx, space, before_nums)?;
    space.finish(sink)?;
    for (part, ids) in &released {
        let ids: Vec<String> = ids.iter().cloned().collect();
        sink.release(part, &ids)?;
    }
    if drop_comment_parts {
        let rels = comment_part_relationships(docx);
        sink.release(&docx.main, &rels)?;
    }
    crate::textbox::sync_all(sink)?;
    crate::revision_ids::unique_since(sink, ids_before)?;
    for author in authors {
        sink.people(None, Some(author))?;
    }
    crate::controls::sync_bound_controls(sink)
}

/// The warning for a row's last cell's text moved out after the row.
fn moved_out_warning(text: &str, said: &[String]) -> String {
    let shortened: String = if text.chars().count() <= 60 {
        text.to_owned()
    } else {
        let mut s: String = text.chars().take(59).collect();
        s.push('\u{2026}');
        s
    };
    let place = if said.is_empty() {
        "paragraphs of its own".to_owned()
    } else {
        said.join(", ")
    };
    format!(
        "a row's last cell whose last paragraph mark was deleted went; its text ({}) is now after the row in {place}, \
         the table split there: Word's Accept All takes that text out of view (word_changes reject keeps it in the \
         cell; undo brings the change back)",
        ooxml_base::pyfmt::repr_str(&shortened)
    )
}
