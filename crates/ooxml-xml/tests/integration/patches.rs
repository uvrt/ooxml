//! Patches, their inverses and rebasing, cursors, and the structural diff, on random trees
//! and random edits (a fixed-seed generator, so the test needs no dependency).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_xml::{
    diff, parse, Attribute, Cursor, Element, Head, HeadPatch, Limits, Node, NodePath, Patch, PatchError, QName, Tree,
};
use std::sync::Arc;

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % (n.max(1) as u64)) as usize
    }
}

fn element(name: &str, children: Vec<Node>) -> Node {
    Node::Element(Arc::new(Element::new(
        Arc::new(QName::new(None, name, None)),
        vec![],
        vec![],
        children,
    )))
}

fn random_node(rng: &mut Rng, depth: usize, serial: &mut usize) -> Node {
    *serial += 1;
    if depth == 0 || rng.below(3) == 0 {
        return Node::Text(format!("t{serial}").into());
    }
    let n = rng.below(4);
    let kids = (0..n).map(|_| random_node(rng, depth - 1, serial)).collect();
    element(if rng.below(2) == 0 { "p" } else { "q" }, kids)
}

fn random_tree(rng: &mut Rng) -> Tree {
    let mut serial = 0;
    let width = 20 + rng.below(120);
    let body = (0..width).map(|_| random_node(rng, 3, &mut serial)).collect();
    Tree::from_root(Arc::new(Element::new(
        Arc::new(QName::new(None, "body", None)),
        vec![],
        vec![],
        body,
    )))
}

/// A random element of the tree, as a cursor.
fn random_element(rng: &mut Rng, tree: &Tree) -> Cursor {
    let mut at = Cursor::root(tree);
    loop {
        let kids: Vec<Cursor> = at.children().filter(|c| c.element().is_some()).collect();
        if kids.is_empty() || rng.below(3) == 0 {
            return at;
        }
        at = kids[rng.below(kids.len())].clone();
    }
}

fn random_patch(rng: &mut Rng, tree: &Tree, serial: &mut usize) -> Patch {
    let at = random_element(rng, tree);
    let len = at.element().unwrap().children().len();
    let start = rng.below(len + 1);
    let end = (start + rng.below(3)).min(len);
    let inserted = (0..rng.below(3)).map(|_| random_node(rng, 2, serial)).collect();
    at.splice(start..end, inserted).unwrap()
}

#[test]
fn patches_apply_and_invert() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut serial = 1_000_000;
    for _ in 0..20 {
        let tree = random_tree(&mut rng);
        for _ in 0..30 {
            let patch = random_patch(&mut rng, &tree, &mut serial);
            let edited = patch.apply(&tree).unwrap();
            // the edited element's children are the old ones spliced; the rest is shared
            let parent = patch.parent().0[1..].iter().fold(edited.root().clone(), |e, &i| {
                e.children().get(i).unwrap().as_element().unwrap().clone()
            });
            for (k, node) in patch.inserted().iter().enumerate() {
                assert!(parent.children().get(patch.start() + k).unwrap().same(node));
            }
            assert_eq!(patch.inverse().apply(&edited).unwrap(), tree);
            // applying it twice finds its removed nodes gone (unless it removed nothing, or
            // what it put there is equal to what it removes: a patch checks by content too)
            let there: Vec<_> = (0..patch.removed().len())
                .filter_map(|k| parent.children().get(patch.start() + k).cloned())
                .collect();
            if !patch.removed().is_empty() && there.as_slice() != patch.removed() {
                assert!(matches!(
                    patch.apply(&edited),
                    Err(PatchError::Stale { .. }) | Err(PatchError::NoElement(_))
                ));
            }
        }
    }
}

#[test]
fn rebased_patches_commute() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut serial = 2_000_000;
    let (mut both, mut conflicts) = (0, 0);
    for _ in 0..800 {
        let tree = random_tree(&mut rng);
        let (a, b) = (
            random_patch(&mut rng, &tree, &mut serial),
            random_patch(&mut rng, &tree, &mut serial),
        );
        let same_point_inserts =
            a.parent() == b.parent() && a.start() == b.start() && a.removed().is_empty() && b.removed().is_empty();
        match (b.rebase(&a), a.rebase(&b)) {
            (Ok(b2), Ok(a2)) => {
                let ab = b2.apply(&a.apply(&tree).unwrap()).unwrap();
                let ba = a2.apply(&b.apply(&tree).unwrap()).unwrap();
                if !same_point_inserts {
                    assert_eq!(ab, ba);
                }
                // `a` published first: moving `b` past it, or `a` past `b` keeping its place,
                // gives one tree, inserts at one point included
                let a_first = a.rebase_first(&b).unwrap();
                assert_eq!(ab, a_first.apply(&b.apply(&tree).unwrap()).unwrap());
                both += 1;
            }
            (Err(PatchError::Conflict(_)), Err(PatchError::Conflict(_))) => conflicts += 1,
            other => panic!("rebase is not symmetric: {other:?}"),
        }
    }
    assert!(both > 500 && conflicts > 3, "{both} rebased, {conflicts} conflicts");
}

