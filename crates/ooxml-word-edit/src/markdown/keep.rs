//! Owns: what a replacement keeps of the paragraphs it replaces (docx-agent's
//! `markdown/keep.py`, round 42): `insert_markdown` at `replace:p:A..p:B` gives the new
//! paragraphs the replaced ones' paragraph settings, as rewriting their text in place would
//! have kept them.
//!
//! - **Matching.** Each replaced paragraph is a donor of one kind: a heading of its outline
//!   level, a list item of its list's kind (bullet or numbered) at its level, or a
//!   paragraph. Each new body paragraph is of the kind Markdown says, and takes the donor of
//!   its kind at its position among them; past the last donor of its kind, that last one's,
//!   but not its page break before (one page break, not one per paragraph). A list's nesting
//!   counts from the replaced items' top level. Empty donors are passed over while a
//!   non-empty one of their kind is there. A new paragraph with no donor of its kind is
//!   written as Markdown says.
//! - **What is kept.** The donor's paragraph properties as they are (style, list membership,
//!   indents, spacing, alignment, keeps, page break, widow control, tabs, borders, shading,
//!   the mark's formatting), never its section break or its revision records.
//! - **What Markdown says wins.** A numbered list starting past 1, or a construct whose style
//!   the style map names, keeps Markdown's style and numbering: only the donor's keeps, page
//!   break and widow control are taken. A list item nested deeper than any donor of its kind
//!   takes the last one with a list of its own at the new level, its indent then the
//!   level's.
//!
//! Must not: write but on the work it is given; read the replaced paragraphs but before
//! anything changes.

use crate::marks::insert_in_order;
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use ooxml_word_model::docx::numbering::{membership, Lists};
use ooxml_word_model::docx::outline::OutlineLevels;
use ooxml_word_model::docx::text::paragraph_text;
use ooxml_word_model::docx::{style_name, Docx};
use ooxml_word_model::ids::Entry;
use ooxml_word_model::markdown::stylemap::WriteMap;
use ooxml_word_model::view::View;
use ooxml_xml::Element;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// A body paragraph's kind, as Markdown says it and a replaced paragraph is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Paragraph,
    Heading,
    Bullet,
    Number,
}

impl Kind {
    fn listed(self) -> bool {
        matches!(self, Kind::Bullet | Kind::Number)
    }

    /// The style map's construct for it.
    fn construct(self) -> &'static str {
        match self {
            Kind::Paragraph => "paragraph",
            Kind::Heading => "heading",
            Kind::Bullet => "bullet_list",
            Kind::Number => "ordered_list",
        }
    }
}

/// A paragraph the writer made: its node, kind and level (a list's nesting from 0, a
/// heading's level), and whether its list restarts (a start other than 1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tagged {
    pub(crate) node: Id,
    pub(crate) kind: Kind,
    pub(crate) level: i64,
    pub(crate) restarts: bool,
}

/// A replaced paragraph, read before anything changes.
#[derive(Debug, Clone)]
pub(crate) struct Donor {
    id: String,
    key: (Kind, i64),
    properties: Option<Arc<Element>>,
    /// Its list comes from its own `w:numPr` (not its style's).
    direct_list: bool,
    empty: bool,
}

/// What one new paragraph kept: its node, the donor's id, the settings by name.
#[derive(Debug, Clone)]
pub(crate) struct Kept {
    pub(crate) node: Id,
    pub(crate) from: String,
    pub(crate) settings: Vec<String>,
}

/// What is kept beside a style or numbering Markdown names.
const PAGE_SETTINGS: [&str; 4] = ["keepNext", "keepLines", "pageBreakBefore", "widowControl"];
/// Never kept: the section a paragraph ends, and its revision record.
const DROPPED: [&str; 2] = ["sectPr", "pPrChange"];
/// The mark's revision records, never kept.
const MARK_REVISIONS: [&str; 5] = ["ins", "del", "moveFrom", "moveTo", "rPrChange"];

/// The name a result says a setting by (word_format's where it has one).
fn name_of(local: &str) -> &str {
    match local {
        "keepNext" => "keep_with_next",
        "keepLines" => "keep_lines",
        "pageBreakBefore" => "page_break_before",
        "widowControl" => "widow_control",
        "numPr" => "list",
        "ind" => "indent",
        "spacing" => "spacing",
        "jc" => "alignment",
        "tabs" => "tabs",
        "pBdr" => "borders",
        "shd" => "shading",
        "rPr" => "mark_formatting",
        "outlineLvl" => "outline_level",
        "contextualSpacing" => "contextual_spacing",
        "suppressAutoHyphens" => "no_hyphenation",
        "framePr" => "frame",
        other => other,
    }
}

