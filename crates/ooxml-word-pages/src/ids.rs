//! Owns: a version's body blocks by the tool's ids ([`BlockIds`]): each paragraph's, table's
//! and row's docx2svg path (`w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]`, the paths the layout's
//! lines and pages name blocks by) with its id as docx-agent names it in the version's
//! bytes (`ooxml_word_model::ids::PartIds`: `p:<paraId>`, `p@body/<n>`, `t:...`, rows'
//! ids) -- so a reader asks the layout by id and maps nothing itself (lane L4b).
//!
//! Paths count an element among its siblings of the same local name, from 1, as the Word
//! model writes them (`read::Steps`). The walk goes through every element but a
//! paragraph's content and `mc:Fallback` (as the ids do).
//!
//! A version holds its main part's ids and tree ([`VersionIds`]): one block's path is found
//! from its id when asked, every block's ([`BlockIds`]) only when a reader asks for them all
//! -- a version laid out names nothing it is not asked for.
//!
//! Must not: stamp or change an id, follow an alias (the caller holds those), or read
//! anything but the main part's tree.

use ooxml_word_model::ids::{Entry, PartIds, MC, W};
use ooxml_word_model::value::Text;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// A version's main part ids, its blocks named when asked.
#[derive(Debug, Default)]
pub struct VersionIds {
    /// The main part's root and its ids, where it has a tree.
    part: Option<(Arc<Element>, Arc<PartIds>)>,
    all: OnceLock<BlockIds>,
    /// Each body child's count among the body's children of its local name (from 1), made
    /// when a path is first asked: a path is then found in time of its depth, not of its
    /// place in the body.
    body_counts: OnceLock<Vec<u32>>,
    /// Where a layout path's indexed steps are among the main part's long parents'
    /// children, made as paths are asked.
    paths: ooxml_word_model::read::paths::ChildIndex,
}

impl VersionIds {
    /// The ids `ids` of the main part whose root is `root`.
    pub(crate) fn new(root: Arc<Element>, ids: Arc<PartIds>) -> VersionIds {
        VersionIds {
            part: Some((root, ids)),
            all: OnceLock::new(),
            body_counts: OnceLock::new(),
            paths: ooxml_word_model::read::paths::ChildIndex::default(),
        }
    }

    /// Every body block's id by path, made once when first asked for.
    pub fn all(&self) -> &BlockIds {
        self.all.get_or_init(|| match &self.part {
            Some((root, ids)) => BlockIds::of(root, ids),
            None => BlockIds::default(),
        })
    }

    /// The body path (`w:body/w:p[3]`) of the paragraph, table or row the id `id` names.
    pub fn path_of(&self, id: &str) -> Option<Text> {
        let (root, ids) = self.part.as_ref()?;
        let place = match ids.get(id) {
            Some(Entry::Paragraph(k)) => ids.paragraphs().get(k).map(|p| p.path),
            Some(Entry::Table(k)) => ids.tables().get(k).map(|t| t.path),
            None => None,
        };
        match place {
            Some(place) => path_at(root, &place, &self.body_counts),
            // A row (its id is its table's): from every block's.
            None => self.all().iter().find(|(_, i)| ***i == *id).map(|(p, _)| p.clone()),
        }
    }

    /// Where the paragraph the id `id` names is inside a body paragraph's content (a text
    /// box's paragraph, which the body's lines do not draw): the body path of the paragraph
    /// holding it (`w:body/w:p[2]`), and its own place (child indexes from the root, every
    /// node counted), which a painted line's path is matched with
    /// ([`VersionIds::place_of_path`]). `None` for any other id.
    pub fn in_paragraph(&self, id: &str) -> Option<(Text, Vec<u32>)> {
        let (root, ids) = self.part.as_ref()?;
        let Some(Entry::Paragraph(k)) = ids.get(id) else {
            return None;
        };
        let place = ids.paragraphs().get(k)?.path;
        let place: &[u32] = &place;
        // The first paragraph on the way down (below the body) holds it.
        let mut node: &Element = root.children().get(*place.first()? as usize)?.as_element()?;
        for (depth, step) in place.iter().enumerate().skip(1) {
            node = node.children().get(*step as usize)?.as_element()?;
            if node.name().is(Some(W), "p") {
                let host = place.get(..=depth)?;
                return (depth + 1 < place.len())
                    .then(|| path_at(root, host, &self.body_counts))
                    .flatten()
                    .map(|path| (path, place.to_vec()));
            }
        }
        None
    }

    /// The place (child indexes from the root, every node counted) of the element a layout
    /// path names in the main part (`ooxml_word_model::read::paths::resolve_path`: a text
    /// box's paragraph's `…/w:r[2]/wp:anchor[1]/wps:txbx/w:txbxContent/w:p[1]` too).
    pub fn place_of_path(&self, path: &str) -> Option<Vec<u32>> {
        let (root, _) = self.part.as_ref()?;
        ooxml_word_model::read::paths::resolve_indexed(root, path, &self.paths).map(|(_, place)| place)
    }

    /// What it holds beyond the version's tree, bytes, roughly.
    pub fn weight(&self) -> usize {
        self.all.get().map_or(0, BlockIds::weight)
    }
}

