//! Owns: one segment of a part's block index -- the live paragraphs, rows and tables found
//! in one child of the part's block container (the body's child, or a story root's), with
//! their places below that child, kinds, owning tables and paraIds as written -- and the
//! walk that finds them, as docx-agent's `PartIndex` walks (`mc:Fallback` content left out).
//!
//! Must not: know where its segment is (its place and order key belong to the index), or
//! name a block (names need the whole part: ordinals and repeats).

use crate::ids::{valid_long_hex, MC, W, W14};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// What a found block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    P,
    Tr,
    Tbl,
}

/// A live block of a segment.
#[derive(Debug, Clone)]
pub(super) struct Block {
    pub(super) element: Arc<Element>,
    /// Its place below the segment's node (empty: the node itself).
    pub(super) rel: Box<[u32]>,
    pub(super) kind: Kind,
    /// For a row, the place in the segment's blocks of its nearest table.
    pub(super) owner: Option<u32>,
    /// The `w14:paraId` as written (paragraphs and rows).
    pub(super) raw: Option<Arc<str>>,
    /// The same, upper-cased (repeats are counted in any case).
    pub(super) upper: Option<Arc<str>>,
    /// Its ordinal among the segment's blocks of its kind (paragraphs, tables).
    pub(super) ordinal: u32,
}

impl Block {
    /// A paragraph or row whose paraId is missing or out of range (stamping gives it one).
    pub(super) fn unstamped(&self) -> bool {
        self.kind != Kind::Tbl && !self.raw.as_deref().is_some_and(valid_long_hex)
    }
}

/// The blocks of one container child.
#[derive(Debug, Default)]
pub(super) struct Body {
    /// The child (compared by allocation with a later version's).
    pub(super) node: Option<Node>,
    /// Its blocks in document order (their `rel`s sorted).
    pub(super) blocks: Box<[Block]>,
    /// The places in `blocks` of its paragraphs, in order.
    pub(super) paragraphs: Box<[u32]>,
    /// The places in `blocks` of its tables, in order.
    pub(super) tables: Box<[u32]>,
    /// How many of its paragraphs and rows are unstamped.
    pub(super) unstamped: u32,
}

impl Body {
    /// The segment of `node` (a container child).
    pub(super) fn of(node: &Node) -> Body {
        let mut blocks = Vec::new();
        if let Node::Element(element) = node {
            if !element.name().is(Some(MC), "Fallback") {
                let mut tables = Vec::new();
                visit(element, &mut Vec::new(), &mut tables, &mut blocks);
            }
        }
        Body::from_blocks(Some(node.clone()), blocks)
    }

    /// The blocks below `root` outside its child `skip` (the container), split at it:
    /// those before and those after, with places from the root.
    pub(super) fn outer(root: &Element, skip: Option<usize>) -> (Body, Body) {
        let (mut before, mut after) = (Vec::new(), Vec::new());
        for (k, node) in root.children().iter().enumerate() {
            if Some(k) == skip {
                continue;
            }
            let Node::Element(child) = node else { continue };
            if child.name().is(Some(MC), "Fallback") {
                continue;
            }
            let into = if skip.is_some_and(|s| k > s) {
                &mut after
            } else {
                &mut before
            };
            let mut path = vec![u32::try_from(k).unwrap_or(u32::MAX)];
            let mut tables = Vec::new();
            found(child, &path, &mut tables, into);
            visit_children(child, &mut path, &mut tables, into);
        }
        (Body::from_blocks(None, before), Body::from_blocks(None, after))
    }

    fn from_blocks(node: Option<Node>, mut blocks: Vec<Block>) -> Body {
        let (mut paragraphs, mut tables) = (Vec::new(), Vec::new());
        let mut unstamped = 0;
        for (k, block) in blocks.iter_mut().enumerate() {
            let k = u32::try_from(k).unwrap_or(u32::MAX);
            match block.kind {
                Kind::P => {
                    block.ordinal = u32::try_from(paragraphs.len()).unwrap_or(u32::MAX);
                    paragraphs.push(k);
                }
                Kind::Tbl => {
                    block.ordinal = u32::try_from(tables.len()).unwrap_or(u32::MAX);
                    tables.push(k);
                }
                Kind::Tr => {}
            }
            unstamped += u32::from(block.unstamped());
        }
        Body {
            node,
            blocks: blocks.into(),
            paragraphs: paragraphs.into(),
            tables: tables.into(),
            unstamped,
        }
    }

    /// The block at `rel`, with its place.
    pub(super) fn at(&self, rel: &[u32]) -> Option<(usize, &Block)> {
        let k = self.blocks.binary_search_by(|b| (*b.rel).cmp(rel)).ok()?;
        Some((k, self.blocks.get(k)?))
    }

    /// Whether this segment was read from `node` (the same allocation).
    pub(super) fn is_of(&self, node: &Node) -> bool {
        self.node.as_ref().is_some_and(|n| n.same(node))
    }
}

fn kind_of(element: &Element) -> Option<Kind> {
    let name = element.name();
    if name.namespace() != Some(W) {
        return None;
    }
    match name.local() {
        "p" => Some(Kind::P),
        "tr" => Some(Kind::Tr),
        "tbl" => Some(Kind::Tbl),
        _ => None,
    }
}

/// Record `element` (at `path`) when it is a block; a table is pushed on `tables`.
fn found(element: &Arc<Element>, path: &[u32], tables: &mut Vec<u32>, out: &mut Vec<Block>) -> bool {
    let Some(kind) = kind_of(element) else { return false };
    let raw = match kind {
        Kind::Tbl => None,
        Kind::P | Kind::Tr => element.attribute(Some(W14), "paraId").map(Arc::<str>::from),
    };
    let upper = raw.as_deref().map(|r| Arc::<str>::from(r.to_uppercase()));
    out.push(Block {
        element: Arc::clone(element),
        rel: path.into(),
        kind,
        owner: if kind == Kind::Tr { tables.last().copied() } else { None },
        raw,
        upper,
        ordinal: 0,
    });
    if kind == Kind::Tbl {
        tables.push(u32::try_from(out.len() - 1).unwrap_or(u32::MAX));
        return true;
    }
    false
}

/// `element` (a container child, at the empty place) and everything below it.
fn visit(element: &Arc<Element>, path: &mut Vec<u32>, tables: &mut Vec<u32>, out: &mut Vec<Block>) {
    let table = found(element, path, tables, out);
    visit_children(element, path, tables, out);
    if table {
        tables.pop();
    }
}

fn visit_children(element: &Element, path: &mut Vec<u32>, tables: &mut Vec<u32>, out: &mut Vec<Block>) {
    for (k, node) in element.children().iter().enumerate() {
        let Node::Element(child) = node else { continue };
        if child.name().is(Some(MC), "Fallback") {
            continue;
        }
        path.push(u32::try_from(k).unwrap_or(u32::MAX));
        let table = found(child, path, tables, out);
        visit_children(child, path, tables, out);
        if table {
            tables.pop();
        }
        path.pop();
    }
}
