//! Owns: what is kept of tables by their content alone, for the process ([`Kept`]): a
//! table's shape as Accept All leaves it (docx-agent's `_RAGGED`, round 57 R124: 512
//! tables, the least recently used dropped first), so that a check after an edit settles
//! again only the tables the edit changed -- and none at all when the document is checked
//! again, reopened, or another document holds the same tables.
//!
//! Must not: keep anything a table's own content does not determine (a table's accepted
//! copy is read from the table alone: no style, numbering or section changes its shape),
//! or hold its lock while a value is made.

use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

/// Tables kept, at most (docx-agent's `_RAGGED_SIZE`).
pub(super) const KEPT: usize = 512;

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

/// Values made from tables, kept by the table's content: at most `size`, the least
/// recently used dropped first.
pub(super) struct Kept<V> {
    entries: Mutex<Entries<V>>,
    size: usize,
}

struct Entries<V> {
    by_table: HashMap<Key, (V, u64)>,
    /// Ticks at each use; an entry keeps the tick of its last.
    clock: u64,
}

impl<V: Clone> Kept<V> {
    /// An empty memo of at most `size` tables.
    pub(super) fn new(size: usize) -> Kept<V> {
        Kept {
            entries: Mutex::new(Entries {
                by_table: HashMap::new(),
                clock: 0,
            }),
            size,
        }
    }

    /// The value kept for a table with `table`'s content, else `make`'s (made without the
    /// lock held, then kept).
    pub(super) fn get_or(&self, table: &Arc<Element>, make: impl FnOnce() -> V) -> V {
        let key = Key::of(table);
        {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            entries.clock += 1;
            let now = entries.clock;
            if let Some((value, used)) = entries.by_table.get_mut(&key) {
                *used = now;
                return value.clone();
            }
        }
        let value = make();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.clock += 1;
        let now = entries.clock;
        entries.by_table.insert(key, (value.clone(), now));
        while entries.by_table.len() > self.size {
            let oldest = entries
                .by_table
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => entries.by_table.remove(&k),
                None => break,
            };
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_xml::{parse, Limits};
    use std::cell::Cell;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn table(text: &str) -> Arc<Element> {
        let xml =
            format!("<w:tbl xmlns:w='{W}'><w:tr><w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc></w:tr></w:tbl>");
        Arc::clone(parse(xml.as_bytes(), &Limits::default()).expect("parses").root())
    }

    #[test]
    fn a_table_with_equal_content_is_made_once() {
        let kept = Kept::new(4);
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
    }

    #[test]
    fn the_least_recently_used_goes_first() {
        let kept = Kept::new(2);
        kept.get_or(&table("a"), || 1);
        kept.get_or(&table("b"), || 2);
        kept.get_or(&table("a"), || 9);
        kept.get_or(&table("c"), || 3);
        assert_eq!(kept.get_or(&table("a"), || 9), 1, "used last, kept");
        assert_eq!(kept.get_or(&table("b"), || 7), 7, "used least, dropped");
    }
}
