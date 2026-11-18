//! Owns: the floating drawings anchored in a table cell, as docx2svg lays a cell out around
//! them (`table.cell_anchors_unmodelled`, `cell_anchors_approximate`, `unwrapped`,
//! `wrap_cell`; `floating.in_cell`, `cell_position_modelled`; measured by
//! `make_cell_anchor_probe.py` and `make_cell_valign_probe.py`, ROADMAP F.16, F.17, F.25):
//! which are positioned against the cell, which positions no probe measured, and a cell's
//! lines laid out around the drawings text wraps around in it -- beside them (`wrapSquare`,
//! `wrapTight`, `wrapThrough`) or below them (`wrapTopAndBottom`), in the cell's text area as
//! their column -- each drawing positioned from the cell laid out without it, held inside
//! the cell, and the row reaching down to its foot ([`wrap_cell`]).
//!
//! Must not: measure a paragraph (`super::build` does), place rows on pages, or draw.

use super::cells::{cell_stack, CellBlock, Content, Positioned};
use super::grid::{inner_part, left_offset, line_at, right_offset, twips_px, CellBox};
use crate::faces::Faces;
use crate::floating::{emu_twips, horizontal, moves_text, vertical, AnchorRef, Frames};
use crate::paginate::{clear_of_bands, Band, DrawingKey};
use crate::px::Px;
use crate::rebreak::{Breaker, LineExtra, Lines, ParaSource, Rebroken};
use crate::table::Unsupported;
use crate::wrap::{break_across, segments, Column, Wrap};
use ooxml_base::{Locale, Q};
use ooxml_text::Line;
use ooxml_word_model::model::{Anchor, Paragraph, Run};
use ooxml_word_model::resolve::ResolvedCell;
use ooxml_word_model::value::{Text, Value};
use ooxml_word_model::WordDocument;
use std::sync::Arc;

/// The wraps text goes around a drawing in a cell by: beside it, above and below it.
const CELL_WRAPS: [&str; 4] = ["wrapSquare", "wrapTight", "wrapThrough", "wrapTopAndBottom"];

/// What a drawing anchored in a cell may be positioned by, as measured:
/// `(axis, relativeFrom, offset or the alignment)` (`CELL_POSITIONS`); against the page
/// the same but `character` (`PAGE_POSITIONS`).
const CELL_POSITIONS: [(&str, &str, &str); 22] = [
    ("h", "column", "offset"),
    ("h", "column", "left"),
    ("h", "column", "center"),
    ("h", "column", "right"),
    ("h", "margin", "offset"),
    ("h", "margin", "left"),
    ("h", "margin", "center"),
    ("h", "margin", "right"),
    ("h", "page", "offset"),
    ("h", "leftMargin", "offset"),
    ("h", "character", "offset"),
    ("v", "paragraph", "offset"),
    ("v", "paragraph", "top"),
    ("v", "paragraph", "center"),
    ("v", "paragraph", "bottom"),
    ("v", "line", "offset"),
    ("v", "margin", "offset"),
    ("v", "margin", "top"),
    ("v", "margin", "center"),
    ("v", "margin", "bottom"),
    ("v", "page", "offset"),
    ("v", "topMargin", "offset"),
];

fn twip_px() -> Q {
    Q::new(300, 1440)
}

fn int(x: i64) -> Q {
    Q::from_integer(i128::from(x))
}

/// Whether a drawing anchored in a cell is positioned against the cell: with
/// `layoutInCell`, in mode 15 whatever it says, and below mode 15 without it where either
/// axis is against the anchor's `character` or `line` (`floating.in_cell`).
pub fn in_cell(anchor: &Anchor, mode15: bool) -> bool {
    mode15
        || anchor.layout_in_cell
        || anchor.h.relative.as_deref().unwrap_or("column") == "character"
        || anchor.v.relative.as_deref().unwrap_or("paragraph") == "line"
}

