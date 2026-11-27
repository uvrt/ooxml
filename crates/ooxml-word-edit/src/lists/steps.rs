//! Owns: one paragraph's list step as docx-agent's `ListOps` makes it -- its checks before
//! anything changes (the messages docx-agent's refusals carry), then `_list_edit`: the
//! change (the numbering part's writes, then each paragraph's properties,
//! tracked as `w:pPrChange`s), the instances and definitions left unused removed, and the
//! edit's end (text boxes synced, revision ids made unique, the people part).
//!
//! Must not: write but through the sink.

use super::numbering::{self as nb, LEVELS};
use super::paragraph::{self as pp, LevelFormat};
use super::read::{self, Place};
use super::{ListAction, ListStep};
use crate::doc::Key;
use crate::marks::clean_properties;
use crate::names::{W, W16DU};
use crate::policy::RevisionPolicy;
use crate::set_text::{block_key, paragraph_has_annotations};
use crate::sink::{annotation_draws, EditSink};
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::numbering::{membership, Lists, Membership};
use ooxml_word_model::docx::Docx;
use std::collections::BTreeSet;

/// What one step came to.
pub(crate) enum Stepped {
    /// It changed the document.
    Done(ListStep),
    /// `remove` on a paragraph in no list: its id.
    Listless(String),
}

/// `ST_NumberFormat` values Word offers for a list level (docx-agent's `NUMBER_FORMATS`).
const NUMBER_FORMATS: [&str; 14] = [
    "bullet",
    "cardinalText",
    "decimal",
    "decimalEnclosedCircle",
    "decimalEnclosedFullstop",
    "decimalEnclosedParen",
    "decimalZero",
    "lowerLetter",
    "lowerRoman",
    "none",
    "ordinal",
    "ordinalText",
    "upperLetter",
    "upperRoman",
];

fn refused(message: String) -> EditError {
    EditError::Refused(message)
}

fn level_range(level: i64) -> Result<()> {
    if (0..LEVELS).contains(&level) {
        Ok(())
    } else {
        Err(refused(format!("level is 0 to {}", LEVELS - 1)))
    }
}

/// `action`'s refusals that hold for the paragraph at `place` before any step runs: its level
/// out of range, or, for a level, the paragraph in no list. The command checks every targeted
/// paragraph this way first, so a step's own refusal (a single-level list's, offering level 0)
/// comes only when its way out works for the whole target.
///
/// # Errors
/// docx-agent's refusals; when the paragraph is not found.
pub(crate) fn precheck(docx: &Docx, place: &Place, action: &ListAction) -> Result<()> {
    if let ListAction::Level(level) = action {
        level_range(*level)?;
        let read::At { id, element, .. } =
            read::at(docx, place).ok_or_else(|| EditError::no_element(place.part.clone()))?;
        if membership(docx, &Lists::read(docx), &element).is_none() {
            return Err(refused(format!("{id} is not in a list")));
        }
    }
    Ok(())
}

