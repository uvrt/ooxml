//! Owns: [`InsertToc`], a table of contents put beside a block as Word's Table of Contents
//! writes one (docx-agent's `insert_toc`): `TOC \o "a-b" \h \z \u`, its begin, instruction
//! and separator opening the first entry and its end in the paragraph after the entries --
//! a new empty paragraph, or, before a paragraph, that paragraph -- its entries computed
//! from the layout until their pages hold; tracked (`_track_toc`), one insertion: the
//! entries' content (inside their hyperlinks), their marks (Word's typing form at a
//! container's end), and the holder's mark when it is new, else the end's run.
//!
//! Must not: write but through the sink.

use super::pages::FieldLayout;
use super::scan::{all_fields, paragraph_of, Located};
use super::toc::build::head;
use super::update::{empty_tocs, settle};
use super::values::Day;
use super::write::{green_path, run_of};
use super::FieldEdited;
use crate::delete::editable;
use crate::doc::{Draw, DrawFrom, Key};
use crate::links::new_w;
use crate::marks::{mark, mark_record};
use crate::names::{W, W14, W16DU};
use crate::paragraphs::insert_paragraph;
use crate::policy::{RevisionPolicy, Stamp};
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::{stamp_document, w14_prefix};
use crate::validate::Orders;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::at_path;
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// A table of contents inserted.
#[derive(Clone)]
pub struct InsertToc<'l> {
    /// The block it goes after; or
    pub after: Option<String>,
    /// the block it goes before.
    pub before: Option<String>,
    /// The heading levels listed, 1-9.
    pub levels: (i64, i64),
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
    /// The day a `DATE` shows (an update's).
    pub today: Day,
    /// The layout of the document's versions.
    pub layout: Option<&'l dyn FieldLayout>,
}

impl std::fmt::Debug for InsertToc<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsertToc")
            .field("after", &self.after)
            .field("before", &self.before)
            .field("levels", &self.levels)
            .finish_non_exhaustive()
    }
}

/// Where the field's runs go: a paragraph of the body.
struct Holder {
    path: Vec<u32>,
    id: String,
    own: bool,
}

impl InsertToc<'_> {
    fn check(&self, doc: &WordDoc) -> Result<crate::paragraphs::Anchor> {
        let anchor = match (&self.after, &self.before) {
            (Some(a), None) | (None, Some(a)) => a,
            _ => return Err(EditError::Refused("give exactly one of after= or before=".to_owned())),
        };
        let (first, last) = self.levels;
        if !(1 <= first && first <= last && last <= 9) {
            return Err(EditError::Refused("levels are (first, last), 1 to 9".to_owned()));
        }
        let docx = doc.docx()?;
        let found = crate::paragraphs::anchor(doc, &docx, anchor)?;
        if found.part != docx.main {
            return Err(EditError::Refused("a table of contents goes in the body".to_owned()));
        }
        Ok(found)
    }

    fn instruction(&self) -> String {
        format!(" TOC \\o \"{}-{}\" \\h \\z \\u ", self.levels.0, self.levels.1)
    }

    /// The paragraph that holds the field: a new empty one (its properties gone), or the
    /// paragraph it goes before.
    fn holder(
        &self,
        sink: &mut EditSink<'_, '_>,
        anchor: &crate::paragraphs::Anchor,
    ) -> Result<(Holder, Vec<(String, String)>)> {
        let docx = sink.docx()?;
        let own = !(self.before.is_some() && anchor.paragraph);
        if !own {
            let renamed = stamp_document(&mut *sink, &docx)?;
            let id = crate::ranges::renamed_id(&renamed, &anchor.id);
            return Ok((
                Holder {
                    path: anchor.path.to_vec(),
                    id,
                    own,
                },
                renamed,
            ));
        }
        let target = self.after.clone().or_else(|| self.before.clone()).unwrap_or_default();
        let after = self.after.is_some();
        let made = sink.with_policy(RevisionPolicy::Untracked, |sink| {
            insert_paragraph(sink, &target, (after, "", None))
        })?;
        let docx = sink.docx()?;
        let part = docx
            .part(&docx.main)
            .ok_or_else(|| EditError::NoPart(docx.main.clone()))?;
        let path = part
            .ids
            .paragraphs()
            .iter()
            .find(|p| p.id == made.id)
            .map(|p| p.path.to_vec())
            .ok_or_else(|| EditError::Path(docx.main.clone()))?;
        Ok((Holder { path, id: made.id, own }, made.renamed))
    }

    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<FieldEdited> {
        let anchor = self.check(sink.doc())?;
        sink.read_everything(); // the headings it lists, wherever they are
        let (holder, renamed) = self.holder(sink, &anchor)?;
        let main = sink.docx()?.main.clone();
        let instruction = self.instruction();
        let begin = put_field(sink, &main, &holder, &instruction)?;
        let docx = sink.docx()?;
        let toc = all_fields(&docx)
            .into_iter()
            .find(|l| l.part == main && l.found.begin == begin)
            .ok_or_else(|| EditError::Path(main.clone()))?;
        let known = (main.clone(), position(&docx, &toc));
        let unknown = settle(&mut *sink, std::slice::from_ref(&known), self.layout, self.today)?;
        let docx = sink.docx()?;
        let toc = find_toc(&docx, &main, &holder)?;
        if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
            track_toc(sink, &toc, tracking, holder.own)?;
        }
        let docx = sink.docx()?;
        let toc = find_toc(&docx, &main, &holder)?;
        if !holder.own {
            renew_text_id(sink, &toc)?;
        }
        let docx = sink.docx()?;
        let toc = find_toc(&docx, &main, &holder)?;
        let id = toc.id();
        let part = docx.part(&main).ok_or_else(|| EditError::NoPart(main.clone()))?;
        let mut places = toc_paragraphs(part.root(), &toc);
        if !holder.own {
            places.pop();
        }
        let mut created = vec![id.clone()];
        for path in places {
            if let Some(p) = part.ids.paragraph_at(&path) {
                created.push(p.id);
            }
        }
        Ok(FieldEdited {
            id: Some(id.clone()),
            changed: true,
            created,
            renamed,
            unknown,
            warnings: empty_tocs(&docx, Some(std::slice::from_ref(&id))),
            ..FieldEdited::default()
        })
    }
}

