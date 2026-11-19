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
use crate::find::Pattern;
use crate::names::{W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::set_text::{find, paragraph_has_annotations, Found};
use crate::sink::{annotation_draws, EditSink, Keep, Numbered};
use crate::stamping::{stamp_document, w14_prefix};
use crate::text::{atoms, write::check_text};
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

/// Every place a find matches replaced (docx-agent's `Document.replace`): within each
/// paragraph, or across paragraphs; the first `count` when given.
#[derive(Debug, Clone)]
pub struct ReplaceText {
    /// What is looked for, and what replaces each match.
    pub pattern: Arc<dyn Pattern>,
    /// Whether a match may run across paragraphs (in each story's text joined by `\n`).
    pub across: bool,
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

/// One range to replace: its paragraph (its first and last, across paragraphs), its
/// characters, the new text.
pub(crate) struct Plan {
    pub(crate) found: Found,
    /// The last paragraph of a range across paragraphs, with the places of every
    /// paragraph from the first to it.
    pub(crate) across: Option<Across>,
    /// The paragraph's place among its story's paragraphs (for the order of the plans).
    pub(crate) index: usize,
    /// The last paragraph's.
    pub(crate) last_index: usize,
    /// Its part's place among the stories.
    pub(crate) story: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) text: String,
}

/// A range across paragraphs: the last paragraph, and every paragraph's place from the
/// first to it.
pub(crate) struct Across {
    pub(crate) last: Found,
    pub(crate) paths: Vec<Box<[u32]>>,
}

/// A range as named: its first paragraph, its last when another, its offsets.
type Range = (Found, Option<Found>, usize, usize);

/// A range as docx-agent's `parse_range` reads one.
fn range_of(doc: &WordDoc, docx: &Docx, address: &str, position: bool) -> Result<Range> {
    if let Some(range) = parse_range(address) {
        let found = find(doc, docx, &range.start_id)?;
        if position {
            return Ok((found, None, range.start, range.start));
        }
        let last = find(doc, docx, &range.end_id)?;
        if last.part != found.part {
            return Err(EditError::Refused(format!("{address} runs across stories")));
        }
        let last = (last.path != found.path).then_some(last);
        return Ok((found, last, range.start, range.end));
    }
    // a paragraph's whole text (a cell's, a bookmark's, a link's: not yet)
    let found = find(doc, docx, address)?;
    let length = atoms(&Work::of(&found.element, Vec::new()), 0).len();
    Ok((found, None, 0, if position { 0 } else { length }))
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

fn plan(doc: &WordDoc, docx: &Docx, (found, last, start, end): Range, text: &str) -> Result<Plan> {
    let (story, index) = places(docx, &found);
    if let Some(last) = last {
        return plan_across(docx, (found, last, start, end), (story, index), text);
    }
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
        across: None,
        index,
        last_index: index,
        story,
        start,
        end,
        text: text.to_owned(),
    })
}

/// A range across paragraphs checked as docx-agent's `_entries` and `_check_across` check
/// it, before anything changes.
fn plan_across(
    docx: &Docx,
    (found, last, start, end): (Found, Found, usize, usize),
    (story, index): (usize, usize),
    text: &str,
) -> Result<Plan> {
    let span = || format!("{}@{start}..{}@{end}", found.id, last.id);
    let (_, last_index) = places(docx, &last);
    if last_index < index {
        return Err(EditError::Refused(format!("{} ends before it starts", span())));
    }
    let part = docx
        .part(&found.part)
        .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
    let paths: Vec<Box<[u32]>> = (index..=last_index)
        .filter_map(|k| part.ids.paragraphs().get(k).map(|p| p.path))
        .collect();
    let parent = |p: &[u32]| p.get(..p.len().saturating_sub(1)).map(<[u32]>::to_vec);
    if paths.iter().any(|p| parent(p) != parent(&found.path)) {
        return Err(EditError::Span(format!(
            "{}: the range runs across a table, a cell or a content control",
            span()
        )));
    }
    let lengths = |f: &Found| atoms(&Work::of(&f.element, Vec::new()), 0).len();
    if start > lengths(&found) || end > lengths(&last) {
        return Err(EditError::Span(format!("{} is outside its paragraphs' text", span())));
    }
    let (work, paragraphs) = stretch(part.root(), &paths).ok_or_else(|| EditError::Path(found.part.clone()))?;
    crate::across::check_across(&work, &paragraphs, start, end)
        .map_err(|e| EditError::Span(format!("{}: {e}", span())))?;
    Ok(Plan {
        found,
        across: Some(Across { last, paths }),
        index,
        last_index,
        story,
        start,
        end,
        text: text.to_owned(),
    })
}

