//! Owns: a copy's list items and page settings as Word applies them (docx-agent's
//! `edit/copy_settings.py`, round 42 R49, round 43): every body paragraph that draws a list
//! label, counted as Word counts them over the whole body ([`Reader`]: in document order,
//! table cells, content controls and custom XML included, not text boxes), and each
//! paragraph of a region's settings ([`region`]) -- its list level, the label its level
//! draws, where its number stands in its count, its indents and its page settings -- read
//! along the region, as the module's "list_numbering" says.
//!
//! Must not: compare two regions (`super::differences` does) or write a fix.

use crate::style::compare::Side;
use crate::table_read::{own_rows, row_cells, Placed};
use ooxml_base::pyfmt::round2;
use ooxml_fonts::symbols::{symbol_face, to_unicode};
use ooxml_word_model::docx::formatting::EffectiveParagraph;
use ooxml_word_model::docx::{find_path, StoryPart};
use ooxml_word_model::ids::W;
use ooxml_word_model::labels::Counters;
use ooxml_word_model::numbering::{Level, ListKey};
use ooxml_word_model::value::Value as Prop;
use ooxml_xml::Element;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Every setting compared, in the order reported.
pub(super) const SETTINGS: [&str; 9] = [
    "list_level",
    "list_format",
    "list_numbering",
    "indent_left",
    "indent_first",
    "keep_with_next",
    "keep_lines",
    "page_break_before",
    "widow_control",
];

/// The list's settings: `word_lists` makes them; `word_follow_sample` keeps a copy's list.
pub(super) const LIST_SETTINGS: [&str; 3] = ["list_level", "list_format", "list_numbering"];

/// The page settings, compared whatever the list.
pub(super) const PAGE: [&str; 4] = ["keep_with_next", "keep_lines", "page_break_before", "widow_control"];

/// `word_compare_blocks`' properties these replace for the paragraphs compared here.
pub(super) const REPLACED: [&str; 5] = [
    "indent_left",
    "indent_first",
    "keep_with_next",
    "keep_lines",
    "page_break_before",
];

/// `list_numbering`'s value where the caller states the copies' lists run on and a copy's
/// item goes on from the copy before it, in the same list.
pub(super) const RUNS_ON: &str = "runs on";

/// A paragraph: its place in the main part and its element.
pub(super) type Paragraph = (Vec<u32>, Arc<Element>);

/// How a list item's number comes (docx-agent's `COUNT`): what whether its number is
/// compared is decided by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Count {
    /// Numbered by the document's outline.
    Outline,
    /// Its count begins inside the region, or it continues an item of the region.
    Own,
    /// Its count goes on from an item before the region.
    RunsOn,
}

/// A body paragraph that draws a list label, as counted.
#[derive(Debug, Clone)]
struct Item {
    key: ListKey,
    level: i64,
    format: Option<String>,
    text: Option<String>,
    number: Option<i64>,
    fresh: bool,
    outline: bool,
}

/// A paragraph's settings, as Word applies them.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Settings {
    pub(super) indent_left: f64,
    pub(super) indent_first: f64,
    pub(super) keep_with_next: bool,
    pub(super) keep_lines: bool,
    pub(super) page_break_before: bool,
    pub(super) widow_control: bool,
    pub(super) list_level: Option<i64>,
    pub(super) list_format: Option<String>,
    pub(super) list_numbering: Option<String>,
    /// How its number comes (none: not numbered).
    pub(super) count: Option<Count>,
    /// The instances counting together with it.
    pub(super) key: Option<ListKey>,
    /// No item of its list at its level or a shallower one before it in the region.
    pub(super) lead: bool,
}

impl Settings {
    /// The setting `name`'s value as reported.
    pub(super) fn value(&self, name: &str) -> Value {
        match name {
            "list_level" => self.list_level.map_or(Value::Null, Value::from),
            "list_format" => self.list_format.clone().map_or(Value::Null, Value::from),
            "list_numbering" => self.list_numbering.clone().map_or(Value::Null, Value::from),
            "indent_left" => Value::from(self.indent_left),
            "indent_first" => Value::from(self.indent_first),
            "keep_with_next" => Value::from(self.keep_with_next),
            "keep_lines" => Value::from(self.keep_lines),
            "page_break_before" => Value::from(self.page_break_before),
            "widow_control" => Value::from(self.widow_control),
            _ => Value::Null,
        }
    }

    /// Whether its label draws bullets.
    pub(super) fn bullets(&self) -> bool {
        self.list_format.as_deref().is_some_and(|f| f.starts_with("bullet "))
    }
}

