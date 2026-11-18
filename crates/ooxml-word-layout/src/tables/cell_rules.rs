//! Owns: which floating drawings in a table cell docx2svg lays the cell out around, as
//! measured (`table.cell_anchors_unmodelled`, `cell_anchors_approximate`, `unwrapped`;
//! `floating.in_cell`, `cell_position_modelled`; `make_cell_anchor_probe.py`, ROADMAP F.16):
//! which are positioned against the cell, which positions no probe measured, and the wraps
//! not measured in a cell, laid out as if text did not wrap around them.
//!
//! Must not: position a drawing (`crate::floating` does) or lay a cell out
//! (`super::anchors` does).

use crate::floating::moves_text;
use ooxml_word_model::model::{Anchor, Paragraph, Run};
use ooxml_word_model::value::Text;

/// The wraps text goes around a drawing in a cell by: beside it, above and below it.
pub(super) const CELL_WRAPS: [&str; 4] = ["wrapSquare", "wrapTight", "wrapThrough", "wrapTopAndBottom"];

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
                out.push((
                    (r, k),
                    format!("is positioned by {why}, which no probe measured in a cell"),
                ));
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
        let measured = anchor(
            ("column", Some(100), None),
            ("paragraph", None, Some("center")),
            "wrapSquare",
        );
        assert_eq!(position_unmodelled(&measured, true), None);
        let inside = anchor(
            ("insideMargin", None, Some("left")),
            ("paragraph", Some(0), None),
            "wrapSquare",
        );
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
