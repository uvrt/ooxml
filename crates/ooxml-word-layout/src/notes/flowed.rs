//! Owns: what the flow holds of notes (docx2svg's `paginate.flow`, its notes' part): each
//! footnote referenced in a paragraph measured as the lines its page's foot stacks, by
//! the line its reference is on ([`NoteMaker::notes_by_line`]); each section's separator
//! and continuation separator heights ([`NoteMaker::separators`]); an endnote's
//! continuation line ([`NoteMaker::continuation`]).
//!
//! Must not: place notes (the paginator does), or decide a paragraph's own lines.

use super::lines::{heights, note_lines, NoteContext, NoteLines, NoteWidth};
use super::numbering::{separator, NotesPart};
use crate::heights::LineHeight;
use crate::paragraph::{measure_paragraph, MeasureContext, ParagraphJob, Unmeasured};
use crate::px::Px;
use crate::sections::SectionGeometry;
use ooxml_text::{Line, Piece, PieceKind};
use ooxml_word_model::model::{Block, Paragraph, Section, SectionProps};
use std::collections::HashMap;
use std::sync::Arc;

/// Every footnote reference of a paragraph: its index in the paragraph's text and its
/// note.
pub fn references(paragraph: &Paragraph) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for run in paragraph.runs.iter() {
        for (at, mark) in run.breaks.iter() {
            let note = mark
                .strip_prefix("footnoteReference:")
                .and_then(|rest| rest.split(':').next())
                .and_then(|id| id.parse::<i64>().ok());
            if let Some(note) = note {
                out.push((i64::try_from(offset + at).unwrap_or(i64::MAX), note));
            }
        }
        offset += run.text.chars().count();
    }
    out
}

/// Text index -> the line its character is on (`_line_of_sources`).
pub fn line_of_sources(pieces: &[Piece], lines: &[Line]) -> HashMap<i64, usize> {
    let mut out = HashMap::new();
    for (number, line) in lines.iter().enumerate() {
        for piece in pieces.get(line.start..line.end).unwrap_or_default() {
            if piece.source >= 0 && !matches!(piece.kind, PieceKind::Tab | PieceKind::Break) {
                out.entry(piece.source).or_insert(number);
            }
        }
    }
    out
}

/// The line a text index is on: its own character's, the one before's, else the last.
pub fn line_at(line_of: &HashMap<i64, usize>, source: i64, lines: usize) -> usize {
    line_of
        .get(&source)
        .or_else(|| line_of.get(&(source - 1)))
        .copied()
        .unwrap_or(lines.saturating_sub(1))
}

/// A section's page and columns as a story of one section: what a page's note area is
/// laid out in (`note_area`'s section: no stories, no title page, no start type).
pub fn note_section(props: &SectionProps, paragraphs: usize) -> Section {
    let mut one = props.clone();
    one.start_type = None;
    one.headers = Arc::from(Vec::new());
    one.footers = Arc::from(Vec::new());
    one.title_page = false;
    Section {
        properties: Arc::new(one),
        first_paragraph: 0,
        last_paragraph: paragraphs,
    }
}

/// A note that cannot be laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unlaid(pub i64);

/// Notes measured for one flow, kept while it is built.
#[derive(Debug, Default)]
pub struct NoteMaker {
    notes: HashMap<(i64, usize, Option<i64>), Option<NoteLines>>,
    separators: HashMap<usize, (Px, Px)>,
    continuations: HashMap<usize, Option<LineHeight>>,
}

impl NoteMaker {
    /// Footnote `note` as lines in `width` of section `section` (`flow.note`); `None`
    /// where it cannot be measured.
    pub fn note(
        &mut self,
        cx: &mut NoteContext<'_, '_>,
        note: i64,
        section: (usize, &SectionGeometry),
        width: Option<i64>,
    ) -> Option<NoteLines> {
        let key = (note, section.0, width);
        if let Some(found) = self.notes.get(&key) {
            return found.clone();
        }
        let found = cx.document.footnotes.notes.get(&note).cloned().and_then(|blocks| {
            note_lines(
                cx,
                &blocks,
                NoteWidth {
                    section: section.1,
                    width,
                },
                Some(note),
            )
            .ok()
            .map(Arc::from)
        });
        self.notes.insert(key, found.clone());
        found
    }

