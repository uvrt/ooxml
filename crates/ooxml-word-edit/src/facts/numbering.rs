//! Owns: `check`'s `numbering_skips_level` (docx-agent's `layout_facts.numbering_skips_level`,
//! round 29 W12): paragraphs numbered at a list level whose label shows a level above it that
//! no paragraph is numbered at -- a section style's `1.1` with no chapter before it -- per
//! list definition, level and paragraph style, with the number Word shows where every level
//! above is unused.
//!
//! Must not: count list items or draw labels (Word counts a level never used from its
//! start: the label is worked out from the levels' starts alone).

use super::{style_name, style_names};
use crate::lists::style_list_of;
use crate::names::W;
use crate::{EditError, Result, WordDoc};
use ooxml_word_model::docx::block_facts::BlockFact;
use ooxml_word_model::docx::formatting::Formatting;
use ooxml_word_model::docx::numbering::{membership, style_numbering, Lists, Membership};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::Element;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// Paragraphs numbered at a level whose label shows a level nothing uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedLevel {
    /// The first one's id.
    pub para: String,
    /// Their paragraph style's name (the default paragraph style's where none is named).
    pub style: Option<String>,
    /// The level, from 0.
    pub level: i64,
    /// The `w:numId` the first one is numbered with.
    pub list: i64,
    /// The levels above it the label shows that no paragraph of the definition's lists is
    /// numbered at, from 0.
    pub unused_levels: Vec<i64>,
    /// The level's label (`w:lvlText`).
    pub label: Option<String>,
    /// The first one's number, where every level above it the label shows is unused and
    /// every level shown is decimal.
    pub shows: Option<String>,
    /// How many paragraphs.
    pub paragraphs: usize,
}

/// `check`'s `numbering_skips_level` of `doc`'s main story (tables too), in document order.
/// A paragraph whose style's level is tied to another style is numbered by none (Word draws
/// no number) and is left out.
///
/// # Errors
/// When the version cannot be read.
pub fn numbering_skips_level(doc: &WordDoc) -> Result<Vec<SkippedLevel>> {
    let docx = doc.docx()?;
    let lists = Lists::read(&docx);
    let body = docx.body().ok_or_else(|| EditError::no_part(docx.main.clone()))?;
    let names = style_names(&docx);
    let default = docx.default_style("paragraph").map(|s| style_name(s));
    let mut used: HashMap<*const Element, HashSet<i64>> = HashMap::new();
    let mut groups: Vec<(*const Element, SkippedLevel)> = Vec::new();
    let mut place: HashMap<(*const Element, i64, Option<String>), usize> = HashMap::new();
    // Whether a style's chain numbers its paragraphs, once per style: a paragraph with no
    // `w:numId` of its own under a style that numbers nothing is in no list (most of a long
    // document), without asking `membership`.
    let mut style_numbers: HashMap<Option<String>, bool> = HashMap::new();
    let mut style_lists: HashMap<Option<String>, bool> = HashMap::new();
    // Each instance's definition, once per `w:numId`.
    let mut definitions: HashMap<i64, Option<*const Element>> = HashMap::new();
    let formatting = Formatting::read(&docx);
    // Each paragraph found by its element; its id named only for a group's first.
    for (ordinal, element, at) in body.ids.paragraphs().placed() {
        let properties = element.find(Some(W), "pPr");
        let style_id = properties
            .and_then(|p| p.find(Some(W), "pStyle"))
            .and_then(|s| s.attribute(Some(W), "val"));
        let direct = properties
            .and_then(|p| p.find(Some(W), "numPr"))
            .is_some_and(|n| n.find(Some(W), "numId").is_some());
        if !direct {
            let numbers = *style_numbers
                .entry(style_id.map(str::to_owned))
                .or_insert_with(|| style_numbering(&docx, style_id).0.is_some_and(|n| n != 0));
            if !numbers {
                continue;
            }
        }
        // Its list, as the block tier keeps it by the paragraph.
        let found = body.block_fact::<Member>(&formatting, element, &at, || {
            membership(&docx, &lists, element).map(Arc::new)
        });
        let Some(found) = found else {
            continue;
        };
        let mut tied = || {
            *style_lists
                .entry(style_id.map(str::to_owned))
                .or_insert_with(|| style_list_of(&docx, style_id).is_none())
        };
        if found.from_style && tied() {
            continue;
        }
        let definition = *definitions
            .entry(found.num_id)
            .or_insert_with(|| lists.definition_of(&docx, found.num_id).map(Arc::as_ptr));
        let Some(definition) = definition else {
            continue;
        };
        used.entry(definition).or_default().insert(found.level);
        let name = match style_id.filter(|s| !s.is_empty()) {
            Some(id) => Some(names.get(id).cloned().unwrap_or_else(|| id.to_owned())),
            None => default.clone().flatten(),
        };
        let key = (definition, found.level, name.clone());
        if let Some(group) = place.get(&key).and_then(|k| groups.get_mut(*k)) {
            group.1.paragraphs += 1;
            continue;
        }
        place.insert(key, groups.len());
        groups.push((
            definition,
            SkippedLevel {
                para: body.ids.paragraphs().get(ordinal).map(|p| p.id).unwrap_or_default(),
                style: name,
                level: found.level,
                list: found.num_id,
                unused_levels: Vec::new(),
                label: found.text.clone(),
                shows: None,
                paragraphs: 1,
            },
        ));
    }
    Ok(groups
        .into_iter()
        .filter_map(|(definition, group)| skipping(&docx, &lists, group, used.get(&definition)))
        .collect())
}

