//! Owns: the block tier of one document's projections ([`Projections`], on the document's
//! memory budget, as [`crate::docx::block_facts`] is): each top-level body element's reading
//! ([`BodyElement`]) kept by the element, the view, the story and what the reading read
//! beside the element -- the theme's colours and the fields open at it -- across the versions
//! that share the element; the latest projection of each view, whose parts the next version
//! takes where they are the same ([`Latest`]); and the parts the blocks resolve against
//! ([`Context`]), kept while they are the same.
//!
//! Memo validity, not block identity: an entry is found by the element's address and taken
//! only where the element it holds (weakly) is still the very element asked about -- upgraded
//! and compared by pointer, so an address a freed element left to a new one is never taken
//! for it. A block's identity comes from its story's index, never from a pointer (ADR-008):
//! an entry is kept only where every block it holds is keyed by a durable key of the story,
//! a key its content carries wherever it stands.
//!
//! Must not: key by a path, an id or a digest, keep an entry whose element no version holds
//! past a sweep, or read a part (the projection does).

use crate::document::{Context, Notes};
use crate::ids::{StoryIx, VersionRef};
use crate::model::{Block, Section, Story};
use crate::read::fields::OpenState;
use crate::read::BodyElement;
use crate::theme::ThemeColours;
use crate::value::Text;
use crate::view::View;
use ooxml_derive::{AddressHash, CacheBudget, Memo, MemoStats, Weigh};
use ooxml_xml::{Element, Tree};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// What a body element's reading is kept under beside its element.
pub(crate) struct ElementOf<'a> {
    pub(crate) view: View,
    pub(crate) story: StoryIx,
    pub(crate) colours: &'a Arc<ThemeColours>,
    pub(crate) open: &'a [OpenState],
}

/// One body element's reading and what it was read with, its element held weakly.
#[derive(Clone)]
struct KeptElement {
    element: Weak<Element>,
    colours: Arc<ThemeColours>,
    open: Arc<[OpenState]>,
    read: Arc<BodyElement>,
}

impl KeptElement {
    /// Whether it is `element`'s reading with `of`: the very element (held, and the one asked
    /// about), the same colours and the same fields open.
    fn of(&self, element: &Arc<Element>, of: &ElementOf<'_>) -> bool {
        self.element.upgrade().is_some_and(|kept| Arc::ptr_eq(&kept, element))
            && (Arc::ptr_eq(&self.colours, of.colours) || *self.colours == **of.colours)
            && *self.open == *of.open
    }

    fn alive(&self) -> bool {
        self.element.strong_count() > 0
    }
}

impl Weigh for KeptElement {
    fn weight(&self) -> usize {
        256 + self.read.blocks.len() * 1024
    }
}

/// An element's reading: by the element's address (taken only where the element it holds is
/// the one asked about), the view and the story.
type ElementKey = (usize, View, StoryIx);

/// Each child of a body's reading by the child's index (`None`: not kept).
pub(crate) type ChildReadings = Arc<[Option<Arc<BodyElement>>]>;

/// The body as one projection read it.
#[derive(Debug, Clone)]
pub(crate) struct BodyRead {
    pub(crate) blocks: Story,
    pub(crate) cached: Arc<[Text]>,
    pub(crate) sections: Arc<[Section]>,
    pub(crate) warnings: Arc<[Text]>,
    /// The body element read, where there is one, and each of its children's reading by the
    /// child's index, where it was read with no field open and closed none (what a later
    /// version takes for the very child it shares, wherever it moved, in one pass over the
    /// two child lists' differences).
    pub(crate) children: Option<(Arc<Element>, ChildReadings)>,
}

/// A part as one projection read it.
#[derive(Debug, Clone)]
pub(crate) enum PartRead {
    /// The main part's body.
    Body(BodyRead),
    /// A header's or a footer's blocks, and the fields it draws as cached.
    Story(Story, Arc<[Text]>),
    /// A notes part's notes of a kind (`footnote`, `endnote`).
    Notes(&'static str, Notes),
}

/// A part read, and the tree and colours it was read from.
#[derive(Debug, Clone)]
pub(crate) struct KeptPart {
    root: Weak<Element>,
    colours: Arc<ThemeColours>,
    pub(crate) read: PartRead,
}

impl KeptPart {
    /// `read`, read from `tree` with `colours`.
    pub(crate) fn new(tree: &Tree, colours: &Arc<ThemeColours>, read: PartRead) -> KeptPart {
        KeptPart {
            root: Arc::downgrade(tree.root()),
            colours: Arc::clone(colours),
            read,
        }
    }

    /// Whether it was read with `colours`.
    pub(crate) fn read_with(&self, colours: &Arc<ThemeColours>) -> bool {
        Arc::ptr_eq(&self.colours, colours) || *self.colours == **colours
    }

    /// Whether it was read from `tree` (the very tree) with `colours`.
    pub(crate) fn of(&self, tree: &Tree, colours: &Arc<ThemeColours>) -> bool {
        self.root.upgrade().is_some_and(|root| Arc::ptr_eq(&root, tree.root()))
            && (Arc::ptr_eq(&self.colours, colours) || *self.colours == **colours)
    }
}

/// The latest projection of one view: the version it was made for, where its delta named
/// it, and its parts as read, by part name.
#[derive(Debug, Default)]
pub(crate) struct Latest {
    pub(crate) version: Option<VersionRef>,
    pub(crate) parts: HashMap<String, KeptPart>,
}

/// The parts the blocks resolve against as last read: the theme's, settings', styles' and
/// numbering's roots, and what was read of them.
#[derive(Debug, Clone)]
struct KeptContext {
    roots: [Option<Arc<Element>>; 4],
    context: Context,
}

/// One document's projections kept across its versions.
pub(crate) struct Projections {
    elements: Mutex<(Memo<ElementKey, KeptElement, AddressHash>, usize)>,
    latest: Mutex<HashMap<View, Arc<Latest>>>,
    context: Mutex<Option<KeptContext>>,
}

impl std::fmt::Debug for Projections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Projections").field("elements", &self.stats()).finish()
    }
}

