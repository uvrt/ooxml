//! Owns: a page's paint written for the changes feed (phase 3 changes-feed design, section
//! 3): the page as groups, each one block on one layer -- a paragraph's lines with the rules
//! and pictures drawn for it (`p:<id>`), a table row's borders and shading under or over
//! the text (`<table id>/<row id>/under`, `/over`), a drawing with its text boxes'
//! content (`d:<paragraph id>/<run>.<index>`), a header's or footer's item with `@<part>`
//! after its id -- each named by its block's id through the version's names, written in
//! its own coordinates (moved up by its offset, drawn with one `translate(0,dy)`), with no
//! paths, its resources' ids minted in its own scope and its pictures by content address
//! ([`FeedPicture`]); and the page written whole from its groups ([`feed_svg`]), what a
//! client that applies the feed rebuilds byte for byte.
//!
//! The groups are made from the paint's items, never from written SVG, and stand in the
//! order the page's SVG draws their first item ([`super::svg::page_element`]'s layers).
//!
//! Must not: lay out or paint, read a picture part, or name a group by a path where its
//! block has an id.

use super::drawings::{DrawingSlot, Layer, SlotName};
use super::names::PageNames;
use super::svg::{
    float_content, line_element, paint_drawing, picture, rect, under_rank, FeedPicture, Ink, PageDefs, Painted,
    SvgContext,
};
use super::{Drawn, ItemName, PagePaint, PaintedLine, Rule};
use crate::px::Px;
use ooxml_scene::exact::{quote, write_lines, Element};
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, BlockKey, Entry, StoryIx, Sub};
use ooxml_word_model::value::Text;
use std::collections::HashMap;

/// A page's frame as the feed writes it: its size in points and device px.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedFrame {
    /// Its width, as the SVG writes it (`816pt`).
    pub width: String,
    /// Its height.
    pub height: String,
    /// Its width, device px.
    pub width_px: i64,
    /// Its height, device px.
    pub height_px: i64,
}

/// One group of a page as the feed writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedGroup {
    /// Its id: its block's, durable across versions.
    pub id: String,
    /// Its offset, device px: what its content is moved up by.
    pub dy: i64,
    /// Its content in its own coordinates: each element on a line of its own.
    pub content: String,
    /// Its resources (gradients, patterns, markers, filters), under ids of its own scope.
    pub defs: String,
    /// The pictures it draws, by content address.
    pub pictures: Vec<FeedPicture>,
}

/// A page as the feed writes it: its frame, and its groups in drawing order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedPage {
    /// Its frame.
    pub frame: FeedFrame,
    /// Its groups, in drawing order.
    pub groups: Vec<FeedGroup>,
}

/// The page as one SVG document, from its frame and its groups in drawing order: what the
/// feed's client writes from what it was sent.
pub fn feed_svg<'g>(frame: &FeedFrame, groups: impl IntoIterator<Item = &'g FeedGroup>) -> String {
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n<rect width=\"{}\" height=\"{}\" fill=\"#FFFFFF\"/>",
        frame.width, frame.height, frame.width_px, frame.height_px, frame.width_px, frame.height_px
    );
    for g in groups {
        out.push_str("\n<g data-feed-group=");
        out.push_str(&quote(&g.id));
        out.push_str(&format!(" transform=\"translate(0,{})\">", g.dy));
        if !g.defs.is_empty() {
            out.push_str("\n<defs>");
            out.push_str(&g.defs);
            out.push_str("</defs>");
        }
        out.push_str(&g.content);
        out.push_str("\n</g>");
    }
    out.push_str("\n</svg>\n");
    out
}

impl FeedPage {
    /// The page as one SVG document ([`feed_svg`]).
    #[must_use]
    pub fn svg(&self) -> String {
        feed_svg(&self.frame, &self.groups)
    }
}

/// What a group is of.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Of {
    /// A block (a paragraph's lines, rules and pictures).
    Block(BlockKey),
    /// A table's row, under the text or over it.
    Row(BlockKey, u16, bool),
    /// A drawing, by its place among the page's.
    Slot(usize),
    /// What no story's index holds, by its path and its story's part.
    Path(Text, Option<Text>),
}

/// One item a group draws.
enum Item<'p> {
    Line(&'p PaintedLine),
    Rule(&'p Rule),
    Picture(&'p Drawn),
    Slot(usize),
}

/// A group being gathered: what it is of, its offset and its items.
struct Gathered<'p> {
    of: Of,
    dy: i64,
    items: Vec<Item<'p>>,
}

/// The page's groups gathered in drawing order.
#[derive(Default)]
struct Groups<'p> {
    list: Vec<Gathered<'p>>,
    at: HashMap<Of, usize>,
}

