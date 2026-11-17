//! Owns: [`PartMap`], a small persistent sorted map from part name to a value: entries in
//! sorted chunks of at most [`CHUNK_MAX`], shared between versions, so a version that
//! changes one part copies one chunk and the list of chunks, not the package's every entry.
//!
//! Must not: know what the values are.
//!
//! (A persistent map crate was considered: imbl, the approved candidate, is MPL-2.0, which
//! the licence policy of design section 2.7 does not allow for a dependency.)

use std::sync::Arc;

/// Most entries in a chunk; a chunk that grows past it is split in two.
const CHUNK_MAX: usize = 32;

type Chunk<V> = Arc<[(Arc<str>, V)]>;

/// A persistent sorted map from names to `V`.
#[derive(Debug, Clone)]
pub(crate) struct PartMap<V> {
    chunks: Arc<[Chunk<V>]>,
}

impl<V> Default for PartMap<V> {
    fn default() -> Self {
        PartMap { chunks: Arc::new([]) }
    }
}

impl<V: Clone> PartMap<V> {
    /// The chunk that holds `key`, or would: the last whose first key is not after it.
    fn chunk_of(&self, key: &str) -> usize {
        self.chunks
            .partition_point(|c| c.first().is_some_and(|(k, _)| &**k <= key))
            .saturating_sub(1)
    }

    /// The value of `key`.
    pub(crate) fn get(&self, key: &str) -> Option<&V> {
        let chunk = self.chunks.get(self.chunk_of(key))?;
        let at = chunk.binary_search_by(|(k, _)| (**k).cmp(key)).ok()?;
        chunk.get(at).map(|(_, v)| v)
    }

    /// Whether `key` is in the map.
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Every entry, in key order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.chunks.iter().flat_map(|c| c.iter().map(|(k, v)| (&**k, v)))
    }

    /// This map with `key` set to `value`, and the value it replaced.
    pub(crate) fn insert(&self, key: &str, value: V) -> (PartMap<V>, Option<V>) {
        let ci = self.chunk_of(key);
        let mut entries: Vec<(Arc<str>, V)> = self.chunks.get(ci).map(|c| c.to_vec()).unwrap_or_default();
        let old = match entries.binary_search_by(|(k, _)| (**k).cmp(key)) {
            Ok(i) => entries.get_mut(i).map(|slot| std::mem::replace(&mut slot.1, value)),
            Err(i) => {
                entries.insert(i, (Arc::from(key), value));
                None
            }
        };
        let replacement: Vec<Chunk<V>> = if entries.len() > CHUNK_MAX {
            let tail = entries.split_off(entries.len() / 2);
            vec![entries.into(), tail.into()]
        } else {
            vec![entries.into()]
        };
        (self.with_chunks(ci, replacement), old)
    }

    /// This map without `key`, and the value removed; `None` when it has no `key`.
    pub(crate) fn remove(&self, key: &str) -> Option<(PartMap<V>, V)> {
        let ci = self.chunk_of(key);
        let mut entries = self.chunks.get(ci)?.to_vec();
        let at = entries.binary_search_by(|(k, _)| (**k).cmp(key)).ok()?;
        let (_, value) = entries.remove(at);
        let replacement = if entries.is_empty() {
            Vec::new()
        } else {
            vec![entries.into()]
        };
        Some((self.with_chunks(ci, replacement), value))
    }

    /// The chunk list with chunk `ci` replaced by `replacement` (it is appended when the
    /// map is empty).
    fn with_chunks(&self, ci: usize, replacement: Vec<Chunk<V>>) -> PartMap<V> {
        let end = (ci + 1).min(self.chunks.len());
        let chunks: Vec<Chunk<V>> = self
            .chunks
            .iter()
            .take(ci)
            .cloned()
            .chain(replacement)
            .chain(self.chunks.iter().skip(end).cloned())
            .collect();
        PartMap { chunks: chunks.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn behaves_as_a_sorted_map_and_shares_versions() {
        let mut map: PartMap<usize> = PartMap::default();
        let mut model = BTreeMap::new();
        let mut versions = Vec::new();
        for i in 0..500usize {
            let key = format!("part{:03}", (i * 7919) % 300);
            if i % 5 == 4 {
                let removed = map.remove(&key);
                assert_eq!(removed.as_ref().map(|(_, v)| *v), model.remove(&key));
                if let Some((m, _)) = removed {
                    map = m;
                }
            } else {
                let (m, old) = map.insert(&key, i);
                assert_eq!(old, model.insert(key.clone(), i));
                map = m;
            }
            versions.push((map.clone(), model.clone()));
        }
        for (m, model) in versions.iter().step_by(37) {
            let got: Vec<(String, usize)> = m.iter().map(|(k, v)| (k.to_owned(), *v)).collect();
            let want: Vec<(String, usize)> = model.iter().map(|(k, v)| (k.clone(), *v)).collect();
            assert_eq!(got, want);
            for (k, v) in model {
                assert_eq!(m.get(k), Some(v));
            }
            assert!(!m.contains_key("absent"));
        }
    }
}