/// `<r>` with the children named, as a tree.
fn row(names: &[&str]) -> Tree {
    let xml = names.iter().map(|n| format!("<{n}/>")).collect::<String>();
    parse(format!("<r>{xml}</r>").as_bytes(), &Limits::default()).unwrap()
}

/// The names of `tree`'s root's children, in order.
fn names(tree: &Tree) -> Vec<String> {
    tree.root()
        .children()
        .iter()
        .map(|n| n.as_element().unwrap().name().local().to_owned())
        .collect()
}

/// A splice of the root's children `start .. start + removed` of `tree` by new elements named
/// `inserted`.
fn splice(tree: &Tree, start: usize, removed: usize, inserted: &[&str]) -> Patch {
    let gone = tree
        .root()
        .children()
        .iter()
        .skip(start)
        .take(removed)
        .cloned()
        .collect();
    let new = inserted.iter().map(|n| element(n, vec![])).collect();
    Patch::splice(NodePath::root(), start, gone, new)
}

/// `past` published first and `later` after it, both made on `tree`: the tree `later` moved
/// past `past` makes, which `past` carried past `later` (keeping its place first) must make
/// too; or the conflict, which must be met both ways.
fn published_in_order(tree: &Tree, past: &Patch, later: &Patch) -> Result<Vec<String>, PatchError> {
    match (later.rebase(past), past.rebase_first(later)) {
        (Ok(moved), Ok(carried)) => {
            let serial = moved.apply(&past.apply(tree).unwrap()).unwrap();
            let carried = carried.apply(&later.apply(tree).unwrap()).unwrap();
            assert_eq!(names(&serial), names(&carried), "both ways round, one result");
            Ok(names(&serial))
        }
        (Err(a), Err(_)) => Err(a),
        other => panic!("a conflict one way only: {other:?}"),
    }
}

#[test]
fn the_tie_rule_at_one_place_case_by_case() {
    let tree = row(&["a", "b", "c", "d"]);
    // 1. both insert at one point: the one published first comes first, either one
    let (x, y) = (splice(&tree, 2, 0, &["x"]), splice(&tree, 2, 0, &["y"]));
    assert_eq!(
        published_in_order(&tree, &x, &y).unwrap(),
        ["a", "b", "x", "y", "c", "d"]
    );
    assert_eq!(
        published_in_order(&tree, &y, &x).unwrap(),
        ["a", "b", "y", "x", "c", "d"]
    );
    // 2. an insert where a range the other replaces starts: before its replacement
    let (insert, replace) = (splice(&tree, 1, 0, &["x"]), splice(&tree, 1, 2, &["B"]));
    let before = ["a", "x", "B", "d"];
    assert_eq!(published_in_order(&tree, &insert, &replace).unwrap(), before);
    assert_eq!(published_in_order(&tree, &replace, &insert).unwrap(), before);
    // 3. an insert where that range ends: after its replacement
    let insert = splice(&tree, 3, 0, &["x"]);
    let after = ["a", "B", "x", "d"];
    assert_eq!(published_in_order(&tree, &insert, &replace).unwrap(), after);
    assert_eq!(published_in_order(&tree, &replace, &insert).unwrap(), after);
    // 4. two ranges that meet at one point: in document order, no conflict
    let (left, right) = (splice(&tree, 0, 2, &["L"]), splice(&tree, 2, 2, &["R"]));
    assert_eq!(published_in_order(&tree, &left, &right).unwrap(), ["L", "R"]);
    assert_eq!(published_in_order(&tree, &right, &left).unwrap(), ["L", "R"]);
    // 5. ranges that overlap, or a change inside a node the other replaces: a conflict
    let (one, two) = (splice(&tree, 0, 2, &["P"]), splice(&tree, 1, 2, &["Q"]));
    assert!(matches!(
        published_in_order(&tree, &one, &two),
        Err(PatchError::Conflict(_))
    ));
    assert!(matches!(
        published_in_order(&tree, &two, &one),
        Err(PatchError::Conflict(_))
    ));
    let nested = parse(b"<r><a><i/></a><b/></r>", &Limits::default()).unwrap();
    let inside = Patch::splice(NodePath(vec![0, 0]), 0, vec![], vec![element("x", vec![])]);
    let whole = splice(&nested, 0, 1, &["A"]);
    assert!(matches!(
        published_in_order(&nested, &inside, &whole),
        Err(PatchError::Conflict(_))
    ));
    assert!(matches!(
        published_in_order(&nested, &whole, &inside),
        Err(PatchError::Conflict(_))
    ));
}

