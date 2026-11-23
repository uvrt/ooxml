//! Owns: `smallest_text` and `small_text` (docx-agent's `text_sizes`, round 16 F8 and
//! round 17 P6b): the smallest size each kind of text is drawn at on the pages asked for,
//! and the text drawn below a minimum -- one number for every kind, or one per kind with a
//! caption's own -- per paragraph, kind and size, smallest first, with the size its styles
//! give the paragraph.
//!
//! Must not: count raised or lowered text (a note's reference mark is drawn small by
//! design); [`crate::line_text`] leaves it out.

use super::tree::resolve;
use super::{story_kind, Facts};
use ooxml_word_model::docx::cells::PartCells;
use ooxml_word_model::docx::StoryPart;
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// The kinds of text a minimum is set for (`TEXT_KINDS`).
pub const TEXT_KINDS: [&str; 7] = ["body", "table", "footnote", "endnote", "header", "footer", "caption"];

/// A minimum text size: one for every kind, or one per kind (the kinds it leaves out not
/// checked; `caption` a paragraph in the Caption style, wherever it is).
#[derive(Debug, Clone, PartialEq)]
pub enum Minimum {
    /// Points, for every kind.
    One(f64),
    /// Points by kind.
    PerKind(BTreeMap<String, f64>),
}

impl Minimum {
    /// The smallest of its points.
    fn least(&self) -> Option<f64> {
        match self {
            Minimum::One(points) => Some(*points),
            Minimum::PerKind(kinds) => kinds.values().copied().reduce(f64::min),
        }
    }
}

/// Text drawn below its minimum: one paragraph's, of one kind and size.
#[derive(Debug, Clone, PartialEq)]
pub struct SmallText {
    /// The paragraph's id.
    pub para: String,
    /// Its kind (`caption` where a caption's minimum applies).
    pub kind: String,
    /// The size, points.
    pub size: f64,
    /// Its characters that are not white space.
    pub chars: usize,
    /// The 1-based pages it is drawn on.
    pub pages: Vec<usize>,
    /// The minimum, where minimums are set per kind.
    pub min: Option<f64>,
    /// The size the paragraph's styles give it, where they resolve one.
    pub style_size: Option<f64>,
    /// Whether clearing the paragraph's direct size fixes it: its styles' size is at least
    /// the minimum.
    pub fixable: bool,
}

/// The text sizes of the pages asked for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextSizes {
    /// The smallest size drawn per kind (`body`, `table`, `footnote`, `endnote`, `header`,
    /// `footer`).
    pub smallest: BTreeMap<String, f64>,
    /// With a minimum: the text drawn below it, smallest first (most characters first
    /// among equal sizes).
    pub small: Option<Vec<SmallText>>,
}

/// A line's paragraph: its id, element and place in its part, and the part.
#[derive(Clone)]
struct Owner<'a> {
    id: String,
    element: &'a Arc<Element>,
    place: Vec<u32>,
    story: &'a StoryPart,
}

/// What one paragraph, kind and size has drawn below its minimum so far.
struct Seen {
    chars: usize,
    pages: BTreeSet<usize>,
    min: f64,
}