/// Run `action`'s step on the paragraph at `place`.
///
/// # Errors
/// docx-agent's refusals; when a part is not XML.
pub(crate) fn step(
    sink: &mut EditSink<'_, '_>,
    place: &Place,
    action: &ListAction,
) -> Result<Stepped> {
    let docx = sink.docx()?;
    let read::At { id, element, .. } =
        read::at(&docx, place).ok_or_else(|| EditError::no_element(place.part.clone()))?;
    let found = membership(&docx, &Lists::read(&docx), &element);
    let in_list = || found.clone().ok_or_else(|| refused(format!("{id} is not in a list")));
    let step = match action {
        ListAction::Add { kind, level } => {
            level_range(*level)?;
            list_edit(sink, place, found.as_ref(), |sink| {
                add(sink, place, kind, *level)
            })?
        }
        ListAction::Remove => {
            if found.is_none() {
                return Ok(Stepped::Listless(id));
            }
            list_edit(sink, place, found.as_ref(), |sink| remove(sink, place))?
        }
        ListAction::Level(level) => {
            level_range(*level)?;
            let num_id = in_list()?.num_id;
            if *level > 0 && Lists::read(&docx).single_level(&docx, num_id) {
                return Err(EditError::Argument {
                    field: "level".to_owned(),
                    message: format!(
                        "{id}'s list ({num_id}) is single-level: Word sets its paragraphs to no level but 0; \
                         to nest it, number it with a multi-level list first"
                    ),
                    options: vec!["0".to_owned()],
                });
            }
            list_edit(sink, place, found.as_ref(), |sink| {
                rewrite(sink, place, |work, p, orders| {
                    pp::set_numbering(work, p, (num_id, *level), orders)
                })?;
                Ok(BTreeSet::new())
            })?
        }
        ListAction::Restart(at) => {
            let list = in_list()?;
            let abstract_id = defined(&list)?;
            let mut done = list_edit(sink, place, found.as_ref(), |sink| {
                restart(sink, place, &list, abstract_id, *at)
            })?;
            if list.from_style {
                done.warnings.extend(restart_note(sink, place, &done.id)?);
            }
            done
        }
        ListAction::Continue(from) => {
            let list = in_list()?;
            let target = continued(&docx, place, &id, &list, from.as_deref())?;
            list_edit(sink, place, found.as_ref(), |sink| {
                continue_list(sink, place, &list, target)
            })?
        }
        ListAction::Format(format) => {
            let list = in_list()?;
            let level = format.level.unwrap_or(list.level);
            level_range(level)?;
            if format.format.as_deref().is_some_and(|f| !NUMBER_FORMATS.contains(&f)) {
                return Err(refused(format!("format is one of {}", NUMBER_FORMATS.join(", "))));
            }
            if format.start.is_some_and(|s| !(0..=32767).contains(&s)) {
                return Err(refused("start is 0 to 32767".to_owned()));
            }
            let abstract_id = defined(&list)?;
            list_edit(sink, place, found.as_ref(), |sink| {
                set_format(sink, &list, (abstract_id, level), format)
            })?
        }
    };
    Ok(Stepped::Done(step))
}

/// The definition a list's instance names, or docx-agent's refusal.
fn defined(list: &Membership) -> Result<i64> {
    list.abstract_id
        .ok_or_else(|| refused(format!("list instance {} names no abstract definition", list.num_id)))
}

/// docx-agent's `_list_edit`: `change`, remove what it (and the paragraph's list
/// before) left unused. The edit's end (text boxes synced, revision ids made unique) is the
/// command's, once after its last step: no step reads what it changes.
fn list_edit(
    sink: &mut EditSink<'_, '_>,
    place: &Place,
    before: Option<&Membership>,
    change: impl FnOnce(&mut EditSink<'_, '_>) -> Result<BTreeSet<i64>>,
) -> Result<ListStep> {
    let mut candidates = change(sink)?;
    if let Some(before) = before {
        candidates.insert(before.num_id);
    }
    let main = sink.docx()?.main.clone();
    let removed = nb::reap(sink, &main, &candidates)?;
    let docx = sink.docx()?;
    let read::At { id, .. } = read::at(&docx, place).ok_or_else(|| EditError::no_element(place.part.clone()))?;
    Ok(ListStep {
        id: id.into(),
        changed: true,
        removed,
        warnings: Vec::new(),
    })
}

/// Rewrite the paragraph at `place` by `edit` in its [`Work`] copy; tracked, its old
/// properties recorded in a `w:pPrChange` (and the people part kept).
fn rewrite(
    sink: &mut EditSink<'_, '_>,
    place: &Place,
    edit: impl FnOnce(&mut Work, Id, &crate::validate::Orders),
) -> Result<()> {
    let docx = sink.docx()?;
    let read::At { id, path, element, .. } =
        read::at(&docx, place).ok_or_else(|| EditError::no_element(place.part.clone()))?;
    let tree = sink
        .doc()
        .tree(&place.part)?
        .ok_or_else(|| EditError::no_part(place.part.clone()))?;
    let mut work = Work::of(&element, scope_above(tree.root(), &path));
    let paragraph = work.root();
    let properties = work.find(paragraph, W, "pPr");
    let old = clean_properties(&mut work, properties, paragraph);
    let shared = sink.doc().shared_handle();
    edit(&mut work, paragraph, shared.facts.orders());
    let wrote = sink.paragraph_properties_changed(&mut work, paragraph, old)?;
    let new = work.build().ok_or_else(|| EditError::unbuilt(place.part.clone()))?;
    if *new != *element {
        let draws = wrote
            .annotations
            .map(|n| annotation_draws(&new, n, 0))
            .unwrap_or_default();
        let mut keys = vec![block_key(&place.part, &id)];
        if paragraph_has_annotations(&element) || paragraph_has_annotations(&new) {
            keys.push(Key::Annotations);
        }
        sink.replace_drawing(&place.part, &path, new, keys, draws)?;
        if wrote.dated {
            sink.ensure_w14(&place.part, false, &[("w16du", W16DU)])?;
        }
    }
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
        sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    }
    Ok(())
}

