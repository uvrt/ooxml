//! Owns: [`InsertField`], a field put at a position as docx-agent's `_insert_field` puts
//! one -- complex (begin, instruction, separator, result, end: the begin's run in the
//! formatting new text there takes, the others in it too, the result `w:noProof`) or a
//! `w:fldSimple` (its result run `w:noProof`), tracked as an insertion (a simple field
//! stands outside revision containers, its result run inserted), its paragraph's text id
//! renewed, the result computed where it is not given -- and [`InsertCrossReference`], a
//! `REF` or `PAGEREF` field as Word's Insert Cross-reference writes one (docx-agent's
//! `insert_cross_reference`).
//!
//! Must not: write but through the sink.

use super::pages::{FieldLayout, FieldPlace};
use super::scan::all_fields;
use super::update::pass_on;
use super::values::Day;
use super::write::green_path;
use super::FieldEdited;
use crate::doc::{Draw, DrawFrom};
use crate::links::new_w;
use crate::marks::insert_in_order;
use crate::names::{W, W14, W16DU, XML};
use crate::policy::RevisionPolicy;
use crate::ranges::renamed_id;
use crate::set_text::block_key;
use crate::sink::{annotation_draws, EditSink, Numbered};
use crate::stamping::{stamp_document, w14_prefix};
use crate::text::atoms;
use crate::text::write::spell;
use crate::text_range::{range, range_id, Span};
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::text::OBJECT;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_xml::QName;
use std::sync::Arc;

/// A field inserted at a position.
#[derive(Clone)]
pub struct InsertField<'l> {
    /// The position (a range: at its end).
    pub at: String,
    /// The instruction (`PAGE`, `SEQ Figure \* ARABIC`), padded with a space each side.
    pub instruction: String,
    /// The cached result; `None`: computed where an update can, else left empty and listed.
    pub result: Option<String>,
    /// A `w:fldSimple`.
    pub simple: bool,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
    /// The day a `DATE` shows.
    pub today: Day,
    /// The layout of the document's versions.
    pub layout: Option<&'l dyn FieldLayout>,
}

impl std::fmt::Debug for InsertField<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsertField")
            .field("at", &self.at)
            .field("instruction", &self.instruction)
            .field("simple", &self.simple)
            .finish_non_exhaustive()
    }
}

/// Where a field goes: its paragraph and the offset.
pub(crate) struct Place {
    pub(crate) part: String,
    pub(crate) path: Vec<u32>,
    pub(crate) id: String,
    pub(crate) offset: usize,
}

/// The paragraph and offset `at` names (`_field_place`: a range's end; `start`: its
/// start, as a cross-reference takes it), refused outside the paragraph's text or inside
/// a field's result.
fn place(doc: &WordDoc, docx: &Docx, at: &str, start: bool) -> Result<Place> {
    let found = range(doc, docx, at)?;
    let span = Span::of(doc, docx, &found)?;
    let id = range_id(&found);
    let paragraph = if start { span.first() } else { span.last() };
    let paragraph = paragraph.ok_or_else(|| EditError::Path(span.part.clone()))?;
    let offset = if start { span.start } else { span.end };
    let work = Work::of(&paragraph.element, Vec::new());
    let items = atoms(&work, work.root());
    if offset > items.len() {
        return Err(EditError::Refused(format!("{id} is outside its paragraph's text")));
    }
    let field_at = |k: usize| items.get(k).and_then(|a| a.field);
    if 0 < offset && offset < items.len() && field_at(offset).is_some() && field_at(offset - 1) == field_at(offset) {
        return Err(EditError::Refused(format!("{id} is inside a field's result")));
    }
    Ok(Place {
        part: span.part.clone(),
        path: paragraph.path.to_vec(),
        id: paragraph.id.clone(),
        offset,
    })
}

/// `instruction` with a space at each end.
fn padded(instruction: &str) -> String {
    let mut out = String::new();
    if !instruction.starts_with(' ') {
        out.push(' ');
    }
    out.push_str(instruction);
    if !instruction.ends_with(' ') {
        out.push(' ');
    }
    out
}

/// A `w:instrText` holding `text`, its spaces kept.
fn instruction_node(work: &mut Work, parent: Id, text: &str) -> Id {
    let node = new_w(work, parent, "instrText", &[]);
    let space = work.qname(XML, "space", "xml");
    work.set(node, space, "preserve");
    work.set_text(node, text);
    node
}

