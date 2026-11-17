//! A version of a Word package as docx-agent's reading tools read it (phase P1): its story
//! parts with their trees and block ids ([`Docx`]), and what `describe` and `word_read`
//! read of them -- paragraphs' text in a view of their revisions ([`text`]), content
//! controls ([`controls`]), drawings ([`drawings`]), the outline ([`outline`]), sections
//! ([`sections`]), fields ([`fields`]) and revision records ([`revisions`]).
//!
//! Owns: docx-agent's rules for these (`docx_agent/edit/*.py`), which in places differ
//! from the layout model's (docx2svg's): names are matched with their namespaces, as lxml
//! matches them, and elements under `mc:Fallback` are not what Word shows.
//!
//! Must not: edit or lay out. A [`Docx`] is one version's reading, shared by the calls on
//! that version; the next version's reuses the story parts whose trees did not change
//! ([`Docx::open_after`]), and [`Versions`] keeps the latest versions' readings for a
//! document.

pub mod cells;
pub mod changes;
pub mod comments;
pub mod controls;
mod derived;
pub mod drawings;
pub mod fields;
pub mod formatting;
pub mod numbering;
pub mod outline;
pub mod revisions;
pub mod sections;
pub mod tables;
pub mod text;
mod versions;
mod walk;

pub use derived::Derived;
pub use versions::Versions;

pub use walk::{at_path, live_paths, Found};

use crate::ids::{story_name, PartIds, W};
use crate::parts::{main_part, story_parts};
use crate::WordError;
use ooxml_opc::Package;
use ooxml_xml::{Element, Tree};
use std::collections::HashMap;
use std::sync::Arc;

/// One story part: its tree and its block ids.
#[derive(Debug, Clone)]
pub struct StoryPart {
    /// The part's name.
    pub part: String,
    /// Its story's name (`body`, `header1`, `footnotes`...).
    pub story: String,
    /// Its tree.
    pub tree: Tree,
    /// Its paragraphs', tables' and rows' ids.
    pub ids: PartIds,
}

impl StoryPart {
    /// The part's root element.
    #[must_use]
    pub fn root(&self) -> &Arc<Element> {
        self.tree.root()
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
}

impl Docx {
    /// Read `package`.
    ///
    /// # Errors
    /// [`WordError`] when the package has no main part or a part is not XML.
    pub fn open(package: &Package) -> Result<Docx, WordError> {
        Docx::open_after(package, None)
    }

    /// Read `package`, taking from `before` (an earlier version's reading of the same
    /// document) every story part whose tree is the very same: its ids are not found again.
    ///
    /// # Errors
    /// As [`Docx::open`].
    pub fn open_after(package: &Package, before: Option<&Docx>) -> Result<Docx, WordError> {
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
            let ids = PartIds::new(&story, tree.root());
            parts.push(Arc::new(StoryPart { part, story, tree, ids }));
        }
        let (styles, style_order) = read_styles(package, &main)?;
        Ok(Docx {
            package: package.clone(),
            main,
            parts,
            styles,
            style_order,
            derived: Derived::default(),
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
        let wanted = casefold(name);
        let styles: Vec<&Arc<Element>> = self.style_order.iter().filter(|s| style_kind(s) == kind).collect();
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