/// Why a drawing anchored in a cell cannot be positioned as measured (`None` where it
/// can): `simplePos`, or a position the probes did not measure (`cell_position_modelled`).
pub fn position_unmodelled(anchor: &Anchor, mode15: bool) -> Option<String> {
    if anchor.simple.is_some() {
        return Some("simplePos".to_owned());
    }
    let cell = in_cell(anchor, mode15);
    for (axis, position) in [("h", &anchor.h), ("v", &anchor.v)] {
        let relative = position
            .relative
            .as_deref()
            .unwrap_or(if axis == "h" { "column" } else { "paragraph" });
        let how = if position.offset.is_some() {
            "offset"
        } else {
            position
                .align
                .as_deref()
                .unwrap_or(if axis == "h" { "left" } else { "top" })
        };
        let known = CELL_POSITIONS
            .iter()
            .any(|k| *k == (axis, relative, how) && (cell || *k != ("h", "character", "offset")));
        if !known {
            return Some(format!("{relative} {how}"));
        }
    }
    None
}

/// Why a cell paragraph's floating drawings are not laid out as measured (`None` where
/// they are, `cell_anchors_unmodelled`): one text wraps around positioned against the
/// page below mode 15, or one with a wrap not measured in a cell.
pub fn unmodelled(paragraph: &Paragraph, mode15: bool) -> Option<String> {
    for anchor in paragraph.runs.iter().flat_map(|r| r.anchors.iter()) {
        if position_unmodelled(anchor, mode15).is_some() {
            continue;
        }
        if moves_text(anchor) && !in_cell(anchor, mode15) {
            return Some("a floating drawing text wraps around in a cell, positioned against the page".to_owned());
        }
        let wrap = anchor.wrap.as_deref().unwrap_or_default();
        if moves_text(anchor) && !CELL_WRAPS.contains(&wrap) {
            return Some(format!("a floating drawing in a cell with {wrap}"));
        }
    }
    None
}

/// Each of a cell paragraph's floating drawings positioned as no probe measured in a
/// cell, by `(run, index in the run)`, and why (`cell_anchors_approximate`).
pub fn approximate(paragraph: &Paragraph, mode15: bool) -> Vec<((usize, usize), String)> {
    let mut out = Vec::new();
    for (r, run) in paragraph.runs.iter().enumerate() {
        for (k, anchor) in run.anchors.iter().enumerate() {
            if let Some(why) = position_unmodelled(anchor, mode15) {
                out.push(((r, k), format!("is positioned by {why}, which no probe measured in a cell")));
            }
        }
    }
    out
}

/// `paragraph` with every floating drawing text wraps around made one it does not
/// (`wrapNone`): what the approximate rules lay a cell out with where the wrap is not
/// measured in a cell (`unwrapped`).
pub fn unwrapped(paragraph: &Paragraph) -> Paragraph {
    let runs: Vec<Run> = paragraph
        .runs
        .iter()
        .map(|run| {
            if run.anchors.is_empty() {
                return run.clone();
            }
            let anchors: Vec<Anchor> = run
                .anchors
                .iter()
                .map(|a| {
                    if moves_text(a) {
                        Anchor {
                            wrap: Some(Text::from("wrapNone")),
                            ..a.clone()
                        }
                    } else {
                        a.clone()
                    }
                })
                .collect();
            Run {
                anchors: anchors.into(),
                ..run.clone()
            }
        })
        .collect();
    Paragraph {
        runs: runs.into(),
        ..paragraph.clone()
    }
}

/// The page a table's cells are laid out on: its size and header and footer distances,
/// twips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPage {
    /// The page's width.
    pub width: i64,
    /// Its height.
    pub height: i64,
    /// The header distance.
    pub header: i64,
    /// The footer distance.
    pub footer: i64,
}

/// Where a cell is across, twips from the page's left: its text area's edges and the
/// inside of its left and right borders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellEdges {
    /// Its text's left edge.
    pub text_left: Q,
    /// Its text's right edge.
    pub text_right: Q,
    /// The inside of its left border.
    pub box_left: Q,
    /// The inside of its right border.
    pub box_right: Q,
}

impl CellEdges {
    /// A cell's edges on grid `lines`, its table's column starting `page_left` twips from
    /// the page's left.
    pub fn of(cell: &ResolvedCell, page_left: Q, lines: &[Q]) -> CellEdges {
        let left = page_left + line_at(lines, cell.column);
        let right = page_left + line_at(lines, cell.column + cell.span);
        CellEdges {
            text_left: left + int(left_offset(cell)),
            text_right: right - int(right_offset(cell)),
            box_left: left + int(inner_part(cell.left_edge.0.as_ref())),
            box_right: right - int(inner_part(cell.right_edge.0.as_ref())),
        }
    }

