//! Owns: [`Package`], an OPC package as a persistent value. Each part is a [`PartVersion`]:
//! raw (its archive entry, never inflated or parsed unless read; parsed at most once, the
//! tree then shared by every version), a tree, bytes, or unwritten (read as a tree made from
//! it, saved as it was: a format that adds to a part in memory, ids say, writes it only
//! when it writes the part anyway). A new version shares every part it did not change.
//! Saving copies untouched entries raw and writes the rest in the write contract's form,
//! compressed as Python's zipfile compresses them.
//!
//! Must not: know any format's parts beyond OPC's own (relationships and content types are
//! in their own modules), or change a part in place.

use crate::map::PartMap;
use crate::name::normalize;
use crate::OpcError;
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
    /// Read as a tree made from the part as it was, saved as it was ([`Unwritten`]).
    Unwritten(Unwritten),
}

/// A part read as one tree and saved as another form of it: the part as it was (an
/// archive entry, a tree or bytes), and the tree made from it that every reader is given.
/// Its content, as a key, is the tree's ([`PartVersion::same`]): writing it
/// ([`Package::written`]) changes what a save writes, never what a reader reads.
#[derive(Debug, Clone)]
pub struct Unwritten {
    /// What a save writes: never itself unwritten.
    saved: Arc<PartVersion>,
    tree: Tree,
}

impl Unwritten {
    /// The part as a save writes it.
    #[must_use]
    pub fn saved(&self) -> &PartVersion {
        &self.saved
    }

    /// The tree it is read as.
    #[must_use]
    pub fn tree(&self) -> &Tree {
        &self.tree
    }
}

impl PartVersion {
    /// Whether `self` and `other` are the same content by identity: the same archive entry
    /// (and its shared tree), the same tree allocation, or the same bytes allocation. What
    /// a store asks to decide that two versions agree on a part without reading it.
    /// An unwritten part is its tree: the same as the tree written in its place.
    pub fn same(&self, other: &PartVersion) -> bool {
        match (self, other) {
            (PartVersion::Raw(a), PartVersion::Raw(b)) => a.index == b.index && Arc::ptr_eq(&a.tree, &b.tree),
            (PartVersion::Tree(a), PartVersion::Tree(b)) => a.ptr_eq(b),
            (PartVersion::Bytes(a), PartVersion::Bytes(b)) => Arc::ptr_eq(a, b),
            (PartVersion::Unwritten(a), PartVersion::Unwritten(b)) => a.tree.ptr_eq(&b.tree),
            (PartVersion::Unwritten(a), PartVersion::Tree(b)) | (PartVersion::Tree(b), PartVersion::Unwritten(a)) => {
                a.tree.ptr_eq(b)
            }
            _ => false,
        }
    }

    /// What a save writes of it: itself, or an unwritten part's form as it was.
    #[must_use]
    pub fn saved(&self) -> &PartVersion {
        match self {
            PartVersion::Unwritten(unwritten) => &unwritten.saved,
            other => other,
        }
    }

    /// Its content as a key ([`PartIdentity`]).
    pub fn identity(&self) -> PartIdentity {
        PartIdentity(self.clone())
    }
}

/// A part's content as a key: two are equal where their contents are the same by identity
/// ([`PartVersion::same`]), hashed by the allocation that identity is. A key holds what it
/// names, so no other content takes its address while the key is kept: what is worked out
/// from a part's content (a metafile drawn, say) is kept under it, across the versions
/// that share the part.
#[derive(Debug, Clone)]
pub struct PartIdentity(PartVersion);

impl PartialEq for PartIdentity {
    fn eq(&self, other: &PartIdentity) -> bool {
        self.0.same(&other.0)
    }
}

impl Eq for PartIdentity {}

