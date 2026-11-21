//! Owns: `check`'s layout facts read from one laid version and the reading of the same
//! version (docx-agent's `layout_facts._Facts`, the facts that read the pages): the
//! smallest text and the text below a minimum ([`sizes`]), empty paragraphs used as space
//! ([`spacers`]), keeps broken at a page's foot and tables kept together ([`keeps`]),
//! tables parted from the line that introduces or follows them ([`parted`]), rows split
//! over pages and header rows not repeated ([`tables`]), columns that fit badly
//! ([`columns`]), what is drawn past the margins ([`margins`]), the pages laid out by
//! rules not measured ([`uncertain`]), each range's extent and page breaks ([`ranges`]) and
//! what a range's keep does in a trial layout ([`trials`]) -- each as typed entries, in docx-agent's order, with
//! what fixes them; on the pages asked for, and about the blocks a `range` names where
//! scoped ([`Scope`]).
//!
//! What every fact reads alike is made once per [`Facts`]: the body's lines by paragraph
//! path and by top-level block (only lines of a paragraph the version names), the body's
//! top-level blocks (block-level content controls opened), and each page's lines' text and
//! boxes, read from its paint when first asked ([`PageReads`]).
//!
//! Paths are the layout's (`w:body/w:tbl[1]/w:tr[2]/w:tc[1]/w:p[1]`); ids the tool's, as
//! the version's bytes name its blocks.
//!
//! Must not: lay out or paint but through [`PageReads`], word a reply (the tool layer
//! does), cap a list, or change the document.

pub mod columns;
pub mod keeps;
pub mod margins;
pub mod parted;
pub mod ranges;
mod scope;
pub mod sizes;
pub mod spacers;
pub mod tables;
mod tree;
pub mod trials;
pub mod uncertain;

use crate::line_text::LineText;
use crate::page_read::{PageBoxes, PageRead};
use crate::LaidVersion;
use ooxml_word_layout::facts::LineFact;
use ooxml_word_layout::px::Px;
use ooxml_word_model::docx::formatting::{EffectiveParagraph, Formatting};
use ooxml_word_model::docx::Docx;
use ooxml_xml::Element;
use std::cell::OnceCell;
use std::collections::{BTreeSet, HashMap};

pub use ranges::{room, BeyondFacts, Break, Extent, RangeLayout};
pub use scope::{Range, Scope, ScopeError};
pub use tree::{Block, BlockKind};
pub use trials::{Blank, KeepEffect, Side, TrialBlocks};

/// Each page as a check reads its paint (`PageEngine::page_read`).
pub trait PageReads {
    /// The 0-based page `page`'s lines' text and boxes (`None`: the page is not painted).
    fn page_read(&self, page: usize) -> Option<PageRead>;
}

/// One body line where it is drawn: its 1-based page and the line.
#[derive(Debug, Clone, Copy)]
pub struct Drawn<'a> {
    /// The 1-based page.
    pub page: usize,
    /// The line.
    pub line: &'a LineFact,
}

/// What the facts of one version read alike.
pub struct Facts<'a> {
    version: &'a LaidVersion,
    docx: &'a Docx,
    formatting: Formatting,
    /// The 1-based pages asked for (`None`: every page).
    wanted: Option<BTreeSet<usize>>,
    /// The body's lines of each paragraph the version names, by path, in drawing order.
    by_path: HashMap<&'a str, Vec<Drawn<'a>>>,
    /// The same lines by the top-level block (paragraph or table) they are in.
    by_block: HashMap<&'a str, Vec<Drawn<'a>>>,
    /// The pages of the same lines by every table they are in (nested ones too).
    by_table: HashMap<&'a str, BTreeSet<usize>>,
    blocks: Vec<Block<'a>>,
    /// Each top-level block's place among them, by path.
    positions: HashMap<String, usize>,
    reads: &'a dyn PageReads,
    painted: Vec<OnceCell<Option<PageRead>>>,
    /// What `range` scopes the facts to (`None`: the whole document).
    scope: Option<Scope>,
}

impl std::fmt::Debug for Facts<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Facts")
            .field("blocks", &self.blocks.len())
            .finish_non_exhaustive()
    }
}

