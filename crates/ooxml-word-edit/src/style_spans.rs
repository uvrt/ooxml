//! Owns: [`StyleSpans`], character styles and emphasis written over a paragraph's text as
//! docx-agent's `Document.style_spans` writes them (what `word_set_text` writes for text
//! with inline emphasis): each span's runs isolated and given the span's character style
//! (a built-in one the document lacks added as Word writes it, the default style as none)
//! and direct `w:b`/`w:i` where asked, removed where not; other run formatting kept; a run
//! in a hyperlink keeping its style where the span has none; a span across a field's edge
//! cut there; tracked, the old formatting recorded in `w:rPrChange`s
//! ([`EditSink::properties_changed`]).
//!
//! Must not: write but through the sink, or read outside what it records reading: the
//! paragraph's own block, the styles part when it adds a style, the ledger seeds the
//! stamping draws from, the annotation ids when tracked.

use crate::doc::Key;
use crate::inline::{isolate, merge_around, prune};
use crate::marks::insert_in_order;
use crate::names::{W, W16DU};
use crate::policy::RevisionPolicy;
use crate::set_text::{block_key, find, paragraph_has_annotations};
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::stamp_document;
use crate::text::{atoms, Atom};
use crate::validate::Orders;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::at_path;

/// One span's look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Its first character.
    pub start: usize,
    /// One past its last.
    pub end: usize,
    /// The character style's name (`None`: none, the runs' own taken away).
    pub style: Option<String>,
    /// Direct bold (taken away where false).
    pub bold: bool,
    /// Direct italic (taken away where false).
    pub italic: bool,
}

/// Write character styles and emphasis over a paragraph's text.
#[derive(Debug, Clone)]
pub struct StyleSpans {
    /// The paragraph.
    pub target: String,
    /// The spans, in order.
    pub spans: Vec<Span>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What a style_spans did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpansStyled {
    /// The paragraph's id after the edit.
    pub id: String,
    /// Ids the edit's stamping renamed, old to new, in the order made.
    pub renamed: Vec<(String, String)>,
    /// The paragraph's part.
    pub part: String,
}

impl Command<WordDoc> for StyleSpans {
    type Out = SpansStyled;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        check(doc, &self.target, &self.spans)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<SpansStyled> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = style_spans(&mut sink, &self.target, &self.spans)?;
        // docx-agent's edit, once it ends: text boxes synced, revision ids unique
        sink.space_written()?;
        crate::textbox::sync(&mut sink)?;
        crate::revision_ids::make_unique(&mut sink)?;
        Ok(out)
    }
}

/// Refuse a paragraph that is not one, a style no document or Word has, or a span outside
/// the paragraph's characters.
fn check(doc: &WordDoc, target: &str, spans: &[Span]) -> Result<()> {
    let docx = doc.docx()?;
    let found = find(doc, &docx, target)?;
    for name in spans.iter().filter_map(|s| s.style.as_deref()) {
        crate::builtin::resolve(&docx, name, "character")?;
    }
    let work = Work::of(&found.element, Vec::new());
    let length = atoms(&work, work.root()).len();
    for span in spans {
        if !(span.start <= span.end && span.end <= length) {
            return Err(EditError::Refused(format!(
                "{}@{}:{} is outside its {length} characters",
                found.id, span.start, span.end
            )));
        }
    }
    Ok(())
}

