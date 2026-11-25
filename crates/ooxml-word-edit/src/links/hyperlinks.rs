//! Owns: [`AddHyperlink`], a range in one paragraph made a hyperlink as docx-agent's
//! `Document.add_hyperlink` makes one, untracked: the range's runs split at its edges and
//! wrapped in a `w:hyperlink` -- to an external address (a relationship of its own, one
//! to the same address reused) or a bookmark (`w:anchor`) -- given the Hyperlink character
//! style (added as Word writes it when the document lacks it), the split pieces merged
//! back.
//!
//! Tracked (`_add_hyperlink_tracked`): the range's runs deleted, and a new hyperlink
//! after them holding their copies (styled, their own property changes dropped) in an
//! insertion; the link split out of any revision container it would sit in
//! (`place_outside_revisions`), the paragraph's textId drawn again. Word, tracking, writes
//! an inserted `HYPERLINK` field over the text instead, which its own Reject All loses the
//! text with; this form rejects to the original.
//!
//! Must not: write but through the sink.

use super::{new_w, set_run_style, LinkEdited};
use crate::merge::merge_runs;
use crate::names::{R, W};
use crate::policy::RevisionPolicy;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::text::atoms;
use crate::text_range::{range, range_id, Span};
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_opc::name::rels_for;
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::bookmark;
use ooxml_word_model::docx::{at_path, Docx};
use std::sync::Arc;

/// The relationship type of a hyperlink.
const HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

/// A range made a hyperlink.
#[derive(Debug, Clone)]
pub struct AddHyperlink {
    /// The range (in one paragraph), or what holds it: a paragraph, a cell, a bookmark.
    pub range: String,
    /// An external address; or
    pub url: Option<String>,
    /// a bookmark's name.
    pub anchor: Option<String>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// Whether `url` starts with a scheme (`[A-Za-z][A-Za-z0-9+.-]*:`).
fn has_scheme(url: &str) -> bool {
    let mut chars = url.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    for c in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
            return false;
        }
    }
    false
}

impl AddHyperlink {
    /// The range checked as docx-agent checks it before anything changes.
    fn check(&self, doc: &WordDoc, docx: &Docx) -> Result<Span> {
        if self.url.is_some() == self.anchor.is_some() {
            return Err(EditError::Refused(
                "give a url or an anchor (a bookmark name), not both".to_owned(),
            ));
        }
        if let Some(url) = self.url.as_deref().filter(|u| !has_scheme(u)) {
            return Err(EditError::Refused(format!(
                "{} is not an address (it needs a scheme: https:, mailto:...)",
                repr_str(url)
            )));
        }
        if let Some(anchor) = &self.anchor {
            bookmark(docx, anchor).map_err(EditError::Address)?;
        }
        let found = range(doc, docx, &self.range)?;
        let span = Span::of(doc, docx, &found)?;
        let id = range_id(&found);
        if !span.single() || span.start == span.end {
            return Err(EditError::Refused(
                "a hyperlink is made from a non-empty range in one paragraph, in the current view".to_owned(),
            ));
        }
        let paragraph = span.first().ok_or_else(|| EditError::no_element(span.part.clone()))?;
        let work = Work::of(&paragraph.element, Vec::new());
        let items = atoms(&work, work.root());
        crate::inline::check_span(&work, &items, span.start, span.end, false)
            .map_err(|e| EditError::Refused(format!("{id}: {e}")))?;
        let chosen = items.get(span.start..span.end).unwrap_or_default();
        if chosen.iter().any(|a| a.field.is_some()) {
            return Err(EditError::Refused(format!("{id} is in a field's result")));
        }
        let linked = |run: Id| {
            let mut at = work.parent(run);
            while let Some(node) = at {
                if work.is(node, W, "hyperlink") {
                    return true;
                }
                at = work.parent(node);
            }
            false
        };
        if chosen.iter().any(|a| a.run.is_some_and(linked)) {
            return Err(EditError::Refused(format!(
                "{id} is already (partly) a hyperlink; change or remove that one"
            )));
        }
        crate::builtin::resolve(docx, "Hyperlink", "character")?;
        Ok(span)
    }
}

/// Where a new hyperlink goes: a relationship's id, or a bookmark.
enum Target {
    Relationship(String),
    Bookmark(String),
}

