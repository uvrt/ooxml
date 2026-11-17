//! Owns: where Word puts a floating drawing (`wp:anchor`) on its page, in twips from the
//! page's top left (docx2svg's `floating`, every rule measured): an offset truncated to
//! whole twips; an alignment placing the box of the extent and its effect extent in its
//! frame ([`horizontal`], [`vertical`]) -- the page, the margins, the column, the
//! anchor's character, paragraph or line ([`Frames`]); `simplePos`. And a paragraph's
//! floating drawings where they stand in its text ([`anchors_of`]).
//!
//! Must not: decide which lines a drawing moves (the paginator does), or draw it.

use crate::paragraph::MeasuredParagraph;
use ooxml_base::Q;
use ooxml_word_model::model::{Anchor, AnchorPosition, Paragraph};

/// EMU per twip.
pub const EMU_PER_TWIP: i64 = 635;

/// EMU to whole twips, truncated towards zero, as Word takes an offset or an extent.
pub fn emu_twips(value: i64) -> i64 {
    value / EMU_PER_TWIP
}

/// What an anchor on one page may be positioned against, twips from the page's top
/// left (`paragraph_top`, `line_top`, `line_bottom` and `character` exact).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frames {
    /// The page's width.
    pub page_width: i64,
    /// Its height.
    pub page_height: i64,
    /// The left margin and gutter.
    pub left: i64,
    /// The right margin.
    pub right: i64,
    /// The top margin.
    pub top: Q,
    /// The bottom margin.
    pub bottom: Q,
    /// The header distance.
    pub header: i64,
    /// The footer distance.
    pub footer: i64,
    /// The page's place, 1-based: its parity picks inside and outside.
    pub page: usize,
    /// Compatibility mode 15 or later.
    pub mode15: bool,
    /// The anchor's paragraph's top.
    pub paragraph_top: Q,
    /// Its line's top.
    pub line_top: Q,
    /// Its line's foot.
    pub line_bottom: Q,
    /// The anchor's character's place.
    pub character: Q,
    /// In a section of several columns, the anchor's column: its left edge and width.
    pub column: Option<(i64, i64)>,
}

fn int(value: i64) -> Q {
    Q::from_integer(i128::from(value))
}

