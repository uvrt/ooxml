//! Owns: what is kept of tables by their content alone, for a document ([`Kept`], on the
//! document's budget): a table's shape as Accept All leaves it (docx-agent's `_RAGGED`,
//! round 57 R124; the least recently used let go first), so that a check after an edit
//! settles again only the tables the edit changed -- and none at all when the document is
//! checked again, or holds the same table twice.
//!
//! Must not: keep anything a table's own content does not determine (a table's accepted
//! copy is read from the table alone: no style, numbering or section changes its shape),
//! or hold its lock while a value is made.

use ooxml_derive::{CacheBudget, Memo, MemoStats, Weigh};
use ooxml_xml::{Element, Node};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

/// A table by its content: the hash of its content, and the table, compared by content
/// where the hashes agree (a shared allocation first).
#[derive(Clone)]
struct Key {
    hash: u64,
    table: Arc<Element>,
}

impl Key {
    fn of(table: &Arc<Element>) -> Key {
        let mut hasher = DefaultHasher::new();
        content_hash(table, &mut hasher);
        Key {
            hash: hasher.finish(),
            table: Arc::clone(table),
        }
    }
}

impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Key) -> bool {
        self.hash == other.hash && (Arc::ptr_eq(&self.table, &other.table) || self.table == other.table)
    }
}

impl Eq for Key {}

/// Feed `element`'s content to `state` as [`Element`]'s equality reads it: its head, then
/// each child by kind, then how many there were.
fn content_hash(element: &Element, state: &mut DefaultHasher) {
    element.head().hash(state);
    let mut count = 0_usize;
    for child in element.children().iter() {
        count += 1;
        match child {
            Node::Element(e) => {
                state.write_u8(0);
                content_hash(e, state);
            }
            Node::Text(text) => {
                state.write_u8(1);
                text.hash(state);
            }
            Node::Comment(text) => {
                state.write_u8(2);
                text.hash(state);
            }
            Node::Pi(pi) => {
                state.write_u8(3);
                pi.hash(state);
            }
            Node::EntityRef(name) => {
                state.write_u8(4);
                name.hash(state);
            }
        }
    }
    state.write_usize(count);
}

/// Values made from tables, kept by the table's content within a budget (the document's
/// account): the least recently used let go first where the budget asks for room.
pub(crate) struct Kept<V> {
    memo: Mutex<Memo<Key, Weighed<V>>>,
    weight: fn(&V) -> usize,
}

/// A value and what it holds, bytes.
#[derive(Clone)]
struct Weighed<V> {
    value: V,
    bytes: usize,
}

impl<V> Weigh for Weighed<V> {
    fn weight(&self) -> usize {
        // The key's table is not counted: the versions holding it share it.
        std::mem::size_of::<Weighed<V>>() + self.bytes
    }
}

impl<V: Clone> std::fmt::Debug for Kept<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kept").field("stats", &self.stats()).finish()
    }
}

impl<V: Clone> Kept<V> {
    /// An empty memo counted on `budget`, each value holding what `weight` says beside
    /// itself.
    pub(crate) fn new(budget: Arc<dyn CacheBudget>, weight: fn(&V) -> usize) -> Kept<V> {
        Kept {
            memo: Mutex::new(Memo::new(budget)),
            weight,
        }
    }

    /// What it holds.
    pub(crate) fn stats(&self) -> MemoStats {
        self.memo.lock().unwrap_or_else(PoisonError::into_inner).stats()
    }

    /// The value kept for a table with `table`'s content, else `make`'s (made without the
    /// lock held, then kept where the budget gives room).
    pub(crate) fn get_or(&self, table: &Arc<Element>, make: impl FnOnce() -> V) -> V {
        let key = Key::of(table);
        if let Some(found) = self.memo.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
            return found.value;
        }
        let value = make();
        let bytes = (self.weight)(&value);
        let kept = Weighed {
            value: value.clone(),
            bytes,
        };
        self.memo
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, kept);
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_derive::Limit;
    use ooxml_xml::{parse, Limits};
    use std::cell::Cell;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn table(text: &str) -> Arc<Element> {
        let xml =
            format!("<w:tbl xmlns:w='{W}'><w:tr><w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc></w:tr></w:tbl>");
        Arc::clone(parse(xml.as_bytes(), &Limits::default()).expect("parses").root())
    }

    fn kept(bytes: usize) -> (Arc<Limit>, Kept<u32>) {
        let budget = Arc::new(Limit::new(bytes));
        let kept = Kept::new(Arc::clone(&budget) as Arc<dyn CacheBudget>, |_| 0);
        (budget, kept)
    }

    #[test]
    fn a_table_with_equal_content_is_made_once() {
        let (budget, kept) = kept(1 << 20);
        let made = Cell::new(0);
        let make = |v: u32| {
            made.set(made.get() + 1);
            v
        };
        assert_eq!(kept.get_or(&table("a"), || make(1)), 1);
        // Parsed again: another allocation, the same content.
        assert_eq!(kept.get_or(&table("a"), || make(2)), 1);
        assert_eq!(kept.get_or(&table("b"), || make(3)), 3);
        assert_eq!(made.get(), 2);
        assert_eq!(budget.used(), kept.stats().bytes, "on the budget");
        assert!(kept.stats().bytes > 0);
        drop(kept);
        assert_eq!(budget.used(), 0, "given back");
    }

    #[test]
    fn the_least_recently_used_goes_first_for_the_budget() {
        let (_, one) = kept(1 << 20);
        one.get_or(&table("a"), || 1);
        let entry = one.stats().bytes;
        let (budget, kept) = kept(2 * entry);
        kept.get_or(&table("a"), || 1);
        kept.get_or(&table("b"), || 2);
        kept.get_or(&table("a"), || 9);
        kept.get_or(&table("c"), || 3);
        assert_eq!(kept.stats().evictions, 1);
        assert!(budget.used() <= 2 * entry);
        assert_eq!(kept.get_or(&table("a"), || 9), 1, "used last, kept");
        assert_eq!(kept.get_or(&table("b"), || 7), 7, "used least, let go");
    }

    #[test]
    fn a_budget_with_no_room_keeps_nothing() {
        let (budget, kept) = kept(8);
        assert_eq!(kept.get_or(&table("a"), || 1), 1);
        assert_eq!(kept.get_or(&table("a"), || 2), 2, "made again");
        assert_eq!(kept.stats().entries, 0);
        assert_eq!(budget.used(), 0);
    }
}
