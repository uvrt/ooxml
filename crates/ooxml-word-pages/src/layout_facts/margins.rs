//! Owns: `past_margins` (docx-agent's `past_margins`): a picture or other drawing of the
//! body, or a body table, drawn past its page's margins by more than [`MARGIN_TOLERANCE`]
//! -- a table as drawn (its rules and its lines' ink), beyond Word's own placement of it
//! (outdented by its first cell's left margin and its outer border's outer half, the same
//! on the right: [`allowances`]), and not where it stays within the body's lines beside it
//! (a negative indent's reach: [`body_indents`]) -- the farthest first, with the resize
//! that fits an inline picture running past the right margin only.
//!
//! Must not: word an entry (the tool layer does) or cap a list.

use super::tree::{self, is_w, W};
use super::{points, Facts};
use crate::page_parts::Part;
use crate::page_read::PageRead;
use ooxml_base::Q;
use ooxml_word_layout::px::Px;
use ooxml_word_layout::tables::outer_part;
use ooxml_word_model::docx::drawings::pictures;
use ooxml_word_model::docx::keeps::Keeps;
use ooxml_word_model::docx::{at_path, StoryPart};
use ooxml_word_model::ids::BlockKey;
use ooxml_word_model::model::Block;
use ooxml_xml::{Element, QName};
use std::collections::HashMap;
use std::sync::Arc;

/// Past a margin by at most this, pt, is on it: the layout's device-pixel rounding.
pub const MARGIN_TOLERANCE: f64 = 0.5;

/// The sides something reaches past the margins, pt, in docx-agent's order (left, right,
/// top, bottom).
pub type Past = Vec<(&'static str, f64)>;

/// What runs past the margins.
#[derive(Debug, Clone, PartialEq)]
pub enum PastMargins {
    /// A drawing: its id (its path where it has none), its page, how far past, and the
    /// width (pt) that fits it, where resizing it does.
    Drawing {
        /// Its id or path.
        drawing: String,
        /// The 1-based page.
        page: usize,
        /// How far past each side.
        past: Past,
        /// The width that fits it.
        fit: Option<f64>,
    },
    /// A table.
    Table {
        /// Its id or path.
        table: String,
        /// The 1-based page.
        page: usize,
        /// How far past each side.
        past: Past,
    },
}

impl PastMargins {
    fn farthest(&self) -> f64 {
        let (Self::Drawing { past, .. } | Self::Table { past, .. }) = self;
        past.iter().map(|(_, a)| *a).fold(f64::NEG_INFINITY, f64::max)
    }
}

/// `past_margins` on the pages asked for, the farthest first.
#[must_use]
pub fn past_margins(facts: &Facts<'_>) -> Vec<PastMargins> {
    let Some(body) = facts.docx().body() else {
        return Vec::new();
    };
    let mut reader = Reader {
        facts,
        body,
        frames: pictures(facts.docx())
            .into_iter()
            .filter(|d| d.part == body.part)
            .map(|d| (d.path, d.id))
            .collect(),
        allowed: allowances(facts),
        indents: HashMap::new(),
    };
    let mut entries = Vec::new();
    for (index, page) in facts.version().facts().pages.iter().enumerate() {
        if !facts.shown([index + 1]) {
            continue;
        }
        let frame = &page.frame;
        let column = (frame.left, frame.right, frame.top, frame.bottom);
        reader.drawings(index, column, &mut entries);
        reader.tables(index, column, &mut entries);
    }
    entries.sort_by(|a, b| b.farthest().total_cmp(&a.farthest()));
    entries
}

/// What `past_margins` reads once for every page.
struct Reader<'f, 'a> {
    facts: &'f Facts<'a>,
    body: &'a StoryPart,
    /// The main part's pictures: their places and ids.
    frames: Vec<(Box<[u32]>, String)>,
    /// What each body table may reach past the margins, by key.
    allowed: HashMap<BlockKey, (Px, Px)>,
    /// The indents of the body's lines beside each table, by key, as found.
    indents: HashMap<BlockKey, Option<(f64, f64)>>,
}

/// A page's text column: left, right, top, bottom.
type Column = (Px, Px, Px, Px);

impl Reader<'_, '_> {
    /// The drawings on the 0-based page `index` past its margins.
    fn drawings(&self, index: usize, column: Column, entries: &mut Vec<PastMargins>) {
        let Some(boxes) = self.facts.boxes(index) else { return };
        let placed = boxes
            .pictures
            .iter()
            .map(|b| (b, true))
            .chain(boxes.floats.iter().map(|b| (b, false)));
        for (drawn, inline) in placed {
            let y = Some((drawn.y, drawn.y + drawn.height));
            let past = past(drawn.x, drawn.x + drawn.width, y, column, !inline);
            if past.is_empty() {
                continue;
            }
            let Some(path) = self.facts.path_of_place(&drawn.place) else {
                continue;
            };
            let id = drawing_id(self.body, &self.frames, &path).unwrap_or(path);
            let only_right = matches!(past.as_slice(), [("right", _)]);
            let right = column.1;
            let fit =
                (inline && only_right && id.starts_with("d:") && drawn.x < right).then(|| points(right - drawn.x));
            entries.push(PastMargins::Drawing {
                drawing: id,
                page: index + 1,
                past,
                fit,
            });
        }
    }

    /// The body tables on the 0-based page `index` past its margins: each one's extent as
    /// drawn, its rules' and its lines' ink.
    fn tables(&mut self, index: usize, column: Column, entries: &mut Vec<PastMargins>) {
        let facts = self.facts;
        let Some(entry) = facts.entry(index) else { return };
        for table in &entry.tables().0 {
            let (low, high) = table.extent;
            let (left, right) = self.allowed.get(&table.table).copied().unwrap_or((Px::ZERO, Px::ZERO));
            let mut over = past(low + left, high - right, None, column, false);
            if !over.is_empty() {
                let drawn = past(low, high, None, column, false);
                self.within_body_lines(table.table, &mut over, &drawn);
            }
            if !over.is_empty() {
                let names = facts.names();
                let named = names.id(table.table).map(|id| id.to_string());
                let Some(id) = named.or_else(|| names.path(ooxml_word_model::ids::Anchor::whole(table.table))) else {
                    continue;
                };
                entries.push(PastMargins::Table {
                    table: id,
                    page: index + 1,
                    past: over,
                });
            }
        }
    }

    /// `over` less the sides on which the table `table`, as `drawn`, stays within the
    /// body's lines beside it (a negative indent's reach; `_past_body_lines`).
    fn within_body_lines(&mut self, table: BlockKey, over: &mut Past, drawn: &Past) {
        let (facts, body) = (self.facts, self.body);
        let lines = *self.indents.entry(table).or_insert_with(|| {
            facts
                .names()
                .place_of(table)
                .map(|place| body_indents(facts, body, &place))
        });
        let Some((left, right)) = lines else { return };
        over.retain(|(side, _)| {
            let reach = if *side == "left" { -left } else { -right };
            let drawn = drawn.iter().find(|(s, _)| s == side).map_or(0.0, |(_, a)| *a);
            !(reach > 0.0 && drawn <= reach + 2.0 * MARGIN_TOLERANCE)
        });
    }
}

/// One page's contribution to `past_margins` and to the tables as drawn (the page tier's,
/// [`crate::page_entry`]): the body tables it draws a rule or an inked line of, in the
/// order first drawn (its rules, then its lines), each with its extent as drawn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableInkPart(pub(crate) Vec<TableInk>);

/// A body table's rules and lines' ink on one page.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableInk {
    /// The outermost table (as `_ROW` reads it).
    pub(crate) table: BlockKey,
    /// Its rules' and lines' ink from left to right, device px.
    pub(crate) extent: (Px, Px),
    /// It draws a rule.
    pub(crate) ruled: bool,
}