/// The extent truncated to twips and the effect extent's four sides, twips.
struct Box6 {
    cx: i64,
    cy: i64,
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

fn box_of(anchor: &Anchor) -> Box6 {
    let [left, top, right, bottom] = anchor.effect.map(emu_twips);
    Box6 {
        cx: emu_twips(anchor.extent.0),
        cy: emu_twips(anchor.extent.1),
        left,
        top,
        right,
        bottom,
    }
}

/// `size` (with `before` and `after` of effect extent) aligned in a frame from `start`,
/// `width` wide; `first`: inside is the frame's start (`_aligned`).
fn aligned(frame: (Q, Q), size: (i64, i64, i64), align: Option<&str>, first: bool) -> Option<Q> {
    let (start, width) = frame;
    let (size, before, after) = size;
    let align = match align {
        Some(a @ ("inside" | "outside")) => {
            if (a == "inside") == first {
                "left"
            } else {
                "right"
            }
        }
        Some(a) => a,
        None => return None,
    };
    match align {
        "left" | "top" => Some(start + int(before)),
        "right" | "bottom" => Some(start + width - int(size) - int(after)),
        "center" => Some(start + (width - int(before + size + after)) / int(2) + int(before)),
        _ => None,
    }
}

/// The drawing's left edge, twips from the page's left; `None` for a frame the model does
/// not know.
pub fn horizontal(anchor: &Anchor, frames: &Frames) -> Option<Q> {
    let b = box_of(anchor);
    if let Some((x, _)) = anchor.simple {
        return Some(int(emu_twips(x)));
    }
    let position: &AnchorPosition = &anchor.h;
    let odd = frames.page % 2 == 1;
    let inside_left = if frames.mode15 { odd } else { !odd };
    let relative = position.relative.as_deref().unwrap_or("column");
    let (pw, l, r) = (frames.page_width, frames.left, frames.right);
    let margin = (int(l), int(pw - l - r));
    let left_margin = (Q::from_integer(0), int(l));
    let right_margin = (int(pw - r), int(r));
    let frame = match relative {
        "page" => (Q::from_integer(0), int(pw)),
        "margin" => margin,
        "column" => frames.column.map_or(margin, |(x, w)| (int(x), int(w))),
        "leftMargin" => left_margin,
        "rightMargin" => right_margin,
        "insideMargin" if inside_left => left_margin,
        "insideMargin" => right_margin,
        "outsideMargin" if inside_left => right_margin,
        "outsideMargin" => left_margin,
        "character" => (frames.character, Q::from_integer(0)),
        _ => return None,
    };
    if let Some(offset) = position.offset {
        return Some(frame.0 + int(emu_twips(offset)));
    }
    let size = (b.cx, b.left, b.right);
    if relative == "character" {
        return aligned(frame, size, position.align.as_deref(), odd);
    }
    aligned(frame, size, Some(position.align.as_deref().unwrap_or("left")), odd)
}

/// The drawing's top edge, twips from the page's top.
pub fn vertical(anchor: &Anchor, frames: &Frames) -> Option<Q> {
    let b = box_of(anchor);
    if let Some((_, y)) = anchor.simple {
        let above = int(emu_twips(y)) - frames.paragraph_top;
        return Some(frames.paragraph_top + (above + Q::new(1, 2)).floor());
    }
    let position: &AnchorPosition = &anchor.v;
    let odd = frames.page % 2 == 1;
    let relative = position.relative.as_deref().unwrap_or("paragraph");
    let height = int(frames.page_height);
    let (top, bottom) = (frames.top, frames.bottom);
    let top_margin = (Q::from_integer(0), top);
    let bottom_margin = (height - bottom, bottom);
    let margins = match relative {
        "page" => Some((Q::from_integer(0), height)),
        "margin" => Some((top, height - top - bottom)),
        "topMargin" => Some(top_margin),
        "bottomMargin" => Some(bottom_margin),
        "insideMargin" => Some(if odd { top_margin } else { bottom_margin }),
        // Below mode 15 the outside margin is the inside one.
        "outsideMargin" => Some(if odd == frames.mode15 {
            bottom_margin
        } else {
            top_margin
        }),
        _ => None,
    };
    let size = (b.cy, b.top, b.bottom);
    if let Some(frame) = margins {
        if let Some(offset) = position.offset {
            return Some(frame.0 + int(emu_twips(offset)));
        }
        let align = position.align.as_deref().unwrap_or("top");
        if relative == "page" && matches!(align, "inside" | "outside") {
            if (align == "inside") == odd {
                return Some(int(frames.header) / int(2) + int(b.top));
            }
            return Some(height - int(frames.footer) / int(2) - int(b.cy) - int(b.bottom));
        }
        return aligned(frame, size, Some(align), odd);
    }
    match relative {
        "paragraph" => Some(frames.paragraph_top + int(position.offset.map_or(b.top, emu_twips))),
        "line" => {
            let top = frames.line_top;
            if let Some(offset) = position.offset {
                return Some(top + int(emu_twips(offset)));
            }
            let bottom = if frames.mode15 { frames.line_bottom } else { top };
            match position.align.as_deref().unwrap_or("top") {
                "top" => Some(top + int(b.top)),
                "bottom" => Some(bottom - int(b.cy) - int(b.bottom)),
                "center" => Some((top + bottom) / int(2) - int(b.top + b.cy + b.bottom) / int(2) + int(b.top)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A floating drawing where it stands in its paragraph's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorRef {
    /// Its index in the paragraph's text.
    pub source: i64,
    /// Its run.
    pub run: usize,
    /// Its index among the run's anchors.
    pub index: usize,
    /// The line it is on.
    pub line: usize,
    /// The drawing.
    pub anchor: Anchor,
}

/// The wraps text goes beside a drawing in.
pub const SIDE_WRAPS: [&str; 3] = ["wrapSquare", "wrapTight", "wrapThrough"];

/// Whether text is laid out around a drawing: every wrap but `wrapNone`.
pub fn moves_text(anchor: &Anchor) -> bool {
    !matches!(anchor.wrap.as_deref(), None | Some("wrapNone"))
}

/// Every floating drawing of `paragraph` that takes no room in its line, where it is in
/// the text and the line it is on (`anchors_of`).
pub fn anchors_of(paragraph: &Paragraph, measured: &MeasuredParagraph) -> Vec<AnchorRef> {
    let m = &measured.measured;
    let line_of = crate::notes::flowed::line_of_sources(&m.pieces, &m.lines);
    let mut out = Vec::new();
    let mut offset = 0usize;
    for (run_index, run) in paragraph.runs.iter().enumerate() {
        for (at, mark) in run.breaks.iter() {
            let Some(k) = mark.strip_prefix("anchor:").and_then(|k| k.parse::<usize>().ok()) else {
                continue;
            };
            let Some(anchor) = run.anchors.get(k) else { continue };
            let wrap = anchor.wrap.as_deref().unwrap_or_default();
            if moves_text(anchor) && wrap != "wrapTopAndBottom" && !SIDE_WRAPS.contains(&wrap) {
                continue;
            }
            let source = i64::try_from(offset + at).unwrap_or(i64::MAX);
            out.push(AnchorRef {
                source,
                run: run_index,
                index: k,
                line: crate::notes::flowed::line_at(&line_of, source, m.lines.len()),
                anchor: anchor.clone(),
            });
        }
        offset += run.text.chars().count();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames() -> Frames {
        Frames {
            page_width: 12240,
            page_height: 15840,
            left: 1440,
            right: 1440,
            top: int(1440),
            bottom: int(1440),
            header: 720,
            footer: 720,
            page: 1,
            mode15: true,
            paragraph_top: int(2000),
            line_top: int(2000),
            line_bottom: int(2300),
            character: Q::from_integer(0),
            column: None,
        }
    }

    #[test]
    fn offsets_truncate() {
        assert_eq!(emu_twips(400_003), 629);
        assert_eq!(emu_twips(-400_003), -629);
        let mut anchor = Anchor {
            extent: (635 * 1000, 635 * 500),
            ..Anchor::default()
        };
        anchor.h.relative = Some("page".into());
        anchor.h.offset = Some(635 * 100);
        assert_eq!(horizontal(&anchor, &frames()), Some(int(100)));
        anchor.h.offset = None;
        anchor.h.align = Some("center".into());
        assert_eq!(horizontal(&anchor, &frames()), Some(int(5620)));
        anchor.v.relative = Some("paragraph".into());
        assert_eq!(vertical(&anchor, &frames()), Some(int(2000)));
        anchor.v.relative = Some("line".into());
        anchor.v.align = Some("bottom".into());
        assert_eq!(vertical(&anchor, &frames()), Some(int(1800)));
    }
}
