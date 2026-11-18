//! Owns: where each piece of a line is drawn (docx2svg's `layout.line_positions`): the pen
//! moved by each piece's width and its kern pair, a tab to its stop -- the text after a
//! right, centre or decimal stop moved to end at, centre on or put its decimal separator at
//! the stop, never left of the tab -- then the line moved by its paragraph's alignment:
//! centred, right, distributed, justified (the slack over the spaces after the last tab),
//! or a justified line past the edge squeezed. Each tab's pen before and after it is kept
//! for its leader. A line broken again carries its own setting ([`wrapped_positions`]):
//! one broken for another of unequal columns is set in that column, one beside drawings
//! segment by segment.
//!
//! Must not: break lines (the text engine has), or know where the line is on the page.

use crate::rebreak::LineExtra;
use crate::wrap::{segment_geometry, units};
use ooxml_text::{next_tab, Geometry, Line, Piece, PieceKind, TabAlign, Units};
use std::collections::BTreeMap;

/// A tab on a line: the pen at it and after it, and whether its stop is a default one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabSpan {
    /// The pen at the tab.
    pub from: Units,
    /// The pen after it.
    pub to: Units,
    /// Where its stop is.
    pub stop: Units,
    /// The stop's alignment.
    pub align: TabAlign,
    /// A default stop (no leader).
    pub default: bool,
}

/// A line's pieces' pen positions, layout units from the column's left edge, and its tabs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Positions {
    /// Piece index to pen x.
    pub at: BTreeMap<usize, Units>,
    /// Piece index of each tab to where it goes, before the line is aligned.
    pub tabs: BTreeMap<usize, TabSpan>,
}

/// How a line is set: its paragraph's alignment and where it is in the paragraph.
#[derive(Debug, Clone, Copy)]
pub struct Setting<'a> {
    /// `w:jc` (`left` where none).
    pub alignment: Option<&'a str>,
    /// The paragraph's first line.
    pub first_line: bool,
    /// The paragraph's last line.
    pub last_line: bool,
    /// The decimal separator a decimal stop lines up.
    pub decimal: char,
}

fn advance(piece: &Piece) -> Units {
    piece.width + piece.kern_next
}

/// The width of the pieces `start..end`, tabs, breaks and soft hyphens left out.
fn segment_width(items: &[Piece], start: usize, end: usize) -> Units {
    let mut total = Units::ZERO;
    for piece in items.get(start..end).unwrap_or_default() {
        if !matches!(piece.kind, PieceKind::Tab | PieceKind::Break | PieceKind::SoftHyphen) {
            total += advance(piece);
        }
    }
    total
}

fn is(items: &[Piece], k: usize, kind: PieceKind) -> bool {
    items.get(k).is_some_and(|p| p.kind == kind)
}

/// Where the text after a pulling tab at piece `j` starts: before its stop by as much of
/// it as the stop pulls back, never left of the pen.
fn pulled(items: &[Piece], j: usize, end: usize, stop: Units, align: TabAlign, decimal: char) -> Units {
    let next = (j + 1..end).find(|&k| is(items, k, PieceKind::Tab)).unwrap_or(end);
    let mut content_end = next;
    while content_end > j + 1
        && (is(items, content_end - 1, PieceKind::Space) || is(items, content_end - 1, PieceKind::Break))
    {
        content_end -= 1;
    }
    match align {
        TabAlign::Decimal => {
            let point = (j + 1..content_end)
                .find(|&k| items.get(k).is_some_and(|p| p.ch == decimal))
                .unwrap_or(content_end);
            stop - segment_width(items, j + 1, point)
        }
        TabAlign::Right => stop - segment_width(items, j + 1, content_end),
        _ => {
            stop - segment_width(items, j + 1, content_end)
                .scale(1, 2)
                .unwrap_or(Units::ZERO)
        }
    }
}

/// The pieces of `line` placed from the line's start, before it is aligned.
fn placed(items: &[Piece], line: &Line, geometry: &Geometry, setting: &Setting<'_>) -> Positions {
    let mut x = if setting.first_line {
        geometry.first_start
    } else {
        geometry.start
    };
    let mut out = Positions::default();
    let mut j = line.start;
    while j < line.end {
        let Some(piece) = items.get(j) else { break };
        if piece.kind == PieceKind::Tab {
            out.at.insert(j, x);
            let from = x;
            let (stop, align, default) = next_tab(x, geometry, setting.first_line);
            if matches!(align, TabAlign::Right | TabAlign::Center | TabAlign::Decimal) {
                x = x.max(pulled(items, j, line.end, stop, align, setting.decimal));
            } else {
                x = stop;
            }
            out.tabs.insert(
                j,
                TabSpan {
                    from,
                    to: x,
                    stop,
                    align,
                    default,
                },
            );
            j += 1;
            continue;
        }
        out.at.insert(j, x);
        if !matches!(piece.kind, PieceKind::Break | PieceKind::SoftHyphen) {
            x += advance(piece);
        }
        j += 1;
    }
    out
}

/// The line's last piece that is drawn: not a space, a break or a soft hyphen it does not
/// end at.
fn last_drawn(items: &[Piece], line: &Line) -> Option<usize> {
    (line.start..line.end)
        .rev()
        .find(|&k| match items.get(k).map(|p| p.kind) {
            Some(PieceKind::Space | PieceKind::Break) | None => false,
            Some(PieceKind::SoftHyphen) => line.hyphenated,
            Some(_) => true,
        })
}

fn moved(out: &mut Positions, by: Units) {
    for x in out.at.values_mut() {
        *x += by;
    }
}

