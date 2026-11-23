//! Owns: the page numbers a field update writes, read from a layout's facts as docx-agent's
//! `word_fields` update and its page cross-reference work-ahead read them from docx2svg's
//! pages ([`PageNumbers`]): a `PAGE`, `NUMPAGES` or `SECTIONPAGES` field's result on the
//! page its paragraph is first drawn on (`_page_field`), and a `PAGEREF`'s -- and a table
//! of contents entry's -- the number of the page its target paragraph is first drawn on, in
//! that page's section's format (`_page_text`); and a `STYLEREF`'s, the text a painted page
//! draws for its run (`_style_reference`: [`style_reference_known`], [`drawn_text`]).
//!
//! This is not what the layout draws for a field (`super::field_values`: the page of the
//! line the field's run is on); it is what an update writes into the file.
//!
//! Must not: find a field's paragraph or a bookmark's (the caller names them by path), or
//! write anything.

use super::styleref::target_in;
use crate::engine::DocumentLayout;
use crate::paginate::PageInfo;
use crate::paint::PagePaint;
use ooxml_base::Locale;
use ooxml_word_model::fieldtext::{field_text, Uncomputable};
use ooxml_word_model::value::Text;
use ooxml_word_model::WordDocument;
use std::collections::HashMap;
use std::sync::Arc;

/// Why an update cannot say a field's result: the paragraph is not drawn, the layout
/// stopped before it, or the field's switches are not computed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the layout cannot say: {0}")]
pub struct Unknown(pub String);

/// The page numbers of one layout, as a field update reads them: what each page is comes
/// from the pagination, the page a paragraph is first drawn on from the layout's first
/// pages ([`DocumentLayout::first_pages`], found without placing a line).
#[derive(Debug, Clone)]
pub struct PageNumbers {
    first: Arc<HashMap<Text, usize>>,
    /// Each section's page number format, `None` for decimal or none stated.
    formats: Vec<Option<String>>,
    /// Each page the facts hold (those of a known section), in order.
    pages: Vec<PageInfo>,
    /// The layout stopped before the body's end.
    stopped: bool,
}

impl PageNumbers {
    /// The page numbers of `layout`.
    pub fn new(layout: &DocumentLayout) -> PageNumbers {
        let formats = layout
            .sections
            .list
            .iter()
            .map(|s| {
                s.props
                    .page_number_format
                    .as_deref()
                    .filter(|f| *f != "decimal")
                    .map(str::to_owned)
            })
            .collect();
        // The pages `LayoutFacts::of` makes facts of, and its `stopped`.
        let pages = layout
            .pagination
            .info
            .iter()
            .filter(|info| layout.sections.get(info.section).is_some())
            .copied()
            .collect();
        let stopped = layout
            .pagination
            .stopped
            .as_ref()
            .and_then(|s| layout.flow.items.get(s.item))
            .is_some();
        PageNumbers {
            first: Arc::clone(layout.first_pages()),
            formats,
            pages,
            stopped,
        }
    }

    /// Each paragraph path's first page.
    fn first(&self) -> &HashMap<Text, usize> {
        &self.first
    }

    /// Whether the paragraph at `path` is drawn: the page its first line is on.
    pub fn drawn_on(&self, path: &str) -> Option<usize> {
        self.first().get(path).copied()
    }

    fn format_of(&self, section: usize) -> Option<&str> {
        self.formats.get(section).and_then(Option::as_deref)
    }

    /// What the paginator knows of the page the paragraph at `path` is first drawn on.
    fn first_page(&self, path: &str) -> Result<&PageInfo, Unknown> {
        let page = *self
            .first()
            .get(path)
            .ok_or_else(|| Unknown(format!("{path} is not drawn")))?;
        self.pages
            .get(page)
            .ok_or_else(|| Unknown(format!("{path} is not on a page")))
    }

