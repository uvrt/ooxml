//! Owns: a page's header and footer as drawn (docx2svg's `_draw_stories` and `_draw_story`):
//! the story the page's kind shows, its page-number fields holding the page's values, laid
//! out as a document of its own and stacked from where Word puts it -- a header's first
//! line's box from `w:pgMar/@w:header` below the page's top, a footer's last from
//! `@w:footer` above its foot.
//!
//! Must not: lay out the body, or compute `STYLEREF` (a story's keeps the text Word cached).

use super::walk::{walk, WalkFrom, Walked};
use super::PageFrame;
use crate::engine::DocumentLayout;
use crate::fields::{field_text, page_counts};
use crate::flow::{Flow, FlowState};
use crate::paginate::PageInfo;
use crate::px::Px;
use crate::stories::{stack_height, story_flow, story_references, StoryContext};
use ooxml_word_model::model::{Block, Paragraph, Run, Table};
use ooxml_word_model::value::Text;
use ooxml_word_model::Model;
use std::sync::Arc;

/// A header or footer as a page draws it.
#[derive(Debug, Clone)]
pub struct StoryPage {
    /// `header` or `footer`.
    pub kind: &'static str,
    /// Its part (`word/header1.xml`).
    pub part: Text,
    /// Its blocks, the page's field values in.
    pub blocks: Arc<[Block]>,
    /// Its flow.
    pub flow: Flow,
    /// Its lines and rows as stacked on the page.
    pub walked: Walked,
}

/// What a page's stories need beyond the layout: the story context and its flow state.
pub struct StoryJob<'a, 'm> {
    /// The context stories are laid out in.
    pub cx: &'a StoryContext<'a>,
    /// The model.
    pub model: &'m mut Model,
    /// The stories' measures, kept by content.
    pub state: &'m mut FlowState,
}

/// `blocks` with each field's run holding its text on the page `info` describes, where it
/// can be computed (`substitute_blocks`).
fn with_fields(blocks: &[Block], text: &dyn Fn(&Run) -> Option<String>) -> Vec<Block> {
    fn paragraph(p: &Arc<Paragraph>, text: &dyn Fn(&Run) -> Option<String>) -> Arc<Paragraph> {
        if !p.runs.iter().any(|r| r.field.is_some()) {
            return Arc::clone(p);
        }
        let runs: Vec<Run> = p
            .runs
            .iter()
            .map(|run| match run.field.as_ref().and_then(|_| text(run)) {
                Some(t) if *t != *run.text => Run {
                    text: Arc::from(t.as_str()),
                    ..run.clone()
                },
                _ => run.clone(),
            })
            .collect();
        Arc::new(Paragraph {
            runs: runs.into(),
            ..(**p).clone()
        })
    }
    fn block(b: &Block, text: &dyn Fn(&Run) -> Option<String>) -> Block {
        match b {
            Block::Paragraph(p) => Block::Paragraph(paragraph(p, text)),
            Block::Table(t) => {
                let rows = t
                    .rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| cell.iter().map(|b| block(b, text)).collect())
                            .collect()
                    })
                    .collect();
                Block::Table(Arc::new(Table { rows, ..(**t).clone() }))
            }
        }
    }
    blocks.iter().map(|b| block(b, text)).collect()
}

/// The header and footer page `info` shows, laid out and stacked in `frame`.
pub fn page_stories(
    job: &mut StoryJob<'_, '_>,
    layout: &DocumentLayout,
    frame: &PageFrame,
    info: &PageInfo,
) -> Vec<StoryPage> {
    let document = job.cx.document;
    if info.blank {
        return Vec::new();
    }
    let references = story_references(&document.sections);
    let (Some((headers, footers)), Some(section)) = (references.get(info.section), document.sections.get(info.section))
    else {
        return Vec::new();
    };
    let props = &section.properties;
    let counts = page_counts(layout, info.section);
    let format = props.page_number_format.as_deref();
    let text = |run: &Run| -> Option<String> {
        let field = run.field.as_ref()?;
        if &*field.keyword == "STYLEREF" {
            return None;
        }
        field_text(&field.instruction, &field.keyword, info.number, counts, format).ok()
    };
    let mode15 = crate::input::rules(document).mode15();
    let mut out = Vec::new();
    for (kind, refs) in [("header", headers), ("footer", footers)] {
        let Some(id) = refs.get(info.story.name()) else {
            continue;
        };
        let Some(blocks) = document.stories.get(id).filter(|b| !b.is_empty()) else {
            continue;
        };
        let blocks: Arc<[Block]> = with_fields(blocks, &text).into();
        let flow = story_flow(job.cx, job.model, job.state, &blocks, props);
        let height = stack_height(&flow.items, mode15);
        let m = &props.margins;
        let top = if kind == "header" {
            Px::twips(m.header)
        } else {
            Px::twips(props.page_size.height - m.footer) - height
        };
        let walked = walk(
            &flow.items,
            &WalkFrom {
                top,
                left: frame.left,
                mode15,
                bottom: None,
            },
        );
        let part = document.story_parts.get(id).cloned().unwrap_or_else(|| id.clone());
        out.push(StoryPage {
            kind,
            part,
            blocks,
            flow,
            walked,
        });
    }
    out
}
