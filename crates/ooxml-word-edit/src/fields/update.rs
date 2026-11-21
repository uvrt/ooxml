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
//! Must not: lay out (the [`FieldLayout`](super::pages::FieldLayout) port does), or write but through the sink.

use super::pages::{EditLayout, LaidFields};
use super::scan::{all_fields, paragraph_of, result_text, same_result, Located, Marks};
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
use ooxml_numfmt::Locale;
use ooxml_store::{Command, Txn};
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
    pub layout: Option<EditLayout<'l>>,
    /// The locale a table of contents' style names are read under (the host's document
    /// locale); `None`: the document's own language.
    pub locale: Option<&'l Locale>,
}

impl std::fmt::Debug for UpdateFields<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateFields")
            .field("ids", &self.ids)
            .field("today", &self.today)
            .field("locale", &self.locale)
            .finish_non_exhaustive()
    }
}

/// What a field update reads beside the document: the day a `DATE` shows, the layout of
/// the document's versions, and the locale a table of contents' style names are read under.
#[derive(Clone, Copy)]
pub(crate) struct Inputs<'l> {
    /// The day a `DATE` shows.
    pub(crate) today: Day,
    /// The layout of the document's versions.
    pub(crate) layout: Option<EditLayout<'l>>,
    /// The call's locale (`None`: the document's own language).
    pub(crate) locale: Option<&'l Locale>,
}

/// A field as passes know it: its part and its place among the part's fields that are
/// not inside a table of contents being rebuilt.
type Known = (String, usize);

/// Each field of `fields` (a version's, as [`all_fields`] lists them) with its place among
/// its part's fields as passes know it, given which tables of contents are rebuilt (`held`):
/// the fields a rebuilt table of contents holds aside are not met.
fn each_known<'f>(fields: &'f [Located], held: &HashSet<(&str, usize)>, each: &mut dyn FnMut(usize, &'f Located)) {
    let mut part: &str = "";
    let mut ordinal = 0;
    let mut aside: HashSet<usize> = HashSet::new();
    for located in fields {
        if located.part != part {
            part = &located.part;
            ordinal = 0;
            aside.clear();
        }
        if aside.contains(&located.found.number) {
            continue;
        }
        if held.contains(&(part, ordinal)) && located.found.keyword() == "TOC" {
            aside.extend(located.found.nested.iter().copied());
        }
        each(ordinal, located);
        ordinal += 1;
    }
}

/// The fields `held` of `docx`, by what passes know them as, in document order (only these
/// are copied out: an update of one field does not copy the document's thousands).
fn identified(doc: &WordDoc, docx: &Docx, held: &[Known]) -> Vec<(Known, Located)> {
    let wanted: HashSet<(&str, usize)> = held.iter().map(|(part, k)| (part.as_str(), *k)).collect();
    let fields = all_fields(doc, docx);
    let mut out = Vec::new();
    each_known(&fields, &wanted, &mut |ordinal, located| {
        if wanted.contains(&(located.part.as_str(), ordinal)) {
            out.push(((located.part.clone(), ordinal), located.clone()));
        }
    });
    out
}

/// What one pass needs.
struct Pass<'p> {
    laid: Option<Box<dyn LaidFields + 'p>>,
    today: Day,
    locale: Option<&'p Locale>,
}

