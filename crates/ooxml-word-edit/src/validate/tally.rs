//! Owns: [`Tally`], a version's validation facts kept in place (design section 7): every
//! XML part's blocks' memoised facts in a persistent tree whose aggregate holds what is
//! order-dependent or a maximum (field balance, the largest annotation id, drawings to
//! stamp, the container's child order and section breaks), and the whole-document counts
//! ([`Counts`]) -- all made from the previous version's tally and the blocks that differ,
//! so a version costs its changed blocks, not the document.
//!
//! Must not: read a block's XML that did not change (its facts are memoised by content),
//! or decide a rule (the kept rules read the tally).

use super::counts::{Counts, Role};
use super::facts::ElementFacts;
use super::order::Orders;
use super::Facts;
use ooxml_derive::{Measured, Monoid, MonoidTree};
use ooxml_word_model::ids::W;
use ooxml_xml::{Element, Node};
use std::collections::BTreeMap;
use std::sync::Arc;

/// How a part's blocks are held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holder {
    /// The main part's body, at this place among the root's children.
    Body(usize),
    /// A story part's root.
    Root,
    /// Any other part, or a main part without a body: the root is one block.
    Whole,
}

/// One child of a part's block container, with its facts.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub(crate) facts: Arc<ElementFacts>,
    /// Its rank in its container's child order, and its name as the order table names it
    /// (an element of a container the table orders).
    order: Option<(usize, Arc<str>)>,
    element: bool,
    sect: bool,
}

/// What a stretch of items holds.
#[derive(Debug, Clone, Default)]
pub(crate) struct Sum {
    pub(crate) field_net: i64,
    pub(crate) field_low: i64,
    pub(crate) annotation_max: Option<i64>,
    pub(crate) unstamped: usize,
    first: Option<(usize, Arc<str>)>,
    last_rank: Option<usize>,
    /// The first element out of order, by name.
    pub(crate) violation: Option<Arc<str>>,
    pub(crate) sects: usize,
    pub(crate) last_is_sect: Option<bool>,
    pub(crate) moves: usize,
}

impl Monoid for Sum {
    fn empty() -> Self {
        Sum::default()
    }

    fn combine(&self, b: &Self) -> Self {
        let boundary = match (self.last_rank, &b.first) {
            (Some(last), Some((first, name))) if *first < last => Some(Arc::clone(name)),
            _ => None,
        };
        Sum {
            field_net: self.field_net + b.field_net,
            field_low: self.field_low.min(self.field_net + b.field_low),
            annotation_max: match (self.annotation_max, b.annotation_max) {
                (Some(x), Some(y)) => Some(x.max(y)),
                (x, y) => x.or(y),
            },
            unstamped: self.unstamped + b.unstamped,
            first: self.first.clone().or_else(|| b.first.clone()),
            last_rank: b.last_rank.or(self.last_rank),
            violation: self.violation.clone().or(boundary).or_else(|| b.violation.clone()),
            sects: self.sects + b.sects,
            last_is_sect: b.last_is_sect.or(self.last_is_sect),
            moves: self.moves + b.moves,
        }
    }
}

impl Sum {
    /// A block's facts as an aggregate (no order of its own).
    pub(crate) fn of_facts(facts: &ElementFacts) -> Sum {
        Sum {
            field_net: facts.field_net,
            field_low: facts.field_low,
            annotation_max: facts.annotation_max,
            unstamped: facts.unstamped_drawings,
            moves: facts.moves.len(),
            ..Sum::default()
        }
    }
}

impl Measured for Item {
    type Summary = Sum;

    fn measure(&self) -> Sum {
        let mut sum = Sum::of_facts(&self.facts);
        if let Some((rank, name)) = &self.order {
            sum.first = Some((*rank, Arc::clone(name)));
            sum.last_rank = Some(*rank);
        }
        if self.element {
            sum.sects = usize::from(self.sect);
            sum.last_is_sect = Some(self.sect);
        }
        sum
    }
}

/// One XML part's facts.
#[derive(Debug, Clone)]
pub(crate) struct PartTally {
    pub(crate) root: Arc<Element>,
    pub(crate) role: Role,
    pub(crate) holder: Holder,
    pub(crate) items: MonoidTree<Item>,
    /// The root's own facts (its head, order, the body's head).
    pub(crate) rest: Arc<ElementFacts>,
    /// The container's name as the order table names it.
    pub(crate) container: Option<Arc<str>>,
}

/// A version's tally.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tally {
    pub(crate) parts: BTreeMap<Arc<str>, Arc<PartTally>>,
    /// XML parts that could not be read, with why.
    pub(crate) unreadable: Vec<(Arc<str>, String)>,
    pub(crate) counts: Counts,
    pub(crate) main: Arc<str>,
    /// The main part, then the story parts, in relationship order.
    pub(crate) stories: Arc<[Arc<str>]>,
}