/// Each piece's pen x on `line`, from the column's left edge, layout units.
pub fn line_positions(items: &[Piece], line: &Line, geometry: &Geometry, setting: &Setting<'_>) -> Positions {
    let mut out = placed(items, line, geometry, setting);
    let Some(last) = last_drawn(items, line) else {
        return out;
    };
    let (Some(&pen), Some(piece)) = (out.at.get(&last), items.get(last)) else {
        return out;
    };
    let text_end = pen
        + match piece.kind {
            PieceKind::SoftHyphen => piece.hyphen,
            PieceKind::Tab => Units::ZERO,
            _ => piece.width,
        };
    let slack = geometry.right - text_end;
    let positive = slack > Units::ZERO;
    let alignment = setting.alignment.unwrap_or("left");
    match alignment {
        "center" if positive => {
            let half = slack.scale(1, 2).map_or(0, Units::floor);
            moved(&mut out, Units::int(i64::try_from(half).unwrap_or(0)));
            out
        }
        "right" | "end" if positive => {
            moved(&mut out, slack);
            out
        }
        "distribute" if positive && (setting.last_line || line.forced) => distributed(out, last, slack),
        _ => {
            let squeezed = alignment == "both" && slack < Units::ZERO && geometry.squeeze;
            let stretched =
                matches!(alignment, "both" | "distribute") && positive && !setting.last_line && !line.forced;
            if squeezed || stretched {
                justified(items, line, out, last, slack)
            } else {
                out
            }
        }
    }
}

/// A distributed paragraph's last line: the slack spread evenly between its glyphs.
fn distributed(mut out: Positions, last: usize, slack: Units) -> Positions {
    let drawn: Vec<usize> = out.at.keys().copied().filter(|k| *k <= last).collect();
    let gaps = drawn.len().saturating_sub(1);
    if gaps == 0 {
        return out;
    }
    let share = slack.scale(1, i128::try_from(gaps).unwrap_or(1)).unwrap_or(Units::ZERO);
    for (k, x) in &mut out.at {
        let place = drawn.iter().position(|d| d == k).unwrap_or(gaps).min(gaps);
        *x += share * i64::try_from(place).unwrap_or(0);
    }
    out
}

/// A justified line: the slack over the spaces after its last tab, each glyph after a space
/// moved by its share; squeezed, the overrun taken from them in proportion to their widths,
/// each shift rounded to the unit.
fn justified(items: &[Piece], line: &Line, mut out: Positions, last: usize, slack: Units) -> Positions {
    let tab = (line.start..=last)
        .rev()
        .find(|&k| is(items, k, PieceKind::Tab))
        .map_or(line.start, |k| k + 1);
    let spaces: Vec<usize> = (tab..last).filter(|&k| is(items, k, PieceKind::Space)).collect();
    if spaces.is_empty() {
        return out;
    }
    let count = i128::try_from(spaces.len()).unwrap_or(1);
    let share = slack.scale(1, count).unwrap_or(Units::ZERO);
    let total_width = spaces
        .iter()
        .filter_map(|k| items.get(*k))
        .fold(Units::ZERO, |sum, p| sum + p.width);
    let mut extra = Units::ZERO;
    let mut before = Units::ZERO;
    for (k, x) in &mut out.at {
        if slack < Units::ZERO {
            let (sn, sd) = slack.parts();
            let (bn, bd) = before.parts();
            let (tn, td) = total_width.parts();
            // floor(slack * before / total + 1/2), exact.
            let shift = Units::ratio(sn * bn * td, sd * bd * tn)
                .and_then(|v| (v + Units::ratio(1, 2).unwrap_or(Units::ZERO)).checked_div(Units::int(1)))
                .map_or(0, Units::floor);
            *x += Units::int(i64::try_from(shift).unwrap_or(0));
        } else {
            *x += extra;
        }
        if spaces.contains(k) {
            extra += share;
            if let Some(p) = items.get(*k) {
                before += p.width;
            }
        }
    }
    out
}

/// [`line_positions`] of a line as the paginator broke it (`wrapped_positions`): set in the
/// column it was broken for, or -- beside drawings -- each segment set in its segment as a
/// line is in the column (the paragraph's last segment as its last line) and moved by the
/// segment's shift.
pub fn wrapped_positions(
    items: &[Piece],
    line: &Line,
    geometry: &Geometry,
    extra: Option<&LineExtra>,
    setting: &Setting<'_>,
) -> Positions {
    let geometry = extra
        .and_then(|e| e.column.as_ref())
        .map_or(geometry, |c| c.geometry.as_ref());
    let segments = extra.map_or(&[][..], |e| e.segments.as_slice());
    if segments.is_empty() {
        return line_positions(items, line, geometry, setting);
    }
    let mut out = Positions::default();
    for (k, part) in segments.iter().enumerate() {
        let here = segment_geometry(geometry, &part.segment, part.first);
        let set = Setting {
            first_line: part.first,
            last_line: setting.last_line && k + 1 == segments.len(),
            ..*setting
        };
        let found = line_positions(items, &part.line, &here, &set);
        let shift = units(part.segment.shift);
        out.at.extend(found.at.into_iter().map(|(j, x)| (j, x + shift)));
        out.tabs.extend(found.tabs.into_iter().map(|(j, t)| {
            (
                j,
                TabSpan {
                    from: t.from + shift,
                    to: t.to + shift,
                    stop: t.stop + shift,
                    ..t
                },
            )
        }));
    }
    out
}
