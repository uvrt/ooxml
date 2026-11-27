//! Owns: a Word package as every reader reads it ([`WordPackage`]): each story's
//! paragraphs, rows and drawings with durable ids from the first read (stamp-on-open design,
//! 2026-11-26). A block without a valid and unique id is given one in memory when the
//! package is opened, by docx-agent's stamping rules ([`blocks`]) and draws from a pool of
//! the ids the opened bytes hold, so the ids are a function of the bytes alone; the part is
//! kept unwritten ([`ooxml_opc::Unwritten`]), saved as it was until an edit writes it. And
//! the namespaces stamping declares ([`declared`], [`namespaces`]).
//!
//! Must not: draw from an edit's ledger (an edit stamps what it writes through its sink),
//! write a part's bytes, or fail where a reader would not: a part that does not read is
//! left as it is, for the reader to refuse.

pub mod blocks;
pub mod declared;
pub mod namespaces;

pub use blocks::{prefix_at, unstamped, with_attribute, Stamp, Unstamped};
pub use declared::ensured_head;

use crate::docx::at_path;
use crate::ids::{
    hex_value, story_name, used_long_hex, valid_long_hex, BlockKind, IdPool, PartIds, StoryDelta, StoryTable, W14,
};
use crate::parts::{main_part, story_parts};
use ooxml_opc::{Package, PartVersion};
use ooxml_xml::Element;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::Arc;

/// A version of a Word package whose every story block has a durable id: what a reading
/// ([`crate::docx::Docx`]), the layout model ([`crate::Model`]) and the page engine are
/// made from. Made by opening a package ([`WordPackage::open`]), or by an editor for a
/// version it made from one ([`WordPackage::edited`]). Cloning is cheap.
#[derive(Debug, Clone)]
pub struct WordPackage {
    package: Package,
    /// Each part stamped at open, by the values it was given: a later open of the same
    /// part with the same values takes its tree ([`WordPackage::open_after`]).
    given: Arc<HashMap<String, Arc<[String]>>>,
}

impl std::ops::Deref for WordPackage {
    type Target = Package;

    fn deref(&self) -> &Package {
        &self.package
    }
}

impl WordPackage {
    /// `package` opened: every story part with a block lacking a durable id read as that
    /// part stamped, kept unwritten; and the pool the ids were drawn from (the values the
    /// bytes hold and those given, with each seed's cursor).
    #[must_use]
    pub fn open(package: Package) -> (WordPackage, IdPool) {
        WordPackage::open_after(package, None)
    }

    /// [`WordPackage::open`], a part `before` stamped from the same content and given the
    /// same values taking `before`'s tree (what a reader keeps of the part stays its own).
    #[must_use]
    pub fn open_after(package: Package, before: Option<&WordPackage>) -> (WordPackage, IdPool) {
        let mut stamped = package.clone();
        let mut given = HashMap::new();
        let names = match main_part(&package) {
            Ok(Some(main)) => {
                let mut names = vec![main.clone()];
                names.extend(story_parts(&package, &main).unwrap_or_default());
                names.into_iter().map(|part| (story_name(&main, &part), part)).collect()
            }
            _ => Vec::new(),
        };
        let mut trees = Vec::with_capacity(names.len());
        for (story, part) in names {
            if let Ok(Some(tree)) = package.tree(&part) {
                trees.push((story, part, tree));
            }
        }
        let roots: Vec<&ooxml_xml::Element> = trees.iter().map(|(_, _, t)| &**t.root()).collect();
        let mut pool = IdPool::of(used_long_hex(&roots));
        for (story, part, tree) in &trees {
            let ids = PartIds::new(story, tree.root());
            let Some(found) = blocks::unstamped(story, part, tree.root(), &ids, true) else {
                continue;
            };
            let mut draw = |seed: &str, count: usize| Ok::<_, Infallible>(pool.generate(seed, count));
            let Ok(stamp) = found.stamp(&mut draw);
            let values: Arc<[String]> = stamp.given.into();
            let kept = before.and_then(|b| b.kept(part, &package, &values));
            let tree = kept.unwrap_or_else(|| tree.with_root(stamp.root));
            if let Ok(next) = stamped.with_unwritten(part, tree) {
                stamped = next;
                given.insert(part.clone(), values);
            }
        }
        let package = WordPackage {
            package: stamped,
            given: Arc::new(given),
        };
        (package, pool)
    }

