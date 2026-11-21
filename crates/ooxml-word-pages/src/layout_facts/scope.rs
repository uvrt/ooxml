//! Owns: what `check`'s `range` scopes the layout facts to (docx-agent's `scope.Scope`,
//! round 35, R18): the body's blocks one or more addresses name, read from the document --
//! every paragraph in them (tables' cells too), the tables holding any of those paragraphs
//! (a cell's table facts are about the cell too), the drawings anchored in those
//! paragraphs, and each range's top-level blocks -- and whether a fact's entry is about
//! them ([`Scope::about`]); each range's pages in a version ([`Scope::pages`]).
//!
//! An entry is about the scope when one of its subjects (`para`, `paras`, `ids`, `table`,
//! `drawing` or `blocks`, whichever it has) is one of those; an entry naming none (the
//! pages the layout could not see whole) is about the pages, and kept on the scope's pages.
//!
//! Must not: word a refusal (the tool layer does), or read the layout but for the pages.

use super::tree::{is_w, table_paths, top_block_path};
use crate::LaidVersion;
use ooxml_word_model::address::{scope_paragraphs, AddressError};
use ooxml_word_model::docx::drawings::all_drawings;
use ooxml_word_model::docx::Docx;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Why a range is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// Range `number` names nothing, or is malformed.
    Address {
        /// Its place among the ranges.
        number: usize,
        /// The address as given.
        address: String,
        /// What the resolver says.
        error: AddressError,
    },
    /// Range `number` is in another story than the body.
    NotBody {
        /// Its place among the ranges.
        number: usize,
        /// The address as given.
        address: String,
        /// The story's name (`header1`, `footnotes`...).
        story: String,
    },
    /// Range `number` holds no paragraph.
    Empty {
        /// Its place among the ranges.
        number: usize,
        /// The address as given.
        address: String,
    },
}

/// One range: its address, its paragraphs' ids and its top-level blocks' ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    /// The address as given.
    pub address: String,
    /// Its paragraphs' ids, in document order.
    pub paragraphs: Vec<String>,
    /// The ids of the top-level blocks they are in, each once, in order.
    pub blocks: Vec<String>,
}

/// The body's blocks one or more ranges name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// Each range.
    pub ranges: Vec<Range>,
    /// The ids of every paragraph in them.
    pub paragraphs: HashSet<String>,
    /// The ids of the tables holding any of those paragraphs.
    pub tables: HashSet<String>,
    /// The ids of the drawings anchored in those paragraphs.
    pub drawings: HashSet<String>,
    /// The body's places a problem is looked for in (docx-agent's `Scope.roots`): each
    /// top-level block every paragraph of which is in the scope, and the paragraphs of a
    /// block that is only partly (a cell's), in the order the ranges first reach them.
    pub roots: Vec<Box<[u32]>>,
}

impl Scope {
    /// The scope of `addresses` in `docx` (each any address `word_read` takes for its
    /// `range`).
    ///
    /// # Errors
    /// The first range that names nothing, nothing of the body, or no paragraph.
    pub fn of(docx: &Docx, addresses: &[String]) -> Result<Scope, ScopeError> {
        let Some(body) = docx.body() else {
            return Ok(Scope::default());
        };
        let mut scope = Scope::default();
        let table_ids: HashMap<Box<[u32]>, String> = body.ids.tables().iter().map(|t| (t.path, t.id)).collect();
        let mut held: Vec<Box<[u32]>> = Vec::new();
        let mut tops: Vec<Box<[u32]>> = Vec::new();
        for (number, address) in addresses.iter().enumerate() {
            let (part, indexes) = scope_paragraphs(docx, address).map_err(|error| ScopeError::Address {
                number,
                address: address.clone(),
                error,
            })?;
            if part.part != body.part {
                return Err(ScopeError::NotBody {
                    number,
                    address: address.clone(),
                    story: part.story.clone(),
                });
            }
            if indexes.is_empty() {
                return Err(ScopeError::Empty {
                    number,
                    address: address.clone(),
                });
            }
            let mut range = Range {
                address: address.clone(),
                paragraphs: Vec::new(),
                blocks: Vec::new(),
            };
            for k in indexes {
                let Some(paragraph) = body.ids.paragraphs().get(k) else {
                    continue;
                };
                let top = top_place(body, &paragraph.path);
                if let Some(top) = top.filter(|t| !tops.iter().any(|seen| &**seen == *t)) {
                    tops.push(top.into());
                }
                if let Some(id) = top.and_then(|t| block_id(body, t)) {
                    if !range.blocks.contains(&id) {
                        range.blocks.push(id);
                    }
                }
                for end in 1..paragraph.path.len() {
                    if let Some(id) = paragraph.path.get(..end).and_then(|above| table_ids.get(above)) {
                        scope.tables.insert(id.clone());
                    }
                }
                scope.paragraphs.insert(paragraph.id.clone());
                range.paragraphs.push(paragraph.id);
                held.push(paragraph.path);
            }
            scope.ranges.push(range);
        }
        if !held.is_empty() {
            let held: HashSet<&[u32]> = held.iter().map(AsRef::as_ref).collect();
            scope.roots = roots(body, &tops, &held);
            for drawing in all_drawings(docx).into_iter().filter(|d| d.part == body.part) {
                if (1..drawing.path.len()).any(|end| drawing.path.get(..end).is_some_and(|above| held.contains(above)))
                {
                    scope.drawings.insert(drawing.id);
                }
            }
        }
        Ok(scope)
    }

