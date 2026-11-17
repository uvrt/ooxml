//! Owns: GDI text -- `EXTTEXTOUT` and `TEXTOUT` placed at the reference point by the text
//! alignment, each character at its `Dx` advance, turned by the escapement, with the
//! opaque background and the current position updated.
//!
//! Measured on Windows: escapement turns text, and `EXTTEXTOUTW`'s `Dx` places each
//! character; a top-aligned string's baseline is the font's ascent below the reference
//! point.
//!
//! Must not: measure a face itself (the fonts layer's tables do, through `Faces`).

use super::paint::path_node;
use super::{transform_of, ClipChoice, Player};
use crate::bytes::strip;
use crate::dc::{self, OPAQUE};
use crate::error::Halt;
use crate::fonts::FontStyle;
use crate::geom;
use crate::pymath::{fsum, hypot, radians};
use ooxml_scene::{Anchor, Fill, Group, GroupTransform, Node, Paint, PathData, Rect, Text};

/// One text call's arguments.
pub(crate) struct TextCall<'t> {
    pub x: f64,
    pub y: f64,
    pub text: &'t str,
    /// Each code unit's advance, logical.
    pub dx: Option<Vec<i64>>,
    pub options: u32,
    pub rect: Option<(f64, f64, f64, f64)>,
}

impl Player<'_> {
    pub fn text(&mut self, call: TextCall<'_>) -> Result<(), Halt> {
        if !self.visible() {
            return Ok(());
        }
        let m = self.matrix();
        if let (true, Some((l, t, r, b))) = (call.options & dc::ETO_OPAQUE != 0, call.rect) {
            if r > l && b > t {
                self.flush()?;
                let data = PathData::new(vec![geom::rect(l, t, r, b)]);
                let fill = Some(Fill::of(Paint::Color(self.dc.bk_color)));
                self.write(
                    path_node(data, transform_of(&m), fill, Vec::new()),
                    true,
                    ClipChoice::Dc,
                )?;
            }
        }
        let text: String = call
            .text
            .chars()
            .take(self.limits.max_text_chars)
            .filter(|&ch| ch >= ' ' || ch == '\t')
            .collect();
        if strip(&text).is_empty() {
            return Ok(());
        }
        self.flush()?;
        let (x, y) = if self.dc.text_align & dc::TA_UPDATECP != 0 {
            self.dc.position
        } else {
            (call.x, call.y)
        };
        self.draw_text(&text, (x, y), call.dx.as_deref())
    }

    fn draw_text(&mut self, text: &str, at: (f64, f64), dx: Option<&[i64]>) -> Result<(), Halt> {
        let m = self.matrix();
        let dc = self.dc.clone();
        let font = &dc.font;
        let (px, py) = m.apply(at.0, at.1);
        let xs = Some(hypot(m.a, m.b)).filter(|v| *v != 0.0).unwrap_or(1.0);
        let ys = Some(hypot(m.c, m.d)).filter(|v| *v != 0.0).unwrap_or(1.0);
        let (ascent, descent) = self.faces.vertical_metrics(&font.face);
        let em = if font.height < 0.0 {
            -font.height
        } else if font.height > 0.0 {
            font.height / (ascent + descent)
        } else {
            16.0
        };
        let size = em * ys;
        let advances = dx.filter(|d| !d.is_empty()).map(|dx| advances_of(text, dx, xs));
        let total = advances
            .as_ref()
            .filter(|a| !a.is_empty())
            .map(|a| fsum(a.iter().copied()));
        let (base, start, anchor) = placement(dc.text_align, (ascent, descent), size, total);
        let angle = font.escapement / 10.0;
        let style = FontStyle {
            bold: font.weight >= 600,
            italic: font.italic,
            underline: font.underline,
            strike: font.strike,
        };
        let mut children = Vec::new();
        if let Some(t) = total.filter(|t| *t != 0.0 && dc.bk_mode == OPAQUE) {
            children.push(Node::Rect(Rect {
                x: start,
                y: base - ascent * size,
                width: t,
                height: (ascent + descent) * size,
                fill: Fill::of(Paint::Color(dc.bk_color)),
            }));
        }
        let x = positions(advances.as_deref(), start);
        children.push(Node::Text(Text {
            x,
            y: vec![base],
            transform: None,
            fill: Fill::of(Paint::Color(dc.text_color)),
            font: self.faces.font(&font.face, size, style, font.pitch_family),
            anchor,
            content: text.to_owned(),
        }));
        let group = Group {
            transform: Some(GroupTransform::TranslateRotate {
                x: px,
                y: py,
                angle: (angle != 0.0).then_some(-angle),
            }),
            children,
            ..Group::default()
        };
        self.write(Node::Group(group), true, ClipChoice::Dc)?;
        if let (true, Some(t)) = (dc.text_align & dc::TA_UPDATECP != 0, total) {
            if let Some(inverse) = m.invert() {
                let rad = radians(angle);
                let (ex, ey) = (px + t * rad.cos(), py - t * rad.sin());
                self.dc.position = inverse.apply(ex, ey);
            }
        }
        Ok(())
    }
}

/// Where the baseline is below the reference point, where the run starts along it, and
/// the anchor of a run with no advances, by the text alignment.
fn placement(align: u32, (ascent, descent): (f64, f64), size: f64, total: Option<f64>) -> (f64, f64, Option<Anchor>) {
    let vertical = align & dc::TA_BASELINE;
    let base = if vertical == dc::TA_BASELINE {
        0.0
    } else if vertical == dc::TA_BOTTOM {
        -descent * size
    } else {
        ascent * size
    };
    let (start, anchor) = match (total, align & dc::TA_CENTER) {
        (Some(t), dc::TA_CENTER) => (-t / 2.0, None),
        (Some(t), dc::TA_RIGHT) => (-t, None),
        (None, dc::TA_CENTER) => (0.0, Some(Anchor::Middle)),
        (None, dc::TA_RIGHT) => (0.0, Some(Anchor::End)),
        _ => (0.0, None),
    };
    (base, start, anchor)
}

/// Each character's position along the baseline from `start`, or `0` with no advances.
fn positions(advances: Option<&[f64]>, start: f64) -> Vec<f64> {
    match advances {
        Some(a) if !a.is_empty() => {
            let mut pos = start;
            a.iter()
                .map(|adv| {
                    let here = pos;
                    pos += adv;
                    here
                })
                .collect()
        }
        _ => vec![0.0],
    }
}

/// Each character's advance along the baseline in device pixels: the `Dx` entries of its
/// UTF-16 code units, summed.
fn advances_of(text: &str, dx: &[i64], xs: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut k = 0usize;
    for ch in text.chars() {
        let count = ch.len_utf16();
        let sum: i64 = dx
            .get(k.min(dx.len())..(k + count).min(dx.len()))
            .unwrap_or(&[])
            .iter()
            .sum();
        out.push(sum as f64 * xs);
        k += count;
    }
    out
}