/// The default paragraph style's id.
fn default_style(docx: &Docx) -> Option<String> {
    docx.default_style("paragraph")
        .and_then(|s| s.attribute(Some(W), "styleId"))
        .map(str::to_owned)
}

/// The paragraph right before `place` in its container, if the block there is one
/// (docx-agent's `_previous_paragraph`).
fn previous(docx: &Docx, place: &Place) -> Option<Place> {
    let read::At { story, path, .. } = read::at(docx, place)?;
    let (&last, above) = path.split_last()?;
    let parent = ooxml_word_model::docx::at_path(story.root(), above)?;
    let children = parent.children();
    let (k, is_paragraph) = (0..last as usize).rev().find_map(|k| match children.get(k) {
        Some(ooxml_xml::Node::Element(e))
            if e.name().namespace() == Some(W) && ["p", "tbl", "sdt"].contains(&e.name().local()) =>
        {
            Some((k, e.name().local() == "p"))
        }
        _ => None,
    })?;
    if !is_paragraph {
        return None;
    }
    let mut at = above.to_vec();
    at.push(u32::try_from(k).ok()?);
    let found = story.ids.paragraph_at(&at)?;
    Some(Place {
        part: place.part.clone(),
        index: found.index,
    })
}

/// docx-agent's `add_to_list` change: join the list right before of the same kind, else a
/// new list of Word's own; a paragraph of the default style gets List Paragraph.
fn add(sink: &mut EditSink<'_, '_>, place: &Place, kind: &str, level: i64) -> Result<BTreeSet<i64>> {
    let docx = sink.docx()?;
    let joined = previous(&docx, place)
        .and_then(|p| read::list_at(&docx, &p))
        .filter(|m| {
            (if m.format.as_deref() == Some("bullet") {
                "bullet"
            } else {
                "number"
            }) == kind
        })
        .map(|m| m.num_id);
    let target = match joined {
        Some(n) => n,
        None => nb::new_list(sink, &docx.main, kind)?,
    };
    let docx = sink.docx()?;
    let read::At { element, .. } = read::at(&docx, place).ok_or_else(|| EditError::no_element(place.part.clone()))?;
    let current = read::paragraph_style_id(&element);
    let style = if current.is_none() || current == default_style(&docx) {
        Some(crate::builtin::ensure(sink, "List Paragraph", "paragraph")?)
    } else {
        None
    };
    rewrite(sink, place, |work, p, orders| {
        pp::set_numbering(work, p, (target, level), orders);
        if let Some(style) = &style {
            crate::paragraph_style::write_style(work, p, Some(style), orders);
        }
    })?;
    Ok(BTreeSet::new())
}

/// docx-agent's `remove_from_list` change: its own numbering taken away, List Paragraph
/// back to the default style, and `numId` 0 where its style still numbers it.
fn remove(sink: &mut EditSink<'_, '_>, place: &Place) -> Result<BTreeSet<i64>> {
    let docx = sink.docx()?;
    let lists = Lists::read(&docx);
    let list_paragraph = docx
        .find_style("List Paragraph", "paragraph")
        .and_then(|s| s.attribute(Some(W), "styleId"))
        .map(str::to_owned);
    rewrite(sink, place, |work, p, orders| {
        pp::clear_numbering(work, p);
        if let Some(current) = pp::style_of(work, p) {
            if list_paragraph.as_deref() == Some(current.as_str()) {
                pp::remove_style(work, p);
            }
        }
        let now = work.build();
        if now.is_some_and(|e| membership(&docx, &lists, &e).is_some()) {
            pp::set_numbering(work, p, (0, 0), orders);
        }
    })?;
    Ok(BTreeSet::new())
}