impl std::hash::Hash for PartIdentity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match &self.0 {
            PartVersion::Raw(raw) => (0u8, raw.index, Arc::as_ptr(&raw.tree) as usize).hash(state),
            // an unwritten part is keyed as its tree ([`PartVersion::same`])
            PartVersion::Tree(tree) | PartVersion::Unwritten(Unwritten { tree, .. }) => {
                (1u8, 0usize, Arc::as_ptr(tree.root()) as usize).hash(state);
            }
            PartVersion::Bytes(bytes) => (2u8, 0usize, bytes.as_ptr() as usize).hash(state),
        }
    }
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
    slots: PartMap<Slot>,
    next_order: u64,
    limits: Limits,
}

impl Package {
    /// Open a package from its bytes within `limits`. No part is inflated or parsed.
    pub fn open(bytes: Arc<[u8]>, limits: &Limits) -> Result<Package, OpcError> {
        let archive = Arc::new(Archive::read(bytes, &limits.zip)?);
        let mut slots = PartMap::default();
        for (index, entry) in archive.entries().iter().enumerate() {
            let name = slot_name(entry);
            let raw = PartVersion::Raw(RawPart {
                index,
                tree: Arc::new(OnceLock::new()),
            });
            let slot = Slot {
                order: index as u64,
                entry: Some(index),
                content: raw,
            };
            let (next, replaced) = slots.insert(&name, slot);
            if replaced.is_some() {
                return Err(OpcError::DuplicatePart(name));
            }
            slots = next;
        }
        let next_order = archive.entries().len() as u64;
        Ok(Package {
            archive,
            slots,
            next_order,
            limits: *limits,
        })
    }

    /// The part names, in archive order (directory entries left out).
    pub fn part_names(&self) -> Vec<String> {
        let mut named: Vec<(u64, &str)> = self
            .slots
            .iter()
            .filter(|(n, _)| !n.ends_with('/'))
            .map(|(n, s)| (s.order, n))
            .collect();
        named.sort_unstable();
        named.into_iter().map(|(_, n)| n.to_owned()).collect()
    }

    /// Whether `other` has the same parts as this package, each the same content by
    /// identity ([`PartVersion::same`]): one version as far as any reader can tell, known
    /// without reading a part.
    pub fn same_parts(&self, other: &Package) -> bool {
        self.slots.iter().count() == other.slots.iter().count()
            && self
                .slots
                .iter()
                .all(|(name, slot)| other.slots.get(name).is_some_and(|o| o.content.same(&slot.content)))
    }

    /// The parts that differ from `before`: each part added or whose content is not the
    /// same, in archive order, then each part removed, in `before`'s. A part is the same by
    /// identity ([`PartVersion::same`]); two trees by structure (equal trees write equal
    /// bytes, and the comparison stops at the nodes both share, so an edit costs its path,
    /// not the part's size), and a stored part whose tree was read and a tree the same way
    /// (an edit's tree shares what it did not change with the stored part's); else by its
    /// bytes -- two archive entries by their stored
    /// bytes, without inflating them; else as each reads -- so that two packages opened
    /// from bytes apart agree on the parts they share.
    ///
    /// Two trees of another structure that would write the same bytes (a text split in two
    /// nodes, say) count as changed: a reader may be told of a part that did not change,
    /// never not told of one that did.
    pub fn changed_parts(&self, before: &Package) -> Vec<String> {
        let mut changed: Vec<String> = self
            .part_names()
            .into_iter()
            .filter(|name| !self.same_content(name, before))
            .collect();
        changed.extend(
            before
                .part_names()
                .into_iter()
                .filter(|name| self.slots.get(name).is_none()),
        );
        changed
    }