/// The paragraphs among the blocks `replaced` of `part` (tables and content controls pass),
/// as donors (`_donors`).
pub(crate) fn donors(docx: &Docx, part: &str, replaced: &[String]) -> Vec<Donor> {
    let Some(story) = docx.part(part) else {
        return Vec::new();
    };
    let outline = OutlineLevels::new(docx);
    let lists = Lists::read(docx);
    let mut out = Vec::new();
    for id in replaced {
        let Some(Entry::Paragraph(k)) = story.ids.get(id) else {
            continue;
        };
        let Some(entry) = story.ids.paragraphs().get(k) else {
            continue;
        };
        let element = &entry.element;
        let member = membership(docx, &lists, element);
        let key = match (outline.effective(element), &member) {
            (Some(level), _) => (Kind::Heading, level.min(6)),
            (None, Some(member)) => {
                let kind = if member.format.as_deref() == Some("bullet") {
                    Kind::Bullet
                } else {
                    Kind::Number
                };
                (kind, member.level)
            }
            (None, None) => (Kind::Paragraph, 0),
        };
        out.push(Donor {
            id: entry.id.clone(),
            key,
            properties: element.find(Some(W), "pPr").map(Arc::clone),
            direct_list: member.as_ref().is_some_and(|m| !m.from_style),
            empty: paragraph_text(element, View::Final).trim().is_empty(),
        });
    }
    let full: HashSet<(Kind, i64)> = out.iter().filter(|d| !d.empty).map(|d| d.key).collect();
    out.into_iter().filter(|d| !d.empty || !full.contains(&d.key)).collect()
}

/// Give the writer's `tagged` paragraphs (in `work`) their donors' settings
/// (`keep_replaced`): per paragraph that kept any, what it kept; and the list instances the
/// new paragraphs stopped naming.
pub(crate) fn keep_replaced(
    work: &mut Work,
    tagged: &[Tagged],
    donors: &[Donor],
    (map, docx, orders): (&WriteMap, &Docx, &Orders),
) -> (Vec<Kept>, BTreeSet<i64>) {
    let mut groups: HashMap<(Kind, i64), Vec<&Donor>> = HashMap::new();
    for donor in donors {
        groups.entry(donor.key).or_default().push(donor);
    }
    let base = donors
        .iter()
        .filter(|d| d.key.0.listed())
        .map(|d| d.key.1)
        .min()
        .unwrap_or(0);
    let mut seen: HashMap<(Kind, i64), usize> = HashMap::new();
    let mut used: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    let mut dropped = BTreeSet::new();
    for paragraph in tagged {
        let mapped = if paragraph.kind.listed() {
            (paragraph.kind, base + paragraph.level)
        } else {
            (paragraph.kind, paragraph.level)
        };
        let index = seen.entry(mapped).or_insert(0);
        let pool = groups.get(&mapped).map(Vec::as_slice).unwrap_or_default();
        let mut donor = pool.get((*index).min(pool.len().saturating_sub(1))).copied();
        *index += 1;
        let mut relevel = false;
        if donor.is_none() && paragraph.kind.listed() {
            donor = donors.iter().rev().find(|d| d.key.0 == paragraph.kind && d.direct_list);
            relevel = donor.is_some();
        }
        let Some(donor) = donor else { continue };
        let at = match paragraph.kind {
            Kind::Bullet | Kind::Number => paragraph.level + 1,
            Kind::Heading => paragraph.level,
            Kind::Paragraph => 0,
        };
        let named = !map.is_default(paragraph.kind.construct(), at);
        let before = num_ids(work, paragraph.node);
        let how = Apply {
            level: mapped.1,
            page_only: named || paragraph.restarts,
            relevel,
            // a donor taken again (more new paragraphs than old) gives no second page break
            again: used.contains(donor.id.as_str()),
        };
        let settings = apply(work, paragraph.node, donor, how, (docx, orders));
        used.insert(donor.id.as_str());
        let after = num_ids(work, paragraph.node);
        dropped.extend(before.difference(&after));
        if !settings.is_empty() {
            out.push(Kept {
                node: paragraph.node,
                from: donor.id.clone(),
                settings,
            });
        }
    }
    (out, dropped)
}

/// How a donor's settings are applied.
#[derive(Debug, Clone, Copy)]
struct Apply {
    level: i64,
    page_only: bool,
    relevel: bool,
    again: bool,
}

/// The list instance a paragraph's own `w:numPr` names, if any.
fn num_ids(work: &Work, paragraph: Id) -> BTreeSet<i64> {
    let found = work
        .find(paragraph, W, "pPr")
        .and_then(|p| work.find(p, W, "numPr"))
        .and_then(|n| work.find(n, W, "numId"))
        .and_then(|n| work.get(n, Some(W), "val"))
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse().ok());
    found.into_iter().collect()
}

