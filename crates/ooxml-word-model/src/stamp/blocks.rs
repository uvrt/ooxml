//! Owns: giving one story part's blocks durable ids, as docx-agent's default stamping
//! policy does (`Document._stamp_document`): every paragraph and row without a valid and
//! unique `w14:paraId` gets a paraId and a textId, every drawing without a valid anchorId
//! and editId gets them, each kind drawn from its own seed in one draw (the part's name for
//! paragraphs, `part\0row`, `part\0drawing`) in document order; `w14` (and `wp14`) then
//! declared at the root ([`Unstamped::stamp`]); and the renames that names the blocks had
//! before.
//!
//! Must not: draw a value itself (the caller's draw does: a pool at open, an edit's sink
//! after a command), or stamp a block with a valid and unique id.

use super::declared::ensured_head;
use super::namespaces::cleaned;
use crate::docx::live_paths;
use crate::ids::{valid_long_hex, BlockId, IdState, PartIds, Renames, TableId, W14, WP, WP14};
use ooxml_xml::{Attribute, Element, Head, Name, Node, QName};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;
use std::sync::Arc;

/// Attributes to set on an element, in order: each one's name (with the prefix it is
/// written with there) and value.
type Attrs = Vec<(Name, String)>;

/// The names of a pair of ids: their namespace, its usual prefix, and the two locals.
type Names = (&'static str, &'static str, [&'static str; 2]);

const BLOCK_IDS: Names = (W14, "w14", ["paraId", "textId"]);
const DRAWING_IDS: Names = (WP14, "wp14", ["anchorId", "editId"]);

/// The blocks of one story part that need ids ([`unstamped`]): what [`Unstamped::stamp`]
/// gives them.
#[derive(Debug)]
pub struct Unstamped<'p> {
    part: &'p str,
    prefix: String,
    root: &'p Arc<Element>,
    /// Each paragraph's place and the id it goes by.
    paragraphs: Vec<(Box<[u32]>, String)>,
    /// Each row's place, the id it goes by, and its table's id where it is the first row.
    rows: Vec<(Box<[u32]>, String, Option<String>)>,
    drawings: Vec<Box<[u32]>>,
}

/// The blocks of the part `part` of the story `story` (its root `root`, its index `ids`)
/// without a durable id; `None` when it has none. Drawings are looked for only where
/// `drawings` says the part may have one without its ids (a caller's memo).
#[must_use]
pub fn unstamped<'p>(
    story: &str,
    part: &'p str,
    root: &'p Arc<Element>,
    ids: &PartIds,
    drawings: bool,
) -> Option<Unstamped<'p>> {
    let prefix = if story == "body" {
        String::new()
    } else {
        format!("{story}/")
    };
    // the index counts what lacks an id: a stamped part is not walked
    let (paragraphs, rows) = if ids.fully_stamped() {
        (Vec::new(), Vec::new())
    } else {
        let paragraphs = ids
            .paragraphs()
            .iter()
            .filter(|p| p.state != IdState::Ok)
            .map(|p| (p.path, p.id))
            .collect();
        (paragraphs, unstamped_rows(ids, &prefix))
    };
    let drawings: Vec<Box<[u32]>> = if drawings {
        live_paths(root, &[], &|e| {
            (e.name().is(Some(WP), "inline") || e.name().is(Some(WP), "anchor"))
                && !(e.attribute(Some(WP14), "anchorId").is_some_and(valid_long_hex)
                    && e.attribute(Some(WP14), "editId").is_some_and(valid_long_hex))
        })
        .into_iter()
        .map(|d| d.path)
        .collect()
    } else {
        Vec::new()
    };
    (!(paragraphs.is_empty() && rows.is_empty() && drawings.is_empty())).then_some(Unstamped {
        part,
        prefix,
        root,
        paragraphs,
        rows,
        drawings,
    })
}