    /// The frames a drawing in the cell is positioned against: the cell's text area as
    /// `column` and `margin`, `page` and `leftMargin` from the inside of its left border,
    /// `margin`, `page` and `topMargin` from `top` twips, the paragraph's, line's and
    /// character's places (twips) as given.
    pub fn frames(&self, page: &CellPage, (top, page_number, mode15): (Q, usize, bool), at: [Q; 4]) -> Frames {
        let [paragraph_top, line_top, line_bottom, character] = at;
        Frames {
            page_width: page.width,
            page_height: page.height,
            left: self.text_left,
            right: int(page.width) - self.text_right,
            top,
            bottom: int(page.height) - top,
            header: page.header,
            footer: page.footer,
            page: page_number,
            mode15,
            paragraph_top,
            line_top,
            line_bottom,
            character,
            column: None,
            origin: (self.box_left, top),
            cell: true,
        }
    }
}

/// What laying a cell out around its drawings reads.
pub struct CellJob<'a> {
    /// The document.
    pub document: &'a WordDocument,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale.
    pub locale: &'a Locale,
    /// The cell.
    pub cell: &'a ResolvedCell,
    /// Where its text goes across.
    pub cell_box: &'a CellBox,
    /// Its edges.
    pub edges: CellEdges,
    /// The page.
    pub page: CellPage,
    /// The styles its stack reads at its edges: the paragraph before the table's, and the
    /// default paragraph style.
    pub styles: (Option<&'a Text>, Option<&'a Text>),
    /// Each block's paragraph, what its lines are broken from (`None`: a nested table).
    pub sources: &'a [Option<ParaSource>],
    /// Compatibility mode 15 or later.
    pub mode15: bool,
}

/// A stacked line of a cell as its drawings are positioned against it, px from below the
/// row's top border: its top, pitch and paragraph's top.
#[derive(Debug, Clone, Copy)]
struct StackTop {
    at: (usize, usize),
    top: Px,
    pitch: Px,
    paragraph_top: Px,
}

/// Each stacked line's top, pitch and paragraph top, as the cell places them
/// (`_stack_tops`), by `(entry, line)`.
fn stack_tops(job: &CellJob<'_>, entries: &[CellBlock]) -> Vec<StackTop> {
    let (stack, _) = cell_stack(entries, job.styles.0, job.styles.1);
    let margin_top = Px::twips(job.cell.margins[0]);
    let mut out = Vec::new();
    let (mut line_y, mut paragraph_top) = (margin_top, margin_top);
    let mut before: Option<&CellBlock> = None;
    for placed in &stack {
        let Some(entry) = entries.get(placed.entry) else { continue };
        if placed.number == 0 {
            paragraph_top = match before {
                Some(b) => {
                    let same = entry.style == b.style;
                    line_y + Px::twips(if b.contextual && same { 0 } else { b.after })
                }
                None => line_y,
            };
        }
        let top = line_y + placed.above;
        out.push(StackTop {
            at: (placed.entry, placed.number),
            top,
            pitch: placed.pitch,
            paragraph_top,
        });
        line_y = top + placed.pitch + placed.below;
        before = Some(entry);
    }
    out
}

fn top_of(tops: &[StackTop], at: (usize, usize)) -> Option<StackTop> {
    tops.iter().find(|t| t.at == at).copied()
}