/// Whether a paragraph's style gives it a list, found once per style: a paragraph with no
/// `w:numPr` of its own whose style gives none is in no list, without reading the lists.
#[derive(Default)]
struct StyleNumbers(std::collections::HashMap<Option<String>, bool>);

impl StyleNumbers {
    fn may_list(&mut self, docx: &Docx, paragraph: &ooxml_xml::Element) -> bool {
        if ooxml_word_model::docx::find_path(paragraph, &["pPr", "numPr"]).is_some() {
            return true;
        }
        let style = read::paragraph_style_id(paragraph);
        *self.0.entry(style.clone()).or_insert_with(|| {
            ooxml_word_model::docx::numbering::style_numbering(docx, style.as_deref())
                .0
                .is_some()
        })
    }
}

/// The paragraphs from `place` on in its story that are in instance `num_id`.
fn following(docx: &Docx, place: &Place, num_id: i64) -> Vec<Place> {
    let Some(story) = docx.part(&place.part) else {
        return Vec::new();
    };
    let lists = Lists::read(docx);
    let mut styles = StyleNumbers::default();
    story
        .ids
        .paragraphs()
        .iter()
        .skip(place.index)
        .filter(|p| styles.may_list(docx, &p.element))
        .filter(|p| membership(docx, &lists, &p.element).is_some_and(|m| m.num_id == num_id))
        .map(|p| Place {
            part: place.part.clone(),
            index: p.index,
        })
        .collect()
}

/// docx-agent's `restart_numbering` change.
fn restart(
    sink: &mut EditSink<'_, '_>,
    place: &Place,
    list: &Membership,
    abstract_id: i64,
    at: i64,
) -> Result<BTreeSet<i64>> {
    let docx = sink.docx()?;
    let main = docx.main.clone();
    let members = if list.from_style {
        vec![place.clone()]
    } else {
        following(&docx, place, list.num_id)
    };
    let num_id = if list.from_style {
        let read::At { element, .. } =
            read::at(&docx, place).ok_or_else(|| EditError::no_element(place.part.clone()))?;
        let style_id = read::paragraph_style_id(&element);
        nb::add_restart(
            sink,
            &main,
            (abstract_id, list.num_id, list.level),
            at,
            style_id.as_deref(),
        )?
    } else {
        nb::add_num(sink, &main, abstract_id, &[(list.level, at)])?
    };
    for member in &members {
        let docx = sink.docx()?;
        let level = read::list_at(&docx, member).map_or(0, |m| m.level);
        rewrite(sink, member, |work, p, orders| {
            pp::set_numbering(work, p, (num_id, level), orders)
        })?;
    }
    Ok(BTreeSet::new())
}

/// The note a restart of a style-numbered item carries where its new level's indent is
/// not its style's.
fn restart_note(sink: &EditSink<'_, '_>, place: &Place, id: &str) -> Result<Option<String>> {
    let docx = sink.docx()?;
    let Some(moved) = read::list_at(&docx, place) else {
        return Ok(None);
    };
    let read::At { element, .. } = read::at(&docx, place).ok_or_else(|| EditError::no_element(place.part.clone()))?;
    let style_ind = read::style_indent(&docx, read::paragraph_style_id(&element).as_deref());
    Ok(read::indent_note(&docx, (moved.num_id, moved.level), style_ind.as_deref()).map(|n| format!("{id}: {n}")))
}