/// How to read a part, from its role.
fn holder_of(root: &Element, role: &Role) -> Holder {
    if role.main {
        return root
            .children()
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "body")))
            .map_or(Holder::Whole, Holder::Body);
    }
    if role.story {
        Holder::Root
    } else {
        Holder::Whole
    }
}

/// The element holding a part's blocks.
fn container_of(root: &Arc<Element>, holder: Holder) -> Option<&Arc<Element>> {
    match holder {
        Holder::Body(k) => root.children().get(k)?.as_element(),
        Holder::Root => Some(root),
        Holder::Whole => None,
    }
}

impl PartTally {
    /// The aggregate of its items, then its root's own facts (the order docx-agent's
    /// check reads them in).
    pub(crate) fn sum(&self) -> Sum {
        self.items.summary().combine(&Sum::of_facts(&self.rest))
    }

    /// Every block's facts, then the root's own, in order (rules read a part whole only
    /// where it is rare: moves).
    pub(crate) fn all(&self) -> impl Iterator<Item = &ElementFacts> {
        self.items.iter().map(|i| &*i.facts).chain(std::iter::once(&*self.rest))
    }
}

/// Reads parts into tallies, with the document's memo.
pub(crate) struct Reader<'a> {
    pub(crate) facts: &'a Facts,
    pub(crate) orders: &'a Orders,
}

impl Reader<'_> {
    fn item(&self, node: &Node, ranks: Option<&std::collections::HashMap<String, usize>>) -> Item {
        match node {
            Node::Element(e) => Item {
                facts: self.facts.element(e),
                order: ranks.map(|r| {
                    let rank = self.orders.rank(r, e).unwrap_or(r.len() + 1);
                    (rank, Arc::from(self.orders.prefixed(e)))
                }),
                element: true,
                sect: e.name().is(Some(W), "sectPr"),
            },
            _ => Item {
                facts: Arc::clone(&self.facts.empty),
                order: None,
                element: false,
                sect: false,
            },
        }
    }

    fn rest(&self, root: &Element, holder: Holder, main: bool) -> Arc<ElementFacts> {
        Arc::new(match holder {
            Holder::Body(k) => {
                let body = root.children().get(k).and_then(Node::as_element);
                ElementFacts::of_rest(root, body.map(|b| &**b), self.orders, false)
            }
            Holder::Root => ElementFacts::of_rest(root, None, self.orders, false),
            Holder::Whole if main => ElementFacts::of_rest(root, None, self.orders, true),
            Holder::Whole => ElementFacts::default(),
        })
    }

    /// The order table's name for a container and its children's ranks.
    fn ranks(
        &self,
        container: Option<&Arc<Element>>,
    ) -> (Option<Arc<str>>, Option<&std::collections::HashMap<String, usize>>) {
        let Some(container) = container else {
            return (None, None);
        };
        let name = self.orders.prefixed(container);
        let ranks = self.orders.ranks(&name);
        (ranks.map(|_| Arc::from(name)), ranks)
    }

    /// A part read whole, its facts added to `counts`.
    pub(crate) fn new_part(&self, name: &Arc<str>, root: &Arc<Element>, role: Role, counts: &mut Counts) -> PartTally {
        let holder = holder_of(root, &role);
        let container = container_of(root, holder);
        let (container_name, ranks) = self.ranks(container);
        let items: MonoidTree<Item> = match container {
            Some(c) => c.children().iter().map(|n| self.item(n, ranks)).collect(),
            None => std::iter::once(self.item(&Node::Element(Arc::clone(root)), None)).collect(),
        };
        let rest = self.rest(root, holder, role.main);
        for item in items.iter() {
            counts.apply(name, &role, &item.facts, 1);
        }
        counts.apply(name, &role, &rest, 1);
        PartTally {
            root: Arc::clone(root),
            role,
            holder,
            items,
            rest,
            container: container_name,
        }
    }

    /// `old`'s facts taken out of `counts`.
    pub(crate) fn drop_part(name: &Arc<str>, old: &PartTally, counts: &mut Counts) {
        for f in old.all() {
            counts.apply(name, &old.role, f, -1);
        }
    }

    /// The tally of `root`, a later version of the part `old` tallies, with the same role:
    /// only the container children it does not share are read; `counts` kept.
    pub(crate) fn after(
        &self,
        name: &Arc<str>,
        old: &PartTally,
        root: &Arc<Element>,
        counts: &mut Counts,
    ) -> PartTally {
        let holder = holder_of(root, &old.role);
        let (old_container, new_container) = (container_of(&old.root, old.holder), container_of(root, holder));
        let (Some(was), Some(now)) = (old_container, new_container) else {
            Self::drop_part(name, old, counts);
            return self.new_part(name, root, old.role.clone(), counts);
        };
        if holder != old.holder || was.name() != now.name() || old.items.len() != was.children().len() {
            Self::drop_part(name, old, counts);
            return self.new_part(name, root, old.role.clone(), counts);
        }
        let mut items = old.items.clone();
        let (_, ranks) = self.ranks(Some(now));
        for (start, old_end, new_end) in was.children().changed_spans(now.children()) {
            for k in start..old_end {
                if let Some(item) = old.items.get(k) {
                    counts.apply(name, &old.role, &item.facts, -1);
                }
            }
            let fresh: Vec<Item> = now
                .children()
                .iter()
                .skip(start)
                .take(new_end - start)
                .map(|n| self.item(n, ranks))
                .collect();
            for item in &fresh {
                counts.apply(name, &old.role, &item.facts, 1);
            }
            let common = (old_end - start).min(fresh.len());
            for (k, item) in fresh.iter().take(common).enumerate() {
                let _ = items.set(start + k, item.clone());
            }
            for _ in start + common..old_end {
                let _ = items.remove(start + common);
            }
            for (k, item) in fresh.into_iter().enumerate().skip(common) {
                let _ = items.insert(start + k, item);
            }
        }
        let rest = if root.head() == old.root.head() && holder == Holder::Root {
            Arc::clone(&old.rest)
        } else {
            let rest = self.rest(root, holder, old.role.main);
            counts.apply(name, &old.role, &old.rest, -1);
            counts.apply(name, &old.role, &rest, 1);
            rest
        };
        PartTally {
            root: Arc::clone(root),
            role: old.role.clone(),
            holder,
            items,
            rest,
            container: old.container.clone(),
        }
    }
}

