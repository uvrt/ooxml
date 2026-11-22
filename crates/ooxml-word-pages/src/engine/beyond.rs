//! Owns: what one page draws that its facts leave out, as the blank at its foot is measured
//! ([`PageEngine::beyond_facts`]), for a version or a trial laid out from one ([`Laid`]):
//! the lines its body's text boxes draw (laid out as a page is painted) and its footnotes'
//! lines (laid out with the page's parts) -- lines docx-agent's `Page.text_lines` counts.
//! A page none of whose paragraphs holds a text box is not painted. And the first page a
//! text box of the body draws one of its paragraphs on
//! ([`PageEngine::first_page_in_text_box`]), which a field there is numbered by.
//!
//! Must not: keep what it paints for a trial (a version's page is taken from, not added to,
//! the pages kept painted), or measure the blank (`layout_facts::room` does).

use super::trial::Trial;
use super::{paint_laid, PageEngine, Painter};
use crate::drawings::text_boxes::holds_text;
use crate::layout_facts::{story_kind, BeyondFacts};
use crate::LaidVersion;
use ooxml_word_layout::px::Px;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::model::{Block, InlineObject, Paragraph};
use std::collections::HashSet;
use std::sync::Arc;

/// What a page is measured in: a version the engine laid out, or a trial laid out from one.
#[derive(Debug, Clone, Copy)]
pub enum Laid<'a> {
    /// A version.
    Version(&'a LaidVersion),
    /// A trial.
    Trial(&'a Trial<'a>),
}

impl<'a> Laid<'a> {
    /// The version it is, or was laid out from.
    fn version(self) -> &'a LaidVersion {
        match self {
            Laid::Version(v) => v,
            Laid::Trial(t) => t.from,
        }
    }

    fn layout(self) -> &'a DocumentLayout {
        match self {
            Laid::Version(v) => &v.layout,
            Laid::Trial(t) => &t.layout,
        }
    }
}

impl PageEngine {
    /// What page `page` (0-based, blank pages counted) of `laid` draws beyond its facts,
    /// measured in the context it was laid out in: nothing known where `laid` is not of
    /// this engine's generation.
    pub fn beyond_facts(&mut self, laid: Laid<'_>, page: usize) -> BeyondFacts {
        BeyondFacts {
            text_boxes: self.text_box_foot(laid, page),
            notes: self.notes_top(laid, page),
        }
    }

    /// The first page (0-based) a text box of the body draws the paragraph the tool's id
    /// `id` names on -- a paragraph of a box, whose lines the facts leave out (docx-agent's
    /// `layout().where` reads every line docx2svg draws, its text boxes' too): the pages the
    /// body paragraph holding the box is drawn on are painted in order, until one draws it.
    /// `None` for an id of no paragraph in a body paragraph's content, or one no page draws.
    pub fn first_page_in_text_box(&mut self, version: &LaidVersion, id: &str) -> Option<usize> {
        let (host, place) = version.ids.in_paragraph(id)?;
        let drawn_in = |line: &ooxml_word_layout::paint::PaintedLine| {
            version.ids.place_of_path(&line.path).as_deref() == Some(place.as_slice())
        };
        version.facts().pages_of(&host).into_iter().find(|&page| {
            self.paint(version, page).is_some_and(|paint| {
                paint
                    .text_boxes
                    .iter()
                    .filter(|b| paint.drawings.get(b.slot).is_some_and(|s| s.story.is_none()))
                    .flat_map(|b| &b.lines)
                    .any(drawn_in)
            })
        })
    }

    /// What `version` (or a trial laid out from it) is measured with, while its generation
    /// is the engine's: its context's painter.
    fn painter_of(&mut self, version: &LaidVersion) -> Option<Painter<'_>> {
        self.generation
            .as_mut()?
            .painter(version, &self.locale, &self.budgets.held)
    }

    /// The bottom of the lowest line the page draws in a text box of the body's.
    fn text_box_foot(&mut self, laid: Laid<'_>, page: usize) -> Option<Px> {
        let facts = match laid {
            Laid::Version(v) => v.facts(),
            Laid::Trial(t) => t.facts(),
        };
        let blocks = &facts.pages.iter().find(|p| p.page == page)?.blocks;
        let drawn: HashSet<&str> = blocks.iter().map(|b| &**b).collect();
        if !draws_text_box(&laid.layout().document.body, &drawn) {
            return None;
        }
        let version = laid.version();
        let with = (self.glyph_size, self.decimal);
        let paint = match laid {
            Laid::Version(v) => self.painted_page(v, page, false)?,
            Laid::Trial(t) => {
                let painter = self.painter_of(version)?;
                Arc::new(paint_laid(painter, &t.layout, version, page, with)?)
            }
        };
        paint
            .text_boxes
            .iter()
            .filter(|b| paint.drawings.get(b.slot).is_some_and(|s| s.story.is_none()))
            .flat_map(|b| &b.lines)
            .filter(|l| l.story.is_none() && story_kind(&l.path) == "body")
            .map(|l| l.top + l.pitch)
            .max()
    }

    /// The top of the first line of the page's footnotes (their separator's included).
    fn notes_top(&mut self, laid: Laid<'_>, page: usize) -> Option<Px> {
        let layout = laid.layout();
        if page >= layout.page_count() {
            return None;
        }
        let painter = self.painter_of(laid.version())?;
        let parts = painter.layout.page_parts(painter.model, layout, page);
        parts
            .footnotes
            .iter()
            .flat_map(|notes| &notes.walked.lines)
            .map(|line| line.top)
            .min()
    }
}

/// Whether one of `blocks`' paragraphs the page draws (`drawn`: its blocks' keys) holds a
/// drawing with text, anchored or inline.
fn draws_text_box(blocks: &[Block], drawn: &HashSet<&str>) -> bool {
    blocks.iter().any(|block| match block {
        Block::Paragraph(p) => drawn.contains(&*p.path) && holds_text_box(p),
        Block::Table(t) => {
            drawn.contains(&*t.path)
                && t.rows
                    .iter()
                    .any(|row| row.iter().any(|cell| draws_text_box(cell, drawn)))
        }
    })
}

fn holds_text_box(paragraph: &Paragraph) -> bool {
    paragraph.runs.iter().any(|run| {
        run.anchors.iter().filter_map(|a| a.graphic.as_deref()).any(holds_text)
            || run
                .drawings
                .iter()
                .any(|d| matches!(d, InlineObject::Graphic(g) if holds_text(g)))
    })
}
