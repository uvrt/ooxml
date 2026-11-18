//! Owns: the text edits within a paragraph that docx-agent makes through `_replace_ranges`
//! -- [`InsertText`] (`Document.insert_text`: new text at a position, in the formatting of
//! the character before it), [`DeleteText`] (`delete_text`: a range's text removed) and
//! [`ReplaceText`] (`Document.replace`: every place a text occurs, replaced) -- each range
//! checked before anything changes, the document stamped, the ranges replaced from the last
//! to the first (tracked or not, by the policy), what they removed outright released, the
//! people part kept, and each paragraph written given a new text id.
//!
//! Must not: write but through the sink, or read outside what it records reading: the
//! paragraphs' own blocks, the ledger seeds it draws from.

use crate::doc::{Draw, DrawFrom, Key, WordDoc};
use crate::names::{W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::set_text::{find, Found};
use crate::sink::{annotation_draws, EditSink, Keep, Numbered};
use crate::stamping::{stamp_document, w14_prefix};
use crate::text::{atoms, paragraph_text, write::check_text};
use crate::work::{scope_above, Work};
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::parse_range;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::QName;
use std::sync::Arc;

/// Text put at a position (`p:A@5`, or a paragraph: its start).
#[derive(Debug, Clone)]
pub struct InsertText {
    /// The position.
    pub at: String,
    /// The text (no `\n`).
    pub text: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// A range's text deleted (`p:A@4:11`, or a paragraph: all of its text).
#[derive(Debug, Clone)]
pub struct DeleteText {
    /// The range.
    pub range: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// Every place a text occurs replaced (plain text, case-sensitive or not, within one
/// paragraph each; the first `count` when given).
#[derive(Debug, Clone)]
pub struct ReplaceText {
    /// The text looked for.
    pub find: String,
    /// What replaces it.
    pub with: String,
    /// Whether case matters.
    pub case: bool,
    /// At most this many, in document order.
    pub count: Option<usize>,
    /// How the replacement takes its formatting.
    pub keep: Keep,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a range edit did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangesEdited {
    /// The first range's paragraph's id after the edit.
    pub id: Option<String>,
    /// How many ranges were replaced.
    pub count: usize,
    /// Ids the edit's stamping renamed, old to new, in the order made.
    pub renamed: Vec<(String, String)>,
}

/// One range to replace: its paragraph, its characters, the new text.
struct Plan {
    found: Found,
    /// The paragraph's place among its story's paragraphs (for the order of the plans).
    index: usize,
    /// Its part's place among the stories.
    story: usize,
    start: usize,
    end: usize,
    text: String,
}

/// A range as docx-agent's `parse_range` reads one: `(paragraph, start, end)`.
fn range_of(doc: &WordDoc, docx: &Docx, address: &str, position: bool) -> Result<(Found, usize, usize)> {
    if let Some(range) = parse_range(address) {
        let found = find(doc, docx, &range.start_id)?;
        let end_found = find(doc, docx, &range.end_id)?;
        if end_found.part != found.part || end_found.path != found.path {
            return Err(EditError::Unsupported(format!(
                "{address}: a range across paragraphs (phase P2c)"
            )));
        }
        let end = if position { range.start } else { range.end };
        return Ok((found, range.start, end));
    }
    // a paragraph's whole text (a cell's, a bookmark's, a link's: not yet)
    let found = find(doc, docx, address)?;
    let length = atoms(&Work::of(&found.element, Vec::new()), 0).len();
    Ok((found, 0, if position { 0 } else { length }))
}

/// The story and paragraph places of `found`, for ordering.
fn places(docx: &Docx, found: &Found) -> (usize, usize) {
    let story = docx
        .parts
        .iter()
        .position(|p| p.part == found.part)
        .unwrap_or(usize::MAX);
    let index = docx
        .part(&found.part)
        .and_then(|p| p.ids.paragraph_at(&found.path))
        .map_or(0, |p| p.index);
    (story, index)
}

fn plan(doc: &WordDoc, docx: &Docx, (found, start, end): (Found, usize, usize), text: &str) -> Result<Plan> {
    let (story, index) = places(docx, &found);
    let work = Work::of(&found.element, Vec::new());
    let items = atoms(&work, work.root());
    if end > items.len() || start > end {
        return Err(EditError::Span(format!(
            "{}@{start}:{end} is outside its paragraphs' text",
            found.id
        )));
    }
    crate::inline::check_span(&work, &items, start, end, true)
        .map_err(|e| EditError::Span(format!("{}@{start}:{end}: {e}", found.id)))?;
    let _ = doc;
    Ok(Plan {
        found,
        index,
        story,
        start,
        end,
        text: text.to_owned(),
    })
}

/// Every place `find` occurs in a paragraph's current text, in document order (docx-agent's
/// `_matches` for plain text within paragraphs).
fn matches(docx: &Docx, wanted: &str, case: bool) -> Vec<(String, usize, usize)> {
    // Python's IGNORECASE on a literal: characters equal, or equal lower-cased one for one
    let lower = |c: char| {
        let mut l = c.to_lowercase();
        match (l.next(), l.next()) {
            (Some(one), None) => one,
            _ => c,
        }
    };
    let same = |a: &char, b: &char| a == b || (!case && lower(*a) == lower(*b));
    let needle: Vec<char> = wanted.chars().collect();
    let mut out = Vec::new();
    for story in &docx.parts {
        for paragraph in story.ids.paragraphs().iter() {
            let work = Work::of(&paragraph.element, Vec::new());
            let text: Vec<char> = paragraph_text(&work, work.root()).chars().collect();
            let mut k = 0;
            while k + needle.len() <= text.len() {
                let here = text.get(k..k + needle.len()).unwrap_or_default();
                if here.iter().zip(&needle).all(|(a, b)| same(a, b)) {
                    out.push((paragraph.id.clone(), k, k + needle.len()));
                    k += needle.len();
                } else {
                    k += 1;
                }
            }
        }
    }
    out
}

/// docx-agent's `_replace_ranges`: the plans ordered last first, overlaps refused, the
/// document stamped, each paragraph's ranges replaced in one copy of it, then the people
/// part, the text ids, the namespaces they need.
fn replace_ranges(sink: &mut EditSink<'_, '_>, mut plans: Vec<Plan>, keep: Keep) -> Result<RangesEdited> {
    if plans.is_empty() {
        return Ok(RangesEdited {
            id: None,
            count: 0,
            renamed: Vec::new(),
        });
    }
    for plan in &plans {
        check_text(&plan.text)?;
    }
    plans.sort_by_key(|p| (p.story, p.index, p.start));
    plans.reverse();
    for pair in plans.windows(2) {
        if let [later, earlier] = pair {
            if earlier.story == later.story && (earlier.index, earlier.end) > (later.index, later.start) {
                return Err(EditError::Span("the ranges overlap".to_owned()));
            }
        }
    }
    let count = plans.len();
    let docx = sink.docx()?;
    for plan in &plans {
        sink.read(Key::Block {
            part: Arc::from(plan.found.part.as_str()),
            id: Arc::from(plan.found.id.as_str()),
        });
    }
    let renamed = stamp_document(sink, &docx)?;
    let first = plans.last().map(|p| renamed_id(&renamed, &p.found.id));
    let mut dated: Vec<String> = Vec::new();
    let mut released: Vec<(String, Vec<String>)> = Vec::new();
    let mut gave = false;
    let mut k = 0;
    while k < plans.len() {
        // a paragraph's ranges (they are together, last first) in one copy of it
        let same = |p: &Plan| {
            plans
                .get(k)
                .is_some_and(|q| q.found.part == p.found.part && q.found.path == p.found.path)
        };
        let group: Vec<&Plan> = plans.iter().skip(k).take_while(|p| same(p)).collect();
        k += group.len();
        let Some(written) = write_paragraph(sink, &group, keep, &renamed)? else {
            break;
        };
        gave |= written.gave;
        if written.dated && !dated.contains(&written.part) {
            dated.push(written.part.clone());
        }
        released.push((written.part, written.released));
    }
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        sink.people(gave.then_some(tracking.author.as_str()), Some(&tracking.author))?;
        for part in &dated {
            sink.ensure_w14(part, false, &[("w16du", W16DU)])?;
        }
    }
    for (part, ids) in &released {
        sink.release(part, ids)?;
    }
    for part in plans
        .iter()
        .map(|p| p.found.part.clone())
        .collect::<std::collections::BTreeSet<_>>()
    {
        sink.ensure_w14(&part, false, &[])?;
    }
    crate::textbox::sync(sink)?;
    crate::revision_ids::make_unique(sink)?;
    Ok(RangesEdited {
        id: first,
        count,
        renamed,
    })
}

/// What writing one paragraph's ranges did.
struct Written {
    part: String,
    released: Vec<String>,
    dated: bool,
    gave: bool,
}

/// One paragraph's ranges (`group`, last first) replaced in one copy of it, its revision
/// ids and new text ids drawn where the change is applied.
fn write_paragraph(
    sink: &mut EditSink<'_, '_>,
    group: &[&Plan],
    keep: Keep,
    renamed: &[(String, String)],
) -> Result<Option<Written>> {
    let Some(head) = group.first() else { return Ok(None) };
    let (part, path) = (head.found.part.clone(), head.found.path.clone());
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let element = at_path(tree.root(), &path).ok_or_else(|| EditError::Path(part.clone()))?;
    let mut work = Work::of(element, scope_above(tree.root(), &path));
    let mut numbered: Option<Numbered> = None;
    let mut written = Written {
        part: part.clone(),
        released: Vec::new(),
        dated: false,
        gave: false,
    };
    for plan in group {
        let wrote = sink.replace_span(&mut work, (plan.start, plan.end), &plan.text, keep)?;
        written.released.extend(wrote.released);
        written.dated |= wrote.dated;
        if let Some(n) = wrote.annotations {
            written.gave |= n.to > n.from;
            numbered = Some(numbered.map_or(n, |m| Numbered { to: n.to, ..m }));
        }
    }
    let new = work.build().ok_or_else(|| EditError::Path(part.clone()))?;
    let mut draws = numbered.map(|n| annotation_draws(&new, n)).unwrap_or_default();
    // docx-agent renews the text id of each range's paragraph, once per range
    if new.attribute(Some(W14), "paraId").is_some() {
        let prefix = w14_prefix(tree.root(), &path);
        for _ in group {
            draws.push(Draw {
                from: DrawFrom::Seed(Arc::from(format!("{part}\0text"))),
                at: Vec::new(),
                attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
            });
        }
    }
    let id = renamed_id(renamed, &head.found.id);
    let keys = vec![Key::Block {
        part: Arc::from(part.as_str()),
        id: Arc::from(id.as_str()),
    }];
    sink.replace_drawing(&part, &path, new, keys, draws)?;
    Ok(Some(written))
}

fn renamed_id(renamed: &[(String, String)], id: &str) -> String {
    renamed
        .iter()
        .find(|(old, _)| old == id)
        .map_or_else(|| id.to_owned(), |(_, new)| new.clone())
}

impl Command<WordDoc> for InsertText {
    type Out = RangesEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        check_text(&self.text)?;
        range_of(doc, &*doc.docx()?, &self.at, true).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<RangesEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let found = range_of(sink.doc(), &docx, &self.at, true)?;
        let plan = plan(sink.doc(), &docx, found, &self.text)?;
        replace_ranges(&mut sink, vec![plan], Keep::First)
    }
}

impl Command<WordDoc> for DeleteText {
    type Out = RangesEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        range_of(doc, &*doc.docx()?, &self.range, false).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<RangesEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let found = range_of(sink.doc(), &docx, &self.range, false)?;
        let plan = plan(sink.doc(), &docx, found, "")?;
        replace_ranges(&mut sink, vec![plan], Keep::First)
    }
}

impl Command<WordDoc> for ReplaceText {
    type Out = RangesEdited;
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        if self.find.is_empty() {
            return Err(EditError::Refused("nothing to find".to_owned()));
        }
        check_text(&self.with)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<RangesEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        // every paragraph is read: a find depends on the whole document
        sink.read_everything();
        let mut found = matches(&docx, &self.find, self.case);
        if let Some(count) = self.count {
            found.truncate(count);
        }
        let mut plans = Vec::new();
        for (id, start, end) in found {
            let paragraph = find(sink.doc(), &docx, &id)?;
            plans.push(plan(sink.doc(), &docx, (paragraph, start, end), &self.with)?);
        }
        replace_ranges(&mut sink, plans, self.keep)
    }
}