    /// A paragraph's footnotes by the line their references are on, in the order
    /// referenced; the first note that cannot be measured, if one cannot.
    pub fn notes_by_line(
        &mut self,
        cx: &mut NoteContext<'_, '_>,
        paragraph: &Paragraph,
        lines: (&[Piece], &[Line]),
        section: (usize, &SectionGeometry),
    ) -> Result<Vec<(usize, NoteLines)>, Unlaid> {
        let found = references(paragraph);
        if found.is_empty() {
            return Ok(Vec::new());
        }
        let line_of = line_of_sources(lines.0, lines.1);
        let mut out: Vec<(usize, NoteLines)> = Vec::with_capacity(found.len());
        for (source, note) in found {
            let laid = self.note(cx, note, section, None).ok_or(Unlaid(note))?;
            out.push((line_at(&line_of, source, lines.1.len()), laid));
        }
        // By line, each line's in the order referenced.
        out.sort_by_key(|(line, _)| *line);
        Ok(out)
    }

    /// Section `section`'s footnote separator's and continuation separator's heights
    /// (`flow.separators`): in mode 15 a page a note goes on to opens with the separator.
    pub fn separators(&mut self, cx: &mut NoteContext<'_, '_>, section: (usize, &SectionGeometry)) -> (Px, Px) {
        if let Some(found) = self.separators.get(&section.0) {
            return *found;
        }
        let mode15 = crate::input::rules(cx.document).mode15();
        let mut height = |kind: &str| -> Px {
            let kind = if kind == "continuationSeparator" && mode15 {
                "separator"
            } else {
                kind
            };
            let blocks = separator(cx.document, kind, NotesPart::Footnotes);
            let width = NoteWidth {
                section: section.1,
                width: None,
            };
            note_lines(cx, &blocks, width, None).map_or(Px::ZERO, |lines| heights(&lines))
        };
        let found = (height("separator"), height("continuationSeparator"));
        self.separators.insert(section.0, found);
        found
    }

    /// The endnotes' continuation separator's line in section `section`: what a page the
    /// endnotes go on to holds above them (`continuation_line`); `None` where it cannot be
    /// measured.
    pub fn continuation(
        &mut self,
        cx: &mut NoteContext<'_, '_>,
        section: (usize, &SectionGeometry),
    ) -> Option<LineHeight> {
        if let Some(found) = self.continuations.get(&section.0) {
            return *found;
        }
        let found = continuation_line(cx, section.1).ok();
        self.continuations.insert(section.0, found);
        found
    }
}

fn continuation_line(cx: &mut NoteContext<'_, '_>, section: &SectionGeometry) -> Result<LineHeight, Unmeasured> {
    let blocks = separator(cx.document, "continuationSeparator", NotesPart::Endnotes);
    let Some(Block::Paragraph(paragraph)) = blocks.iter().find(|b| matches!(b, Block::Paragraph(_))) else {
        return Err(Unmeasured::Text("no continuation separator".into()));
    };
    let resolved = cx.model.resolve_paragraph(cx.document, paragraph);
    let geometry = crate::input::section_geometry(cx.document, &resolved.paragraph, section);
    let mcx = MeasureContext {
        document: cx.document,
        faces: cx.faces,
        locale: cx.locale,
    };
    let job = ParagraphJob {
        paragraph,
        resolved: &resolved,
        geometry: &geometry,
        columns: section.multi_column(),
        label: None,
    };
    let measured = measure_paragraph(&mcx, cx.measurer, &job)?;
    measured.heights.first().copied().ok_or(Unmeasured::NoFaceMetrics)
}
