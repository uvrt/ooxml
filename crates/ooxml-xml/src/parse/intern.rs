//! Owns: sharing while one part is parsed -- equal short strings become one [`Atom`], equal
//! names one [`Name`], equal heads one [`Head`], and equal small subtrees one element
//! (hash-consing, as rowan's node cache does). This is what keeps a tree near its XML's
//! size (design section 4: at most 2.5 times).
//!
//! Strings, heads and elements are shared through direct-mapped caches of fixed size: a fast
//! hash picks the slot, the slot is checked by comparing, and a miss overwrites it. Sharing
//! is best-effort (what is used often stays cached), costs one lookup whatever the input, and
//! needs no table that grows with the part. Names are resolved exactly, once per namespace
//! scope, in tables with the standard library's keyed hasher (so crafted input cannot make
//! them quadratic), behind a cache of the same kind.
//!
//! Must not: outlive the parse (the tables are dropped with it; what they built is shared by
//! the tree), or decide anything a reader could observe other than which allocations are shared.

use crate::name::{Atom, Name, QName};
use crate::node::{Attribute, Element, Head, NamespaceDecl, Node};
use crate::rope::Children;
use std::collections::HashMap;
use std::sync::Arc;

/// Strings at most this long are shared.
const SHARE_MAX: usize = 64;
/// Elements with at most this many children are hash-consed.
const CONS_MAX_CHILDREN: usize = 16;
/// Slots in the caches (powers of two).
const STRING_SLOTS: usize = 1 << 15;
const NAME_SLOTS: usize = 1 << 10;
const HEAD_SLOTS: usize = 1 << 15;
const ELEMENT_SLOTS: usize = 1 << 17;
const TAG_SLOTS: usize = 1 << 12;
/// Start tags at most this long are remembered.
const TAG_MAX: usize = 256;

/// A start tag remembered: its bytes and scope, and what they made.
pub(crate) struct Tag {
    scope: usize,
    bytes: Box<[u8]>,
    /// The head the tag made.
    pub(crate) head: Arc<Head>,
    /// Whether it was `/>`.
    pub(crate) empty: bool,
}

/// What a remembered tag gives back.
pub(crate) struct TagFound {
    /// The head.
    pub(crate) head: Arc<Head>,
    /// Whether it was `/>`.
    pub(crate) empty: bool,
}

/// One word folded into a running hash (FxHash's step).
fn mix(h: u64, word: u64) -> u64 {
    (h.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95)
}

/// A final avalanche (MurmurHash3's), so every bit of the key spreads to the low bits.
fn avalanche(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
}

/// An unkeyed fast hash of a short byte string (for picking a cache slot only).
fn bytes_hash(seed: u64, bytes: &[u8]) -> u64 {
    let (words, tail) = bytes.as_chunks::<8>();
    let mut h = mix(seed, bytes.len() as u64);
    for w in words {
        h = mix(h, u64::from_le_bytes(*w));
    }
    let mut last = [0u8; 8];
    for (d, s) in last.iter_mut().zip(tail) {
        *d = *s;
    }
    avalanche(mix(h, u64::from_le_bytes(last)))
}

fn addr<T: ?Sized>(p: &Arc<T>) -> u64 {
    Arc::as_ptr(p).cast::<u8>() as usize as u64
}

/// A cached name: the scope it was resolved in, whether it is an attribute's, and the name.
type NameSlot = Option<(usize, bool, Name)>;

/// The parse's sharing tables.
pub(crate) struct Interner {
    strings: Vec<Option<Atom>>,
    recent_names: Vec<NameSlot>,
    /// Names per namespace scope and kind (element 0, attribute 1), by the name as written.
    names: Vec<[HashMap<Box<str>, Name>; 2]>,
    heads: Vec<Option<Arc<Head>>>,
    elements: Vec<Option<Arc<Element>>>,
    /// Start tags read before, by their bytes: a tag repeated in one scope makes the same
    /// head, so it is not read again (tags that declare namespaces are not remembered).
    tags: Vec<Option<Tag>>,
}

impl Default for Interner {
    fn default() -> Self {
        Interner {
            strings: vec![None; STRING_SLOTS],
            recent_names: vec![None; NAME_SLOTS],
            names: Vec::new(),
            heads: vec![None; HEAD_SLOTS],
            elements: vec![None; ELEMENT_SLOTS],
            tags: std::iter::repeat_with(|| None).take(TAG_SLOTS).collect(),
        }
    }
}

/// The slot of `hash` in a cache of `slots` (a power of two).
fn slot(hash: u64, slots: usize) -> usize {
    (hash as usize) & (slots - 1)
}

impl Interner {
    /// `s` as an atom, shared with earlier equal ones when short.
    pub(crate) fn atom(&mut self, s: &str) -> Atom {
        if s.len() > SHARE_MAX {
            return Atom::from(s);
        }
        let Some(cached) = self.strings.get_mut(slot(bytes_hash(0, s.as_bytes()), STRING_SLOTS)) else {
            return Atom::from(s);
        };
        match cached {
            Some(a) if **a == *s => Arc::clone(a),
            _ => Arc::clone(cached.insert(Atom::from(s))),
        }
    }

