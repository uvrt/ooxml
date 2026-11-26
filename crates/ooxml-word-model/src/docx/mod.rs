//! A version of a Word package as docx-agent's reading tools read it (phase P1): its story
//! parts with their trees and block ids ([`Docx`]), and what `describe` and `word_read`
//! read of them -- paragraphs' text in a view of their revisions ([`text`]), content
//! controls ([`controls`]), drawings ([`drawings`]), the outline ([`outline`]), sections
//! ([`sections`]), fields ([`fields`]) and revision records ([`revisions`]); and what a
//! layout's anchors are named at the edge ([`VersionNames`]).
//!
//! Owns: docx-agent's rules for these (`docx_agent/edit/*.py`), which in places differ
//! from the layout model's (docx2svg's): names are matched with their namespaces, as lxml
//! matches them, and elements under `mc:Fallback` are not what Word shows.
//!
//! Must not: edit or lay out. A [`Docx`] is one version's reading, shared by the calls on
//! that version; the next version's reuses the story parts whose trees did not change
//! ([`Docx::open_after`]), and [`Versions`] keeps the latest versions' readings for a
//! document.

pub mod block_facts;
pub mod cells;
pub mod changes;
pub mod comments;
pub mod controls;
mod delta;
mod derived;
pub mod drawings;
pub mod facts;
pub mod fields;
pub mod formatting;
pub mod graphics;
pub mod keeps;
mod kept;
mod kept_rows;
pub mod names;
pub mod numbering;
pub mod outline;
pub mod provenance;
pub mod revisions;
pub mod sections;
pub mod statistics;
pub mod tables;
pub mod text;
mod texts;
mod versions;
mod walk;

pub use derived::Derived;
pub use kept::Kept;
pub use names::VersionNames;
pub use texts::PartTexts;
pub use versions::Versions;

pub use walk::{at_path, live_paths, Found};

use crate::ids::{story_name, PartIds, W};
use crate::parts::{main_part, story_parts};
use crate::WordError;
use facts::PartFacts;
use ooxml_opc::Package;
use ooxml_xml::{Element, Tree};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// One story part: its tree, its block ids, and what the document-wide lookups read of it
/// (read once, when first asked).
#[derive(Debug)]
pub struct StoryPart {
    /// The part's name.
    pub part: String,
    /// Its story's name (`body`, `header1`, `footnotes`...).
    pub story: String,
    /// Its tree.
    pub tree: Tree,
    /// Its paragraphs', tables' and rows' ids.
    pub ids: PartIds,
    facts: OnceLock<Arc<PartFacts>>,
    texts: OnceLock<Arc<PartTexts>>,
    /// The document's latest readings of the part: the paragraphs and blocks this version
    /// shares with the version that read them are taken from them.
    kept: Arc<Kept>,
}

impl StoryPart {
    /// The part `part` of the story `story`, whose tree is `tree`; `earlier`, the same
    /// part of the same story in a version before (its ids are taken for every block the
    /// two share); `kept`, the document's latest readings (texts and facts are taken from
    /// them likewise).
    #[must_use]
    pub fn new(part: String, story: String, tree: Tree, earlier: Option<&StoryPart>, kept: &Arc<Kept>) -> StoryPart {
        // a part changed by a path copy (or handed over and shared again with the version
        // before): only what changed is walked again
        let ids = match earlier {
            Some(earlier) => earlier.ids.after(earlier.root(), tree.root()),
            None => PartIds::new(&story, tree.root()),
        };
        StoryPart {
            part,
            story,
            tree,
            ids,
            facts: OnceLock::new(),
            texts: OnceLock::new(),
            kept: Arc::clone(kept),
        }
    }

    /// The element a layout path names in the part, and its place (child indexes from the
    /// root): [`crate::read::paths::resolve_path`], a step among the block container's
    /// children found from the part's ids in O(log n).
    #[must_use]
    pub fn resolve_path(&self, path: &str) -> Option<(&Arc<Element>, Vec<u32>)> {
        crate::read::paths::resolve_in(self.root(), path, &self.ids)
    }

    /// The part's root element.
    #[must_use]
    pub fn root(&self) -> &Arc<Element> {
        self.tree.root()
    }

    /// `F`'s value for the block `element` at `place` (child indexes from the root) under
    /// `formatting`: as the document keeps it across its versions (the block, the outermost
    /// table holding it and the formatting parts unchanged), else made by `read` and kept.
    pub fn block_fact<F: block_facts::BlockFact>(
        &self,
        formatting: &formatting::Formatting,
        element: &Arc<Element>,
        place: &[u32],
        read: impl FnOnce() -> F::Value,
    ) -> F::Value {
        let outer = block_facts::outer_table(self.root(), place);
        self.kept.blocks().get::<F>(formatting, element, outer, read)
    }

    /// Each paragraph's text in the current view ([`PartTexts`]), read once.
    pub fn texts(&self) -> &Arc<PartTexts> {
        self.texts.get_or_init(|| {
            self.kept
                .texts(&self.part, |earlier| PartTexts::read(&self.ids, earlier))
        })
    }

