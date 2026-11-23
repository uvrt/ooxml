//! Owns: a version's body blocks by the tool's ids ([`VersionIds`]): each paragraph's, table's
//! and row's docx2svg path (`w:body/w:tbl[2]/w:tr[3]/w:tc[1]/w:p[1]`, the paths the layout's
//! lines and pages name blocks by) with its id as docx-agent names it in the version's
//! bytes (`ooxml_word_model::ids::PartIds`: `p:<paraId>`, `p@body/<n>`, `t:...`, rows'
//! ids) -- so a reader asks the layout by id and maps nothing itself (lane L4b).
//!
//! Paths count an element among its siblings of the same local name, from 1, as the Word
//! model writes them (`read::Steps`). A block is named through every element but a
//! paragraph's content and `mc:Fallback` (as the ids do).
//!
//! A version holds its main part's ids and tree: one block's path is found from its id,
//! and one path's id from the path, when asked -- each in time of the path's depth (the
//! body's children by name indexed once), each id named once. Nothing walks the body to
//! name every block: a check names only the blocks it reports.
//!
//! Must not: stamp or change an id, follow an alias (the caller holds those), or read
//! anything but the main part's tree.

use ooxml_word_model::ids::{Entry, PartIds, MC, W};
use ooxml_word_model::value::Text;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// A version's main part ids, its blocks named when asked.
#[derive(Debug, Default)]
pub struct VersionIds {
    /// The main part's root and its ids, where it has a tree.
    part: Option<(Arc<Element>, Arc<PartIds>)>,
    /// Each paragraph's and table's id by its ordinal, named when first asked.
    named: OnceLock<Named>,
    /// What each body path asked names (`None`: no block the ids name), found once.
    found: Mutex<HashMap<Box<str>, Option<Found>>>,
    /// Each row's id by its place, read from the tables when a row is first asked.
    rows: OnceLock<HashMap<Vec<u32>, Text>>,
    /// Each body child's count among the body's children of its local name (from 1), made
    /// when a path is first asked: a path is then found in time of its depth, not of its
    /// place in the body.
    body_counts: OnceLock<Vec<u32>>,
    /// A long parent's (the body's, a long table's) counted children by local name (child
    /// indexes, in order), by the parent's address in the main part's tree (which the
    /// version holds), made when a path first steps through it.
    long: Mutex<HashMap<usize, Arc<ByName>>>,
    /// Where a layout path's indexed steps are among the main part's long parents'
    /// children, made as paths are asked.
    paths: ooxml_word_model::read::paths::ChildIndex,
}

/// What a body path names: a paragraph or table by its ordinal, or a row by its place.
#[derive(Debug, Clone)]
enum Found {
    Block(Entry),
    Row(Arc<[u32]>),
}

/// The ids of a part's paragraphs and tables, each named when first asked.
#[derive(Debug)]
struct Named {
    paragraphs: Box<[OnceLock<Option<Text>>]>,
    tables: Box<[OnceLock<Option<Text>>]>,
}

impl Named {
    fn of(ids: &PartIds) -> Named {
        let slots = |n: usize| (0..n).map(|_| OnceLock::new()).collect();
        Named {
            paragraphs: slots(ids.paragraphs().len()),
            tables: slots(ids.tables().len()),
        }
    }

    fn weight(&self) -> usize {
        self.paragraphs
            .iter()
            .chain(self.tables.iter())
            .map(|slot| {
                std::mem::size_of::<OnceLock<Option<Text>>>()
                    + slot.get().and_then(Option::as_ref).map_or(0, |id| id.len())
            })
            .sum()
    }
}

impl VersionIds {
    /// The ids `ids` of the main part whose root is `root`.
    pub(crate) fn new(root: Arc<Element>, ids: Arc<PartIds>) -> VersionIds {
        VersionIds {
            part: Some((root, ids)),
            ..VersionIds::default()
        }
    }

