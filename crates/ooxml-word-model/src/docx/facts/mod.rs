//! Owns: [`PartFacts`], what docx-agent's document-wide lookups read of one story part --
//! its content controls, revision records and drawing frames with their places (live
//! elements, `mc:Fallback` left out, in document order), the bookmarks its hyperlinks point
//! at, its bookmarks' markers (fallbacks' too, as docx-agent's `_bookmark_markers` walks
//! them), and whether a table cell's end is one Accept All takes away -- found in one walk and
//! kept with the part, each top-level block's facts apart: a later version of the part
//! takes every block it shares with the version before (the same subtree, by pointer) from
//! that version's facts and walks only the blocks that changed.
//!
//! Must not: number anything document-wide (`cc:<id>#k` and the like count across parts:
//! [`super::controls`], [`super::revisions`] and [`super::drawings`] number them from these
//! facts), or keep anything a part's tree does not determine.

use crate::docx::revisions::is_revision;
use crate::docx::Found;
use crate::ids::{W, WP};
use crate::view::View;
use ooxml_xml::{Element, Node};
use runs::Runs;
use std::sync::Arc;

mod runs;

/// One block's facts (a body's child, or the root's child in another story), with places
/// relative to the block (its own place is `[]`).
#[derive(Debug, Default)]
struct BlockFacts {
    controls: Vec<Found>,
    revisions: Vec<Found>,
    frames: Vec<Found>,
    anchors: Vec<String>,
    /// `w:bookmarkStart`s: name, id, place.
    starts: Vec<(String, String, Box<[u32]>)>,
    /// `w:bookmarkEnd`s: id, place.
    ends: Vec<(String, Box<[u32]>)>,
    cell_ends: bool,
    /// A `w:del` or `w:cellDel` is in it, fallbacks included.
    deletions: bool,
}

/// What the lookups read of one story part: the root's children's facts (a body left out)
/// and each body's blocks' facts, in runs later versions share ([`Runs`]).
#[derive(Debug, Clone)]
pub struct PartFacts {
    root: Runs<BlockFacts>,
    bodies: Vec<(u32, Runs<BlockFacts>)>,
}

/// A block's place in the part: the root's child `at[0]`, or the body `at[0]`'s child
/// `at[1]`.
#[derive(Clone, Copy)]
struct Place {
    at: [u32; 2],
    len: usize,
}

impl Place {
    /// A place relative to the block, as the part's.
    fn with(self, path: &[u32]) -> Box<[u32]> {
        let mut out = Vec::with_capacity(self.len + path.len());
        out.extend(self.at.iter().take(self.len));
        out.extend_from_slice(path);
        out.into_boxed_slice()
    }
}

/// A root's child and its facts, placed.
fn top<'a>((k, _, facts): (u32, &'a Arc<Element>, &'a Arc<BlockFacts>)) -> (Place, &'a BlockFacts) {
    (Place { at: [k, 0], len: 1 }, facts)
}

fn is_body(element: &Element) -> bool {
    element.name().is(Some(W), "body")
}

impl PartFacts {
    /// The facts of the part whose root is `root`; `earlier`, the facts of another version
    /// of it, whose blocks this version shares are taken as they are (and the runs of them
    /// outside the stretch where the two differ, without visiting them).
    #[must_use]
    pub fn read(root: &Element, earlier: Option<&PartFacts>) -> PartFacts {
        let make = |block: &Arc<Element>| BlockFacts::read(block);
        let others = Runs::read(root.children(), earlier.map(|e| &e.root), &|e| !is_body(e), &make);
        let mut bodies = Vec::new();
        for (k, node) in root.children().iter().enumerate() {
            let Node::Element(body) = node else { continue };
            if is_body(body) {
                let before = earlier.and_then(|e| e.bodies.get(bodies.len())).map(|(_, r)| r);
                bodies.push((index(k), Runs::read(body.children(), before, &|_| true, &make)));
            }
        }
        PartFacts { root: others, bodies }
    }

    /// Each block's place and facts, in document order (the root's children around its
    /// body; a second body, which no valid part has, follows the first).
    fn blocks(&self) -> impl Iterator<Item = (Place, &BlockFacts)> {
        let first = self.bodies.first().map_or(u32::MAX, |(k, _)| *k);
        let before = self.root.iter().take_while(move |(k, _, _)| *k < first).map(top);
        let after = self.root.iter().skip_while(move |(k, _, _)| *k < first).map(top);
        let bodies = self.bodies.iter().flat_map(|(k, runs)| {
            runs.iter()
                .map(move |(j, _, facts)| (Place { at: [*k, j], len: 2 }, &**facts))
        });
        before.chain(bodies).chain(after)
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
        self.blocks().flat_map(|(_, f)| f.anchors.iter().map(String::as_str))
    }

