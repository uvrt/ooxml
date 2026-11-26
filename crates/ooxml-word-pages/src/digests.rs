//! Owns: a painted page digested as docx-agent's layout summary digests it
//! (`_page_digest`, `_line_digest`): what it draws besides its lines (its frame, rules,
//! pictures and drawings, never the paths they are drawn for), and each line by its
//! paragraph's path and story; for a comparison of the layouts themselves
//! (`DocumentLayout.compare`, over `Page.text_lines`), its text boxes' lines too.
//!
//! Must not: paint (the engine hands it the page's paint), or keep anything.

use crate::engine::{LineDigest, PageDigests};
use ooxml_base::{Digest, DigestKey};
use ooxml_word_layout::paint::names::PageNames;
use ooxml_word_layout::paint::{PagePaint, PaintedLine};
use ooxml_word_model::docx::VersionNames;

/// Which lines a page's digests hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lines {
    /// The page's own (`Page.lines`): the layout summary's.
    Page,
    /// Every line of text it draws, its text boxes' after its own (`Page.text_lines`).
    WithTextBoxes,
}

/// `paint` digested under `key`, with the lines `which` says, each named through the
/// version's `names`.
pub(crate) fn digests(paint: &PagePaint, names: &VersionNames, key: DigestKey, which: Lines) -> PageDigests {
    let named = PageNames::of_page(names, paint);
    let digest = |value: &dyn std::fmt::Debug| ooxml_base::Digest::of(key, format!("{value:?}").as_bytes());
    // What docx-agent's summary digests (`_page_digest`, `_line_digest`): what is drawn
    // and where, never the paths it is drawn for (an edit before a page moves them).
    let rules: Vec<_> = paint
        .rules
        .iter()
        .map(|r| (r.kind, r.x, r.y, r.width, r.height, &r.color, &r.style))
        .collect();
    let pictures: Vec<_> = paint
        .pictures
        .iter()
        .map(|p| (&p.relationship, &p.part, p.x, p.y, p.width, p.height))
        .collect();
    let drawings: Vec<_> = paint
        .drawings
        .iter()
        .map(|d| (d.x, d.y, d.width, d.height, d.layer, d.story.as_ref().map(|s| s.0)))
        .collect();
    let frame = (paint.geometry.frame.width_px, paint.geometry.frame.height_px);
    let rest = digest(&(frame, rules, pictures, drawings));
    let lines = paint.lines.iter().map(|line| line_digest(line, &named, &digest));
    let lines = match which {
        Lines::Page => lines.collect(),
        Lines::WithTextBoxes => lines
            .chain(
                paint
                    .text_boxes
                    .iter()
                    .flat_map(|b| b.lines.iter())
                    .map(|line| line_digest(line, &named, &digest)),
            )
            .collect(),
    };
    PageDigests { rest, lines }
}

/// A line by its paragraph's path and story, with what it draws (its baseline, top,
/// pitch and spans) digested.
fn line_digest(
    line: &PaintedLine,
    names: &PageNames<'_>,
    digest: &dyn Fn(&dyn std::fmt::Debug) -> Digest,
) -> LineDigest {
    let spans: Vec<_> = line
        .spans
        .iter()
        .map(|s| {
            (
                &s.chars,
                &s.xs,
                s.y,
                s.end,
                &s.face,
                s.bold,
                s.italic,
                s.half_points,
                &s.color,
                &s.underline,
                (
                    s.strike,
                    s.double_strike,
                    s.highlight,
                    &s.shading,
                    s.kind,
                    &s.vertical_align,
                ),
            )
        })
        .collect();
    let drawn = digest(&(
        line.story.as_ref().map(|s| s.0),
        line.baseline,
        line.top,
        line.pitch,
        spans,
    ));
    (names.written(&line.name), line.story.clone(), drawn)
}