/// The field's nodes put at `offset` of the paragraph (the root of `work`): the simple
/// field, or the complex field's five runs (`_field_runs`).
fn field_runs(
    work: &mut Work,
    offset: usize,
    instruction: &str,
    simple: bool,
    orders: &crate::validate::Orders,
) -> Result<Vec<Id>> {
    let paragraph = work.root();
    if simple {
        let node = new_w(work, paragraph, "fldSimple", &[("instr", instruction)]);
        let properties = super::write::no_proof_properties(work, node);
        let run = super::write::result_run(work, node, "", Some(properties));
        work.append(node, run);
        let (parent, index) = crate::inline::position(work, paragraph, offset)?;
        work.insert(parent, index, node);
        return Ok(vec![node]);
    }
    let begin = new_w(work, paragraph, "fldChar", &[("fldCharType", "begin")]);
    let first = crate::inline::new_run(work, paragraph, offset, &[begin])?;
    let properties = work.find(first, W, "rPr");
    let result_properties = match properties {
        Some(p) => work.deep_copy(p),
        None => new_w(work, first, "rPr", &[]),
    };
    if work.find(result_properties, W, "noProof").is_none() {
        let proof = new_w(work, result_properties, "noProof", &[]);
        insert_in_order(work, result_properties, proof, orders);
    }
    let mut runs = vec![first];
    let mut previous = first;
    for step in 0..4 {
        let run = new_w(work, paragraph, "r", &[]);
        let props = if step == 2 { Some(result_properties) } else { properties };
        if let Some(props) = props.filter(|p| !work.children(*p).is_empty()) {
            let copy = work.deep_copy(props);
            work.append(run, copy);
        }
        let children = match step {
            0 => vec![instruction_node(work, run, instruction)],
            1 => vec![new_w(work, run, "fldChar", &[("fldCharType", "separate")])],
            2 => Vec::new(),
            _ => vec![new_w(work, run, "fldChar", &[("fldCharType", "end")])],
        };
        for child in children {
            work.append(run, child);
        }
        work.add_next(previous, run);
        previous = run;
        runs.push(run);
    }
    Ok(runs)
}

/// Tracked, `nodes` recorded as inserted where they stand (`_track_inserted`; a simple
/// field moved out of revision containers first, its result runs the insertion).
pub(crate) fn track(
    sink: &mut EditSink<'_, '_>,
    work: &mut Work,
    nodes: &[Id],
    simple: bool,
) -> Result<Option<(Numbered, bool)>> {
    let RevisionPolicy::Tracked(tracking) = sink.policy().clone() else {
        return Ok(None);
    };
    let ((), numbered, dated) = sink.stamped(tracking, |stamp| {
        let runs: Vec<Id> = if simple {
            let Some(&field) = nodes.first() else { return Ok(()) };
            crate::track::place_outside_revisions(work, field, stamp);
            work.children(field)
                .iter()
                .copied()
                .filter(|c| work.is(*c, W, "r"))
                .collect()
        } else {
            nodes.to_vec()
        };
        let (Some(&first), true) = (runs.first(), !runs.is_empty()) else {
            return Ok(());
        };
        let (Some(parent), Some(index)) = (work.parent(first), work.index(first)) else {
            return Ok(());
        };
        for run in &runs {
            work.remove(*run);
        }
        crate::track::insert_nodes(work, parent, index, &runs, stamp);
        Ok(())
    })?;
    Ok(Some((numbered, dated)))
}

/// The paragraph `work` holds put back at `place` (`renew`: its text id renewed), the
/// revision ids it took drawn where the change is applied.
pub(crate) fn put_back(
    sink: &mut EditSink<'_, '_>,
    work: &Work,
    (place, renew): (&Place, bool),
    tracked: Option<(Numbered, bool)>,
    renamed: &[(String, String)],
) -> Result<()> {
    let part = place.part.clone();
    let tree = sink.doc().tree(&part)?.ok_or_else(|| EditError::NoPart(part.clone()))?;
    let new = work.build().ok_or_else(|| EditError::Path(part.clone()))?;
    let mut draws = tracked.map(|(n, _)| annotation_draws(&new, n, 0)).unwrap_or_default();
    if renew && new.attribute(Some(W14), "paraId").is_some() {
        let prefix = w14_prefix(tree.root(), &place.path);
        draws.push(Draw {
            from: DrawFrom::Seed(Arc::from(format!("{part}\0text"))),
            node: 0,
            at: Vec::new(),
            attribute: Arc::new(QName::new(Some(&prefix), "textId", Some(W14))),
        });
    }
    let id = renamed_id(renamed, &place.id);
    sink.replace_drawing(&part, &place.path, new, vec![block_key(&part, &id)], draws)?;
    if let (RevisionPolicy::Tracked(tracking), Some((numbered, dated))) = (sink.policy().clone(), tracked) {
        sink.people(
            (numbered.to > numbered.from).then_some(tracking.author.as_str()),
            Some(&tracking.author),
        )?;
        if dated {
            sink.ensure_w14(&part, false, &[("w16du", W16DU)])?;
        }
    }
    Ok(())
}

