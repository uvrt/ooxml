//! Owns: the one line-breaking loop -- greedy first fit, exact, inclusive (`x <= right`),
//! spaces at a line's end not counted -- with Word's rules as data: where a line may end
//! ([`may_break`]), the justified squeeze of mode 15 ([`squeezes`]), tab stops past the
//! edge, and automatic hyphens ([`auto_hyphen_allowed`]); docx2svg's `linebreak.break_line`,
//! `break_pieces` and `break_rest`.
//!
//! Must not: measure (pieces come measured), or know a paragraph's properties.

use crate::geometry::{next_tab, Geometry};
use crate::pieces::{Piece, PieceKind};
use crate::units::Units;

/// Characters a line may break after, between two others: hyphen-minus, en and em dash.
pub const BREAK_AFTER: [char; 3] = ['-', '\u{2013}', '\u{2014}'];

/// One line: pieces `start..end`, and why it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Line {
    /// Its first piece.
    pub start: usize,
    /// One past its last.
    pub end: usize,
    /// Ended by a break rather than by the edge or the paragraph's end.
    pub forced: bool,
    /// Broken inside a word because the word alone did not fit.
    pub emergency: bool,
    /// Ended at a soft hyphen, which is then drawn.
    pub hyphenated: bool,
}

impl Line {
    fn new(start: usize, end: usize) -> Line {
        Line {
            start,
            end,
            forced: false,
            emergency: false,
            hyphenated: false,
        }
    }
}

/// Whether a line may end between two adjacent pieces.
pub fn may_break(before: &Piece, after: &Piece) -> bool {
    if after.kind == PieceKind::Space {
        return false;
    }
    matches!(before.kind, PieceKind::Space | PieceKind::SoftHyphen)
        || after.kind == PieceKind::Tab
        || (before.kind == PieceKind::Glyph && BREAK_AFTER.contains(&before.ch) && before.source >= 0)
}

/// How far a space lets a justified line in mode 15 run past the edge: a quarter of its
/// width, rounded half up to the unit.
pub fn squeeze(space: &Piece) -> Units {
    let quarter = space.width().scale(1, 4).unwrap_or(Units::ZERO) + Units::ratio(1, 2).unwrap_or(Units::ZERO);
    Units::ratio(quarter.floor(), 1).unwrap_or(Units::ZERO)
}

/// An edge: a length, or none (a custom left stop past the edge: nothing after it wraps).
type Limit = Option<Units>;

/// Whether Word keeps a word `over` units past the edge on a justified line in mode 15,
/// compressing the line's `spaces` spaces: no space shrinks by more than a quarter, and
/// each shrinks by at most half of what each would grow were the word sent down.
pub fn squeezes(over: Units, room: Units, spaces: i64, slack: Limit, spaces_before: i64) -> bool {
    if over > room {
        return false;
    }
    if spaces_before == 0 {
        return true;
    }
    match slack {
        // An infinite slack times a count of spaces: infinite where there are spaces.
        None => spaces > 0,
        Some(slack) => over * (2 * spaces_before) <= slack * spaces,
    }
}

/// Whether a line may end at an automatic hyphen whose hyphen fits: within
/// `w:consecutiveHyphenLimit`, and below mode 15 the word starts at least the zone before
/// the edge and the letter after the point fits too; in mode 15 (not justified) the room
/// after the line's text before the word exceeds the gap.
pub fn auto_hyphen_allowed(
    geometry: &Geometry,
    word_x: Units,
    text_x: Units,
    next_fits: bool,
    hyphens_before: u32,
) -> bool {
    if geometry.hyphen_limit != 0 && hyphens_before >= geometry.hyphen_limit {
        return false;
    }
    if geometry.hyphen_zone.is_some_and(|zone| geometry.right - word_x < zone) {
        return false;
    }
    if geometry.hyphen_next && !next_fits {
        return false;
    }
    !geometry.hyphen_gap.is_some_and(|gap| geometry.right - text_x <= gap)
}