/// The list a paragraph is in (`membership`), as the block tier keeps it ([`BlockFact`]): read
/// from the paragraph, the styles and the numbering alone.
struct Member;

impl BlockFact for Member {
    type Value = Option<Arc<Membership>>;

    fn weight(value: &Self::Value) -> usize {
        std::mem::size_of::<Self::Value>()
            + value.as_ref().map_or(0, |m| {
                std::mem::size_of::<Membership>()
                    + m.format.as_ref().map_or(0, String::len)
                    + m.text.as_ref().map_or(0, String::len)
            })
    }
}

/// `group` with the levels above it its label shows that no paragraph of its definition
/// (`levels`, those used) is numbered at, and the number it shows where every level above
/// is unused; `None` where it skips none.
fn skipping(
    docx: &Docx,
    lists: &Lists,
    mut group: SkippedLevel,
    levels: Option<&HashSet<i64>>,
) -> Option<SkippedLevel> {
    let shown = shown_levels(group.label.as_deref().unwrap_or(""));
    let unused: Vec<i64> = shown
        .iter()
        .copied()
        .filter(|k| *k < group.level && !levels.is_some_and(|l| l.contains(k)))
        .collect();
    if unused.is_empty() {
        return None;
    }
    // Where every level above is unused, each shows its start (measured: 1.1, 1.2).
    if shown.iter().filter(|k| **k < group.level).all(|k| unused.contains(k)) {
        group.shows = first_label(docx, lists, group.list, group.label.as_deref().unwrap_or(""), &shown);
    }
    group.unused_levels = unused;
    Some(group)
}

/// Each style's name by its id (its id where it has no name; the last of an id).
/// The levels a label shows (`%1` is level 0's), from 0, in order.
fn shown_levels(label: &str) -> Vec<i64> {
    let mut out = BTreeSet::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            if let Some(digit) = chars
                .peek()
                .and_then(|d| d.to_digit(10))
                .filter(|d| (1..=9).contains(d))
            {
                out.insert(i64::from(digit) - 1);
                chars.next();
            }
        }
    }
    out.into_iter().collect()
}

/// A label as its level's first paragraph shows it with the levels above it never used:
/// each level shown at its start (its instance's `w:startOverride`, else its `w:start`, else
/// 0); `None` unless every level shown is decimal (docx-agent's `_first_label`).
fn first_label(docx: &Docx, lists: &Lists, num_id: i64, label: &str, shown: &[i64]) -> Option<String> {
    let num = lists.num_element(num_id);
    let mut starts: HashMap<i64, String> = HashMap::new();
    for level in shown {
        let lvl = lists.level(docx, num_id, *level)?;
        let format = match lvl.find(Some(W), "numFmt") {
            Some(f) => f.attribute(Some(W), "val"),
            None => Some("decimal"),
        };
        if format != Some("decimal") {
            return None;
        }
        let mut start = lvl
            .find(Some(W), "start")
            .and_then(|s| s.attribute(Some(W), "val"))
            .and_then(py_int)
            .unwrap_or(0);
        for o in num
            .into_iter()
            .flat_map(|n| n.child_elements())
            .filter(|e| e.name().is(Some(W), "lvlOverride"))
        {
            let given = o.find(Some(W), "startOverride");
            if o.attribute(Some(W), "ilvl").and_then(py_int) == Some(*level) {
                if let Some(given) = given {
                    start = given.attribute(Some(W), "val").and_then(py_int).unwrap_or(0);
                }
            }
        }
        starts.insert(*level, start.to_string());
    }
    let mut out = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        let level = (c == '%')
            .then(|| {
                chars
                    .peek()
                    .and_then(|d| d.to_digit(10))
                    .filter(|d| (1..=9).contains(d))
            })
            .flatten();
        match level.and_then(|d| starts.get(&(i64::from(d) - 1))) {
            Some(start) => {
                out.push_str(start);
                chars.next();
            }
            None => out.push(c),
        }
    }
    Some(out)
}