    /// The bookmarks' starts (name, id, place), in document order.
    #[must_use]
    pub fn bookmark_starts(&self) -> Vec<(String, String, Box<[u32]>)> {
        let mut out = Vec::new();
        for (place, facts) in self.blocks() {
            for (name, id, path) in &facts.starts {
                out.push((name.clone(), id.clone(), place.with(path)));
            }
        }
        out
    }

    /// The first bookmark start in document order whose name `matches`: its name, id and
    /// place (one name looked up without copying out every start).
    #[must_use]
    pub fn bookmark_start(&self, matches: &dyn Fn(&str) -> bool) -> Option<(String, String, Box<[u32]>)> {
        self.blocks().find_map(|(place, facts)| {
            facts
                .starts
                .iter()
                .find(|(name, _, _)| matches(name))
                .map(|(name, id, path)| (name.clone(), id.clone(), place.with(path)))
        })
    }

    /// The place of the last end of the bookmark id `id`.
    #[must_use]
    pub fn bookmark_end(&self, id: &str) -> Option<Box<[u32]>> {
        let mut last = None;
        for (place, facts) in self.blocks() {
            if let Some((_, path)) = facts.ends.iter().rev().find(|(end, _)| end == id) {
                last = Some(place.with(path));
            }
        }
        last
    }

    /// The bookmarks' ends (id, place), in document order.
    #[must_use]
    pub fn bookmark_ends(&self) -> Vec<(String, Box<[u32]>)> {
        let mut out = Vec::new();
        for (place, facts) in self.blocks() {
            for (id, path) in &facts.ends {
                out.push((id.clone(), place.with(path)));
            }
        }
        out
    }

    /// Whether a table row of the part kept in the final view has a cell whose end Accept All
    /// takes away ([`crate::view::CellEnds::any`]): the next cell taken into it, or the cell
    /// gone with its text. A deleted cell is no cell end (it is removed, or widens the cell
    /// before it).
    #[must_use]
    pub fn cell_ends(&self) -> bool {
        self.blocks().any(|(_, f)| f.cell_ends)
    }

    /// Whether a `w:del` or `w:cellDel` is anywhere in the part, `mc:Fallback` content
    /// included (unlike [`PartFacts::revisions`]): where not, a walk for deletion records
    /// finds none.
    #[must_use]
    pub fn holds_deletions(&self) -> bool {
        self.blocks().any(|(_, f)| f.deletions)
    }

