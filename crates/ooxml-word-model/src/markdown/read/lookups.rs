//! Owns: the document-wide lookups every reading of a version starts from ([`Shared`],
//! each read once per version, on first use, and kept with it): the styles, numbering and theme formatting
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
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// An element's key: its part and its place there.
type Key = (String, Box<[u32]>);

/// The document-wide lookups of one version. What formatting resolves against and the lists
/// are read with it; every other lookup on first use (with the version it was made for), so
/// a read of a few blocks that name no control, record, drawing, bookmark, note or comment
/// walks nothing else of the document.
pub struct Shared {
    /// What formatting resolves against.
    pub(super) formatting: Formatting,
    /// The lists.
    pub(super) lists: Lists,
    /// Content controls' ids by place.
    controls: OnceLock<HashMap<Key, String>>,
    /// Revision records' ids by place.
    revisions: OnceLock<HashMap<Key, String>>,
    /// Pictures' ids by place.
    pictures: OnceLock<HashMap<Key, String>>,
    /// Drawings' ids by place.
    drawings: OnceLock<HashMap<Key, String>>,
    /// Notes' ids by `(fn|en, w:id)`.
    notes: OnceLock<HashMap<(String, String), String>>,
    /// Bookmarks internal hyperlinks point at.
    anchors: OnceLock<HashSet<String>>,
    /// Comments by `w:id`.
    comments: OnceLock<HashMap<String, Comment>>,
    /// What the layout model reads of each top-level table read so far, by part and place.
    tables: Mutex<HashMap<Key, Arc<TableFacts>>>,
    /// Whether a table cell's end is one Accept All takes away ([`PartFacts::cell_ends`]).
    cell_ends: OnceLock<bool>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

/// An element's key in its part.
fn key(part: &str, path: &[u32]) -> Key {
    (part.to_owned(), Box::<[u32]>::from(path))
}

impl Shared {
    /// The lookups of `docx`'s version, kept with it.
    #[must_use]
    pub fn of(docx: &Docx) -> Arc<Shared> {
        docx.memo().reading(|| Shared::read(docx))
    }

    /// The lookups of `docx`: what formatting resolves against and the lists read now, the
    /// rest on first use.
    #[must_use]
    pub fn read(docx: &Docx) -> Shared {
        Shared {
            formatting: Formatting::read(docx),
            lists: Lists::read(docx),
            controls: OnceLock::new(),
            revisions: OnceLock::new(),
            pictures: OnceLock::new(),
            drawings: OnceLock::new(),
            notes: OnceLock::new(),
            anchors: OnceLock::new(),
            comments: OnceLock::new(),
            tables: Mutex::new(HashMap::new()),
            cell_ends: OnceLock::new(),
        }
    }

    /// Content controls' ids by place (of `docx`, the version these lookups are of).
    pub(super) fn controls(&self, docx: &Docx) -> &HashMap<Key, String> {
        self.controls.get_or_init(|| {
            controls(docx)
                .into_iter()
                .map(|c| (key(&c.part, &c.path), c.id))
                .collect()
        })
    }

    /// Revision records' ids by place.
    pub(super) fn revisions(&self, docx: &Docx) -> &HashMap<Key, String> {
        self.revisions.get_or_init(|| {
            revision_records(docx)
                .into_iter()
                .map(|r| (key(&r.part, &r.path), r.id))
                .collect()
        })
    }

    /// Pictures' ids by place.
    pub(super) fn pictures(&self, docx: &Docx) -> &HashMap<Key, String> {
        self.pictures.get_or_init(|| {
            pictures(docx)
                .into_iter()
                .map(|d| (key(&d.part, &d.path), d.id))
                .collect()
        })
    }

    /// Drawings' ids by place.
    pub(super) fn drawings(&self, docx: &Docx) -> &HashMap<Key, String> {
        self.drawings.get_or_init(|| {
            all_drawings(docx)
                .into_iter()
                .map(|d| (key(&d.part, &d.path), d.id))
                .collect()
        })
    }

    /// Notes' ids by `(fn|en, w:id)`.
    pub(super) fn notes(&self, docx: &Docx) -> &HashMap<(String, String), String> {
        self.notes.get_or_init(|| notes_of(docx))
    }

    /// Bookmarks internal hyperlinks point at.
    pub(super) fn anchors(&self, docx: &Docx) -> &HashSet<String> {
        self.anchors.get_or_init(|| anchors(docx))
    }

    /// Comments by `w:id`.
    pub(super) fn comments(&self, docx: &Docx) -> &HashMap<String, Comment> {
        self.comments.get_or_init(|| {
            comments(docx)
                .into_iter()
                .filter_map(|c| c.w_id.clone().map(|w| (w, c)))
                .collect()
        })
    }

    /// What the layout model reads of the table `table` of `part` of `docx`, read once
    /// (`None` where the part has no index).
    pub fn table_facts(&self, docx: &Docx, part: &str, table: &TableId) -> Option<Arc<TableFacts>> {
        let key = (part.to_owned(), table.path.clone());
        if let Some(found) = self.tables.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
            return Some(Arc::clone(found));
        }
        let made = Arc::new(TableFacts::of(docx, part, &table.path)?);
        self.tables
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, Arc::clone(&made));
        Some(made)
    }

    /// What formatting resolves against.
    #[must_use]
    pub fn formatting(&self) -> &Formatting {
        &self.formatting
    }

    /// Whether a table cell's end of `docx` is one Accept All takes away: docx-agent's final
    /// view joins the next cell's text in, the core's does not yet.
    #[must_use]
    pub fn cell_ends(&self, docx: &Docx) -> bool {
        *self
            .cell_ends
            .get_or_init(|| docx.parts.iter().any(|p| p.facts().cell_ends()))
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
