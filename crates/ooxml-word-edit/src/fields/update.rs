//! Owns: [`UpdateFields`], fields' cached results computed as docx-agent's
//! `update_fields` computes them -- untracked, in passes over a layout of the document as
//! each pass leaves it until the results hold (`_update_loop`, at most four), `SEQ` fields
//! first and then the rest in order, a table of contents rebuilt where it stands; what the
//! layout cannot say left as it is and listed (never marked dirty: Word would ask on
//! opening) -- and the passes themselves ([`settle`]), which inserting a table of contents
//! runs too.
//!
//! A field is known across passes by its place among its part's fields, the fields a
//! table of contents being rebuilt holds aside (they are made anew each pass, as docx-agent
//! knows a field by its element).
//!
//! Must not: lay out (the [`FieldLayout`] port does), or write but through the sink.

use super::pages::{FieldLayout, LaidFields};
use super::scan::{all_fields, paragraph_of, result_text, Located};
use super::toc::build::EMPTY_TOC;
use super::values::{sequence_values, value, Day};
use super::write::{locate, write_complex, write_simple, Complex};
use super::FieldEdited;
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::set_text::block_key;
use crate::sink::EditSink;
use crate::work::{scope_above, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::marks::Bookmarks;
use ooxml_word_model::docx::{at_path, Docx};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The fields whose results come from the layout.
const LAID_OUT: [&str; 6] = ["PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF", "TOC", "STYLEREF"];
/// How many passes an update makes at most.
pub(crate) const PASSES: usize = 4;

/// The fields' results updated.
#[derive(Clone)]
pub struct UpdateFields<'l> {
    /// Only these fields (`fld:...`); `None`: every one.
    pub ids: Option<Vec<String>>,
    /// The day `DATE` fields show.
    pub today: Day,
    /// The layout of the document's versions.
    pub layout: Option<&'l dyn FieldLayout>,
}

impl std::fmt::Debug for UpdateFields<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateFields")
            .field("ids", &self.ids)
            .field("today", &self.today)
            .finish_non_exhaustive()
    }
}

/// A field as passes know it: its part and its place among the part's fields that are
/// not inside a table of contents being rebuilt.
type Known = (String, usize);

/// The fields of `docx` by what passes know them as, given which tables of contents are rebuilt.
fn identified(docx: &Docx, held: &[Known]) -> Vec<(Known, Located)> {
    let mut out = Vec::new();
    let mut part = String::new();
    let mut ordinal = 0;
    let mut aside: Vec<usize> = Vec::new();
    for located in all_fields(docx) {
        if located.part != part {
            part.clone_from(&located.part);
            ordinal = 0;
            aside.clear();
        }
        if aside.contains(&located.found.number) {
            continue;
        }
        let known = (located.part.clone(), ordinal);
        ordinal += 1;
        if located.found.keyword() == "TOC" && held.contains(&known) {
            aside.extend(located.found.nested.iter().copied());
        }
        out.push((known, located));
    }
    out
}

/// What one pass needs.
struct Pass<'p> {
    laid: Option<Box<dyn LaidFields + 'p>>,
    today: Day,
}

/// The results of the fields `held` computed in passes until they hold: the fields the
/// layout could not say (by id), with a note when the results did not settle.
///
/// # Errors
/// [`EditError::Unsupported`] where a field needs the layout and there is none, or for a
/// field whose result is not ported yet.
pub(crate) fn settle(
    sink: &mut EditSink<'_, '_>,
    held: &[Known],
    layout: Option<&dyn FieldLayout>,
    today: Day,
) -> Result<Vec<String>> {
    let mut previous: Option<Vec<(String, String)>> = None;
    let mut unknown = Vec::new();
    for _ in 0..PASSES {
        unknown = pass(sink, held, layout, today)?;
        let state: Vec<(String, String)> = state(&*sink.docx()?);
        if previous.as_ref() == Some(&state) {
            return Ok(unknown);
        }
        previous = Some(state);
    }
    unknown.push("(the page numbers did not settle)".to_owned());
    Ok(unknown)
}

/// One pass over the one field `located` (inserting it computes its result so:
/// `_compute`); the fields the layout could not say.
///
/// # Errors
/// As [`settle`].
pub(crate) fn pass_on(
    sink: &mut EditSink<'_, '_>,
    located: &Located,
    layout: Option<&dyn FieldLayout>,
    today: Day,
) -> Result<Vec<String>> {
    let docx = sink.docx()?;
    let known = identified(&docx, &[])
        .into_iter()
        .find(|(_, l)| l.part == located.part && l.found.begin == located.found.begin)
        .map(|(i, _)| i);
    match known {
        Some(known) => pass(sink, &[known], layout, today),
        None => Ok(Vec::new()),
    }
}

