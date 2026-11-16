//! Owns: [`diff`], the structural difference of two versions of a tree as patches. It
//! short-circuits on shared subtrees (pointer equality), so comparing two versions costs
//! what differs between them, not what they hold. Within an element whose children differ
//! it finds a shortest edit script (Myers' O(ND) algorithm, the edit distance capped at
//! [`MAX_EDITS`]: beyond it the differing middle is one splice), and descends into a child
//! replaced by one of the same head.
//!
//! Must not: guess at moves (a moved block is a removal and an insertion), or compare text
//! by character (a changed text node is replaced whole).

use crate::node::{Element, Node, Tree};
use crate::patch::{NodePath, Patch};
use crate::rope::Children;
use std::sync::Arc;

/// The largest edit distance searched for within one element's children.
pub const MAX_EDITS: usize = 1024;

/// Patches that, applied in order to `old`, give a tree equal to `new`. Empty when the two
/// are equal.
pub fn diff(old: &Tree, new: &Tree) -> Vec<Patch> {
    let mut out = Vec::new();
    let (a, b) = (old.root(), new.root());
    if Arc::ptr_eq(a, b) {
        return out;
    }
    if a.head() != b.head() {
        out.push(Patch::replace_root(a, Arc::clone(b)));
        return out;
    }
    diff_children(a, b, &NodePath::root(), &mut out);
    out
}

/// A run of `old[a..a + n]` replaced by `new[b..b + m]`.
#[derive(Debug, Clone, Copy)]
struct Hunk {
    a: usize,
    n: usize,
    b: usize,
    m: usize,
}

fn diff_children(a: &Element, b: &Element, path: &NodePath, out: &mut Vec<Patch>) {
    let (old, new) = (a.children(), b.children());
    if old.ptr_eq(new) {
        return;
    }
    let (n, m) = (old.len(), new.len());
    let same = |i: usize, j: usize| old.get(i).zip(new.get(j)).is_some_and(|(x, y)| x == y);
    let mut prefix = 0;
    while prefix < n.min(m) && same(prefix, prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < (n - prefix).min(m - prefix) && same(n - 1 - suffix, m - 1 - suffix) {
        suffix += 1;
    }
    let middle = Hunk {
        a: prefix,
        n: n - prefix - suffix,
        b: prefix,
        m: m - prefix - suffix,
    };
    let hunks = edit_script(old, new, middle).unwrap_or_else(|| vec![middle]);
    // last first, so each patch's position is still the old tree's
    for h in hunks.iter().rev() {
        emit(old, new, *h, path, out);
    }
}

/// One hunk as patches: element by element where both sides have as many children and the
/// pairs share a head (descending into them), else one splice.
fn emit(old: &Children, new: &Children, h: Hunk, path: &NodePath, out: &mut Vec<Patch>) {
    let paired = h.n == h.m
        && (0..h.n).all(|k| match (old.get(h.a + k), new.get(h.b + k)) {
            (Some(Node::Element(x)), Some(Node::Element(y))) => x.head() == y.head(),
            _ => false,
        });
    if paired {
        for k in (0..h.n).rev() {
            if let (Some(Node::Element(x)), Some(Node::Element(y))) = (old.get(h.a + k), new.get(h.b + k)) {
                diff_children(x, y, &path.child(h.a + k), out);
            }
        }
        return;
    }
    let removed = (h.a..h.a + h.n).filter_map(|k| old.get(k).cloned()).collect();
    let inserted = (h.b..h.b + h.m).filter_map(|k| new.get(k).cloned()).collect();
    out.push(Patch::splice(path.clone(), h.a, removed, inserted));
}

/// The hunks of a shortest edit script of `old[h.a..]` into `new[h.b..]` within `h`
/// (Myers 1986, the greedy forward search, keeping each step's frontier to trace back), or
/// `None` when it takes more than [`MAX_EDITS`] edits.
fn edit_script(old: &Children, new: &Children, h: Hunk) -> Option<Vec<Hunk>> {
    let (n, m) = (h.n as isize, h.m as isize);
    let same = |x: isize, y: isize| {
        old.get(h.a + x as usize)
            .zip(new.get(h.b + y as usize))
            .is_some_and(|(p, q)| p == q)
    };
    let offset = MAX_EDITS as isize + 1;
    let mut v = vec![0isize; 2 * MAX_EDITS + 3];
    // each step's frontier before it, diagonals -d-1 ..= d+1 (all the trace back reads)
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let at = |k: isize| (k + offset) as usize;
    for d in 0..=(MAX_EDITS as isize).min(n + m) {
        trace.push(v.get(at(-d - 1)..=at(d + 1))?.to_vec());
        for k in (-d..=d).step_by(2) {
            let down = k == -d || (k != d && v.get(at(k - 1)) < v.get(at(k + 1)));
            let mut x = if down {
                v.get(at(k + 1)).copied()?
            } else {
                v.get(at(k - 1)).copied()? + 1
            };
            let mut y = x - k;
            while x < n && y < m && same(x, y) {
                x += 1;
                y += 1;
            }
            *v.get_mut(at(k))? = x;
            if x >= n && y >= m {
                return Some(trace_back(&trace, n, m, h));
            }
        }
    }
    None
}

/// The hunks of the path the frontiers in `trace` found, merged where they touch.
fn trace_back(trace: &[Vec<isize>], n: isize, m: isize, h: Hunk) -> Vec<Hunk> {
    let (mut x, mut y) = (n, m);
    let mut edits: Vec<(isize, isize, bool)> = Vec::new(); // (x, y) before the edit; true = insertion
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        if d == 0 {
            break;
        }
        // the window of step d starts at diagonal -d-1
        let at = |k: isize| (k + d + 1) as usize;
        let k = x - y;
        let down = k == -d || (k != d && v.get(at(k - 1)) < v.get(at(k + 1)));
        let prev_k = if down { k + 1 } else { k - 1 };
        let prev_x = v.get(at(prev_k)).copied().unwrap_or(0);
        let prev_y = prev_x - prev_k;
        edits.push((prev_x, prev_y, down));
        x = prev_x;
        y = prev_y;
    }
    edits.reverse();
    let mut hunks: Vec<Hunk> = Vec::new();
    for (x, y, insertion) in edits {
        let (x, y) = (x as usize, y as usize);
        let (dn, dm) = if insertion { (0, 1) } else { (1, 0) };
        match hunks.last_mut() {
            Some(last) if last.a + last.n == h.a + x && last.b + last.m == h.b + y => {
                last.n += dn;
                last.m += dm;
            }
            _ => hunks.push(Hunk {
                a: h.a + x,
                n: dn,
                b: h.b + y,
                m: dm,
            }),
        }
    }
    hunks
}