impl AddHyperlink {
    /// The link's target: the external relationship to its address (added when the part
    /// has none to it), or its bookmark.
    fn target(&self, sink: &mut EditSink<'_, '_>, part: &str) -> Result<Target> {
        if let Some(url) = &self.url {
            let (package, rid) = crate::new_part::add_external(sink.doc().package(), part, HYPERLINK, url)?;
            if let Some(package) = package {
                let rels = rels_for(part);
                if let Some(tree) = package.tree(&rels)? {
                    sink.put(&rels, tree)?;
                }
            }
            return Ok(Target::Relationship(rid));
        }
        Ok(Target::Bookmark(self.anchor.clone().unwrap_or_default()))
    }
}

/// `runs` (siblings under `parent`, in order) wrapped in a new `w:hyperlink` to `target`.
fn wrap(work: &mut Work, runs: &[Id], parent: Id, target: Target) -> Id {
    let link = new_link(work, parent, target);
    let (Some(&first), Some(&last)) = (runs.first(), runs.last()) else {
        return link;
    };
    let at = work.index(first).unwrap_or(0);
    work.insert(parent, at, link);
    let mut node = Some(first);
    while let Some(run) = node {
        let following = work.next(run);
        work.append(link, run);
        if run == last {
            break;
        }
        node = following;
    }
    link
}

/// The tracked form of the link (docx-agent's `_add_hyperlink_tracked`): `runs` (siblings
/// under `parent`) deleted, and after them a new link to `target` holding their copies --
/// given the character style `style`, their own property changes dropped -- in an
/// insertion. The link, the relationship ids the deletion released, the revision ids
/// drawn and whether a record is dated.
fn tracked_link(
    work: &mut Work,
    (runs, parent): (&[Id], Id),
    target: Target,
    (style, orders): (&str, &crate::validate::Orders),
    sink: &mut EditSink<'_, '_>,
    tracking: crate::policy::Tracking,
) -> Result<(Id, Vec<String>, crate::sink::Numbered, bool)> {
    let last = *runs
        .last()
        .ok_or_else(|| EditError::Refused("an empty range".to_owned()))?;
    let copies: Vec<Id> = runs.iter().map(|&run| work.deep_copy(run)).collect();
    for &copy in &copies {
        for change in work.iter(copy, W, &["rPrChange"]) {
            work.remove(change);
        }
        set_run_style(work, copy, style, orders);
    }
    let ((link, released), numbered, dated) = sink.stamped(tracking, |stamp| {
        // where the link goes: after the last run, whatever the deletion makes of the runs
        let place = new_w(work, parent, "proofErr", &[]);
        work.add_next(last, place);
        let released = crate::track::delete_runs(work, runs, stamp);
        let link = new_link(work, parent, target);
        work.add_previous(place, link);
        work.remove(place);
        crate::track::place_outside_revisions(work, link, stamp);
        let container = stamp.make(work, "ins");
        work.append(link, container);
        for copy in copies {
            work.append(container, copy);
        }
        Ok((link, released))
    })?;
    Ok((link, released, numbered, dated))
}

/// A new, detached `w:hyperlink` to `target` (`_new_link`), named as `parent` names its
/// children.
fn new_link(work: &mut Work, parent: Id, target: Target) -> Id {
    let link = new_w(work, parent, "hyperlink", &[]);
    match target {
        Target::Relationship(rid) => {
            // lxml, where nothing declares the relationships namespace, declares it on the
            // element under a made-up prefix
            let name = if work.has_prefix_for(parent, R) {
                work.qname(R, "id", "r")
            } else {
                work.declare(link, "ns1", R);
                Arc::new(ooxml_xml::QName::new(Some("ns1"), "id", Some(R)))
            };
            work.set(link, name, &rid);
        }
        Target::Bookmark(anchor) => {
            let name = work.qname(W, "anchor", "w");
            work.set(link, name, &anchor);
        }
    }
    let history = work.qname(W, "history", "w");
    work.set(link, history, "1");
    link
}