/// The frames a drawing on line `line` of entry `k` is positioned against as the cell is
/// laid out from its top (`wrap_cell`'s `frames_of`).
fn frames_of(
    job: &CellJob<'_>,
    entries: &[CellBlock],
    tops: &[StackTop],
    (k, line, source, run): (usize, usize, i64, usize),
) -> Option<Frames> {
    let StackTop {
        top,
        pitch,
        paragraph_top,
        ..
    } = top_of(tops, (k, line))?;
    let entry = entries.get(k)?;
    let Content::Paragraph(measured) = &entry.content else {
        return None;
    };
    let alignment = job
        .sources
        .get(k)
        .and_then(Option::as_ref)
        .and_then(|s| s.resolved.paragraph.get("jc").and_then(Value::str).map(str::to_owned));
    let character = crate::paint::drawings::character_px(
        measured,
        (line, entry.drawings.extra.get(line), alignment.as_deref()),
        (source, run),
        twips_px(job.edges.text_left),
    )?;
    let px = twip_px();
    let line_top = if line == 0 { paragraph_top } else { top };
    Some(job.edges.frames(
        &job.page,
        (Q::from_integer(0), 1, job.mode15),
        [
            paragraph_top.value() / px,
            line_top.value() / px,
            (top + pitch).value() / px,
            character.value() / px,
        ],
    ))
}

/// The drawings of a cell text goes around, positioned from the cell laid out without
/// them, and the bands of those it goes above and below.
struct Around {
    wraps: Vec<Wrap>,
    bands: Vec<Band>,
    foot: Px,
}

/// Lay a cell's text out around the drawings anchored in it that text wraps around
/// (`wrap_cell`), and say how far down the lowest of them reaches (px from below the row's
/// top border; `None` where there are none). `loose`, where given (a cell aligned
/// vertically), gets `(top, foot)` of every drawing text does not wrap around positioned
/// in the cell.
pub fn wrap_cell(
    job: &CellJob<'_>,
    entries: &mut [CellBlock],
    loose: Option<&mut Vec<(Px, Px)>>,
) -> Result<Option<Px>, Unsupported> {
    let tops = stack_tops(job, entries);
    let px = twip_px();
    let mut anchored: Vec<(usize, AnchorRef)> = Vec::new();
    for (k, entry) in entries.iter().enumerate().filter(|(_, e)| !e.hidden) {
        for a in &entry.drawings.anchors {
            if entry.drawings.approximated((a.run, a.index)).is_none() {
                anchored.push((k, a.clone()));
            }
        }
    }
    let wraps_in_cell = |a: &Anchor| CELL_WRAPS.contains(&a.wrap.as_deref().unwrap_or_default());
    if let Some(loose) = loose {
        for (k, a) in anchored
            .iter()
            .filter(|(_, a)| !wraps_in_cell(&a.anchor) && in_cell(&a.anchor, job.mode15))
        {
            let frames = frames_of(job, entries, &tops, (*k, a.line, a.source, a.run));
            if let Some(y) = frames.and_then(|f| vertical(&a.anchor, &f)) {
                let cy = int(emu_twips(a.anchor.extent.1));
                loose.push((Px::from(y * px), Px::from((y + cy) * px)));
            }
        }
    }
    let wrapping: Vec<&(usize, AnchorRef)> =
        anchored.iter().filter(|(_, a)| wraps_in_cell(&a.anchor)).collect();
    if wrapping.is_empty() {
        return Ok(None);
    }
    let around = position(job, entries, &tops, &wrapping);
    for k in 0..entries.len() {
        lay_around(job, entries, k, &around)?;
    }
    Ok(Some(around.foot))
}

