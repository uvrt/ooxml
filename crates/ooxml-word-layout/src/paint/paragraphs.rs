//! Owns: a body paragraph's borders and shading on a page, on whole device pixels
//! (docx2svg's `_paragraph_rules` and `paragraph_rects`, measured there): side borders
//! `w:space` and 6 px outside the text edges, top and bottom borders `w:space` beyond the
//! first and last lines' text rows, a `w:between` border where the paragraph shares its box
//! with the one above, and the shading inside the borders.
//!
//! Must not: decide where the paragraph's lines are (the geometry has).

use super::spans::{colour, shading};
use super::Rule;
use crate::borders::{border_of, drawn};
use crate::px::Px;
use ooxml_base::Q;
use ooxml_word_model::value::{Border, Props, Value};

/// How far outside the text a border box is drawn beyond its `w:space`, device px.
const OUTSET: i64 = 6;

/// A paragraph's part on a page: its text edges, its first and last lines' text rows,
/// where it starts below its space before and where the next one's space starts, and
/// which of its edges are on this page and drawn.
pub(super) struct ParagraphBox<'a> {
    pub(super) pp: &'a Props,
    pub(super) path: &'a str,
    pub(super) left: Px,
    pub(super) right: Px,
    pub(super) band: (i64, i64),
    pub(super) y_gap: Px,
    pub(super) y_after: Px,
    pub(super) top_side: bool,
    pub(super) between_above: bool,
    pub(super) bottom_side: bool,
}

/// A border line's drawn width: `w:sz` eighths of a point, truncated to whole px.
fn width_px(b: &Border) -> i64 {
    Px::points(Q::new(i128::from(b.size), 8)).floor().max(1)
}

/// A border's `w:space` as drawn: whole px, truncated.
fn space_px(b: Option<&Border>) -> i64 {
    b.map_or(0, |b| Px::points(Q::from_integer(i128::from(b.space))).floor())
}

fn side(pp: &Props, name: &str) -> Option<Border> {
    border_of(pp, name).filter(|b| drawn(Some(b))).cloned()
}

/// `left` and `right` of the paragraph's text: its column's edges less its indents.
pub(super) fn text_edges(pp: &Props, column: (Px, Px)) -> (Px, Px) {
    let int = |k: &str| pp.get(k).and_then(Value::int).unwrap_or(0);
    (
        column.0 + Px::twips(int("ind.left")),
        column.1 - Px::twips(int("ind.right")),
    )
}

fn line(kind: &'static str, b: &Border, path: &str, x: (i64, i64), y: i64) -> Rule {
    Rule {
        kind,
        path: path.to_owned(),
        x: Px::int(x.0),
        y: Px::int(y),
        width: Px::int(x.1 - x.0),
        height: Px::int(width_px(b)),
        color: Some(colour(b.color.as_deref()).unwrap_or_else(|| "000000".to_owned())),
    }
}

/// The paragraph's border lines and shading: `(borders, shading)` -- the shading goes under
/// the page's other rules.
pub(super) fn paragraph_rects(p: &ParagraphBox<'_>) -> (Vec<Rule>, Option<Rule>) {
    let (top, left, bottom, right, between) = (
        side(p.pp, "top"),
        side(p.pp, "left"),
        side(p.pp, "bottom"),
        side(p.pp, "right"),
        side(p.pp, "between"),
    );
    let any = [&top, &left, &bottom, &right, &between].iter().any(|s| s.is_some());
    let fill = shading(p.pp.get("shd"));
    if !any && fill.is_none() {
        return (Vec::new(), None);
    }
    let text_left = p.left.round_half_up();
    let text_right = p.right.floor();
    let inner_left = left.as_ref().map(|b| text_left - space_px(Some(b)) - OUTSET);
    let inner_right = right.as_ref().map(|b| text_right + space_px(Some(b)) + OUTSET);
    let outer_left = match (&left, inner_left) {
        (Some(b), Some(i)) => i - width_px(b),
        _ => text_left - OUTSET,
    };
    let outer_right = match (&right, inner_right) {
        (Some(b), Some(i)) => i + width_px(b),
        _ => text_right + OUTSET,
    };
    let span = (outer_left, outer_right);
    let mut out = Vec::new();
    let (mut box_top, mut box_bottom) = p.band;
    match (&top, &between) {
        (Some(b), _) if p.top_side => {
            let inner = p.band.0 - space_px(Some(b));
            out.push(line("border-top", b, p.path, span, inner - width_px(b)));
            box_top = inner;
        }
        (_, Some(b)) if p.between_above && !(p.top_side && top.is_some()) => {
            let start = p.y_gap.round_half_up();
            out.push(line("border-between", b, p.path, span, start));
            box_top = start + width_px(b);
        }
        _ => {}
    }
    let mut outer_top = box_top;
    let mut outer_bottom = box_bottom;
    if let (true, Some(b)) = (p.top_side, &top) {
        outer_top = box_top - width_px(b);
    }
    match &bottom {
        Some(b) if p.bottom_side => {
            let inner = p.band.1 + space_px(Some(b));
            out.push(line("border-bottom", b, p.path, span, inner));
            box_bottom = inner;
            outer_bottom = inner + width_px(b);
        }
        _ if !p.bottom_side && any => {
            box_bottom = p.y_after.round_half_up();
            outer_bottom = box_bottom;
        }
        _ => {}
    }
    let sides = [
        ("border-left", &left, inner_left, true),
        ("border-right", &right, inner_right, false),
    ];
    out.extend(side_rules(p.path, &sides, (outer_top, outer_bottom)));
    let shaded = fill.map(|fill| Rule {
        kind: "paragraph-shading",
        path: p.path.to_owned(),
        x: Px::int(inner_left.unwrap_or(text_left - OUTSET)),
        y: Px::int(box_top),
        width: Px::int(inner_right.unwrap_or(text_right + OUTSET) - inner_left.unwrap_or(text_left - OUTSET)),
        height: Px::int(box_bottom - box_top),
        color: Some(fill),
    });
    (out, shaded)
}

/// The side borders: each `(kind, border, inner edge, drawn outward of it)` from the box's
/// outer top to its outer bottom.
fn side_rules(
    path: &str,
    sides: &[(&'static str, &Option<Border>, Option<i64>, bool)],
    (top, bottom): (i64, i64),
) -> Vec<Rule> {
    let mut out = Vec::new();
    for (kind, b, inner, outward) in sides {
        if let (Some(b), Some(inner)) = (b, inner) {
            let w = width_px(b);
            let x0 = if *outward { inner - w } else { *inner };
            out.push(Rule {
                kind,
                path: path.to_owned(),
                x: Px::int(x0),
                y: Px::int(top),
                width: Px::int(w),
                height: Px::int(bottom - top),
                color: Some(colour(b.color.as_deref()).unwrap_or_else(|| "000000".to_owned())),
            });
        }
    }
    out
}