/// Every field's instruction and result.
fn state(docx: &Docx) -> Vec<(String, String)> {
    all_fields(docx)
        .into_iter()
        .map(|l| {
            let text = docx
                .part(&l.part)
                .map_or_else(String::new, |p| result_text(p.root(), &l.found));
            (l.found.instruction, text)
        })
        .collect()
}

/// One pass (`_compute`): the results written from one layout; the ids of the fields it
/// could not say.
fn pass(
    sink: &mut EditSink<'_, '_>,
    held: &[Known],
    layout: Option<&dyn FieldLayout>,
    today: Day,
) -> Result<Vec<String>> {
    let docx = sink.docx()?;
    let held_set: HashSet<&Known> = held.iter().collect();
    let fields = identified(&docx, held);
    let current: Vec<&Located> = fields
        .iter()
        .filter(|(i, _)| held_set.contains(i))
        .map(|(_, l)| l)
        .collect();
    if current.iter().any(|l| l.found.keyword() == "STYLEREF") {
        return Err(EditError::Unsupported("a STYLEREF field's update".to_owned()));
    }
    let needs = current.iter().any(|l| LAID_OUT.contains(&l.found.keyword().as_str()));
    let laid = match (needs, layout) {
        (false, _) => None,
        (true, None) => {
            return Err(EditError::Unsupported(
                "a field update that needs the layout".to_owned(),
            ))
        }
        (true, Some(layout)) => Some(
            layout
                .lay_out(sink.doc().package())
                .map_err(|why| EditError::Unsupported(format!("the layout of this version: {why}")))?,
        ),
    };
    let pass = Pass { laid, today };
    let mut unknown = Vec::new();
    let sequences = sequence_values(&all_fields(&docx));
    let mut writes = Vec::new();
    for located in current.iter().filter(|l| l.found.keyword() == "SEQ") {
        match sequences.get(&(located.part.clone(), located.found.number)) {
            Some(Some(text)) => writes.push(((*located).clone(), text.clone())),
            _ => unknown.push(located.id()),
        }
    }
    apply(sink, &writes)?;
    let order: Vec<Known> = fields
        .iter()
        .filter(|(i, l)| held_set.contains(i) && l.found.keyword() != "SEQ")
        .map(|(i, _)| i.clone())
        .collect();
    let mut batch: Vec<(Located, String)> = Vec::new();
    let mut docx = sink.docx()?;
    let mut now: HashMap<Known, Located> = identified(&docx, held).into_iter().collect();
    let mut marks = Bookmarks::of(&docx);
    for known in order {
        let Some(located) = now.get(&known).cloned() else {
            continue;
        };
        if located.found.keyword() == "TOC" {
            apply(sink, &std::mem::take(&mut batch))?;
            drop(marks);
            docx = sink.docx()?;
            let fresh = identified(&docx, held).into_iter().find(|(i, _)| *i == known);
            let laid = pass
                .laid
                .as_deref()
                .ok_or_else(|| EditError::Unsupported("no layout".to_owned()))?;
            if let Some((_, toc)) = fresh {
                super::toc::rebuild(sink, &toc, laid, &mut unknown)?;
            }
            docx = sink.docx()?;
            now = identified(&docx, held).into_iter().collect();
            marks = Bookmarks::of(&docx);
            continue;
        }
        match value(sink.doc(), (&docx, &marks), &located, pass.laid.as_deref(), pass.today) {
            Ok(Some(text)) => batch.push((located, text)),
            Ok(None) => {}
            Err(_) => unknown.push(located.id()),
        }
    }
    apply(sink, &batch)?;
    Ok(unknown)
}

/// Writes by the part and paragraph they begin in.
type Groups<'w> = BTreeMap<(String, Vec<u32>), Vec<&'w (Located, String)>>;

/// `writes` (fields and their new results) written, a copy of each paragraph they begin
/// in made once.
fn apply(sink: &mut EditSink<'_, '_>, writes: &[(Located, String)]) -> Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    let docx = sink.docx()?;
    let mut groups: Groups<'_> = BTreeMap::new();
    for write in writes {
        let Some(part) = docx.part(&write.0.part) else { continue };
        if let Some(paragraph) = paragraph_of(part.root(), &write.0.found.begin) {
            groups.entry((write.0.part.clone(), paragraph)).or_default().push(write);
        }
    }
    for ((part, path), group) in groups {
        write_paragraph(sink, &docx, (&part, &path), &group)?;
    }
    Ok(())
}

