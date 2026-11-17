//! Owns: a footnote's (or a separator's) paragraphs as the lines a page's foot stacks
//! ([`NoteLine`], [`note_lines`]: each line's pitch, the note's own spacing above its
//! first), and where widow control lets a note end its page's part ([`note_minimum`],
//! [`note_prefix`]) -- docx2svg's `paginate.NoteLine`, `note_spacing`, `_note_lines`,
//! `_split_allowed`, `note_minimum`, `note_prefix`.
//!
//! Must not: decide which of a page's notes go on it (the paginator does), or draw.

use crate::faces::Faces;
use crate::input::geometry;
use crate::paragraph::{measure_paragraph, MeasureContext, ParagraphJob, Unmeasured};
use crate::px::Px;
use crate::sections::SectionGeometry;
use crate::vertical::paragraph_gap;
use ooxml_base::Locale;
use ooxml_text::Measurer;
use ooxml_word_model::model::{Block, Paragraph};
use ooxml_word_model::value::Value;
use ooxml_word_model::{Model, WordDocument};
use std::sync::Arc;

/// One line of a note as the page's foot stacks it: its pitch with the paragraph spacing
/// above it, and what its paragraph's widow control allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NoteLine {
    /// Its pitch, and on its paragraph's first line the spacing above it.
    pub height: Px,
    /// Its line in its paragraph.
    pub line: usize,
    /// How many lines its paragraph has.
    pub count: usize,
    /// Its paragraph has widow control.
    pub widow: bool,
    /// The note it is a line of (`None`: a separator's).
    pub note: Option<i64>,
    /// Its paragraph in the note.
    pub paragraph: usize,
    /// Its place among the note's lines.
    pub index: usize,
}

/// A note's lines.
pub type NoteLines = Arc<[NoteLine]>;

/// The twips a note's paragraph spacing adds to its room: its first paragraph's space
/// before and its last one's space after -- in mode 15 the larger, below it the space
/// before less the space after, none when negative (`note_spacing`).
pub fn note_spacing(before: i64, after: i64, mode15: bool) -> i64 {
    if mode15 {
        before.max(after)
    } else {
        (before - after).max(0)
    }
}

/// What measuring a note needs: the layout's faces and locale, the text engine's
/// measures, and the model its paragraphs resolve in.
pub struct NoteContext<'a, 'm> {
    /// The document.
    pub document: &'a WordDocument,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale measures are keyed under.
    pub locale: &'a Locale,
    /// The text engine's measures.
    pub measurer: &'m mut Measurer,
    /// The model.
    pub model: &'m mut Model,
}

/// Where a note's lines break: its section's first column, or one column `width` twips
/// wide.
#[derive(Debug, Clone, Copy)]
pub struct NoteWidth<'s> {
    /// The section.
    pub section: &'s SectionGeometry,
    /// One of its columns' width, where not its first column's.
    pub width: Option<i64>,
}

fn int(props: &ooxml_word_model::value::Props, key: &str) -> i64 {
    props.get(key).and_then(Value::int).unwrap_or(0)
}