/// Whether `a` and `b` are the same part's root: the very element, or equal content.
fn same_root(a: Option<&Arc<Element>>, b: Option<&Arc<Element>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || **a == **b,
        (None, None) => true,
        _ => false,
    }
}

impl Projections {
    /// None kept; the elements' readings counted on `budget` (the document's account).
    pub(crate) fn new(budget: Arc<dyn CacheBudget>) -> Projections {
        Projections {
            elements: Mutex::new((Memo::with_hasher(budget), 0)),
            latest: Mutex::new(HashMap::new()),
            context: Mutex::new(None),
        }
    }

    /// What the elements' memo holds.
    pub(crate) fn stats(&self) -> MemoStats {
        self.elements.lock().unwrap_or_else(PoisonError::into_inner).0.stats()
    }

    /// `element`'s reading with `of`, where one is kept.
    pub(crate) fn element(&self, element: &Arc<Element>, of: &ElementOf<'_>) -> Option<Arc<BodyElement>> {
        let key: ElementKey = (Arc::as_ptr(element) as usize, of.view, of.story);
        let mut memo = self.elements.lock().unwrap_or_else(PoisonError::into_inner);
        memo.0
            .touch(&key)
            .filter(|kept| kept.of(element, of))
            .map(|kept| Arc::clone(&kept.read))
    }

    /// Keep `read` as `element`'s reading with `of`: only where every block it holds is keyed
    /// by a durable key of `of.story` (a key that names a place -- a table without rows, a
    /// path -- would name another place where the element is taken again).
    pub(crate) fn keep_element(&self, element: &Arc<Element>, of: &ElementOf<'_>, read: Arc<BodyElement>) {
        if !read.blocks.iter().all(|b| durable(b, of.story)) {
            return;
        }
        let key: ElementKey = (Arc::as_ptr(element) as usize, of.view, of.story);
        let kept = KeptElement {
            element: Arc::downgrade(element),
            colours: Arc::clone(of.colours),
            open: Arc::from(of.open),
            read,
        };
        let mut memo = self.elements.lock().unwrap_or_else(PoisonError::into_inner);
        let (kept_now, held) = &mut *memo;
        kept_now.insert(key, kept);
        // Readings of elements no version holds (each holds its blocks): swept once the memo
        // has grown by a sixteenth since, so what they hold stays a small part of what the
        // versions in use hold, at an amortised cost of a few entries per reading kept.
        if kept_now.stats().entries > *held + *held / 16 + 16 {
            kept_now.sweep(KeptElement::alive);
            *held = kept_now.stats().entries;
        }
    }

    /// The latest projection of `view`.
    pub(crate) fn latest(&self, view: View) -> Option<Arc<Latest>> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&view)
            .cloned()
    }

    /// Keep `latest` as the latest projection of `view`.
    pub(crate) fn keep_latest(&self, view: View, latest: Latest) {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(view, Arc::new(latest));
    }

    /// What blocks resolve against, read from the theme, settings, styles and numbering
    /// `trees` (in that order): each piece as last read where what it is read from is the
    /// same (the very tree or equal content), so a version whose formatting parts are the
    /// same resolves against the very same context.
    pub(crate) fn context(&self, trees: [Option<&Tree>; 4]) -> Context {
        let roots = trees.map(|t| t.map(|t| Arc::clone(t.root())));
        let mut slot = self.context.lock().unwrap_or_else(PoisonError::into_inner);
        let made = match slot.as_ref() {
            Some(kept)
                if kept
                    .roots
                    .iter()
                    .zip(&roots)
                    .all(|(a, b)| same_root(a.as_ref(), b.as_ref())) =>
            {
                return kept.context.clone();
            }
            Some(kept) => context_after(&kept.roots, &kept.context, trees),
            None => super::read_context_after(None, trees),
        };
        *slot = Some(KeptContext {
            roots,
            context: made.clone(),
        });
        made
    }
}

/// The context `trees` read, taking from `before` (read from `roots`) each piece whose parts
/// are the same (the very tree or equal content).
fn context_after(roots: &[Option<Arc<Element>>; 4], before: &Context, trees: [Option<&Tree>; 4]) -> Context {
    let same = |k: usize| {
        let tree = trees.get(k).copied().flatten();
        same_root(roots.get(k).and_then(Option::as_ref), tree.map(Tree::root))
    };
    super::read_context_after(Some((before, [same(0), same(1), same(2), same(3)])), trees)
}

/// Whether every key in `block` is durable and of `story`.
pub(crate) fn durable(block: &Block, story: StoryIx) -> bool {
    let ok = |key: Option<crate::ids::BlockKey>| key.is_some_and(|k| k.durable() && k.story() == story);
    match block {
        Block::Paragraph(p) => ok(p.key),
        Block::Table(t) => {
            ok(t.key)
                && t.rows
                    .iter()
                    .flat_map(|r| r.iter())
                    .flat_map(|c| c.iter())
                    .all(|b| durable(b, story))
        }
    }
}