impl InsertField<'_> {
    fn check(&self, doc: &WordDoc) -> Result<()> {
        let keyword = self.instruction.split_whitespace().next().unwrap_or("").to_uppercase();
        if keyword == "TOC" {
            return Err(EditError::Refused(
                "a table of contents is inserted with insert_toc".to_owned(),
            ));
        }
        if keyword == "HYPERLINK" {
            return Err(EditError::Unsupported(
                "a HYPERLINK field (written as a hyperlink)".to_owned(),
            ));
        }
        place(doc, &*doc.docx()?, &self.at, false).map(|_| ())
    }

    /// The edit, through `sink`.
    pub(crate) fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<FieldEdited> {
        self.check(sink.doc())?;
        let docx = sink.docx()?;
        let here = place(sink.doc(), &docx, &self.at, false)?;
        sink.read(block_key(&here.part, &here.id));
        if self.result.is_none() {
            sink.read_everything(); // its result counts or reads the rest of the document
        }
        let renamed = stamp_document(&mut *sink, &docx)?;
        let tree = sink
            .doc()
            .tree(&here.part)?
            .ok_or_else(|| EditError::NoPart(here.part.clone()))?;
        let element = at_path(tree.root(), &here.path).ok_or_else(|| EditError::Path(here.part.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &here.path));
        let shared = sink.doc().shared_handle();
        let instruction = padded(&self.instruction);
        let nodes = field_runs(&mut work, here.offset, &instruction, self.simple, shared.facts.orders())?;
        if let Some(text) = &self.result {
            let result = if self.simple {
                nodes.first().and_then(|f| work.find(*f, W, "r"))
            } else {
                nodes.get(3).copied()
            };
            if let Some(run) = result {
                for child in spell(&mut work, text, None) {
                    work.append(run, child);
                }
            }
        }
        let tracked = track(&mut *sink, &mut work, &nodes, self.simple)?;
        let begin = if self.simple {
            nodes.first().copied()
        } else {
            nodes.first().and_then(|r| work.find(*r, W, "fldChar"))
        };
        let below = begin
            .and_then(|b| green_path(&work, b))
            .ok_or_else(|| EditError::Path(here.part.clone()))?;
        put_back(&mut *sink, &work, (&here, true), tracked, &renamed)?;
        let mut path = here.path.clone();
        path.extend(below);
        let docx = sink.docx()?;
        let located = all_fields(&docx)
            .into_iter()
            .find(|l| l.part == here.part && l.found.begin == path)
            .ok_or_else(|| EditError::Path(here.part.clone()))?;
        let id = located.id();
        let unknown = if self.result.is_none() {
            pass_on(&mut *sink, &located, self.layout, self.today)?
        } else {
            Vec::new()
        };
        Ok(FieldEdited {
            id: Some(id.clone()),
            changed: true,
            created: vec![id],
            renamed,
            unknown,
            ..FieldEdited::default()
        })
    }
}

impl Command<WordDoc> for InsertField<'_> {
    type Out = FieldEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<FieldEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

/// A cross-reference inserted: a `REF` (the bookmark's text) or `PAGEREF` (its page) field
/// with `\h`, its result computed before.
#[derive(Clone)]
pub struct InsertCrossReference<'l> {
    /// The position (a range: at its start).
    pub at: String,
    /// The bookmark.
    pub bookmark: String,
    /// Its page rather than its text.
    pub page: bool,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
    /// The layout of the document's versions.
    pub layout: Option<&'l dyn FieldLayout>,
}

impl std::fmt::Debug for InsertCrossReference<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsertCrossReference")
            .field("at", &self.at)
            .field("bookmark", &self.bookmark)
            .field("page", &self.page)
            .finish_non_exhaustive()
    }
}

