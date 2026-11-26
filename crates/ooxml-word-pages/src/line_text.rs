//! Owns: what a check reads of a painted page's lines' text ([`LineText`], lane L4b), made
//! in Rust from the page's paint so that no line is read into Python for it: each line's
//! count of characters that are not white space (an empty paragraph, `spacers`), the sizes
//! its text is drawn at with their characters (`smallest_text`), and the extent of its ink
//! and its widest word (`column_fit`, a table's ink past the margins), and where it is drawn
//! (a text box's line in a range's extent) -- docx-agent's
//! `layout_facts` rules (`_ink`, `_line_widths`, `text_sizes`), on the same spans.
//!
//! A line names its paragraph by key ([`ItemPlace`]), never by path: what a check read of
//! a page is kept across versions by the page tier ([`crate::page_reads`]), and a path
//! moves where a key does not. What a fact reads of the paragraph's place -- its kind of
//! text, the body table's row and cell it is in ([`LinePlace`]) -- is read when the line
//! is, by keys too.
//!
//! Must not: paint (the engine does), or judge what it measures.

use crate::layout_facts::{cell_line, row_of, story_kind};
use ooxml_word_layout::paint::{PaintedLine, Span, SpanKind};
use ooxml_word_layout::px::Px;
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::ids::{Anchor, BlockKey, StoryIx};
use ooxml_word_model::value::Text;

/// What a painted item is drawn for, in the version's blocks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemPlace {
    /// A block, or a place on one, its story's index holds.
    Anchored(Anchor),
    /// A place below a paragraph its story's index holds: its steps below the paragraph (a
    /// run's, a drawing's, a text box's item's), which move with the paragraph. A text box's
    /// blocks are named so until their story's index keys them (phase 3, RD1).
    Below(Anchor, Text),
    /// What nothing its story's index holds is above (a separator the layout makes, a text
    /// box's item in a drawing with no anchor): its path.
    Path(Text),
}

impl ItemPlace {
    /// The place of an item painted for `anchor` with `path`: its anchor; else below the
    /// paragraph `host` holding it (a text box's item: its drawing's paragraph); else its
    /// path.
    #[must_use]
    pub fn of(names: &VersionNames, anchor: Option<Anchor>, path: &str, host: Option<Anchor>) -> ItemPlace {
        if let Some(anchor) = anchor {
            return ItemPlace::Anchored(anchor);
        }
        let below = host.and_then(|host| {
            let named = names.path(Anchor::whole(host.block))?;
            let rest = path.strip_prefix(named.as_str())?.strip_prefix('/')?;
            Some(ItemPlace::Below(Anchor::whole(host.block), Text::from(rest)))
        });
        below.unwrap_or_else(|| ItemPlace::Path(Text::from(path)))
    }

    /// Its paragraph's (or block's) key, where its story's index holds it.
    #[must_use]
    pub fn key(&self) -> Option<BlockKey> {
        match self {
            ItemPlace::Anchored(anchor) | ItemPlace::Below(anchor, _) => Some(anchor.block),
            ItemPlace::Path(_) => None,
        }
    }

    /// Its anchor, where it is one the index holds itself.
    #[must_use]
    pub fn anchor(&self) -> Option<Anchor> {
        match self {
            ItemPlace::Anchored(anchor) => Some(*anchor),
            _ => None,
        }
    }

    /// Its path, named through the version's `names` (what the layout model's path is).
    #[must_use]
    pub fn named(&self, names: &VersionNames) -> Option<String> {
        match self {
            ItemPlace::Anchored(anchor) => names.path(*anchor),
            ItemPlace::Below(host, below) => Some(format!("{}/{below}", names.path(*host)?)),
            ItemPlace::Path(path) => Some(path.to_string()),
        }
    }
}

/// A body table's row a line or rule is drawn in, as docx-agent's `_ROW` reads it: the
/// outermost table's key and the row, from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableRow {
    /// The outermost table.
    pub table: BlockKey,
    /// The row, from 0.
    pub row: usize,
}

/// What a fact reads of where a line's paragraph is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinePlace {
    /// Its story's kind (`_story_kind`): `body`, `footnotes`, `endnotes`, `header` or
    /// `footer`.
    pub story: &'static str,
    /// Its kind of text (`_kind`): `body`, `table` (a body table's cell), `footnote`,
    /// `endnote`, `header` or `footer`.
    pub kind: &'static str,
    /// The body table's row it is in.
    pub row: Option<TableRow>,
    /// Where it is a paragraph right in that row's cell (`_CELL_LINE`): the cell, from 0, and
    /// the paragraph's steps below the cell (`w:p[n]`, or `w:sdt[n]/w:sdtContent[1]/w:p[n]`).
    pub cell: Option<(usize, Text)>,
}