/// A work whose root stands for the stretch of a parent's children from the first of
/// `paths` (siblings, in order) to the last, with the ids of those children in it.
pub(crate) fn stretch(root: &Arc<ooxml_xml::Element>, paths: &[Box<[u32]>]) -> Option<(Work, Vec<crate::work::Id>)> {
    let (first, last) = (paths.first()?, paths.last()?);
    let (&from, parent_path) = first.split_last()?;
    let &to = last.last()?;
    let parent = if parent_path.is_empty() {
        root
    } else {
        at_path(root, parent_path)?
    };
    let slice: Vec<ooxml_xml::Node> = parent
        .children()
        .iter()
        .skip(from as usize)
        .take((to - from) as usize + 1)
        .cloned()
        .collect();
    let holder = Arc::new(ooxml_xml::Element::from_head(
        Arc::clone(parent.head()),
        ooxml_xml::Children::from_vec(slice.clone()),
    ));
    let scope = if parent_path.is_empty() {
        Vec::new()
    } else {
        scope_above(root, parent_path)
    };
    let work = Work::of(&holder, scope);
    let ids: Vec<crate::work::Id> = work.children(work.root()).to_vec();
    let mut placed = Vec::new();
    let mut it = ids.iter();
    for (k, node) in slice.iter().enumerate() {
        if matches!(node, ooxml_xml::Node::Text(_)) {
            continue;
        }
        let id = *it.next()?;
        let place = from + u32::try_from(k).ok()?;
        if paths.iter().any(|p| p.last() == Some(&place)) {
            placed.push(id);
        }
    }
    Some((work, placed))
}

