//! Owns: a painted page digested as docx-agent's layout summary digests it
//! (`_page_digest`, `_line_digest`), path-free: what it draws besides its lines (its frame,
//! rules, pictures and drawings with their markup, its text boxes' content, the feed groups
//! each item is drawn in and their order, and the parts beside the text its drawing reads),
//! and each line by its paragraph's path and story with what it draws (its number in its
//! paragraph, its paragraph's id, its spans with their faces as drawn and their text
//! effects); for a comparison of the layouts themselves (`DocumentLayout.compare`, over
//! `Page.text_lines`), its text boxes' lines too. Everything a page's SVG or the changes
//! feed writes of the page is digested here, but the paths items are written under (an
//! edit before a page moves them), and what the engine's options fix for every version
//! (the glyph size): two pages whose digests agree are written alike (lane
//! fix-feed-compare).
//!
//! Must not: paint (the engine hands it the page's paint and the parts it reads), or keep
//! anything.

use crate::drawn_parts::PartRead;
use crate::engine::PageDigests;
use ooxml_base::{Digest, DigestKey};
use ooxml_word_layout::paint::drawings::DrawingSlot;
use ooxml_word_layout::paint::names::PageNames;
use ooxml_word_layout::paint::{Drawn, FaceNames, PagePaint, PaintedLine, Rule};
use ooxml_word_model::docx::VersionNames;
use ooxml_word_model::model::Graphic;

/// Which lines a page's digests hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lines {
    /// The page's own (`Page.lines`): the layout summary's.
    Page,
    /// Every line of text it draws, its text boxes' after its own (`Page.text_lines`).
    WithTextBoxes,
}

/// What a page's digests read beside its paint: the version's names, how its faces are
/// drawn, and the parts beside the text its drawing reads ([`crate::drawn_parts`]).
pub(crate) struct DigestOf<'a> {
    pub(crate) names: &'a VersionNames,
    pub(crate) faces: &'a dyn FaceNames,
    pub(crate) parts: &'a [PartRead],
    /// The settings' colour map (`w:clrSchemeMapping`): what a drawing's or a text effect's
    /// scheme colours are mapped through.
    pub(crate) colour_map: &'a std::collections::BTreeMap<ooxml_word_model::value::Text, ooxml_word_model::value::Text>,
}

/// `paint` digested under `key`, with the lines `which` says, each named through the
/// version's names.
pub(crate) fn digests(paint: &PagePaint, of: &DigestOf<'_>, key: DigestKey, which: Lines) -> PageDigests {
    let named = PageNames::of_page(of.names, paint);
    let digest = |value: &dyn std::fmt::Debug| ooxml_base::Digest::of(key, format!("{value:?}").as_bytes());
    let drawn_line = |line: &PaintedLine| line_drawn(line, of.faces, &digest);
    let frame = &paint.geometry.frame;
    let frame = (frame.width_px, frame.height_px, frame.width_pt, frame.height_pt);
    let rules: Vec<_> = paint.rules.iter().map(rule).collect();
    let pictures: Vec<_> = paint.pictures.iter().map(picture).collect();
    let drawings: Vec<_> = paint.drawings.iter().map(slot).collect();
    // A text box's content is drawn in its drawing's group, with no paths.
    let text_boxes: Vec<_> = paint
        .text_boxes
        .iter()
        .map(|t| {
            (
                t.slot,
                t.lines.iter().map(drawn_line).collect::<Vec<_>>(),
                t.rules.iter().map(rule).collect::<Vec<_>>(),
                t.pictures.iter().map(picture).collect::<Vec<_>>(),
                t.drawings.iter().map(slot).collect::<Vec<_>>(),
            )
        })
        .collect();
    // Which feed group each item is drawn in, by its block's durable id, in drawing order.
    let groups = ooxml_word_layout::paint::feed::group_ids(paint, of.names);
    // A drawing's or a text effect's scheme colours go through the colour map.
    let coloured = !paint.drawings.is_empty()
        || paint.text_boxes.iter().any(|t| !t.drawings.is_empty())
        || paint
            .lines
            .iter()
            .chain(paint.text_boxes.iter().flat_map(|t| t.lines.iter()))
            .any(|l| l.spans.iter().any(|s| s.effects.is_some()));
    let colour_map = coloured.then_some(of.colour_map);
    let rest = digest(&(
        frame, rules, pictures, drawings, text_boxes, groups, of.parts, colour_map,
    ));
    let line = |line: &PaintedLine| (named.written(&line.name), line.story.clone(), drawn_line(line));
    let lines = paint.lines.iter().map(line);
    let lines = match which {
        Lines::Page => lines.collect(),
        Lines::WithTextBoxes => lines
            .chain(paint.text_boxes.iter().flat_map(|b| b.lines.iter()).map(line))
            .collect(),
    };
    PageDigests { rest, lines }
}

/// A rule as drawn: its kind, box, colour and style (what it is drawn for is its group's).
fn rule(r: &Rule) -> impl std::fmt::Debug + '_ {
    (r.kind, r.x, r.y, r.width, r.height, &r.color, &r.style)
}

/// A picture as drawn: its part's relationship, box and crop.
fn picture(p: &Drawn) -> impl std::fmt::Debug + '_ {
    (&p.relationship, &p.part, p.x, p.y, p.width, p.height, p.crop)
}

/// A drawing's slot as drawn: its run and place among its anchors, its box, layer, stack
/// order and story, and its graphic.
fn slot(d: &DrawingSlot) -> impl std::fmt::Debug + '_ {
    (
        (d.run, d.index, d.inline),
        (d.x, d.y, d.width, d.height),
        (d.layer, d.order, &d.story),
        d.graphic.as_deref().map(graphic),
    )
}

/// A graphic as its painter reads it, without its structural path (an edit before it
/// moves it) or its text box's blocks (drawn as the text box's paint).
fn graphic(g: &Graphic) -> String {
    let children: Vec<String> = g.children.iter().map(|c| graphic(c)).collect();
    format!(
        "{:?}",
        (
            (&g.kind, g.offset, g.extent, g.rotation, g.flip_h, g.flip_v),
            (&g.relationship, &g.body, g.child_offset, g.child_extent),
            (&g.unsupported, &g.uri, &g.source),
            children,
        )
    )
}

/// What a line draws (its story, number in its paragraph, paragraph's id, baseline, top,
/// pitch and spans), digested.
fn line_drawn(line: &PaintedLine, faces: &dyn FaceNames, digest: &dyn Fn(&dyn std::fmt::Debug) -> Digest) -> Digest {
    let spans: Vec<_> = line
        .spans
        .iter()
        .map(|s| {
            (
                (&s.chars, &s.xs, s.y, s.end),
                (&s.face, s.bold, s.italic, faces.drawing_name(&s.face, s.bold, s.italic)),
                (s.half_points, &s.color, &s.underline, s.kind, &s.vertical_align),
                (s.strike, s.double_strike, s.highlight, &s.shading, &s.effects),
            )
        })
        .collect();
    digest(&(
        &line.story,
        (line.number, &line.paragraph_id),
        (line.baseline, line.top, line.pitch),
        spans,
    ))
}