/// The results of the fields `held` computed in passes until they hold: the fields the
/// layout could not say (by id), with a note when the results did not settle.
///
/// # Errors
/// [`EditError::Unsupported`] where a field needs the layout and there is none, or the
/// layout does not lay the version out.
pub(crate) fn settle(sink: &mut EditSink<'_, '_>, held: &[Known], inputs: Inputs<'_>) -> Result<Vec<String>> {
    // A pass that changes nothing has settled: the next would lay out the same version.
    let mut previous: Option<Vec<(String, String)>> = Some(state(sink.doc(), &*sink.docx()?, held));
    let mut unknown = Vec::new();
    for _ in 0..PASSES {
        unknown = pass(sink, held, inputs)?;
        let state: Vec<(String, String)> = state(sink.doc(), &*sink.docx()?, held);
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
pub(crate) fn pass_on(sink: &mut EditSink<'_, '_>, located: &Located, inputs: Inputs<'_>) -> Result<Vec<String>> {
    let docx = sink.docx()?;
    let mut known = None;
    each_known(&all_fields(sink.doc(), &docx), &HashSet::new(), &mut |ordinal, l| {
        if known.is_none() && l.part == located.part && l.found.begin == located.found.begin {
            known = Some((l.part.clone(), ordinal));
        }
    });
    match known {
        Some(known) => pass(sink, &[known], inputs),
        None => Ok(Vec::new()),
    }
}

/// The instruction and result of each field `held` (as passes know them): what a pass
/// changes. A field not held is written by no pass, and one a table of contents being
/// rebuilt holds aside is in that table's result, so the document's fields are as they
/// were exactly when these are.
fn state(doc: &WordDoc, docx: &Docx, held: &[Known]) -> Vec<(String, String)> {
    identified(doc, docx, held)
        .into_iter()
        .map(|(_, l)| {
            let text = docx
                .part(&l.part)
                .map_or_else(String::new, |p| result_text(p.root(), &l.found));
            (l.found.instruction, text)
        })
        .collect()
}

/// One pass (`_compute`): the results written from one layout; the ids of the fields it
/// could not say.
fn pass(sink: &mut EditSink<'_, '_>, held: &[Known], inputs: Inputs<'_>) -> Result<Vec<String>> {
    let Inputs { today, layout, locale } = inputs;
    let docx = sink.docx()?;
    let fields = identified(sink.doc(), &docx, held);
    let current: Vec<&Located> = fields.iter().map(|(_, l)| l).collect();
    let needs = current.iter().any(|l| LAID_OUT.contains(&l.found.keyword().as_str()));
    let laid = match (needs, layout) {
        (false, _) => None,
        (true, None) => {
            return Err(EditError::Unsupported(
                "a field update that needs the layout".to_owned(),
            ))
        }
        (true, Some(layout)) => Some(layout.lay_out(sink.doc().package())?),
    };
    let pass = Pass { laid, today, locale };
    let mut unknown = Vec::new();
    let writes = sequence_writes(sink.doc(), &docx, &current, &mut unknown);
    apply(sink, &writes)?;
    let order: Vec<Known> = fields
        .iter()
        .filter(|(_, l)| l.found.keyword() != "SEQ")
        .map(|(i, _)| i.clone())
        .collect();
    let mut batch: Vec<(Located, String)> = Vec::new();
    let mut docx = sink.docx()?;
    let mut now: HashMap<Known, Located> = identified(sink.doc(), &docx, held).into_iter().collect();
    // The bookmarks REF and PAGEREF look up.
    let names = order
        .iter()
        .filter_map(|k| now.get(k))
        .filter(|l| matches!(l.found.keyword().as_str(), "REF" | "PAGEREF"))
        .count();
    let mut marks = Marks::of(&docx, names);
    for known in order {
        let Some(located) = now.get(&known).cloned() else {
            continue;
        };
        if located.found.keyword() == "TOC" {
            apply(sink, &std::mem::take(&mut batch))?;
            drop(marks);
            docx = sink.docx()?;
            let fresh = identified(sink.doc(), &docx, held)
                .into_iter()
                .find(|(i, _)| *i == known);
            let laid = pass
                .laid
                .as_deref()
                .ok_or_else(|| EditError::Unsupported("no layout".to_owned()))?;
            if let Some((_, toc)) = fresh {
                super::toc::rebuild(sink, &toc, laid, pass.locale, &mut unknown)?;
            }
            docx = sink.docx()?;
            now = identified(sink.doc(), &docx, held).into_iter().collect();
            marks = Marks::of(&docx, names);
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

/// The `SEQ` fields among `current` with their numbers (counted over every field, so only
/// where one is held); those that have none added to `unknown`.
fn sequence_writes(
    doc: &WordDoc,
    docx: &Docx,
    current: &[&Located],
    unknown: &mut Vec<String>,
) -> Vec<(Located, String)> {
    let numbered: Vec<&Located> = current.iter().copied().filter(|l| l.found.keyword() == "SEQ").collect();
    if numbered.is_empty() {
        return Vec::new();
    }
    let sequences = sequence_values(&all_fields(doc, docx));
    let mut writes = Vec::new();
    for located in numbered {
        match sequences.get(&(located.part.clone(), located.found.number)) {
            Some(Some(text)) => writes.push((located.clone(), text.clone())),
            _ => unknown.push(located.id()),
        }
    }
    writes
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
pub(crate) fn empty_tocs(doc: &WordDoc, docx: &Docx, ids: Option<&[String]>) -> Vec<String> {
    let mut out = Vec::new();
    for located in all_fields(doc, docx).iter() {
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

/// How many fields of `after` (a version made from `before`) have an id `before` gives no
/// field with the same result (`_update_fields`' count): a field whose result stands in the
/// same elements of both versions is not read.
fn changed_results(doc: &WordDoc, before: (&Docx, &[Located]), after: &Docx) -> usize {
    // By id (`fld:<story>/<number>`), its parts not written out.
    let mut was: HashMap<(&str, usize), Vec<&Located>> = HashMap::new();
    for located in before.1 {
        was.entry((&located.story, located.found.number))
            .or_default()
            .push(located);
    }
    let text = |docx: &Docx, l: &Located| {
        docx.part(&l.part)
            .map_or_else(String::new, |p| result_text(p.root(), &l.found))
    };
    all_fields(doc, after)
        .iter()
        .filter(|now| {
            let Some(earlier) = was.get(&(now.story.as_str(), now.found.number)) else {
                return true;
            };
            let roots = (before.0.part(&now.part), after.part(&now.part));
            let mut text_now = None;
            !earlier.iter().any(|then| {
                if let (Some(old), Some(new)) = roots {
                    if then.part == now.part
                        && then.found == now.found
                        && same_result(old.root(), new.root(), &now.found)
                    {
                        return true;
                    }
                }
                text(before.0, then) == *text_now.get_or_insert_with(|| text(after, now))
            })
        })
        .count()
}

impl Command<WordDoc> for UpdateFields<'_> {
    type Out = FieldEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.held(doc, &*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<FieldEdited> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        sink.read_everything();
        let before = sink.docx()?;
        let fields_before = all_fields(sink.doc(), &before);
        let held = self.held(sink.doc(), &before)?;
        let inputs = Inputs {
            today: self.today,
            layout: self.layout,
            locale: self.locale,
        };
        let unknown = settle(&mut sink, &held, inputs)?;
        let docx = sink.docx()?;
        let count = changed_results(sink.doc(), (&before, &fields_before), &docx);
        let mut warnings: Vec<String> = unknown.iter().map(|u| format!("{u}: the layout cannot say")).collect();
        warnings.extend(empty_tocs(sink.doc(), &docx, self.ids.as_deref()));
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
    fn held(&self, doc: &WordDoc, docx: &Docx) -> Result<Vec<Known>> {
        let everything = all_fields(doc, docx);
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
        let mut aside: HashSet<usize> = HashSet::new();
        for located in everything.iter() {
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
