//! Owns: Word's vertical model of a line, exact (docx2svg's `vertical`): a face's natural
//! height and a line rule's pitch, a line's extent over the items on it (separate maxima,
//! a label by its ascent only, a raised run, a run border), the pitch of a mixed line, the
//! space between paragraphs and at a page's top, paragraph and run borders, and where a
//! line's baseline falls in its box.
//!
//! Must not: decide which characters are on a line (`crate::heights` does, from the text
//! engine's lines), or place lines on pages.

use crate::faces::Faces;
use crate::px::Px;
use ooxml_base::Q;
use ooxml_fonts::FaceMetrics;
use ooxml_text::FaceKey;

/// `w:spacing/@w:lineRule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineRule {
    /// `auto`: `w:line` in 240ths of the natural height.
    Auto,
    /// `exact`: `w:line` twips.
    Exact,
    /// `atLeast`: at least `w:line` twips.
    AtLeast,
}

impl LineRule {
    /// The rule as written (`auto` for anything unknown or unstated).
    pub fn parse(value: Option<&str>) -> LineRule {
        match value {
            Some("exact") => LineRule::Exact,
            Some("atLeast") => LineRule::AtLeast,
            _ => LineRule::Auto,
        }
    }

    /// Its name as written.
    pub fn name(self) -> &'static str {
        match self {
            LineRule::Auto => "auto",
            LineRule::Exact => "exact",
            LineRule::AtLeast => "atLeast",
        }
    }
}

/// A face's vertical extent in its own units of an em: a real face's metrics, or a mixed
/// line's synthetic face (units per em 1, extents fractions of an em).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shape {
    /// Units per em.
    pub units_per_em: Q,
    /// Ascent.
    pub ascent: Q,
    /// Descent, positive down.
    pub descent: Q,
    /// Line gap.
    pub line_gap: Q,
}

impl Shape {
    /// A real face's.
    pub fn of(face: &FaceMetrics) -> Shape {
        Shape {
            units_per_em: Q::from_integer(i128::from(face.units_per_em)),
            ascent: Q::from_integer(i128::from(face.ascent)),
            descent: Q::from_integer(i128::from(face.descent)),
            line_gap: Q::from_integer(i128::from(face.line_gap)),
        }
    }

    fn em(&self, half_points: i64) -> Q {
        if *self.units_per_em.numer() == 0 {
            return Q::from_integer(0);
        }
        Q::new(i128::from(half_points), 2) * Px::per_point() / self.units_per_em
    }

    /// Single-spaced pitch: `(ascent + descent + lineGap) / upm x size`, rounded to the
    /// layout unit (`natural_height_px`).
    pub fn natural(&self, half_points: i64) -> Px {
        Px::quantise_q((self.ascent + self.descent + self.line_gap) * self.em(half_points))
    }

    /// The text's extent about the baseline at a size: `(ascent + lineGap, descent)`.
    pub fn extent(&self, half_points: i64) -> (Px, Px) {
        let em = self.em(half_points);
        (
            Px::from((self.ascent + self.line_gap) * em),
            Px::from(self.descent * em),
        )
    }

    /// The pitch of a line rule (`line_pitch_px`).
    pub fn pitch(&self, half_points: i64, rule: LineRule, line: i64) -> Px {
        let natural = self.natural(half_points);
        match rule {
            LineRule::Auto => Px::quantise_q(natural.value() * Q::new(i128::from(line), 240)),
            LineRule::Exact => Px::twips(line),
            LineRule::AtLeast => natural.max(Px::twips(line)),
        }
    }
}

/// One thing on a line that takes part in its height: a character's format (its face,
/// size, raised position and run border) or a picture bullet. Characters differing only
/// in the character are one item (`lines.height_key`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeightItem {
    /// The face.
    pub face: Option<FaceKey>,
    /// Its `w:sz`, half points (a raised or lowered run's, not its drawn size).
    pub half_points: i64,
    /// `w:position`, half points.
    pub position: i64,
    /// `w:bdr`: width in eighths of a point, space in points.
    pub border: Option<(i64, i64)>,
    /// A picture bullet: how far it reaches above the baseline.
    pub picture: Option<Px>,
}

/// What a run border adds above and below its run: space plus width, the width truncated
/// to whole twips and then to the layout unit (`run_border_px`).
pub fn run_border_px(width_eighths: i64, space_points: i64) -> Px {
    let twips = (width_eighths * 20).div_euclid(8);
    Px::twips(space_points * 20) + Px::points(Q::new(i128::from(twips), 20)).quantise_down()
}

/// How far one character reaches above and below the baseline (`item_extent`).
fn item_extent(item: &HeightItem, shape: &Shape) -> (Px, Px) {
    let (mut above, mut below) = shape.extent(item.half_points);
    let shift = Px::twips(item.position * 10);
    above += shift;
    below -= shift;
    if let Some((width, space)) = item.border {
        let pad = run_border_px(width, space);
        above += pad;
        below += pad;
    }
    (above, below)
}

/// One item's natural height, the one an `auto` multiple scales: its face's at its size,
/// and its run border twice (`item_natural_px`).
fn item_natural(item: &HeightItem, shape: &Shape) -> Px {
    let natural = shape.natural(item.half_points);
    match item.border {
        Some((width, space)) => {
            let pad = run_border_px(width, space);
            natural + pad + pad
        }
        None => natural,
    }
}

/// A line's extent: the face it is measured in (real, or a mixed line's synthetic one),
/// the size, and whether it is mixed (`line_extent`'s answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Extent {
    /// The face.
    pub shape: Shape,
    /// The size, half points.
    pub half_points: i64,
    /// More than one face or size, a label, a raised run or a border.
    pub mixed: bool,
}