/// Each drawing text wraps around positioned in the cell (`wrap_cell`'s first part): held
/// inside it, recorded on its paragraph, and as what lines keep off.
fn position(
    job: &CellJob<'_>,
    entries: &mut [CellBlock],
    tops: &[StackTop],
    wrapping: &[&(usize, AnchorRef)],
) -> Around {
    let px = twip_px();
    let mut out = Around {
        wraps: Vec::new(),
        bands: Vec::new(),
        foot: Px::ZERO,
    };
    for (k, a) in wrapping {
        let frames = frames_of(job, entries, tops, (*k, a.line, a.source, a.run));
        let found = frames.and_then(|f| Some((horizontal(&a.anchor, &f)?, vertical(&a.anchor, &f)?)));
        let Some(entry) = entries.get_mut(*k) else { continue };
        let Some((mut x, mut y)) = found else {
            entry
                .drawings
                .approximate
                .push(((a.run, a.index), "cannot be positioned".to_owned()));
            continue;
        };
        if !job.mode15 {
            x = (x + Q::new(1, 2)).floor();
        }
        let anchor = &a.anchor;
        let (cx, cy) = (int(emu_twips(anchor.extent.0)), int(emu_twips(anchor.extent.1)));
        x = (x.min(job.edges.box_right - cx)).max(job.edges.box_left);
        y = y.max(int(job.cell.margins[0]));
        let eb = int(emu_twips(anchor.effect[3]));
        let [dt, db, _, _] = anchor.distance.map(|v| int(emu_twips(v)) * px);
        entry.drawings.positioned.push(Positioned {
            at: (a.source, a.run, a.index),
            x: Px::from(x * px),
            y: Px::from(y * px),
        });
        let foot = Px::from((y + cy + eb) * px + if job.mode15 { db } else { Q::from_integer(0) });
        out.foot = out.foot.max(foot);
        let key = DrawingKey {
            block: entry.block,
            source: a.source,
            run: a.run,
            index: a.index,
        };
        if anchor.wrap.as_deref() == Some("wrapTopAndBottom") {
            out.bands.push(Band {
                top: Px::from(y * px - dt),
                bottom: Px::from((y + cy) * px + db),
                y: Px::from(y * px),
                key,
                span: None,
            });
            continue;
        }
        out.wraps.push(side_wrap(anchor, (x, y), key, job.mode15));
    }
    out
}

/// A drawing text goes beside in a cell, at `(x, y)` twips (`wrap_cell`'s `Wrap`): its box
/// with its effect extent and distances, or its wrap polygon (below mode 15 without its
/// distances above and below).
fn side_wrap(anchor: &Anchor, (x, y): (Q, Q), key: DrawingKey, mode15: bool) -> Wrap {
    let px = twip_px();
    let (cx, cy) = (int(emu_twips(anchor.extent.0)), int(emu_twips(anchor.extent.1)));
    let [el, et, er, eb] = anchor.effect.map(|v| int(emu_twips(v)));
    let [mut dt, mut db, dl, dr] = anchor.distance.map(|v| int(emu_twips(v)) * px);
    let tight = matches!(anchor.wrap.as_deref(), Some("wrapTight" | "wrapThrough"));
    let polygon: Option<Vec<(Q, Q)>> = anchor.polygon.as_ref().filter(|p| tight && !p.is_empty()).map(|points| {
        points
            .iter()
            .map(|(pa, pb)| {
                let w = emu_twips(anchor.extent.0);
                let h = emu_twips(anchor.extent.1);
                (
                    (x + int(i64::try_from(i128::from(w) * i128::from(*pa) / 21600).unwrap_or(0))) * px,
                    (y + int(i64::try_from(i128::from(h) * i128::from(*pb) / 21600).unwrap_or(0))) * px,
                )
            })
            .collect()
    });
    if polygon.is_some() && !mode15 {
        dt = Q::from_integer(0);
        db = Q::from_integer(0);
    }
    let (top, bottom) = match &polygon {
        Some(points) => (
            points.iter().map(|p| p.1).min().unwrap_or_default() - dt,
            points.iter().map(|p| p.1).max().unwrap_or_default() + db,
        ),
        None => ((y - et) * px - dt, (y + cy + eb) * px + db),
    };
    Wrap {
        top,
        bottom,
        left: (x - el) * px,
        right: (x + cx + er) * px,
        dist_left: dl,
        dist_right: dr,
        side: anchor.wrap_text.as_deref().unwrap_or("bothSides").to_owned(),
        y: y * px,
        polygon,
        dist_top: dt,
        dist_bottom: db,
        half_up: false,
        text_shift: None,
        drop_cap: None,
        table: None,
        drawing: Some((key, x * px)),
    }
}