/// The rows of `ids`' tables without a durable id, in document order: each one's place,
/// its id, and its table's id where it is the table's first row.
fn unstamped_rows(ids: &PartIds, prefix: &str) -> Vec<(Box<[u32]>, String, Option<String>)> {
    let stamped = format!("{prefix}tr:");
    ids.tables()
        .iter()
        .flat_map(|table: TableId| {
            let TableId { id, rows, .. } = table;
            rows.into_iter()
                .enumerate()
                .filter(|(_, (_, row))| !row.starts_with(&stamped))
                .map(|(k, (path, row))| (path, row, (k == 0).then(|| id.clone())))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// What stamping made of a part: its root, and the renames (each block's id before, and
/// its durable id), paragraphs and rows in document order, then the tables.
#[derive(Debug, Clone)]
pub struct Stamp {
    /// The part's root, every block stamped.
    pub root: Arc<Element>,
    /// The renames.
    pub renames: Renames,
    /// Every value drawn, in the order given: what the part's stamps are known by.
    pub given: Vec<String>,
}

impl Unstamped<'_> {
    /// The part stamped, its ids drawn by `draw` (a seed and a count: that many new values,
    /// upper-case hex): one draw per seed.
    ///
    /// # Errors
    /// What `draw` returns.
    pub fn stamp<E>(&self, draw: &mut dyn FnMut(&str, usize) -> Result<Vec<String>, E>) -> Result<Stamp, E> {
        self.stamp_inheriting(&HashMap::new(), draw)
    }

    /// The places of the paragraphs and rows to stamp, in document order.
    pub fn blocks(&self) -> impl Iterator<Item = &[u32]> + '_ {
        let paragraphs = self.paragraphs.iter().map(|(path, _)| &**path);
        paragraphs.chain(self.rows.iter().map(|(path, _, _)| &**path))
    }

    /// [`Unstamped::stamp`], a paragraph or row whose place `inherited` holds given those
    /// ids (its paraId and textId) instead of drawn ones.
    ///
    /// # Errors
    /// What `draw` returns.
    pub fn stamp_inheriting<E>(
        &self,
        inherited: &HashMap<Box<[u32]>, [String; 2]>,
        draw: &mut dyn FnMut(&str, usize) -> Result<Vec<String>, E>,
    ) -> Result<Stamp, E> {
        let part = self.part;
        let prefix = &self.prefix;
        let mut sets: Vec<(Box<[u32]>, Attrs)> = Vec::new();
        let mut renames = Renames::new();
        let mut given = Vec::new();
        let paths: Vec<&[u32]> = self.paragraphs.iter().map(|(path, _)| &**path).collect();
        let drawn = self.pairs(draw, part, &paths, inherited, BLOCK_IDS, &mut given)?;
        for ((path, id), (attrs, para)) in self.paragraphs.iter().zip(drawn) {
            sets.push((path.clone(), attrs));
            renames.push((BlockId::from(id), BlockId::from(format!("{prefix}p:{para}"))));
        }
        let paths: Vec<&[u32]> = self.rows.iter().map(|(path, _, _)| &**path).collect();
        let drawn = self.pairs(draw, &format!("{part}\0row"), &paths, inherited, BLOCK_IDS, &mut given)?;
        let mut tables = Vec::new();
        for ((path, row, table), (attrs, para)) in self.rows.iter().zip(drawn) {
            sets.push((path.clone(), attrs));
            renames.push((BlockId::from(row), BlockId::from(format!("{prefix}tr:{para}"))));
            if let Some(table) = table {
                tables.push((BlockId::from(table), BlockId::from(format!("{prefix}t:{para}"))));
            }
        }
        renames.extend(tables);
        let paths: Vec<&[u32]> = self.drawings.iter().map(|path| &**path).collect();
        let none = HashMap::new();
        let drawn = self.pairs(
            draw,
            &format!("{part}\0drawing"),
            &paths,
            &none,
            DRAWING_IDS,
            &mut given,
        )?;
        for (path, (attrs, _)) in self.drawings.iter().zip(drawn) {
            sets.push((path.clone(), attrs));
        }
        let mut root = rewrite(
            self.root,
            &[],
            &sets.iter().map(|(p, s)| (&**p, s)).collect::<BTreeMap<_, _>>(),
        );
        if !self.paragraphs.is_empty() {
            root = ensured(&root, false);
        }
        root = ensured(&root, !self.drawings.is_empty());
        Ok(Stamp { root, renames, given })
    }

    /// Two ids for each element of `paths`, in order: those `inherited` holds for its place,
    /// else drawn from `seed`, all at once (one draw per seed: an edit's draw copies the
    /// version's id maps); for each, the attributes `locals` (in `namespace`, written with the
    /// prefix in scope there) and the first id. Every value drawn goes in `given`.
    fn pairs<E>(
        &self,
        draw: &mut dyn FnMut(&str, usize) -> Result<Vec<String>, E>,
        seed: &str,
        paths: &[&[u32]],
        inherited: &HashMap<Box<[u32]>, [String; 2]>,
        names: Names,
        given: &mut Vec<String>,
    ) -> Result<Vec<(Attrs, String)>, E> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let (namespace, usual, locals) = names;
        let wanted = paths.iter().filter(|p| !inherited.contains_key(**p)).count();
        let ids = if wanted == 0 {
            Vec::new()
        } else {
            draw(seed, 2 * wanted)?
        };
        let mut fresh = ids.as_chunks::<2>().0.iter();
        let pairs: Vec<[String; 2]> = paths
            .iter()
            .map(|path| match inherited.get(*path) {
                Some(pair) => pair.clone(),
                None => fresh.next().cloned().unwrap_or_default(),
            })
            .collect();
        let out = paths
            .iter()
            .zip(&pairs)
            .map(|(path, pair)| {
                let prefix = prefix_at(self.root, path, namespace, usual);
                let attrs: Attrs = locals
                    .iter()
                    .zip(pair)
                    .map(|(local, value)| {
                        (
                            Arc::new(QName::new(Some(&prefix), local, Some(namespace))),
                            value.clone(),
                        )
                    })
                    .collect();
                let [first, _] = pair;
                (attrs, first.clone())
            })
            .collect();
        given.extend(ids);
        Ok(out)
    }
}

