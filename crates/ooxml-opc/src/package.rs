//! Owns: [`Package`], an OPC package as a persistent value. Each part is a [`PartVersion`]:
//! raw (its archive entry, never inflated or parsed unless read; parsed at most once, the
//! tree then shared by every version), a tree, or bytes. A new version shares every part it
//! did not change. Saving copies untouched entries raw and writes the rest in the write
//! contract's form, compressed as Python's zipfile compresses them.
//!
//! Must not: know any format's parts beyond OPC's own (relationships and content types are
//! in their own modules), or change a part in place.

use crate::name::normalize;
use crate::OpcError;
use imbl::OrdMap;
use ooxml_base::Limits;
use ooxml_xml::{parse, Tree};
use ooxml_zip::{Archive, Entry, Method, NewEntry, Writer};
use std::sync::{Arc, OnceLock};

/// What a part is in one version of a package.
#[derive(Debug, Clone)]
pub enum PartVersion {
    /// As stored in the archive the package was opened from.
    Raw(RawPart),
    /// A tree (touched: written in the write contract's form on save).
    Tree(Tree),
    /// Bytes put in place (touched).
    Bytes(Arc<[u8]>),
}

/// An untouched part: its entry, and its tree once something read it.
#[derive(Debug, Clone)]
pub struct RawPart {
    index: usize,
    tree: Arc<OnceLock<Result<Tree, OpcError>>>,
}

/// A part's slot: where it goes in the archive, the entry it came from, its content.
#[derive(Debug, Clone)]
struct Slot {
    order: u64,
    /// The source entry, for its name as stored and its metadata on save.
    entry: Option<usize>,
    content: PartVersion,
}

/// An OPC package (one version of it). Cloning is cheap; versions share parts.
#[derive(Debug, Clone)]
pub struct Package {
    archive: Arc<Archive>,
    slots: OrdMap<String, Slot>,
    next_order: u64,
    limits: Limits,
}

impl Package {
    /// Open a package from its bytes within `limits`. No part is inflated or parsed.
    pub fn open(bytes: Arc<[u8]>, limits: &Limits) -> Result<Package, OpcError> {
        let archive = Arc::new(Archive::read(bytes, &limits.zip)?);
        let mut slots = OrdMap::new();
        for (index, entry) in archive.entries().iter().enumerate() {
            let name = slot_name(entry);
            let raw = PartVersion::Raw(RawPart { index, tree: Arc::new(OnceLock::new()) });
            let slot = Slot { order: index as u64, entry: Some(index), content: raw };
            if slots.insert(name.clone(), slot).is_some() {
                return Err(OpcError::DuplicatePart(name));
            }
        }
        let next_order = archive.entries().len() as u64;
        Ok(Package { archive, slots, next_order, limits: *limits })
    }

    /// The part names, in archive order (directory entries left out).
    pub fn part_names(&self) -> Vec<String> {
        let mut named: Vec<(u64, &String)> =
            self.slots.iter().filter(|(n, _)| !n.ends_with('/')).map(|(n, s)| (s.order, n)).collect();
        named.sort_unstable();
        named.into_iter().map(|(_, n)| n.clone()).collect()
    }

    /// Whether the package has the part `name` (any spelling of it).
    pub fn has_part(&self, name: &str) -> bool {
        self.slots.contains_key(&normalize(name))
    }

    /// The part's version, if the package has it.
    pub fn part(&self, name: &str) -> Option<&PartVersion> {
        self.slots.get(&normalize(name)).map(|s| &s.content)
    }

    /// The part's current bytes: as stored (inflated), as its tree writes, or as put.
    pub fn read(&self, name: &str) -> Result<Option<Arc<[u8]>>, OpcError> {
        Ok(match self.part(name) {
            None => None,
            Some(PartVersion::Raw(raw)) => Some(self.inflate(raw.index)?.into()),
            Some(PartVersion::Tree(tree)) => Some(tree.write().into()),
            Some(PartVersion::Bytes(bytes)) => Some(Arc::clone(bytes)),
        })
    }

    /// The part's tree: parsed once for a raw part (and shared by every version that has
    /// the part untouched), parsed from bytes put in place, or the tree put in place.
    pub fn tree(&self, name: &str) -> Result<Option<Tree>, OpcError> {
        let name = normalize(name);
        let Some(slot) = self.slots.get(&name) else { return Ok(None) };
        let xml = |e| OpcError::Xml { part: name.clone(), error: e };
        match &slot.content {
            PartVersion::Tree(tree) => Ok(Some(tree.clone())),
            PartVersion::Bytes(bytes) => parse(bytes, &self.limits.xml).map(Some).map_err(xml),
            PartVersion::Raw(raw) => raw
                .tree
                .get_or_init(|| self.inflate(raw.index).and_then(|b| parse(&b, &self.limits.xml).map_err(xml)))
                .clone()
                .map(Some),
        }
    }