/// `smallest_text`, and `small_text` with a minimum.
#[must_use]
pub fn text_sizes(facts: &Facts<'_>, minimum: Option<&Minimum>) -> TextSizes {
    let least = minimum.and_then(Minimum::least);
    let mut out = TextSizes::default();
    let mut owners: HashMap<(String, String), Option<Owner<'_>>> = HashMap::new();
    let mut small: Vec<((String, String, i64), Seen, Owner<'_>)> = Vec::new();
    let mut at: HashMap<(String, String, i64), usize> = HashMap::new();
    let mut captions: HashMap<String, bool> = HashMap::new();
    for number in 1..=facts.page_count() {
        if !facts.shown([number]) {
            continue;
        }
        for line in facts.text_lines(number - 1) {
            let part = part_of(facts, &line.path, line.story.as_ref().map(|(_, p)| p.as_str()));
            let owner = owners
                .entry((part.clone(), line.path.clone()))
                .or_insert_with(|| owner(facts, &part, &line.path))
                .clone();
            if let Some(scope) = facts.scope() {
                // a range's facts: its paragraphs' text only
                if owner.as_ref().is_none_or(|o| !scope.paragraphs.contains(&o.id)) {
                    continue;
                }
            }
            let kind = kind_of(&line.path, line.story.as_ref().map(|(k, _)| *k));
            for &(half_points, chars) in &line.sizes {
                #[allow(clippy::cast_precision_loss)]
                let size = half_points as f64 / 2.0;
                let smallest = out.smallest.entry(kind.to_owned()).or_insert(10_000.0);
                if size < *smallest {
                    *smallest = size;
                }
                let (Some(least), Some(minimum), Some(owner)) = (least, minimum, owner.as_ref()) else {
                    continue;
                };
                if size >= least {
                    continue;
                }
                let caption = || {
                    *captions
                        .entry(owner.id.clone())
                        .or_insert_with(|| facts.is_caption(owner.element))
                };
                let (named, min) = minimum_of(minimum, kind, caption);
                let Some(min) = min.filter(|m| size < *m) else { continue };
                let key = (owner.id.clone(), named, half_points);
                match at.get(&key).and_then(|k| small.get_mut(*k)) {
                    Some((_, seen, _)) => {
                        seen.chars += chars;
                        seen.pages.insert(number);
                    }
                    None => {
                        at.insert(key.clone(), small.len());
                        small.push((
                            key,
                            Seen {
                                chars,
                                pages: BTreeSet::from([number]),
                                min,
                            },
                            owner.clone(),
                        ));
                    }
                }
            }
        }
    }
    out.smallest.retain(|_, v| *v < 10_000.0);
    if let Some(minimum) = minimum {
        out.small = Some(entries(facts, small, matches!(minimum, Minimum::PerKind(_))));
    }
    out
}

/// The entries of what was drawn small, smallest first, each with its styles' size.
fn entries(
    facts: &Facts<'_>,
    mut small: Vec<((String, String, i64), Seen, Owner<'_>)>,
    per_kind: bool,
) -> Vec<SmallText> {
    small.sort_by(|(a, x, _), (b, y, _)| a.2.cmp(&b.2).then(y.chars.cmp(&x.chars)));
    let mut styled: HashMap<String, Option<f64>> = HashMap::new();
    small
        .into_iter()
        .map(|((para, kind, half_points), seen, owner)| {
            let style_size = *styled.entry(para.clone()).or_insert_with(|| style_size(facts, &owner));
            #[allow(clippy::cast_precision_loss)]
            let size = half_points as f64 / 2.0;
            SmallText {
                para,
                kind,
                size,
                chars: seen.chars,
                pages: seen.pages.into_iter().collect(),
                min: per_kind.then_some(seen.min),
                style_size,
                fixable: style_size.is_some_and(|s| s >= seen.min),
            }
        })
        .collect()
}

/// `(kind, minimum)` of text of `kind`: its kind's, a caption's own where set
/// (`minimum`, `minimum_for`).
fn minimum_of(minimum: &Minimum, kind: &str, caption: impl FnOnce() -> bool) -> (String, Option<f64>) {
    let kinds = match minimum {
        Minimum::One(points) => return (kind.to_owned(), Some(*points)),
        Minimum::PerKind(kinds) => kinds,
    };
    if kinds.contains_key("caption") && caption() {
        return ("caption".to_owned(), kinds.get("caption").copied());
    }
    (kind.to_owned(), kinds.get(kind).copied())
}

/// The size a paragraph's styles give a run with no properties of its own
/// (`_style_size`).
fn style_size(facts: &Facts<'_>, owner: &Owner<'_>) -> Option<f64> {
    let mut cells = PartCells::of(facts.formatting(), owner.story);
    let cell = cells.cell(&owner.place);
    Some(facts.formatting().run(owner.element, None, 'a', cell.as_ref()).size)
}

/// The part a line's path is in: its story's part for a header's or footer's line, the
/// notes part for a note's, else the main part (`_part_of`).
fn part_of(facts: &Facts<'_>, path: &str, part: Option<&str>) -> String {
    if let Some(part) = part {
        return part.to_owned();
    }
    let docx = facts.docx();
    let story = match story_kind(path) {
        "footnotes" => docx.story("footnotes"),
        "endnotes" => docx.story("endnotes"),
        _ => None,
    };
    story
        .or_else(|| docx.body())
        .map(|s| s.part.clone())
        .unwrap_or_default()
}

/// The paragraph a line of `part` at `path` is of, where the part's ids name it.
fn owner<'a>(facts: &Facts<'a>, part: &str, path: &str) -> Option<Owner<'a>> {
    let story = facts.docx().part(part)?;
    let (element, place) = resolve(story, path)?;
    if !super::tree::is_w(element, "p") {
        return None;
    }
    let id = story.ids.paragraph_at(&place)?.id;
    Some(Owner {
        id,
        element,
        place,
        story,
    })
}

/// A line's kind of text (`_kind`): `table` for a body table's cell, the story's otherwise.
fn kind_of(path: &str, story: Option<&'static str>) -> &'static str {
    match story.unwrap_or_else(|| story_kind(path)) {
        "body" if path.contains("/w:tc[") => "table",
        "body" => "body",
        "footnotes" => "footnote",
        "endnotes" => "endnote",
        other => other,
    }
}
