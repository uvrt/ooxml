//! Owns: the endnotes' continuation separator's line at the top of a page the endnotes go
//! on to, as drawn (docx2svg's `_continuation`): the separator's paragraph laid out in
//! the section of the page's first item, its one line where the paginator put it.
//!
//! Must not: place the line (the paginator has), or draw anything else of the endnotes
//! (they are the body's flow).

use super::footnotes::story_flow;
use super::lines::{DrawnLine, LineSource};
use super::stories::{StoryJob, StoryPage};
use super::walk::Walked;
use super::PageGeometry;
use crate::engine::DocumentLayout;
use crate::flow::FlowItem;
use crate::notes::{separator, NotesPart};
use ooxml_word_model::model::Block;
use std::sync::Arc;

/// The continuation separator's line page `geometry` opens with, as a flow of its own;
/// `None` where it opens with none.
pub fn page_opening(job: &mut StoryJob<'_, '_>, layout: &DocumentLayout, geometry: &PageGeometry) -> Option<StoryPage> {
    let line = geometry
        .lines
        .iter()
        .find(|l| matches!(l.source, LineSource::Opening { .. }))?;
    let section = layout
        .flow
        .items
        .get(line.source.item())
        .map(FlowItem::section)
        .and_then(|s| layout.sections.get(s))?;
    let paragraph = separator(job.cx.document, "continuationSeparator", NotesPart::Endnotes)
        .into_iter()
        .find(|b| matches!(b, Block::Paragraph(_)))?;
    let blocks: Arc<[Block]> = Arc::from(vec![paragraph]);
    let flow = story_flow(job, &blocks, &section.props);
    let item = flow.items.first()?.para()?;
    let drawn = DrawnLine {
        source: LineSource::Flow { item: 0 },
        block: Some(item.block),
        body: Some(item.body),
        ..*line
    };
    Some(StoryPage {
        kind: "opening",
        part: Arc::from(""),
        blocks,
        flow,
        walked: Walked {
            lines: vec![drawn],
            rows: Vec::new(),
            end: line.top + line.pitch,
        },
        clip: None,
        broken: crate::paginate::LineStore::new(),
    })
}