/// The field's place among its part's fields (what the passes know it as).
fn position(docx: &ooxml_word_model::docx::Docx, toc: &Located) -> usize {
    all_fields(docx)
        .iter()
        .filter(|l| l.part == toc.part)
        .take_while(|l| l.found.begin != toc.found.begin)
        .count()
}

/// The table of contents whose end is in the holder paragraph (its end is kept by every
/// rebuild; for its own new paragraph, the last whose end is there).
fn find_toc(docx: &ooxml_word_model::docx::Docx, main: &str, holder: &Holder) -> Result<Located> {
    let part = docx.part(main).ok_or_else(|| EditError::NoPart(main.to_owned()))?;
    let id = &holder.id;
    let path = part
        .ids
        .paragraphs()
        .iter()
        .find(|p| p.id == *id)
        .map(|p| p.path.to_vec())
        .ok_or_else(|| EditError::Path(main.to_owned()))?;
    all_fields(docx)
        .into_iter()
        .filter(|l| l.part == main && l.found.keyword() == "TOC")
        .filter(|l| l.found.end.as_ref().and_then(|e| paragraph_of(part.root(), e)).as_ref() == Some(&path))
        .last()
        .ok_or_else(|| EditError::Path(main.to_owned()))
}

/// The field's begin, instruction, separator and end put first in the holder (after its
/// properties), each a run of its own: the begin's path.
fn put_field(sink: &mut EditSink<'_, '_>, part: &str, holder: &Holder, instruction: &str) -> Result<Vec<u32>> {
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    let element = at_path(tree.root(), &holder.path).ok_or_else(|| EditError::Path(part.to_owned()))?;
    let mut work = Work::of(element, scope_above(tree.root(), &holder.path));
    let paragraph = work.root();
    if holder.own {
        if let Some(properties) = work.find(paragraph, W, "pPr") {
            work.remove(properties);
        }
    }
    let at = usize::from(work.find(paragraph, W, "pPr").is_some());
    let mut runs = head(&mut work, paragraph, instruction);
    let end_run = new_w(&mut work, paragraph, "r", &[]);
    let end = new_w(&mut work, end_run, "fldChar", &[("fldCharType", "end")]);
    work.append(end_run, end);
    runs.push(end_run);
    for (k, run) in runs.iter().enumerate() {
        work.insert(paragraph, at + k, *run);
    }
    let begin = runs
        .first()
        .and_then(|r| work.find(*r, W, "fldChar"))
        .and_then(|b| green_path(&work, b))
        .ok_or_else(|| EditError::Path(part.to_owned()))?;
    let new = work.build().ok_or_else(|| EditError::Path(part.to_owned()))?;
    sink.replace(part, &holder.path, new, vec![Key::Part(Arc::from(part))])?;
    let mut path = holder.path.clone();
    path.extend(begin);
    Ok(path)
}

