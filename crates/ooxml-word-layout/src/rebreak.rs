//! Owns: a paragraph broken again while its pages are laid out (docx2svg's
//! `paginate.ParaSource` and `Para.set_lines`, `fit_width`, `plain`,
//! `move_hyphenated_word`): what its lines are broken from ([`ParaSource`]), its lines as
//! the paginator last broke them and what follows from them -- heights, breaks, notes,
//! anchors ([`Rebroken`]) -- and the view the paginator reads a paragraph's lines through,
//! the flow's or broken again ([`Lines`]).
//!
//! Must not: decide where a line goes (the paginator does), or keep anything between
//! layouts.

use crate::faces::Faces;
use crate::floating::{anchors_of, AnchorRef};
use crate::flow::ParaItem;
use crate::heights::LineHeight;
use crate::notes::flowed::{line_at, line_of_sources};
use crate::notes::NoteLines;
use crate::paragraph::{derive, LinesOf, MeasureContext, MeasuredParagraph};
use crate::sections::SectionGeometry;
use ooxml_base::Locale;
use ooxml_text::{break_from, Geometry, Line, Measured};
use ooxml_word_model::model::Paragraph;
use ooxml_word_model::{ResolvedParagraph, WordDocument};
use std::sync::Arc;

/// What a paragraph's lines are broken from, kept so that the paginator can break them
/// again: beside a drawing, in a column of another width, or before a hyphenated word Word
/// moves to the next page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParaSource {
    /// The paragraph.
    pub paragraph: Arc<Paragraph>,
    /// Its resolved formats.
    pub resolved: ResolvedParagraph,
    /// Where its lines start and end in its section's first column.
    pub geometry: Geometry,
    /// Its section has several columns.
    pub columns: bool,
    /// Its section, where its columns are of unequal widths: a line broken for a column
    /// other than the first carries that column's geometry.
    pub section: Option<SectionGeometry>,
    /// Every footnote reference: its index in the text and the note's lines.
    pub notes: Vec<(i64, NoteLines)>,
}

/// A line broken for a column other than its paragraph's first (one of unequal columns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnLine {
    /// The column's width, twips.
    pub width: i64,
    /// The paragraph's geometry in that column.
    pub geometry: Arc<Geometry>,
}

/// What a line broken again carries beyond its pieces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineExtra {
    /// Broken for a column other than the first.
    pub column: Option<ColumnLine>,
}

/// A paragraph's lines as the paginator broke them again, and what follows from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebroken {
    /// Its pieces and lines, each line's height and breaks.
    pub measured: Arc<MeasuredParagraph>,
    /// Per line, what it carries beyond its pieces.
    pub extra: Vec<LineExtra>,
    /// Its footnotes by line.
    pub notes: Vec<(usize, NoteLines)>,
    /// Its floating drawings by line.
    pub anchors: Vec<AnchorRef>,
    /// The first of its lines broken as a line of the whole column is, with every line
    /// after it.
    pub plain_from: usize,
}

/// A paragraph's lines as the paginator reads them: the flow's, or as broken again.
#[derive(Debug, Clone)]
pub enum Lines<'a> {
    /// As the flow broke them.
    Flow(&'a ParaItem),
    /// As the paginator broke them again.
    Again(Arc<Rebroken>),
}

impl Lines<'_> {
    /// The measure: pieces, lines, heights and breaks.
    pub fn measured(&self) -> &MeasuredParagraph {
        match self {
            Lines::Flow(p) => &p.measured,
            Lines::Again(r) => &r.measured,
        }
    }

    /// Each line's height.
    pub fn heights(&self) -> &[LineHeight] {
        &self.measured().heights
    }

    /// How many lines.
    pub fn count(&self) -> usize {
        self.heights().len()
    }

    /// The footnotes referenced on line `line`.
    pub fn notes_on(&self, line: usize) -> Vec<NoteLines> {
        let notes = match self {
            Lines::Flow(p) => &p.notes,
            Lines::Again(r) => &r.notes,
        };
        notes.iter().filter(|(l, _)| *l == line).map(|(_, n)| NoteLines::clone(n)).collect()
    }

    /// The floating drawings and their lines.
    pub fn anchors(&self) -> &[AnchorRef] {
        match self {
            Lines::Flow(p) => &p.anchors,
            Lines::Again(r) => &r.anchors,
        }
    }

    /// What line `line` carries beyond its pieces.
    pub fn extra(&self, line: usize) -> Option<&LineExtra> {
        match self {
            Lines::Flow(_) => None,
            Lines::Again(r) => r.extra.get(line),
        }
    }

    /// Where lines broken as lines of the whole column start.
    pub fn plain_from(&self) -> usize {
        match self {
            Lines::Flow(_) => 0,
            Lines::Again(r) => r.plain_from,
        }
    }
}

/// What breaking a paragraph again needs: the document and the faces.
#[derive(Clone, Copy)]
pub struct Breaker<'a> {
    /// The document.
    pub document: &'a WordDocument,
    /// The faces.
    pub faces: &'a dyn Faces,
    /// The locale.
    pub locale: &'a Locale,
}

