//! Owns: the page numbers a field update writes, read from a layout's facts as docx-agent's
//! `word_fields` update and its page cross-reference work-ahead read them from docx2svg's
//! pages ([`PageNumbers`]): a `PAGE`, `NUMPAGES` or `SECTIONPAGES` field's result on the
//! page its paragraph is first drawn on (`_page_field`), and a `PAGEREF`'s -- and a table
//! of contents entry's -- the number of the page its target paragraph is first drawn on, in
//! that page's section's format (`_page_text`).
//!
//! This is not what the layout draws for a field (`super::field_values`: the page of the
//! line the field's run is on); it is what an update writes into the file.
//!
//! Must not: find a field's paragraph or a bookmark's (the caller names them by path), or
//! write anything.

use crate::engine::DocumentLayout;
use crate::facts::LayoutFacts;
use crate::paginate::PageInfo;
use ooxml_word_model::fieldtext::{field_text, Uncomputable};
use ooxml_word_model::value::Text;
use std::borrow::Borrow;

/// Why an update cannot say a field's result: the paragraph is not drawn, the layout
/// stopped before it, or the field's switches are not computed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the layout cannot say: {0}")]
pub struct Unknown(pub String);

/// The page numbers of one layout, as a field update reads them, over its facts `F` (a
/// reference, or whatever holds them for as long as it is asked). What each page is comes
/// from the pagination; the facts (the pages' lines) are read only where a paragraph's page
/// is asked for.
#[derive(Debug)]
pub struct PageNumbers<F> {
    facts: F,
    /// Each section's page number format, `None` for decimal or none stated.
    formats: Vec<Option<String>>,
    /// Each page the facts hold (those of a known section), in order.
    pages: Vec<PageInfo>,
    /// The layout stopped before the body's end.
    stopped: bool,
    /// Each paragraph path's first page (0-based, as `LayoutFacts::first_page`), made once
    /// when first asked: a field update and a check ask it for every field and reference
    /// (lane L4b: a walk of every page per field was most of a 3,000-page version's
    /// warnings).
    first: std::sync::OnceLock<std::collections::HashMap<Text, usize>>,
}

impl<F: Borrow<LayoutFacts>> PageNumbers<F> {
    /// The page numbers of `layout`, whose facts are `facts`.
    pub fn new(layout: &DocumentLayout, facts: F) -> PageNumbers<F> {
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
            facts,
            formats,
            pages,
            stopped,
            first: std::sync::OnceLock::new(),
        }
    }

    fn first(&self) -> &std::collections::HashMap<Text, usize> {
        self.first.get_or_init(|| {
            let mut first = std::collections::HashMap::new();
            for page in &self.facts.borrow().pages {
                for line in &page.lines {
                    first.entry(Text::clone(&line.path)).or_insert(page.page);
                }
            }
            first
        })
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
        let instruction = instruction.replacen("PAGEREF", "PAGE", 1);
        field_text(
            &instruction,
            "PAGE",
            info.number,
            (None, None),
            self.format_of(info.section),
        )
        .map_err(|Uncomputable(why)| Unknown(why))
    }
}