    fn placed(&self, list: impl Fn(&BlockFacts) -> &Vec<Found>) -> Vec<Found> {
        let mut out = Vec::new();
        for (place, facts) in self.blocks() {
            for found in list(facts) {
                out.push(Found {
                    element: Arc::clone(&found.element),
                    path: place.with(&found.path),
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
        out.visit(block, &mut Vec::new(), Within::default());
        out
    }

    fn visit(&mut self, element: &Arc<Element>, path: &mut Vec<u32>, within: Within) {
        let name = element.name();
        let within = Within {
            fallback: within.fallback,
            row_gone: within.row_gone || (name.is(Some(W), "tr") && View::Final.removes_row(element)),
        };
        let fallback = within.fallback;
        if name.is(Some(W), "del") || name.is(Some(W), "cellDel") {
            self.deletions = true;
        }
        if name.is(Some(W), "hyperlink") {
            if let Some(a) = element.attribute(Some(W), "anchor").filter(|a| !a.is_empty()) {
                self.anchors.push(a.to_owned());
            }
        }
        if name.is(Some(W), "bookmarkStart") {
            let attribute = |local| element.attribute(Some(W), local).unwrap_or("").to_owned();
            self.starts
                .push((attribute("name"), attribute("id"), path.clone().into_boxed_slice()));
        } else if name.is(Some(W), "bookmarkEnd") {
            let id = element.attribute(Some(W), "id").unwrap_or("").to_owned();
            self.ends.push((id, path.clone().into_boxed_slice()));
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
            } else if name.is(Some(W), "tr") && !within.row_gone && crate::view::row_ends(element).any() {
                self.cell_ends = true;
            }
        }
        for (k, node) in element.children().iter().enumerate() {
            if let Node::Element(child) = node {
                let within = Within {
                    fallback: within.fallback || crate::mc::unchosen(element, child),
                    ..within
                };
                path.push(index(k));
                self.visit(child, path, within);
                path.pop();
            }
        }
    }
}

/// Where the walk is: inside a branch of an `mc:AlternateContent` the readers do not read
/// ([`crate::mc::chosen`]), inside a table row whose deletion is pending.
#[derive(Clone, Copy, Default)]
struct Within {
    fallback: bool,
    /// Accept All removes the row whole (Word 16.106, round 60 R137): its cells' ends, and
    /// those of tables nested in it, take nothing in.
    row_gone: bool,
}

#[cfg(test)]
mod tests {
    use super::PartFacts;
    use crate::docx::walk::live_paths;
    use crate::ids::W;
    use ooxml_base::XmlLimits;

    const DOC: &str = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body><w:sdt><w:sdtContent><w:p><w:ins w:id="1"><w:r><w:t>a</w:t></w:r></w:ins></w:p></w:sdtContent></w:sdt><w:p><w:hyperlink w:anchor="x"/><mc:AlternateContent><mc:Choice/><mc:Fallback><w:sdt/><w:hyperlink w:anchor="y"/></mc:Fallback></mc:AlternateContent></w:p><w:tbl><w:tr><w:tc><w:p><w:pPr><w:rPr><w:del w:id="2"/></w:rPr></w:pPr></w:p></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl></w:body></w:document>"#;

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
        let first =
            |f: &PartFacts| std::sync::Arc::clone(f.bodies[0].1.iter().next().unwrap_or_else(|| panic!("a block")).2);
        assert!(std::sync::Arc::ptr_eq(&first(&again), &first(&facts)));
    }

    #[test]
    fn bookmark_markers_are_found_beside_blocks_inside_them_and_in_fallbacks() {
        let doc = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body><w:bookmarkStart w:id="1" w:name="a"/><w:p><w:bookmarkEnd w:id="1"/><w:bookmarkStart w:id="2" w:name="b"/></w:p><w:p><mc:AlternateContent><mc:Fallback><w:bookmarkEnd w:id="2"/></mc:Fallback></mc:AlternateContent></w:p></w:body></w:document>"#;
        let tree = ooxml_xml::parse(doc.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        let facts = PartFacts::read(tree.root(), None);
        let starts: Vec<(String, String, Vec<u32>)> = facts
            .bookmark_starts()
            .into_iter()
            .map(|(n, i, p)| (n, i, p.to_vec()))
            .collect();
        assert_eq!(
            starts,
            [
                ("a".into(), "1".into(), vec![0, 0]),
                ("b".into(), "2".into(), vec![0, 1, 1])
            ]
        );
        let ends: Vec<(String, Vec<u32>)> = facts
            .bookmark_ends()
            .into_iter()
            .map(|(i, p)| (i, p.to_vec()))
            .collect();
        assert_eq!(ends, [("1".into(), vec![0, 1, 0]), ("2".into(), vec![0, 2, 0, 0, 0])]);
    }

    #[test]
    fn deletions_are_found_in_any_block_fallbacks_included() {
        let holds = |body: &str| {
            let doc = format!(
                r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body>{body}</w:body></w:document>"#
            );
            let tree = ooxml_xml::parse(doc.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
            PartFacts::read(tree.root(), None).holds_deletions()
        };
        assert!(!holds(
            r#"<w:p><w:ins w:id="1"><w:r><w:t>a</w:t></w:r></w:ins></w:p><w:p/>"#
        ));
        assert!(holds(
            r#"<w:p/><w:p><w:del w:id="1"><w:r><w:delText>a</w:delText></w:r></w:del></w:p>"#
        ));
        assert!(holds(
            r#"<w:tbl><w:tr><w:tc><w:tcPr><w:cellDel w:id="1"/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#
        ));
        assert!(holds(
            r#"<w:p><mc:AlternateContent><mc:Fallback><w:del w:id="1"/></mc:Fallback></mc:AlternateContent></w:p>"#
        ));
        assert!(holds(r#"<w:p><w:pPr><w:rPr><w:del w:id="2"/></w:rPr></w:pPr></w:p>"#));
    }

    /// A body of one table: `rows`, each a `w:tr`'s inner XML.
    fn cell_ends_of(rows: &[&str]) -> bool {
        let rows: String = rows.iter().map(|r| format!("<w:tr>{r}</w:tr>")).collect();
        let doc = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:tbl>{rows}</w:tbl></w:body></w:document>"#
        );
        let tree = ooxml_xml::parse(doc.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        PartFacts::read(tree.root(), None).cell_ends()
    }

    #[test]
    fn a_pending_row_deletion_takes_no_cell_in() {
        // Word 16.106 (round 60, R137): Accept All removes a deleted row whole, its cells'
        // marks with it -- a nested table's too; a deleted mark in a row kept still counts
        let mark = r#"<w:tc><w:p><w:pPr><w:rPr><w:del w:id="2"/></w:rPr></w:pPr><w:del w:id="3"><w:r><w:delText>GV</w:delText></w:r></w:del></w:p></w:tc>"#;
        let gone = r#"<w:trPr><w:del w:id="1"/></w:trPr>"#;
        let kept = "<w:tc><w:p><w:r><w:t>ID</w:t></w:r></w:p></w:tc>";
        assert!(!cell_ends_of(&[kept, &format!("{gone}{mark}{mark}"), kept]));
        let nested = format!("<w:tc><w:tbl><w:tr>{mark}</w:tr></w:tbl><w:p/></w:tc>");
        assert!(!cell_ends_of(&[&format!("{gone}{nested}")]));
        assert!(cell_ends_of(&[kept, &format!("{mark}{kept}")]));
    }
}