    /// The id of the paragraph, table or row at body path `path` (`w:body/w:p[3]`), as the
    /// tool names it; `None` where the path names no block of the body the ids name.
    pub fn id_of(&self, path: &str) -> Option<&str> {
        let (root, ids) = self.part.as_ref()?;
        let found = {
            let known = self
                .found
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(path)
                .cloned();
            match known {
                Some(found) => found,
                None => {
                    let found = self.find(root, ids, path);
                    self.found
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(path.into(), found.clone());
                    found
                }
            }
        };
        match found? {
            Found::Row(place) => self.rows(ids).get(&*place).map(|id| &**id),
            Found::Block(entry) => {
                let named = self.named.get_or_init(|| Named::of(ids));
                let slot = match entry {
                    Entry::Paragraph(k) => named.paragraphs.get(k)?,
                    Entry::Table(k) => named.tables.get(k)?,
                };
                slot.get_or_init(|| ids.id_of(entry).map(|id| Text::from(id.as_str())))
                    .as_deref()
            }
        }
    }

    /// What the body path `path` names among the blocks the ids name, found from its steps.
    fn find(&self, root: &Element, ids: &PartIds, path: &str) -> Option<Found> {
        let (place, row) = self.block_place(root, path)?;
        if row {
            return Some(Found::Row(place.into()));
        }
        ids.at(&place).map(Found::Block)
    }

    /// Every row's id by its place (child indexes from the root).
    fn rows(&self, ids: &PartIds) -> &HashMap<Vec<u32>, Text> {
        self.rows.get_or_init(|| {
            ids.tables()
                .iter()
                .flat_map(|t| t.rows.into_iter())
                .map(|(place, id)| (place.to_vec(), Text::from(id.as_str())))
                .collect()
        })
    }

    /// The place (child indexes from the root) of the block a body path names, and
    /// whether it is a row: each step `w:<local>[<n>]` the n-th counted child of that local
    /// name, from the root's first `w:body`, through no paragraph (the inverse of the
    /// paths [`path_at`] writes).
    fn block_place(&self, root: &Element, path: &str) -> Option<(Vec<u32>, bool)> {
        let mut steps = path.split('/');
        if steps.next()? != "w:body" {
            return None;
        }
        let (at, body) = root
            .children()
            .iter()
            .enumerate()
            .find_map(|(k, n)| n.as_element().filter(|e| e.name().is(Some(W), "body")).map(|e| (k, e)))?;
        let mut place = vec![u32::try_from(at).ok()?];
        let mut node: &Element = body;
        for (depth, text) in steps.enumerate() {
            if depth > 0 && node.name().is(Some(W), "p") {
                return None;
            }
            let (local, n) = parse_step(text)?;
            let k = self.nth_named(node, local, n)?;
            node = node.children().get(k as usize)?.as_element()?;
            place.push(k);
        }
        (place.len() > 1).then(|| (place, node.name().is(Some(W), "tr")))
    }

    /// The child index of `parent`'s `n`-th counted child named `local` (from 1): from the
    /// parent's children by name, made once, where it has many; else counted through.
    fn nth_named(&self, parent: &Element, local: &str, n: usize) -> Option<u32> {
        if parent.children().len() < INDEXED_FROM {
            return nth_named(parent, local, n);
        }
        let named = {
            let mut long = self.long.lock().unwrap_or_else(PoisonError::into_inner);
            let key = std::ptr::from_ref(parent) as usize;
            Arc::clone(long.entry(key).or_insert_with(|| Arc::new(by_name(parent))))
        };
        named.get(local)?.get(n - 1).copied()
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
            // A row (its id is its table's): its place among the tables' rows.
            None => {
                let rows = self.rows(ids);
                let mut found: Vec<&Vec<u32>> = rows.iter().filter(|(_, i)| ***i == *id).map(|(p, _)| p).collect();
                found.sort();
                // Only a row a path names (none inside a paragraph's content).
                found
                    .into_iter()
                    .filter_map(|place| path_at(root, place, &self.body_counts))
                    .find(|path| self.id_of(path) == Some(id))
            }
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
        let rows = self.rows.get().map_or(0, |rows| {
            rows.iter()
                .map(|(place, id)| place.len() * 4 + id.len() + 4 * std::mem::size_of::<usize>())
                .sum()
        });
        let named: usize = self
            .long
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .flat_map(|named| named.iter())
            .map(|(name, places)| name.len() + places.len() * 4 + 4 * std::mem::size_of::<usize>())
            .sum();
        let found: usize = self
            .found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .map(|path| path.len() + 6 * std::mem::size_of::<usize>())
            .sum();
        self.named.get().map_or(0, Named::weight) + rows + named + found
    }
}