impl Command<WordDoc> for AddHyperlink {
    type Out = LinkEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(doc, &*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<LinkEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl AddHyperlink {
    /// The edit, through `sink`.
    pub(crate) fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<LinkEdited> {
        let docx = sink.docx()?;
        if self.anchor.is_some() {
            sink.read_everything(); // the bookmark it goes to, wherever it is
        }
        let span = self.check(sink.doc(), &docx)?;
        let paragraph = span.first().ok_or_else(|| EditError::no_element(span.part.clone()))?;
        let part = span.part.clone();
        sink.read(block_key(&part, &paragraph.id));
        let id = range_id(&crate::text_range::range(sink.doc(), &docx, &self.range)?);
        let renamed = stamp_document(&mut *sink, &docx)?;
        let new_id = crate::ranges::renamed_id(&renamed, &paragraph.id);
        let tree = sink
            .doc()
            .tree(&part)?
            .ok_or_else(|| EditError::no_part(part.clone()))?;
        let element = at_path(tree.root(), &paragraph.path).ok_or_else(|| EditError::no_element(part.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &paragraph.path));
        let root = work.root();
        let runs = crate::inline::isolate(&mut work, root, span.start, span.end)?;
        let Some(&first) = runs.first() else {
            return Err(EditError::no_element(part.clone()));
        };
        let parent = work.parent(first).ok_or_else(|| EditError::no_element(part.clone()))?;
        if runs.iter().any(|r| work.parent(*r) != Some(parent)) {
            return Err(EditError::Refused(format!(
                "{id} spans runs in different containers (an insertion, a content control); a hyperlink cannot wrap them"
            )));
        }
        let target = self.target(&mut *sink, &part)?;
        if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
            let place = crate::fields::Place {
                part,
                path: paragraph.path.to_vec(),
                id: paragraph.id.clone(),
                offset: 0,
            };
            return put_tracked(sink, work, (&runs, parent), target, (&place, tracking), renamed);
        }
        let link = wrap(&mut work, &runs, parent, target);
        let style = crate::builtin::ensure(&mut *sink, "Hyperlink", "character")?;
        let shared = sink.doc().shared_handle();
        for &run in &runs {
            set_run_style(&mut work, run, &style, shared.facts.orders());
        }
        merge_runs(&mut work, &runs);
        let index = work
            .iter(root, W, &["hyperlink"])
            .iter()
            .position(|l| *l == link)
            .unwrap_or(0);
        sink.keep_written(&part, None, &work);
        let new = work.build().ok_or_else(|| EditError::unbuilt(part.clone()))?;
        sink.replace(&part, &paragraph.path, new, vec![block_key(&part, &new_id)])?;
        Ok(made(&new_id, index, renamed))
    }
}

/// The reply of a link made: the `index`th of the paragraph `id`.
fn made(id: &str, index: usize, renamed: Vec<(String, String)>) -> LinkEdited {
    let made = format!("hl:{id}/{index}");
    LinkEdited {
        id: Some(made.clone()),
        changed: true,
        created: vec![made],
        renamed,
        ..LinkEdited::default()
    }
}

/// The tracked link made in the paragraph `work` holds ([`tracked_link`]) and the
/// paragraph put back at `place`, its textId drawn again, the relationships the deletion
/// released let go.
fn put_tracked(
    sink: &mut EditSink<'_, '_>,
    mut work: Work,
    (runs, parent): (&[Id], Id),
    target: Target,
    (place, tracking): (&crate::fields::Place, crate::policy::Tracking),
    renamed: Vec<(String, String)>,
) -> Result<LinkEdited> {
    let style = crate::builtin::ensure(&mut *sink, "Hyperlink", "character")?;
    let shared = sink.doc().shared_handle();
    let styling = (style.as_str(), shared.facts.orders());
    let (link, released, numbered, dated) = tracked_link(&mut work, (runs, parent), target, styling, sink, tracking)?;
    let root = work.root();
    let index = work
        .iter(root, W, &["hyperlink"])
        .iter()
        .position(|l| *l == link)
        .unwrap_or(0);
    sink.keep_written(&place.part, None, &work);
    crate::fields::put_back(&mut *sink, &work, (place, true), Some((numbered, dated)), &renamed)?;
    sink.release(&place.part, &released)?;
    let new_id = crate::ranges::renamed_id(&renamed, &place.id);
    Ok(made(&new_id, index, renamed))
}