/// Entry `k`'s lines laid out again around the drawings, line by line from its first
/// (`wrap_cell`'s second part): each below the bands it would reach into, then broken
/// across what the drawings beside it leave, or below them where no segment takes text.
fn lay_around(job: &CellJob<'_>, entries: &mut [CellBlock], k: usize, around: &Around) -> Result<(), Unsupported> {
    let Some(source) = job.sources.get(k).and_then(Option::as_ref) else {
        return Ok(());
    };
    if entries.get(k).is_none_or(|e| e.hidden) {
        return Ok(());
    }
    let breaker = Breaker {
        document: job.document,
        faces: job.faces,
        locale: job.locale,
    };
    let column = Column {
        exact: twips_px(job.edges.text_left).value(),
        start: job.cell_box.text_left.value(),
        width: {
            let (n, d) = ooxml_text::twips(job.cell_box.budget).parts();
            Q::new(n, d)
        },
        mode15: job.mode15,
        width_px: Px::twips(job.cell_box.budget).value(),
    };
    let mut number = 0;
    loop {
        let tops = stack_tops(job, entries);
        let Some(entry) = entries.get(k) else { break };
        let Content::Paragraph(measured) = &entry.content else { break };
        let count = measured.measured.lines.len();
        if number >= count {
            break;
        }
        let Some(stacked) = top_of(&tops, (k, number)) else { break };
        let top = stacked.top - entry.drawings.drop(number);
        let after = Px::twips(entry.after);
        let current = Lines::Again(Arc::new(Rebroken {
            measured: Arc::clone(measured),
            extra: entry.drawings.extra.clone(),
            notes: Vec::new(),
            anchors: entry.drawings.anchors.clone(),
            plain_from: 0,
        }));
        let placed = place_line(&breaker, source, &current, (number, top, after), (around, &column), job.mode15)?;
        let (y, line, extra) = placed;
        let Some(entry) = entries.get_mut(k) else { break };
        entry.drawings.drops.retain(|(l, _)| *l != number);
        if y != top {
            entry.drawings.drops.push((number, y - top));
        }
        let pieces = &current.measured().measured.pieces;
        let n = pieces.len();
        let old = current.measured().measured.lines.get(number).copied();
        if extra.wrapped() || old.is_none_or(|o| (o.start, o.end) != (line.start, line.end)) {
            let mut lines: Vec<(Line, LineExtra)> = current
                .measured()
                .measured
                .lines
                .iter()
                .zip(entry.drawings.extra.iter().cloned().chain(std::iter::repeat(LineExtra::default())))
                .take(number)
                .map(|(l, e)| (*l, e))
                .collect();
            let ends_newline = line.forced && n > 0 && pieces.get(n - 1).is_some_and(|p| p.ch == '\n');
            let tail = if line.end < n || ends_newline {
                breaker.rest(source, pieces, line.end, false, None)
            } else {
                Vec::new()
            };
            lines.push((line, extra));
            lines.extend(tail);
            let again = breaker
                .set_lines(source, &current, lines, 0)
                .ok_or_else(|| Unsupported("no face metrics in a cell".into()))?;
            entry.pitches = again.measured.heights.iter().map(|h| h.pitch).collect();
            entry.drawings.extra = again.extra.clone();
            entry.drawings.anchors = again.anchors.clone();
            entry.content = Content::Paragraph(Arc::clone(&again.measured));
        }
        number += 1;
    }
    Ok(())
}

