//! Owns: the matching blocks of two sequences, as Python's
//! `difflib.SequenceMatcher(None, a, b, autojunk=False).get_matching_blocks()` finds them --
//! the same longest matches, found in the same order, joined the same way -- each
//! `find_longest_match` answered by a suffix automaton of `b`'s range, so one costs the
//! lengths of the two ranges rather than difflib's product of `a`'s range and the places
//! each of its elements has in `b` (minutes for 20,000 characters of repeated words).
//!
//! A diff that would examine more than [`WORK`] elements over all its longest matches (texts
//! long and unlike, so that matches peel off one short block at a time) is not matched
//! further: the common start and end are kept and everything between is one change
//! ([`ends_only`]). Word reads either the same way; docx-agent has no such bound and does not
//! finish on those texts.
//!
//! Must not: tell replacements, deletions and insertions apart, or fold anything; the
//! callers' rules ([`super::diff`]) do that.

use std::collections::HashMap;
use std::hash::Hash;

/// The most elements all `find_longest_match` calls of one diff may examine together (the
/// lengths of both ranges, per call): about a tenth of a second.
pub(crate) const WORK: usize = 4_000_000;

/// `(i, j, k)`: `a[i..i + k]` is `b[j..j + k]`.
pub(crate) type Block = (usize, usize, usize);

/// No symbol: an element of `a` that `b` does not have, or no state.
const NONE: u32 = u32::MAX;

/// `get_matching_blocks()` (ending in the `(len(a), len(b), 0)` sentinel), or
/// [`ends_only`] past [`WORK`].
pub(crate) fn matching_blocks<T: Eq + Hash>(a: &[T], b: &[T]) -> Vec<Block> {
    matching_blocks_within(a, b, WORK)
}