/// A path step `w:<local>[<n>]` as [`path_at`] writes it: its local name and `n` (from 1,
/// written without leading zeros).
fn parse_step(text: &str) -> Option<(&str, usize)> {
    let (local, rest) = text.strip_prefix("w:")?.split_once('[')?;
    let digits = rest.strip_suffix(']')?;
    let n: usize = digits.parse().ok()?;
    (n >= 1 && n.to_string() == digits && !local.is_empty()).then_some((local, n))
}

/// A parent with fewer children than this is counted through for each step: only a long
/// one's children are indexed by name.
const INDEXED_FROM: usize = 64;

/// A parent's counted children by local name: each name's child indexes, in order.
type ByName = HashMap<String, Vec<u32>>;

/// `element`'s counted children by local name.
fn by_name(element: &Element) -> ByName {
    let mut out: HashMap<String, Vec<u32>> = HashMap::new();
    for (k, node) in element.children().iter().enumerate() {
        if let Some(e) = node.as_element().filter(|e| counted(e)) {
            out.entry(e.name().local().to_owned())
                .or_default()
                .push(u32::try_from(k).unwrap_or(u32::MAX));
        }
    }
    out
}

/// The child index of `element`'s `n`-th counted child named `local` (from 1).
fn nth_named(element: &Element, local: &str, n: usize) -> Option<u32> {
    let (k, _) = element
        .children()
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.as_element()
                .is_some_and(|e| e.name().local() == local && counted(e))
        })
        .nth(n - 1)?;
    u32::try_from(k).ok()
}

