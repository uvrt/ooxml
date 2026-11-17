//! Owns: EMF+ text -- `DrawString` laid out in its box (measured on Windows: the box moved a
//! sixth of an em left and padded a sixth of an em on either side, lines the ascent and
//! descent apart, the first baseline 0.97 of the ascent below the top, an aligned block
//! 0.12 em taller than its lines, wrapped at spaces with the renderers' measured widths,
//! clipped to the moved box) and `DrawDriverString` (each character at its position, or
//! from the first with natural advances).
//!
//! Must not: measure a face itself (`Faces` does).

use super::objects::{Brush, FontObj, StringFormat};
use super::paint::{colour, Layer};
use super::Plus;
use crate::bytes::strip;
use crate::error::{Halt, ReadError, RecordError};
use crate::fonts::FontStyle;
use crate::geom;
use crate::player::transform_of;
use ooxml_scene::{Anchor, Group, GroupTransform, Matrix, Node, Path, PathData, ResourceKind, Text};

impl Plus<'_, '_> {
    /// The em in world units: a `UnitWorld` size as it is, any other unit in pixels at the
    /// header's resolution.
    fn font_size(&self, font: &FontObj) -> f64 {
        font.em * self.unit(font.unit, 1)
    }

    /// The paint text is drawn with; a brush drawn another way is its centre colour (a
    /// path gradient's; any other such brush makes the record unreadable, as it does the
    /// reference's).
    fn text_paint(&mut self, brush: Option<&Brush>) -> Result<Option<Layer>, RecordError> {
        let m = self.matrix();
        let Some(brush) = brush else {
            return Ok(None);
        };
        match self.paint_layers(brush, &m, false)? {
            None => {
                self.p.skip("EmfPlus path gradient text (drawn in its centre colour)");
                match brush {
                    Brush::PathGradient(g) => Ok(Some(colour(g.center_color))),
                    _ => Err(ReadError::Index.into()),
                }
            }
            Some(layers) => Ok(layers.last().copied()),
        }
    }

    fn style(font: &FontObj) -> FontStyle {
        FontStyle {
            bold: font.style & 1 != 0,
            italic: font.style & 2 != 0,
            underline: font.style & 4 != 0,
            strike: font.style & 8 != 0,
        }
    }

