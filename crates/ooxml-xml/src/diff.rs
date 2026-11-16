//! Owns: [`diff`], the structural difference of two versions of a tree as patches, which
//! short-circuits on shared subtrees (pointer equality), so comparing two versions costs
//! what differs between them, not what they hold.
//!
//! Must not: guess at moves (a moved block is a removal and an insertion), or compare text
//! by character (a changed text node is replaced whole).

use crate::node::{Element, Node, Tree};
use crate::patch::{NodePath, Patch};
use std::sync::Arc;

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
    diff_children(a, b, NodePath::root(), &mut out);
    out
}

/// Whether two children are equal, cheaply when they are shared.
fn equal(a: &Node, b: &Node) -> bool {
    a == b
}

fn diff_children(a: &Element, b: &Element, path: NodePath, out: &mut Vec<Patch>) {
    let (old, new) = (a.children(), b.children());
    if old.ptr_eq(new) {
        return;
    }
    let (n, m) = (old.len(), new.len());
    let mut prefix = 0;
    while prefix < n.min(m) && old.get(prefix).zip(new.get(prefix)).is_some_and(|(x, y)| equal(x, y)) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < (n - prefix).min(m - prefix)
        && old.get(n - 1 - suffix).zip(new.get(m - 1 - suffix)).is_some_and(|(x, y)| equal(x, y))
    {
        suffix += 1;
    }
    let (old_mid, new_mid) = (n - prefix - suffix, m - prefix - suffix);
    if old_mid == new_mid {
        // position by position: descend into elements of one head, replace the rest
        for k in prefix..prefix + old_mid {
            let (Some(x), Some(y)) = (old.get(k), new.get(k)) else { continue };
            match (x, y) {
                _ if equal(x, y) => {}
                (Node::Element(ex), Node::Element(ey)) if ex.head() == ey.head() => {
                    diff_children(ex, ey, path.child(k), out);
                }
                _ => out.push(Patch::splice(path.clone(), k, vec![x.clone()], vec![y.clone()])),
            }
        }
        return;
    }
    let removed = (prefix..n - suffix).filter_map(|k| old.get(k).cloned()).collect();
    let inserted = (prefix..m - suffix).filter_map(|k| new.get(k).cloned()).collect();
    out.push(Patch::splice(path, prefix, removed, inserted));
}