/// One document's body as the settings read it: its list items counted, and each
/// paragraph's formatting as Word applies it.
pub(super) struct Reader<'s, 'd> {
    side: &'s Side<'d>,
    body: &'s StoryPart,
    items: HashMap<Vec<u32>, Item>,
}

impl<'s, 'd> Reader<'s, 'd> {
    /// The body `body` of `side`'s document, its list items counted.
    pub(super) fn new(side: &'s Side<'d>, body: &'s StoryPart) -> Self {
        let mut reader = Reader {
            side,
            body,
            items: HashMap::new(),
        };
        reader.items = reader.count();
        reader
    }

    /// The body read.
    pub(super) fn body(&self) -> &'s StoryPart {
        self.body
    }

    /// The paragraph's formatting as Word applies it, in its cell where it is in one.
    pub(super) fn applied(&self, (path, element): &Paragraph) -> EffectiveParagraph {
        let cell = self.side.cell(self.body, element, path);
        self.side.formatting().paragraph(element, cell.as_ref())
    }

    fn count(&self) -> HashMap<Vec<u32>, Item> {
        let numbering = self.side.formatting().numbering();
        let mut counters = Counters::default();
        let mut out = HashMap::new();
        for paragraph in body_paragraphs(self.body) {
            let resolved = self.applied(&paragraph).resolved;
            let Some(num_id) = resolved.get("numPr.numId").and_then(Prop::int).filter(|n| *n != 0) else {
                continue;
            };
            let level = resolved.get("numPr.ilvl").and_then(Prop::int).unwrap_or(0);
            // Whether the item begins its count, read before it is counted (round 43).
            let fresh = counters.begins(numbering, num_id, level);
            let (label, number) = counters.item_number(numbering, num_id, level);
            if label.is_none() {
                continue;
            }
            let found = numbering.level(num_id, level);
            let outline = by_outline(resolved.get("outlineLvl").and_then(Prop::int), &paragraph.1, found);
            out.insert(
                paragraph.0,
                Item {
                    key: numbering.list_of(num_id),
                    level,
                    format: found.and_then(|l| l.format.as_deref()).map(str::to_owned),
                    text: found.and_then(|l| shown(l)),
                    number,
                    fresh,
                    outline,
                },
            );
        }
        out
    }
}

/// The body's paragraphs in document order as the layout reads them: table cells,
/// block-level content controls and custom XML included, text boxes not.
fn body_paragraphs(body: &StoryPart) -> Vec<Paragraph> {
    fn blocks(parent: &Element, at: &[u32], out: &mut Vec<Paragraph>) {
        for (k, node) in parent.children().iter().enumerate() {
            let Some(child) = node.as_element() else { continue };
            let place = [at, &[u32::try_from(k).unwrap_or(u32::MAX)]].concat();
            if child.name().is(Some(W), "p") {
                out.push((place, Arc::clone(child)));
            } else if child.name().is(Some(W), "tbl") {
                for row in own_rows(child) {
                    let row = Placed {
                        path: [place.as_slice(), &row.path].concat(),
                        element: row.element,
                    };
                    for (cell, _, _) in row_cells(&row) {
                        blocks(&cell.element, &cell.path, out);
                    }
                }
            } else if child.name().is(Some(W), "sdt") {
                if let Some((j, content)) = child.children().iter().enumerate().find_map(|(j, n)| {
                    n.as_element()
                        .filter(|e| e.name().is(Some(W), "sdtContent"))
                        .map(|e| (j, e))
                }) {
                    let inner = [place.as_slice(), &[u32::try_from(j).unwrap_or(u32::MAX)]].concat();
                    blocks(content, &inner, out);
                }
            } else if child.name().is(Some(W), "customXml") {
                blocks(child, &place, out);
            }
        }
    }
    let mut out = Vec::new();
    let root = body.root();
    for (k, node) in root.children().iter().enumerate() {
        if let Some(element) = node.as_element().filter(|e| e.name().is(Some(W), "body")) {
            blocks(element, &[u32::try_from(k).unwrap_or(u32::MAX)], &mut out);
        }
    }
    out
}