struct ItemExtent {
    above: Px,
    below: Px,
    half_points: i64,
    face: Option<FaceMetrics>,
    label: bool,
    moved: bool,
}

/// One line's extent over its text items and its label's (`line_extent`): the largest
/// ascent and the largest descent, maxed separately; a label by its ascent only.
pub fn line_extent(text: &[HeightItem], label: &[HeightItem], faces: &dyn Faces) -> Option<Extent> {
    let mut extents: Vec<ItemExtent> = Vec::new();
    let tagged = text.iter().map(|i| (i, false)).chain(label.iter().map(|i| (i, true)));
    for (item, is_label) in tagged {
        if let Some(picture) = item.picture {
            extents.push(ItemExtent {
                above: picture + Px::twips(item.position * 10),
                below: Px::ZERO,
                half_points: item.half_points,
                face: None,
                label: true,
                moved: true,
            });
            continue;
        }
        let Some(face) = item.face.as_ref().and_then(|f| faces.metrics(f)) else {
            continue;
        };
        let shape = Shape::of(&face);
        let (above, below) = item_extent(item, &shape);
        let moved = (above, below) != shape.extent(item.half_points);
        extents.push(ItemExtent {
            above,
            below: if is_label { Px::ZERO } else { below },
            half_points: item.half_points,
            face: Some(face),
            label: is_label,
            moved,
        });
    }
    let first = extents.first()?;
    let mut top = first;
    for e in &extents {
        if e.above > top.above {
            top = e;
        }
    }
    let above = top.above;
    let half_points = top.half_points;
    let below = extents.iter().map(|e| e.below).max().unwrap_or(Px::ZERO);
    let one = extents
        .iter()
        .all(|e| (e.face, e.half_points) == (first.face, first.half_points));
    if one && !extents.iter().all(|e| e.label) && !extents.iter().any(|e| e.moved) {
        if let Some(face) = first.face {
            return Some(Extent {
                shape: Shape::of(&face),
                half_points: first.half_points,
                mixed: false,
            });
        }
    }
    let em = Q::new(i128::from(half_points), 2) * Px::per_point();
    if *em.numer() == 0 {
        return None;
    }
    Some(Extent {
        shape: Shape {
            units_per_em: Q::from_integer(1),
            ascent: above.value() / em,
            descent: below.value() / em,
            line_gap: Q::from_integer(0),
        },
        half_points,
        mixed: true,
    })
}

/// The largest natural height of any one item (`tallest_natural`).
pub fn tallest_natural(items: &[HeightItem], faces: &dyn Faces) -> Option<Px> {
    items
        .iter()
        .filter_map(|item| {
            let face = faces.metrics(item.face.as_ref()?)?;
            Some(item_natural(item, &Shape::of(&face)))
        })
        .max()
}

/// The pitch of a line of several items (`mixed_line_pitch`): under an `auto` multiple,
/// the natural height of everything plus the multiple's extra over the tallest text item
/// alone; otherwise the rule's pitch of the combined extent.
pub fn mixed_line_pitch(
    extent: &Extent,
    text_extent: Option<&Extent>,
    rule: LineRule,
    line: i64,
    text_natural: Option<Px>,
) -> Px {
    if rule != LineRule::Auto || line == 240 || (text_extent.is_none() && text_natural.is_none()) {
        return extent.shape.pitch(extent.half_points, rule, line);
    }
    let text_natural = match (text_natural, text_extent) {
        (Some(n), _) => n,
        (None, Some(t)) => t.shape.natural(t.half_points),
        (None, None) => Px::ZERO,
    };
    let everything = extent.shape.natural(extent.half_points);
    Px::quantise_q(everything.value() + text_natural.value() * (Q::new(i128::from(line), 240) - Q::from_integer(1)))
}

/// The space between two paragraphs and the part of it in the second one's box, from
/// their spaces after and before in twips (`paragraph_gap_px`): they collapse to the
/// larger, in twips.
pub fn paragraph_gap(previous_after: i64, before: i64) -> (Px, Px) {
    let box_before = if before > previous_after {
        Px::twips(before - previous_after)
    } else {
        Px::ZERO
    };
    (Px::twips(previous_after) + box_before, box_before)
}

/// Whether a paragraph that starts a page keeps its space before there: a section's
/// first, and below mode 15 a `pageBreakBefore` one (`keeps_space_before_at_page_top`).
pub fn keeps_space_before_at_page_top(section_start: bool, page_break_before: bool, mode15: bool) -> bool {
    section_start || (page_break_before && !mode15)
}

/// The space before of a paragraph that starts a page: how far it moves the text down,
/// and the space above the text in the first line's box (`page_top_gap_px`).
pub fn page_top_gap(before: i64, keeps: bool, previous_after: i64) -> (Px, Px) {
    if keeps && previous_after != 0 {
        let (_, kept) = paragraph_gap(previous_after, before);
        return (kept, kept);
    }
    let (kept, in_box) = paragraph_gap(0, before);
    (if keeps { kept } else { Px::ZERO }, in_box)
}

/// Whether an autospaced space is there at all, given the paragraph on that side (`None`:
/// nothing there) (`autospace_kept`).
pub fn autospace_kept(numbered: bool, neighbour_numbered: Option<bool>) -> bool {
    match neighbour_numbered {
        None => false,
        Some(n) => !(numbered && n),
    }
}

/// The height a paragraph border side of `width` eighths of a point and `space` points
/// takes (`border_px`): space plus width, the width truncated to whole twips.
pub fn border_px(width_eighths: i64, space_points: i64) -> Px {
    Px::twips(space_points * 20) + Px::twips((width_eighths * 20).div_euclid(8))
}