/// The plans checked and ordered last first, as docx-agent orders them; overlaps refused.
/// Untracked, a range across paragraphs joins its paragraphs, so another range in one of
/// them is refused; tracked, the paragraphs stay and each range is written in turn.
fn order_plans(plans: &mut [Plan], tracked: bool) -> Result<()> {
    for plan in plans.iter() {
        check_text(&plan.text)?;
    }
    plans.sort_by_key(|p| (p.story, p.index, p.start));
    plans.reverse();
    for pair in plans.windows(2) {
        if let [later, earlier] = pair {
            if earlier.story == later.story && (earlier.last_index, earlier.end) > (later.index, later.start) {
                return Err(EditError::Span("the ranges overlap".to_owned()));
            }
        }
    }
    let shares = |a: &Plan, b: &Plan| a.story == b.story && a.index <= b.last_index && b.index <= a.last_index;
    for (k, plan) in plans.iter().enumerate() {
        if !tracked && plan.across.is_some() && plans.iter().enumerate().any(|(j, other)| j != k && shares(plan, other))
        {
            return Err(EditError::Unsupported(
                "ranges sharing a paragraph with a range across paragraphs".to_owned(),
            ));
        }
    }
    Ok(())
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
    order_plans(&mut plans, matches!(sink.policy(), RevisionPolicy::Tracked(_)))?;
    let count = plans.len();
    let docx = sink.docx()?;
    for plan in &plans {
        sink.read(Key::Block {
            part: Arc::from(plan.found.part.as_str()),
            id: Arc::from(plan.found.id.as_str()),
        });
    }
    let renamed = stamp_document(sink, &docx)?;
    let first = plans.last().map(|p| {
        let named = p.across.as_ref().map_or(&p.found.id, |a| &a.last.id);
        renamed_id(&renamed, named)
    });
    let mut dated: Vec<String> = Vec::new();
    let mut released: Vec<(String, Vec<String>)> = Vec::new();
    let mut gave = false;
    let mut k = 0;
    while k < plans.len() {
        // a paragraph's ranges (they are together, last first) in one copy of it
        // (a range across paragraphs is a group of its own)
        let same = |p: &Plan| {
            plans.get(k).is_some_and(|q| {
                q.across.is_none() && p.across.is_none() && q.found.part == p.found.part && q.found.path == p.found.path
            })
        };
        let group: Vec<&Plan> = match plans.get(k) {
            Some(p) if p.across.is_some() => vec![p],
            _ => plans.iter().skip(k).take_while(|p| same(p)).collect(),
        };
        k += group.len();
        let written = match group.first() {
            Some(plan) if plan.across.is_some() => crate::ranges_across::write_across(sink, plan, keep, &renamed)?,
            _ => write_paragraph(sink, &group, keep, &renamed)?,
        };
        let Some(written) = written else {
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
    sink.space_written()?;
    crate::textbox::sync(sink)?;
    crate::revision_ids::make_unique(sink)?;
    Ok(RangesEdited {
        id: first,
        count,
        renamed,
    })
}

/// What writing one paragraph's ranges did.
pub(crate) struct Written {
    pub(crate) part: String,
    pub(crate) released: Vec<String>,
    pub(crate) dated: bool,
    pub(crate) gave: bool,
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
    sink.load_written(&part, Some(&path), &mut work);
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
    sink.keep_written(&part, Some(&path), &work);
    let new = work.build().ok_or_else(|| EditError::Path(part.clone()))?;
    let mut draws = numbered.map(|n| annotation_draws(&new, n, 0)).unwrap_or_default();
    // docx-agent renews the text id of each range's paragraph, once per range
    if new.attribute(Some(W14), "paraId").is_some() {
        let prefix = w14_prefix(tree.root(), &path);
        for _ in group {
            draws.push(Draw {
                from: DrawFrom::Seed(Arc::from(format!("{part}\0text"))),
                node: 0,
                at: Vec::new(),
                attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
            });
        }
    }
    let id = renamed_id(renamed, &head.found.id);
    let mut keys = vec![crate::set_text::block_key(&part, &id)];
    // the revision ids it gives, or takes away, are the annotation ids in use, which a
    // tracked edit numbers from (and an edit renumbering a repeated id reads)
    if paragraph_has_annotations(element) || paragraph_has_annotations(&new) {
        keys.push(Key::Annotations);
    }
    sink.replace_drawing(&part, &path, new, keys, draws)?;
    Ok(Some(written))
}

pub(crate) fn renamed_id(renamed: &[(String, String)], id: &str) -> String {
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
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<RangesEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        // every paragraph is read: a find depends on the whole document
        sink.read_everything();
        let mut found = crate::find::matches(&docx, self.pattern.as_ref(), self.across)?;
        if let Some(count) = self.count {
            found.truncate(count);
        }
        let mut plans = Vec::new();
        for (first, start, last, end, with) in found {
            let paragraph = find(sink.doc(), &docx, &first)?;
            let last = (last != first).then(|| find(sink.doc(), &docx, &last)).transpose()?;
            plans.push(plan(sink.doc(), &docx, (paragraph, last, start, end), &with)?);
        }
        replace_ranges(&mut sink, plans, self.keep)
    }
}

/// The range `address` names replaced by `text` (docx-agent's `TextRange.replace`, the
/// first replaced character's formatting kept), as one write of an edit through `sink`.
///
/// # Errors
/// The range's refusal.
pub(crate) fn replace_range(sink: &mut EditSink<'_, '_>, address: &str, text: &str) -> Result<RangesEdited> {
    let docx = sink.docx()?;
    let found = range_of(sink.doc(), &docx, address, false)?;
    let plan = plan(sink.doc(), &docx, found, text)?;
    replace_ranges(sink, vec![plan], Keep::First)
}