/// The running state of one line.
struct State {
    x: Units,
    word_x: Units,
    text_x: Units,
    text_end: Units,
    limit: Limit,
    room: Units,
    spaces: i64,
    before_x: Units,
    before_spaces: i64,
    /// Every place the line may end so far: the piece, and where its word starts and the
    /// text before that word ends.
    breaks: Vec<(usize, Units, Units)>,
}

fn past(x: Units, limit: Limit) -> bool {
    limit.is_some_and(|l| x > l)
}

/// One line of greedy first fit from piece `i`, at the first line's start where `first`.
/// `hyphens_before`: how many lines in a row before it end in an automatic hyphen.
pub fn break_line(items: &[Piece], i: usize, geometry: &Geometry, first: bool, hyphens_before: u32) -> Line {
    let x = if first { geometry.first_start } else { geometry.start };
    let mut s = State {
        x,
        word_x: x,
        text_x: x,
        text_end: x,
        limit: Some(geometry.right),
        room: Units::ZERO,
        spaces: 0,
        before_x: x,
        before_spaces: 0,
        breaks: Vec::new(),
    };
    let start = i;
    for (j, piece) in items.iter().enumerate().skip(i) {
        if let Some(before) = j.checked_sub(1).filter(|_| j > start).and_then(|b| items.get(b)) {
            if may_break(before, piece) {
                note_break(&mut s, geometry, before, j - 1);
            }
        }
        if piece.kind == PieceKind::Break {
            return Line {
                forced: true,
                ..Line::new(start, j + 1)
            };
        }
        let x_new = advance(&mut s, geometry, piece, first, j, start, items);
        let over =
            piece.kind != PieceKind::Space && piece.kind != PieceKind::SoftHyphen && past(x_new, s.limit) && j > start;
        if over {
            let limit = s.limit.unwrap_or(geometry.right);
            if !(geometry.squeeze
                && squeezes(
                    x_new - limit,
                    s.room,
                    s.spaces,
                    Some(limit - s.before_x),
                    s.before_spaces,
                ))
            {
                return overflow(&s, items, geometry, piece, start, j, hyphens_before);
            }
        }
        if matches!(piece.kind, PieceKind::Glyph | PieceKind::Object | PieceKind::Tab) {
            s.text_end = x_new;
        }
        s.x = x_new;
    }
    Line::new(start, items.len())
}

fn note_break(s: &mut State, geometry: &Geometry, before: &Piece, at: usize) {
    let fits = before.kind != PieceKind::SoftHyphen
        || s.x + before.hyphen() <= geometry.right
        || (before.auto
            && geometry.squeeze
            && squeezes(
                s.x + before.hyphen() - geometry.right,
                s.room,
                s.spaces,
                s.limit.map(|l| l - s.before_x),
                s.before_spaces,
            ));
    if fits {
        s.breaks.push((at, s.word_x, s.text_x));
    }
    if before.kind != PieceKind::SoftHyphen {
        s.word_x = s.x;
        s.text_x = s.text_end;
    }
}

/// Where the pen is after `piece`, moving the edge for a tab and counting a space's
/// squeeze.
fn advance(
    s: &mut State,
    geometry: &Geometry,
    piece: &Piece,
    first: bool,
    j: usize,
    start: usize,
    items: &[Piece],
) -> Units {
    if piece.kind == PieceKind::Tab {
        let (stop, align, default) = next_tab(s.x, geometry, first);
        let x_new = if align.pulls_back() {
            s.limit = Some(if stop <= geometry.right {
                geometry.right
            } else if align != crate::geometry::TabAlign::Center {
                stop
            } else {
                stop * 2 - s.x
            });
            s.x
        } else if stop > geometry.right && !default {
            s.limit = None;
            stop
        } else {
            s.limit = Some(geometry.right);
            stop
        };
        s.room = Units::ZERO;
        s.spaces = 0;
        s.before_spaces = 0;
        s.before_x = x_new;
        return x_new;
    }
    let kern = if j + 1 < items.len() {
        piece.kern_next()
    } else {
        Units::ZERO
    };
    let x_new = s.x + piece.width() + kern;
    if piece.kind == PieceKind::Space && geometry.squeeze {
        if j == start || items.get(j - 1).is_some_and(|p| p.kind != PieceKind::Space) {
            s.before_x = s.x;
            s.before_spaces = s.spaces;
        }
        s.room += squeeze(piece);
        s.spaces += 1;
    }
    x_new
}