    /// Whether `id` is one of the scope's paragraphs, tables or drawings.
    #[must_use]
    pub fn holds(&self, id: &str) -> bool {
        self.paragraphs.contains(id) || self.tables.contains(id) || self.drawings.contains(id)
    }

    /// Whether an entry whose subjects are `subjects` is about the scope: one of them is
    /// held, or it names none.
    #[must_use]
    pub fn about(&self, subjects: &[&str]) -> bool {
        subjects.is_empty() || subjects.iter().any(|s| self.holds(s))
    }

    /// Each range's 1-based pages in `version`, in order: its paragraphs', else its blocks'
    /// (`[]` for a range nothing of which is drawn).
    #[must_use]
    pub fn pages(&self, version: &LaidVersion) -> Vec<Vec<usize>> {
        let mut drawn: HashMap<&str, BTreeSet<usize>> = HashMap::new();
        for (index, page) in version.facts().pages.iter().enumerate() {
            for line in &page.lines {
                let path: &str = &line.path;
                drawn.entry(path).or_default().insert(index + 1);
                for table in table_paths(path) {
                    drawn.entry(table).or_default().insert(index + 1);
                }
                if let Some(top) = top_block_path(path).filter(|t| *t != path) {
                    drawn.entry(top).or_default().insert(index + 1);
                }
            }
        }
        let pages_of = |ids: &[String]| -> BTreeSet<usize> {
            ids.iter()
                .filter_map(|id| version.path_of(id))
                .filter_map(|path| drawn.get(&*path))
                .flatten()
                .copied()
                .collect()
        };
        self.ranges
            .iter()
            .map(|range| {
                let mut pages = pages_of(&range.paragraphs);
                if pages.is_empty() {
                    pages = pages_of(&range.blocks);
                }
                pages.into_iter().collect()
            })
            .collect()
    }
}

/// Each of `tops` (top-level blocks' places) every paragraph of which is `held`, else the
/// paragraphs of it that are.
fn roots(story: &ooxml_word_model::docx::StoryPart, tops: &[Box<[u32]>], held: &HashSet<&[u32]>) -> Vec<Box<[u32]>> {
    let mut out = Vec::new();
    for top in tops {
        let inside: Vec<Box<[u32]>> = story
            .ids
            .paragraphs()
            .iter()
            .map(|p| p.path)
            .filter(|path| path.starts_with(top))
            .collect();
        if inside.iter().all(|path| held.contains(&**path)) {
            out.push(top.clone());
        } else {
            out.extend(inside.into_iter().filter(|path| held.contains(&**path)));
        }
    }
    out
}

/// The place of the top-level block a body paragraph at `place` is in: the first element
/// on its way below the body that is a paragraph or table (past block-level content
/// controls).
fn top_place<'p>(story: &ooxml_word_model::docx::StoryPart, place: &'p [u32]) -> Option<&'p [u32]> {
    let mut node = story.root();
    for (end, &k) in place.iter().enumerate() {
        node = node.children().get(k as usize)?.as_element()?;
        if end >= 1 && (is_w(node, "p") || is_w(node, "tbl")) {
            return place.get(..=end);
        }
    }
    None
}

/// The id of the paragraph or table at `place` in `story`.
fn block_id(story: &ooxml_word_model::docx::StoryPart, place: &[u32]) -> Option<String> {
    story.ids.at(place).and_then(|entry| story.ids.id_of(entry))
}