impl<'a> Facts<'a> {
    /// The facts of `version`, whose reading is `docx`, on the 1-based `pages` (`None`:
    /// every page), its pages' paint read through `reads`.
    #[must_use]
    pub fn new(
        version: &'a LaidVersion,
        docx: &'a Docx,
        pages: Option<&[usize]>,
        reads: &'a dyn PageReads,
    ) -> Facts<'a> {
        let mut by_path: HashMap<&str, Vec<Drawn<'_>>> = HashMap::new();
        let mut by_block: HashMap<&str, Vec<Drawn<'_>>> = HashMap::new();
        let mut by_table: HashMap<&str, BTreeSet<usize>> = HashMap::new();
        let facts = version.facts();
        for (index, page) in facts.pages.iter().enumerate() {
            for line in &page.lines {
                let path: &str = &line.path;
                if story_kind(path) != "body" || version.id_of(path).is_none() {
                    continue;
                }
                let drawn = Drawn { page: index + 1, line };
                by_path.entry(path).or_default().push(drawn);
                if let Some(top) = tree::top_block_path(path) {
                    by_block.entry(top).or_default().push(drawn);
                }
                for table in tree::table_paths(path) {
                    by_table.entry(table).or_default().insert(index + 1);
                }
            }
        }
        let blocks = tree::top_level(docx);
        let positions = blocks.iter().enumerate().map(|(k, b)| (b.path.clone(), k)).collect();
        Facts {
            version,
            docx,
            formatting: Formatting::read(docx),
            wanted: pages.map(|p| p.iter().copied().collect()),
            by_path,
            by_block,
            by_table,
            blocks,
            positions,
            reads,
            painted: (0..facts.pages.len()).map(|_| OnceCell::new()).collect(),
            scope: None,
        }
    }

    /// These facts scoped to `scope` (`check`'s `range`): the entries about its blocks
    /// ([`Scope::about`]), the text sizes of its paragraphs only. The caller asks for the
    /// scope's pages.
    #[must_use]
    pub fn scoped(mut self, scope: Scope) -> Facts<'a> {
        self.scope = Some(scope);
        self
    }

    /// What the facts are scoped to.
    #[must_use]
    pub fn scope(&self) -> Option<&Scope> {
        self.scope.as_ref()
    }

    /// Whether an entry naming `subjects` is about the scope (always, unscoped).
    #[must_use]
    pub fn about(&self, subjects: &[&str]) -> bool {
        self.scope.as_ref().is_none_or(|s| s.about(subjects))
    }

    /// The version.
    #[must_use]
    pub fn version(&self) -> &'a LaidVersion {
        self.version
    }

    /// The version's reading.
    #[must_use]
    pub fn docx(&self) -> &'a Docx {
        self.docx
    }

    /// The pages laid out.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.painted.len()
    }

    /// The body's top-level blocks, in order.
    #[must_use]
    pub fn blocks(&self) -> &[Block<'a>] {
        &self.blocks
    }

    /// Whether any of `pages` (1-based) is asked for (`shown`).
    pub fn shown(&self, pages: impl IntoIterator<Item = usize>) -> bool {
        match &self.wanted {
            None => true,
            Some(wanted) => pages.into_iter().any(|p| wanted.contains(&p)),
        }
    }

    /// Whether every page is asked for.
    #[must_use]
    pub fn whole(&self) -> bool {
        self.wanted.is_none()
    }

    /// The body lines of the paragraph at `path`, in drawing order.
    #[must_use]
    pub fn lines_of(&self, path: &str) -> &[Drawn<'a>] {
        self.by_path.get(path).map_or(&[], Vec::as_slice)
    }

    /// The 1-based pages the paragraph at `path` is drawn on, in order (`pages_of`).
    #[must_use]
    pub fn pages_of(&self, path: &str) -> Vec<usize> {
        pages(self.lines_of(path))
    }

    /// The 1-based pages the body table at `path` (nested too) draws a line of text on, in
    /// order (`_table_pages`).
    #[must_use]
    pub fn table_pages(&self, path: &str) -> Vec<usize> {
        self.by_table
            .get(path)
            .map(|p| p.iter().copied().collect())
            .unwrap_or_default()
    }

    /// The 1-based pages the main part's table `id` draws a line of text on, in order; none
    /// for another part's (docx-agent counts the main part's paragraphs only).
    #[must_use]
    pub fn table_pages_of(&self, id: &str) -> Vec<usize> {
        self.version
            .path_of(id)
            .map(|path| self.table_pages(&path))
            .unwrap_or_default()
    }

    /// The top-level block at `path`, with its place among them.
    #[must_use]
    pub fn block_at(&self, path: &str) -> Option<(usize, &Block<'a>)> {
        let k = *self.positions.get(path)?;
        Some((k, self.blocks.get(k)?))
    }

    /// Where a top-level block is first drawn: its first page and its lines' top there
    /// (`first_line`).
    #[must_use]
    pub fn first_line(&self, block: &Block<'_>) -> Option<(usize, Px)> {
        match block.kind {
            BlockKind::Paragraph => {
                let found = self.lines_of(&block.path);
                let page = found.first()?.page;
                found
                    .iter()
                    .filter(|d| d.page == page)
                    .map(|d| d.line.top)
                    .min()
                    .map(|top| (page, top))
            }
            BlockKind::Table => self
                .by_block
                .get(block.path.as_str())?
                .iter()
                .map(|d| (d.page, d.line.top))
                .min(),
        }
    }

    /// The paragraph `element`'s formatting as Word applies it (outside a table).
    #[must_use]
    pub fn paragraph(&self, element: &Element) -> EffectiveParagraph {
        self.formatting.paragraph(element, None)
    }

    /// What formatting resolves against.
    #[must_use]
    pub fn formatting(&self) -> &Formatting {
        &self.formatting
    }

    /// The 0-based page `page` as a check reads it, painted once.
    fn read(&self, page: usize) -> Option<&PageRead> {
        self.painted
            .get(page)?
            .get_or_init(|| self.reads.page_read(page))
            .as_ref()
    }

    /// The 0-based page `page`'s lines' text, painted once.
    #[must_use]
    pub fn texts(&self, page: usize) -> Option<&[LineText]> {
        self.read(page).map(|r| r.texts.as_slice())
    }

    /// Every line of text the 0-based page `page` draws, its text boxes' after its own
    /// (docx2svg's `Page.text_lines`, which docx-agent's text sizes read), painted once.
    pub fn text_lines(&self, page: usize) -> impl Iterator<Item = &LineText> {
        self.read(page)
            .into_iter()
            .flat_map(|r| r.texts.iter().chain(&r.box_texts))
    }

    /// What the 0-based page `page` draws beside its lines, painted once.
    #[must_use]
    pub fn boxes(&self, page: usize) -> Option<&PageBoxes> {
        self.read(page).map(|r| &r.boxes)
    }

    /// The style a paragraph is in, by name (its id where it has none; the default
    /// paragraph style where it names none) -- `style_of`.
    #[must_use]
    pub fn style_of(&self, element: &Element) -> Option<String> {
        let named = tree::paragraph_style(element);
        let id = match named {
            Some(id) => id?,
            None => ooxml_word_model::docx::style_id(self.docx.default_style("paragraph")?).to_owned(),
        };
        Some(self.style_name(&id))
    }

    /// A style's name by its id (the id where it has no name or no style).
    #[must_use]
    pub fn style_name(&self, id: &str) -> String {
        self.docx
            .styles()
            .iter()
            .rev()
            .find(|s| ooxml_word_model::docx::style_id(s) == id)
            .and_then(|s| ooxml_word_model::docx::style_name(s))
            .filter(|n| !n.is_empty())
            .unwrap_or(id)
            .to_owned()
    }

    /// Whether a paragraph is in the Caption style, by its id or its name (`is_caption`).
    #[must_use]
    pub fn is_caption(&self, element: &Element) -> bool {
        let Some(named) = tree::paragraph_style(element) else {
            return false;
        };
        let id = named.unwrap_or_default();
        if id.to_lowercase() == "caption" {
            return true;
        }
        self.docx
            .styles()
            .iter()
            .find(|s| ooxml_word_model::docx::style_id(s) == id)
            .and_then(|s| ooxml_word_model::docx::style_name(s))
            .is_some_and(|n| n.to_lowercase() == "caption")
    }
}