/// [`matching_blocks`] with its own bound on the work.
pub(crate) fn matching_blocks_within<T: Eq + Hash>(a: &[T], b: &[T], work: usize) -> Vec<Block> {
    let (la, lb) = (a.len(), b.len());
    let mut blocks = Symbols::new(a, b).blocks(work).unwrap_or_else(|| ends_only(a, b));
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

/// The diff past [`WORK`]: the longest common start and, of what is left, the longest
/// common end.
fn ends_only<T: Eq>(a: &[T], b: &[T]) -> Vec<Block> {
    let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let rest = (a.len() - start).min(b.len() - start);
    let end = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(rest)
        .take_while(|(x, y)| x == y)
        .count();
    [(0, 0, start), (a.len() - end, b.len() - end, end)]
        .into_iter()
        .filter(|&(_, _, k)| k > 0)
        .collect()
}

/// Both sequences as dense symbols (`b`'s elements numbered; an element of `a` that `b`
/// lacks is [`NONE`]).
struct Symbols {
    a: Vec<u32>,
    b: Vec<u32>,
}

impl Symbols {
    fn new<T: Eq + Hash>(a: &[T], b: &[T]) -> Self {
        let mut ids: HashMap<&T, u32> = HashMap::new();
        let b = b
            .iter()
            .map(|x| {
                let next = u32::try_from(ids.len()).unwrap_or(NONE);
                *ids.entry(x).or_insert(next)
            })
            .collect();
        let a = a.iter().map(|x| ids.get(x).copied().unwrap_or(NONE)).collect();
        Symbols { a, b }
    }

    /// The longest matches `get_matching_blocks` finds, unsorted; `None` past `work`.
    fn blocks(&self, work: usize) -> Option<Vec<Block>> {
        let mut automaton = Automaton::default();
        let mut spent = 0usize;
        let mut queue = vec![(0, self.a.len(), 0, self.b.len())];
        let mut blocks = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            spent += (ahi - alo) + (bhi - blo);
            if spent > work {
                return None;
            }
            let (i, j, k) = self.longest(&mut automaton, alo, ahi, blo, bhi);
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
        Some(blocks)
    }

    /// `find_longest_match(alo, ahi, blo, bhi)` without junk: the longest common stretch of
    /// `a[alo..ahi]` and `b[blo..bhi]`; of the longest, the one starting first in `a`, and of
    /// those the one starting first in `b`; `(alo, blo, 0)` when there is none.
    ///
    /// difflib walks `a` and, for each element, its places in `b` in order, keeping a
    /// strictly longer match only: the first `i` at which a longest match ends, then the
    /// first `j`. Here the automaton gives, at each `i`, the longest match ending there and
    /// the first place in `b`'s range it ends at.
    fn longest(&self, automaton: &mut Automaton, alo: usize, ahi: usize, blo: usize, bhi: usize) -> Block {
        automaton.build(self.b.get(blo..bhi).unwrap_or_default());
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        let (mut state, mut length) = (0u32, 0usize);
        for (i, &symbol) in self.a.iter().enumerate().take(ahi).skip(alo) {
            (state, length) = automaton.step(state, length, symbol);
            if length > bestsize {
                let first = automaton.first(state);
                (besti, bestj, bestsize) = (i + 1 - length, blo + first + 1 - length, length);
            }
        }
        (besti, bestj, bestsize)
    }
}

/// A state of the suffix automaton: the substrings of `b`'s range sharing their end places.
#[derive(Default)]
struct State {
    /// The longest substring's length.
    len: usize,
    /// The suffix link ([`NONE`] for the root).
    link: u32,
    /// Where (in the range) the first occurrence of its substrings ends.
    first: usize,
    /// The transitions, sorted by symbol (the root has one per distinct symbol: a word
    /// diff's whole vocabulary).
    next: Vec<(u32, u32)>,
}

/// A suffix automaton, rebuilt per range; its states' allocations are reused.
#[derive(Default)]
struct Automaton {
    states: Vec<State>,
    size: usize,
}

impl Automaton {
    fn build(&mut self, text: &[u32]) {
        self.size = 0;
        let mut last = self.push(0, NONE, 0);
        for (at, &symbol) in text.iter().enumerate() {
            last = self.extend(last, symbol, at);
        }
    }

    fn push(&mut self, len: usize, link: u32, first: usize) -> u32 {
        let id = self.size;
        self.size += 1;
        match self.states.get_mut(id) {
            Some(state) => {
                (state.len, state.link, state.first) = (len, link, first);
                state.next.clear();
            }
            None => self.states.push(State {
                len,
                link,
                first,
                next: Vec::new(),
            }),
        }
        u32::try_from(id).unwrap_or(NONE)
    }

    fn state(&self, id: u32) -> Option<&State> {
        self.states.get(id as usize)
    }

    fn len(&self, id: u32) -> usize {
        self.state(id).map_or(0, |s| s.len)
    }

    fn link(&self, id: u32) -> u32 {
        self.state(id).map_or(NONE, |s| s.link)
    }

    fn first(&self, id: u32) -> usize {
        self.state(id).map_or(0, |s| s.first)
    }

    fn next(&self, id: u32, symbol: u32) -> u32 {
        self.state(id)
            .and_then(|s| {
                let at = s.next.binary_search_by_key(&symbol, |&(c, _)| c).ok()?;
                s.next.get(at)
            })
            .map_or(NONE, |&(_, to)| to)
    }

    fn set_next(&mut self, id: u32, symbol: u32, to: u32) {
        if let Some(state) = self.states.get_mut(id as usize) {
            match state.next.binary_search_by_key(&symbol, |&(c, _)| c) {
                Ok(at) => {
                    if let Some(edge) = state.next.get_mut(at) {
                        edge.1 = to;
                    }
                }
                Err(at) => state.next.insert(at, (symbol, to)),
            }
        }
    }

    fn set_link(&mut self, id: u32, link: u32) {
        if let Some(state) = self.states.get_mut(id as usize) {
            state.link = link;
        }
    }

    /// Append `symbol` (at `at` in the range) to the automaton whose whole text ends in
    /// `last`; the new `last`.
    fn extend(&mut self, last: u32, symbol: u32, at: usize) -> u32 {
        let current = self.push(self.len(last) + 1, NONE, at);
        let mut p = last;
        while p != NONE && self.next(p, symbol) == NONE {
            self.set_next(p, symbol, current);
            p = self.link(p);
        }
        if p == NONE {
            self.set_link(current, 0);
            return current;
        }
        let q = self.next(p, symbol);
        if self.len(p) + 1 == self.len(q) {
            self.set_link(current, q);
            return current;
        }
        let edges = self.state(q).map(|s| s.next.clone()).unwrap_or_default();
        let clone = self.push(self.len(p) + 1, self.link(q), self.first(q));
        if let Some(state) = self.states.get_mut(clone as usize) {
            state.next = edges;
        }
        while p != NONE && self.next(p, symbol) == q {
            self.set_next(p, symbol, clone);
            p = self.link(p);
        }
        self.set_link(q, clone);
        self.set_link(current, clone);
        current
    }

    /// From the longest match `length` ending in `state`, read `symbol`: the state and
    /// length of the longest match ending with it.
    fn step(&self, mut state: u32, mut length: usize, symbol: u32) -> (u32, usize) {
        if symbol == NONE {
            return (0, 0);
        }
        loop {
            let to = self.next(state, symbol);
            if to != NONE {
                return (to, length + 1);
            }
            if state == 0 {
                return (0, 0);
            }
            state = self.link(state);
            length = self.len(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// difflib's own `find_longest_match` and `get_matching_blocks` (the `j2len` walk), as
    /// the reference the automaton must agree with.
    fn reference(a: &[u32], b: &[u32]) -> Vec<Block> {
        let mut b2j: HashMap<u32, Vec<usize>> = HashMap::new();
        for (j, x) in b.iter().enumerate() {
            b2j.entry(*x).or_default().push(j);
        }
        let longest = |alo: usize, ahi: usize, blo: usize, bhi: usize| {
            let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
            let mut j2len: HashMap<usize, usize> = HashMap::new();
            for (i, x) in a.iter().enumerate().take(ahi).skip(alo) {
                let mut new: HashMap<usize, usize> = HashMap::new();
                for &j in b2j.get(x).map_or(&[][..], Vec::as_slice) {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                    new.insert(j, k);
                    if k > bestsize {
                        (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
                    }
                }
                j2len = new;
            }
            (besti, bestj, bestsize)
        };
        let mut queue = vec![(0, a.len(), 0, b.len())];
        let mut blocks = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = longest(alo, ahi, blo, bhi);
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
        blocks
    }

    /// A small deterministic generator (xorshift).
    struct Random(u64);

    impl Random {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }

        fn text(&mut self, alphabet: u64) -> Vec<u32> {
            let len = self.below(60);
            (0..len)
                .map(|_| u32::try_from(self.below(alphabet)).unwrap_or(0))
                .collect()
        }
    }

    #[test]
    fn the_automaton_finds_difflibs_longest_matches() {
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        for round in 0..4000 {
            let alphabet = 1 + round % 6;
            let a = random.text(alphabet);
            let b = if round % 3 == 0 {
                // alike: b is a with a few elements changed
                let mut b = a.clone();
                for _ in 0..random.below(4) {
                    if let Some(x) = b.get_mut(usize::try_from(random.below(60)).unwrap_or(0)) {
                        *x = u32::try_from(random.below(alphabet)).unwrap_or(0);
                    }
                }
                b
            } else {
                random.text(alphabet)
            };
            let mut want = reference(&a, &b);
            let mut found = Symbols::new(&a, &b).blocks(WORK).unwrap_or_default();
            want.sort_unstable();
            found.sort_unstable();
            assert_eq!(found, want, "{a:?} {b:?}");
        }
    }

    #[test]
    fn past_the_work_bound_only_the_ends_are_kept() {
        let a: Vec<char> = "the start, before text before text, the end".chars().collect();
        let b: Vec<char> = "the start, after text after text, the end".chars().collect();
        assert_eq!(
            matching_blocks_within(&a, &b, 10),
            vec![(0, 0, 11), (29, 27, 14), (43, 41, 0)]
        );
        let same: Vec<char> = "same".chars().collect();
        assert_eq!(matching_blocks_within(&same, &same, 0), vec![(0, 0, 4), (4, 4, 0)]);
    }

    #[test]
    fn long_unlike_texts_finish() {
        let a: Vec<char> = "before text ".repeat(1900).chars().collect();
        let b: Vec<char> = "after text ".repeat(1890).chars().collect();
        let blocks = matching_blocks(&a, &b);
        assert_eq!(blocks.last(), Some(&(a.len(), b.len(), 0)));
    }
}