impl InsertCrossReference<'_> {
    /// The bookmark's name as stored, then the place.
    fn check(&self, doc: &WordDoc, docx: &Docx) -> Result<(String, Place)> {
        let name = ooxml_word_model::address::bookmark(docx, &self.bookmark).map_err(EditError::Address)?;
        let here = place(doc, docx, &self.at, true)?;
        Ok((name.name.clone(), here))
    }

    /// The result: the bookmark's text's first line, or the number of the page it starts
    /// on (`1` and listed where the layout cannot say).
    fn result(&self, sink: &EditSink<'_, '_>, docx: &Docx, name: &str) -> Result<(String, bool)> {
        let marked = range(sink.doc(), docx, &format!("bm:{name}"))?;
        let span = Span::of(sink.doc(), docx, &marked)?;
        let (text, unknown) = if self.page {
            let layout = self
                .layout
                .ok_or_else(|| EditError::Unsupported("a page cross-reference without the layout".to_owned()))?;
            let laid = layout
                .lay_out(sink.doc().package())
                .map_err(|why| EditError::Unsupported(format!("the layout of this version: {why}")))?;
            let start = span.first().map(|f| f.id.clone()).unwrap_or_default();
            let at = if span.part == docx.main { Some(start) } else { None };
            match at.map(|a| laid.page_reference(" PAGEREF x ", FieldPlace::Body(&a))) {
                Some(Ok(label)) => (label, false),
                _ => ("1".to_owned(), true),
            }
        } else {
            (span.text().split('\n').next().unwrap_or("").to_owned(), false)
        };
        let text = text.replace(OBJECT, "");
        Ok((if text.is_empty() { " ".to_owned() } else { text }, unknown))
    }

    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<FieldEdited> {
        let docx = sink.docx()?;
        let (name, here) = self.check(sink.doc(), &docx)?;
        sink.read_everything(); // the bookmark it shows, wherever it is
        let (text, unknown) = self.result(sink, &docx, &name)?;
        let instruction = format!(" {} {name} \\h ", if self.page { "PAGEREF" } else { "REF" });
        let renamed = stamp_document(&mut *sink, &docx)?;
        let tree = sink
            .doc()
            .tree(&here.part)?
            .ok_or_else(|| EditError::NoPart(here.part.clone()))?;
        let element = at_path(tree.root(), &here.path).ok_or_else(|| EditError::Path(here.part.clone()))?;
        let mut work = Work::of(element, scope_above(tree.root(), &here.path));
        let paragraph = work.root();
        let begin = new_w(&mut work, paragraph, "fldChar", &[("fldCharType", "begin")]);
        let first = crate::inline::new_run(&mut work, paragraph, here.offset, &[begin])?;
        let properties = work.find(first, W, "rPr");
        let mut runs = vec![first];
        let mut previous = first;
        for step in 0..4 {
            let run = new_w(&mut work, paragraph, "r", &[]);
            if let Some(properties) = properties {
                let copy = work.deep_copy(properties);
                work.append(run, copy);
            }
            let children = match step {
                0 => vec![instruction_node(&mut work, run, &instruction)],
                1 => vec![new_w(&mut work, run, "fldChar", &[("fldCharType", "separate")])],
                2 => spell(&mut work, &text, None),
                _ => vec![new_w(&mut work, run, "fldChar", &[("fldCharType", "end")])],
            };
            for child in children {
                work.append(run, child);
            }
            work.add_next(previous, run);
            previous = run;
            runs.push(run);
        }
        let tracked = track(&mut *sink, &mut work, &runs, false)?;
        put_back(&mut *sink, &work, (&here, true), tracked, &renamed)?;
        let id = renamed_id(&renamed, &here.id);
        Ok(FieldEdited {
            id: Some(id),
            changed: true,
            renamed,
            unknown: if unknown {
                vec![format!("PAGEREF {name}")]
            } else {
                Vec::new()
            },
            ..FieldEdited::default()
        })
    }
}

impl Command<WordDoc> for InsertCrossReference<'_> {
    type Out = FieldEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(doc, &*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<FieldEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::padded;

    #[test]
    fn instructions_are_padded() {
        assert_eq!(padded("PAGE"), " PAGE ");
        assert_eq!(padded(" NUMPAGES "), " NUMPAGES ");
    }
}
