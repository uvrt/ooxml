//! Owns: `smallest_text` and `small_text` (docx-agent's `text_sizes`, round 16 F8 and
//! round 17 P6b): the smallest size each kind of text is drawn at on the pages asked for,
//! and the text drawn below a minimum -- one number for every kind, or one per kind with a
//! caption's own -- per paragraph, kind and size, smallest first, with the size its styles
//! give the paragraph.
//!
//! Must not: count raised or lowered text (a note's reference mark is drawn small by
//! design); [`crate::line_text`] leaves it out.

use super::Facts;
use crate::line_text::ItemPlace;
use crate::page_parts::Part;
use crate::page_read::PageRead;
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
    /// The largest of its points: text at or above it is below no kind's minimum. (Round 60,
    /// R140: docx-agent 0.1 kept only the text below the smallest, so a kind with a higher
    /// minimum than another's was never checked against its own.)
    fn most(&self) -> Option<f64> {
        match self {
            Minimum::One(points) => Some(*points),
            Minimum::PerKind(kinds) => kinds.values().copied().reduce(f64::max),
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

/// One size of text one paragraph's lines draw on a page: the first of those lines (its
/// place among the page's lines, its text boxes' after its own), the text's kind, its size
/// (half points) and its characters that are not white space, every such line's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sized {
    /// The first line drawing it.
    pub line: usize,
    /// Its kind of text (before a caption's minimum is asked for).
    pub kind: &'static str,
    /// Its size, half points.
    pub half_points: i64,
    /// Its characters.
    pub chars: usize,
}

/// One page's contribution to the text sizes (the page tier's, [`crate::page_entry`]):
/// the smallest size of each kind it draws, and each size each paragraph's lines draw, in
/// the order first drawn. A pure function of the page's read: the minimum, the scope and
/// the paragraphs' ids are the fold's ([`text_sizes`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SizesPart {
    /// The smallest size (half points) of each kind, in the order the kinds are first drawn.
    pub smallest: Vec<(&'static str, i64)>,
    /// Each paragraph's sizes by kind, in the order first drawn.
    pub sized: Vec<Sized>,
}

impl Part<PageRead> for SizesPart {
    /// The contribution of a page read as `read`.
    fn of(read: &PageRead) -> SizesPart {
        let mut out = SizesPart::default();
        let mut at: HashMap<(&ItemPlace, Option<&str>, i64), usize> = HashMap::new();
        for (k, line) in read.texts.iter().chain(&read.box_texts).enumerate() {
            let kind = line.at.kind;
            let story = line.story.as_ref().map(|(_, part)| part.as_str());
            for &(half_points, chars) in &line.sizes {
                match out.smallest.iter_mut().find(|(named, _)| *named == kind) {
                    Some((_, least)) => *least = (*least).min(half_points),
                    None => out.smallest.push((kind, half_points)),
                }
                match at.get(&(&line.place, story, half_points)) {
                    Some(&found) => {
                        if let Some(sized) = out.sized.get_mut(found) {
                            sized.chars += chars;
                        }
                    }
                    None => {
                        at.insert((&line.place, story, half_points), out.sized.len());
                        out.sized.push(Sized {
                            line: k,
                            kind,
                            half_points,
                            chars,
                        });
                    }
                }
            }
        }
        out
    }

    /// What the contribution of a page read as `read` holds at most, bytes.
    fn weight_of(read: &PageRead) -> usize {
        let sizes: usize = read.texts.iter().chain(&read.box_texts).map(|l| l.sizes.len()).sum();
        std::mem::size_of::<SizesPart>() + sizes * std::mem::size_of::<Sized>()
    }
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

/// What [`text_sizes`] folds the pages' contributions into.
struct Fold<'f, 'a> {
    facts: &'f Facts<'a>,
    minimum: Option<&'f Minimum>,
    /// The largest minimum ([`Minimum::most`]): what text below any kind's minimum is below.
    most: Option<f64>,
    smallest: BTreeMap<String, f64>,
    /// Each line's paragraph, by its place (and its story's part, a header's or footer's),
    /// found once.
    owners: HashMap<(Option<String>, ItemPlace), Option<Owner<'a>>>,
    small: Vec<((String, String, i64), Seen, Owner<'a>)>,
    at: HashMap<(String, String, i64), usize>,
    captions: HashMap<String, bool>,
}

/// `smallest_text`, and `small_text` with a minimum: a fold over the pages' contributions
/// ([`SizesPart`]), in page order, a paragraph named only where its text is below the
/// minimum (or the facts are scoped to some paragraphs).
#[must_use]
pub fn text_sizes(facts: &Facts<'_>, minimum: Option<&Minimum>) -> TextSizes {
    let mut fold = Fold {
        facts,
        minimum,
        most: minimum.and_then(Minimum::most),
        smallest: BTreeMap::new(),
        owners: HashMap::new(),
        small: Vec::new(),
        at: HashMap::new(),
        captions: HashMap::new(),
    };
    for number in 1..=facts.page_count() {
        if !facts.shown([number]) {
            continue;
        }
        if let Some(entry) = facts.entry(number - 1) {
            fold.page(number, entry.read(), entry.sizes());
        }
    }
    let mut out = TextSizes {
        smallest: fold.smallest,
        small: None,
    };
    out.smallest.retain(|_, v| *v < 10_000.0);
    if let Some(minimum) = minimum {
        out.small = Some(entries(facts, fold.small, matches!(minimum, Minimum::PerKind(_))));
    }
    out
}

impl<'a> Fold<'_, 'a> {
    /// The 1-based page `number`'s contribution, its read `read`, folded in.
    fn page(&mut self, number: usize, read: &PageRead, part: &SizesPart) {
        if self.facts.scope().is_none() {
            for &(kind, half_points) in &part.smallest {
                self.smallest_is(kind, half_points);
            }
            let Some(most) = self.most else { return };
            for sized in part.sized.iter().filter(|s| size(s.half_points) < most) {
                if let Some(owner) = self.owner(read, sized) {
                    self.small(number, sized, &owner);
                }
            }
            return;
        }
        for sized in &part.sized {
            // a range's facts: its paragraphs' text only
            let Some(owner) = self.owner(read, sized) else { continue };
            if !self.facts.scope().is_some_and(|s| s.paragraphs.contains(&owner.id)) {
                continue;
            }
            self.smallest_is(sized.kind, sized.half_points);
            if self.most.is_some_and(|most| size(sized.half_points) < most) {
                self.small(number, sized, &owner);
            }
        }
    }

    fn smallest_is(&mut self, kind: &str, half_points: i64) {
        let smallest = self.smallest.entry(kind.to_owned()).or_insert(10_000.0);
        *smallest = smallest.min(size(half_points));
    }

    /// The paragraph `sized`'s line on a page read as `read` is of, found once.
    fn owner(&mut self, read: &PageRead, sized: &Sized) -> Option<Owner<'a>> {
        let line = read
            .texts
            .get(sized.line)
            .or_else(|| read.box_texts.get(sized.line.checked_sub(read.texts.len())?))?;
        let part = line.story.as_ref().map(|(_, p)| p.clone());
        let facts = self.facts;
        self.owners
            .entry((part, line.place.clone()))
            .or_insert_with_key(|(part, place)| owner(facts, part.as_deref(), place))
            .clone()
    }

    /// `sized`, of `owner`, on the 1-based page `number`, kept where below its minimum.
    fn small(&mut self, number: usize, sized: &Sized, owner: &Owner<'a>) {
        let Some(minimum) = self.minimum else { return };
        let facts = self.facts;
        let captions = &mut self.captions;
        let caption = || {
            *captions
                .entry(owner.id.clone())
                .or_insert_with(|| facts.is_caption(owner.element))
        };
        let (named, min) = minimum_of(minimum, sized.kind, caption);
        let Some(min) = min.filter(|m| size(sized.half_points) < *m) else {
            return;
        };
        let key = (owner.id.clone(), named, sized.half_points);
        match self.at.get(&key).and_then(|k| self.small.get_mut(*k)) {
            Some((_, seen, _)) => {
                seen.chars += sized.chars;
                seen.pages.insert(number);
            }
            None => {
                self.at.insert(key.clone(), self.small.len());
                self.small.push((
                    key,
                    Seen {
                        chars: sized.chars,
                        pages: BTreeSet::from([number]),
                        min,
                    },
                    owner.clone(),
                ));
            }
        }
    }
}

/// A size in points, from half points.
fn size(half_points: i64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let points = half_points as f64 / 2.0;
    points
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

/// The paragraph a line at `place` (of the story part `part`, a header's or footer's) is
/// of, where its part's ids name it.
fn owner<'a>(facts: &Facts<'a>, part: Option<&str>, place: &ItemPlace) -> Option<Owner<'a>> {
    let (story, element, place) = facts.element_of(place, part)?;
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