    /// Whether the part `name` (normalised) saves the same content here and in `other`.
    fn same_content(&self, name: &str, other: &Package) -> bool {
        let (Some(mine), Some(theirs)) = (self.slots.get(name), other.slots.get(name)) else {
            return false;
        };
        let (mine, theirs) = (mine.content.saved(), theirs.content.saved());
        if mine.same(theirs) {
            return true;
        }
        match (mine, theirs) {
            (PartVersion::Tree(a), PartVersion::Tree(b)) => return a == b,
            (PartVersion::Raw(raw), PartVersion::Tree(tree)) | (PartVersion::Tree(tree), PartVersion::Raw(raw)) => {
                if let Some(Ok(parsed)) = raw.tree.get() {
                    return parsed == tree;
                }
            }
            (PartVersion::Raw(a), PartVersion::Raw(b)) => {
                if let (Ok(x), Ok(y)) = (self.entry(a.index), other.entry(b.index)) {
                    let (m, n) = (x.meta(), y.meta());
                    return m.crc32 == n.crc32
                        && m.size == n.size
                        && m.method == n.method
                        && self.archive.stored_data(x) == other.archive.stored_data(y);
                }
            }
            _ => {}
        }
        matches!((self.saved_bytes(mine), other.saved_bytes(theirs)), (Ok(x), Ok(y)) if x == y)
    }

    /// The bytes a save writes of `content`, one of this package's parts.
    fn saved_bytes(&self, content: &PartVersion) -> Result<Arc<[u8]>, OpcError> {
        Ok(match content.saved() {
            PartVersion::Raw(raw) => self.inflate(raw.index)?.into(),
            PartVersion::Tree(tree) => tree.write().into(),
            PartVersion::Bytes(bytes) => Arc::clone(bytes),
            PartVersion::Unwritten(unwritten) => unwritten.tree.write().into(),
        })
    }

    /// Whether the package has the part `name` (any spelling of it).
    pub fn has_part(&self, name: &str) -> bool {
        self.slots.contains_key(&normalize(name))
    }

    /// The part's version, if the package has it.
    pub fn part(&self, name: &str) -> Option<&PartVersion> {
        self.slots.get(&normalize(name)).map(|s| &s.content)
    }

    /// The part's current bytes: as stored (inflated), as its tree writes (an unwritten
    /// part's tree too: what it is read as), or as put.
    pub fn read(&self, name: &str) -> Result<Option<Arc<[u8]>>, OpcError> {
        Ok(match self.part(name) {
            None => None,
            Some(PartVersion::Raw(raw)) => Some(self.inflate(raw.index)?.into()),
            Some(PartVersion::Tree(tree) | PartVersion::Unwritten(Unwritten { tree, .. })) => Some(tree.write().into()),
            Some(PartVersion::Bytes(bytes)) => Some(Arc::clone(bytes)),
        })
    }

    /// The part's bytes as a save writes them: an unwritten part's as it was, any other
    /// part's as [`Package::read`] reads them.
    ///
    /// # Errors
    /// As [`Package::read`].
    pub fn read_saved(&self, name: &str) -> Result<Option<Arc<[u8]>>, OpcError> {
        match self.part(name) {
            None => Ok(None),
            Some(content) => self.saved_bytes(content).map(Some),
        }
    }

    /// The size of the part's bytes where it is known without reading them: as stored (the
    /// entry's uncompressed size) or as put; `None` for a tree, or no such part.
    pub fn size(&self, name: &str) -> Option<u64> {
        match self.part(name)? {
            PartVersion::Raw(raw) => self.entry(raw.index).ok().map(|e| e.meta().size),
            PartVersion::Bytes(bytes) => u64::try_from(bytes.len()).ok(),
            PartVersion::Tree(_) | PartVersion::Unwritten(_) => None,
        }
    }

    /// The part's tree: parsed once for a raw part (and shared by every version that has
    /// the part untouched), parsed from bytes put in place, or the tree put in place (an
    /// unwritten part's tree).
    pub fn tree(&self, name: &str) -> Result<Option<Tree>, OpcError> {
        let name = normalize(name);
        let Some(slot) = self.slots.get(&name) else {
            return Ok(None);
        };
        self.tree_of(&name, &slot.content).map(Some)
    }

    /// The tree a save writes of the part `name`: an unwritten part's as it was (parsed
    /// once, as [`Package::tree`] parses it), any other part's as [`Package::tree`] reads it.
    ///
    /// # Errors
    /// As [`Package::tree`].
    pub fn saved_tree(&self, name: &str) -> Result<Option<Tree>, OpcError> {
        let name = normalize(name);
        let Some(slot) = self.slots.get(&name) else {
            return Ok(None);
        };
        self.tree_of(&name, slot.content.saved()).map(Some)
    }

