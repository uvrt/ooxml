//! Owns: Word's fields edited as docx-agent's `edit/fields.py` edits them (as measured
//! with Word, its module docstring) -- found
//! ([`scan`], [`fields`]), inserted at a position ([`InsertField`]: complex or simple, the
//! result computed), a caption ([`InsertCaption`]), a table of contents ([`InsertToc`]),
//! and every result updated ([`UpdateFields`]) from the layout the host hands in through
//! the [`FieldLayout`] port.
//!
//! Must not: lay out, or write but through the sink.

mod caption;
mod facts;
mod insert;
mod insert_toc;
mod pages;
pub(crate) mod scan;
mod story;
mod switches;
mod toc;
mod update;
mod values;
mod write;

pub use caption::InsertCaption;
pub use facts::{field_facts, FieldFacts};
pub(crate) use insert::{place, put_back, Place};
pub use insert::{InsertCrossReference, InsertField};
pub use insert_toc::InsertToc;
pub use pages::{
    CharAdvance, EditLayout, FieldLayout, FieldPlace, FieldRun, LaidFields, LayOutError, PageReflow, PageShows, Unknown,
};
pub use story::{SetStoryNumbered, StoryNumber, StoryNumbered};
pub use update::UpdateFields;
pub use values::{day_in_picture, format_date, Day};

use crate::{Result, WordDoc};

/// What a field edit did, as docx-agent's `EditResult` says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FieldEdited {
    /// The id it names (a field's, a caption's paragraph), if any.
    pub id: Option<String>,
    /// Whether it changed the document.
    pub changed: bool,
    /// Ids it made.
    pub created: Vec<String>,
    /// Ids renamed by the stamping, old to new.
    pub renamed: Vec<(String, String)>,
    /// Results changed (an update's count).
    pub count: usize,
    /// The fields (and table of contents entries) whose result the layout cannot say.
    pub unknown: Vec<String>,
    /// What it could not do.
    pub warnings: Vec<String>,
}

/// A field as `word_fields`' list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldInfo {
    /// Its id (`fld:body/0`).
    pub id: String,
    /// Its kind (`PAGE`).
    pub keyword: String,
    /// Its instruction, trimmed.
    pub instruction: String,
    /// Its cached result's text.
    pub result: String,
    /// The paragraph it begins in.
    pub paragraph: Option<String>,
}

/// The places of the paragraphs the tables of contents of the part `part` of `docx` (a
/// reading of `doc`) span, each from its field's begin to its end, both included
/// (docx-agent's `_toc_paragraphs` of every `TOC` field `scan` finds), in order.
#[must_use]
pub fn toc_paragraphs(doc: &WordDoc, docx: &ooxml_word_model::docx::Docx, part: &str) -> Vec<Vec<u32>> {
    let Some(story) = docx.part(part) else {
        return Vec::new();
    };
    scan::all_fields(doc, docx)
        .iter()
        .filter(|located| located.part == part && located.found.keyword() == "TOC")
        .flat_map(|located| insert_toc::toc_paragraphs(story.root(), located))
        .collect()
}

/// Every field of `doc`, in story and document order (nested ones after their outer).
///
/// # Errors
/// When the document cannot be read.
pub fn fields(doc: &WordDoc) -> Result<Vec<FieldInfo>> {
    let docx = doc.docx()?;
    let mut out = Vec::new();
    for located in scan::all_fields(doc, &docx).iter() {
        let Some(part) = docx.part(&located.part) else { continue };
        let root = part.root();
        let paragraph = scan::paragraph_of(root, &located.found.begin)
            .and_then(|p| part.ids.paragraph_at(&p))
            .map(|p| p.id);
        out.push(FieldInfo {
            id: located.id(),
            keyword: located.found.keyword(),
            instruction: located.found.instruction.trim().to_owned(),
            result: scan::result_text(root, &located.found),
            paragraph,
        });
    }
    Ok(out)
}