/// The sorted 1-based pages of `drawn`.
fn pages(drawn: &[Drawn<'_>]) -> Vec<usize> {
    let set: BTreeSet<usize> = drawn.iter().map(|d| d.page).collect();
    set.into_iter().collect()
}

/// The story a layout path is in (docx-agent's `_story_kind`).
#[must_use]
pub fn story_kind(path: &str) -> &'static str {
    if path.starts_with("w:footnote[") {
        "footnotes"
    } else if path.starts_with("w:endnote[") {
        "endnotes"
    } else if path.starts_with("w:hdr") {
        "header"
    } else if path.starts_with("w:ftr") {
        "footer"
    } else {
        "body"
    }
}

/// A length in points as docx-agent says it: device px over 300/72, rounded to 2 places
/// (`_pt`).
#[must_use]
pub fn points(px: Px) -> f64 {
    round(ratio(px.value() / Px::per_point()), 2)
}

/// `value` as the nearest float.
fn ratio(value: ooxml_base::Q) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let (n, d) = (*value.numer() as f64, *value.denom() as f64);
    n / d
}

/// Python's `round(value, places)`: the float nearest the decimal rounding of `value`'s
/// exact binary value, ties to even.
#[must_use]
pub fn round(value: f64, places: usize) -> f64 {
    format!("{value:.places$}").parse().unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_are_said_as_docx_agent_rounds_them() {
        // 300 device px are 72 pt; 1 px is 0.24 pt
        assert!((points(Px::int(300)) - 72.0).abs() < f64::EPSILON);
        assert!((points(Px::int(1)) - 0.24).abs() < f64::EPSILON);
        assert!(
            (round(2.675, 2) - 2.67).abs() < f64::EPSILON,
            "the binary value is below the tie"
        );
        assert!((round(0.25, 1) - 0.2).abs() < f64::EPSILON, "a tie goes to even");
        assert_eq!(story_kind("w:hdr/w:p[1]"), "header");
        assert_eq!(story_kind("w:footnote[2]/w:p[1]"), "footnotes");
        assert_eq!(story_kind("w:body/w:p[1]"), "body");
    }
}
