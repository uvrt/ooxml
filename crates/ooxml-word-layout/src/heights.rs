//! Owns: a paragraph's lines' heights (docx2svg's `lines`): which characters each line
//! holds ([`shares`]), what of them takes part in its height (no space; the paragraph
//! mark only on a line of spaces; the list label on the first line only), the height of
//! a line of text ([`LineHeight`], [`HeightRules::line_height`]) and of a line holding
//! inline pictures ([`HeightRules::object_line_height`]).
//!
//! Must not: break lines (the text engine has), or place them.

use crate::faces::Faces;
use crate::px::{CompactPx, Px};
use crate::vertical::{line_extent, mixed_line_pitch, tallest_natural, HeightItem, LineRule, Shape};
use ooxml_base::Q;
use ooxml_text::{Line, Piece, PieceKind};
use std::collections::HashMap;

/// One line's vertical facts, exact device px (`lines.LineHeight`), its lengths kept
/// compact ([`CompactPx`]) and read as [`Px`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LineHeight {
    pitch: CompactPx,
    text_above: CompactPx,
    text_below: CompactPx,
    natural: CompactPx,
    object_drop: Option<CompactPx>,
    /// The rule.
    pub rule: LineRule,
    /// `w:line`.
    pub line: i64,
    /// More than one face or size on it.
    pub mixed: bool,
}

impl LineHeight {
    /// A line of text: its pitch, the text's extent above the baseline (ascent and line
    /// gap) and below it, the natural height the rule scales or compares with, the rule,
    /// `w:line`, and whether more than one face or size is on it.
    pub fn new(
        pitch: Px,
        text_above: Px,
        text_below: Px,
        natural: Px,
        rule: LineRule,
        line: i64,
        mixed: bool,
    ) -> LineHeight {
        LineHeight {
            pitch: pitch.into(),
            text_above: text_above.into(),
            text_below: text_below.into(),
            natural: natural.into(),
            object_drop: None,
            rule,
            line,
            mixed,
        }
    }

    /// The same line holding inline pictures, the lowest a `drop` below its top.
    pub fn with_object_drop(self, drop: Px) -> LineHeight {
        LineHeight {
            object_drop: Some(drop.into()),
            ..self
        }
    }

    /// Its pitch.
    pub fn pitch(&self) -> Px {
        self.pitch.get()
    }

    /// The text's extent above the baseline (ascent and line gap).
    pub fn text_above(&self) -> Px {
        self.text_above.get()
    }

    /// The text's extent below the baseline.
    pub fn text_below(&self) -> Px {
        self.text_below.get()
    }

    /// The natural height the rule scales or compares with.
    pub fn natural(&self) -> Px {
        self.natural.get()
    }

    /// A line holding inline pictures: how far below its top a picture's bottom stands.
    pub fn object_drop(&self) -> Option<Px> {
        self.object_drop.as_ref().map(CompactPx::get)
    }

    /// The part of an `auto` multiple above one that is below the text
    /// (`paginate.multiple_extra`).
    pub fn multiple_extra(&self) -> Px {
        if self.rule == LineRule::Auto && self.line > 240 {
            self.pitch() - self.text_above() - self.text_below()
        } else {
            Px::ZERO
        }
    }

    /// What of the line must fit above the margin: its pitch less an `auto` multiple's
    /// extra (`paginate.fits`, rule `text-border`, before the border).
    pub fn fit(&self) -> Px {
        self.pitch() - self.multiple_extra()
    }
}

/// Which characters each line holds: per line, `(run, character)` for every character of
/// a run not hidden, by the line its piece is on (a character with no piece, the line
/// before's) (`lines.shares_of`).
pub fn shares(pieces: &[Piece], lines: &[Line], runs: &[(usize, bool, &str)]) -> Vec<Vec<(usize, char)>> {
    let mut line_of: HashMap<i64, usize> = HashMap::new();
    for (number, line) in lines.iter().enumerate() {
        for piece in pieces.get(line.start..line.end).unwrap_or_default() {
            if piece.source >= 0 && !matches!(piece.kind, PieceKind::Tab | PieceKind::Break | PieceKind::Object) {
                line_of.entry(piece.source).or_insert(number);
            }
        }
    }
    let mut out: Vec<Vec<(usize, char)>> = vec![Vec::new(); lines.len()];
    let mut index: i64 = 0;
    let mut current = 0;
    for &(run, vanish, text) in runs {
        let mut count = 0;
        for ch in text.chars() {
            if !vanish {
                current = line_of.get(&(index + count)).copied().unwrap_or(current);
                if let Some(line) = out.get_mut(current) {
                    line.push((run, ch));
                }
            }
            count += 1;
        }
        index += count;
    }
    out
}

/// The text items of a line: every character but a space; on a line of spaces alone, the
/// paragraph mark (`vertical.line_items`).
pub fn line_items(chars: &[(HeightItem, char)], mark: &HeightItem) -> Vec<HeightItem> {
    if chars.iter().any(|(_, c)| *c != ' ') {
        chars
            .iter()
            .filter(|(_, c)| *c != ' ')
            .map(|(i, _)| i.clone())
            .collect()
    } else {
        vec![mark.clone()]
    }
}

fn distinct(items: Vec<HeightItem>) -> Vec<HeightItem> {
    let mut out: Vec<HeightItem> = Vec::with_capacity(items.len());
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// What decides a line of text's height.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HeightKey {
    text: Vec<HeightItem>,
    label: Vec<HeightItem>,
    rule: LineRule,
    line: i64,
}