    /// `content`'s tree, `name` its part's.
    fn tree_of(&self, name: &str, content: &PartVersion) -> Result<Tree, OpcError> {
        let xml = |e| OpcError::Xml {
            part: name.to_owned(),
            error: e,
        };
        match content {
            PartVersion::Tree(tree) | PartVersion::Unwritten(Unwritten { tree, .. }) => Ok(tree.clone()),
            PartVersion::Bytes(bytes) => parse(bytes, &self.limits.xml).map_err(xml),
            PartVersion::Raw(raw) => raw
                .tree
                .get_or_init(|| {
                    self.inflate(raw.index)
                        .and_then(|b| parse(&b, &self.limits.xml).map_err(xml))
                })
                .clone(),
        }
    }

    /// A version whose part `name` (which must exist) is read as `tree` and saved as it is
    /// now ([`Unwritten`]); a part unwritten already keeps its saved form.
    ///
    /// # Errors
    /// [`OpcError::NoPart`] when the package has no part `name`.
    pub fn with_unwritten(&self, name: &str, tree: Tree) -> Result<Package, OpcError> {
        let key = normalize(name);
        let slot = self.slots.get(&key).ok_or_else(|| OpcError::NoPart(key.clone()))?;
        let saved = match &slot.content {
            PartVersion::Unwritten(unwritten) => Arc::clone(&unwritten.saved),
            other => Arc::new(other.clone()),
        };
        self.replaced(&key, PartVersion::Unwritten(Unwritten { saved, tree }))
    }

    /// The names of the unwritten parts, in archive order.
    pub fn unwritten(&self) -> Vec<String> {
        let mut named: Vec<(u64, &str)> = self
            .slots
            .iter()
            .filter(|(_, s)| matches!(s.content, PartVersion::Unwritten(_)))
            .map(|(n, s)| (s.order, n))
            .collect();
        named.sort_unstable();
        named.into_iter().map(|(_, n)| n.to_owned()).collect()
    }

    /// A version with every unwritten part written: each one its tree, the content every
    /// reader read already (the same by identity), saved as that tree writes.
    #[must_use]
    pub fn written(&self) -> Package {
        let mut next = self.clone();
        for (name, slot) in self.slots.iter() {
            if let PartVersion::Unwritten(unwritten) = &slot.content {
                let slot = Slot {
                    content: PartVersion::Tree(unwritten.tree.clone()),
                    ..slot.clone()
                };
                next.slots = next.slots.insert(name, slot).0;
            }
        }
        next
    }

    /// A version with the part `name` (which must exist) as `tree`.
    pub fn with_tree(&self, name: &str, tree: Tree) -> Result<Package, OpcError> {
        self.replaced(name, PartVersion::Tree(tree))
    }

    /// A version with the part `name` as `bytes`; a new part goes at the end of the
    /// archive.
    pub fn with_bytes(&self, name: &str, bytes: Arc<[u8]>) -> Package {
        self.put(name, PartVersion::Bytes(bytes))
    }

    /// A version with the part `name` as the tree `tree`; a new part goes at the end of
    /// the archive.
    pub fn with_new_tree(&self, name: &str, tree: Tree) -> Package {
        self.put(name, PartVersion::Tree(tree))
    }

    /// `content` in the slot of `name`: an existing part keeps its place and entry, a new
    /// one goes at the end.
    fn put(&self, name: &str, content: PartVersion) -> Package {
        let key = normalize(name);
        let mut next = self.clone();
        let slot = match self.slots.get(&key) {
            Some(s) => Slot { content, ..s.clone() },
            None => {
                next.next_order += 1;
                Slot {
                    order: self.next_order,
                    entry: None,
                    content,
                }
            }
        };
        next.slots = next.slots.insert(&key, slot).0;
        next
    }