/// `root` with `w14` (and `wp14`) declared as docx-agent's `ensure_w14` leaves it.
fn ensured(root: &Arc<Element>, drawings: bool) -> Arc<Element> {
    match ensured_head(root, drawings, &[]) {
        None => Arc::clone(root),
        Some((head, declared)) => {
            let new = Arc::new(Element::from_head(Arc::new(head), root.children().clone()));
            if declared {
                cleaned(&new)
            } else {
                new
            }
        }
    }
}

/// The prefix `namespace`'s attributes are written with at the element at `path` below
/// `root`: the nearest declaration on the way down, else the namespace's usual prefix.
#[must_use]
pub fn prefix_at(root: &Element, path: &[u32], namespace: &str, usual: &str) -> String {
    let declared = |e: &Element| {
        e.namespaces()
            .iter()
            .find(|d| &*d.uri == namespace && d.prefix.is_some())
            .and_then(|d| d.prefix.as_deref().map(str::to_owned))
    };
    let mut found = declared(root);
    let mut at = root;
    for k in path {
        let Some(child) = at.children().get(*k as usize).and_then(Node::as_element) else {
            break;
        };
        at = child;
        found = declared(at).or(found);
    }
    found.unwrap_or_else(|| usual.to_owned())
}

/// `element` (at `place`) with the attributes of `sets` set on the elements they name,
/// every unchanged subtree shared.
fn rewrite(element: &Arc<Element>, place: &[u32], sets: &BTreeMap<&[u32], &Attrs>) -> Arc<Element> {
    // the paths below `place` follow it in the map's order (one range, not every path:
    // walking all of them at every element is quadratic in a part's blocks)
    let below: Vec<u32> = sets
        .range::<[u32], _>((Bound::Excluded(place), Bound::Unbounded))
        .map(|(p, _)| *p)
        .take_while(|p| p.starts_with(place))
        .filter_map(|p| p.get(place.len()).copied())
        .collect();
    let mut children = element.children().clone();
    let mut done = BTreeSet::new();
    for k in below {
        if !done.insert(k) {
            continue;
        }
        let Some(Node::Element(child)) = element.children().get(k as usize) else {
            continue;
        };
        let mut path = place.to_vec();
        path.push(k);
        let edited = rewrite(child, &path, sets);
        if let Some(spliced) = children.splice(k as usize..k as usize + 1, vec![Node::Element(edited)]) {
            children = spliced;
        }
    }
    match sets.get(place) {
        Some(attributes) => {
            let mut head: Head = (**element.head()).clone();
            for (name, value) in attributes.iter() {
                head = with_attribute(&head, name, value);
            }
            Arc::new(Element::from_head(Arc::new(head), children))
        }
        None => Arc::new(element.with_children(children)),
    }
}

/// `head` with the attribute `name` set to `value`: in its place when the element has it,
/// else after the others.
#[must_use]
pub fn with_attribute(head: &Head, name: &Name, value: &str) -> Head {
    let mut attributes = head.attributes.to_vec();
    let local = name.local();
    match attributes.iter_mut().find(|a| a.name.is(name.namespace(), local)) {
        Some(a) => a.value = Arc::from(value),
        None => attributes.push(Attribute {
            name: Arc::clone(name),
            value: Arc::from(value),
        }),
    }
    Head {
        name: head.name.clone(),
        namespaces: head.namespaces.clone(),
        attributes: attributes.into(),
    }
}
