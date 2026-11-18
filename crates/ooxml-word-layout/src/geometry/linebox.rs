//! Owns: a line's box -- its pitch with what of its paragraph's spacing and borders is in
//! it ([`LineBox`]) -- and the whole device pixels Word draws it on: its baseline
//! ([`baseline_in_box`]) and the rows its text fills ([`text_band`]) -- docx2svg's
//! `vertical.LineBox`, `_line_extent`, `_text_line`, `baseline_in_box` and
//! `layout.text_band`, measured there.
//!
//! Must not: place lines (the paginator has), or decide what a box holds (its caller says).

use crate::heights::LineHeight;
use crate::px::Px;
use crate::vertical::LineRule;
use ooxml_base::Q;

/// Where a plain line with nothing below its text puts a fraction `f` of a pixel of slack:
/// `round(above - LAMBDA * f)`.
const LAMBDA_PLAIN: (i128, i128) = (1, 2);
/// The same for an `exact` line.
const LAMBDA_EXACT: (i128, i128) = (1, 4);

/// One line's box, exact device px, from the top of its first part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineBox {
    /// The line's own facts: its pitch, text extent and rule.
    pub height: LineHeight,
    /// The paragraph space (and top border) in the box above the text, first line only.
    pub space_before: Px,
    /// What sits below the pitch: the paragraph's space after and bottom border, last
    /// line only.
    pub space_after: Px,
}

impl LineBox {
    /// A line's box with `space_before` above its pitch and `space_after` below it.
    pub fn new(height: LineHeight, space_before: Px, space_after: Px) -> LineBox {
        LineBox {
            height,
            space_before,
            space_after,
        }
    }

    /// Its whole height.
    pub fn whole(&self) -> Px {
        self.space_before + self.height.pitch() + self.space_after
    }

    /// Everything in it below the text line (`below_text`).
    pub fn below_text(&self) -> Px {
        self.height.multiple_extra() + self.space_after
    }
}

/// `(above, below)` of the baseline within the line's pitch, and whether the line puts its
/// slack above the text (`_line_extent`).
pub fn line_extent(height: &LineHeight) -> (Px, Px, bool) {
    let (h, above, below, natural) = (
        height.pitch(),
        height.text_above(),
        height.text_below(),
        height.natural(),
    );
    match height.rule {
        LineRule::Exact => {
            let below = (h * Q::new(1, 5)).quantise_down();
            (h - below, below, true)
        }
        LineRule::AtLeast if h > natural => (h - below, below, true),
        LineRule::Auto if height.line < 240 && !natural.is_zero() => {
            let scale = h.value() / natural.value();
            (above * scale, below * scale, false)
        }
        LineRule::Auto if height.line > 240 => (above, below, false),
        _ => (above, h - above, false),
    }
}

/// `(space above the text inside the pitch, the text's own pitch, above, below)`
/// (`_text_line`).
fn text_line(height: &LineHeight) -> (Px, Px, Px, Px) {
    let h = height.pitch();
    if height.rule == LineRule::AtLeast && h > height.natural() {
        return (
            h - height.natural(),
            height.natural(),
            height.text_above(),
            height.natural() - height.text_above(),
        );
    }
    if height.rule == LineRule::Auto && height.line > 240 {
        return (
            Px::ZERO,
            height.text_above() + height.text_below(),
            height.text_above(),
            height.text_below(),
        );
    }
    let (above, below, _) = line_extent(height);
    (Px::ZERO, h, above, below)
}

/// The device-pixel row of a line's baseline, `top` the exact top of its box
/// (`baseline_in_box`).
pub fn baseline_in_box(top: Px, b: &LineBox) -> i64 {
    let whole = b.whole();
    let below_text = b.below_text();
    if below_text.is_zero() {
        let (above, below, extra_above) = line_extent(&b.height);
        let above = above + b.space_before;
        let f = whole.fract();
        if f >= Px::from(Q::new(1, 2)) {
            return (top + whole).round_half_up() - below.round_half_up();
        }
        if b.height.rule == LineRule::Exact && b.space_before.is_zero() {
            return top.round_half_up() + (above - f * Q::new(LAMBDA_EXACT.0, LAMBDA_EXACT.1)).round_half_up();
        }
        if extra_above || !b.space_before.is_zero() {
            return top.round_half_up() + whole.floor() - below.round_half_up();
        }
        return top.round_half_up() + (above - f * Q::new(LAMBDA_PLAIN.0, LAMBDA_PLAIN.1)).round_half_up();
    }
    let (inner, t, above, below) = text_line(&b.height);
    let xa = b.space_before + inner;
    let rounded_xa = xa.round_half_up();
    let rest = t + (xa - Px::int(rounded_xa)) + (below_text - Px::int(below_text.round_half_up()));
    let from_top =
        top.round_half_up() + rounded_xa + above.round_half_up().max(rest.round_half_up() - below.round_half_up());
    from_top.min((top + whole).round_half_up() - below.round_half_up())
}

/// The rows a line's text occupies, as Word rounds them: what a highlight, run shading or
/// paragraph shading fills (`text_band`), `top` the exact top of the box.
pub fn text_band(top: Px, b: &LineBox) -> (i64, i64) {
    let (inner, t, _, _) = text_line(&b.height);
    let xa = b.space_before + inner;
    let below_text = b.below_text();
    if below_text.is_zero() {
        let start = top + xa;
        return (start.round_half_up(), (start + t).round_half_up());
    }
    let first = top.round_half_up() + xa.round_half_up();
    let rest = t + (xa - Px::int(xa.round_half_up())) + (below_text - Px::int(below_text.round_half_up()));
    (first, first + rest.round_half_up())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(pitch: Px, above: Px, below: Px) -> LineHeight {
        LineHeight::new(pitch, above, below, pitch, LineRule::Auto, 240, false)
    }

    #[test]
    fn a_whole_pixel_line_sits_on_its_rounded_ascent() {
        let h = plain(Px::int(46), Px::int(37), Px::int(9));
        let b = LineBox::new(h, Px::ZERO, Px::ZERO);
        assert_eq!(baseline_in_box(Px::int(300), &b), 337);
        assert_eq!(text_band(Px::int(300), &b), (300, 346));
    }

    #[test]
    fn space_below_rounds_from_the_top() {
        let h = plain(Px::int(46), Px::int(37), Px::int(9));
        let b = LineBox::new(h, Px::ZERO, Px::twips(160));
        assert_eq!(baseline_in_box(Px::int(300), &b), 337);
    }
}