/// docx-agent's `style_spans` inside an edit, without what the edit does once it ends
/// (text boxes synced, revision ids made unique), which the command's caller does.
///
/// # Errors
/// A refusal as [`StyleSpans`]'s validation gives.
pub(crate) fn style_spans(sink: &mut EditSink<'_, '_>, target: &str, spans: &[Span]) -> Result<SpansStyled> {
    check(sink.doc(), target, spans)?;
    let docx = sink.docx()?;
    let found = find(sink.doc(), &docx, target)?;
    sink.read(block_key(&found.part, &found.id));
    let renamed = stamp_document(sink, &docx)?;
    let id = crate::ranges::renamed_id(&renamed, &found.id);
    let mut ids: Vec<(String, String)> = Vec::new();
    for name in spans.iter().filter_map(|s| s.style.as_deref()) {
        if !ids.iter().any(|(n, _)| n == name) {
            let made = crate::builtin::ensure(sink, name, "character")?;
            ids.push((name.to_owned(), made));
        }
    }
    let default = sink
        .docx()?
        .default_style("character")
        .and_then(|s| s.attribute(Some(W), "styleId"))
        .map(str::to_owned);
    let tree = sink
        .doc()
        .tree(&found.part)?
        .ok_or_else(|| EditError::NoPart(found.part.clone()))?;
    let element = at_path(tree.root(), &found.path).ok_or_else(|| EditError::Path(found.part.clone()))?;
    let mut work = Work::of(element, scope_above(tree.root(), &found.path));
    sink.load_written(&found.part, &mut work);
    let paragraph = work.root();
    let tracker = sink.watch_properties(&mut work, &[paragraph]);
    let shared = sink.doc().shared_handle();
    let orders = shared.facts.orders();
    for span in spans {
        let style = span
            .style
            .as_ref()
            .and_then(|name| ids.iter().find(|(n, _)| n == name).map(|(_, id)| id.clone()))
            .filter(|id| Some(id) != default.as_ref());
        let look = Look {
            style: style.as_deref(),
            bold: span.bold,
            italic: span.italic,
        };
        for (a, b) in field_pieces(&atoms(&work, paragraph), span.start, span.end) {
            let runs = isolate(&mut work, paragraph, a, b)?;
            for &run in &runs {
                if look.style.is_none() && !look.bold && !look.italic && in_link(&work, run) {
                    continue;
                }
                write_look(&mut work, run, &look, orders);
            }
            merge_around(&mut work, &runs);
        }
    }
    prune(&mut work, paragraph);
    let wrote = sink.properties_changed(&mut work, tracker)?;
    sink.keep_written(&found.part, &work);
    let new = work.build().ok_or_else(|| EditError::Path(found.part.clone()))?;
    if *new != **element {
        let draws = wrote
            .annotations
            .map(|n| annotation_draws(&new, n, 0))
            .unwrap_or_default();
        let mut keys = vec![block_key(&found.part, &id)];
        if paragraph_has_annotations(&found.element) || paragraph_has_annotations(&new) {
            keys.push(Key::Annotations);
        }
        sink.replace_drawing(&found.part, &found.path, new, keys, draws)?;
        if wrote.dated {
            sink.ensure_w14(&found.part, false, &[("w16du", W16DU)])?;
        }
    }
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
        sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    }
    Ok(SpansStyled {
        id,
        renamed,
        part: found.part,
    })
}

/// What a span gives its runs.
struct Look<'a> {
    style: Option<&'a str>,
    bold: bool,
    italic: bool,
}

/// `[start, end)` cut where the field a character is in changes.
fn field_pieces(items: &[Atom], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut k = start;
    while k < end {
        let owner = items.get(k).and_then(|a| a.field);
        let mut j = k + 1;
        while j < end && items.get(j).and_then(|a| a.field) == owner {
            j += 1;
        }
        out.push((k, j));
        k = j;
    }
    out
}

/// Whether a run is inside a hyperlink of its paragraph.
fn in_link(work: &Work, run: Id) -> bool {
    let mut parent = work.parent(run);
    while let Some(p) = parent.filter(|p| !work.is(*p, W, "p")) {
        if work.is(p, W, "hyperlink") {
            return true;
        }
        parent = work.parent(p);
    }
    false
}

/// The run's `w:rStyle`, `w:b`, `w:bCs`, `w:i`, `w:iCs` as the look says (an existing one
/// keeps its place and its other attributes), its `w:rPr` made in schema order and taken
/// away when left empty.
fn write_look(work: &mut Work, run: Id, look: &Look<'_>, orders: &Orders) {
    let properties = work.find(run, W, "rPr").unwrap_or_else(|| {
        let p = work.new_element(work.element_name(run, W, "rPr", "w"));
        insert_in_order(work, run, p, orders);
        p
    });
    set_child(work, properties, "rStyle", look.style.map(|s| ("val", s)), orders);
    for (local, on) in [
        ("b", look.bold),
        ("bCs", look.bold),
        ("i", look.italic),
        ("iCs", look.italic),
    ] {
        set_child(work, properties, local, on.then_some(("", "")), orders);
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}

/// docx-agent's `_formatting._set_child`: `parent`'s `w:{local}` removed (`None`), or
/// kept or made in schema order with the attribute given (an empty name: none).
fn set_child(work: &mut Work, parent: Id, local: &str, attribute: Option<(&str, &str)>, orders: &Orders) {
    let node = work.find(parent, W, local);
    let Some((name, value)) = attribute else {
        if let Some(node) = node {
            work.remove(node);
        }
        return;
    };
    let node = node.unwrap_or_else(|| {
        let n = work.new_element(work.element_name(parent, W, local, "w"));
        insert_in_order(work, parent, n, orders);
        n
    });
    if !name.is_empty() {
        let qname = work.qname(W, name, "w");
        work.set(node, qname, value);
    }
}
