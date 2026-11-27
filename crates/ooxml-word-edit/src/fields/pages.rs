//! Owns: [`FieldLayout`], the port a field update asks the pages through -- the layout of a
//! version of the document being edited, as docx-agent's `word_fields` reads its layout
//! (`_page_field`, `_page_text`, `_style_reference`; the first page showing a header or
//! footer, `rust/fields.py`'s `_story_page`) -- the run a field's text is drawn for
//! ([`FieldRun`]), and [`Unknown`], what it answers where the
//! layout cannot say; and, through the same port, the reflow between two versions
//! ([`PageReflow`], docx-agent's `Reflow`), which `word_template` reports; and the pages a
//! body block is drawn on ([`LaidFields::blocks_pages`]), which a table's warnings and
//! `check` read; and a character's advance in the fonts a version is laid out with
//! ([`LaidFields::advance`]), which a table of contents' numbered entry is measured with.
//!
//! The edit core does not lay out (section 3: `ooxml-word-layout` is beside it, not below):
//! the host hands a command an adapter over its page engine, which answers in the words a
//! field's result is written in.
//!
//! An edit asks the port through [`EditLayout`]: the port with what stops laying a version
//! out (the call's limit on laying out, or its cancellation); a stop is a timeout that
//! changes nothing ([`EditError::LayoutStopped`]).
//!
//! Must not: lay out, or name anything of the layout's own (pages' structures): a place is a
//! block's id as the edit core names it, or a page's number; a run, its path as the Word
//! model names it (`ooxml_word_model::docx::cells::story_model_path`).

use crate::error::EditError;
use ooxml_base::Cancel;
use ooxml_word_model::WordPackage;
use std::time::Duration;

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

/// The run a field's text is drawn for: its story part (`word/header1.xml`) and its path as
/// the Word model names it (`w:body/w:p[3]/w:r[2]`: a complex field's, the run holding its
/// instruction's first piece; a simple field's, its `w:fldSimple`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldRun<'a> {
    /// Its story part.
    pub part: &'a str,
    /// Its model path.
    pub path: &'a str,
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

/// A character's advance in a face: font units, and the face's units per em.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharAdvance {
    /// The advance, font units.
    pub advance: u32,
    /// The face's units per em.
    pub units_per_em: u32,
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

    /// What a `STYLEREF` (`instruction` as written) whose run is `run` shows at `at`: the
    /// text the layout draws for that run on the page its paragraph is first drawn on, or
    /// (a header's or footer's) on the page `at` names; empty where it draws none.
    ///
    /// # Errors
    /// [`Unknown`] where the instruction is not computed (a switch, a style not measured),
    /// or the paragraph is not drawn.
    fn style_reference(&self, instruction: &str, at: FieldPlace<'_>, run: FieldRun<'_>) -> Result<String, Unknown>;

    /// Each page, in order.
    fn pages(&self) -> Vec<PageShows>;

    /// The advance of `ch` in the face the fonts this version is laid out with measure
    /// `family` (bold, italic) with -- a table of contents' list number is measured so
    /// (docx-agent's `_TocLooks._width`); `None` where no font says (the default: a layout
    /// that does not measure).
    fn advance(&self, family: &str, bold: bool, italic: bool, ch: char) -> Option<CharAdvance> {
        let _ = (family, bold, italic, ch);
        None
    }

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
    /// `package` (a version of the document the call edits) laid out, stopped where
    /// `cancel` says so.
    ///
    /// # Errors
    /// [`LayOutError::Stopped`] where `cancel` stopped it; why it was not laid out (a
    /// version the layout does not lay out yet: the update is left to the host).
    fn lay_out(&self, package: &WordPackage, cancel: &dyn Cancel) -> Result<Box<dyn LaidFields + '_>, LayOutError>;

    /// Which pages of `after` differ from `before`'s (two versions of the document,
    /// `after` made from `before`), ids as the versions' bytes name them; laying either out
    /// stopped where `cancel` says so.
    ///
    /// # Errors
    /// [`LayOutError::Stopped`] where `cancel` stopped it; why the pages cannot be compared
    /// (a version the layout does not lay out, or a layout that does not compare versions:
    /// the default).
    fn reflow(
        &self,
        before: &WordPackage,
        after: &WordPackage,
        cancel: &dyn Cancel,
    ) -> Result<PageReflow, LayOutError> {
        let _ = (before, after, cancel);
        Err(LayOutError::Unsupported(
            "this layout does not compare versions".to_owned(),
        ))
    }
}

/// Why [`FieldLayout::lay_out`] laid no version out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayOutError {
    /// What stops it said stop (the call's limit on laying out passed, or it was cancelled).
    Stopped,
    /// The layout does not lay the version out (in its words).
    Unsupported(String),
}

/// The layout an edit reads: the host's port, with what stops laying a version out and
/// the limit that stop keeps, as a stop is said.
#[derive(Clone, Copy)]
pub struct EditLayout<'l> {
    /// The host's layout.
    pub layout: &'l dyn FieldLayout,
    /// Says stop where the call's limit on laying out passed or the call is cancelled.
    pub stop: &'l dyn Cancel,
    /// That limit, where the call has one.
    pub limit: Option<Duration>,
}

impl std::fmt::Debug for EditLayout<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditLayout")
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl<'l> EditLayout<'l> {
    /// `package` laid out, as an edit says why not.
    ///
    /// # Errors
    /// [`EditError::LayoutStopped`] where laying it out was stopped (nothing changed);
    /// [`EditError::Unsupported`] where the layout does not lay it out.
    pub fn lay_out(&self, package: &WordPackage) -> Result<Box<dyn LaidFields + 'l>, EditError> {
        self.layout.lay_out(package, self.stop).map_err(|error| match error {
            LayOutError::Stopped => EditError::LayoutStopped(self.limit),
            LayOutError::Unsupported(why) => EditError::Unsupported(format!("the layout of this version: {why}")),
        })
    }

    /// Which pages of `after` differ from `before`'s ([`FieldLayout::reflow`]), as an edit
    /// says why not: inside, why the layout cannot compare them (the edit goes on and says
    /// so).
    ///
    /// # Errors
    /// [`EditError::LayoutStopped`] where laying either version out was stopped (nothing
    /// changes).
    pub fn reflow(&self, before: &WordPackage, after: &WordPackage) -> Result<Result<PageReflow, String>, EditError> {
        match self.layout.reflow(before, after, self.stop) {
            Ok(found) => Ok(Ok(found)),
            Err(LayOutError::Stopped) => Err(EditError::LayoutStopped(self.limit)),
            Err(LayOutError::Unsupported(why)) => Ok(Err(why)),
        }
    }
}