    /// The tree this version stamped `part` as, where `package` has the part as this
    /// version saves it and it was given `values` here too.
    fn kept(&self, part: &str, package: &Package, values: &[String]) -> Option<ooxml_xml::Tree> {
        if self.given.get(part).is_none_or(|given| **given != *values) {
            return None;
        }
        let PartVersion::Unwritten(unwritten) = self.package.part(part)? else {
            return None;
        };
        let theirs = package.part(part)?;
        unwritten.saved().same(theirs.saved()).then(|| unwritten.tree().clone())
    }

    /// A version an editor made from a [`WordPackage`]: its commands stamp every block they
    /// write without an id (word-edit's `Stamped`), so every story block of `package` has
    /// one.
    #[must_use]
    pub fn edited(package: Package) -> WordPackage {
        WordPackage {
            package,
            given: Arc::default(),
        }
    }

    /// The package.
    #[must_use]
    pub fn package(&self) -> &Package {
        &self.package
    }
}

/// A story part another writer put in place (a host's commit of parts, co-authoring's
/// refresh) whose blocks lack ids, stamped in memory: each paragraph or row the part's
/// version before held the same but for stamps (paired as a version's change set pairs
/// them, [`PartIds::delta`]) keeps that block's paraId and textId where no other block
/// holds them (`held`: whether a value is held elsewhere in the package); the rest drawn by
/// `draw`, as at open. `None` when every block of it has its id: the part is used as it is.
///
/// # Errors
/// What `draw` returns.
pub fn received<E>(
    (story, part): (&str, &str),
    before: Option<&Arc<Element>>,
    root: &Arc<Element>,
    held: &dyn Fn(u32) -> bool,
    draw: &mut dyn FnMut(&str, usize) -> Result<Vec<String>, E>,
) -> Result<Option<Stamp>, E> {
    let ids = PartIds::new(story, root);
    let Some(found) = blocks::unstamped(story, part, root, &ids, true) else {
        return Ok(None);
    };
    let inherited = match before {
        Some(old_root) => inherited(story, old_root, &ids, root, &found, held),
        None => HashMap::new(),
    };
    found.stamp_inheriting(&inherited, draw).map(Some)
}

/// Each block of `found` (in the part `new`, rooted at `root`) the version before
/// (`old_root`) held the same but for stamps: its place, and that block's paraId and textId.
fn inherited(
    story: &str,
    old_root: &Arc<Element>,
    new: &PartIds,
    root: &Arc<Element>,
    found: &Unstamped<'_>,
    held: &dyn Fn(u32) -> bool,
) -> HashMap<Box<[u32]>, [String; 2]> {
    let old = PartIds::new(story, old_root);
    let mut stories = StoryTable::new();
    let Some(ix) = stories.intern(story) else {
        return HashMap::new();
    };
    let StoryDelta::Blocks(delta) = old.delta(old_root, new, root, ix) else {
        return HashMap::new();
    };
    let wanted: HashSet<&[u32]> = found.blocks().collect();
    let mut taken: HashSet<u32> = HashSet::new();
    let mut out = HashMap::new();
    for (was, now) in &delta.renamed {
        if !matches!(was.kind(), BlockKind::Paragraph | BlockKind::Row) || was.kind() != now.kind() {
            continue;
        }
        let (Some(from), Some(to)) = (old.place_of(*was), new.place_of(*now)) else {
            continue;
        };
        if !wanted.contains(&*to) {
            continue;
        }
        let Some(element) = at_path(old_root, &from) else {
            continue;
        };
        let value = |local: &str| element.attribute(Some(W14), local).filter(|v| valid_long_hex(v));
        let (Some(para), Some(text)) = (value("paraId"), value("textId")) else {
            continue;
        };
        let Some(number) = hex_value(para) else { continue };
        // a value another block holds now, or one this part already gave, is not taken
        if held(number) || !taken.insert(number) {
            continue;
        }
        out.insert(to, [para.to_owned(), text.to_owned()]);
    }
    out
}