/// The instance `continue` joins (docx-agent's `continue_numbering` before its change).
fn continued(docx: &Docx, place: &Place, id: &str, list: &Membership, from: Option<&str>) -> Result<i64> {
    let lists = Lists::read(docx);
    let read::At { element, .. } = read::at(docx, place).ok_or_else(|| EditError::no_element(place.part.clone()))?;
    let own_style = if list.from_style {
        None
    } else {
        read::style_list(docx, read::paragraph_style_id(&element).as_deref())
    };
    let style_abstract = own_style.and_then(|(n, _)| lists.canonical(lists.abstract_of(n)));
    if let Some(from) = from {
        let place = super::paragraphs_of_place(docx, from)?;
        return read::list_at(docx, &place)
            .map(|m| m.num_id)
            .ok_or_else(|| refused(format!("{from} is not in a list")));
    }
    let story = docx
        .part(&place.part)
        .ok_or_else(|| EditError::no_part(place.part.clone()))?;
    let earlier = story.ids.paragraphs().iter().take(place.index).collect::<Vec<_>>();
    let mut styles = StyleNumbers::default();
    for other in earlier.into_iter().rev() {
        if !styles.may_list(docx, &other.element) {
            continue;
        }
        let Some(theirs) = membership(docx, &lists, &other.element) else {
            continue;
        };
        if theirs.num_id == list.num_id {
            continue;
        }
        if theirs.abstract_id == list.abstract_id
            || (style_abstract.is_some() && lists.canonical(theirs.abstract_id) == style_abstract)
        {
            return Ok(theirs.num_id);
        }
    }
    if own_style.is_some() {
        let style_id = read::paragraph_style_id(&element).or_else(|| default_style(docx));
        let name = style_id.as_deref().and_then(|s| {
            docx.styles()
                .iter()
                .find(|n| n.attribute(Some(W), "styleId") == Some(s))
                .map(|n| {
                    ooxml_word_model::docx::style_name(n)
                        .filter(|v| !v.is_empty())
                        .unwrap_or(s)
                        .to_owned()
                })
        });
        let name = name.or(style_id).unwrap_or_default();
        return Err(refused(format!(
            "{id} has direct numbering (list {}) while its style {} numbers itself, and no earlier paragraph is in a list over either definition: nothing to continue",
            list.num_id,
            ooxml_base::pyfmt::repr_str(&name)
        )));
    }
    Err(refused(format!("no earlier list over the same definition as {id}'s")))
}

/// docx-agent's `continue_numbering` change: each item of the list from here on joins
/// `target`, or takes its style's numbering where that is the target's definition at its
/// level.
fn continue_list(sink: &mut EditSink<'_, '_>, place: &Place, list: &Membership, target: i64) -> Result<BTreeSet<i64>> {
    let docx = sink.docx()?;
    let members = following(&docx, place, list.num_id);
    for member in &members {
        let docx = sink.docx()?;
        let lists = Lists::read(&docx);
        let target_abstract = lists.canonical(lists.abstract_of(target));
        let read::At { element, .. } =
            read::at(&docx, member).ok_or_else(|| EditError::no_element(member.part.clone()))?;
        let level = read::list_at(&docx, member).map_or(0, |m| m.level);
        let by_style = read::style_list(&docx, read::paragraph_style_id(&element).as_deref());
        let take_style =
            by_style.is_some_and(|(n, l)| l == level && lists.canonical(lists.abstract_of(n)) == target_abstract);
        rewrite(sink, member, |work, p, orders| {
            if take_style {
                pp::clear_numbering(work, p);
            } else {
                pp::set_numbering(work, p, (target, level), orders);
            }
        })?;
    }
    Ok(BTreeSet::new())
}

/// docx-agent's `set_list_format` change: a definition of its own first when another
/// instance shares it (copy-on-write), then the level's fields.
fn set_format(
    sink: &mut EditSink<'_, '_>,
    list: &Membership,
    (abstract_id, level): (i64, i64),
    format: &LevelFormat,
) -> Result<BTreeSet<i64>> {
    let docx = sink.docx()?;
    let main = docx.main.clone();
    let part = nb::part_of(sink, &main)?.ok_or_else(|| EditError::no_part("word/numbering.xml".to_owned()))?;
    let lists = Lists::read(&docx);
    let root = read::numbering_root(&docx).ok_or_else(|| EditError::no_part(part.clone()))?;
    let sharing = nb::listed(&root, "num", "numId")
        .iter()
        .any(|(n, _)| *n != list.num_id && lists.abstract_of(*n) == Some(abstract_id));
    let mut abstract_id = abstract_id;
    if sharing {
        abstract_id = nb::copy_abstract(sink, &main, &part, abstract_id)?;
        nb::set_abstract_of(sink, &part, list.num_id, abstract_id)?;
    }
    let shared = sink.doc().shared_handle();
    let changed = nb::edit_level(sink, &part, (abstract_id, level), |work, lvl| {
        pp::set_level(work, lvl, format, shared.facts.orders());
    })?;
    if !changed {
        return Err(refused(format!("the list's definition has no level {level}")));
    }
    Ok(BTreeSet::new())
}