impl Part<PageRead> for TableInkPart {
    fn of(read: &PageRead) -> TableInkPart {
        let mut out: Vec<TableInk> = Vec::new();
        let rules = read.boxes.rules.iter().map(|r| (r.row, r.x, r.x + r.width, true));
        let inks = read
            .texts
            .iter()
            .filter(|l| l.story.is_none())
            .filter_map(|l| l.ink.map(|(low, high)| (l.at.row, low, high, false)));
        for (row, low, high, ruled) in rules.chain(inks) {
            let Some(table) = row.map(|r| r.table) else { continue };
            match out.iter_mut().find(|t| t.table == table) {
                Some(found) => {
                    found.extent = (found.extent.0.min(low), found.extent.1.max(high));
                    found.ruled |= ruled;
                }
                None => out.push(TableInk {
                    table,
                    extent: (low, high),
                    ruled,
                }),
            }
        }
        TableInkPart(out)
    }

    fn weight_of(read: &PageRead) -> usize {
        let each = std::mem::size_of::<TableInk>();
        let rules = read.boxes.rules.iter().filter(|r| r.row.is_some()).count();
        let lines = read
            .texts
            .iter()
            .filter(|l| l.story.is_none() && l.ink.is_some() && l.at.row.is_some())
            .count();
        std::mem::size_of::<TableInkPart>() + (rules + lines) * each
    }
}

/// How far `x0..x1` (and `y0..y1`, where `vertical`) reach past `column`'s edges, pt,
/// where by more than [`MARGIN_TOLERANCE`] (`_past`).
fn past(x0: Px, x1: Px, y: Option<(Px, Px)>, column: (Px, Px, Px, Px), vertical: bool) -> Past {
    let (left, right, top, bottom) = column;
    let mut sides = vec![("left", left - x0), ("right", x1 - right)];
    if let (true, Some((y0, y1))) = (vertical, y) {
        sides.extend([("top", top - y0), ("bottom", y1 - bottom)]);
    }
    sides
        .into_iter()
        .map(|(side, amount)| (side, points(amount)))
        .filter(|(_, amount)| *amount > MARGIN_TOLERANCE)
        .collect()
}