    /// What the document-wide lookups read of the part ([`PartFacts`]), read once.
    pub fn facts(&self) -> &Arc<PartFacts> {
        self.facts.get_or_init(|| {
            self.kept
                .facts(&self.part, |earlier| PartFacts::read(self.root(), earlier))
        })
    }
}

/// A version of a Word package as docx-agent reads it.
#[derive(Debug, Clone)]
pub struct Docx {
    /// The package.
    pub package: Package,
    /// The main part's name.
    pub main: String,
    /// The main part, then every story part (headers, footers, notes, comments) the
    /// package has, in relationship order.
    pub parts: Vec<Arc<StoryPart>>,
    /// The styles part's `w:style` elements by id, where there is a styles part.
    styles: HashMap<String, Arc<Element>>,
    /// The styles in document order.
    style_order: Vec<Arc<Element>>,
    /// What is read from this version once, for every call on it.
    derived: Derived,
    /// The document's latest readings of its parts, shared with its other versions.
    kept: Arc<Kept>,
}

impl Docx {
    /// Its sections in order (as [`sections::sections`] reads them), read once per version.
    #[must_use]
    pub fn sections(&self) -> &[sections::Section] {
        self.derived.sections(self)
    }

    /// The relationships of its package's part `source` (`""` for the package's own),
    /// read once per version.
    ///
    /// # Errors
    /// Its relationships part is not XML.
    pub fn relationships(&self, source: &str) -> Result<Arc<crate::parts::PartRelationships>, ooxml_opc::OpcError> {
        self.derived.relationships(self, source)
    }

    /// Read `package`.
    ///
    /// # Errors
    /// [`WordError`] when the package has no main part or a part is not XML.
    pub fn open(package: &Package) -> Result<Docx, WordError> {
        Docx::open_after(package, None)
    }

    /// [`Docx::open`], what its versions keep of its parts in `kept` (a document's first
    /// version: its later ones share it).
    ///
    /// # Errors
    /// As [`Docx::open`].
    pub fn open_keeping(package: &Package, kept: Arc<Kept>) -> Result<Docx, WordError> {
        Docx::read(package, None, kept)
    }

    /// Read `package`, taking from `before` (an earlier version's reading of the same
    /// document) every story part whose tree is the very same: its ids are not found again.
    ///
    /// # Errors
    /// As [`Docx::open`].
    pub fn open_after(package: &Package, before: Option<&Docx>) -> Result<Docx, WordError> {
        let latest = before.map_or_else(|| Arc::new(Kept::default()), |b| Arc::clone(&b.kept));
        Docx::read(package, before, latest)
    }

    /// What its versions keep of its tables' readings: entries and bytes.
    #[must_use]
    pub fn table_readings(&self) -> (usize, usize) {
        self.kept.table_readings()
    }

    /// What its versions keep of its blocks' facts: entries and bytes.
    #[must_use]
    pub fn block_facts(&self) -> (usize, usize) {
        let stats = self.kept.blocks().stats();
        (stats.entries, stats.bytes)
    }

    /// What its versions keep of its tables' rows' keeps: entries and bytes.
    #[must_use]
    pub fn row_keeps(&self) -> (usize, usize) {
        self.kept.row_keeps()
    }

    /// `package` read, taking from `before` its parts kept, what its versions keep in `latest`.
    fn read(package: &Package, before: Option<&Docx>, latest: Arc<Kept>) -> Result<Docx, WordError> {
        let main = main_part(package)?.ok_or_else(|| WordError::NoMainPart(String::new()))?;
        let mut names = vec![main.clone()];
        names.extend(story_parts(package, &main)?);
        let mut parts = Vec::with_capacity(names.len());
        for part in names {
            let Some(tree) = package.tree(&part)? else { continue };
            let kept = before
                .and_then(|b| b.parts.iter().find(|p| p.part == part))
                .filter(|p| Arc::ptr_eq(p.tree.root(), tree.root()) && p.story == story_name(&main, &part));
            if let Some(kept) = kept {
                parts.push(Arc::clone(kept));
                continue;
            }
            let story = story_name(&main, &part);
            let earlier = before
                .and_then(|b| b.parts.iter().find(|p| p.part == part))
                .filter(|p| p.story == story);
            parts.push(Arc::new(StoryPart::new(
                part,
                story,
                tree,
                earlier.map(|e| &**e),
                &latest,
            )));
        }
        let (styles, style_order) = read_styles(package, &main)?;
        Ok(Docx {
            package: package.clone(),
            main,
            parts,
            styles,
            style_order,
            derived: Derived::after(before.map(|b| &b.derived)),
            kept: latest,
        })
    }

    /// What is read from this version once and kept with it.
    #[must_use]
    pub fn memo(&self) -> &Derived {
        &self.derived
    }

    /// The main part's story.
    #[must_use]
    pub fn body(&self) -> Option<&StoryPart> {
        self.parts.iter().map(|p| &**p).find(|p| p.part == self.main)
    }