    /// A version with the part `name` (which must exist) as `tree`.
    pub fn with_tree(&self, name: &str, tree: Tree) -> Result<Package, OpcError> {
        self.replaced(name, PartVersion::Tree(tree))
    }

    /// A version with the part `name` as `bytes`; a new part goes at the end of the
    /// archive.
    pub fn with_bytes(&self, name: &str, bytes: Arc<[u8]>) -> Package {
        let key = normalize(name);
        let mut next = self.clone();
        let slot = match self.slots.get(&key) {
            Some(s) => Slot { content: PartVersion::Bytes(bytes), ..s.clone() },
            None => {
                next.next_order += 1;
                Slot { order: self.next_order, entry: None, content: PartVersion::Bytes(bytes) }
            }
        };
        next.slots.insert(key, slot);
        next
    }

    /// A version without the part `name`.
    pub fn without(&self, name: &str) -> Result<Package, OpcError> {
        let key = normalize(name);
        let mut next = self.clone();
        next.slots.remove(&key).ok_or(OpcError::NoPart(key))?;
        Ok(next)
    }

    fn replaced(&self, name: &str, content: PartVersion) -> Result<Package, OpcError> {
        let key = normalize(name);
        let slot = self.slots.get(&key).ok_or_else(|| OpcError::NoPart(key.clone()))?;
        let mut next = self.clone();
        next.slots.insert(key, Slot { content, ..slot.clone() });
        Ok(next)
    }

    fn entry(&self, index: usize) -> Result<&Entry, OpcError> {
        self.archive.entries().get(index).ok_or_else(|| OpcError::NoPart(format!("entry {index}")))
    }

    fn inflate(&self, index: usize) -> Result<Vec<u8>, OpcError> {
        Ok(self.archive.read_entry(self.entry(index)?)?)
    }

    /// The package's bytes. Untouched entries are copied raw (an unchanged package saves
    /// byte for byte as it was read); touched parts are written as Python's `to_bytes()`
    /// writes them: the entry's name, time, attributes and method kept, a new part's as
    /// `ooxml_edit` makes one.
    pub fn save(&self) -> Result<Vec<u8>, OpcError> {
        let mut slots: Vec<(&String, &Slot)> = self.slots.iter().collect();
        slots.sort_unstable_by_key(|(_, s)| s.order);
        let mut w = Writer::new();
        for (name, slot) in slots {
            match (&slot.content, slot.entry) {
                (PartVersion::Raw(raw), _) => w.copy(&self.archive, self.entry(raw.index)?)?,
                (PartVersion::Tree(tree), entry) => w.add(&self.new_entry(name, entry)?, &tree.write())?,
                (PartVersion::Bytes(bytes), entry) => w.add(&self.new_entry(name, entry)?, bytes)?,
            }
        }
        Ok(w.finish(self.archive.comment())?)
    }

    /// The header a touched part is written with: its source entry's, or a new part's.
    fn new_entry(&self, name: &str, entry: Option<usize>) -> Result<NewEntry, OpcError> {
        Ok(match entry {
            Some(index) => {
                let meta = self.entry(index)?.meta();
                NewEntry {
                    name: meta.name.clone(),
                    method: meta.method,
                    dos_time: meta.dos_time,
                    dos_date: meta.dos_date,
                    external_attr: meta.external_attr,
                    create_system: meta.version_made_by.to_be_bytes()[0],
                }
            }
            // ooxml_edit's _new_entry: 1980-01-01, Unix, XML deflated and the rest stored
            None => NewEntry {
                name: name.to_owned(),
                method: if name.ends_with(".xml") || name.ends_with(".rels") { Method::Deflated } else { Method::Stored },
                dos_time: 0,
                dos_date: 1 << 5 | 1,
                external_attr: 0,
                create_system: 3,
            },
        })
    }
}

/// The key of an entry's slot: its part name, or a directory's name as stored.
fn slot_name(entry: &Entry) -> String {
    let meta = entry.meta();
    if meta.is_dir() {
        meta.name.clone()
    } else {
        normalize(&meta.name)
    }
}