impl LinePlace {
    /// The place of a line painted with `path`, of `story` (a header's or footer's kind),
    /// its table found through the version's `names`.
    #[must_use]
    pub fn of(names: &VersionNames, path: &str, story: Option<&'static str>) -> LinePlace {
        let row = table_row(names, path);
        let cell = row
            .and_then(|_| cell_line(path))
            .map(|(_, _, cell, below)| (cell, Text::from(below)));
        LinePlace {
            story: story.unwrap_or_else(|| story_kind(path)),
            kind: kind_of(path, story),
            row,
            cell,
        }
    }
}

/// The body table's row an item painted with `path` is drawn in ([`row_of`]), its table by
/// key.
#[must_use]
pub fn table_row(names: &VersionNames, path: &str) -> Option<TableRow> {
    let (table, row) = row_of(path)?;
    Some(TableRow {
        table: names.key_at_path(StoryIx::BODY, table)?,
        row,
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

/// One painted line's text, as a check reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineText {
    /// Its paragraph.
    pub place: ItemPlace,
    /// Where its paragraph is, as the facts read it.
    pub at: LinePlace,
    /// Its story's kind and part (a header's or footer's), where it is one's.
    pub story: Option<(&'static str, String)>,
    /// Its number in its paragraph.
    pub number: usize,
    /// The top of its pitch, px from the page's top.
    pub top: Px,
    /// Its pitch.
    pub pitch: Px,
    /// Its text column, in a section of several.
    pub column: Option<usize>,
    /// Its characters that are not white space, every span's.
    pub inked: usize,
    /// Each text or label span not raised or lowered with characters that are not white
    /// space: its size (half points) and those characters.
    pub sizes: Vec<(i64, usize)>,
    /// From its first character that is not white space to the end of its last, device
    /// px (`_ink`).
    pub ink: Option<(Px, Px)>,
    /// Its longest run of characters without a space, device px (`_line_widths`).
    pub widest: Option<Px>,
}

/// White space as Python's `str.isspace` says it (Rust's `White_Space` and the four
/// information separators).
fn space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// A character's start and end on its line: its pen, and the next one's (the span's end
/// after its last).
fn extent(span: &Span, k: usize) -> Option<(Px, Px)> {
    let start = span.x(k)?;
    let end = span.x(k + 1).unwrap_or(span.end);
    Some((start, end))
}

impl LineText {
    /// `line`'s text facts, its paragraph named through the version's `names`; `host`, the
    /// paragraph whose drawing a text box's line is drawn in.
    pub fn of(line: &PaintedLine, names: &VersionNames, host: Option<Anchor>) -> LineText {
        let mut inked = 0;
        let mut sizes = Vec::new();
        let mut ink: Option<(Px, Px)> = None;
        let mut widest: Option<Px> = None;
        let mut word: Option<(Px, Px)> = None;
        for span in &line.spans {
            let mine = span.chars.iter().filter(|c| !space(**c)).count();
            inked += mine;
            let scripted = span
                .vertical_align
                .as_deref()
                .is_some_and(|v| v == "superscript" || v == "subscript");
            if matches!(span.kind, SpanKind::Text | SpanKind::Label) && !scripted && mine > 0 {
                sizes.push((span.half_points, mine));
            }
            for (k, c) in span.chars.iter().enumerate() {
                let Some((start, end)) = extent(span, k) else { continue };
                if space(*c) {
                    if let Some((a, b)) = word.take() {
                        widest = Some(widest.map_or(b - a, |w| w.max(b - a)));
                    }
                    continue;
                }
                let (low, high) = (start.min(end), start.max(end));
                ink = Some(ink.map_or((low, high), |(l, h)| (l.min(low), h.max(high))));
                word = Some(word.map_or((start, end), |(a, _)| (a, end)));
            }
        }
        if let Some((a, b)) = word {
            widest = Some(widest.map_or(b - a, |w| w.max(b - a)));
        }
        let story = line.story.as_ref().map(|(kind, _)| *kind);
        LineText {
            place: ItemPlace::of(names, line.anchor, &line.path, host),
            at: LinePlace::of(names, &line.path, story),
            story: line.story.as_ref().map(|(kind, part)| (*kind, part.to_string())),
            number: line.number,
            top: line.top,
            pitch: line.pitch,
            column: line.column,
            inked,
            sizes,
            ink,
            widest: ink.and(widest.or(Some(Px::ZERO))),
        }
    }
}
