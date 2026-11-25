//! Owns: what a list edit ([`super::ListEdit`]) depends on, as a read by condition
//! ([`ListsDepend`]) instead of the whole document: the paragraphs in a list (numbered
//! directly or by their style) on its snapshot or on the head, the paragraphs it names, the
//! paragraphs of text boxes (which it keeps in step), tables and rows, and every key that is
//! not a block -- the numbering, the styles, the annotation ids, any part put whole. An edit
//! of a paragraph in none of those, committed meanwhile, is no conflict: the list edit's
//! outcome is the same on the head.
//!
//! Must not: read more than the one key it is asked about on the head (it runs at every
//! commit that checks it); name keys for the history (an undo or redo treats it as a read of
//! everything, as before).

use super::read::{numbered_by_style, paragraph_style_id};
use crate::doc::{Key, WordDoc};
use ooxml_store::ReadPredicate;
use ooxml_word_model::docx::{at_path, find_path, Docx, StoryPart};
use ooxml_word_model::ids::Entry;
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A list edit's read by condition. Holds, by part and id, the paragraphs that were in a
/// list on its snapshot and the ones it names: a write to any of them conflicts, so a
/// paragraph that left a list meanwhile does too ([`ReadPredicate`]'s contract).
#[derive(Debug)]
pub(crate) struct ListsDepend {
    held: HashSet<(Arc<str>, Arc<str>)>,
}

impl ListsDepend {
    /// The paragraphs of `docx` (the edit's snapshot) in a list, and `named`.
    pub(crate) fn of(docx: &Docx, named: impl IntoIterator<Item = (Arc<str>, Arc<str>)>) -> Self {
        let mut held: HashSet<(Arc<str>, Arc<str>)> = named.into_iter().collect();
        // whether a style numbers its paragraphs, by the style a paragraph names: worked out
        // once per style, not once per paragraph
        let mut numbers: HashMap<Option<String>, bool> = HashMap::new();
        for story in &docx.parts {
            let part: Arc<str> = Arc::from(story.part.as_str());
            for paragraph in story.ids.paragraphs().iter() {
                let element = &paragraph.element;
                let direct = find_path(element, &["pPr", "numPr"]).is_some();
                let listed = direct
                    || *numbers
                        .entry(paragraph_style_id(element))
                        .or_insert_with(|| numbered_by_style(docx, element).is_some());
                if listed {
                    held.insert((Arc::clone(&part), Arc::from(paragraph.id.as_str())));
                }
            }
        }
        Self { held }
    }
}

impl ReadPredicate<WordDoc> for ListsDepend {
    fn depends(&self, head: &WordDoc, key: &Key) -> bool {
        match key {
            Key::Block { part, id } => {
                if self.held.contains(&(Arc::clone(part), Arc::clone(id))) {
                    return true;
                }
                let Ok(docx) = head.docx() else { return true };
                docx.part(part).is_none_or(|story| block_counts(&docx, story, id))
            }
            // an id drawn elsewhere is merged where the change lands
            Key::Seq(_) => false,
            Key::Part(_) | Key::Root(_) | Key::Annotations | Key::People | Key::StyleUse(_) => true,
        }
    }
}

/// Whether the block `id` of `story` counts: a paragraph in a list or in a text box, a
/// table or row, or one the head no longer has.
fn block_counts(docx: &Docx, story: &StoryPart, id: &str) -> bool {
    match story.ids.get(id) {
        Some(Entry::Paragraph(k)) => story
            .ids
            .paragraphs()
            .get(k)
            .is_none_or(|p| in_list(docx, &p.element) || in_text_box(story.root(), &p.path)),
        Some(Entry::Table(_)) | None => true,
    }
}

/// Whether `paragraph` is numbered: directly, or by its style.
fn in_list(docx: &Docx, paragraph: &Element) -> bool {
    find_path(paragraph, &["pPr", "numPr"]).is_some() || numbered_by_style(docx, paragraph).is_some()
}

/// Whether the element at `path` below `root` is within a text box's content.
fn in_text_box(root: &Arc<Element>, path: &[u32]) -> bool {
    (1..path.len()).any(|end| {
        path.get(..end)
            .and_then(|prefix| at_path(root, prefix))
            .is_some_and(|e| e.name().local() == "txbxContent")
    })
}