fn overflow(
    s: &State,
    items: &[Piece],
    geometry: &Geometry,
    piece: &Piece,
    start: usize,
    j: usize,
    hyphens_before: u32,
) -> Line {
    for &(at, word_x, text_x) in s.breaks.iter().rev() {
        let Some(candidate) = items.get(at) else { continue };
        if candidate.auto {
            let allowed = auto_hyphen_allowed(geometry, word_x, text_x, at + 1 < j, hyphens_before);
            let proofed = candidate.proof_end.is_some_and(|end| piece.source >= end);
            if !allowed || proofed {
                continue;
            }
        }
        return Line {
            hyphenated: candidate.kind == PieceKind::SoftHyphen,
            ..Line::new(start, at + 1)
        };
    }
    Line {
        emergency: true,
        ..Line::new(start, j)
    }
}

fn auto_hyphenated(items: &[Piece], line: &Line) -> bool {
    line.hyphenated
        && line.end > line.start
        && line
            .end
            .checked_sub(1)
            .and_then(|e| items.get(e))
            .is_some_and(|p| p.auto)
}

/// Whether a break ending the paragraph puts its mark on a line of its own: a line break
/// does; so does a column break in a section of several columns.
fn ends_on_its_own_line(ch: char, columns: bool) -> bool {
    ch == '\n' || (columns && ch == '\u{b}')
}

/// Every line of a paragraph's pieces, from piece `start` (0: the whole paragraph) --
/// greedy first fit, a line depending on nothing before its first piece but whether it is
/// the paragraph's first (`first_line`). `hyphens`: how many lines before `start` in a row
/// end in an automatic hyphen (`None`: one if the piece before `start` is an automatic
/// hyphen, all a caller breaking from the middle of a paragraph knows). `columns`: the
/// paragraph is in a section of several columns.
pub fn break_from(
    items: &[Piece],
    start: usize,
    geometry: &Geometry,
    first_line: bool,
    columns: bool,
    hyphens: Option<u32>,
) -> Vec<Line> {
    let n = items.len();
    let mut lines: Vec<Line> = Vec::new();
    let mut i = start;
    let mut carried = hyphens;
    while i < n || lines.is_empty() {
        if i >= n {
            lines.push(Line::new(n, n));
            break;
        }
        let before = carried
            .unwrap_or_else(|| u32::from(i > 0 && i.checked_sub(1).and_then(|b| items.get(b)).is_some_and(|p| p.auto)));
        let line = break_line(items, i, geometry, lines.is_empty() && first_line, before);
        carried = Some(if auto_hyphenated(items, &line) {
            carried.unwrap_or(0) + 1
        } else {
            0
        });
        lines.push(line);
        i = line.end;
        if i >= n {
            let last = n.checked_sub(1).and_then(|e| items.get(e)).map(|p| p.ch);
            if line.forced && last.is_some_and(|c| ends_on_its_own_line(c, columns)) {
                lines.push(Line::new(n, n));
            }
            break;
        }
    }
    lines
}

/// Every line of a paragraph (docx2svg's `break_pieces`).
pub fn break_pieces(items: &[Piece], geometry: &Geometry, columns: bool) -> Vec<Line> {
    break_from(items, 0, geometry, true, columns, Some(0))
}
