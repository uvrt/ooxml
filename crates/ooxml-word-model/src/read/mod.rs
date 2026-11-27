//! Owns: reading a part's stories into the source model ([`Reader`]) as docx2svg's parser
//! (`docx2svg.parse.document`) reads them -- runs and their marks, fields threaded through
//! the part, bookmarks, tables, drawings' anchors, sections -- in the final view of tracked
//! changes: what Accept All leaves (the repository's rule for the render).
//!
//! Must not: resolve properties, or read a part's bytes (the document does, `crate::document`).

mod blocks;
mod drawing;
pub(crate) mod fields;
mod join;
mod paragraph;
pub mod paths;
mod runs;
mod sections;
mod tables;
pub mod vml;

pub use blocks::{body_story, BodyElement};
pub use fields::Fields;
pub use sections::{section_props, sections};
pub use tables::{cell_conditions, leading_header_rows};

use crate::ids::BlockKey;
use crate::model::{ParaProps, RunProps};
use crate::props::{paragraph_properties, run_properties};
use crate::theme::ThemeColours;
use crate::value::{text, Text};
use crate::w;
use crate::xmlread::local;
use ooxml_xml::Element;
use std::collections::HashMap;
use std::sync::Arc;

/// The structural path counter: 1-based indexes among same-named siblings (by local
/// name), each step written `w:<name>[k]` after a prefix.
pub(crate) struct Steps {
    prefix: Text,
    counts: HashMap<String, usize>,
    /// The last path given by [`Steps::step_str`].
    written: String,
}

impl Steps {
    pub(crate) fn new(prefix: &str) -> Steps {
        Steps {
            prefix: text(prefix),
            counts: HashMap::new(),
            written: String::new(),
        }
    }

    /// Count `node` and give its path.
    pub(crate) fn step(&mut self, node: &Element) -> Text {
        text(self.step_str(node))
    }

    /// Count `node` and give its path, written in a buffer the next step reuses (a reader
    /// that keeps most paths as they were allocates none).
    pub(crate) fn step_str(&mut self, node: &Element) -> &str {
        use std::fmt::Write as _;
        let name = local(node);
        let count = match self.counts.get_mut(name) {
            Some(count) => count,
            None => self.counts.entry(name.to_owned()).or_insert(0),
        };
        *count += 1;
        self.written.clear();
        if !self.prefix.is_empty() {
            self.written.push_str(&self.prefix);
            self.written.push('/');
        }
        // Writing to a String does not fail.
        let _ = write!(self.written, "w:{name}[{count}]");
        &self.written
    }
}

/// What keys a story's blocks from its part's index, by their places (child indexes from
/// the part's root).
pub(crate) trait KeyOf {
    /// The key of the paragraph or table at `place`.
    fn key_at(&self, place: &[u32]) -> Option<BlockKey>;
    /// The layout path of the block at `place`, for one the index does not key.
    fn path_at(&self, place: &[u32]) -> Option<String>;
}

/// Where a container's blocks are read: below a place a story's index holds (each block
/// keyed from it), or by path (a text box's blocks, by their steps below their paragraph,
/// keyed once read where the paragraph is keyed: `paragraph::key_text_boxes`; a part no
/// index holds; a table read on its own).
#[derive(Clone)]
pub(crate) enum At<'k> {
    /// At this place (child indexes from the part's root) of the part `keys` keys.
    Keyed(&'k dyn KeyOf, Vec<u32>),
    /// At this path.
    Path(Text),
}

impl<'k> At<'k> {
    /// The child element `node`, child `k` of the node read here, counted by `steps` (the
    /// path's 1-based count among siblings of its local name).
    pub(crate) fn child(&self, k: usize, node: &Element, steps: &mut Steps) -> At<'k> {
        match self {
            At::Keyed(keys, place) => At::Keyed(*keys, below(place, &[k])),
            At::Path(_) => At::Path(steps.step(node)),
        }
    }

    /// A content control's content, child `k` of the control here.
    pub(crate) fn content(&self, k: usize) -> At<'k> {
        match self {
            At::Keyed(keys, place) => At::Keyed(*keys, below(place, &[k])),
            At::Path(path) => At::Path(text(&format!("{path}/w:sdtContent[1]"))),
        }
    }

    /// The table here's cell at `(row, cell)` (its place as written), at `rel` below the
    /// table (row- and cell-level wrappers passed through).
    pub(crate) fn cell(&self, (row, cell): (u32, u32), rel: &[usize]) -> At<'k> {
        match self {
            At::Keyed(keys, place) => At::Keyed(*keys, below(place, rel)),
            At::Path(path) => At::Path(text(&crate::model::cell_path(path, row, cell))),
        }
    }

    /// The block here's name: its key, else the path it is named by.
    pub(crate) fn name(&self) -> (Option<BlockKey>, Option<Text>) {
        match self {
            At::Keyed(keys, place) => match keys.key_at(place) {
                Some(key) => (Some(key), None),
                None => (None, keys.path_at(place).map(|p| text(&p))),
            },
            At::Path(path) => (None, Some(path.clone())),
        }
    }
}

/// `place` with `rel` below it.
fn below(place: &[u32], rel: &[usize]) -> Vec<u32> {
    let mut out = place.to_vec();
    out.extend(rel.iter().map(|k| u32::try_from(*k).unwrap_or(u32::MAX)));
    out
}

/// Reads stories of one package: theme colours read as Word draws them.
#[derive(Debug, Clone, Copy)]
pub struct Reader<'c> {
    colours: &'c ThemeColours,
}

impl<'c> Reader<'c> {
    /// A reader with the package's theme colours.
    pub fn new(colours: &'c ThemeColours) -> Reader<'c> {
        Reader { colours }
    }

    /// A `w:rPr` as written (`None` where there is none).
    pub(crate) fn run_props(&self, rpr: Option<&Element>) -> Option<Arc<RunProps>> {
        let rpr = rpr?;
        let style_id = w::RPr::new(rpr).style().and_then(w::Val::val).map(text);
        Some(Arc::new(RunProps {
            style_id,
            declared: run_properties(Some(rpr), self.colours),
        }))
    }

    /// A `w:pPr` as written.
    pub(crate) fn para_props(&self, ppr: Option<&Element>) -> Option<Arc<ParaProps>> {
        let ppr = ppr?;
        let view = w::PPr::new(ppr);
        Some(Arc::new(ParaProps {
            style_id: view.style().and_then(w::Val::val).map(text),
            declared: paragraph_properties(Some(ppr), self.colours),
            mark: self.run_props(view.mark().map(w::RPr::element)),
        }))
    }

    /// The package's theme colours.
    pub(crate) fn colours(&self) -> &'c ThemeColours {
        self.colours
    }
}
