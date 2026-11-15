//! Python's `difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()`, ported
//! step for step: the same longest matches (the earliest in `a`, then in `b`, of the
//! longest), found in the same order (the same stack of ranges), joined the same way, so
//! the same opcodes for the same sequences.  For sequences of fixed-width keys (a version's
//! blocks by their digests): what a call changed in a long document, in milliseconds.
use std::collections::HashMap;

/// An opcode as difflib gives it: the tag and the ranges `a[i1..i2]`, `b[j1..j2]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opcode {
    pub tag: Tag,
    pub i1: usize,
    pub i2: usize,
    pub j1: usize,
    pub j2: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Replace,
    Delete,
    Insert,
    Equal,
}

impl Tag {
    pub fn name(self) -> &'static str {
        match self {
            Tag::Replace => "replace",
            Tag::Delete => "delete",
            Tag::Insert => "insert",
            Tag::Equal => "equal",
        }
    }
}

struct Matcher<'s, T> {
    a: &'s [T],
    b: &'s [T],
    b2j: HashMap<&'s T, Vec<usize>>,
}

impl<'s, T: Eq + std::hash::Hash> Matcher<'s, T> {
    fn new(a: &'s [T], b: &'s [T]) -> Self {
        // set_seq2 / __chain_b with no junk and autojunk off: every element, its places in
        // order.
        let mut b2j: HashMap<&T, Vec<usize>> = HashMap::new();
        for (j, x) in b.iter().enumerate() {
            b2j.entry(x).or_default().push(j);
        }
        Matcher { a, b, b2j }
    }

    /// `find_longest_match(alo, ahi, blo, bhi)`.
    fn longest(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (a, b) = (self.a, self.b);
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        let mut newj2len: HashMap<usize, usize> = HashMap::new();
        for (i, x) in a.iter().enumerate().take(ahi).skip(alo) {
            newj2len.clear();
            if let Some(places) = self.b2j.get(x) {
                for &j in places {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = if j > 0 {
                        j2len.get(&(j - 1)).copied().unwrap_or(0)
                    } else {
                        0
                    } + 1;
                    newj2len.insert(j, k);
                    if k > bestsize {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        bestsize = k;
                    }
                }
            }
            std::mem::swap(&mut j2len, &mut newj2len);
        }
        // The extensions over equal (non-junk) elements on either side: no junk here.
        while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < ahi && bestj + bestsize < bhi && a[besti + bestsize] == b[bestj + bestsize] {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }

    /// `get_matching_blocks()`.
    fn matching_blocks(&self) -> Vec<(usize, usize, usize)> {
        let (la, lb) = (self.a.len(), self.b.len());
        let mut queue = vec![(0, la, 0, lb)];
        let mut blocks = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.longest(alo, ahi, blo, bhi);
            if k > 0 {
                blocks.push((i, j, k));
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        blocks.sort_unstable();
        let (mut i1, mut j1, mut k1) = (0, 0, 0);
        let mut out = Vec::new();
        for (i2, j2, k2) in blocks {
            if i1 + k1 == i2 && j1 + k1 == j2 {
                k1 += k2;
            } else {
                if k1 > 0 {
                    out.push((i1, j1, k1));
                }
                (i1, j1, k1) = (i2, j2, k2);
            }
        }
        if k1 > 0 {
            out.push((i1, j1, k1));
        }
        out.push((la, lb, 0));
        out
    }
}

/// `difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()`.
pub fn opcodes<T: Eq + std::hash::Hash>(a: &[T], b: &[T]) -> Vec<Opcode> {
    let matcher = Matcher::new(a, b);
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    for (ai, bj, size) in matcher.matching_blocks() {
        let tag = if i < ai && j < bj {
            Some(Tag::Replace)
        } else if i < ai {
            Some(Tag::Delete)
        } else if j < bj {
            Some(Tag::Insert)
        } else {
            None
        };
        if let Some(tag) = tag {
            out.push(Opcode {
                tag,
                i1: i,
                i2: ai,
                j1: j,
                j2: bj,
            });
        }
        i = ai + size;
        j = bj + size;
        if size > 0 {
            out.push(Opcode {
                tag: Tag::Equal,
                i1: ai,
                i2: i,
                j1: bj,
                j2: j,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(a: &str, b: &str) -> Vec<(&'static str, usize, usize, usize, usize)> {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        opcodes(&a, &b)
            .into_iter()
            .map(|o| (o.tag.name(), o.i1, o.i2, o.j1, o.j2))
            .collect()
    }

    #[test]
    fn as_difflib() {
        // difflib.SequenceMatcher(None, "qabxcd", "abycdf", autojunk=False).get_opcodes()
        assert_eq!(
            ops("qabxcd", "abycdf"),
            vec![
                ("delete", 0, 1, 0, 0),
                ("equal", 1, 3, 0, 2),
                ("replace", 3, 4, 2, 3),
                ("equal", 4, 6, 3, 5),
                ("insert", 6, 6, 5, 6)
            ]
        );
        // repeated elements: difflib's choice among equal matches (the earliest)
        assert_eq!(ops("aaaa", "aa"), vec![("equal", 0, 2, 0, 2), ("delete", 2, 4, 2, 2)]);
        assert_eq!(ops("abab", "ab"), vec![("equal", 0, 2, 0, 2), ("delete", 2, 4, 2, 2)]);
        assert_eq!(ops("", ""), vec![]);
        assert_eq!(ops("ab", ""), vec![("delete", 0, 2, 0, 0)]);
        assert_eq!(ops("", "ab"), vec![("insert", 0, 0, 0, 2)]);
    }
}