    /// A version whose part `name` is as it is in `from`, an earlier or later version of
    /// this package (opened from the same archive): its content, its place and its entry,
    /// or no part where `from` has none. What undo puts back, exactly.
    pub fn restored(&self, name: &str, from: &Package) -> Result<Package, OpcError> {
        if !Arc::ptr_eq(&self.archive, &from.archive) {
            return Err(OpcError::OtherPackage(normalize(name)));
        }
        let key = normalize(name);
        let mut next = self.clone();
        next.next_order = self.next_order.max(from.next_order);
        next.slots = match from.slots.get(&key) {
            Some(slot) => next.slots.insert(&key, slot.clone()).0,
            None => match next.slots.remove(&key) {
                Some((slots, _)) => slots,
                None => return Ok(next),
            },
        };
        Ok(next)
    }

    /// A version without the part `name`.
    pub fn without(&self, name: &str) -> Result<Package, OpcError> {
        let key = normalize(name);
        let mut next = self.clone();
        next.slots = next.slots.remove(&key).ok_or(OpcError::NoPart(key))?.0;
        Ok(next)
    }

    fn replaced(&self, name: &str, content: PartVersion) -> Result<Package, OpcError> {
        let key = normalize(name);
        let slot = self.slots.get(&key).ok_or_else(|| OpcError::NoPart(key.clone()))?;
        let mut next = self.clone();
        next.slots = next
            .slots
            .insert(
                &key,
                Slot {
                    content,
                    ..slot.clone()
                },
            )
            .0;
        Ok(next)
    }

    fn entry(&self, index: usize) -> Result<&Entry, OpcError> {
        self.archive
            .entries()
            .get(index)
            .ok_or_else(|| OpcError::NoPart(format!("entry {index}")))
    }

    fn inflate(&self, index: usize) -> Result<Vec<u8>, OpcError> {
        Ok(self.archive.read_entry(self.entry(index)?)?)
    }

    /// The package's bytes. Untouched entries are copied raw (an unchanged package saves
    /// byte for byte as it was read when its entries lie end to end from the start, as
    /// writers lay them out: bytes between or before entries are not kept), and so is a
    /// touched part whose bytes are the ones read ([`Package::write_part`]); other touched
    /// parts are written as Python's `to_bytes()` writes them: the entry's name, time,
    /// attributes and method kept, a new part's as `ooxml_edit` makes one.
    pub fn save(&self) -> Result<Vec<u8>, OpcError> {
        let mut slots: Vec<(&str, &Slot)> = self.slots.iter().collect();
        slots.sort_unstable_by_key(|(_, s)| s.order);
        let mut w = Writer::new();
        for (name, slot) in slots {
            match (slot.content.saved(), slot.entry) {
                (PartVersion::Raw(raw), _) => w.copy(&self.archive, self.entry(raw.index)?)?,
                (PartVersion::Tree(tree) | PartVersion::Unwritten(Unwritten { tree, .. }), entry) => {
                    self.write_part(&mut w, name, entry, &tree.write())?;
                }
                (PartVersion::Bytes(bytes), entry) => self.write_part(&mut w, name, entry, bytes)?,
            }
        }
        Ok(w.finish(self.archive.comment())?)
    }

    /// A touched part's entry: the one it was read with, copied as it is, where `data` is
    /// the bytes read and that entry's header may be copied (co-authoring request 1: an
    /// entry keeps its MS-FSSHTTPD chunk unless its content changed; ooxml-edit's
    /// `OpcPackage.write`, the same rule); else written anew.
    fn write_part(&self, w: &mut Writer, name: &str, entry: Option<usize>, data: &[u8]) -> Result<(), OpcError> {
        if let Some(index) = entry {
            let read = self.entry(index)?;
            let same_size = usize::try_from(read.meta().size).is_ok_and(|size| size == data.len());
            if same_size && self.archive.header_copyable(read) && self.inflate(index)? == data {
                w.copy(&self.archive, read)?;
                return Ok(());
            }
        }
        w.add(&self.new_entry(name, entry)?, data)?;
        Ok(())
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
                method: if name.ends_with(".xml") || name.ends_with(".rels") {
                    Method::Deflated
                } else {
                    Method::Stored
                },
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