/// The paragraphs at and below `element` (at `path`), in document order; text boxes'
/// only where `boxes`.
pub(super) fn paragraphs_in(element: &Arc<Element>, path: &[u32], boxes: bool) -> Vec<Paragraph> {
    fn walk(element: &Arc<Element>, path: &[u32], boxes: bool, out: &mut Vec<Paragraph>) {
        if element.name().is(Some(W), "p") {
            out.push((path.to_vec(), Arc::clone(element)));
        }
        for (k, node) in element.children().iter().enumerate() {
            let Some(child) = node.as_element() else { continue };
            if !boxes && child.name().is(Some(W), "txbxContent") {
                continue;
            }
            let place = [path, &[u32::try_from(k).unwrap_or(u32::MAX)]].concat();
            walk(child, &place, boxes, out);
        }
    }
    let mut out = Vec::new();
    walk(element, path, boxes, &mut out);
    out
}

/// A level's text as it shows: a symbol face's characters as the Unicode they draw.
fn shown(level: &Level) -> Option<String> {
    let text = level.text.as_deref()?;
    let face = match level.run.get("rFonts.ascii") {
        Some(Prop::Font(font)) => font.face.as_deref().and_then(symbol_face),
        _ => None,
    };
    let Some(face) = face else {
        return Some(text.to_owned());
    };
    Some(
        text.chars()
            .map(|ch| to_unicode(face, ch).unwrap_or_else(|| ch.to_string()))
            .collect(),
    )
}

/// Whether an item is numbered by the document's outline: the paragraph stands at an
/// outline level (0-8, its own or its style's) and its numbering is its style's (no
/// `w:numId` of its own) or a level tied to a paragraph style. Read from the structure,
/// never the style's name.
fn by_outline(outline: Option<i64>, paragraph: &Element, level: Option<&Arc<Level>>) -> bool {
    if !outline.is_some_and(|o| (0..=8).contains(&o)) {
        return false;
    }
    let own = find_path(paragraph, &["pPr", "numPr", "numId"])
        .and_then(|n| n.attribute(Some(W), "val"))
        .and_then(|v| v.trim().parse::<i64>().ok())
        .is_some_and(|n| n != 0);
    !own || level.is_some_and(|l| l.style.is_some())
}

/// Each of `paragraphs` (in document order: a copy's or a reference's) by its place: its
/// settings, `list_numbering` read along the region.
pub(super) fn region(reader: &Reader<'_, '_>, paragraphs: &[Paragraph]) -> HashMap<Vec<u32>, Settings> {
    let mut out = HashMap::new();
    // level -> (list key, number) of the item last there
    let mut seen: BTreeMap<i64, (ListKey, Option<i64>)> = BTreeMap::new();
    // list key -> the shallowest level of its items so far
    let mut shallowest: HashMap<ListKey, i64> = HashMap::new();
    for paragraph in paragraphs {
        let applied = reader.applied(paragraph);
        let mut values = Settings {
            indent_left: round2(applied.indent_left),
            indent_first: round2(applied.first_line - applied.hanging),
            keep_with_next: applied.keep_with_next,
            keep_lines: applied.keep_together,
            page_break_before: applied.page_break_before,
            widow_control: applied.resolved.get("widowControl").is_none_or(Prop::truthy),
            list_level: None,
            list_format: None,
            list_numbering: None,
            count: None,
            key: None,
            lead: false,
        };
        if let Some(item) = reader.items.get(&paragraph.0) {
            values.list_level = Some(item.level);
            values.list_format = Some(format!(
                "{} {}",
                item.format.as_deref().filter(|f| !f.is_empty()).unwrap_or("decimal"),
                item.text.as_deref().unwrap_or("None")
            ));
            values.key = Some(item.key);
            let lowest = shallowest.get(&item.key).copied().unwrap_or(9);
            values.lead = lowest > item.level;
            shallowest.insert(item.key, lowest.min(item.level));
            let numbered = !matches!(item.format.as_deref(), Some("bullet" | "none"))
                && item.text.as_deref().is_some_and(|t| t.contains('%'));
            if numbered {
                let goes_on = seen.get(&item.level).is_some_and(|(key, number)| {
                    *key == item.key && number.zip(item.number).is_some_and(|(before, now)| now == before + 1)
                });
                values.list_numbering = Some(if goes_on {
                    "continues".to_owned()
                } else {
                    format!(
                        "starts at {}",
                        item.number.map_or_else(|| "None".to_owned(), |n| n.to_string())
                    )
                });
                values.count = Some(if item.outline {
                    Count::Outline
                } else if goes_on || item.fresh {
                    Count::Own
                } else {
                    Count::RunsOn
                });
            }
            seen.insert(item.level, (item.key, item.number));
            seen.retain(|level, _| *level <= item.level);
        }
        out.insert(paragraph.0.clone(), values);
    }
    out
}