/// The id of the picture a box at `path` draws (`_drawing_id`): the first of the main
/// part's pictures (`frames`, their places and ids) at or below the element the path
/// names.
fn drawing_id(story: &StoryPart, frames: &[(Box<[u32]>, String)], path: &str) -> Option<String> {
    let place = drawing_place(story, path)?;
    frames
        .iter()
        .find(|(at, _)| at.starts_with(&place))
        .map(|(_, id)| id.clone())
}

/// The place of what a drawing's path names: its run's, or the run's `k`-th floating or
/// inline frame where the path ends at one (`.../w:r[2]/wp:anchor[1]`).
fn drawing_place(story: &StoryPart, path: &str) -> Option<Vec<u32>> {
    let (run_path, frame) = match path.rsplit_once('/') {
        Some((run, last)) if last.starts_with("wp:anchor[") || last.starts_with("wp:inline[") => (run, Some(last)),
        _ => (path, None),
    };
    let (run, mut place) = tree::resolve(story, run_path)?;
    let Some(step) = frame else { return Some(place) };
    let (name, count) = step.strip_prefix("wp:")?.split_once('[')?;
    let count: usize = count.strip_suffix(']')?.parse().ok()?;
    let found = run_frames(run, name).into_iter().nth(count.checked_sub(1)?)?;
    place.extend(found);
    Some(place)
}

/// A run's `wp:<name>` frames in order, each with its place below the run: in its
/// `w:drawing`s, an `mc:AlternateContent`'s choice's too.
fn run_frames(run: &Arc<Element>, name: &str) -> Vec<Vec<u32>> {
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
    let mut out = Vec::new();
    for (k, child) in elements(run) {
        let drawings: Vec<(Vec<u32>, &Arc<Element>)> = if is_w(child, "drawing") {
            vec![(vec![k], child)]
        } else if child.name().is(Some(MC), "AlternateContent") {
            elements(child)
                .filter(|(_, c)| c.name().is(Some(MC), "Choice"))
                .flat_map(|(c, choice)| {
                    elements(choice)
                        .filter(|(_, d)| is_w(d, "drawing"))
                        .map(move |(d, drawing)| (vec![k, c, d], drawing))
                })
                .take(1)
                .collect()
        } else {
            Vec::new()
        };
        for (at, drawing) in drawings {
            if let Some((f, _)) = elements(drawing).find(|(_, f)| f.name().is(Some(WP), name)) {
                out.push([at, vec![f]].concat());
            }
        }
    }
    out
}

/// `element`'s element children with their child indexes.
fn elements(element: &Arc<Element>) -> impl Iterator<Item = (u32, &Arc<Element>)> + '_ {
    element
        .children()
        .iter()
        .enumerate()
        .filter_map(|(k, n)| Some((u32::try_from(k).ok()?, n.as_element()?)))
}

/// The device px each body table may reach past the margins as Word places it, by its
/// key (`_table_allowance`): its first row's first cell's left margin and left outer
/// border's outer half, its last cell's right margin and right one's.
fn allowances(facts: &Facts<'_>) -> HashMap<BlockKey, (Px, Px)> {
    let px = |twips: i64| Px::points(Q::new(twips.into(), 20.into()));
    let mut out = HashMap::new();
    for block in facts.version().layout().document.body.iter() {
        let Block::Table(table) = block else { continue };
        let Some(key) = table.key else { continue };
        let resolved = facts.resolved_table(table);
        let Some((first, last)) = resolved
            .rows
            .first()
            .and_then(|r| Some((r.cells.first()?, r.cells.last()?)))
        else {
            continue;
        };
        let left = first.margins[1] + outer_part(first.left_edge.0.as_ref());
        let right = last.margins[3] + outer_part(last.right_edge.0.as_ref());
        out.insert(key, (px(left), px(right)));
    }
    out
}

/// The left and right indents, pt, of the body's lines beside the table at `place`
/// (docx-agent's `body_lines`): those of the nearest paragraph before it among its
/// siblings, else after it, else the default paragraph style's.
fn body_indents(facts: &Facts<'_>, story: &StoryPart, place: &[u32]) -> (f64, f64) {
    let mut look = Keeps::new(facts.formatting(), story);
    let effective = match around(story.root(), place).and_then(|p| Some((at_path(story.root(), &p)?, p))) {
        Some((element, at)) => look.effective(element, &at, true),
        None => {
            let name = Arc::new(QName::new(Some("w"), "p", Some(W)));
            let empty = Element::new(name, Vec::new(), Vec::new(), Vec::new());
            look.effective(&empty, &[], true)
        }
    };
    (effective.indent_left, effective.indent_right)
}

/// The place of the nearest paragraph before the element at `place` among its siblings,
/// else after it.
fn around(root: &Arc<Element>, place: &[u32]) -> Option<Vec<u32>> {
    let (&at, above) = place.split_last()?;
    let parent = at_path(root, above)?;
    let paragraph = |k: usize| {
        let node = parent.children().get(k)?.as_element()?;
        let k = u32::try_from(k).ok()?;
        is_w(node, "p").then(|| [above, &[k]].concat())
    };
    let at = at as usize;
    (0..at)
        .rev()
        .find_map(paragraph)
        .or_else(|| (at + 1..parent.children().len()).find_map(paragraph))
}
