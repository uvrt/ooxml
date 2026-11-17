//! Owns: the room a paragraph's borders take above its first line and below its last,
//! given the paragraphs beside it -- one border box with a neighbour whose borders and
//! indents match (docx2svg's `vertical.paragraph_borders_px`, `border_box`, `side_px`).
//!
//! Whether a paragraph's neighbours share its box is [`box_neighbours`].
//!
//! Must not: draw a border, or decide where a paragraph goes.

use crate::px::Px;
use crate::vertical::border_px;
use ooxml_word_model::value::{Props, Value};

fn int(props: &Props, key: &str) -> i64 {
    props.get(key).and_then(Value::int).unwrap_or(0)
}
const BORDER_SIDES: [&str; 6] = ["top", "left", "bottom", "right", "between", "bar"];

pub(crate) fn border_of<'a>(pp: &'a Props, side: &str) -> Option<&'a ooxml_word_model::value::Border> {
    match pp.get(format!("pBdr.{side}").as_str()) {
        Some(Value::Border(b)) => Some(b),
        _ => None,
    }
}

pub(crate) fn drawn(border: Option<&ooxml_word_model::value::Border>) -> bool {
    border.is_some_and(|b| !matches!(b.style.as_deref(), None | Some("nil" | "none")))
}

/// The height one `w:pBdr` side takes (`side_px`): space and width, a `double` line
/// three widths; a `w:between` border its space on both sides.
fn side_px(border: Option<&ooxml_word_model::value::Border>, between: bool) -> Px {
    let Some(b) = border.filter(|b| drawn(Some(b))) else {
        return Px::ZERO;
    };
    let tall = if b.style.as_deref() == Some("double") { 3 } else { 1 };
    let height = border_px(b.size * tall, b.space);
    if between {
        height + Px::twips(b.space * 20)
    } else {
        height
    }
}

/// What decides whether two consecutive paragraphs share one border box (`border_box`).
fn border_box(pp: &Props) -> (Vec<Option<&ooxml_word_model::value::Border>>, i64, i64) {
    (
        BORDER_SIDES.iter().map(|s| border_of(pp, s)).collect(),
        int(pp, "ind.left"),
        int(pp, "ind.right"),
    )
}

/// Whether the paragraphs before and after one (`None`: none, or not a paragraph) share
/// its border box: it draws a border, and theirs and its indents are the same
/// (`_details`' `box_neighbours`).
pub fn box_neighbours(pp: &Props, previous: Option<&Props>, following: Option<&Props>) -> (bool, bool) {
    let own = border_box(pp);
    if !own.0.iter().any(|b| drawn(*b)) {
        return (false, false);
    }
    (
        previous.is_some_and(|p| border_box(p) == own),
        following.is_some_and(|f| border_box(f) == own),
    )
}

/// `(above, below)`: the room a paragraph's borders take above its first line and below
/// its last, given the paragraphs beside it (`paragraph_borders_px`).
pub fn paragraph_borders(pp: &Props, previous: Option<&Props>, following: Option<&Props>) -> (Px, Px) {
    let own = border_box(pp);
    if !own.0.iter().any(|b| drawn(*b)) {
        return (Px::ZERO, Px::ZERO);
    }
    let after_same = following.is_some_and(|f| border_box(f) == own);
    let before_same = previous.is_some_and(|p| border_box(p) == own);
    let between = border_of(pp, "between");
    let upper = match between.filter(|b| drawn(Some(b))) {
        Some(b) => Px::twips(b.space * 20),
        None => Px::ZERO,
    };
    let above = if before_same {
        side_px(between, true) - upper
    } else {
        side_px(border_of(pp, "top"), false)
    };
    let below = if after_same {
        upper
    } else {
        side_px(border_of(pp, "bottom"), false)
    };
    (above, below)
}