/// The fields of one paragraph written.
fn write_paragraph(
    sink: &mut EditSink<'_, '_>,
    docx: &Docx,
    (part, path): (&str, &[u32]),
    group: &[&(Located, String)],
) -> Result<()> {
    let story = docx.part(part).ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    let root = story.root();
    let element = at_path(root, path).ok_or_else(|| EditError::Path(part.to_owned()))?;
    let mut work = Work::of(element, scope_above(root, path));
    let below = |p: &[u32]| p.get(path.len()..).map(<[u32]>::to_vec);
    let mut changed = false;
    let shared = sink.doc().shared_handle();
    let orders = shared.facts.orders();
    let mut places = Vec::new();
    for (located, text) in group {
        let found = &located.found;
        let begin = below(&found.begin).and_then(|p| locate(&work, element, &p));
        let Some(begin) = begin else { continue };
        if found.simple {
            places.push((None, begin, text.as_str(), String::new()));
            continue;
        }
        let same = found
            .end
            .as_ref()
            .and_then(|e| paragraph_of(root, e))
            .is_some_and(|p| p == path);
        let complex = if same {
            let end = found
                .end
                .as_deref()
                .and_then(below)
                .and_then(|p| locate(&work, element, &p));
            let separate = found
                .separate
                .as_deref()
                .and_then(below)
                .and_then(|p| locate(&work, element, &p));
            end.map(|end| Complex {
                begin,
                separate,
                end,
                keyword: found.keyword(),
            })
        } else {
            None
        };
        places.push((Some(complex), begin, text.as_str(), result_text(root, found)));
    }
    for (complex, begin, text, now) in places {
        match complex {
            None => changed |= write_simple(&mut work, begin, text),
            Some(field) => {
                if let Some(field) = field {
                    changed |= write_complex(&mut work, &field, text, &now, orders);
                }
                if work.get(begin, Some(W), "dirty").is_some() {
                    work.remove_attr(begin, Some(W), "dirty");
                    changed = true;
                }
            }
        }
    }
    if !changed {
        return Ok(());
    }
    let new = work.build().ok_or_else(|| EditError::Path(part.to_owned()))?;
    let id = story.ids.paragraph_at(path).map(|p| p.id).unwrap_or_default();
    sink.replace(part, path, new, vec![block_key(part, &id)])
}

/// The warnings for the tables of contents (of `ids`, else every one) that list nothing
/// (`_empty_tocs`).
pub(crate) fn empty_tocs(docx: &Docx, ids: Option<&[String]>) -> Vec<String> {
    let mut out = Vec::new();
    for located in all_fields(docx) {
        let id = located.id();
        if located.found.keyword() != "TOC" || ids.is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        let text = docx
            .part(&located.part)
            .map_or_else(String::new, |p| result_text(p.root(), &located.found));
        let text = text.trim();
        if text.is_empty() || text == EMPTY_TOC {
            out.push(format!(
                "{id}: the table of contents has no entries: no heading at its levels yet; add the headings, then update the fields"
            ));
        }
    }
    out
}

/// Every field's result by id.
fn results(docx: &Docx) -> Vec<(String, String)> {
    all_fields(docx)
        .into_iter()
        .map(|l| {
            let text = docx
                .part(&l.part)
                .map_or_else(String::new, |p| result_text(p.root(), &l.found));
            (l.id(), text)
        })
        .collect()
}

impl Command<WordDoc> for UpdateFields<'_> {
    type Out = FieldEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.held(&*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<FieldEdited> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        sink.read_everything();
        let docx = sink.docx()?;
        let before = results(&docx);
        let held = self.held(&docx)?;
        let unknown = settle(&mut sink, &held, self.layout, self.today)?;
        let docx = sink.docx()?;
        let after = results(&docx);
        let count = after
            .iter()
            .filter(|(id, text)| !before.iter().any(|(i, t)| i == id && t == text))
            .count();
        let mut warnings: Vec<String> = unknown.iter().map(|u| format!("{u}: the layout cannot say")).collect();
        warnings.extend(empty_tocs(&docx, self.ids.as_deref()));
        crate::after::after_edit(&mut sink)?;
        Ok(FieldEdited {
            changed: count > 0 || !unknown.is_empty(),
            count,
            unknown,
            warnings,
            ..FieldEdited::default()
        })
    }
}

impl UpdateFields<'_> {
    /// What the passes know the fields to update as (as [`identified`] names them once these
    /// are held).
    fn held(&self, docx: &Docx) -> Result<Vec<Known>> {
        let everything = all_fields(docx);
        if let Some(ids) = &self.ids {
            let mut missing: Vec<&str> = ids
                .iter()
                .filter(|id| !everything.iter().any(|l| l.id() == **id))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                missing.sort_unstable();
                missing.dedup();
                return Err(EditError::Refused(format!("no field {}", missing.join(", "))));
            }
        }
        let mut held = Vec::new();
        let mut part = String::new();
        let mut ordinal = 0;
        let mut aside: Vec<usize> = Vec::new();
        for located in everything {
            if located.part != part {
                part.clone_from(&located.part);
                ordinal = 0;
                aside.clear();
            }
            if aside.contains(&located.found.number) {
                continue;
            }
            let known = (located.part.clone(), ordinal);
            ordinal += 1;
            if self.ids.as_ref().is_none_or(|ids| ids.contains(&located.id())) {
                if located.found.keyword() == "TOC" {
                    aside.extend(located.found.nested.iter().copied());
                }
                held.push(known);
            }
        }
        Ok(held)
    }
}
