//! Owns: text beside a floating drawing -- `wrapSquare`, `wrapTight`, `wrapThrough`
//! (docx2svg's `wrap`, every rule measured): which lines are beside a drawing and what it
//! keeps them off ([`Wrap`], [`blocked`]: its box widened by its distances, or its wrap
//! polygon across the line's text), what is left of the column in segments of at least
//! 360 twips ([`segments`]), each segment's text start on Word's pixel rounding, and a
//! line broken across the segments, a segment whose first word does not fit passed over
//! ([`break_across`]).
//!
//! Must not: position a drawing (`crate::floating` does) or decide which page a line goes
//! on (the paginator does).

use crate::px::Px;
use ooxml_base::Q;
use ooxml_text::{break_line, Geometry, Line, Piece, PieceKind, Units};

/// The narrowest segment that takes text, twips.
pub const MIN_SEGMENT_TWIPS: i64 = 360;

fn twip_px() -> Q {
    Q::new(300, 1440)
}

/// Layout units per device px.
fn units_per_px() -> Q {
    Q::from_integer(1) / Px::layout_unit()
}

/// A drawing text wraps beside on one page, device px from the page's top left: its box
/// widened by `distT` and `distB` (`top`, `bottom`), its box with its effect extent
/// (`left`, `right`) and what text keeps further off across; its wrap polygon in page px
/// (`wrapTight`, `wrapThrough`); its `wrapText`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrap {
    /// The top of what it keeps lines off.
    pub top: Q,
    /// Its foot.
    pub bottom: Q,
    /// Its box's left edge.
    pub left: Q,
    /// Its box's right edge.
    pub right: Q,
    /// What text keeps off left of it.
    pub dist_left: Q,
    /// And right of it.
    pub dist_right: Q,
    /// `wrapText`: `bothSides`, `left`, `right` or `largest`.
    pub side: String,
    /// The drawing's top as positioned.
    pub y: Q,
    /// Its wrap polygon, page px.
    pub polygon: Option<Vec<(Q, Q)>>,
    /// `distT` (mode 15: the polygon's band is widened by it).
    pub dist_top: Q,
    /// `distB`.
    pub dist_bottom: Q,
    /// The text right of it rounds a half pixel up (a floating table's) where a drawing's
    /// rounds it down.
    pub half_up: bool,
    /// How far the text right of it moves from its edge, where it is not the drawings'
    /// rounding (a drop cap's frame).
    pub text_shift: Option<Q>,
}

/// The least and greatest x of the region `points` encloses between heights `y0` and
/// `y1`, or `None` where the band misses it (`polygon_span`).
pub fn polygon_span(points: &[(Q, Q)], y0: Q, y1: Q) -> Option<(Q, Q)> {
    let mut closed: Vec<(Q, Q)> = points.to_vec();
    if let (Some(first), Some(last)) = (closed.first().copied(), closed.last().copied()) {
        if first != last {
            closed.push(first);
        }
    }
    let mut xs: Vec<Q> = Vec::new();
    for pair in closed.windows(2) {
        let [(ax, ay), (bx, by)] = [pair.first().copied(), pair.get(1).copied()].map(|p| p.unwrap_or_default());
        let (lo, hi) = if ay <= by { (ay, by) } else { (by, ay) };
        if hi < y0 || lo > y1 {
            continue;
        }
        if ay == by {
            xs.extend([ax, bx]);
            continue;
        }
        for y in [lo.max(y0), hi.min(y1)] {
            xs.push(ax + (bx - ax) * (y - ay) / (by - ay));
        }
    }
    let lo = xs.iter().copied().min()?;
    let hi = xs.iter().copied().max()?;
    Some((lo, hi))
}

/// What of the band from `y0` to `y1` `wrap` keeps text off, page px (distances
/// included), or `None` where the line is not beside it; a wrap polygon is read over the
/// line's text only, down to `text` (`blocked`).
pub fn blocked(wrap: &Wrap, y0: Q, y1: Q, text: Option<Q>) -> Option<(Q, Q)> {
    if !(y0 < wrap.bottom && y1 > wrap.top) {
        return None;
    }
    let Some(polygon) = &wrap.polygon else {
        return Some((wrap.left - wrap.dist_left, wrap.right + wrap.dist_right));
    };
    let span = polygon_span(polygon, y0 - wrap.dist_bottom, text.unwrap_or(y1) + wrap.dist_top)?;
    Some((span.0 - wrap.dist_left, span.1 + wrap.dist_right))
}