/// A note's (or a separator's) paragraphs as lines, stacked as the body stacks them,
/// with the note's own spacing above its first line (`_note_lines`); `note` its id.
pub fn note_lines(
    cx: &mut NoteContext<'_, '_>,
    blocks: &[Block],
    width: NoteWidth<'_>,
    note: Option<i64>,
) -> Result<Vec<NoteLine>, Unmeasured> {
    let mut out: Vec<NoteLine> = Vec::new();
    let (mut previous_after, mut first_before, mut last_after) = (0, 0, 0);
    let mode15 = crate::input::rules(cx.document).mode15();
    let paragraphs: Vec<&Arc<Paragraph>> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            Block::Table(_) => None,
        })
        .collect();
    for (index, paragraph) in paragraphs.into_iter().enumerate() {
        let resolved = cx.model.resolve_paragraph(cx.document, paragraph);
        let pp = Arc::clone(&resolved.paragraph);
        let (column_width, columns) = match width.width {
            Some(w) => (w, false),
            None => (width.section.column_width(0), width.section.multi_column()),
        };
        let lines_at = geometry(cx.document, &pp, column_width);
        let mcx = MeasureContext {
            document: cx.document,
            faces: cx.faces,
            locale: cx.locale,
        };
        let job = ParagraphJob {
            paragraph,
            resolved: &resolved,
            geometry: &lines_at,
            columns,
            label: None,
        };
        let measured = measure_paragraph(&mcx, cx.measurer, &job)?;
        let before = int(&pp, "spacing.before");
        let gap = if index == 0 {
            first_before = before;
            Px::ZERO
        } else {
            paragraph_gap(previous_after, before).0
        };
        let widow = pp.get("widowControl").is_none_or(Value::truthy);
        let count = measured.heights.len();
        for (number, height) in measured.heights.iter().enumerate() {
            let at = out.len();
            out.push(NoteLine {
                height: height.pitch + if number == 0 { gap } else { Px::ZERO },
                line: number,
                count,
                widow,
                note,
                paragraph: index,
                index: at,
            });
        }
        previous_after = int(&pp, "spacing.after");
        last_after = previous_after;
    }
    if let Some(first) = out.first_mut() {
        first.height += Px::twips(note_spacing(first_before, last_after, mode15));
    }
    Ok(out)
}

/// The lines' heights together.
pub fn heights(lines: &[NoteLine]) -> Px {
    lines.iter().fold(Px::ZERO, |sum, l| sum + l.height)
}

/// Whether a note may end its page's part after its first `k` lines (`_split_allowed`).
fn split_allowed(lines: &[NoteLine], k: usize) -> bool {
    if k >= lines.len() {
        return true;
    }
    if k == 0 {
        return false;
    }
    let (Some(before), Some(after)) = (lines.get(k - 1), lines.get(k)) else {
        return true;
    };
    if after.line == 0 || !after.widow {
        return true;
    }
    before.line + 1 >= 2 && after.count - after.line >= 2
}

/// The least of a note that must go on its reference's page: its first lines up to the
/// first place widow control lets it split, all of it if nowhere (`note_minimum`).
pub fn note_minimum(lines: &[NoteLine]) -> Px {
    let k = (1..=lines.len())
        .find(|k| split_allowed(lines, *k))
        .unwrap_or(lines.len());
    heights(lines.get(..k).unwrap_or_default())
}

/// How many of a note's lines go in `room`: the most that fit where it may split
/// (`note_prefix`).
pub fn note_prefix(lines: &[NoteLine], room: Px) -> usize {
    let (mut best, mut total) = (0, Px::ZERO);
    for k in 1..=lines.len() {
        total += lines.get(k - 1).map_or(Px::ZERO, |l| l.height);
        if total > room {
            break;
        }
        if split_allowed(lines, k) {
            best = k;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(counts: &[usize], widow: bool) -> Vec<NoteLine> {
        let mut out = Vec::new();
        for (p, count) in counts.iter().enumerate() {
            for line in 0..*count {
                out.push(NoteLine {
                    height: Px::int(10),
                    line,
                    count: *count,
                    widow,
                    note: Some(1),
                    paragraph: p,
                    index: out.len(),
                });
            }
        }
        out
    }

    #[test]
    fn widow_control_splits() {
        let five = lines(&[5], true);
        assert_eq!(note_minimum(&five), Px::int(20));
        assert_eq!(note_prefix(&five, Px::int(35)), 3);
        assert_eq!(note_prefix(&five, Px::int(45)), 3);
        assert_eq!(note_prefix(&five, Px::int(15)), 0);
        let free = lines(&[5], false);
        assert_eq!(note_minimum(&free), Px::int(10));
        assert_eq!(note_prefix(&lines(&[1, 1], true), Px::int(15)), 1);
    }

    #[test]
    fn spacing() {
        assert_eq!(note_spacing(120, 240, true), 240);
        assert_eq!(note_spacing(240, 120, false), 120);
        assert_eq!(note_spacing(120, 240, false), 0);
    }
}