/// The paragraphs from the field's begin to its end, both included (`_toc_paragraphs`).
fn toc_paragraphs(root: &Arc<Element>, toc: &Located) -> Vec<Vec<u32>> {
    let Some(first) = paragraph_of(root, &toc.found.begin) else {
        return Vec::new();
    };
    let last = toc.found.end.as_ref().and_then(|e| paragraph_of(root, e));
    let mut out = vec![first.clone()];
    let Some((&b, container)) = first.split_last() else {
        return out;
    };
    let Some(parent) = at_path(root, container) else {
        return out;
    };
    let Some(last) = last else { return out };
    for k in b + 1..u32::try_from(parent.children().len()).unwrap_or(0) {
        let mut path = container.to_vec();
        path.push(k);
        let is_p = parent
            .children()
            .get(k as usize)
            .and_then(Node::as_element)
            .is_some_and(|e| e.name().is(Some(W), "p"));
        if is_p {
            out.push(path.clone());
        }
        if path == last {
            break;
        }
    }
    out
}

/// Whether a block follows the child `k` of `parent` (`is_last_in_container`'s negation).
fn followed(parent: &Element, k: u32) -> bool {
    parent.children().iter().skip(k as usize + 1).any(|n| {
        n.as_element().is_some_and(|e| {
            e.name().namespace() == Some(W) && ["p", "tbl", "sdt", "customXml"].contains(&e.name().local())
        })
    })
}

/// The new table of contents recorded as one insertion (`_track_toc`).
fn track_toc(sink: &mut EditSink<'_, '_>, toc: &Located, tracking: crate::policy::Tracking, own: bool) -> Result<()> {
    let docx = sink.docx()?;
    let part = toc.part.clone();
    let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
    let root = Arc::clone(story.root());
    let mut places = toc_paragraphs(&root, toc);
    let holder = places.last().cloned().ok_or_else(|| EditError::Path(part.clone()))?;
    if !own {
        places.pop();
    }
    let (Some(first), Some(last)) = (places.first().cloned(), holder.last().copied()) else {
        return Ok(());
    };
    let (&b, container) = first.split_last().ok_or_else(|| EditError::Path(part.clone()))?;
    let parent = at_path(&root, container).ok_or_else(|| EditError::Path(part.clone()))?;
    let previous = (0..b).rev().find(|k| {
        parent
            .children()
            .get(*k as usize)
            .and_then(Node::as_element)
            .is_some_and(|e| e.name().is(Some(W), "p") || e.name().is(Some(W), "tbl"))
    });
    let from = previous.unwrap_or(b);
    let span = |k: u32| {
        let mut p = container.to_vec();
        p.push(k);
        p
    };
    let ends = vec![span(from), span(last)];
    let end_path = toc.found.end.clone().ok_or_else(|| EditError::Path(part.clone()))?;
    let wanted: Vec<Vec<u32>> = places
        .iter()
        .cloned()
        .chain(previous.map(span))
        .chain([end_path])
        .collect();
    let wanted_refs: Vec<&[u32]> = wanted.iter().map(Vec::as_slice).collect();
    let (mut work, parent_path, stretch, found) =
        editable(&root, &ends, &wanted_refs).ok_or_else(|| EditError::Path(part.clone()))?;
    let entries: Vec<Id> = found.iter().take(places.len()).copied().collect();
    let previous_node = previous.and(found.get(places.len()).copied());
    let end_node = found.last().copied();
    let last_flags: Vec<bool> = places
        .iter()
        .map(|p| p.last().is_some_and(|k| !followed(parent, *k)))
        .collect();
    let shared = sink.doc().shared_handle();
    let orders = shared.facts.orders();
    let ((), numbered, dated) = sink.stamped(tracking, |stamp| {
        for paragraph in &entries {
            insert_content(&mut work, *paragraph, stamp);
        }
        if !own {
            if let Some(end) = end_node {
                let end_run = run_of(&work, end);
                let container = stamp.make(&mut work, "ins");
                work.add_previous(end_run, container);
                work.append(container, end_run);
            }
        }
        mark_entries(&mut work, (&entries, &last_flags, previous_node), stamp, orders);
        Ok(())
    })?;
    let nodes = work
        .build_content(work.root())
        .ok_or_else(|| EditError::Path(part.clone()))?;
    let mut draws: Vec<Draw> = Vec::new();
    for (k, node) in nodes.iter().enumerate() {
        if let (Node::Element(e), Ok(k)) = (node, u32::try_from(k)) {
            draws.extend(annotation_draws(e, numbered, k));
        }
    }
    let count = (stretch.1 - stretch.0) as usize + 1;
    let keys = vec![Key::Part(Arc::from(part.as_str())), Key::Annotations];
    sink.splice_children(&part, (&parent_path, stretch.0 as usize, count), nodes, keys, draws)?;
    if dated {
        sink.ensure_w14(&part, false, &[("w16du", W16DU)])?;
    }
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        sink.people(
            (numbered.to > numbered.from).then_some(tracking.author.as_str()),
            Some(&tracking.author),
        )?;
    }
    Ok(())
}

