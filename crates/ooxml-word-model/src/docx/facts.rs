//! Owns: [`PartFacts`], what docx-agent's document-wide lookups read of one story part --
//! its content controls, revision records and drawing frames with their places (live
//! elements, `mc:Fallback` left out, in document order), the bookmarks its hyperlinks point
//! at, and whether a table cell's end is one Accept All takes away -- found in one walk and
//! kept with the part, each top-level block's facts apart: a later version of the part
//! takes every block it shares with the version before (the same subtree, by pointer) from
//! that version's facts and walks only the blocks that changed.
//!
//! Must not: number anything document-wide (`cc:<id>#k` and the like count across parts:
//! [`super::controls`], [`super::revisions`] and [`super::drawings`] number them from these
//! facts), or keep anything a part's tree does not determine.

use crate::docx::revisions::is_revision;
use crate::docx::Found;
use crate::ids::{MC, W, WP};
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// One block's facts (a body's child, or the root's child in another story), with places
/// relative to the block (its own place is `[]`).
#[derive(Debug, Default)]
struct BlockFacts {
    controls: Vec<Found>,
    revisions: Vec<Found>,
    frames: Vec<Found>,
    anchors: Vec<String>,
    cell_ends: bool,
}

/// A block of the part and its facts.
#[derive(Debug, Clone)]
struct Block {
    element: Arc<Element>,
    place: Box<[u32]>,
    facts: Arc<BlockFacts>,
}

/// What the lookups read of one story part.
#[derive(Debug, Clone)]
pub struct PartFacts {
    blocks: Vec<Block>,
}

impl PartFacts {
    /// The facts of the part whose root is `root`; `earlier`, the facts of a version
    /// before, whose blocks this version shares are taken as they are.
    #[must_use]
    pub fn read(root: &Element, earlier: Option<&PartFacts>) -> PartFacts {
        let known: HashMap<*const Element, &Arc<BlockFacts>> = earlier
            .map(|e| e.blocks.iter().map(|b| (Arc::as_ptr(&b.element), &b.facts)).collect())
            .unwrap_or_default();
        let mut blocks = Vec::new();
        let mut take = |element: &Arc<Element>, place: Vec<u32>| {
            let facts = known.get(&Arc::as_ptr(element)).map_or_else(
                || Arc::new(BlockFacts::read(element)),
                |found| Arc::clone(found),
            );
            blocks.push(Block {
                element: Arc::clone(element),
                place: place.into_boxed_slice(),
                facts,
            });
        };
        for (k, node) in root.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            let k = index(k);
            if child.name().is(Some(W), "body") {
                for (j, node) in child.children().iter().enumerate() {
                    if let Node::Element(block) = node {
                        take(block, vec![k, index(j)]);
                    }
                }
            } else {
                take(child, vec![k]);
            }
        }
        PartFacts { blocks }
    }

    /// The content controls (`w:sdt`), in document order.
    #[must_use]
    pub fn controls(&self) -> Vec<Found> {
        self.placed(|f| &f.controls)
    }

    /// The revision records, in document order.
    #[must_use]
    pub fn revisions(&self) -> Vec<Found> {
        self.placed(|f| &f.revisions)
    }

    /// The drawing frames (`wp:inline`, `wp:anchor`), in document order.
    #[must_use]
    pub fn frames(&self) -> Vec<Found> {
        self.placed(|f| &f.frames)
    }

    /// The bookmarks the part's hyperlinks point at (fallbacks' too), each once per block.
    pub fn anchors(&self) -> impl Iterator<Item = &str> {
        self.blocks.iter().flat_map(|b| b.facts.anchors.iter().map(String::as_str))
    }

    /// Whether a table cell of the part ends with a paragraph mark that is deleted or moved
    /// away, or is itself deleted: Accept All takes the next cell's text into it.
    #[must_use]
    pub fn cell_ends(&self) -> bool {
        self.blocks.iter().any(|b| b.facts.cell_ends)
    }

    fn placed(&self, list: impl Fn(&BlockFacts) -> &Vec<Found>) -> Vec<Found> {
        let mut out = Vec::new();
        for block in &self.blocks {
            for found in list(&block.facts) {
                let mut path = block.place.to_vec();
                path.extend_from_slice(&found.path);
                out.push(Found {
                    element: Arc::clone(&found.element),
                    path: path.into_boxed_slice(),
                });
            }
        }
        out
    }
}