    /// What the paginator knows of each page, in order.
    pub fn pages(&self) -> impl Iterator<Item = &PageInfo> + '_ {
        self.pages.iter()
    }

    /// A `PAGE`, `NUMPAGES` or `SECTIONPAGES` field's result, `instruction` as written, in
    /// the paragraph at `path`: on the page that paragraph is first drawn on. The page
    /// count is every page, blank ones included; a section's, its pages that are not blank;
    /// neither is known past where the layout stopped.
    pub fn page_field(&self, instruction: &str, keyword: &str, path: &str) -> Result<String, Unknown> {
        let info = self.first_page(path)?;
        self.page_field_on(instruction, keyword, info)
    }

    /// A `PAGE`, `NUMPAGES` or `SECTIONPAGES` field's result on page `page` (0-based): a
    /// header's or footer's on a page that shows it.
    ///
    /// # Errors
    /// [`Unknown`] where there is no such page, the count is not known or a switch is not
    /// computed.
    pub fn page_field_at(&self, instruction: &str, keyword: &str, page: usize) -> Result<String, Unknown> {
        let info = self.pages.get(page).ok_or_else(|| Unknown(format!("no page {page}")))?;
        self.page_field_on(instruction, keyword, info)
    }

    fn page_field_on(&self, instruction: &str, keyword: &str, info: &PageInfo) -> Result<String, Unknown> {
        let known = !self.stopped;
        let pages = known.then(|| i64::try_from(self.pages.len()).unwrap_or(i64::MAX));
        let section_pages = known.then(|| {
            let count = self
                .pages
                .iter()
                .filter(|p| p.section == info.section && !p.blank)
                .count();
            i64::try_from(count).unwrap_or(i64::MAX)
        });
        field_text(
            instruction,
            keyword,
            info.number,
            (pages, section_pages),
            self.format_of(info.section),
        )
        .map_err(|Uncomputable(why)| Unknown(why))
    }

    /// What a `PAGEREF` (`instruction` as written) to the paragraph at `path` shows -- or a
    /// table of contents entry for it, with `PAGEREF x \h`: the number of the page that
    /// paragraph is first drawn on, in its section's format unless a switch formats it.
    pub fn page_reference(&self, instruction: &str, path: &str) -> Result<String, Unknown> {
        let info = self.first_page(path)?;
        Self::page_reference_on(instruction, info, self.format_of(info.section))
    }

    /// [`PageNumbers::page_reference`] to a paragraph first drawn on page `page` (0-based):
    /// one a text box draws, whose page the caller found.
    ///
    /// # Errors
    /// [`Unknown`] where there is no such page or a switch is not computed.
    pub fn page_reference_at(&self, instruction: &str, page: usize) -> Result<String, Unknown> {
        let info = self.pages.get(page).ok_or_else(|| Unknown(format!("no page {page}")))?;
        Self::page_reference_on(instruction, info, self.format_of(info.section))
    }

    fn page_reference_on(instruction: &str, info: &PageInfo, format: Option<&str>) -> Result<String, Unknown> {
        let instruction = instruction.replacen("PAGEREF", "PAGE", 1);
        field_text(&instruction, "PAGE", info.number, (None, None), format).map_err(|Uncomputable(why)| Unknown(why))
    }
}

/// Whether a `STYLEREF` (`instruction` as written) in `document`, laid out in `locale`, is
/// computed (`_style_reference`'s first question): its switches read and its style found, or
/// Word's error text for it known.
///
/// # Errors
/// [`Unknown`] where it is not: the field keeps its result.
pub fn style_reference_known(document: &WordDocument, locale: &Locale, instruction: &str) -> Result<(), Unknown> {
    target_in(document, locale, instruction)
        .map(|_| ())
        .map_err(|Uncomputable(why)| Unknown(why))
}

/// The text `page` draws for the run whose model path is `run` (`w:body/w:p[3]/w:r[2]`):
/// the characters of its spans, in drawing order, on the body's lines -- or, with `story`, on
/// the lines of that header's or footer's part (`word/header1.xml`). Empty where it draws
/// none (what Word stores for a `STYLEREF` that shows nothing).
pub fn drawn_text(page: &PagePaint, story: Option<&str>, run: &str) -> String {
    let part = |name: &str| name.trim_start_matches('/').to_owned();
    let wanted = story.map(part);
    page.lines
        .iter()
        .filter(|line| line.story.as_ref().map(|(_, p)| part(p)) == wanted)
        .flat_map(|line| line.spans.iter())
        .filter(|span| span.path == run)
        .flat_map(|span| span.chars.iter())
        .collect()
}