impl Breaker<'_> {
    /// The lines the paragraph's rest from piece `start` breaks into -- in a column
    /// `width` twips wide where given (one of unequal columns), each carrying that
    /// column's geometry (`ParaSource.rest`).
    pub fn rest(&self, source: &ParaSource, pieces: &[ooxml_text::Piece], start: usize, first: bool, width: Option<i64>) -> Vec<(Line, LineExtra)> {
        match (width, &source.section) {
            (Some(width), Some(_)) => {
                let geometry = Arc::new(crate::input::geometry(self.document, &source.resolved.paragraph, width));
                break_from(pieces, start, &geometry, first, source.columns, None)
                    .into_iter()
                    .map(|line| {
                        let column = Some(ColumnLine {
                            width,
                            geometry: Arc::clone(&geometry),
                        });
                        (line, LineExtra { column })
                    })
                    .collect()
            }
            _ => break_from(pieces, start, &source.geometry, first, source.columns, None)
                .into_iter()
                .map(|line| (line, LineExtra::default()))
                .collect(),
        }
    }

    /// The width (twips) of the column a line was broken for (`width_of`).
    pub fn width_of(source: &ParaSource, extra: Option<&LineExtra>) -> i64 {
        match (extra.and_then(|e| e.column.as_ref()), &source.section) {
            (Some(column), _) => column.width,
            (None, Some(section)) => section.column_width(0),
            (None, None) => 0,
        }
    }

    /// `lines` taken as the paragraph's, everything that follows from them derived again
    /// (`set_lines`); `None` where a line's height cannot be measured.
    pub fn set_lines(
        &self,
        source: &ParaSource,
        current: &Lines<'_>,
        lines: Vec<(Line, LineExtra)>,
        plain_from: usize,
    ) -> Option<Rebroken> {
        let (text, extra): (Vec<Line>, Vec<LineExtra>) = lines.into_iter().unzip();
        let measured = Arc::new(Measured {
            pieces: current.measured().measured.pieces.clone(),
            lines: text,
        });
        let cx = MeasureContext {
            document: self.document,
            faces: self.faces,
            locale: self.locale,
        };
        let of = LinesOf {
            paragraph: &source.paragraph,
            resolved: &source.resolved,
            columns: source.columns,
        };
        let derived = derive(&cx, &of, measured).ok()?;
        let line_of = line_of_sources(&derived.measured.pieces, &derived.measured.lines);
        let count = derived.measured.lines.len();
        let mut notes: Vec<(usize, NoteLines)> = source
            .notes
            .iter()
            .map(|(at, laid)| (line_at(&line_of, *at, count), NoteLines::clone(laid)))
            .collect();
        notes.sort_by_key(|(line, _)| *line);
        let anchors = anchors_of(&source.paragraph, &derived);
        Some(Rebroken {
            measured: Arc::new(derived),
            extra,
            notes,
            anchors,
            plain_from,
        })
    }

    /// The paragraph's lines as they stand, each with what it carries.
    fn current(current: &Lines<'_>) -> Vec<(Line, LineExtra)> {
        let lines = &current.measured().measured.lines;
        lines
            .iter()
            .enumerate()
            .map(|(k, l)| (*l, current.extra(k).cloned().unwrap_or_default()))
            .collect()
    }

    /// The paragraph broken again from line `line` on for a column `width` twips wide,
    /// where its lines were broken for another (`fit_width`); `None` where nothing changes.
    pub fn fit_width(&self, source: &ParaSource, current: &Lines<'_>, line: usize, width: i64) -> Option<Rebroken> {
        source.section.as_ref()?;
        let lines = Breaker::current(current);
        let (at, extra) = lines.get(line)?;
        if Breaker::width_of(source, Some(extra)) == width {
            return None;
        }
        let pieces = &current.measured().measured.pieces;
        let mut out: Vec<(Line, LineExtra)> = lines.get(..line).unwrap_or_default().to_vec();
        out.extend(self.rest(source, pieces, at.start, line == 0, Some(width)));
        self.set_lines(source, current, out, current.plain_from())
    }

    /// The paragraph broken again from line `line` on as lines of the whole column, where
    /// an earlier placement broke them beside a drawing (`plain`).
    pub fn plain(&self, source: &ParaSource, current: &Lines<'_>, line: usize) -> Option<Rebroken> {
        if current.plain_from() <= line {
            return None;
        }
        let lines = Breaker::current(current);
        let pieces = &current.measured().measured.pieces;
        let start = lines.get(line).map_or(pieces.len(), |(l, _)| l.start);
        let mut out: Vec<(Line, LineExtra)> = lines.get(..line).unwrap_or_default().to_vec();
        out.extend(self.rest(source, pieces, start, line == 0, None));
        self.set_lines(source, current, out, line)
    }

    /// Line `line` ends a page in an automatic hyphen and Word moves only the word: the
    /// line ends before the word, and the paragraph is broken again from it
    /// (`move_hyphenated_word`); `None` where the word is the whole line.
    pub fn move_hyphenated_word(&self, source: &ParaSource, current: &Lines<'_>, line: usize) -> Option<Rebroken> {
        let lines = Breaker::current(current);
        let (broken, _) = lines.get(line)?;
        let pieces = &current.measured().measured.pieces;
        let mut k = broken.end.checked_sub(1)?;
        while k > broken.start && !pieces.get(k - 1).is_some_and(|p| p.ch.is_whitespace()) {
            k -= 1;
        }
        if k <= broken.start {
            return None;
        }
        let kept = Line {
            start: broken.start,
            end: k,
            forced: false,
            emergency: false,
            hyphenated: false,
        };
        let mut out: Vec<(Line, LineExtra)> = lines.get(..line).unwrap_or_default().to_vec();
        out.push((kept, LineExtra::default()));
        out.extend(self.rest(source, pieces, k, false, None));
        self.set_lines(source, current, out, current.plain_from().max(line + 1))
    }
}