/// The body path of the element at child indexes `place` below `root` (the body's own
/// first): each step named by its local name and counted among its siblings of that name,
/// as a body walk names it; the body's children counted once into `body_counts`.
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    /// Every body block's id by path, named by one walk of the body: what [`VersionIds::id_of`]
    /// answers path by path.
    #[derive(Debug, Clone, Default)]
    struct BlockIds {
        by_path: HashMap<Text, Text>,
    }

    impl BlockIds {
        /// The ids of the body of the part whose root is `root`, indexed as `ids`.
        fn of(root: &Element, ids: &PartIds) -> BlockIds {
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
        fn get(&self, path: &str) -> Option<&Text> {
            self.by_path.get(path)
        }

        /// Every block's path and id, in no order.
        fn iter(&self) -> impl Iterator<Item = (&Text, &Text)> + '_ {
            self.by_path.iter()
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

    /// A body of every kind of block: tables (one nested, one in a content control),
    /// paragraphs in content controls and custom XML, a choice with its fallback, a
    /// paragraph holding a text box with a table, and an element of another namespace.
    fn made_up() -> Arc<Element> {
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let mc = "http://schemas.openxmlformats.org/markup-compatibility/2006";
        let cell = "<w:tc><w:p/><w:p/></w:tc>";
        let nested = format!("<w:tc><w:tbl><w:tr>{cell}</w:tr><w:tr>{cell}{cell}</w:tr></w:tbl><w:p/></w:tc>");
        let text_box = format!(
            "<w:p><w:r><w:pict><w:txbxContent><w:tbl><w:tr>{cell}</w:tr></w:tbl><w:p/></w:txbxContent></w:pict></w:r></w:p>"
        );
        let body = format!(
            "<w:p/><w:tbl><w:tr>{cell}{cell}</w:tr><w:tr>{nested}</w:tr></w:tbl><w:p/>\n\
             <w:sdt><w:sdtContent><w:p/><w:tbl><w:tr>{cell}</w:tr></w:tbl></w:sdtContent></w:sdt>\
             <w:customXml><w:p/></w:customXml>\
             <mc:AlternateContent><mc:Choice><w:p/></mc:Choice><mc:Fallback><w:p/></mc:Fallback></mc:AlternateContent>\
             {text_box}<m:oMathPara xmlns:m=\"urn:math\"/><w:p/><w:tbl><w:tr>{cell}</w:tr></w:tbl>"
        );
        let xml =
            format!(r#"<w:document xmlns:w="{w}" xmlns:mc="{mc}"><w:body>{body}<w:sectPr/></w:body></w:document>"#);
        let tree = ooxml_xml::parse(xml.as_bytes(), &ooxml_base::Limits::default().xml).unwrap();
        Arc::clone(tree.root())
    }

    #[test]
    fn a_path_names_the_block_one_walk_of_the_body_names_and_no_other() {
        let root = made_up();
        let ids = Arc::new(PartIds::new("body", &root));
        let walked = BlockIds::of(&root, &ids);
        let version = VersionIds::new(Arc::clone(&root), Arc::clone(&ids));
        let named: Vec<(Text, Text)> = walked.iter().map(|(p, i)| (p.clone(), i.clone())).collect();
        assert!(named.len() > 20, "{named:?}");
        assert!(named.iter().any(|(p, _)| p.contains("w:tr[")), "{named:?}");
        for (path, id) in &named {
            assert_eq!(version.id_of(path), Some(&**id), "{path}");
            assert_eq!(version.path_of(id).as_ref(), Some(path), "{id}");
        }
        // Paths the walk does not name: steps through a paragraph, the body, odd spellings.
        let first = named
            .iter()
            .find(|(p, _)| p.ends_with("w:p[1]"))
            .map(|(p, _)| p.clone())
            .unwrap();
        for path in [
            "w:body".to_owned(),
            "w:body/w:p[0]".to_owned(),
            "w:body/w:p[01]".to_owned(),
            "w:body/w:p[99]".to_owned(),
            "w:body/p[1]".to_owned(),
            "w:body/w:Fallback[1]".to_owned(),
            "w:body/w:AlternateContent[1]/w:Fallback[1]/w:p[1]".to_owned(),
            "w:body/w:p[3]/w:r[1]/w:pict[1]/w:txbxContent[1]/w:tbl[1]".to_owned(),
            "w:body/w:p[3]/w:r[1]/w:pict[1]/w:txbxContent[1]/w:tbl[1]/w:tr[1]".to_owned(),
            "w:body/w:sectPr[1]".to_owned(),
            "w:body/w:oMathPara[1]".to_owned(),
            format!("{first}/w:r[1]"),
            "w:hdr/w:p[1]".to_owned(),
        ] {
            assert_eq!(version.id_of(&path), walked.get(&path).map(|i| &**i), "{path}");
        }
        // Every element's path the way the walk writes them, named or not.
        let mut every = Vec::new();
        let body = root.children().iter().find_map(Node::as_element).unwrap();
        all_paths(body, "w:body", &mut every);
        assert!(every.len() > named.len());
        for path in &every {
            assert_eq!(version.id_of(path), walked.get(path).map(|i| &**i), "{path}");
        }
    }

    /// The path of every element below `element` (whose path is `path`), counted as the
    /// walk counts, through paragraphs too.
    fn all_paths(element: &Element, path: &str, out: &mut Vec<String>) {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for node in element.children().iter() {
            let Some(child) = node.as_element() else { continue };
            if !counted(child) {
                continue;
            }
            let local = child.name().local();
            let count = counts.entry(local).or_insert(0);
            *count += 1;
            let here = format!("{path}/w:{local}[{count}]");
            out.push(here.clone());
            all_paths(child, &here, out);
        }
    }
}
