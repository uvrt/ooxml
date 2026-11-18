//! Owns: what a check reads of a painted page's lines' text ([`LineText`], lane L4b), made
//! in Rust from the page's paint so that no line is read into Python for it: each line's
//! count of characters that are not white space (an empty paragraph, `spacers`), the sizes
//! its text is drawn at with their characters (`smallest_text`), and the extent of its ink
//! and its widest word (`column_fit`, a table's ink past the margins) -- docx-agent's
//! `layout_facts` rules (`_ink`, `_line_widths`, `text_sizes`), on the same spans.
//!
//! Must not: paint (the engine does), or judge what it measures.

use ooxml_word_layout::paint::{PaintedLine, Span, SpanKind};
use ooxml_word_layout::px::Px;

/// One painted line's text, as a check reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineText {
    /// Its paragraph's path.
    pub path: String,
    /// Its story's kind and part (a header's or footer's), where it is one's.
    pub story: Option<(&'static str, String)>,
    /// Its number in its paragraph.
    pub number: usize,
    /// Its characters that are not white space, every span's.
    pub inked: usize,
    /// Each text or label span not raised or lowered with characters that are not white
    /// space: its size (half points) and those characters.
    pub sizes: Vec<(i64, usize)>,
    /// From its first character that is not white space to the end of its last, device
    /// px (`_ink`).
    pub ink: Option<(Px, Px)>,
    /// Its longest run of characters without a space, device px (`_line_widths`).
    pub widest: Option<Px>,
}

/// White space as Python's `str.isspace` says it (Rust's `White_Space` and the four
/// information separators).
fn space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// A character's start and end on its line: its pen, and the next one's (the span's end
/// after its last).
fn extent(span: &Span, k: usize) -> Option<(Px, Px)> {
    let start = *span.xs.get(k)?;
    let end = span.xs.get(k + 1).copied().unwrap_or(span.end);
    Some((start, end))
}

impl LineText {
    /// `line`'s text facts.
    pub fn of(line: &PaintedLine) -> LineText {
        let mut inked = 0;
        let mut sizes = Vec::new();
        let mut ink: Option<(Px, Px)> = None;
        let mut widest: Option<Px> = None;
        let mut word: Option<(Px, Px)> = None;
        for span in &line.spans {
            let mine = span.chars.iter().filter(|c| !space(**c)).count();
            inked += mine;
            let scripted = span
                .vertical_align
                .as_deref()
                .is_some_and(|v| v == "superscript" || v == "subscript");
            if matches!(span.kind, SpanKind::Text | SpanKind::Label) && !scripted && mine > 0 {
                sizes.push((span.half_points, mine));
            }
            for (k, c) in span.chars.iter().enumerate() {
                let Some((start, end)) = extent(span, k) else { continue };
                if space(*c) {
                    if let Some((a, b)) = word.take() {
                        widest = Some(widest.map_or(b - a, |w| w.max(b - a)));
                    }
                    continue;
                }
                let (low, high) = (start.min(end), start.max(end));
                ink = Some(ink.map_or((low, high), |(l, h)| (l.min(low), h.max(high))));
                word = Some(word.map_or((start, end), |(a, _)| (a, end)));
            }
        }
        if let Some((a, b)) = word {
            widest = Some(widest.map_or(b - a, |w| w.max(b - a)));
        }
        LineText {
            path: line.path.to_string(),
            story: line.story.as_ref().map(|(kind, part)| (*kind, part.to_string())),
            number: line.number,
            inked,
            sizes,
            ink,
            widest: ink.and(widest.or(Some(Px::ZERO))),
        }
    }
}
