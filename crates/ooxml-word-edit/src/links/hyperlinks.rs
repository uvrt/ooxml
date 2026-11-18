//! Owns: [`AddHyperlink`], a range in one paragraph made a hyperlink as docx-agent's
//! `Document.add_hyperlink` makes one, untracked: the range's runs split at its edges and
//! wrapped in a `w:hyperlink` -- to an external address (a relationship of its own, one
//! to the same address reused) or a bookmark (`w:anchor`) -- given the Hyperlink character
//! style (added as Word writes it when the document lacks it), the split pieces merged
//! back.
//!
//! Tracked, docx-agent deletes the range's runs and inserts their copies in a new
//! hyperlink, splitting revision containers around it (`place_outside_revisions`): not
//! ported yet ([`EditError::Unsupported`]).
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
        let paragraph = span.first().ok_or_else(|| EditError::Path(span.part.clone()))?;
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

impl Command<WordDoc> for AddHyperlink {
    type Out = LinkEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(doc, &*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<LinkEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        if self.anchor.is_some() {
            sink.read_everything(); // the bookmark it goes to, wherever it is
        }
        let span = self.check(sink.doc(), &docx)?;
        if matches!(self.policy, RevisionPolicy::Tracked(_)) {
            return Err(EditError::Unsupported(
                "a hyperlink made while tracking changes".to_owned(),
            ));
        }
        let paragraph = span.first().ok_or_else(|| EditError::Path(span.part.clone()))?;
        let part = span.part.clone();
        sink.read(block_key(&part, &paragraph.id));
        let id = range_id(&crate::text_range::range(sink.doc(), &docx, &self.range)?);
        let renamed = stamp_document(&mut sink, &docx)?;
        let new_id = crate::ranges::renamed_id(&renamed, &paragraph.id);
        let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
        let element = at_path(tree.root(), &paragraph.path).ok_or_else(|| EditError::Path(part.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &paragraph.path));
        let root = work.root();
        let runs = crate::inline::isolate(&mut work, root, span.start, span.end)?;
        let Some(&first) = runs.first() else {
            return Err(EditError::Path(part.clone()));
        };
        let parent = work.parent(first).ok_or_else(|| EditError::Path(part.clone()))?;
        if runs.iter().any(|r| work.parent(*r) != Some(parent)) {
            return Err(EditError::Refused(format!(
                "{id} spans runs in different containers (an insertion, a content control); a hyperlink cannot wrap them"
            )));
        }
        let target = self.target(&mut sink, &part)?;
        let link = wrap(&mut work, &runs, parent, target);
        let style = crate::builtin::ensure(&mut sink, "Hyperlink", "character")?;
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
        let new = work.build().ok_or_else(|| EditError::Path(part.clone()))?;
        sink.replace(&part, &paragraph.path, new, vec![block_key(&part, &new_id)])?;
        let made = format!("hl:{new_id}/{index}");
        Ok(LinkEdited {
            id: Some(made.clone()),
            changed: true,
            created: vec![made],
            renamed,
            ..LinkEdited::default()
        })
    }
}
