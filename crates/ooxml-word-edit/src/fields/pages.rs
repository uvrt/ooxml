//! Owns: [`FieldLayout`], the port a field update asks the pages through -- the layout of a
//! version of the document being edited, as docx-agent's `word_fields` reads its layout
//! (`_page_field`, `_page_text`, `_style_reference`; the first page showing a header or
//! footer, `rust/fields.py`'s `_story_page`) -- and [`Unknown`], what it answers where the
//! layout cannot say; and, through the same port, the reflow between two versions
//! ([`PageReflow`], docx-agent's `Reflow`), which `word_template` reports; and the pages a
//! body block is drawn on ([`LaidFields::blocks_pages`]), which a table's warnings and
//! `check` read.
//!
//! The edit core does not lay out (section 3: `ooxml-word-layout` is beside it, not below):
//! the host hands a command an adapter over its page engine, which answers in the words a
//! field's result is written in.
//!
//! Must not: lay out, or name anything of the layout's own (paths, pages' structures): a
//! place is a block's id as the edit core names it, or a page's number.

use ooxml_opc::Package;

/// Where a field is, or where a reference points: a body paragraph by its id (as the
/// version's bytes name it), or a page (0-based: a header's or footer's field, on the
/// first page that shows it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldPlace<'a> {
    /// A body paragraph (table cells' too).
    Body(&'a str),
    /// A page.
    Page(usize),
}

/// What a page shows of its section: the section's index, the kind of header and footer
/// it shows (`default`, `first`, `even`), and whether it is a blank page Word inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageShows {
    /// The section of its first line.
    pub section: usize,
    /// The header and footer kind it shows.
    pub kind: &'static str,
    /// A blank page Word inserts (it shows no header or footer).
    pub blank: bool,
}

/// Why the layout cannot say a field's result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the layout cannot say: {0}")]
pub struct Unknown(pub String);

/// One version laid out, as a field update reads it.
pub trait LaidFields {
    /// What a `PAGE`, `NUMPAGES` or `SECTIONPAGES` field (`instruction` as written) at
    /// `at` shows: on the page its paragraph is first drawn on, in that page's section's
    /// format unless a switch formats it.
    ///
    /// # Errors
    /// [`Unknown`] where the paragraph is not drawn, the count is not known (the layout
    /// stopped) or a switch is not computed.
    fn page_field(&self, instruction: &str, keyword: &str, at: FieldPlace<'_>) -> Result<String, Unknown>;

    /// What a `PAGEREF` (`instruction` as written; a table of contents entry's with
    /// `PAGEREF x \h`) to the paragraph at `at` shows.
    ///
    /// # Errors
    /// [`Unknown`] where that paragraph is not drawn or a switch is not computed.
    fn page_reference(&self, instruction: &str, at: FieldPlace<'_>) -> Result<String, Unknown>;

    /// Each page, in order.
    fn pages(&self) -> Vec<PageShows>;

    /// For each of `ids` (body blocks -- a paragraph, a table, a row -- by id, as the
    /// version's bytes name them), in their order, the 0-based pages it draws something on,
    /// in page order: a table's header rows drawn again count, a table taller than a page
    /// is on more than one (docx-agent's `layout().where`, `_taller_than_page`; `check`'s
    /// pages of a block). The ids are found and the pages read once for all of them.
    ///
    /// # Errors
    /// Per id, [`Unknown`] where it names no body block of the version, or the block is not
    /// drawn (the layout stopped before it). A layout that does not place blocks answers
    /// [`Unknown`] for every id (the default).
    fn blocks_pages(&self, ids: &[&str]) -> Vec<Result<Vec<usize>, Unknown>> {
        ids.iter()
            .map(|id| Err(Unknown(format!("this layout does not place blocks ({id})"))))
            .collect()
    }

    /// [`LaidFields::blocks_pages`] for one block.
    ///
    /// # Errors
    /// As [`LaidFields::blocks_pages`].
    fn block_pages(&self, id: &str) -> Result<Vec<usize>, Unknown> {
        self.blocks_pages(&[id])
            .pop()
            .unwrap_or_else(|| Err(Unknown(format!("{id} is not laid out"))))
    }
}

/// What changed in the pages between two versions (docx-agent's `Reflow`, as its tools
/// report it): pages by 1-based number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageReflow {
    /// The pages whose drawing differs, among the pages both versions know.
    pub changed: Vec<usize>,
    /// The page count before and after.
    pub page_count: (usize, usize),
    /// Body blocks whose first page changed, by id: (before, after), in the order the
    /// later version first draws them.
    pub moved: Vec<(String, (usize, usize))>,
    /// Pages past either version's last known page: not compared.
    pub unknown: Vec<usize>,
}

/// The layout of a document's versions, for the field updates of one call, and the
/// reflow an edit reports.
pub trait FieldLayout: Send + Sync {
    /// `package` (a version of the document the call edits) laid out.
    ///
    /// # Errors
    /// Why it was not laid out (a version the layout does not lay out yet): the update is
    /// left to the host.
    fn lay_out(&self, package: &Package) -> Result<Box<dyn LaidFields + '_>, String>;

    /// Which pages of `after` differ from `before`'s (two versions of the document,
    /// `after` made from `before`), ids as the versions' bytes name them.
    ///
    /// # Errors
    /// Why the pages cannot be compared (a version the layout does not lay out, or a
    /// layout that does not compare versions: the default).
    fn reflow(&self, before: &Package, after: &Package) -> Result<PageReflow, String> {
        let _ = (before, after);
        Err("this layout does not compare versions".to_owned())
    }
}