impl<'p> Groups<'p> {
    /// Add `item` to the group of `of`, made with offset `dy` where it is the first.
    fn add(&mut self, of: Of, dy: i64, item: Item<'p>) {
        let k = *self.at.entry(of.clone()).or_insert_with(|| {
            self.list.push(Gathered {
                of,
                dy,
                items: Vec::new(),
            });
            self.list.len() - 1
        });
        if let Some(group) = self.list.get_mut(k) {
            group.items.push(item);
        }
    }

    fn lines(&mut self, lines: impl Iterator<Item = &'p PaintedLine>) {
        for line in lines {
            let of = match &line.name {
                ItemName::Anchored(anchor) => Of::Block(anchor.block),
                ItemName::Path(path) => Of::Path(path.clone(), line.story.as_ref().map(|(_, p)| p.clone())),
            };
            self.add(of, line.baseline, Item::Line(line));
        }
    }

    fn rules(&mut self, rules: impl Iterator<Item = &'p Rule>) {
        for rule in rules {
            let of = match &rule.name {
                ItemName::Anchored(Anchor {
                    block,
                    sub: Sub::Cell(row, _),
                }) => Of::Row(*block, *row, under_rank(rule.kind).is_none()),
                ItemName::Anchored(anchor) => Of::Block(anchor.block),
                ItemName::Path(path) => Of::Path(path.clone(), None),
            };
            self.add(of, rule.y.floor(), Item::Rule(rule));
        }
    }

    fn pictures(&mut self, pictures: &'p [Drawn]) {
        for d in pictures {
            let of = match &d.name {
                ItemName::Anchored(anchor) => Of::Block(anchor.block),
                ItemName::Path(path) => Of::Path(path.clone(), d.part.clone()),
            };
            self.add(of, d.y.floor(), Item::Picture(d));
        }
    }