/// The body path of the element at child indexes `place` below `root` (the body's own
/// first): each step named by its local name and counted among its siblings of that name,
/// as [`BlockIds::of`] names it; the body's children counted once into `body_counts`.
fn path_at(root: &Element, place: &[u32], body_counts: &OnceLock<Vec<u32>>) -> Option<Text> {
    let (first, rest) = place.split_first()?;
    let mut node: &Element = root.children().get(*first as usize)?.as_element()?;
    if !node.name().is(Some(W), "body") {
        return None;
    }
    let mut path = String::from("w:body");
    for (depth, step) in rest.iter().enumerate() {
        let at = *step as usize;
        let children = node.children();
        let child: &Element = children.get(at)?.as_element()?;
        let local = child.name().local();
        let count = if depth == 0 {
            *body_counts.get_or_init(|| counts(node)).get(at)?
        } else {
            count_before(node, at, local)
        };
        path.push_str("/w:");
        path.push_str(local);
        path.push_str(&format!("[{count}]"));
        node = child;
    }
    Some(Text::from(path.as_str()))
}

/// Whether `e` is counted among its siblings of its name (an `mc:Fallback` is not).
fn counted(e: &Element) -> bool {
    !e.name().is(Some(MC), "Fallback")
}

/// The count of the child at `at` of `element` among its children named `local` (from 1).
fn count_before(element: &Element, at: usize, local: &str) -> u32 {
    let before = element
        .children()
        .iter()
        .take(at)
        .filter_map(Node::as_element)
        .filter(|e| e.name().local() == local && counted(e))
        .count();
    u32::try_from(before + 1).unwrap_or(u32::MAX)
}

/// Each child's count among `element`'s children of its local name (from 1; 0 for what is
/// not a counted element).
fn counts(element: &Element) -> Vec<u32> {
    let mut seen: HashMap<&str, u32> = HashMap::new();
    element
        .children()
        .iter()
        .map(|node| match node.as_element().filter(|e| counted(e)) {
            Some(e) => {
                let count = seen.entry(e.name().local()).or_insert(0);
                *count += 1;
                *count
            }
            None => 0,
        })
        .collect()
}

/// A version's body blocks' ids by path, and their paths by id.
#[derive(Debug, Clone, Default)]
pub struct BlockIds {
    by_path: HashMap<Text, Text>,
}

impl BlockIds {
    /// The ids of the body of the part whose root is `root`, indexed as `ids`.
    pub fn of(root: &Element, ids: &PartIds) -> BlockIds {
        let rows: HashMap<Vec<u32>, String> = ids
            .tables()
            .iter()
            .flat_map(|t| t.rows.into_iter().map(|(path, id)| (path.to_vec(), id)))
            .collect();
        let mut walk = Walk {
            ids,
            rows,
            out: HashMap::new(),
            place: Vec::new(),
        };
        for (k, node) in root.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            if child.name().is(Some(W), "body") {
                walk.place.push(u32::try_from(k).unwrap_or(u32::MAX));
                walk.container(child, "w:body");
                walk.place.pop();
                break;
            }
        }
        BlockIds { by_path: walk.out }
    }

    /// The id of the paragraph, table or row at `path`.
    pub fn get(&self, path: &str) -> Option<&Text> {
        self.by_path.get(path)
    }

    /// Every block's path and id, in no order.
    pub fn iter(&self) -> impl Iterator<Item = (&Text, &Text)> + '_ {
        self.by_path.iter()
    }

    /// How many blocks it names.
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    /// Whether it names none.
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// What it holds, bytes, roughly.
    pub fn weight(&self) -> usize {
        self.by_path
            .iter()
            .map(|(path, id)| path.len() + id.len() + 4 * std::mem::size_of::<usize>())
            .sum()
    }
}

/// One walk of a part's tree.
struct Walk<'a> {
    ids: &'a PartIds,
    rows: HashMap<Vec<u32>, String>,
    out: HashMap<Text, Text>,
    /// The child indexes from the root of the element walked now.
    place: Vec<u32>,
}

impl Walk<'_> {
    /// Name every block below `element`, whose path is `path`.
    fn container(&mut self, element: &Arc<Element>, path: &str) {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (k, node) in element.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            if child.name().is(Some(MC), "Fallback") {
                continue;
            }
            let local = child.name().local();
            let count = counts.entry(local).or_insert(0);
            *count += 1;
            let here = format!("{path}/w:{local}[{count}]");
            self.place.push(u32::try_from(k).unwrap_or(u32::MAX));
            let named = if child.name().is(Some(W), "tr") {
                self.rows.get(&self.place).cloned()
            } else {
                self.ids
                    .at(self.place.as_slice())
                    .and_then(|entry: Entry| self.ids.id_of(entry))
            };
            if let Some(id) = named {
                self.out.insert(Text::from(here.as_str()), Text::from(id.as_str()));
            }
            if !child.name().is(Some(W), "p") {
                self.container(child, &here);
            }
            self.place.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn a_path_found_from_its_id_is_the_path_every_block_is_named_by() {
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let cell = "<w:tc><w:p/><w:p/></w:tc>";
        let body = format!(
            "<w:p/><w:tbl><w:tr>{cell}{cell}</w:tr><w:tr>{cell}</w:tr></w:tbl><w:p/>\n<w:sdt><w:sdtContent><w:p/></w:sdtContent></w:sdt><w:p/><w:tbl><w:tr>{cell}</w:tr></w:tbl>"
        );
        let xml = format!(r#"<w:document xmlns:w="{w}"><w:body>{body}<w:sectPr/></w:body></w:document>"#);
        let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_base::Limits::default().xml).unwrap();
        let root = Arc::clone(tree.root());
        let ids = Arc::new(PartIds::new("body", &root));
        let version = VersionIds::new(Arc::clone(&root), ids);
        let named: Vec<(Text, Text)> = version.all().iter().map(|(p, i)| (p.clone(), i.clone())).collect();
        assert!(named.len() > 10, "{named:?}");
        let fresh = VersionIds::new(root, Arc::new(PartIds::new("body", tree.root())));
        for (path, id) in &named {
            assert_eq!(fresh.path_of(id).as_ref(), Some(path), "{id}");
        }
    }
}