impl Tally {
    /// The tally of `doc`, made from `previous` (an earlier version's of the same
    /// document) by reading only what differs: parts whose tree is the very same are
    /// kept, a changed part's container children that are the same allocations keep
    /// their facts.
    pub(crate) fn of(doc: &crate::doc::WordDoc, previous: Option<&Tally>) -> crate::Result<Tally> {
        let docx = doc.docx()?;
        let package = doc.package();
        let styles = ooxml_word_model::parts::related(package, &docx.main, REL_STYLES)
            .ok()
            .flatten();
        let stories: Arc<[Arc<str>]> = docx.parts.iter().map(|p| Arc::from(p.part.as_str())).collect();
        let shared = doc.shared();
        let reader = Reader {
            facts: &shared.facts,
            orders: shared.facts.orders(),
        };
        let mut tally = match previous {
            Some(p) => Tally {
                parts: p.parts.clone(),
                unreadable: Vec::new(),
                counts: p.counts.clone(),
                main: Arc::from(docx.main.as_str()),
                stories: Arc::clone(&stories),
            },
            None => Tally {
                main: Arc::from(docx.main.as_str()),
                stories: Arc::clone(&stories),
                ..Tally::default()
            },
        };
        let mut seen: Vec<Arc<str>> = Vec::new();
        for name in package.part_names() {
            if !(name.ends_with(".xml") || name.ends_with(".rels")) {
                continue;
            }
            let name: Arc<str> = Arc::from(name.as_str());
            let tree = match package.tree(&name) {
                Ok(Some(tree)) => tree,
                Ok(None) => continue,
                Err(error) => {
                    tally.unreadable.push((Arc::clone(&name), error.to_string()));
                    continue;
                }
            };
            seen.push(Arc::clone(&name));
            let root = tree.root();
            let local = (root.name().namespace() == Some(W)).then(|| root.name().local());
            let role = Role::of(
                &name,
                *name == *docx.main,
                stories.contains(&name),
                styles.as_deref() == Some(&*name),
                local,
            );
            let made = match tally.parts.get(&name) {
                Some(old) if Arc::ptr_eq(&old.root, root) && old.role == role => continue,
                Some(old) if old.role == role => reader.after(&name, old, root, &mut tally.counts),
                Some(old) => {
                    Reader::drop_part(&name, old, &mut tally.counts);
                    reader.new_part(&name, root, role, &mut tally.counts)
                }
                None => reader.new_part(&name, root, role, &mut tally.counts),
            };
            tally.parts.insert(name, Arc::new(made));
        }
        let gone: Vec<Arc<str>> = tally.parts.keys().filter(|k| !seen.contains(k)).cloned().collect();
        for name in gone {
            if let Some(old) = tally.parts.remove(&name) {
                Reader::drop_part(&name, &old, &mut tally.counts);
            }
        }
        Ok(tally)
    }
}

const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
