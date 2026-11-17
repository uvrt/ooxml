//! Owns: the document-wide lookups every reading of a version starts from ([`Shared`],
//! read once per version and kept with it): the styles, numbering and theme formatting
//! resolves against, the lists, the ids of controls, revision records, pictures and
//! drawings by their place, every note with its id (`fn:1`, `en:2`) by its reference's
//! kind and `w:id`, the comments by `w:id`, and the bookmark names internal hyperlinks
//! point at (shown even when hidden, as `_Toc` ones are).
//!
//! Must not: read a story's blocks.

use super::{index, Placed};
use crate::docx::cells::TableFacts;
use crate::docx::comments::{comments, Comment};
use crate::docx::controls::controls;
use crate::docx::drawings::{all_drawings, pictures};
use crate::docx::formatting::Formatting;
use crate::docx::numbering::Lists;
use crate::docx::revisions::records as revision_records;
use crate::docx::Docx;
use crate::ids::TableId;
use crate::ids::W;
use ooxml_xml::Node;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

/// An element's key: its part and its place there.
type Key = (String, Box<[u32]>);

/// The document-wide lookups of one version.
pub struct Shared {
    /// What formatting resolves against.
    pub(super) formatting: Formatting,
    /// The lists.
    pub(super) lists: Lists,
    /// Content controls' ids by place.
    pub(super) controls: HashMap<Key, String>,
    /// Revision records' ids by place.
    pub(super) revisions: HashMap<Key, String>,
    /// Pictures' ids by place.
    pub(super) pictures: HashMap<Key, String>,
    /// Drawings' ids by place.
    pub(super) drawings: HashMap<Key, String>,
    /// Notes' ids by `(fn|en, w:id)`.
    pub(super) notes: HashMap<(String, String), String>,
    /// Bookmarks internal hyperlinks point at.
    pub(super) anchors: HashSet<String>,
    /// Comments by `w:id`.
    pub(super) comments: HashMap<String, Comment>,
    /// What the layout model reads of each top-level table read so far, by part and place.
    tables: Mutex<HashMap<Key, Arc<TableFacts>>>,
    /// The drawings whose one-line summary is a chart's or a SmartArt diagram's (one, or a
    /// group holding one): docx-agent writes one, the core does not yet (phase P3).
    graphics: Vec<String>,
    /// Whether a table cell's end is one Accept All takes away ([`PartFacts::cell_ends`]).
    cell_ends: bool,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl Shared {
    /// The lookups of `docx`'s version, read once and kept with it.
    #[must_use]
    pub fn of(docx: &Docx) -> Arc<Shared> {
        docx.memo().reading(|| Shared::read(docx))
    }

    /// The lookups of `docx`.
    #[must_use]
    pub fn read(docx: &Docx) -> Shared {
        let key = |part: &str, path: &[u32]| (part.to_owned(), Box::<[u32]>::from(path));
        let drawings = all_drawings(docx);
        Shared {
            formatting: Formatting::read(docx),
            lists: Lists::read(docx),
            controls: controls(docx)
                .into_iter()
                .map(|c| (key(&c.part, &c.path), c.id))
                .collect(),
            revisions: revision_records(docx)
                .into_iter()
                .map(|r| (key(&r.part, &r.path), r.id))
                .collect(),
            pictures: pictures(docx)
                .into_iter()
                .map(|d| (key(&d.part, &d.path), d.id))
                .collect(),
            graphics: drawings.iter().filter(|d| summarised(d)).map(|d| d.id.clone()).collect(),
            drawings: drawings
                .into_iter()
                .map(|d| (key(&d.part, &d.path), d.id))
                .collect(),
            cell_ends: docx.parts.iter().any(|p| p.facts().cell_ends()),
            notes: notes_of(docx),
            anchors: anchors(docx),
            comments: comments(docx)
                .into_iter()
                .filter_map(|c| c.w_id.clone().map(|w| (w, c)))
                .collect(),
            tables: Mutex::new(HashMap::new()),
        }
    }

    /// What the layout model reads of the table `table` of `part`, read once.
    pub fn table_facts(&self, part: &str, table: &TableId) -> Arc<TableFacts> {
        let key = (part.to_owned(), table.path.clone());
        if let Some(found) = self.tables.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
            return Arc::clone(found);
        }
        let made = Arc::new(TableFacts::read(&self.formatting, &table.element));
        self.tables
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, Arc::clone(&made));
        made
    }

    /// What formatting resolves against.
    #[must_use]
    pub fn formatting(&self) -> &Formatting {
        &self.formatting
    }

    /// The drawings whose summary docx-agent writes from the chart model (a chart, a
    /// SmartArt diagram, a group holding one), which the core does not read yet (phase P3).
    #[must_use]
    pub fn graphics(&self) -> &[String] {
        &self.graphics
    }

    /// Whether a table cell's end is one Accept All takes away: docx-agent's final view joins
    /// the next cell's text in, the core's does not yet.
    #[must_use]
    pub fn cell_ends(&self) -> bool {
        self.cell_ends
    }
}

/// Whether docx-agent writes `drawing`'s summary from the chart model: a chart, a diagram,
/// or a group with one among its members.
fn summarised(drawing: &crate::docx::drawings::Drawing) -> bool {
    use crate::docx::drawings::{group_members, member_kind};
    match drawing.kind {
        "chart" | "diagram" => true,
        "group" => group_members(&drawing.element, &drawing.id, false)
            .iter()
            .any(|(_, member)| matches!(member_kind(member), "chart" | "diagram")),
        _ => false,
    }
}

/// The notes by `(fn|en, w:id)`.
pub(super) fn notes_of(docx: &Docx) -> HashMap<(String, String), String> {
    let mut out = HashMap::new();
    for (part, identifier, _) in note_elements(docx) {
        let _ = part;
        if let Some((prefix, id)) = identifier.split_once(':') {
            out.insert((prefix.to_owned(), id.to_owned()), identifier.clone());
        }
    }
    out
}

/// Every footnote and endnote (separators aside): its part, id (`fn:1`) and element.
#[must_use]
pub fn note_elements(docx: &Docx) -> Vec<(String, String, Placed)> {
    let mut out = Vec::new();
    for part in &docx.parts {
        let root = part.root();
        let prefix = if root.name().is(Some(W), "footnotes") {
            "fn"
        } else if root.name().is(Some(W), "endnotes") {
            "en"
        } else {
            continue;
        };
        for (k, node) in root.children().iter().enumerate() {
            let Node::Element(note) = node else { continue };
            let is_note = note.name().is(Some(W), "footnote") || note.name().is(Some(W), "endnote");
            if is_note && matches!(note.attribute(Some(W), "type"), None | Some("normal")) {
                let id = note.attribute(Some(W), "id").unwrap_or("None");
                out.push((
                    part.part.clone(),
                    format!("{prefix}:{id}"),
                    Placed {
                        element: Arc::clone(note),
                        path: vec![index(k)].into_boxed_slice(),
                    },
                ));
            }
        }
    }
    out
}

/// Bookmark names an internal hyperlink points at.
pub(super) fn anchors(docx: &Docx) -> HashSet<String> {
    docx.parts
        .iter()
        .flat_map(|part| part.facts().anchors().map(str::to_owned).collect::<Vec<_>>())
        .collect()
}