    /// A name as written under namespace scope `scope`, made by `resolve` the first time.
    pub(crate) fn name<E>(
        &mut self,
        scope: usize,
        attribute: bool,
        qualified: &str,
        resolve: impl FnOnce(&mut Self) -> Result<Option<Atom>, E>,
    ) -> Result<Name, E> {
        let at = slot(
            bytes_hash(mix(scope as u64, u64::from(attribute)), qualified.as_bytes()),
            NAME_SLOTS,
        );
        if let Some(Some((s, a, n))) = self.recent_names.get(at) {
            if *s == scope && *a == attribute && n.qualified() == qualified {
                return Ok(Arc::clone(n));
            }
        }
        let kind = usize::from(attribute);
        let known = self
            .names
            .get(scope)
            .and_then(|t| t.get(kind))
            .and_then(|t| t.get(qualified))
            .cloned();
        let name = match known {
            Some(n) => n,
            None => {
                let namespace = resolve(self)?;
                let name = Arc::new(QName::from_parts(self.atom(qualified), namespace));
                if self.names.len() <= scope {
                    self.names.resize_with(scope + 1, Default::default);
                }
                if let Some(t) = self.names.get_mut(scope).and_then(|t| t.get_mut(kind)) {
                    t.insert(qualified.into(), Arc::clone(&name));
                }
                name
            }
        };
        if let Some(cached) = self.recent_names.get_mut(at) {
            *cached = Some((scope, attribute, Arc::clone(&name)));
        }
        Ok(name)
    }

    /// The head a start tag of these bytes made before in scope `scope`.
    pub(crate) fn tag(&self, scope: usize, bytes: &[u8]) -> Option<TagFound> {
        if bytes.len() > TAG_MAX {
            return None;
        }
        match self.tags.get(slot(bytes_hash(scope as u64, bytes), TAG_SLOTS))? {
            Some(t) if t.scope == scope && *t.bytes == *bytes => Some(TagFound {
                head: Arc::clone(&t.head),
                empty: t.empty,
            }),
            _ => None,
        }
    }

    /// Remember what a start tag of these bytes made in scope `scope`.
    pub(crate) fn remember_tag(&mut self, scope: usize, bytes: &[u8], head: &Arc<Head>, empty: bool) {
        if bytes.len() > TAG_MAX {
            return;
        }
        if let Some(cached) = self.tags.get_mut(slot(bytes_hash(scope as u64, bytes), TAG_SLOTS)) {
            *cached = Some(Tag {
                scope,
                bytes: bytes.into(),
                head: Arc::clone(head),
                empty,
            });
        }
    }

    /// The head of `name`, `namespaces` and `attributes`, shared with an equal one built
    /// before (built only when there is none).
    pub(crate) fn head(&mut self, name: Name, namespaces: &[NamespaceDecl], attributes: &[Attribute]) -> Arc<Head> {
        let mut h = mix(0x9e37_79b9_7f4a_7c15, addr(&name));
        for ns in namespaces {
            h = mix(mix(h, ns.prefix.as_ref().map_or(1, addr)), addr(&ns.uri));
        }
        for a in attributes {
            h = mix(mix(h, addr(&a.name)), addr(&a.value));
        }
        let make = || {
            Arc::new(Head {
                name: name.clone(),
                namespaces: namespaces.into(),
                attributes: attributes.into(),
            })
        };
        let Some(cached) = self.heads.get_mut(slot(avalanche(h), HEAD_SLOTS)) else {
            return make();
        };
        match cached {
            Some(f) if f.name == name && *f.namespaces == *namespaces && *f.attributes == *attributes => Arc::clone(f),
            _ => Arc::clone(cached.insert(make())),
        }
    }

    /// An element of `head` and the children `kids[start..]` (taken from `kids`), or an
    /// equal small one built before.
    pub(crate) fn element(&mut self, head: Arc<Head>, kids: &mut Vec<Node>, start: usize) -> Arc<Element> {
        let children = kids.get(start..).unwrap_or_default();
        let Some(key) = cons_key(&head, children) else {
            return Arc::new(Element::from_head(head, Children::from_drain(kids, start)));
        };
        let Some(cached) = self.elements.get_mut(slot(key, ELEMENT_SLOTS)) else {
            return Arc::new(Element::from_head(head, Children::from_drain(kids, start)));
        };
        match cached {
            Some(f) if Arc::ptr_eq(f.head(), &head) && same_children(f.children(), children) => {
                let shared = Arc::clone(f);
                kids.truncate(start);
                shared
            }
            _ => Arc::clone(cached.insert(Arc::new(Element::from_head(head, Children::from_drain(kids, start))))),
        }
    }
}

/// The hash-consing key of a small element whose children are elements and shared text.
fn cons_key(head: &Arc<Head>, children: &[Node]) -> Option<u64> {
    if children.len() > CONS_MAX_CHILDREN {
        return None;
    }
    let mut h = mix(0x2545_f491_4f6c_dd1d, addr(head));
    for c in children {
        h = match c {
            Node::Element(e) => mix(h, addr(e)),
            Node::Text(t) if t.len() <= SHARE_MAX => mix(mix(h, 3), addr(t)),
            _ => return None,
        };
    }
    Some(avalanche(h))
}

fn same_children(have: &Children, want: &[Node]) -> bool {
    have.as_slice().is_some_and(|have| {
        have.len() == want.len()
            && have.iter().zip(want).all(|(a, b)| match (a, b) {
                (Node::Element(x), Node::Element(y)) => Arc::ptr_eq(x, y),
                (Node::Text(x), Node::Text(y)) => Arc::ptr_eq(x, y),
                _ => false,
            })
    })
}