#[test]
fn diff_gives_patches_to_the_new_version() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut serial = 3_000_000;
    for _ in 0..30 {
        let old = random_tree(&mut rng);
        let mut new = old.clone();
        for _ in 0..rng.below(8) {
            let p = random_patch(&mut rng, &new, &mut serial);
            new = p.apply(&new).unwrap();
        }
        let patches = diff(&old, &new);
        let replayed = patches.iter().fold(old.clone(), |t, p| p.apply(&t).unwrap());
        assert_eq!(replayed, new);
        assert!(diff(&new, &new).is_empty());
    }
    // a changed root head is one document-level patch
    let a = parse(b"<a x=\"1\"><b/></a>", &Limits::default()).unwrap();
    let b = parse(b"<a x=\"2\"><b/></a>", &Limits::default()).unwrap();
    let patches = diff(&a, &b);
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].parent(), &NodePath::document());
    assert_eq!(patches[0].apply(&a).unwrap(), b);
}

#[test]
fn cursors_know_their_way_back() {
    let tree = parse(b"<r><a><b>x</b><c/></a><d/></r>", &Limits::default()).unwrap();
    let root = Cursor::root(&tree);
    let a = root.child(0).unwrap();
    let c = a.child(1).unwrap();
    assert_eq!(c.path(), NodePath(vec![0, 0, 1]));
    assert_eq!(c.prev_sibling().unwrap().element().unwrap().name().local(), "b");
    assert_eq!(a.next_sibling().unwrap().element().unwrap().name().local(), "d");
    assert_eq!(c.parent().unwrap().element().unwrap().name().local(), "a");
    let replaced = c.replace(Node::Text("y".into())).unwrap().apply(&tree).unwrap();
    assert_eq!(replaced.root().text_content(), "xy");
    let root_patch = root.replace(element("s", vec![])).unwrap();
    assert_eq!(root_patch.apply(&tree).unwrap().root().name().local(), "s");
    assert!(matches!(root_patch.rebase(&root_patch), Err(PatchError::Conflict(_))));
}

/// A head patch giving the element at `at` one more attribute.
fn random_head_patch(at: &Cursor, serial: &mut usize) -> HeadPatch {
    *serial += 1;
    let element = at.element().unwrap();
    let old = Arc::clone(element.head());
    let mut attributes = old.attributes.to_vec();
    attributes.push(Attribute {
        name: Arc::new(QName::new(None, &format!("a{serial}"), None)),
        value: "v".into(),
    });
    let new = Arc::new(Head {
        name: old.name.clone(),
        namespaces: old.namespaces.clone(),
        attributes: attributes.into(),
    });
    HeadPatch::new(at.path(), old, new)
}

#[test]
fn head_patches_apply_invert_and_commute_with_splices() {
    let mut rng = Rng(0x0DDB_1A5E_5BAD_5EED);
    let mut serial = 4_000_000;
    let (mut both, mut conflicts) = (0, 0);
    for _ in 0..600 {
        let tree = random_tree(&mut rng);
        let at = random_element(&mut rng, &tree);
        let h = random_head_patch(&at, &mut serial);
        let edited = h.apply(&tree).unwrap();
        assert_eq!(h.inverse().apply(&edited).unwrap(), tree);
        assert!(matches!(h.apply(&edited), Err(PatchError::Stale { .. })));
        let s = random_patch(&mut rng, &tree, &mut serial);
        match (h.rebase(&s), s.rebase_head(&h)) {
            (Ok(h2), Ok(s2)) => {
                let sh = h2.apply(&s.apply(&tree).unwrap()).unwrap();
                let hs = s2.apply(&h.apply(&tree).unwrap()).unwrap();
                assert_eq!(sh, hs);
                both += 1;
            }
            (Err(PatchError::Conflict(_)), Err(PatchError::Conflict(_))) => conflicts += 1,
            other => panic!("rebase is not symmetric: {other:?}"),
        }
        let other = random_head_patch(&random_element(&mut rng, &tree), &mut serial);
        assert_eq!(other.rebase_head(&h).is_err(), other.path() == h.path());
    }
    assert!(both > 300 && conflicts > 0, "{both} rebased, {conflicts} conflicts");
}