/// A piece of a line beside drawings, layout units from the text column's left edge:
/// from `left` to `right`; its text set out from `left` (or the indent) and moved right
/// by `shift`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Its left edge.
    pub left: Q,
    /// Its right edge.
    pub right: Q,
    /// The pixel rounding its text moves by.
    pub shift: Q,
    /// `left` is a drawing's edge rather than the column's start.
    pub edge: bool,
    /// Its tab stops are measured from `left` (below mode 15).
    pub tabs_here: bool,
}

/// Where the page's text column is: its left margin's exact edge and its start as drawn
/// (device px), its width in layout units and in exact px, the mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    /// The left margin's exact edge.
    pub exact: Q,
    /// The column's start as drawn.
    pub start: Q,
    /// Its width, layout units.
    pub width: Q,
    /// Compatibility mode 15 or later.
    pub mode15: bool,
    /// Its width, exact px.
    pub width_px: Q,
}

/// One free span of the column: from `left` to `right` px from the margin's exact edge,
/// and the drawing whose side `left` is (`None`: the column's start).
type Free<'w> = (Q, Q, Option<&'w Wrap>);

/// What `wraps` leave of the column from `y0` down to `y1`, left to right, and whether
/// the band is beside any (`free_spans`).
fn free_spans<'w>(wraps: &'w [Wrap], band: (Q, Q, Option<Q>), column: &Column) -> (Vec<Free<'w>>, Vec<&'w Wrap>) {
    let (y0, y1, text) = band;
    let mut beside: Vec<&Wrap> = Vec::new();
    let mut spans: Vec<(Q, Q, &Wrap)> = Vec::new();
    let far = Q::from_integer(1_000_000_000);
    for wrap in wraps {
        let Some((lo, hi)) = blocked(wrap, y0, y1, text) else {
            continue;
        };
        beside.push(wrap);
        let (mut lo, mut hi) = (lo - column.exact, hi - column.exact);
        let side = if wrap.side == "largest" {
            if lo >= column.width_px - hi {
                "left"
            } else {
                "right"
            }
        } else {
            wrap.side.as_str()
        };
        match side {
            "left" => hi = far,
            "right" => lo = -far,
            _ => {}
        }
        spans.push((lo, hi, wrap));
    }
    spans.sort_by_key(|s| (s.0, s.1));
    let mut free: Vec<Free<'_>> = vec![(Q::from_integer(0), column.width_px, None)];
    for (lo, hi, wrap) in spans {
        let mut out = Vec::new();
        for (a, b, edge) in free {
            if hi <= a || lo >= b {
                out.push((a, b, edge));
                continue;
            }
            if lo > a {
                out.push((a, lo, edge));
            }
            if hi < b {
                out.push((hi, b, Some(wrap)));
            }
        }
        free = out;
    }
    (free, beside)
}

fn half_up(x: Q) -> Q {
    (x + Q::new(1, 2)).floor()
}

fn half_down(x: Q) -> Q {
    (x - Q::new(1, 2)).ceil()
}

/// The segments a line from `y0` down to `y1` has beside `wraps`, left to right, and the
/// drawings it is beside; `None` where it is beside none (the whole column is its).
pub fn segments<'w>(wraps: &'w [Wrap], band: (Q, Q, Option<Q>), column: &Column) -> (Option<Vec<Segment>>, Vec<&'w Wrap>) {
    let (free, beside) = free_spans(wraps, band, column);
    if beside.is_empty() {
        return (None, beside);
    }
    let mut out = Vec::new();
    for (a, b, edge) in free {
        if (b - a) / twip_px() < Q::from_integer(i128::from(MIN_SEGMENT_TWIPS)) {
            continue;
        }
        let shift = match edge {
            None => Q::from_integer(0),
            Some(edge) => {
                let page = column.exact + a;
                let at = column.start + a;
                match edge.text_shift {
                    Some(shift) => shift,
                    None if column.mode15 => {
                        if page == at {
                            Q::from_integer(0)
                        } else if edge.half_up {
                            half_up(page) - half_up(at)
                        } else {
                            half_up(page) - half_down(at)
                        }
                    }
                    None if edge.half_up => half_up(page) - at,
                    None => half_down(page) - at,
                }
            }
        };
        out.push(Segment {
            left: a * units_per_px(),
            right: b * units_per_px(),
            shift: shift * units_per_px(),
            edge: edge.is_some(),
            tabs_here: edge.is_some() && !column.mode15,
        });
    }
    (Some(out), beside)
}