    fn floats(&mut self, slots: &[(usize, &'p DrawingSlot)]) {
        for (k, slot) in slots {
            self.add(Of::Slot(*k), slot.y.floor(), Item::Slot(*k));
        }
    }
}

/// The page `paint` as the feed writes it.
pub fn feed_page(paint: &PagePaint, cx: &SvgContext<'_>) -> FeedPage {
    let frame = &paint.geometry.frame;
    let names = PageNames::of_page(cx.version, paint);
    let groups = gathered(paint);
    let mut ids = Ids::default();
    let groups = groups
        .list
        .iter()
        .map(|group| {
            let id = ids.unique(group_id(&group.of, paint, cx.version));
            written(group, id, paint, cx, &names)
        })
        .collect();
    FeedPage {
        frame: FeedFrame {
            width: format!("{}pt", super::svg::number(frame.width_pt)),
            height: format!("{}pt", super::svg::number(frame.height_pt)),
            width_px: frame.width_px,
            height_px: frame.height_px,
        },
        groups,
    }
}

/// The page's items gathered into groups, in the order the page's SVG draws them.
fn gathered(paint: &PagePaint) -> Groups<'_> {
    let mut slots: Vec<(usize, &DrawingSlot)> = paint.drawings.iter().enumerate().collect();
    slots.sort_by_key(|(_, s)| s.order);
    let layer = |l: Layer, story: bool| -> Vec<(usize, &DrawingSlot)> {
        slots
            .iter()
            .copied()
            .filter(|(_, s)| s.layer == l && s.story.is_some() == story)
            .collect()
    };
    let (story_behind, story_front) = (layer(Layer::Behind, true), layer(Layer::Front, true));
    let (body_behind, body_front) = (layer(Layer::Behind, false), layer(Layer::Front, false));
    let story_lines = paint.lines.iter().any(|l| l.story.is_some());
    let mut under: Vec<&Rule> = paint.rules.iter().filter(|r| under_rank(r.kind).is_some()).collect();
    under.sort_by_key(|r| under_rank(r.kind));
    let over = paint.rules.iter().filter(|r| under_rank(r.kind).is_none());
    let mut groups = Groups::default();
    if story_behind.is_empty() && story_front.is_empty() && !(story_lines && !body_behind.is_empty()) {
        groups.floats(&body_behind);
        groups.rules(under.into_iter());
        groups.lines(paint.lines.iter());
    } else {
        groups.floats(&story_behind);
        groups.lines(paint.lines.iter().filter(|l| l.story.is_some()));
        groups.floats(&story_front);
        groups.floats(&body_behind);
        groups.rules(under.into_iter());
        groups.lines(paint.lines.iter().filter(|l| l.story.is_none()));
    }
    groups.rules(over);
    groups.pictures(&paint.pictures);
    groups.floats(&body_front);
    groups
}

/// The ids given on a page: one given twice gets its count after it (`#2`).
#[derive(Default)]
struct Ids(HashMap<String, usize>);

impl Ids {
    fn unique(&mut self, id: String) -> String {
        let n = self.0.entry(id.clone()).or_insert(0);
        *n += 1;
        if *n == 1 {
            id
        } else {
            format!("{id}#{n}")
        }
    }
}

/// The id of the block `key` names, with its story's part where it is not the body's.
fn block_id(key: BlockKey, names: &VersionNames) -> String {
    let id = names.id(key).map_or_else(
        || format!("x:{}", names.path(Anchor::whole(key)).unwrap_or_default()),
        |id| id.to_string(),
    );
    with_part(id, key.story(), names)
}

fn with_part(id: String, story: StoryIx, names: &VersionNames) -> String {
    if story == StoryIx::BODY {
        return id;
    }
    match names.part(story) {
        Some(part) => format!("{id}@{}", part.part),
        None => id,
    }
}

/// The id of row `row` of the table `table` names.
fn row_id(table: BlockKey, row: u16, names: &VersionNames) -> Option<String> {
    let part = names.part(table.story())?;
    let Entry::Table(n) = part.ids.entry_of(table)? else {
        return None;
    };
    let key = part.ids.row_key_of(table.story(), n, usize::from(row))?;
    Some(part.ids.id_of_key(key)?.to_string())
}

/// A group's id: its block's, through the version's names.
fn group_id(of: &Of, paint: &PagePaint, names: &VersionNames) -> String {
    match of {
        Of::Block(key) => block_id(*key, names),
        Of::Row(table, row, over) => {
            let table_id = names.id(*table).map(|id| id.to_string()).unwrap_or_default();
            let row = row_id(*table, *row, names).unwrap_or_else(|| format!("r{row}"));
            let side = if *over { "over" } else { "under" };
            with_part(format!("{table_id}/{row}/{side}"), table.story(), names)
        }
        Of::Slot(k) => match paint.drawings.get(*k).map(|s| (&s.name, s)) {
            Some((SlotName::Anchored(anchor), slot)) => {
                let id = names.id(anchor.block).map(|id| id.to_string()).unwrap_or_default();
                with_part(
                    format!("d:{id}/{}.{}", slot.run, slot.index),
                    anchor.block.story(),
                    names,
                )
            }
            Some((SlotName::Path { path, .. }, slot)) => match &slot.story {
                Some((_, part)) => format!("d:{path}@{part}"),
                None => format!("d:{path}"),
            },
            None => format!("d:{k}"),
        },
        Of::Path(path, part) => match part {
            Some(part) => format!("x:{path}@{part}"),
            None => format!("x:{path}"),
        },
    }
}

/// The scope a group's resources' ids are minted under: its id's letters and digits, the
/// rest `_` (`grad-p_1A2B3C4D-1`).
fn scope_of(id: &str) -> String {
    let mut scope: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    scope.push('-');
    scope
}

/// A drawing's slot moved up by `dy`.
fn moved(slot: &DrawingSlot, dy: i64) -> DrawingSlot {
    DrawingSlot {
        y: slot.y - Px::int(dy),
        ..slot.clone()
    }
}

/// A group written: its items in its own coordinates, its resources in its own scope.
fn written(
    group: &Gathered<'_>,
    id: String,
    paint: &PagePaint,
    cx: &SvgContext<'_>,
    names: &PageNames<'_>,
) -> FeedGroup {
    let ink = Ink {
        names,
        feed: Some(group.dy),
    };
    let mut defs = PageDefs::feed(scope_of(&id));
    let mut content = Vec::new();
    for item in &group.items {
        match item {
            Item::Line(line) => content.push(line_element(line, cx, &mut defs, ink)),
            Item::Rule(rule) => content.push(rect(rule, ink)),
            Item::Picture(d) => content.push(picture(d, cx, &mut defs, ink)),
            Item::Slot(k) => content.extend(slot_content(paint, *k, group.dy, (cx, names), &mut defs, ink)),
        }
    }
    let pictures = defs.feed.take().map(|f| f.pictures).unwrap_or_default();
    FeedGroup {
        id,
        dy: group.dy,
        content: write_lines(&content),
        defs: defs.defs.concat(),
        pictures,
    }
}

/// Drawing `k` of the page painted moved up by `dy`, with its text boxes, their pictures
/// and their inline drawings; nothing where it has no painter.
fn slot_content(
    paint: &PagePaint,
    k: usize,
    dy: i64,
    (cx, names): (&SvgContext<'_>, &PageNames<'_>),
    defs: &mut PageDefs,
    ink: Ink<'_>,
) -> Vec<Element> {
    let Some(slot) = paint.drawings.get(k) else {
        return Vec::new();
    };
    let slot = moved(slot, dy);
    let text: Vec<_> = paint.text_boxes.iter().filter(|t| t.slot == k).collect();
    let inner: Vec<DrawingSlot> = text
        .iter()
        .flat_map(|t| t.drawings.iter())
        .map(|inner| moved(inner, dy))
        .collect();
    let inner: Vec<&DrawingSlot> = inner.iter().collect();
    let (drawn, inner) = paint_drawing(&slot, &inner, (cx, names), defs);
    let painted = Painted {
        slot: &slot,
        paint: drawn,
        text,
        inner,
    };
    if painted.paint.is_none() {
        return Vec::new();
    }
    float_content(&painted, cx, defs, ink)
}