    /// The story part named `part`.
    #[must_use]
    pub fn part(&self, part: &str) -> Option<&StoryPart> {
        self.parts.iter().map(|p| &**p).find(|p| p.part == part)
    }

    /// The story part of the story `name`.
    #[must_use]
    pub fn story(&self, name: &str) -> Option<&StoryPart> {
        self.parts.iter().map(|p| &**p).find(|p| p.story == name)
    }

    /// The `w:style` with id `id`.
    #[must_use]
    pub fn style(&self, id: &str) -> Option<&Arc<Element>> {
        self.styles.get(id)
    }

    /// Every `w:style`, in document order.
    #[must_use]
    pub fn styles(&self) -> &[Arc<Element>] {
        &self.style_order
    }

    /// The default style of `kind` (`paragraph`, `character`, `table`): the first
    /// `w:style` of that type marked `w:default` on.
    #[must_use]
    pub fn default_style(&self, kind: &str) -> Option<&Arc<Element>> {
        self.style_order
            .iter()
            .find(|s| style_kind(s) == kind && s.attribute(Some(W), "default").is_some_and(on))
    }

    /// The style `name` names among those of `kind`, as docx-agent's `Styles.find` finds
    /// it: by `w:name` in any case, then an alias, then its id, then its id in any case.
    #[must_use]
    pub fn find_style(&self, name: &str, kind: &str) -> Option<&Arc<Element>> {
        self.find_style_in(name, Some(kind))
    }

    /// The style `name` names among all of them, found as [`Docx::find_style`] finds it.
    #[must_use]
    pub fn find_any_style(&self, name: &str) -> Option<&Arc<Element>> {
        self.find_style_in(name, None)
    }

    fn find_style_in(&self, name: &str, kind: Option<&str>) -> Option<&Arc<Element>> {
        let wanted = casefold(name);
        let styles: Vec<&Arc<Element>> = self
            .style_order
            .iter()
            .filter(|s| kind.is_none_or(|k| style_kind(s) == k))
            .collect();
        let tests: [&dyn Fn(&Element) -> bool; 4] = [
            &|s| casefold(style_name(s).unwrap_or("")) == wanted,
            &|s| {
                child_val(s, "aliases")
                    .unwrap_or("")
                    .split(',')
                    .map(str::trim)
                    .any(|a| !a.is_empty() && casefold(a) == wanted)
            },
            &|s| style_id(s) == name,
            &|s| casefold(style_id(s)) == wanted,
        ];
        tests.iter().find_map(|test| styles.iter().find(|s| test(s)).copied())
    }
}

/// A style's type (`paragraph` where it states none).
#[must_use]
pub fn style_kind(style: &Element) -> &str {
    style
        .attribute(Some(W), "type")
        .filter(|t| !t.is_empty())
        .unwrap_or("paragraph")
}

/// A style's id (`""` where it states none).
#[must_use]
pub fn style_id(style: &Element) -> &str {
    style.attribute(Some(W), "styleId").unwrap_or("")
}

/// A style's `w:name`.
#[must_use]
pub fn style_name(style: &Element) -> Option<&str> {
    child_val(style, "name")
}

/// Python's `str.casefold` for what style names hold (lower case, `ß` as `ss`).
#[must_use]
pub fn casefold(text: &str) -> String {
    text.to_lowercase().replace('ß', "ss")
}

/// An `ST_OnOff` value docx-agent reads as on.
#[must_use]
pub fn on(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "1" | "true" | "on")
}

/// A `w:` child's `w:val`.
#[must_use]
pub fn child_val<'a>(element: &'a Element, local: &str) -> Option<&'a str> {
    element.find(Some(W), local)?.attribute(Some(W), "val")
}

/// The element at `path` of `w:` steps (`["pPr", "pStyle"]`), each the first of its name.
#[must_use]
pub fn find_path<'a>(element: &'a Element, path: &[&str]) -> Option<&'a Element> {
    let mut node = element;
    for step in path {
        node = node.find(Some(W), step)?;
    }
    Some(node)
}

type Styles = (HashMap<String, Arc<Element>>, Vec<Arc<Element>>);

fn read_styles(package: &Package, main: &str) -> Result<Styles, WordError> {
    let part = crate::parts::related(
        package,
        main,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
    )?;
    let Some(tree) = (match part {
        Some(part) => package.tree(&part)?,
        None => None,
    }) else {
        return Ok((HashMap::new(), Vec::new()));
    };
    let order: Vec<Arc<Element>> = tree
        .root()
        .child_elements()
        .filter(|e| e.name().is(Some(W), "style"))
        .cloned()
        .collect();
    let mut by_id = HashMap::new();
    for style in &order {
        // A dictionary by id: the last of an id wins.
        by_id.insert(
            style.attribute(Some(W), "styleId").unwrap_or("").to_owned(),
            Arc::clone(style),
        );
    }
    Ok((by_id, order))
}