/// A paragraph's line rule and the faces lines are measured with, and the heights found,
/// by what decides them (`lines.line_height`'s cache).
pub struct HeightRules<'a> {
    /// The faces.
    pub faces: &'a dyn Faces,
    /// `w:spacing/@w:lineRule`.
    pub rule: LineRule,
    /// `w:spacing/@w:line`.
    pub line: i64,
    /// The document is in compatibility mode 15 or later.
    pub mode15: bool,
    cache: HashMap<HeightKey, Option<LineHeight>>,
}

impl<'a> HeightRules<'a> {
    /// The rules of a paragraph.
    pub fn new(faces: &'a dyn Faces, rule: LineRule, line: i64, mode15: bool) -> HeightRules<'a> {
        HeightRules {
            faces,
            rule,
            line,
            mode15,
            cache: HashMap::new(),
        }
    }

    /// The height of a line whose characters are `chars`; `label` the label's items (the
    /// first line's only); `None` where no face on it has metrics (`lines.line_height`).
    pub fn line_height(
        &mut self,
        chars: &[(HeightItem, char)],
        mark: &HeightItem,
        label: &[HeightItem],
    ) -> Option<LineHeight> {
        let key = HeightKey {
            text: distinct(line_items(chars, mark)),
            label: distinct(label.to_vec()),
            rule: self.rule,
            line: self.line,
        };
        if let Some(found) = self.cache.get(&key) {
            return found.clone();
        }
        let found = self.compute(&key.text, &key.label);
        self.cache.insert(key, found.clone());
        found
    }

    fn compute(&self, text: &[HeightItem], label: &[HeightItem]) -> Option<LineHeight> {
        let combined = line_extent(text, label, self.faces)?;
        let text_only = if label.is_empty() {
            None
        } else {
            line_extent(text, &[], self.faces)
        };
        let text_natural = tallest_natural(text, self.faces);
        let (above, below) = combined.shape.extent(combined.half_points);
        Some(LineHeight::new(
            mixed_line_pitch(&combined, text_only.as_ref(), self.rule, self.line, text_natural),
            above,
            below,
            combined.shape.natural(combined.half_points),
            self.rule,
            self.line,
            combined.mixed,
        ))
    }

    /// A line holding inline pictures `objects` (their heights, twips): a picture stands on
    /// the baseline with nothing below it; below mode 15 the line is at least the mark's
    /// natural height, in mode 15 the natural height of the runs holding the pictures
    /// (`runs`, their spaces' items); `label` the first line's label items
    /// (`lines.object_line_height`).
    pub fn object_line_height(
        &mut self,
        chars: &[(HeightItem, char)],
        mark: &HeightItem,
        label: &[HeightItem],
        objects: &[i64],
        runs: &[HeightItem],
    ) -> Option<LineHeight> {
        let picture = Px::twips(objects.iter().copied().max().unwrap_or(0));
        let mark_face = self.faces.metrics(mark.face.as_ref()?)?;
        let mark_natural = Shape::of(&mark_face).natural(mark.half_points);
        let text = if chars.iter().any(|(_, c)| !c.is_whitespace()) {
            self.line_height(chars, mark, label)
        } else {
            None
        };
        let mut above = text.as_ref().map_or(picture, |t| picture.max(t.text_above()));
        let below = text.as_ref().map_or(Px::ZERO, LineHeight::text_below);
        let mut natural = above + below;
        if text.is_none() && !self.mode15 {
            natural = natural.max(mark_natural);
        } else if text.is_none() && !runs.is_empty() {
            for item in runs {
                if let Some(face) = item.face.as_ref().and_then(|f| self.faces.metrics(f)) {
                    natural = natural.max(Shape::of(&face).natural(item.half_points));
                }
            }
            above = natural;
        }
        let pitch = match self.rule {
            LineRule::Exact => Px::twips(self.line),
            LineRule::AtLeast => natural.max(Px::twips(self.line)),
            LineRule::Auto if self.line != 240 => {
                let base = text.as_ref().map_or(mark_natural, LineHeight::natural);
                Px::quantise_q(
                    natural.value() + base.value() * (Q::new(i128::from(self.line), 240) - Q::from_integer(1)),
                )
            }
            LineRule::Auto => natural,
        };
        let descent = text.as_ref().map_or(Px::ZERO, LineHeight::text_below);
        let drop = match self.rule {
            LineRule::Exact => pitch * Q::new(4, 5),
            LineRule::AtLeast => pitch - descent,
            LineRule::Auto => natural - descent,
        };
        Some(LineHeight::new(pitch, above, natural - above, natural, self.rule, self.line, true).with_object_drop(drop))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_height_keeps_its_lengths_exact_in_half_the_room() {
        let third = Px::ratio(1, 3).unwrap_or(Px::ZERO);
        let h = LineHeight::new(
            Px::int(50),
            Px::twips(3),
            third,
            Px::int(46),
            LineRule::AtLeast,
            300,
            true,
        );
        assert_eq!(
            (h.pitch(), h.text_above(), h.text_below(), h.natural(), h.object_drop()),
            (Px::int(50), Px::twips(3), third, Px::int(46), None)
        );
        let wide = Px::ratio(1, i128::from(u32::MAX) + 2).unwrap_or(Px::ZERO);
        assert_eq!(h.clone().with_object_drop(wide).object_drop(), Some(wide));
        assert_eq!(std::mem::size_of::<LineHeight>(), 96);
    }
}