/// A rational as the text engine's units.
fn units(q: Q) -> Units {
    Units::ratio(*q.numer(), *q.denom()).unwrap_or(Units::ZERO)
}

fn q_of(u: Units) -> Q {
    let (n, d) = u.parts();
    Q::new(n, d)
}

/// `geometry` for the text of one segment: from the segment's left edge or the indent,
/// whichever is further right, the first line's indent moving it from there (a hanging
/// one no further left than the edge); to the segment's right edge or the indent's
/// (`segment_geometry`).
pub fn segment_geometry(geometry: &Geometry, segment: &Segment, first: bool) -> Geometry {
    let right = units(segment.right.min(q_of(geometry.right)));
    if !segment.edge {
        return Geometry {
            right,
            ..geometry.clone()
        };
    }
    let edge = segment.left;
    let start = edge.max(q_of(geometry.start));
    let first_start = if first {
        edge.max(start + q_of(geometry.first_start) - q_of(geometry.start))
    } else {
        start
    };
    Geometry {
        first_start: units(first_start),
        start: units(start),
        right,
        hanging_stop: None,
        tab_origin: if segment.tabs_here { units(edge) } else { Units::ZERO },
        ..geometry.clone()
    }
}

/// Whether a line broken in a segment keeps within it: not a word broken for want of
/// room, and its first piece not past its edge (`fits_in`).
fn fits_in(items: &[Piece], line: &Line, geometry: &Geometry, first: bool) -> bool {
    if line.emergency {
        return false;
    }
    if line.end == line.start {
        return true;
    }
    let x = if first { geometry.first_start } else { geometry.start };
    let Some(piece) = items.get(line.start) else { return true };
    if matches!(piece.kind, PieceKind::Space | PieceKind::Break | PieceKind::Tab | PieceKind::SoftHyphen) {
        return true;
    }
    x + piece.width <= geometry.right
}

/// One segment's part of a wrapped line: its pieces, the segment, and whether the
/// paragraph's first line's indent applies to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentLine {
    /// The pieces it holds.
    pub line: Line,
    /// The segment.
    pub segment: Segment,
    /// The first line's indent applies.
    pub first: bool,
}

/// One line from piece `start` across `segments`: each takes as much as fits in it, in
/// order, a segment its next word does not fit in passed over; `None` where no segment
/// takes anything (`break_across`).
pub fn break_across(
    items: &[Piece],
    start: usize,
    segments: &[Segment],
    geometry: &Geometry,
    first: bool,
) -> Option<(Line, Vec<SegmentLine>)> {
    let n = items.len();
    if start >= n {
        let segment = segments.first()?.clone();
        let line = Line {
            start,
            end: start,
            forced: false,
            emergency: false,
            hyphenated: false,
        };
        return Some((line, vec![SegmentLine { line, segment, first }]));
    }
    let mut placed: Vec<SegmentLine> = Vec::new();
    let mut i = start;
    for segment in segments {
        if i >= n {
            break;
        }
        let here = first && placed.is_empty();
        let g = segment_geometry(geometry, segment, here);
        if g.right <= if here { g.first_start } else { g.start } {
            continue;
        }
        let hyphens = u32::from(i > 0 && items.get(i - 1).is_some_and(|p| p.auto));
        let line = break_line(items, i, &g, here, hyphens);
        if !fits_in(items, &line, &g, here) {
            continue;
        }
        i = line.end;
        let forced = line.forced;
        placed.push(SegmentLine {
            line,
            segment: segment.clone(),
            first: here,
        });
        if forced {
            break;
        }
    }
    let (first_line, last_line) = (placed.first()?.line, placed.last()?.line);
    let whole = Line {
        start: first_line.start,
        end: last_line.end,
        forced: last_line.forced,
        emergency: false,
        hyphenated: last_line.hyphenated,
    };
    Some((whole, placed))
}