/// The children of `parent` named `local`.
fn children_named(work: &Work, parent: Id, local: &str) -> Vec<Id> {
    work.children(parent)
        .iter()
        .copied()
        .filter(|c| work.is(*c, W, local))
        .collect()
}

/// `_apply`: the donor's settings given to `paragraph`; the settings it took, by name.
fn apply(work: &mut Work, paragraph: Id, donor: &Donor, how: Apply, (docx, orders): (&Docx, &Orders)) -> Vec<String> {
    let source = donor.properties.as_ref().and_then(|p| work.adopt(p));
    if let (Some(source), true) = (source, how.again) {
        for node in children_named(work, source, "pageBreakBefore") {
            work.remove(node);
        }
    }
    if how.page_only {
        return source.map_or_else(Vec::new, |source| page_settings(work, paragraph, source, orders));
    }
    let fresh = source.unwrap_or_else(|| work.new_element(work.element_name(paragraph, W, "pPr", "w")));
    clean(work, fresh, how);
    match work.find(paragraph, W, "pPr") {
        Some(target) => {
            work.add_previous(target, fresh);
            work.remove(target);
        }
        None => work.insert(paragraph, 0, fresh),
    }
    if work.children(fresh).iter().all(|c| !work.is_element(*c)) {
        work.remove(fresh);
        return Vec::new();
    }
    named(work, fresh, donor, docx)
}

/// The donor's page settings (`source`, its properties' copy) put in `paragraph`'s own, each
/// replacing the paragraph's; their names.
fn page_settings(work: &mut Work, paragraph: Id, source: Id, orders: &Orders) -> Vec<String> {
    let mut names = Vec::new();
    let mut target = work.find(paragraph, W, "pPr");
    for local in PAGE_SETTINGS {
        let Some(node) = work.find(source, W, local) else {
            continue;
        };
        let properties = *target.get_or_insert_with(|| {
            let made = work.new_element(work.element_name(paragraph, W, "pPr", "w"));
            work.insert(paragraph, 0, made);
            made
        });
        if let Some(old) = work.find(properties, W, local) {
            work.remove(old);
        }
        work.remove(node);
        insert_in_order(work, properties, node, orders);
        names.push(name_of(local).to_owned());
    }
    names
}

/// The donor's properties (`fresh`, a copy) without what is never kept -- its section break,
/// its change record, its mark's revisions -- and, re-levelled, at `how.level` without its own
/// indent.
fn clean(work: &mut Work, fresh: Id, how: Apply) {
    for local in DROPPED {
        for node in children_named(work, fresh, local) {
            work.remove(node);
        }
    }
    if let Some(mark) = work.find(fresh, W, "rPr") {
        for local in MARK_REVISIONS {
            for node in children_named(work, mark, local) {
                work.remove(node);
            }
        }
        if work.children(mark).is_empty() {
            work.remove(mark);
        }
    }
    if how.relevel {
        if let Some(numbering) = work.find(fresh, W, "numPr") {
            let ilvl = work.find(numbering, W, "ilvl").unwrap_or_else(|| {
                let made = work.new_element(work.element_name(numbering, W, "ilvl", "w"));
                work.insert(numbering, 0, made);
                made
            });
            work.set(ilvl, work.qname(W, "val", "w"), &how.level.to_string());
        }
        for node in children_named(work, fresh, "ind") {
            work.remove(node);
        }
    }
}

/// The settings `properties` (a paragraph's, kept whole from `donor`) hold, by name.
fn named(work: &Work, properties: Id, donor: &Donor, docx: &Docx) -> Vec<String> {
    let mut names = Vec::new();
    for &child in work.children(properties) {
        let Some(name) = work.name(child) else { continue };
        if name.namespace() != Some(W) {
            names.push(name.local().to_owned());
            continue;
        }
        match name.local() {
            "pStyle" => {
                let id = work.get(child, Some(W), "val").unwrap_or_default();
                let shown = docx.style(id).and_then(|s| style_name(s)).unwrap_or(id);
                names.push(format!("style {shown}"));
            }
            "numPr" => {
                let level = work
                    .find(child, W, "ilvl")
                    .and_then(|l| work.get(l, Some(W), "val"))
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0);
                names.push(format!("list level {level}"));
            }
            local => names.push(name_of(local).to_owned()),
        }
    }
    if donor.key.0.listed() && !names.iter().any(|n| n.starts_with("list")) {
        // its style's numbering
        names.push(format!("list level {}", donor.key.1));
    }
    names
}
