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

use super::format::{field_text, Uncomputable};
use crate::engine::DocumentLayout;
use crate::facts::LayoutFacts;
use crate::paginate::PageInfo;

/// Why an update cannot say a field's result: the paragraph is not drawn, the layout
/// stopped before it, or the field's switches are not computed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the layout cannot say: {0}")]
pub struct Unknown(pub String);

/// The page numbers of one layout, as a field update reads them.
#[derive(Debug)]
pub struct PageNumbers<'f> {
    facts: &'f LayoutFacts,
    /// Each section's page number format, `None` for decimal or none stated.
    formats: Vec<Option<String>>,
}

impl<'f> PageNumbers<'f> {
    /// The page numbers of `layout`, whose facts are `facts`.
    pub fn new(layout: &DocumentLayout, facts: &'f LayoutFacts) -> PageNumbers<'f> {
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
        PageNumbers { facts, formats }
    }

    fn format_of(&self, section: usize) -> Option<&str> {
        self.formats.get(section).and_then(Option::as_deref)
    }

    /// What the paginator knows of the page the paragraph at `path` is first drawn on.
    fn first_page(&self, path: &str) -> Result<&PageInfo, Unknown> {
        let page = self
            .facts
            .first_page(path)
            .ok_or_else(|| Unknown(format!("{path} is not drawn")))?;
        self.facts
            .pages
            .get(page)
            .map(|p| &p.info)
            .ok_or_else(|| Unknown(format!("{path} is not on a page")))
    }

    /// A `PAGE`, `NUMPAGES` or `SECTIONPAGES` field's result, `instruction` as written, in
    /// the paragraph at `path`: on the page that paragraph is first drawn on. The page
    /// count is every page, blank ones included; a section's, its pages that are not blank;
    /// neither is known past where the layout stopped.
    pub fn page_field(&self, instruction: &str, keyword: &str, path: &str) -> Result<String, Unknown> {
        let info = self.first_page(path)?;
        let known = self.facts.stopped.is_none();
        let pages = known.then(|| i64::try_from(self.facts.pages.len()).unwrap_or(i64::MAX));
        let section_pages = known.then(|| {
            let count = self
                .facts
                .pages
                .iter()
                .filter(|p| p.info.section == info.section && !p.info.blank)
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