/// Each entry's mark recorded as inserted; at its container's end, Word's typing form:
/// the block before's mark instead, where that is a paragraph with no mark record and no
/// section break.
fn mark_entries(
    work: &mut Work,
    (entries, last, previous): (&[Id], &[bool], Option<Id>),
    stamp: &mut Stamp,
    orders: &Orders,
) {
    for (k, paragraph) in entries.iter().enumerate() {
        if last.get(k).copied().unwrap_or(false) {
            let eligible = previous.filter(|p| {
                work.is(*p, W, "p")
                    && mark_record(work, *p).is_none()
                    && work
                        .find(*p, W, "pPr")
                        .and_then(|pr| work.find(pr, W, "sectPr"))
                        .is_none()
            });
            if let Some(previous) = eligible {
                mark(work, previous, "ins", stamp, orders);
                continue;
            }
        }
        mark(work, *paragraph, "ins", stamp, orders);
    }
}

/// A new paragraph's runs recorded as inserted (`_insert_content`): those in a hyperlink
/// inside it, the others in groups between bookmarks.
fn insert_content(work: &mut Work, paragraph: Id, stamp: &mut Stamp) {
    let mut groups: Vec<Vec<Id>> = Vec::new();
    for child in work.children(paragraph).to_vec() {
        if !work.is_element(child) || work.is(child, W, "pPr") {
            continue;
        }
        if work.is(child, W, "hyperlink") {
            let inner: Vec<Id> = work
                .children(child)
                .iter()
                .copied()
                .filter(|c| work.is_element(*c))
                .collect();
            if let Some(&first) = inner.first() {
                let container = stamp.make(work, "ins");
                work.add_previous(first, container);
                for node in inner {
                    work.append(container, node);
                }
            }
            groups.push(Vec::new());
            continue;
        }
        if work.is_any(child, W, &["bookmarkStart", "bookmarkEnd"]) {
            groups.push(Vec::new());
            continue;
        }
        if groups.last().is_none_or(Vec::is_empty) {
            groups.push(Vec::new());
        }
        if let Some(group) = groups.last_mut() {
            group.push(child);
        }
    }
    for group in groups {
        let Some(&first) = group.first() else { continue };
        let container = stamp.make(work, "ins");
        work.add_previous(first, container);
        for node in group {
            work.append(container, node);
        }
    }
}

/// The holder paragraph's text id renewed (it was there before).
fn renew_text_id(sink: &mut EditSink<'_, '_>, toc: &Located) -> Result<()> {
    let docx = sink.docx()?;
    let part = &toc.part;
    let story = docx.part(part).ok_or_else(|| EditError::NoPart(part.clone()))?;
    let root = story.root();
    let Some(path) = toc.found.end.as_ref().and_then(|e| paragraph_of(root, e)) else {
        return Ok(());
    };
    let element = at_path(root, &path).ok_or_else(|| EditError::Path(part.clone()))?;
    if element.attribute(Some(W14), "paraId").is_none() {
        return Ok(());
    }
    let prefix = w14_prefix(root, &path);
    let draw = Draw {
        from: DrawFrom::Seed(Arc::from(format!("{part}\0text"))),
        node: 0,
        at: Vec::new(),
        attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
    };
    let id = story.ids.paragraph_at(&path).map(|p| p.id).unwrap_or_default();
    sink.replace_drawing(
        part,
        &path,
        Arc::clone(element),
        vec![crate::set_text::block_key(part, &id)],
        vec![draw],
    )
}

impl Command<WordDoc> for InsertToc<'_> {
    type Out = FieldEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(doc).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<FieldEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}
