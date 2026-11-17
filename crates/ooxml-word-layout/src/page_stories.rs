//! Owns: the pages whose header or footer a `STYLEREF` makes taller or shorter (docx2svg's
//! `_Placer._story_heights`, measured in round 21): for each page of a pass, each story
//! holding a `STYLEREF` laid out with the texts its fields show on that page, and the
//! pages where that changes the heights the story has with its cached results -- what the
//! next pass lays those pages out with ([`PageStories`]).
//!
//! Must not: draw a story, or lay out the body.

use crate::engine::DocumentLayout;
use crate::fields::styleref::{has_style_reference, story_reference, with_story_references, Occurrence, StyleSearch};
use crate::fields::{drawn_pages, occurrence_pages};
use crate::flow::FlowState;
use crate::notes::NoteRole;
use crate::paginate::PageStories;
use crate::px::Px;
use crate::stories::{story_height, story_references, StoryContext, StoryHeights};
use ooxml_word_model::model::Block;
use ooxml_word_model::value::Text;
use ooxml_word_model::Model;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// A story laid out with its `STYLEREF` texts: its relationship id, the page's size and
/// margins, and the texts.
type HeightKey = (Text, i64, i64, [i64; 7], Vec<((usize, usize), Option<String>)>);

/// The stories' heights per text, kept across a version's passes.
#[derive(Debug, Default)]
pub struct StoryMemo {
    heights: HashMap<HeightKey, Px>,
}

/// What finding a pass's page stories reads beyond its layout.
pub struct PageStoryJob<'a> {
    /// How stories are laid out.
    pub cx: &'a StoryContext<'a>,
    /// Per top-level block, what of the endnotes placed in the flow it is (those are not
    /// searched).
    pub roles: &'a [Option<NoteRole>],
}

/// Whether any header or footer of the document holds a `STYLEREF`.
pub fn any_story_reference(stories: &std::collections::BTreeMap<Text, Arc<[Block]>>) -> bool {
    stories.values().any(|blocks| has_style_reference(blocks))
}

/// The pages of `layout` (one pass) whose header or footer heights, with their
/// `STYLEREF`s computed for the page, differ from their story's: each with its own
/// `(header, footer)`. Empty where no story holds a `STYLEREF`.
pub fn page_stories(
    job: &PageStoryJob<'_>,
    model: &mut Model,
    state: &mut FlowState,
    layout: &DocumentLayout,
    memo: &mut StoryMemo,
) -> PageStories {
    let mut out = PageStories::new();
    let document = &layout.document;
    if !any_story_reference(&document.stories) {
        return out;
    }
    let references = story_references(&document.sections);
    let placed = |body: usize| job.roles.get(body).is_some_and(Option::is_some);
    let mut search = StyleSearch::new(document, job.cx.locale, &placed);
    let drawn = drawn_pages(layout);
    let mut found_pages: HashMap<usize, Arc<[BTreeSet<usize>]>> = HashMap::new();
    let mut pages_of = |occurrences: &[Occurrence]| -> Arc<[BTreeSet<usize>]> {
        let key = occurrences.as_ptr() as usize;
        Arc::clone(
            found_pages
                .entry(key)
                .or_insert_with(|| occurrence_pages(layout, &drawn, occurrences).into()),
        )
    };
    for (page, info) in layout.pagination.info.iter().enumerate() {
        if info.blank {
            continue;
        }
        let (Some(section), Some((headers, footers))) =
            (document.sections.get(info.section), references.get(info.section))
        else {
            continue;
        };
        let props = &section.properties;
        let base = layout.stories.heights(info.section, info.story);
        let mut found = [base.0, base.1];
        for (slot, kinds) in [headers, footers].into_iter().enumerate() {
            let Some(id) = kinds.get(info.story.name()) else {
                continue;
            };
            let Some(blocks) = document
                .stories
                .get(id)
                .filter(|b| !b.is_empty() && has_style_reference(b))
            else {
                continue;
            };
            let mut text_of = |run: &ooxml_word_model::model::Run| {
                let instruction = run.field.as_ref().map(|f| f.instruction.clone())?;
                story_reference(&mut search, &instruction, page, &mut pages_of)
            };
            let (substituted, texts) = with_story_references(blocks, &mut text_of);
            let m = &props.margins;
            let key = (
                id.clone(),
                props.page_size.width,
                props.page_size.height,
                [m.top, m.right, m.bottom, m.left, m.header, m.footer, m.gutter],
                texts,
            );
            let height = match memo.heights.get(&key) {
                Some(h) => *h,
                None => {
                    let h = story_height(job.cx, model, state, &substituted, props);
                    memo.heights.insert(key, h);
                    h
                }
            };
            if let Some(f) = found.get_mut(slot) {
                *f = height;
            }
        }
        if (found[0], found[1]) != base {
            out.insert(page, (found[0], found[1]));
        }
    }
    out
}