fn index(k: usize) -> u32 {
    u32::try_from(k).unwrap_or(u32::MAX)
}

impl BlockFacts {
    /// The facts of `block` and what it holds, in one walk.
    fn read(block: &Arc<Element>) -> BlockFacts {
        let mut out = BlockFacts::default();
        out.visit(block, &mut Vec::new(), false);
        out
    }

    fn visit(&mut self, element: &Arc<Element>, path: &mut Vec<u32>, fallback: bool) {
        let name = element.name();
        let fallback = fallback || name.is(Some(MC), "Fallback");
        if name.is(Some(W), "hyperlink") {
            if let Some(a) = element.attribute(Some(W), "anchor").filter(|a| !a.is_empty()) {
                self.anchors.push(a.to_owned());
            }
        }
        if !fallback {
            let found = || Found {
                element: Arc::clone(element),
                path: path.clone().into_boxed_slice(),
            };
            if name.is(Some(W), "sdt") {
                self.controls.push(found());
            } else if is_revision(element) {
                self.revisions.push(found());
            } else if name.is(Some(WP), "inline") || name.is(Some(WP), "anchor") {
                self.frames.push(found());
            } else if name.is(Some(W), "tc") && cell_end_taken(element) {
                self.cell_ends = true;
            }
        }
        for (k, node) in element.children().iter().enumerate() {
            if let Node::Element(child) = node {
                path.push(index(k));
                self.visit(child, path, fallback);
                path.pop();
            }
        }
    }
}

/// Whether Accept All takes away a cell's end: its last paragraph's mark deleted or moved
/// away, or the cell deleted.
fn cell_end_taken(cell: &Element) -> bool {
    let deleted = |e: &Element| e.name().is(Some(W), "del") || e.name().is(Some(W), "moveFrom");
    if cell
        .find(Some(W), "tcPr")
        .is_some_and(|p| p.find(Some(W), "cellDel").is_some())
    {
        return true;
    }
    let last = cell.child_elements().filter(|c| c.name().is(Some(W), "p")).last();
    last.and_then(|p| p.find(Some(W), "pPr"))
        .and_then(|p| p.find(Some(W), "rPr"))
        .is_some_and(|r| r.child_elements().any(|c| deleted(c)))
}

#[cfg(test)]
mod tests {
    use super::PartFacts;
    use crate::docx::walk::live_paths;
    use crate::ids::W;
    use ooxml_base::XmlLimits;

    const DOC: &str = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body><w:sdt><w:sdtContent><w:p><w:ins w:id="1"><w:r><w:t>a</w:t></w:r></w:ins></w:p></w:sdtContent></w:sdt><w:p><w:hyperlink w:anchor="x"/><mc:AlternateContent><mc:Choice/><mc:Fallback><w:sdt/><w:hyperlink w:anchor="y"/></mc:Fallback></mc:AlternateContent></w:p><w:tbl><w:tr><w:tc><w:p><w:pPr><w:rPr><w:del w:id="2"/></w:rPr></w:pPr></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;

    #[test]
    fn one_walk_finds_what_the_walks_did_and_a_later_version_reuses_its_blocks() {
        let tree = ooxml_xml::parse(DOC.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        let facts = PartFacts::read(tree.root(), None);
        let paths = |found: Vec<crate::docx::Found>| found.into_iter().map(|f| f.path.to_vec()).collect::<Vec<_>>();
        let sdts = live_paths(tree.root(), &[], &|e| e.name().is(Some(W), "sdt"));
        assert_eq!(paths(facts.controls()), paths(sdts));
        let revisions = live_paths(tree.root(), &[], &crate::docx::revisions::is_revision);
        assert_eq!(paths(facts.revisions()), paths(revisions));
        let mut anchors: Vec<&str> = facts.anchors().collect();
        anchors.sort_unstable();
        assert_eq!(anchors, ["x", "y"]);
        assert!(facts.cell_ends());
        let again = PartFacts::read(tree.root(), Some(&facts));
        assert!(std::sync::Arc::ptr_eq(&again.blocks[0].facts, &facts.blocks[0].facts));
    }
}
