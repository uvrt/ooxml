//! Owns: drop caps (docx2svg's `paginate.DropCap`, `drop_cap_of`, measured): a paragraph
//! in a frame whose `w:dropCap` is `drop` or `margin`, before a paragraph of the body,
//! takes no room in the stack -- the paragraph after it carries it ([`DropCap`]): its
//! lines, how wide its text is, and where the text beside it starts.
//!
//! Must not: place the drop cap or the text beside it (the paginator does).

use super::FlowMaker;
use crate::paragraph::MeasuredParagraph;
use crate::px::Px;
use ooxml_base::Q;
use ooxml_text::PieceKind;
use ooxml_word_model::model::{Block, Paragraph};
use ooxml_word_model::value::{Props, Text, Value};
use std::sync::Arc;

/// What Word adds to a drop cap's width where the text beside it starts, layout units.
pub const DROP_CAP_EXTRA_UNITS: i64 = 103;

/// A drop cap as the paragraph it drops into carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropCap {
    /// Its paragraph's top-level index.
    pub body: usize,
    /// Its paragraph measured.
    pub measured: Arc<MeasuredParagraph>,
    /// Its paragraph's space before, twips.
    pub before: i64,
    /// How far its widest line reaches from the column's start, its indent included,
    /// layout units.
    pub width: Q,
    /// `w:framePr/@w:dropCap`.
    pub kind: Option<Text>,
    /// `w:hSpace`, twips.
    pub hspace: i64,
    /// `w:hAnchor` (`page` unstated).
    pub anchor: Text,
}

impl DropCap {
    /// Its height: its space before and its lines.
    pub fn height(&self) -> Px {
        self.measured
            .heights
            .iter()
            .fold(Px::twips(self.before), |sum, h| sum + h.pitch)
    }

    /// Whether text goes beside it: a `drop` cap's, and a `margin` cap's anchored to the
    /// text or the margin.
    pub fn beside(&self) -> bool {
        self.kind.as_deref() == Some("drop") || matches!(&*self.anchor, "text" | "margin")
    }

    /// Where the text beside it starts (device px on the page) and how far its text is
    /// then moved, from the column's start (a whole pixel) and the margin's exact edge
    /// (`text_start`).
    pub fn text_start(&self, column_start: Q, margin_exact: Q, mode15: bool) -> (Q, Q) {
        let lu = Px::layout_unit();
        let width = self.width * lu + Px::twips(self.hspace).value();
        let start = column_start + width + Q::from_integer(i128::from(DROP_CAP_EXTRA_UNITS)) * lu;
        let fract = |x: Q| x - x.floor();
        let step = if fract(width) < fract(margin_exact) {
            Q::from_integer(1)
        } else {
            Q::from_integer(0)
        };
        if mode15 {
            return (start, step);
        }
        let twip = Q::new(300, 1440);
        let twips = (start / twip + Q::new(1, 2)).floor() * twip;
        (twips, (twips + step - Q::new(1, 2)).ceil() - twips)
    }
}

/// Whether a paragraph's resolved properties make it a drop cap's.
pub fn is_drop_cap(pp: &Props) -> bool {
    frame_attr(pp, "dropCap").is_some_and(|v| matches!(v, "drop" | "margin"))
}

fn frame_attr<'p>(pp: &'p Props, name: &str) -> Option<&'p str> {
    match pp.get("framePr") {
        Some(Value::Attrs(attrs)) => attrs.iter().find(|(k, _)| &**k == name).map(|(_, v)| &**v),
        _ => None,
    }
}

impl FlowMaker<'_, '_> {
    /// The top-level paragraphs that are drop caps the flow lays out: each followed by a
    /// paragraph.
    pub(super) fn drop_caps(&mut self) -> Vec<usize> {
        let body = self.cx.blocks;
        let mut out = Vec::new();
        for (k, pair) in body.windows(2).enumerate() {
            if let [Block::Paragraph(_), Block::Paragraph(_)] = pair {
                let block = self.firsts.get(k).copied().unwrap_or(0);
                if self.props_of_block(block).is_some_and(|pp| is_drop_cap(&pp)) {
                    out.push(k);
                }
            }
        }
        out
    }

    /// A drop cap's paragraph laid out as a line of its own (`drop_cap_of`); `None` where
    /// it cannot be measured.
    pub(super) fn drop_cap(&mut self, paragraph: &Arc<Paragraph>, body: usize, section: usize) -> Option<DropCap> {
        let measured = self.measured(paragraph, body, section).ok()?;
        let block = self.firsts.get(body).copied().unwrap_or(0);
        let pp = self.props_of_block(block)?;
        let geometry = self.cx.sections.get(section)?;
        let at = crate::input::section_geometry(self.cx.document, &pp, geometry);
        let q = |u: ooxml_text::Units| {
            let (n, d) = u.parts();
            Q::new(n, d)
        };
        let m = &measured.measured;
        let mut width = Q::from_integer(0);
        for (number, line) in m.lines.iter().enumerate() {
            let mut total = q(if number == 0 { at.first_start } else { at.start });
            for j in line.start..line.end {
                let Some(piece) = m.pieces.get(j) else { continue };
                if matches!(piece.kind, PieceKind::Tab | PieceKind::Break | PieceKind::SoftHyphen) {
                    continue;
                }
                total += q(piece.width);
                if j + 1 < line.end {
                    total += q(piece.kern_next);
                }
            }
            width = width.max(total);
        }
        let int = |key: &str| pp.get(key).and_then(Value::int).unwrap_or(0);
        Some(DropCap {
            body,
            before: int("spacing.before"),
            width,
            kind: frame_attr(&pp, "dropCap").map(Text::from),
            hspace: frame_attr(&pp, "hSpace").and_then(|v| v.parse().ok()).unwrap_or(0),
            anchor: Text::from(frame_attr(&pp, "hAnchor").filter(|a| !a.is_empty()).unwrap_or("page")),
            measured,
        })
    }
}