    pub(super) fn draw_string(
        &mut self,
        font: &FontObj,
        brush: Option<Brush>,
        fmt: StringFormat,
        rect: [f64; 4],
        text: &str,
    ) -> Result<(), RecordError> {
        let Some(paint) = self.text_paint(brush.as_ref())? else {
            return Ok(());
        };
        let text: String = text.chars().take(self.p.limits.max_text_chars).collect();
        let size = self.font_size(font);
        if size <= 0.0 || !size.is_finite() {
            return Ok(());
        }
        let style = Self::style(font);
        if fmt.flags & 0x0003 != 0 {
            self.p
                .skip("EmfPlusDrawString right-to-left or vertical (drawn left to right)");
        }
        let [x, y, w, h] = rect;
        let (ascent, descent, _gap) = self.p.faces.line_metrics(&font.family);
        let pad_em = size / 6.0;
        let x = x - pad_em;
        let line_height = (ascent + descent) * size;
        let wrap = w > 0.0 && fmt.flags & 0x1000 == 0;
        let inner = w - 2.0 * pad_em - (fmt.leading + fmt.trailing) * size;
        let lines = self.lines(&text, size, style.bold, &font.family, wrap.then_some(inner));
        if lines.is_empty() {
            return Ok(());
        }
        let total = lines.len() as f64 * line_height + 0.12 * size;
        let top = match (h > 0.0, fmt.line_align) {
            (true, 1) => y + (h - total) / 2.0,
            (true, 2) => y + h - total,
            (false, 1) => y - total / 2.0,
            (false, 2) => y - total,
            _ => y,
        };
        let (tx, anchor) = match fmt.align {
            1 => (x + w / 2.0, Some(Anchor::Middle)),
            2 => (x + w - pad_em - fmt.trailing * size, Some(Anchor::End)),
            _ => (x + pad_em + fmt.leading * size, None),
        };
        let font_attrs = self.p.faces.font(&font.family, size, style, 0);
        let parts: Vec<Node> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !strip(line).is_empty())
            .map(|(k, line)| {
                Node::Text(Text {
                    x: vec![tx],
                    y: vec![top + k as f64 * line_height + 0.97 * ascent * size],
                    transform: None,
                    fill: paint.fill(false),
                    font: font_attrs.clone(),
                    anchor,
                    content: line.clone(),
                })
            })
            .collect();
        if parts.is_empty() {
            return Ok(());
        }
        let mut body = parts;
        if w > 0.0 && h > 0.0 && fmt.flags & 0x4000 == 0 {
            let id = self.p.new_id(ResourceKind::Clip);
            self.p.define(ooxml_scene::Def::ClipPath {
                id,
                parent: None,
                paths: vec![Path {
                    data: PathData::new(vec![geom::rect(x, y, x + w, y + h)]),
                    ..Path::default()
                }],
            })?;
            body = vec![Node::Group(Group {
                clip: Some(id),
                children: body,
                ..Group::default()
            })];
        }
        Ok(self.write_wrapped(body)?)
    }

    /// Draws `body` in the current transform: in a group when there is one; as one drawn
    /// element either way.
    fn write_wrapped(&mut self, body: Vec<Node>) -> Result<(), Halt> {
        match transform_of(&self.matrix()) {
            Some(t) => self.write(Node::Group(Group {
                transform: Some(GroupTransform::Matrix(t)),
                children: body,
                ..Group::default()
            })),
            None => self.write_all(body),
        }
    }

    pub(super) fn draw_driver_string(
        &mut self,
        font: &FontObj,
        brush: Option<Brush>,
        options: u32,
        glyphs: &[u16],
        positions: &[(f64, f64)],
        matrix: Option<Matrix>,
    ) -> Result<bool, RecordError> {
        if options & 0x1 == 0 {
            self.p.skip("EmfPlusDrawDriverString by glyph index (not drawn)");
            return Ok(true);
        }
        if options & 0x2 != 0 {
            self.p.skip("EmfPlusDrawDriverString vertical (drawn horizontally)");
        }
        let paint = self.text_paint(brush.as_ref())?;
        let Some(paint) = paint.filter(|_| !glyphs.is_empty()) else {
            return Ok(true);
        };
        let text: String = glyphs
            .iter()
            .map(|&g| {
                let ok = (0x20..0xD800).contains(&g) || (0xE000..=0xFFFD).contains(&g);
                if ok {
                    char::from_u32(u32::from(g)).unwrap_or(' ')
                } else {
                    ' '
                }
            })
            .collect();
        let size = self.font_size(font);
        let font_attrs = self.p.faces.font(&font.family, size, Self::style(font), 0);
        let (x, y) = if options & 0x4 != 0 {
            let first = positions.first().copied().unwrap_or_default();
            (vec![first.0], vec![first.1])
        } else {
            (
                positions.iter().map(|p| p.0).collect(),
                positions.iter().map(|p| p.1).collect(),
            )
        };
        let mut m = self.matrix();
        if let Some(local) = matrix {
            m = local.then(&m);
        }
        self.write(Node::Text(Text {
            x,
            y,
            transform: transform_of(&m),
            fill: paint.fill(false),
            font: font_attrs,
            anchor: None,
            content: text,
        }))?;
        Ok(true)
    }

    /// `text` broken at its line breaks and, given a width, wrapped at spaces (a word longer
    /// than the line broken by characters), measured with the renderers' widths.
    fn lines(&self, text: &str, size: f64, bold: bool, family: &str, width: Option<f64>) -> Vec<String> {
        let family = if family.is_empty() { "Arial" } else { family };
        let measure = |s: &str| self.p.faces.width(s, size * 0.75, bold, family);
        let mut out = Vec::new();
        let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
        for paragraph in normalised.split('\n') {
            let Some(width) = width.filter(|w| *w > 0.0) else {
                out.push(paragraph.to_owned());
                continue;
            };
            let limit = width + 1e-6;
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if line.is_empty() || measure(&candidate) <= limit {
                    line = candidate;
                    while measure(&line) > limit && line.chars().count() > 1 {
                        // Break a word too long for the line by characters.
                        let chars: Vec<char> = line.chars().collect();
                        let mut cut = chars.len() - 1;
                        while cut > 1 && measure(&chars.get(..cut).unwrap_or(&[]).iter().collect::<String>()) > limit {
                            cut -= 1;
                        }
                        out.push(chars.get(..cut).unwrap_or(&[]).iter().collect());
                        line = chars.get(cut..).unwrap_or(&[]).iter().collect();
                    }
                    continue;
                }
                out.push(std::mem::replace(&mut line, word.to_owned()));
            }
            out.push(line);
        }
        out
    }
}