/// Where line `number` of a cell paragraph goes among the drawings, from its stacked
/// `top`: below the bands it would reach into, then beside the drawings it reaches into,
/// or at the foot of those it cannot go beside (a polygon's: a line's pitch lower at a
/// time); its top, the line and what it carries.
fn place_line(
    breaker: &Breaker<'_>,
    source: &ParaSource,
    current: &Lines<'_>,
    (number, top, after): (usize, Px, Px),
    (around, column): (&Around, &Column),
    mode15: bool,
) -> Result<(Px, Line, LineExtra), Unsupported> {
    let measured = current.measured();
    let pieces = &measured.measured.pieces;
    let n = pieces.len();
    let first = number == 0;
    let start = measured.measured.lines.get(number).map_or(n, |l| l.start);
    let plain = if start >= n {
        Line {
            start: n,
            end: n,
            forced: false,
            emergency: false,
            hyphenated: false,
        }
    } else {
        let hyphens = u32::from(start > 0 && pieces.get(start - 1).is_some_and(|p| p.auto));
        ooxml_text::break_line(pieces, start, &source.geometry, first, hyphens)
    };
    let height = breaker
        .height(source, pieces, &plain, first)
        .ok_or_else(|| Unsupported("no face metrics in a cell".into()))?;
    let last = number + 1 == measured.measured.lines.len();
    let reach = if mode15 {
        height.pitch - height.multiple_extra()
    } else {
        height.pitch + if last { after } else { Px::ZERO }
    };
    let mut y = top;
    if !around.bands.is_empty() {
        let tall = height.pitch + if last && !mode15 { after } else { Px::ZERO };
        y = clear_of_bands(y, tall, &around.bands);
    }
    let (mut line, mut extra) = (plain, LineExtra::default());
    for _ in 0..4096 {
        if around.wraps.is_empty() {
            break;
        }
        let text = y + height.pitch - height.multiple_extra();
        let band = (y.value(), (y + reach).value(), Some(text.value()));
        let (segs, beside) = segments(&around.wraps, band, column);
        let Some(segs) = segs else {
            line = plain;
            break;
        };
        if let Some((found, parts)) = break_across(pieces, start, &segs, &source.geometry, first) {
            line = found;
            extra = LineExtra {
                segments: parts,
                ..LineExtra::default()
            };
            break;
        }
        let below = beside
            .iter()
            .map(|w| {
                if w.polygon.is_some() {
                    (y + height.pitch).value()
                } else {
                    w.bottom
                }
            })
            .min();
        match below {
            Some(below) if Px::from(below) > y => y = Px::from(below),
            _ => break,
        }
    }
    Ok((y, line, extra))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_word_model::model::AnchorPosition;

    fn anchor(h: (&str, Option<i64>, Option<&str>), v: (&str, Option<i64>, Option<&str>), wrap: &str) -> Anchor {
        let position = |(relative, offset, align): (&str, Option<i64>, Option<&str>)| AnchorPosition {
            relative: Some(Text::from(relative)),
            offset,
            align: align.map(Text::from),
        };
        Anchor {
            h: position(h),
            v: position(v),
            wrap: Some(Text::from(wrap)),
            ..Anchor::default()
        }
    }

    #[test]
    fn a_drawing_is_in_its_cell_in_mode_15_or_by_its_anchor() {
        let page = anchor(("page", Some(0), None), ("page", Some(0), None), "wrapSquare");
        assert!(in_cell(&page, true));
        assert!(!in_cell(&page, false));
        let character = anchor(("character", Some(0), None), ("page", Some(0), None), "wrapSquare");
        assert!(in_cell(&character, false));
        let laid = Anchor {
            layout_in_cell: true,
            ..page.clone()
        };
        assert!(in_cell(&laid, false));
    }

    #[test]
    fn positions_no_probe_measured_are_named() {
        let measured = anchor(("column", Some(100), None), ("paragraph", None, Some("center")), "wrapSquare");
        assert_eq!(position_unmodelled(&measured, true), None);
        let inside = anchor(("insideMargin", None, Some("left")), ("paragraph", Some(0), None), "wrapSquare");
        assert_eq!(position_unmodelled(&inside, true).as_deref(), Some("insideMargin left"));
        // `character` is measured only in the cell.
        let character = anchor(("character", Some(0), None), ("page", Some(0), None), "wrapNone");
        assert_eq!(position_unmodelled(&character, true), None);
        let simple = Anchor {
            simple: Some((0, 0)),
            ..measured
        };
        assert_eq!(position_unmodelled(&simple, true).as_deref(), Some("simplePos"));
    }

    #[test]
    fn wraps_not_measured_in_a_cell_are_laid_out_as_none() {
        let against_page = anchor(("page", Some(0), None), ("page", Some(0), None), "wrapSquare");
        let run = Run {
            anchors: vec![against_page].into(),
            ..Run::default()
        };
        let paragraph = Paragraph {
            runs: vec![run].into(),
            ..Paragraph::default()
        };
        assert_eq!(
            unmodelled(&paragraph, false).as_deref(),
            Some("a floating drawing text wraps around in a cell, positioned against the page")
        );
        assert_eq!(unmodelled(&paragraph, true), None);
        let laid = unwrapped(&paragraph);
        let wraps: Vec<&str> = laid
            .runs
            .iter()
            .flat_map(|r| r.anchors.iter().filter_map(|a| a.wrap.as_deref()))
            .collect();
        assert_eq!(wraps, ["wrapNone"]);
    }
}
